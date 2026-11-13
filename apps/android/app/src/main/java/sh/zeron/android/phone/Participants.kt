package sh.zeron.android.phone

/**
 * Who the other people in a conversation are, from one message's address rows. Google Messages writes RCS
 * chats with the person's own number among the addresses (as a recipient of what they received, as the
 * sender of what they sent), so that number must not count as a participant.
 */
object Participants {
    /** Same number however written: the last ten digits agree (or all digits, if shorter). */
    fun same(a: String, b: String): Boolean {
        val x = a.filter { it.isDigit() }.takeLast(10)
        val y = b.filter { it.isDigit() }.takeLast(10)
        return x.isNotEmpty() && x == y
    }

    /**
     * [from] is the sender row, [to] the recipient rows, [self] the phone's own numbers when known.
     * A received message names its sender plus any other recipients; a sent one names its recipients.
     * With no known own number, a received message to exactly one recipient is to the phone itself.
     */
    fun others(incoming: Boolean, from: String?, to: List<String>, self: List<String>): List<String> {
        fun mine(n: String) = self.any { same(it, n) }
        val recipients = to.filter { it.isNotBlank() }
        val result = if (incoming) {
            val rest = if (self.isEmpty() && recipients.size == 1) emptyList() else recipients.filter { !mine(it) }
            listOfNotNull(from?.takeIf { it.isNotBlank() && !mine(it) }) + rest
        } else {
            recipients.filter { !mine(it) }
        }
        val distinct = ArrayList<String>()
        for (n in result) if (distinct.none { same(it, n) }) distinct += n
        return distinct
    }
}
