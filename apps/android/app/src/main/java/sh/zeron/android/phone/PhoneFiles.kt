package sh.zeron.android.phone

import android.content.Context
import android.os.Environment
import android.os.UserManager
import android.util.Base64
import java.io.File
import java.io.InputStream
import java.io.RandomAccessFile
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import org.json.JSONArray
import org.json.JSONObject

/**
 * Files in shared storage, behind one switch ("Full access to this phone's files") and Android's All files access:
 * list, stat, hash, read in chunks, and write in chunks (to a `<name>.zeron-part` next to the target, renamed when the
 * size and checksum match; missing folders are created). The floor ([FilesPolicy]): only shared storage (internal
 * and SD cards), never Android/data or obb, never Zeron's own private data. Errors name their code first:
 * files.no_grant, files.denied, files.not_found, files.exists, files.too_large, files.bad_offset, files.unavailable.
 */
class PhoneFiles(private val context: Context) {
    private val settings = PhoneSettings(context)

    private fun fail(code: String, message: String): Nothing = throw PhoneToolException("$code: $message")

    /** An entry or a root: one file or directory. */
    private class Entry(val name: String, val path: String, val dir: Boolean, val size: Long, val mtime: Long)

    private fun allFilesAccess(): Boolean = android.os.Build.VERSION.SDK_INT >= 30 && Environment.isExternalStorageManager()

    /** Internal storage and every other volume (SD cards) that is mounted. */
    private fun storageRoots(): List<String> {
        val out = ArrayList<String>()
        out += "/storage/emulated/0"
        File("/storage").listFiles()?.forEach { v ->
            if (v.isDirectory && v.name != "emulated" && v.name != "self" && v.canRead()) out += v.path
        }
        return out
    }

    private fun privateDirs(): List<String> = listOfNotNull(
        runCatching { context.dataDir.canonicalPath }.getOrNull(),
        runCatching { context.filesDir.canonicalPath }.getOrNull(),
        runCatching { context.cacheDir.canonicalPath }.getOrNull(),
        runCatching { context.createDeviceProtectedStorageContext().dataDir.canonicalPath }.getOrNull(),
    )

    /** The canonical, allowed [File] for [path]; throws files.* otherwise. */
    private fun locate(path: String): File {
        if (!path.startsWith("/")) fail("files.denied", "paths are absolute (start with /)")
        if (!context.getSystemService(UserManager::class.java).isUserUnlocked) fail("files.unavailable", "the phone's storage isn't unlocked yet: the person has to unlock it once after a restart")
        if (!allFilesAccess()) fail("files.no_grant", "full file access needs Android's All files access for Zeron: the person turns it on in Zeron → Settings → Agent phone access")
        val lexical = FilesPolicy.clean(path)
        val canonical = runCatching { File(lexical).canonicalPath }.getOrDefault(lexical)
        val roots = storageRoots().map { runCatching { File(it).canonicalPath }.getOrDefault(it) } + storageRoots()
        FilesPolicy.refusal(lexical, roots, privateDirs())?.let { fail("files.denied", "$path: $it") }
        FilesPolicy.refusal(canonical, roots, privateDirs())?.let { fail("files.denied", "$path: $it") }
        return File(canonical)
    }

    /** A real file, for handing to another app (attachments). */
    fun readableFile(path: String): File {
        val f = locate(path)
        if (!f.isFile) fail("files.not_found", "$path is not a file")
        return f
    }

    private fun entryOf(f: File): Entry {
        if (!f.exists()) fail("files.not_found", f.path)
        return Entry(f.name, f.path, f.isDirectory, if (f.isFile) f.length() else 0, f.lastModified())
    }

    private fun children(f: File): List<Entry> {
        if (!f.exists()) fail("files.not_found", f.path)
        if (!f.isDirectory) fail("files.denied", "${f.path} is not a folder")
        val list = f.listFiles() ?: fail("files.denied", "${f.path} can't be listed")
        return list.filter { FilesPolicy.refusal(it.path, storageRoots(), privateDirs()) == null }
            .map { Entry(it.name, it.path, it.isDirectory, if (it.isFile) it.length() else 0, it.lastModified()) }
    }

    private fun openRead(f: File, offset: Long): InputStream {
        if (!f.isFile) fail("files.not_found", f.path)
        return RandomAccessFile(f, "r").also { it.seek(offset) }.let { raf -> object : InputStream() {
            override fun read(): Int = raf.read()
            override fun read(b: ByteArray, off: Int, len: Int): Int = raf.read(b, off, len)
            override fun close() = raf.close()
        } }
    }

    // ── the tools ───────────────────────────────────────────────────────────

