package sh.zeron.android.car

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/** One file of the model, pinned by size and SHA-256. */
data class ModelFile(val name: String, val size: Long, val sha256: String)

/**
 * The on-device dictation model: NVIDIA Parakeet Unified 0.6B EN, INT8, in the ONNX
 * conversion published at `bobNight/parakeet-unified-en-0.6b-onnx`, pinned to one
 * immutable revision with the SHA-256 of every file (the download is verified and
 * refused on a mismatch, as desktop dictation's is).
 *
 * Licences: NVIDIA's weights are under the NVIDIA Open Model License
 * (https://huggingface.co/nvidia/parakeet-unified-en-0.6b, `license: other`); the
 * ONNX conversion's card says CC-BY-4.0. See docs/reference/android-car-dictation.md.
 */
object ParakeetUnified {
    const val REVISION = "09e9060322d99c5f070010724786e6ee090fd51d"
    const val BASE_URL = "https://huggingface.co/bobNight/parakeet-unified-en-0.6b-onnx/resolve/$REVISION/"
    val FILES = listOf(
        ModelFile("encoder.int8.onnx", 42_606_669, "c81adfab77634e00c1668a221a14f244c5fb3409e7c14eeebaf6ac963425910f"),
        ModelFile("encoder.int8.onnx.data", 611_491_584, "3d54dd04646c15677bd2844a84df3770b12cc1ce183481f7b6e0def31c92114a"),
        ModelFile("decoder_joint.int8.onnx", 8_995_064, "7f76ad5f35035f25630075699c6c942a2c0c05ff42cb398f966f3c256d148e1e"),
        ModelFile("tokenizer.model", 251_056, "07d4e5a63840a53ab2d4d106d2874768143fb3fbdd47938b3910d2da05bfb0a9"),
    )
}

sealed interface ModelState {
    data object Missing : ModelState
    data class Downloading(val done: Long, val total: Long) : ModelState
    data object Ready : ModelState
    data class Failed(val message: String) : ModelState
}

/** An HTTP source for [LocalModel]: a range read of [url] from [from]. */
fun interface ModelSource {
    /** Opens [url] at byte [from]; the response says its status (206 for a range, 200 for everything). */
    fun open(url: String, from: Long): HttpURLConnection
}

private val httpSource = ModelSource { url, from ->
    (URL(url).openConnection() as HttpURLConnection).apply {
        connectTimeout = 15_000
        readTimeout = 30_000
        instanceFollowRedirects = true
        if (from > 0) setRequestProperty("Range", "bytes=$from-")
    }
}

/**
 * Downloads, verifies and keeps the model in [dir]. A file is staged as `.part`
 * (resumed after an interruption), hashed, and only then moved into place; the
 * model is [ModelState.Ready] when every file is present at its size and the
 * `.verified` marker (written after all hashes matched) names this revision.
 */
class LocalModel(
    val dir: File,
    private val files: List<ModelFile> = ParakeetUnified.FILES,
    private val baseUrl: String = ParakeetUnified.BASE_URL,
    private val revision: String = ParakeetUnified.REVISION,
    private val source: ModelSource = httpSource,
) {
    private val marker get() = File(dir, ".verified")
    private val staging get() = File(dir, ".staging")
    val totalBytes: Long = files.sumOf { it.size }

    private val _state = MutableStateFlow(if (isReady()) ModelState.Ready else ModelState.Missing)
    val state: StateFlow<ModelState> = _state.asStateFlow()

    /** Cheap: sizes and the marker, not the hashes (those were checked on download). */
    fun isReady(): Boolean =
        marker.takeIf { it.isFile }?.readText()?.trim() == revision &&
            files.all { File(dir, it.name).let { f -> f.isFile && f.length() == it.size } }

    /** Downloads what is missing, verifying each file; returns once [ModelState.Ready] or throws. */
    suspend fun download(): Unit = withContext(Dispatchers.IO) {
        if (isReady()) {
            _state.value = ModelState.Ready
            return@withContext
        }
        try {
            dir.mkdirs()
            staging.mkdirs()
            marker.delete()
            var done = files.filter { File(dir, it.name).let { f -> f.isFile && f.length() == it.size } }.sumOf { it.size }
            _state.value = ModelState.Downloading(done, totalBytes)
            for (file in files) {
                val target = File(dir, file.name)
                if (target.isFile && target.length() == file.size && sha256(target) == file.sha256) continue
                target.delete()
                val base = done
                fetch(file, File(staging, file.name + ".part")) { got ->
                    _state.value = ModelState.Downloading(base + got, totalBytes)
                }
                done += file.size
            }
            marker.writeText(revision)
            staging.deleteRecursively()
            _state.value = ModelState.Ready
        } catch (e: CancellationException) {
            _state.value = ModelState.Missing
            throw e
        } catch (e: Exception) {
            _state.value = ModelState.Failed(e.message ?: e.javaClass.simpleName)
            throw e
        }
    }

    private suspend fun fetch(file: ModelFile, part: File, progress: (Long) -> Unit) {
        val ctx = currentCoroutineContext()
        var have = if (part.isFile) part.length() else 0L
        if (have > file.size) {
            part.delete()
            have = 0
        }
        if (have < file.size) {
            val connection = source.open(baseUrl + file.name, have)
            try {
                val append = connection.responseCode == 206 && have > 0
                if (connection.responseCode != 200 && connection.responseCode != 206) {
                    throw IOException("${file.name}: HTTP ${connection.responseCode}")
                }
                if (!append) have = 0 // The server sent the whole file: start over.
                java.io.FileOutputStream(part, append).use { out ->
                    connection.inputStream.use { input ->
                        val buffer = ByteArray(256 * 1024)
                        var got = have
                        var reported = 0L
                        while (true) {
                            ctx.ensureActive()
                            val n = input.read(buffer)
                            if (n < 0) break
                            out.write(buffer, 0, n)
                            got += n
                            if (got - reported >= 512 * 1024) {
                                progress(got)
                                reported = got
                            }
                        }
                    }
                }
            } finally {
                connection.disconnect()
            }
        }
        if (part.length() != file.size) throw IOException("${file.name}: ${part.length()} of ${file.size} bytes")
        val actual = sha256(part)
        if (actual != file.sha256) {
            part.delete()
            throw IOException("${file.name}: checksum mismatch")
        }
        progress(file.size)
        val target = File(dir, file.name)
        if (!part.renameTo(target)) throw IOException("${file.name}: could not move into place")
    }

    /** Deletes the model (and any half-finished download). */
    fun remove() {
        marker.delete()
        dir.deleteRecursively()
        _state.value = ModelState.Missing
    }

    /** Forgets a failure so the next download starts clean from the same part files. */
    fun reset() {
        if (_state.value is ModelState.Failed) _state.value = if (isReady()) ModelState.Ready else ModelState.Missing
    }

    companion object {
        fun sha256(file: File): String {
            val digest = MessageDigest.getInstance("SHA-256")
            file.inputStream().use { input ->
                val buffer = ByteArray(1 shl 20)
                while (true) {
                    val n = input.read(buffer)
                    if (n < 0) break
                    digest.update(buffer, 0, n)
                }
            }
            return digest.digest().joinToString("") { "%02x".format(it) }
        }
    }
}
