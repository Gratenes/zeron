package sh.zeron.android.phone

/**
 * Reads a voicemail out of a Google Voice or Google Phone notification. The shapes are guessed from what these
 * apps are known to post ("New voicemail from X", "Voicemail from X", a body that is the transcript, or a
 * MessagingStyle whose messages are the transcript), so [looksLikeVoicemail] is deliberately a little loose and the
 * raw pieces are kept in the log for checking against a real one. Pure, so it is tested without Android.
 */
object VoicemailParse {
    val PACKAGES = setOf("com.google.android.apps.googlevoice", "com.google.android.dialer", "com.android.dialer", "com.android.server.telecom")
    private val GOOGLE_VOICE = "com.google.android.apps.googlevoice"

    class Voicemail(val caller: String, val transcript: String, val source: String)

    private val FROM = Regex("(?i)\\bvoice ?mail(?: message)?(?: from| by)?\\s*[:\\-–]?\\s*(.+)$")
    private val NEW_FROM = Regex("(?i)^(?:new )?voice ?mail(?: message)?\\s*(?:from|by)\\s+(.+)$")
    private val TRANSCRIBING = Regex("(?i)^(transcribing|transcription (is )?(unavailable|not available)|no transcript|tap to (listen|play)|play voicemail)")

    /** A notification from one of the voicemail apps that is about a voicemail. */
    fun looksLikeVoicemail(pkg: String, channelId: String?, title: String?, text: String?, category: String?): Boolean {
        if (pkg !in PACKAGES) return false
        val all = listOfNotNull(channelId, title, text).joinToString(" ").lowercase()
        if (Regex("voice ?mail").containsMatchIn(all)) return true
        // Google Voice's own message/missed-call notifications are not voicemails.
        return false
    }

    /**
     * [title] and [text] are the notification's, [bigText] the expanded body, [messages] a MessagingStyle's texts
     * (oldest first). The caller is whatever follows "voicemail from"; the transcript is the longest body that is not
     * a header, a "transcribing" placeholder or a button label.
     */
    fun parse(pkg: String, title: String?, text: String?, bigText: String?, messages: List<String>, subText: String?): Voicemail? {
        if (!looksLikeVoicemail(pkg, null, title, listOfNotNull(text, bigText, subText).joinToString(" "), null)) return null
        val t = title?.trim().orEmpty()
        var caller = NEW_FROM.find(t)?.groupValues?.get(1)?.trim()
            ?: FROM.find(t)?.groupValues?.get(1)?.trim()?.takeIf { it.isNotEmpty() && !it.equals("voicemail", true) }
        val bodies = (messages + listOfNotNull(bigText, text)).map { it.trim() }.filter { it.isNotEmpty() }
        if (caller == null) {
            // "New voicemail" as the title: the caller is the first line of the body ("from X" or just X).
            val first = bodies.firstOrNull()?.lines()?.firstOrNull()?.trim()
            caller = first?.let { Regex("(?i)^(?:from|by)\\s+(.+)$").find(it)?.groupValues?.get(1)?.trim() ?: it.takeIf { l -> l.length <= 60 && !Regex("(?i)voice ?mail").containsMatchIn(l) } }
        }
        val transcript = bodies.map { stripHeader(it, caller) }
            .filter { it.isNotEmpty() && !TRANSCRIBING.containsMatchIn(it) && !Regex("(?i)^(new )?voice ?mail").containsMatchIn(it) }
            .maxByOrNull { it.length }.orEmpty()
        val source = if (pkg == GOOGLE_VOICE) "Google Voice" else "Google Phone"
        return Voicemail(caller?.takeIf { it.isNotEmpty() } ?: "unknown caller", transcript, source)
    }

    /** A body may start with the caller's name/number line; the transcript is what follows it. */
    private fun stripHeader(body: String, caller: String?): String {
        var lines = body.lines().map { it.trim() }.filter { it.isNotEmpty() }
        if (caller != null && lines.size > 1 && (lines.first().equals(caller, true) || lines.first().removePrefix("from ").equals(caller, true))) lines = lines.drop(1)
        return lines.joinToString("\n")
    }
}
