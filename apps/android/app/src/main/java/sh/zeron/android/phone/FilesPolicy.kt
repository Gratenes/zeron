package sh.zeron.android.phone

/**
 * The hard floor under "full access to this phone's files": what is never reachable, whatever the switch says.
 * Pure, so it is tested without Android. [path] is already canonical (symlinks and `..` resolved).
 */
object FilesPolicy {
    /** The storage roots a path may be under, given the volume directories that exist (`/storage/emulated/0`, `/storage/1234-ABCD`, …). */
    fun under(path: String, roots: List<String>): String? =
        roots.filter { path == it || path.startsWith("$it/") }.maxByOrNull { it.length }

    /** Null if reachable, else the reason (without a code). */
    fun refusal(path: String, roots: List<String>, privateDirs: List<String>): String? {
        val root = under(path, roots) ?: return "outside shared storage (only /storage/emulated/0 and SD cards)"
        val below = path.removePrefix(root).trim('/').split('/').filter { it.isNotEmpty() }
        if (below.size >= 2 && below[0].equals("Android", ignoreCase = true) && (below[1].equals("data", true) || below[1].equals("obb", true))) {
            return "Android's ${below[0]}/${below[1]} folders are closed to every app"
        }
        if (privateDirs.any { path == it || path.startsWith("$it/") }) return "that is Zeron's own private data"
        return null
    }

    /** `..`, `.` and repeated slashes resolved lexically. */
    fun clean(path: String): String {
        val parts = ArrayList<String>()
        for (p in path.split('/')) when (p) {
            "", "." -> Unit
            ".." -> if (parts.isNotEmpty()) parts.removeAt(parts.lastIndex)
            else -> parts += p
        }
        return "/" + parts.joinToString("/")
    }
}
