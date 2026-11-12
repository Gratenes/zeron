package sh.zeron.android.phone

import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import android.os.Build
import org.json.JSONObject
import uniffi.zeron_core.PhoneToolHandler
import uniffi.zeron_core.PhoneToolReply

/**
 * The tools agents call (their `phone_*` in Zeron's MCP). Every call checks
 * this phone's own switches first: a tool that is off answers with why, so the
 * person's setting holds whatever an agent or a device asks.
 */
class PhoneTools(private val context: Context, private val settings: PhoneSettings) : PhoneToolHandler {
    override fun call(tool: String, argsJson: String): PhoneToolReply = try {
        PhoneLink.touch(context)
        val args = JSONObject(argsJson.ifBlank { "{}" })
        if (!settings.enabled) {
            fail("Agent access is switched off on this phone (Zeron → Settings → Agent phone access)")
        } else {
            when (tool) {
                "ping" -> ok(ping())
                "screen_read" -> screenRead(args)
                else -> fail("this phone doesn't know the tool $tool (update Zeron for Android)")
            }
        }
    } catch (e: Exception) {
        fail("${e.javaClass.simpleName}: ${e.message}")
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
            .put("allowed", JSONObject().also { o ->
                PhoneSettings.Group.entries.forEach { o.put(it.key, settings.groupOn(it)) }
            })
            .put("foregroundPackage", service?.rootInActiveWindow?.packageName?.toString() ?: JSONObject.NULL)
    }

    private fun screenRead(args: JSONObject): PhoneToolReply {
        if (!settings.groupOn(PhoneSettings.Group.SCREEN_READ)) return fail("Reading the screen is switched off on this phone")
        val service = PhoneAccessibilityService.instance
            ?: return fail("Zeron's accessibility service is off: turn it on in Android Settings → Accessibility → Zeron")
        val screen = service.readScreen(args.optInt("max_nodes", 400).coerceIn(1, 2000))
        return if (screen.has("error")) fail(screen.getString("error")) else ok(screen)
    }

    private fun ok(value: JSONObject) = PhoneToolReply(true, value.toString())

    private fun fail(message: String) = PhoneToolReply(false, message)
}
