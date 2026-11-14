package sh.zeron.android.car

import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * On a device or emulator: the real library and ONNX Runtime from the APK, the real model.
 * The model and clips are pushed beforehand, into the debug app's files directory (run-as):
 *   adb push model /data/local/tmp/m && adb shell run-as sh.zeron.android cp -r /data/local/tmp/m files/model
 *   the same into files/clips (the app's private files directory)
 */
@RunWith(AndroidJUnit4::class)
class LocalAsrDeviceTest {
    private val context: Context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private val files get() = context.filesDir

    private fun asr(): LocalAsr {
        val model = LocalModel(File(files, "model"))
        assumeTrue("model not pushed", model.isReady() || File(files, "model/tokenizer.model").exists())
        return LocalAsr(context, model)
    }

    /** 16 kHz mono 16-bit WAV to its PCM bytes. */
    private fun clip(name: String): ByteArray {
        val file = File(files, "clips/$name")
        assumeTrue("clip $name not pushed", file.exists())
        return file.readBytes().copyOfRange(44, file.length().toInt())
    }

    private fun transcribe(asr: LocalAsr, pcm: ByteArray, partials: MutableList<String>? = null): String {
        val session = asr.session()
        try {
            var at = 0
            while (at < pcm.size) {
                val n = minOf(5_120, pcm.size - at)
                session.feed(pcm.copyOfRange(at, at + n), n)?.let { partials?.add(it) }
                at += n
            }
            return session.finish()
        } finally {
            session.close()
        }
    }

    @Test
    fun thePinnedModelFilesDownloadFromHuggingFaceAndVerify() = runBlocking<Unit> {
        // The two small files, through the real URLs (redirects, Range) and the real pins.
        val small = ParakeetUnified.FILES.filter { it.size < 20_000_000 && it.name != "encoder.int8.onnx" }
        val dir = File(context.cacheDir, "pin-check").apply { deleteRecursively() }
        val model = LocalModel(dir, small)
        model.download()
        assertTrue(model.isReady())
        dir.deleteRecursively()
    }

    @Test
    fun streamsPartialsAndDecodesTheFinalTextOffline() {
        val asr = asr()
        assertTrue("the native library loads", asr.usable)
        val partials = mutableListOf<String>()
        val text = transcribe(asr, clip("plain.wav"), partials).lowercase()
        assertTrue("partials while talking: $partials", partials.size >= 3)
        assertTrue("partials grow: $partials", partials.last().length >= partials.first().length)
        assertTrue("final text: $text", text.contains("main branch") && text.contains("tests"))
    }

    @Test
    fun keepsFillerWords() {
        val text = transcribe(asr(), clip("hmm.wav")).lowercase()
        assertTrue("final text: $text", text.contains("um") && text.contains("let me think"))
    }

    @Test
    fun loadsReleasesAndLoadsAgainWithoutCrashing() {
        val asr = asr()
        repeat(3) {
            val text = transcribe(asr, clip("plain.wav")).lowercase()
            assertTrue(text.contains("main branch"))
            asr.release() // the model's sessions are dropped; ONNX Runtime itself stays loaded
        }
    }

    @Test
    fun emptyAndTinyAudioAreHandled() {
        val asr = asr()
        assertEquals("", transcribe(asr, ByteArray(0)))
        transcribe(asr, ByteArray(300)) // under one frame: no crash, whatever it says
    }
}
