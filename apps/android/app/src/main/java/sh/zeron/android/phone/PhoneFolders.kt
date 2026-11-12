package sh.zeron.android.phone

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import org.json.JSONArray
import org.json.JSONObject

/**
 * The folders the person lets agents (and other devices) use, with a "may write" flag each (off by default).
 * Nothing outside them is reachable. A folder is an absolute path; if it was picked with the system folder picker
 * it also carries the picked tree (`uri`) for use when "All files access" is off.
 */
object PhoneFolders {
    class Folder(val path: String, val write: Boolean, val uri: String?)

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences("phone-folders", Context.MODE_PRIVATE)

    fun all(context: Context): List<Folder> {
        val raw = prefs(context).getString("folders", "[]") ?: "[]"
        val out = ArrayList<Folder>()
        runCatching {
            val a = JSONArray(raw)
            for (i in 0 until a.length()) {
                val o = a.getJSONObject(i)
                out += Folder(o.getString("path"), o.optBoolean("write", false), o.optString("uri").takeIf { it.isNotEmpty() })
            }
        }
        return out
    }

    private fun save(context: Context, folders: List<Folder>) {
        val a = JSONArray()
        folders.forEach { a.put(JSONObject().put("path", it.path).put("write", it.write).put("uri", it.uri ?: "")) }
        prefs(context).edit().putString("folders", a.toString()).apply()
    }

    /** Adds (or replaces) a folder; a path is made canonical, trailing slashes dropped. */
    fun add(context: Context, path: String, uri: String? = null, write: Boolean = false) {
        val clean = normalize(path)
        val rest = all(context).filter { it.path != clean }
        save(context, rest + Folder(clean, write, uri))
    }

    fun remove(context: Context, path: String) {
        all(context).firstOrNull { it.path == path }?.uri?.let { u ->
            runCatching { context.contentResolver.releasePersistableUriPermission(Uri.parse(u), 3) }
        }
        save(context, all(context).filter { it.path != path })
    }

    fun setWrite(context: Context, path: String, write: Boolean) {
        save(context, all(context).map { if (it.path == path) Folder(it.path, write, it.uri) else it })
    }

    fun normalize(path: String): String = path.trim().trimEnd('/').ifEmpty { "/" }

    /** The filesystem path of a folder picked with `OpenDocumentTree` (`primary:Recordings/Call` → `/storage/emulated/0/Recordings/Call`). */
    fun pathOfTree(tree: Uri): String? {
        val id = runCatching { DocumentsContract.getTreeDocumentId(tree) }.getOrNull() ?: return null
        val (volume, rest) = id.split(":", limit = 2).let { it[0] to it.getOrElse(1) { "" } }
        val base = if (volume == "primary") "/storage/emulated/0" else "/storage/$volume"
        return normalize(if (rest.isEmpty()) base else "$base/$rest")
    }
}
