package sh.zeron.android.phone

import android.content.Context
import java.io.File
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject

/**
 * What agents did on this phone, newest last: when, which tool, a short redacted summary (never
 * a message's text or what was typed: only its length), whether it worked, and which chat asked.
 * One JSON line each in a file in the app's private storage, capped.
 */
object PhoneActionLog {
    data class Entry(val at: Long, val tool: String, val summary: String, val ok: Boolean, val who: String?)

    private const val KEEP = 500
    private val lock = Any()
    private var file: File? = null
    private val _entries = MutableStateFlow<List<Entry>>(emptyList())
    val entries: StateFlow<List<Entry>> = _entries

    fun init(context: Context) = synchronized(lock) {
        if (file != null) return
        val f = File(context.applicationContext.filesDir, "phone-actions.jsonl")
        file = f
        _entries.value = if (f.exists()) f.readLines().takeLast(KEEP).mapNotNull(::parse) else emptyList()
    }

    fun add(context: Context, tool: String, summary: String, ok: Boolean, who: String?) {
        init(context)
        val entry = Entry(System.currentTimeMillis(), tool, summary, ok, who)
        synchronized(lock) {
            val all = (_entries.value + entry).takeLast(KEEP)
            _entries.value = all
            val f = file ?: return
            runCatching {
                if (all.size == KEEP) f.writeText(all.joinToString("") { json(it) + "\n" }) else f.appendText(json(entry) + "\n")
            }
        }
    }

    fun clear() = synchronized(lock) {
        _entries.value = emptyList()
        runCatching { file?.delete() }
    }

    private fun json(e: Entry) = JSONObject().put("at", e.at).put("tool", e.tool).put("summary", e.summary)
        .put("ok", e.ok).put("who", e.who ?: JSONObject.NULL).toString()

    private fun parse(line: String): Entry? = runCatching {
        val o = JSONObject(line)
        Entry(o.getLong("at"), o.getString("tool"), o.getString("summary"), o.getBoolean("ok"), o.optString("who").takeIf { it.isNotEmpty() && it != "null" })
    }.getOrNull()
}
