package sh.zeron.android.phone

import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.BatteryManager
import android.os.Build
import java.text.DateFormat
import java.util.Date
import org.json.JSONArray
import org.json.JSONObject
import uniffi.zeron_core.PhoneToolHandler
import uniffi.zeron_core.PhoneToolReply

/**
 * The tools agents call (their `phone_*` in Zeron's MCP). Every call first checks this phone's own
 * switches, so the person's settings hold whatever an agent or a device asks; shows the "agent is
 * using this phone" notice; and is written to the action log (a summary, never message text or
 * what was typed).
 */
class PhoneTools(private val context: Context, private val settings: PhoneSettings) : PhoneToolHandler {
    private val sms by lazy { PhoneSms(context) }

    override fun call(tool: String, argsJson: String, originJson: String): PhoneToolReply {
        PhoneLink.touch(context)
        val args = runCatching { JSONObject(argsJson.ifBlank { "{}" }) }.getOrDefault(JSONObject())
        val origin = runCatching { JSONObject(originJson) }.getOrNull()
        val who = origin?.optString("chatTitle")?.takeIf { it.isNotBlank() && it != "null" }
            ?: origin?.optString("chatId")?.takeIf { it.isNotBlank() }?.take(8)
        val summary = summarize(tool, args)
        return try {
            gate(tool)
            if (tool != "ping") PhoneNotice.active(context, who, summary)
            val result = run(tool, args, who)
            PhoneActionLog.add(context, tool, summary, true, who)
            PhoneToolReply(true, result.toString())
        } catch (e: PhoneToolException) {
            PhoneActionLog.add(context, tool, "$summary — refused: ${e.message}".take(300), false, who)
            PhoneToolReply(false, e.message ?: "refused")
        } catch (e: Exception) {
            PhoneActionLog.add(context, tool, "$summary — failed: ${e.javaClass.simpleName}".take(300), false, who)
            PhoneToolReply(false, "${e.javaClass.simpleName}: ${e.message}")
        }
    }