    fun list(path: String, cursor: String?): JSONObject {
        if (path.isEmpty() || path == "/") {
            if (!allFilesAccess()) fail("files.no_grant", "full file access needs Android's All files access for Zeron: the person turns it on in Zeron → Settings → Agent phone access")
            val out = JSONArray()
            storageRoots().forEach { r -> out.put(JSONObject().put("name", if (r == "/storage/emulated/0") "Internal storage" else r.substringAfterLast('/')).put("path", r).put("kind", "dir").put("writable", true)) }
            return JSONObject().put("entries", out).put("allFilesAccess", true)
        }
        val all = children(locate(path)).sortedBy { it.name }
        val page = all.filter { cursor == null || it.name > cursor }.take(PAGE)
        val out = JSONArray()
        page.forEach { out.put(entryJson(it)) }
        val more = page.size == PAGE && all.any { it.name > page.last().name }
        return JSONObject().put("entries", out).apply { if (more) put("cursor", page.last().name) }
    }

    fun stat(path: String): JSONObject = entryJson(entryOf(locate(path)))

    fun hash(path: String): JSONObject {
        val f = locate(path)
        val e = entryOf(f)
        if (e.dir) fail("files.denied", "${e.path} is a folder")
        val digest = MessageDigest.getInstance("SHA-256")
        var n = 0L
        openRead(f, 0).use { input ->
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
        val f = locate(path)
        val e = entryOf(f)
        if (e.dir) fail("files.denied", "${e.path} is a folder")
        val buffer = ByteArray(len)
        var got = 0
        openRead(f, offset).use { input ->
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

    // Writes in progress: transfer id → the target's path, so a transfer can't be pointed at two targets.
    private val transfers = ConcurrentHashMap<String, String>()

    /**
     * One chunk of a file written to [path]: the first (offset 0) starts `<path>.zeron-part` (refused if the target
     * exists, unless [overwrite]; the folder is created if it is missing); each next one must start where the held
     * bytes end (else files.bad_offset names how many are held, so the sender can resume); the final one checks the
     * size and [sha256] and renames. Every finished file is logged with its path and size.
     */
    fun writePart(transferId: String, path: String, offset: Long, data: ByteArray, final: Boolean, sha256: String?, overwrite: Boolean, confirm: (() -> Boolean)?): JSONObject {
        if (data.size > CHUNK) fail("files.too_large", "a chunk is at most $CHUNK bytes")
        if (transferId.isBlank()) fail("files.denied", "transferId is empty")
        val target = locate(path)
        if (target.path == "/storage/emulated/0" || storageRoots().contains(target.path)) fail("files.denied", "that is the storage root itself")
        if (target.name.endsWith(".zeron-part")) fail("files.denied", "names ending .zeron-part are the transfer's own temporary files")
        val part = File(target.parentFile, "${target.name}.zeron-part")
        if (target.isDirectory) fail("files.denied", "${target.path} is a folder")
        val existing = transfers[transferId]
        if (existing != null && existing != path) fail("files.denied", "transfer $transferId is already writing $existing")
        if (offset == 0L && existing == null) {
            if (target.exists() && !overwrite) fail("files.exists", target.path)
            if (confirm != null && !confirm()) fail("files.denied", "the person declined to let a file be written to $path")
            transfers[transferId] = path
        }
        val held = if (part.isFile) part.length() else 0L
        if (offset != held) fail("files.bad_offset", "the phone holds $held bytes of $path (send from offset $held)")
        val dir = target.parentFile ?: fail("files.denied", "no folder for $path")
        if (!dir.isDirectory && !dir.mkdirs()) fail("files.denied", "couldn't create the folder ${dir.path}")
        if (offset == 0L && part.exists()) part.delete()
        java.io.FileOutputStream(part, true).use { it.write(data) }
        val received = held + data.size
        if (!final) return JSONObject().put("received", received)
        if (sha256 != null) {
            val digest = MessageDigest.getInstance("SHA-256")
            openRead(part, 0).use { input ->
                val buffer = ByteArray(64 * 1024)
                while (true) { val r = input.read(buffer); if (r < 0) break; digest.update(buffer, 0, r) }
            }
            val actual = digest.digest().joinToString("") { "%02x".format(it) }
            if (!actual.equals(sha256, ignoreCase = true)) {
                transfers.remove(transferId)
                part.delete()
                fail("files.denied", "the checksum of what arrived ($actual) is not the one given: the partial file was discarded")
            }
        }
        if (target.exists() && overwrite) target.delete()
        else if (target.exists()) { transfers.remove(transferId); part.delete(); fail("files.exists", target.path) }
        if (!part.renameTo(target)) fail("files.denied", "couldn't move the finished file into place")
        transfers.remove(transferId)
        PhoneActionLog.add(context, "files_write", "wrote ${target.path} (${received} bytes)", true, null)
        return JSONObject().put("received", received).put("final", true).put("path", path)
    }

    private fun entryJson(e: Entry): JSONObject =
        JSONObject().put("name", e.name).put("path", e.path).put("kind", if (e.dir) "dir" else "file").put("size", e.size).put("mtime", e.mtime)

    companion object {
        const val CHUNK = 524_288
        const val PAGE = 500
    }
}
