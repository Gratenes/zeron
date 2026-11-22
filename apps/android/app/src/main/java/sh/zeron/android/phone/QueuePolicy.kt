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

    private fun digits(s: String) = s.filter { it.isDigit() }.takeLast(10)

    /** The same person and the same text (case and surrounding space aside): a second request is the first one again. */
    fun sameRequest(numberA: String, textA: String, numberB: String, textB: String): Boolean =
        digits(numberA).isNotEmpty() && digits(numberA) == digits(numberB) && textA.trim().equals(textB.trim(), ignoreCase = true)

    /** What an agent is told to do after each outcome (null when there is nothing to add). */
    fun advice(state: String): String? = when (state) {
        "failed", "expired" -> "Do NOT resend: the text may have been sent by hand or may still go out. Ask the user first."
        "superseded" -> "Nothing to do: the same text was already sent from the phone."
        "cancelled", "handed" -> "Do not resend unless the user asks."
        "queued", "sending" -> "Not sent yet. Do not resend; cancel it with phone_sms_queue {cancel: true} if it is no longer wanted."
        else -> null
    }
}
