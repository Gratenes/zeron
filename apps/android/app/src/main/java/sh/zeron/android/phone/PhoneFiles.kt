package sh.zeron.android.phone

import android.content.Context
import android.net.Uri
import android.os.Environment
import android.os.UserManager
import android.provider.DocumentsContract
import android.util.Base64
import androidx.documentfile.provider.DocumentFile
import java.io.File
import java.io.InputStream
import java.io.RandomAccessFile
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import org.json.JSONArray
import org.json.JSONObject

/**
 * Files, for the person's allowed folders only ([PhoneFolders]): list, stat, hash, read in chunks, and write in
 * chunks (to a `<name>.zeron-part` next to the target, renamed when the size and checksum match). Paths are absolute
 * strings. With "All files access" the folders are read as plain files; without it, through the tree the person
 * picked (the same path strings come back). Errors name their code first: files.no_grant, files.denied,
 * files.not_found, files.exists, files.too_large, files.bad_offset, files.unavailable.
 */
class PhoneFiles(private val context: Context) {
    private val settings = PhoneSettings(context)

    private fun fail(code: String, message: String): Nothing = throw PhoneToolException("$code: $message")

    /** An entry or a root: one file or directory. */
    private class Entry(val name: String, val path: String, val dir: Boolean, val size: Long, val mtime: Long)

    // ── resolving a path inside a granted folder ────────────────────────────

    /** A granted folder and the way it is read: plain files, or through its picked tree. */
    private class Root(val folder: PhoneFolders.Folder, val tree: DocumentFile?)

    private fun allFilesAccess(): Boolean = android.os.Build.VERSION.SDK_INT >= 30 && Environment.isExternalStorageManager()

    private fun roots(): List<Root> {
        if (!context.getSystemService(UserManager::class.java).isUserUnlocked) fail("files.unavailable", "the phone's storage isn't unlocked yet: the person has to unlock it once after a restart")
        val folders = PhoneFolders.all(context)
        if (folders.isEmpty()) fail("files.no_grant", "no folder is allowed yet: the person adds folders in Zeron → Settings → Agent phone access → Files")
        return folders.map { f ->
            val tree = if (!allFilesAccess() && f.uri != null) DocumentFile.fromTreeUri(context, Uri.parse(f.uri)) else null
            Root(f, tree)
        }
    }

    private fun grantedList(): String = PhoneFolders.all(context).joinToString(", ") { it.path }

    /** The root a path is inside (longest match, after making it canonical), and the path's segments below it. */
    private fun locate(path: String): Pair<Root, List<String>> {
        if (!path.startsWith("/")) fail("files.denied", "paths are absolute (start with /)")
        val roots = roots()
        val canonical = canonicalize(path)
        val hit = roots.filter { canonical == it.folder.path || canonical.startsWith(it.folder.path + "/") }.maxByOrNull { it.folder.path.length }
            ?: fail("files.no_grant", "$path is not inside an allowed folder (allowed: ${grantedList()})")
        val below = canonical.removePrefix(hit.folder.path).split('/').filter { it.isNotEmpty() }
        return hit to below
    }

    /** `..`, `.` and repeated slashes resolved; for real files also symlinks (so a link can't leave the folder). */
    private fun canonicalize(path: String): String {
        val parts = ArrayList<String>()
        for (p in path.split('/')) when (p) {
            "", "." -> Unit
            ".." -> if (parts.isNotEmpty()) parts.removeAt(parts.lastIndex)
            else -> parts += p
        }
        val lexical = "/" + parts.joinToString("/")
        return if (allFilesAccess()) runCatching { File(lexical).canonicalPath }.getOrDefault(lexical) else lexical
    }

    private fun requireWrite(root: Root) {
        if (!root.folder.write) fail("files.denied", "${root.folder.path} is read-only: the person has to allow writing there in Zeron → Settings → Agent phone access → Files")
    }

    // ── the backends (files, or a picked tree) ──────────────────────────────

    private fun fileOf(root: Root, below: List<String>) = File(root.folder.path + below.joinToString("") { "/$it" })

    private fun docOf(root: Root, below: List<String>): DocumentFile? {
        var d = root.tree ?: return null
        for (seg in below) d = d.findFile(seg) ?: return null
        return d
    }

    private fun entryOf(root: Root, below: List<String>): Entry {
        val path = root.folder.path + below.joinToString("") { "/$it" }
        if (root.tree == null) {
            val f = fileOf(root, below)
            if (!f.exists()) fail("files.not_found", path)
            return Entry(f.name, path, f.isDirectory, if (f.isFile) f.length() else 0, f.lastModified())
        }
        val d = docOf(root, below) ?: fail("files.not_found", path)
        return Entry(d.name ?: below.lastOrNull() ?: root.folder.path.substringAfterLast('/'), path, d.isDirectory, if (d.isFile) d.length() else 0, d.lastModified())
    }