    /** The master switch, the Stop pause, and the tool's group. */
    private fun gate(tool: String) {
        if (!settings.enabled) throw PhoneToolException("Agent access is switched off on this phone (Zeron → Settings → Agent phone access)")
        if (settings.paused) {
            val until = DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(settings.pausedUntil))
            throw PhoneToolException("The person stopped agent access on this phone until $until")
        }
        val group = GROUPS[tool] ?: if (tool in NO_GROUP) return else throw PhoneToolException("this phone doesn't know the tool $tool (update Zeron for Android)")
        if (!settings.groupOn(group)) throw PhoneToolException("${group.label} is switched off on this phone")
    }

    private fun run(tool: String, args: JSONObject, who: String?): JSONObject {
        fun service() = PhoneAccessibilityService.instance
            ?: throw PhoneToolException("Zeron's accessibility service is off: turn it on in Android Settings → Accessibility → Zeron")
        return when (tool) {
            "ping" -> ping()
            "screen_read" -> service().readScreen(args.optInt("max_nodes", 400).coerceIn(1, 2000))
            "screenshot" -> service().screenshot(args.optInt("max_edge", 1280).coerceIn(200, 2400))
            "tap" -> service().tap(args)
            "long_press" -> service().longPress(args)
            "type" -> service().type(args)
            "swipe" -> service().swipe(args)
            "scroll" -> service().scroll(args)
            "press" -> service().press(args)
            "launch_app" -> launchApp(args.optString("app"))
            "list_apps" -> listApps(args.optString("query"))
            "wait" -> {
                Thread.sleep(args.optLong("ms", 1000).coerceIn(100, 10_000))
                JSONObject().put("package", PhoneAccessibilityService.instance?.foregroundPackage() ?: JSONObject.NULL)
            }
            "sms_threads" -> sms.threads(args.optInt("limit", 20).coerceIn(1, 100), args.optString("query").takeIf { it.isNotBlank() })
            "sms_read" -> sms.read(
                args.optString("thread_id").takeIf { it.isNotBlank() }, args.optString("address").takeIf { it.isNotBlank() },
                args.optInt("limit", 30).coerceIn(1, 200), args.optString("before_id").takeIf { it.isNotBlank() },
            )
            "sms_send" -> {
                val text = args.optString("text")
                val via = args.optString("via").ifEmpty { "auto" }
                if (via != "auto" && via != "sms") throw PhoneToolException("via is auto or sms")
                val to = args.optString("to").takeIf { it.isNotBlank() }
                sms.send(to, args.optString("thread_id").takeIf { it.isNotBlank() }, text, via, if (settings.confirmSmsSend) ({ how ->
                    PhoneConfirm.ask(context, "${who ?: "An agent"} wants to send a message $how".take(120), text)
                }) else null)
            }
            "contacts_search" -> sms.contacts(args.optString("query"), args.optInt("limit", 10).coerceIn(1, 50))
            else -> throw PhoneToolException("this phone doesn't know the tool $tool")
        }
    }

    private fun ping(): JSONObject {
        val battery = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = battery?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = battery?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
        val service = PhoneAccessibilityService.instance
        return JSONObject()
            .put("device", Build.MODEL)
            .put("android", Build.VERSION.RELEASE)
            .put("sdk", Build.VERSION.SDK_INT)
            .put("batteryPercent", if (level >= 0) level * 100 / scale else JSONObject.NULL)
            .put("accessibilityOn", service != null)
            .put("notificationAccessOn", PhoneNotificationListener.instance != null)
            .put("paused", settings.paused)
            .put("confirmsTextSends", settings.confirmSmsSend)
            .put("allowed", JSONObject().also { o -> PhoneSettings.Group.entries.forEach { o.put(it.key, settings.groupOn(it)) } })
            .put("foregroundPackage", service?.foregroundPackage() ?: JSONObject.NULL)
    }

    // ── apps ────────────────────────────────────────────────────────────────

    private data class App(val name: String, val pkg: String)

    private fun apps(): List<App> {
        val pm = context.packageManager
        val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        return pm.queryIntentActivities(intent, 0)
            .map { App(it.loadLabel(pm).toString(), it.activityInfo.packageName) }
            .distinctBy { it.pkg }
            .sortedBy { it.name.lowercase() }
    }

    private fun listApps(query: String): JSONObject {
        val q = query.trim().lowercase()
        val out = JSONArray()
        apps().filter { q.isEmpty() || it.name.lowercase().contains(q) || it.pkg.lowercase().contains(q) }
            .take(300).forEach { out.put(JSONObject().put("name", it.name).put("package", it.pkg)) }
        return JSONObject().put("apps", out)
    }

    private fun launchApp(app: String): JSONObject {
        val key = app.trim()
        if (key.isEmpty()) throw PhoneToolException("app is empty")
        val all = apps()
        val chosen = all.firstOrNull { it.pkg == key }
            ?: all.firstOrNull { it.name.equals(key, ignoreCase = true) }
            ?: all.filter { it.name.contains(key, ignoreCase = true) }.let { partial ->
                when (partial.size) {
                    1 -> partial[0]
                    0 -> throw PhoneToolException("no app matches \"$key\" (phone_list_apps shows what is installed)")
                    else -> throw PhoneToolException("several apps match \"$key\": ${partial.take(8).joinToString { "${it.name} (${it.pkg})" }}")
                }
            }
        if (chosen.pkg in PhoneAccessibilityService.BLOCKED_PACKAGES) throw PhoneToolException("${chosen.name} is never opened by agents")
        val intent = context.packageManager.getLaunchIntentForPackage(chosen.pkg)
            ?: throw PhoneToolException("${chosen.name} can't be opened")
        // The accessibility service may start an activity from the background; an app context may not.
        val starter: Context = PhoneAccessibilityService.instance ?: context
        starter.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        return JSONObject().put("ok", true).put("opened", chosen.name).put("package", chosen.pkg)
    }

    /** One line for the log: what was done, with no message text and nothing typed. */
    private fun summarize(tool: String, a: JSONObject): String = when (tool) {
        "ping" -> "checked in"
        "screen_read" -> "read the screen"
        "screenshot" -> "took a screenshot"
        "tap" -> "tapped ${a.optString("node").ifEmpty { "(${a.optInt("x")}, ${a.optInt("y")})" }}"
        "long_press" -> "long-pressed ${a.optString("node").ifEmpty { "(${a.optInt("x")}, ${a.optInt("y")})" }}"
        "type" -> "typed ${a.optString("text").length} characters"
        "swipe" -> "swiped ${a.optString("direction").ifEmpty { "a path" }}"
        "scroll" -> "scrolled ${a.optString("direction")}"
        "press" -> "pressed ${a.optString("button")}"
        "launch_app" -> "opened ${a.optString("app")}"
        "list_apps" -> "listed apps"
        "wait" -> "waited ${a.optInt("ms", 1000)} ms"
        "sms_threads" -> "listed text conversations"
        "sms_read" -> "read texts from ${a.optString("thread_id").ifEmpty { a.optString("address") }}"
        "sms_send" -> "sent a message (${a.optString("text").length} characters) to ${a.optString("to").ifEmpty { "thread " + a.optString("thread_id") }}"
        "contacts_search" -> "searched contacts"
        else -> tool
    }

    companion object {
        private val GROUPS = mapOf(
            "screen_read" to PhoneSettings.Group.SCREEN_READ,
            "screenshot" to PhoneSettings.Group.SCREENSHOT,
            "tap" to PhoneSettings.Group.CONTROL, "long_press" to PhoneSettings.Group.CONTROL,
            "type" to PhoneSettings.Group.CONTROL, "swipe" to PhoneSettings.Group.CONTROL,
            "scroll" to PhoneSettings.Group.CONTROL, "press" to PhoneSettings.Group.CONTROL,
            "launch_app" to PhoneSettings.Group.CONTROL, "list_apps" to PhoneSettings.Group.CONTROL,
            "wait" to PhoneSettings.Group.SCREEN_READ,
            "sms_threads" to PhoneSettings.Group.SMS_READ, "sms_read" to PhoneSettings.Group.SMS_READ,
            "sms_send" to PhoneSettings.Group.SMS_SEND,
            "contacts_search" to PhoneSettings.Group.CONTACTS,
        )
        private val NO_GROUP = setOf("ping")
    }
}
