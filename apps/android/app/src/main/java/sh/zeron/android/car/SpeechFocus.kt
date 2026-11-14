package sh.zeron.android.car

/**
 * Audio focus for speaking, held across a whole readback: taken when the first
 * piece is spoken, kept through the gaps between pieces (a slow network voice,
 * a pause between replies), and given back once, a moment after the last one.
 * Abandoning between pieces is what let the music climb back to full volume
 * halfway through a reply. Main thread.
 */
class SpeechFocus<T : Any>(
    private val acquire: (pause: Boolean) -> T?,
    private val abandon: (T) -> Unit,
    /** Run [block] after [delayMs]; the result cancels it. */
    private val later: (delayMs: Long, block: () -> Unit) -> () -> Unit,
    private val holdOverMs: Long = HOLD_OVER_MS,
) {
    private var held: T? = null
    private var heldPause = false
    private var cancelRelease: (() -> Unit)? = null

    val isHeld: Boolean get() = held != null

    /** Speaking begins (or goes on): focus is ours, and stays so. */
    fun hold(pause: Boolean) {
        cancelRelease?.invoke()
        cancelRelease = null
        if (held != null && heldPause == pause) return
        held?.let(abandon)
        held = acquire(pause)
        heldPause = pause
    }

    /** Nothing is being said right now; give focus back soon unless speech resumes. */
    fun settle() {
        if (held == null) return
        cancelRelease?.invoke()
        cancelRelease = later(holdOverMs) {
            cancelRelease = null
            drop()
        }
    }

    /** Give focus back now (speech stopped, focus lost, listening starts). */
    fun drop() {
        cancelRelease?.invoke()
        cancelRelease = null
        held?.let(abandon)
        held = null
    }

    companion object {
        /** Long enough to bridge the gap between two replies read back to back. */
        const val HOLD_OVER_MS = 2_500L
    }
}

/** Long text as pieces the engine can say one after another, each ending at a sentence or word. */
internal fun speechPieces(text: String, max: Int = 240): List<String> {
    val pieces = ArrayList<String>()
    var rest = text.trim()
    while (rest.length > max) {
        val window = rest.substring(0, max)
        var cut = window.lastIndexOfAny(charArrayOf('.', '!', '?', '\n', ';', ':')).let { if (it >= max / 3) it + 1 else -1 }
        if (cut < 0) cut = window.lastIndexOf(' ').let { if (it > 0) it else max }
        pieces += rest.substring(0, cut).trim()
        rest = rest.substring(cut).trim()
    }
    if (rest.isNotEmpty()) pieces += rest
    return pieces.filter { it.isNotEmpty() }
}