    private fun children(root: Root, below: List<String>): List<Entry> {
        val base = root.folder.path + below.joinToString("") { "/$it" }
        if (root.tree == null) {
            val f = fileOf(root, below)
            if (!f.exists()) fail("files.not_found", base)
            if (!f.isDirectory) fail("files.denied", "$base is not a folder")
            return (f.listFiles() ?: fail("files.denied", "$base can't be listed")).map { Entry(it.name, "$base/${it.name}", it.isDirectory, if (it.isFile) it.length() else 0, it.lastModified()) }
        }
        val d = docOf(root, below) ?: fail("files.not_found", base)
        if (!d.isDirectory) fail("files.denied", "$base is not a folder")
        return d.listFiles().map { Entry(it.name ?: "?", "$base/${it.name}", it.isDirectory, if (it.isFile) it.length() else 0, it.lastModified()) }
    }

    private fun openRead(root: Root, below: List<String>, offset: Long): InputStream {
        val path = root.folder.path + below.joinToString("") { "/$it" }
        if (root.tree == null) {
            val f = fileOf(root, below)
            if (!f.isFile) fail("files.not_found", path)
            return RandomAccessFile(f, "r").also { it.seek(offset) }.let { raf -> object : InputStream() {
                override fun read(): Int = raf.read()
                override fun read(b: ByteArray, off: Int, len: Int): Int = raf.read(b, off, len)
                override fun close() = raf.close()
            } }
        }
        val d = docOf(root, below)?.takeIf { it.isFile } ?: fail("files.not_found", path)
        val input = context.contentResolver.openInputStream(d.uri) ?: fail("files.denied", "$path can't be opened")
        var skipped = 0L
        while (skipped < offset) { val s = input.skip(offset - skipped); if (s <= 0) break; skipped += s }
        return input
    }

    // ── the tools ───────────────────────────────────────────────────────────

    fun list(path: String, cursor: String?): JSONObject {
        if (path.isEmpty() || path == "/") {
            val roots = roots()
            val out = JSONArray()
            roots.forEach { r -> out.put(JSONObject().put("name", r.folder.path.substringAfterLast('/').ifEmpty { "/" }).put("path", r.folder.path).put("kind", "dir").put("writable", r.folder.write)) }
            return JSONObject().put("entries", out).put("allFilesAccess", allFilesAccess())
        }
        val (root, below) = locate(path)
        val all = children(root, below).sortedBy { it.name }
        val page = all.filter { cursor == null || it.name > cursor }.take(PAGE)
        val out = JSONArray()
        page.forEach { out.put(entryJson(it)) }
        val more = page.size == PAGE && all.any { it.name > page.last().name }
        return JSONObject().put("entries", out).apply { if (more) put("cursor", page.last().name) }
    }

    fun stat(path: String): JSONObject {
        val (root, below) = locate(path)
        return entryJson(entryOf(root, below))
    }

