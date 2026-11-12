package sh.zeron.android.phone

import android.accessibilityservice.AccessibilityService
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import org.json.JSONArray
import org.json.JSONObject

/**
 * The phone's eyes and hands for agents. Android binds it once the person
 * turns it on in Settings → Accessibility, restarts it after the app is
 * killed or the phone reboots, and it stays bound while on.
 */
class PhoneAccessibilityService : AccessibilityService() {
    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
        // Android keeps this service bound, and its process with it: the phone link rides on it,
        // with no foreground service and no permanent notification.
        PhoneLink.onAccessibilityBound(applicationContext)
    }

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        if (instance === this) instance = null
        PhoneLink.onAccessibilityGone(applicationContext)
        return super.onUnbind(intent)
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit

    override fun onInterrupt() = Unit

    /** The foreground window as a flat list of nodes, in reading order. */
    fun readScreen(maxNodes: Int): JSONObject {
        val root = rootInActiveWindow ?: return JSONObject().put("error", "no active window (the screen may be off or locked)")
        val pkg = root.packageName?.toString().orEmpty()
        if (pkg in BLOCKED_PACKAGES) {
            root.recycle()
            return JSONObject().put("package", pkg).put("blocked", true)
                .put("error", "this app is never shown to agents")
        }
        val nodes = JSONArray()
        var truncated = false
        val queue = ArrayDeque<Pair<AccessibilityNodeInfo, String?>>()
        queue.add(root to null)
        var next = 0
        while (queue.isNotEmpty()) {
            val (node, parent) = queue.removeFirst()
            if (node.isVisibleToUser) {
                if (nodes.length() >= maxNodes) {
                    truncated = true
                    break
                }
                val id = "n${next++}"
                nodes.put(describe(node, id, parent))
                for (i in 0 until node.childCount) node.getChild(i)?.let { queue.add(it to id) }
            }
        }
        return JSONObject().put("package", pkg).put("nodes", nodes).put("truncated", truncated)
            .put("note", "bounds are screen pixels: left, top, right, bottom")
    }

    private fun describe(node: AccessibilityNodeInfo, id: String, parent: String?): JSONObject {
        val bounds = android.graphics.Rect().also(node::getBoundsInScreen)
        val o = JSONObject().put("id", id).put("class", node.className?.toString()?.substringAfterLast('.'))
        if (parent != null) o.put("parent", parent)
        if (node.isPassword) {
            o.put("text", "[password field]")
        } else {
            node.text?.toString()?.takeIf { it.isNotEmpty() }?.let { o.put("text", it.take(500)) }
        }
        node.contentDescription?.toString()?.takeIf { it.isNotEmpty() }?.let { o.put("desc", it.take(300)) }
        node.viewIdResourceName?.let { o.put("resId", it) }
        o.put("bounds", JSONArray(listOf(bounds.left, bounds.top, bounds.right, bounds.bottom)))
        val flags = JSONArray()
        if (node.isClickable) flags.put("clickable")
        if (node.isEditable) flags.put("editable")
        if (node.isScrollable) flags.put("scrollable")
        if (node.isChecked) flags.put("checked")
        if (node.isFocused) flags.put("focused")
        if (!node.isEnabled) flags.put("disabled")
        if (flags.length() > 0) o.put("flags", flags)
        return o
    }

    companion object {
        @Volatile
        var instance: PhoneAccessibilityService? = null
            private set

        /** Apps that are never read or driven: this one, and the system's own prompts that grant power. */
        val BLOCKED_PACKAGES = setOf("sh.zeron.android", "com.android.packageinstaller", "com.google.android.packageinstaller")
    }
}
