package sh.zeron.android.phone

/**
 * What Google Messages' compose screen says about how a message will go: its send button ("Send RCS message",
 * "Send SMS message", "Send message") and the compose box's hint ("RCS message", "Text message", "Chat message")
 * name the transport it will use for this conversation. Pure, so it can be tested.
 */
object MessagesUi {
    /** `rcs`, `sms`, or `unknown` when the screen doesn't say. */
    fun transportOf(vararg labels: String?): String {
        val text = labels.filterNotNull().joinToString(" ").lowercase()
        return when {
            Regex("\\b(rcs|chat|chatting)\\b").containsMatchIn(text) -> "rcs"
            Regex("\\b(sms|mms|text message|text)\\b").containsMatchIn(text) -> "sms"
            else -> "unknown"
        }
    }

    /** The compose screen's header line ("Texting with (555) … (SMS/MMS)", "Chatting with …"). */
    fun isComposeHeader(label: String?): Boolean = label?.trim()?.lowercase()?.let { Regex("^(texting|chatting|messaging) with ").containsMatchIn(it) } == true

    /** A label that is the compose screen's own send button (not "Send attachment", "Send money", …). */
    fun isSendButton(label: String?): Boolean {
        val l = label?.trim()?.lowercase() ?: return false
        return Regex("^send( (rcs|sms|mms|chat|text))? ?(message)?$").matches(l)
    }

    /**
     * Whether Messages' compose screen shows that the files were attached: any label that names a file, or says
     * image, photo, video, audio, file or attachment. Sending without this would send the caption alone.
     */
    fun showsAttachment(labels: List<String>, fileNames: List<String>): Boolean {
        val all = labels.map { it.lowercase() }
        if (fileNames.any { n -> n.isNotBlank() && all.any { it.contains(n.lowercase()) } }) return true
        return all.any { Regex("\\b(attachment|attached|image|photo|picture|video|audio|file|remove)\\b").containsMatchIn(it) && !it.startsWith("send ") && !it.startsWith("add ") && !it.contains("attach files") }
    }
}
