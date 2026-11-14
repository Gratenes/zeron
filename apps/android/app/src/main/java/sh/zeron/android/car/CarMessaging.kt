package sh.zeron.android.car

/** What a report says to Assistant as a message, and what a voice reply from it means. */
internal object CarMessaging {
    /** The key Android Auto puts the driver's dictated reply under. */
    const val REPLY_KEY = "reply_input"

    /** The text Assistant reads aloud for [report]: the reply, the question asked, or the failure. */
    fun body(report: Report): String = when (report.kind) {
        ReportKind.Finished -> Spoken.summary(report.text, MAX_BODY, wholeText = true).ifEmpty { "Done." }
        ReportKind.NeedsInput ->
            report.spoken.removePrefix("${report.title} needs you.").trim()
                .ifEmpty { Spoken.summary(report.text, MAX_BODY, wholeText = true) }
                .ifEmpty { "Needs your answer." }
        ReportKind.Failed -> "Ran into an error."
    }

    /** What the driver dictated, or null for nothing worth sending. */
    fun dictated(text: CharSequence?): String? = text?.toString()?.trim()?.takeIf { it.isNotEmpty() }

    sealed interface Result {
        data class Sent(val chatId: String) : Result
        /** Nothing was dictated, or no chat to send it to. */
        data object Nothing : Result
        data class Failed(val message: String) : Result
    }

    /** A voice reply to [chatId]: sent through [send], and the outcome in words for the driver. */
    suspend fun reply(chatId: String?, text: CharSequence?, send: suspend (String, String) -> CarAgents.Outcome): Result {
        val said = dictated(text)
        if (chatId.isNullOrEmpty() || said == null) return Result.Nothing
        return when (val outcome = send(chatId, said)) {
            is CarAgents.Outcome.Sent -> Result.Sent(chatId)
            CarAgents.Outcome.NoHost -> Result.Failed("None of your computers is online.")
            CarAgents.Outcome.SignedOut -> Result.Failed("Sign in to Zeron on your phone first.")
            is CarAgents.Outcome.Failed -> Result.Failed(outcome.message)
        }
    }

    /** Long enough that Assistant reads a whole reply; the car shows only its start. */
    private const val MAX_BODY = 1_000
}
