package sh.zeron.android.phone

/** What the finder needs of an accessibility node, so the matching can be tested without Android. */
data class PadNode(val id: String?, val text: String?, val desc: String?, val pkg: String?, val visible: Boolean)

/**
 * Recognises a lock screen's PIN keys. The stock bouncer (and Nothing OS's) names its keys `…:id/key0`…`key9` with
 * the digit as content description. Those are matched by id whether or not the service reports them visible
 * (the keyguard window is layered over another and the flag can be false); a bare digit label is only taken from a
 * visible node of the lock screen's own package.
 */
object PinPad {
    private val KEY_ID = Regex(".*:id/key([0-9])$")

    fun digit(n: PadNode, rootPkg: String?): Char? {
        n.id?.let { id -> KEY_ID.matchEntire(id)?.groupValues?.get(1)?.get(0)?.let { return it } }
        val label = (n.text?.takeIf { it.isNotEmpty() } ?: n.desc)?.takeIf { it.length == 1 && it[0].isDigit() } ?: return null
        return if (n.visible && n.pkg != null && n.pkg == rootPkg) label[0] else null
    }

    fun isEnter(n: PadNode): Boolean {
        val label = (n.desc ?: n.text).orEmpty()
        return (n.id.orEmpty().endsWith(":id/key_enter") || label.equals("Enter", true) || label.equals("OK", true)) && n.visible
    }
}
