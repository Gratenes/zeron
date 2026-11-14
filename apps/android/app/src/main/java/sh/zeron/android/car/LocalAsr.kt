package sh.zeron.android.car

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.system.Os
import android.util.Log
import java.io.File

/** Which recognizer the car dictates with. */
enum class DictationEngine {
    /** On-device Parakeet (once its model is downloaded). */
    Local,
    /** The phone's Android / Google recognizer. */
    Android,
}

/**
 * The engine to listen with: the chosen one when it can run, else Android's, which
 * is always the fallback (no model yet, the library failed to load, an error).
 */
fun engineFor(chosen: DictationEngine, modelReady: Boolean, localUsable: Boolean): DictationEngine =
    if (chosen == DictationEngine.Local && modelReady && localUsable) DictationEngine.Local else DictationEngine.Android

/**
 * On-device dictation: the downloaded [LocalModel] loaded into the native
 * recognizer (`libzeron_local_asr.so`, which loads the ONNX Runtime the app ships)
 * on first use, kept for [IDLE_MS] after the last listen, then released (about
 * 0.9 GB). Thread-safe; [session] blocks for the load (about 2 s the first time).
 */
class LocalAsr(context: Context, val model: LocalModel) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val lock = Any()
    private var handle = 0L
    private var open = 0
    private var failed = false
    private val unload = Runnable { releaseIfIdle() }

    /** False once the native library failed to load (this device or ABI can't run it). */
    val usable: Boolean get() = !failed && runCatching { ensureLibrary() }.isSuccess

    /** Load the model ahead of the first utterance (a car connecting), off the main thread. */
    fun warm() {
        if (!model.isReady()) return
        Thread({ runCatching { synchronized(lock) { ensureLoaded() }; scheduleUnload() } }, "asr-warm").start()
    }

    /** A new utterance; throws if the model can't load. */
    fun session(): AsrSession = synchronized(lock) {
        main.removeCallbacks(unload)
        val model = ensureLoaded()
        val session = LocalAsrNative.open(model)
        open++
        object : AsrSession {
            private var closed = false
            override fun feed(pcm: ByteArray, length: Int): String? = LocalAsrNative.feed(session, pcm, length)
            override fun finish(): String = LocalAsrNative.finish(session)
            override fun close() = synchronized(lock) {
                if (closed) return@synchronized
                closed = true
                LocalAsrNative.close(session)
                open--
                scheduleUnload()
            }
        }
    }

    private fun scheduleUnload() {
        main.removeCallbacks(unload)
        main.postDelayed(unload, IDLE_MS)
    }

    private fun releaseIfIdle() = synchronized(lock) {
        if (open == 0 && handle != 0L) {
            LocalAsrNative.unload(handle)
            handle = 0
            Log.i(TAG, "released the dictation model")
        }
    }

    /** Release the model now (it was removed, or the engine switched away). */
    fun release() = synchronized(lock) {
        main.removeCallbacks(unload)
        if (open == 0) releaseIfIdle()
    }

    private fun ensureLibrary() {
        if (loaded) return
        try {
            // The recognizer loads ONNX Runtime by path, from the library the app ships.
            Os.setenv("ORT_DYLIB_PATH", File(context.applicationInfo.nativeLibraryDir, "libonnxruntime.so").path, true)
            System.loadLibrary("zeron_local_asr")
            loaded = true
        } catch (e: Throwable) {
            failed = true
            Log.w(TAG, "local dictation unavailable", e)
            throw e
        }
    }

    private fun ensureLoaded(): Long {
        ensureLibrary()
        if (handle == 0L) {
            val started = System.nanoTime()
            handle = LocalAsrNative.load(model.dir.path, THREADS)
            Log.i(TAG, "loaded the dictation model in ${(System.nanoTime() - started) / 1_000_000} ms")
        }
        return handle
    }

    companion object {
        private const val TAG = "ZeronCar"
        const val THREADS = 4
        const val IDLE_MS = 5 * 60_000L
        @Volatile private var loaded = false
    }
}

/** The JNI surface (`jni_bridge.rs`). Methods throw IllegalStateException on failure. */
internal object LocalAsrNative {
    @JvmStatic external fun load(dir: String, threads: Int): Long
    @JvmStatic external fun unload(model: Long)
    @JvmStatic external fun open(model: Long): Long
    @JvmStatic external fun close(session: Long)
    @JvmStatic external fun feed(session: Long, pcm: ByteArray, length: Int): String?
    @JvmStatic external fun finish(session: Long): String
}
