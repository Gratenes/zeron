package sh.zeron.android.phone

/**
 * How Google Messages writes a reaction into the message database: an ordinary text row, `👍 to “the original text”`
 * (with zero-width spaces around the emoji), or for tapbacks that came from an iPhone `Loved “…”`. A reader that
 * doesn't know this shows them as messages of their own.
 */
object Reactions {
    class Reaction(val emoji: String, val to: String)

    private val ZW = Regex("[\\u200b\\u200c\\u200d\\u2060\\ufeff]")
    private val QUOTE_OPEN = "[“\"]"
    private val QUOTE_CLOSE = "[”\"]"
    private val EMOJI_TO = Regex("^\\s*([^\\s\\p{L}\\p{N}]{1,16}?)\\s+to\\s+$QUOTE_OPEN\\s*(.*?)\\s*$QUOTE_CLOSE\\s*$", RegexOption.DOT_MATCHES_ALL)
    private val TAPBACK = Regex("^\\s*(Loved|Liked|Disliked|Laughed at|Emphasized|Questioned)\\s+$QUOTE_OPEN\\s*(.*?)\\s*$QUOTE_CLOSE\\s*$", RegexOption.DOT_MATCHES_ALL)
    private val TAPBACK_EMOJI = mapOf("Loved" to "❤️", "Liked" to "👍", "Disliked" to "👎", "Laughed at" to "😂", "Emphasized" to "‼️", "Questioned" to "❓")

    fun parse(text: String): Reaction? {
        val clean = text.replace(ZW, "")
        TAPBACK.matchEntire(clean)?.let { return Reaction(TAPBACK_EMOJI.getValue(it.groupValues[1]), it.groupValues[2]) }
        EMOJI_TO.matchEntire(clean)?.let { return Reaction(it.groupValues[1], it.groupValues[2]) }
        return null
    }

    /** Whether [row] (a message text) is the reaction [emoji] to a message that starts like [target]. */
    fun matches(row: String, emoji: String, target: String): Boolean {
        val r = parse(row) ?: return false
        val a = r.emoji.replace("️", "")
        val b = emoji.replace("️", "")
        val t = target.trim().take(24)
        return a == b && (t.isEmpty() || r.to.trim().startsWith(t) || t.startsWith(r.to.trim().take(24)))
    }
}
