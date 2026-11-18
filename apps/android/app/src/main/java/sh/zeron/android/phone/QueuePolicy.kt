package sh.zeron.android.phone

/** When a queued text is tried: pure, so the waiting rules are tested without a phone. */
object QueuePolicy {
    enum class Decision { WAIT, GO, EXPIRED }

    /** Expired after [lifetimeMs]; otherwise go as soon as the person is not using the phone, else wait. */
    fun decide(now: Long, created: Long, personActive: Boolean, lifetimeMs: Long): Decision = when {
        now - created > lifetimeMs -> Decision.EXPIRED
        personActive -> Decision.WAIT
        else -> Decision.GO
    }

    /** What a retry that threw means: busy → back to waiting; anything else → the text has failed. */
    fun afterAttempt(busy: Boolean): String = if (busy) "queued" else "failed"
}