    fun hash(path: String): JSONObject {
        val (root, below) = locate(path)
        val e = entryOf(root, below)
        if (e.dir) fail("files.denied", "${e.path} is a folder")
        val digest = MessageDigest.getInstance("SHA-256")
        var n = 0L
        openRead(root, below, 0).use { input ->
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val r = input.read(buffer)
                if (r < 0) break
                digest.update(buffer, 0, r)
                n += r
            }
        }
        return JSONObject().put("sha256", digest.digest().joinToString("") { "%02x".format(it) }).put("size", n).put("path", e.path)
    }

    fun read(path: String, offset: Long, len: Int): JSONObject {
        if (len < 1 || len > CHUNK) fail("files.too_large", "len is 1 to $CHUNK bytes")
        if (offset < 0) fail("files.bad_offset", "offset is negative")
        val (root, below) = locate(path)
        val e = entryOf(root, below)
        if (e.dir) fail("files.denied", "${e.path} is a folder")
        val buffer = ByteArray(len)
        var got = 0
        openRead(root, below, offset).use { input ->
            while (got < len) {
                val r = input.read(buffer, got, len - got)
                if (r < 0) break
                got += r
            }
        }
        val next = offset + got
        return JSONObject().put("data", Base64.encodeToString(buffer, 0, got, Base64.NO_WRAP)).put("size", e.size).put("mtime", e.mtime)
            .put("nextOffset", next).put("done", next >= e.size)
    }

    // Writes in progress: transfer id → the part file's path, so a transfer can't be pointed at two targets.
    private val transfers = ConcurrentHashMap<String, String>()

    /**
     * One chunk of a file written to [path]: the first (offset 0) starts `<path>.zeron-part` (refused if the target
     * exists, unless [overwrite]); each next one must start where the held bytes end (else files.bad_offset names how
     * many are held, so the sender can resume); the final one checks the size and [sha256] and renames.
     */
    fun writePart(transferId: String, path: String, offset: Long, data: ByteArray, final: Boolean, sha256: String?, overwrite: Boolean, confirm: (() -> Boolean)?): JSONObject {
        if (data.size > CHUNK) fail("files.too_large", "a chunk is at most $CHUNK bytes")
        if (transferId.isBlank()) fail("files.denied", "transferId is empty")
        val (root, below) = locate(path)
        requireWrite(root)
        if (below.isEmpty()) fail("files.denied", "that is the folder itself")
        val name = below.last()
        val parent = below.dropLast(1)
        val partName = "$name.zeron-part"
        val target = entryOrNull(root, below)
        if (target?.dir == true) fail("files.denied", "${target.path} is a folder")
        val existing = transfers[transferId]
        if (existing != null && existing != path) fail("files.denied", "transfer $transferId is already writing $existing")
        if (offset == 0L && existing == null) {
            if (target != null && !overwrite) fail("files.exists", target.path)
            if (confirm != null && !confirm()) fail("files.denied", "the person declined to let a file be written to $path")
            transfers[transferId] = path
        }
        val held = partSize(root, parent, partName)
        if (offset != held) fail("files.bad_offset", "the phone holds $held bytes of $path (send from offset $held)")
        appendPart(root, parent, partName, data, create = offset == 0L)
        val received = held + data.size
        if (!final) return JSONObject().put("received", received)
        if (sha256 != null) {
            val actual = sha256Of(root, parent, partName)
            if (!actual.equals(sha256, ignoreCase = true)) {
                transfers.remove(transferId)
                deletePart(root, parent, partName)
                fail("files.denied", "the checksum of what arrived ($actual) is not the one given: the partial file was discarded")
            }
        }
        finishPart(root, parent, partName, name, overwrite)
        transfers.remove(transferId)
        return JSONObject().put("received", received).put("final", true).put("path", path)
    }

    private fun entryOrNull(root: Root, below: List<String>): Entry? = try { entryOf(root, below) } catch (_: PhoneToolException) { null }

    private fun partSize(root: Root, parent: List<String>, partName: String): Long {
        if (root.tree == null) return fileOf(root, parent + partName).takeIf { it.isFile }?.length() ?: 0
        return docOf(root, parent + partName)?.takeIf { it.isFile }?.length() ?: 0
    }

    private fun appendPart(root: Root, parent: List<String>, partName: String, data: ByteArray, create: Boolean) {
        if (root.tree == null) {
            val dir = fileOf(root, parent)
            if (!dir.isDirectory) fail("files.not_found", dir.path)
            val part = File(dir, partName)
            if (create && part.exists()) part.delete()
            java.io.FileOutputStream(part, true).use { it.write(data) }
            return
        }
        val dir = docOf(root, parent)?.takeIf { it.isDirectory } ?: fail("files.not_found", root.folder.path + parent.joinToString("") { "/$it" })
        var part = dir.findFile(partName)
        if (create && part != null) { part.delete(); part = null }
        if (part == null) part = dir.createFile("application/octet-stream", partName) ?: fail("files.denied", "couldn't create $partName")
        context.contentResolver.openOutputStream(part.uri, "wa")?.use { it.write(data) } ?: fail("files.denied", "couldn't write $partName")
    }

    private fun sha256Of(root: Root, parent: List<String>, partName: String): String {
        val digest = MessageDigest.getInstance("SHA-256")
        openRead(root, parent + partName, 0).use { input ->
            val buffer = ByteArray(64 * 1024)
            while (true) { val r = input.read(buffer); if (r < 0) break; digest.update(buffer, 0, r) }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    private fun deletePart(root: Root, parent: List<String>, partName: String) {
        if (root.tree == null) File(fileOf(root, parent), partName).delete() else docOf(root, parent + partName)?.delete()
    }

    private fun finishPart(root: Root, parent: List<String>, partName: String, name: String, overwrite: Boolean) {
        if (root.tree == null) {
            val dir = fileOf(root, parent)
            val target = File(dir, name)
            if (target.exists() && overwrite) target.delete()
            if (!File(dir, partName).renameTo(target)) fail("files.denied", "couldn't move the finished file into place")
            return
        }
        val dir = docOf(root, parent) ?: fail("files.not_found", root.folder.path)
        dir.findFile(name)?.let { if (overwrite) it.delete() else fail("files.exists", name) }
        val part = dir.findFile(partName) ?: fail("files.not_found", partName)
        if (!part.renameTo(name)) fail("files.denied", "couldn't move the finished file into place")
    }

    private fun entryJson(e: Entry): JSONObject =
        JSONObject().put("name", e.name).put("path", e.path).put("kind", if (e.dir) "dir" else "file").put("size", e.size).put("mtime", e.mtime)

    companion object {
        const val CHUNK = 524_288
        const val PAGE = 500
    }
}
