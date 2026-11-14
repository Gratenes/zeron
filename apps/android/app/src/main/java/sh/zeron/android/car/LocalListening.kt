package sh.zeron.android.car

/** One utterance of the on-device recognizer: audio in, partial text out, then the final text. */
interface AsrSession : AutoCloseable {
    /** Feed [length] bytes of 16 kHz mono 16-bit PCM; the whole partial text so far if it changed. */
    fun feed(pcm: ByteArray, length: Int): String?

    /** The final text, the whole utterance decoded again offline. */
    fun finish(): String
}

/** How a local listen ended. */
sealed interface LocalResult {
    /** Words (possibly none: [text] blank means nothing was said). */
    data class Text(val text: String) : LocalResult
    /** The driver cancelled. */
    data object Cancelled : LocalResult
    /** The recognizer failed; [partial] is whatever it had heard. */
    data class Failed(val partial: String?, val error: Throwable) : LocalResult
}

/**
 * Pumps one microphone stream through an [AsrSession] until the [Endpointer] says
 * the utterance is over (or the stream ends, or [cancelled]): streaming partials
 * go to [onPartial] as they change, and the final text is the offline re-decode.
 * Blocking; run it off the main thread.
 */
class LocalListening(
    private val session: AsrSession,
    private val endpointer: Endpointer = Endpointer(),
    /** Audio is handed to the recognizer in steps this long: a JNI call per 16 ms buffer would be wasteful. */
    private val stepBytes: Int = STEP_BYTES,
    /** A guard like the Android path's: the end of speech ends a normal utterance, this a stuck one. */
    private val limitBytes: Long = LIMIT_BYTES,
) {
    fun run(stream: AudioStream, onPartial: (String) -> Unit, cancelled: () -> Boolean): LocalResult {
        var lastPartial: String? = null
        try {
            val read = ByteArray(BUFFER)
            val step = ByteArray(stepBytes + BUFFER)
            var filled = 0
            var total = 0L
            while (!cancelled() && total < limitBytes) {
                val n = stream.read(read, 0, read.size)
                if (n < 0) break
                if (n > 0) {
                    total += n
                    System.arraycopy(read, 0, step, filled, n)
                    filled += n
                    if (filled >= stepBytes) {
                        session.feed(step, filled)?.let {
                            lastPartial = it
                            onPartial(it)
                        }
                        filled = 0
                    }
                }
                if (endpointer.feed(read, n.coerceAtLeast(0))) break
            }
            if (cancelled()) return LocalResult.Cancelled
            if (filled > 0) session.feed(step, filled)?.let { lastPartial = it }
            return LocalResult.Text(session.finish().ifBlank { lastPartial.orEmpty() })
        } catch (e: Exception) {
            return LocalResult.Failed(lastPartial, e)
        } finally {
            stream.stop()
        }
    }

    companion object {
        const val BUFFER = 512
        /** 160 ms of 16 kHz 16-bit mono. */
        const val STEP_BYTES = 5_120
        /** 90 s of 16 kHz 16-bit mono. */
        const val LIMIT_BYTES = 90L * 32_000
    }
}
