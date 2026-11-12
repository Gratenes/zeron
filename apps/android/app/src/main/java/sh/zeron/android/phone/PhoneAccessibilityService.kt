package sh.zeron.android.phone

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Path
import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.util.Base64
import android.view.Display
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import java.io.ByteArrayOutputStream
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject

/**
 * The phone's eyes and hands for agents. Android binds it once the person turns it on in
 * Settings → Accessibility, restarts it after the app is killed or the phone reboots, and it
 * stays bound while on. Every method here runs on a tool thread and waits for the system.
 */
class PhoneAccessibilityService : AccessibilityService() {
    /** The nodes of the last [readScreen], by the ids it gave. */
    private val nodes = ConcurrentHashMap<String, AccessibilityNodeInfo>()

    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
        // Android keeps this service bound, and its process with it: the phone link rides on it,
        // with no foreground service and no permanent notification.
        PhoneLink.onAccessibilityBound(applicationContext)
    }

    override fun onUnbind(intent: Intent?): Boolean {
        if (instance === this) instance = null
        PhoneLink.onAccessibilityGone(applicationContext)
        return super.onUnbind(intent)
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        // Any click, long click, scroll or touch that isn't ours is the person using the phone (agents don't unlock
        // over them). Only the kind of event is looked at, never its content.
        when (event?.eventType) {
            AccessibilityEvent.TYPE_VIEW_CLICKED, AccessibilityEvent.TYPE_VIEW_LONG_CLICKED,
            AccessibilityEvent.TYPE_VIEW_SCROLLED, AccessibilityEvent.TYPE_TOUCH_INTERACTION_START -> PhoneUnlock.onTouch()
        }
    }

    override fun onInterrupt() = Unit

    // ── reading ──────────────────────────────────────────────────────────────

    fun foregroundPackage(): String? = rootInActiveWindow?.packageName?.toString()

    /** The foreground window as a flat list of nodes, in reading order. */
    fun readScreen(maxNodes: Int): JSONObject {
        val root = rootInActiveWindow ?: throw PhoneToolException("no active window (the screen may be off or locked)")
        val pkg = root.packageName?.toString().orEmpty()
        if (pkg in BLOCKED_PACKAGES) throw PhoneToolException("$pkg is never shown to agents")
        val out = JSONArray()
        nodes.clear()
        var truncated = false
        val queue = ArrayDeque<Pair<AccessibilityNodeInfo, String?>>()
        queue.add(root to null)
        var next = 0
        while (queue.isNotEmpty()) {
            val (node, parent) = queue.removeFirst()
            if (!node.isVisibleToUser) continue
            if (out.length() >= maxNodes) {
                truncated = true
                break
            }
            val id = "n${next++}"
            nodes[id] = node
            out.put(describe(node, id, parent))
            for (i in 0 until node.childCount) node.getChild(i)?.let { queue.add(it to id) }
        }
        return JSONObject().put("package", pkg).put("nodes", out).put("truncated", truncated)
            .put("note", "bounds are screen pixels: left, top, right, bottom")
    }

    private fun describe(node: AccessibilityNodeInfo, id: String, parent: String?): JSONObject {
        val bounds = Rect().also(node::getBoundsInScreen)
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

    /** A node from the last read, refreshed: it must still be on screen. */
    private fun node(id: String): AccessibilityNodeInfo {
        val node = nodes[id] ?: throw PhoneToolException("unknown node $id: read the screen again (phone_screen_read)")
        if (!node.refresh()) throw PhoneToolException("node $id is gone: the screen changed, read it again")
        if (node.packageName?.toString() in BLOCKED_PACKAGES) throw PhoneToolException("that app is never driven by agents")
        return node
    }

    /** Controls refuse to act on the apps that are never driven. */
    private fun guardForeground() {
        val pkg = foregroundPackage()
        if (pkg != null && pkg in BLOCKED_PACKAGES) throw PhoneToolException("$pkg is never driven by agents")
    }

    // ── acting ───────────────────────────────────────────────────────────────

    fun tap(args: JSONObject): JSONObject {
        PhoneUnlock.ours(2500)
        guardForeground()
        val id = args.optString("node").takeIf { it.isNotEmpty() }
        if (id != null) {
            val target = node(id)
            var click: AccessibilityNodeInfo? = target
            while (click != null && !click.isClickable) click = click.parent
            if (click != null && click.performAction(AccessibilityNodeInfo.ACTION_CLICK)) {
                return JSONObject().put("ok", true).put("how", "click")
            }
            val b = Rect().also(target::getBoundsInScreen)
            return tapAt(b.exactCenterX(), b.exactCenterY(), 60)
        }
        val (x, y) = point(args)
        return tapAt(x, y, 60)
    }

    fun longPress(args: JSONObject): JSONObject {
        PhoneUnlock.ours(2500)
        guardForeground()
        val ms = args.optLong("duration_ms", 800).coerceIn(300, 3000)
        val id = args.optString("node").takeIf { it.isNotEmpty() }
        if (id != null) {
            val target = node(id)
            if (target.performAction(AccessibilityNodeInfo.ACTION_LONG_CLICK)) return JSONObject().put("ok", true).put("how", "long click")
            val b = Rect().also(target::getBoundsInScreen)
            return tapAt(b.exactCenterX(), b.exactCenterY(), ms)
        }
        val (x, y) = point(args)
        return tapAt(x, y, ms)
    }

    private fun point(args: JSONObject): Pair<Float, Float> {
        if (!args.has("x") || !args.has("y")) throw PhoneToolException("name a node, or give x and y")
        return args.getDouble("x").toFloat() to args.getDouble("y").toFloat()
    }

    private fun tapAt(x: Float, y: Float, ms: Long): JSONObject {
        val path = Path().apply { moveTo(x, y) }
        if (!gesture(path, ms)) throw PhoneToolException("the tap was not performed (the screen may be locked, or the app blocks gestures)")
        return JSONObject().put("ok", true).put("how", "gesture").put("x", x.toInt()).put("y", y.toInt())
    }

    fun type(args: JSONObject): JSONObject {
        PhoneUnlock.ours(2500)
        guardForeground()
        val text = args.optString("text", "")
        val id = args.optString("node").takeIf { it.isNotEmpty() }
        val field = if (id != null) node(id) else rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            ?: throw PhoneToolException("no text field has focus: tap one first, or name its node")
        if (field.isPassword) throw PhoneToolException("password fields are never typed into by agents")
        if (!field.isEditable) throw PhoneToolException("that node is not a text field")
        field.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
        val existing = if (args.optBoolean("append", false) && !field.isShowingHintText) field.text?.toString().orEmpty() else ""
        val set = Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, existing + text)
        }
        if (!field.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, set)) throw PhoneToolException("the field refused the text")
        var submitted = false
        if (args.optBoolean("submit", false)) {
            submitted = field.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
        }
        return JSONObject().put("ok", true).put("chars", text.length).put("submitted", submitted)
    }

    fun swipe(args: JSONObject): JSONObject {
        PhoneUnlock.ours(2500)
        guardForeground()
        val m = resources.displayMetrics
        val w = m.widthPixels.toFloat()
        val h = m.heightPixels.toFloat()
        val ms = args.optLong("duration_ms", 300).coerceIn(50, 3000)
        val path = Path()
        val direction = args.optString("direction").takeIf { it.isNotEmpty() }
        if (direction != null) {
            val d = args.optDouble("distance", 0.5).coerceIn(0.1, 0.9).toFloat()
            val cx = w / 2
            val cy = h / 2
            // The content moves the way the finger does: `up` drags from low to high.
            when (direction) {
                "up" -> { path.moveTo(cx, cy + h * d / 2); path.lineTo(cx, cy - h * d / 2) }
                "down" -> { path.moveTo(cx, cy - h * d / 2); path.lineTo(cx, cy + h * d / 2) }
                "left" -> { path.moveTo(cx + w * d / 2, cy); path.lineTo(cx - w * d / 2, cy) }
                "right" -> { path.moveTo(cx - w * d / 2, cy); path.lineTo(cx + w * d / 2, cy) }
                else -> throw PhoneToolException("direction is up, down, left or right")
            }
        } else {
            if (!args.has("from_x") || !args.has("to_x")) throw PhoneToolException("give a direction, or from_x, from_y, to_x and to_y")
            path.moveTo(args.getDouble("from_x").toFloat(), args.getDouble("from_y").toFloat())
            path.lineTo(args.getDouble("to_x").toFloat(), args.getDouble("to_y").toFloat())
        }
        if (!gesture(path, ms)) throw PhoneToolException("the swipe was not performed")
        return JSONObject().put("ok", true)
    }

    fun scroll(args: JSONObject): JSONObject {
        PhoneUnlock.ours(2500)
        guardForeground()
        val forward = when (args.optString("direction")) {
            "forward" -> true
            "backward" -> false
            else -> throw PhoneToolException("direction is forward or backward")
        }
        val id = args.optString("node").takeIf { it.isNotEmpty() }
        val target = if (id != null) node(id) else nodes.values.firstOrNull { it.isScrollable && it.refresh() }
            ?: throw PhoneToolException("no scrollable node: read the screen, or name one")
        val ok = target.performAction(if (forward) AccessibilityNodeInfo.ACTION_SCROLL_FORWARD else AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD)
        if (!ok) throw PhoneToolException("that node can't scroll further that way")
        return JSONObject().put("ok", true)
    }

    fun press(args: JSONObject): JSONObject {
        val action = when (args.optString("button")) {
            "back" -> GLOBAL_ACTION_BACK
            "home" -> GLOBAL_ACTION_HOME
            "recents" -> GLOBAL_ACTION_RECENTS
            "notifications" -> GLOBAL_ACTION_NOTIFICATIONS
            "quick_settings" -> GLOBAL_ACTION_QUICK_SETTINGS
            "lock_screen" -> GLOBAL_ACTION_LOCK_SCREEN
            else -> throw PhoneToolException("button is back, home, recents, notifications, quick_settings or lock_screen")
        }
        if (!performGlobalAction(action)) throw PhoneToolException("the phone refused that button")
        return JSONObject().put("ok", true)
    }

    fun screenshot(maxEdge: Int): JSONObject {
        if (Build.VERSION.SDK_INT < 30) throw PhoneToolException("screenshots need Android 11 or newer")
        val pkg = foregroundPackage()
        if (pkg != null && pkg in BLOCKED_PACKAGES) throw PhoneToolException("$pkg is never shown to agents")
        val latch = CountDownLatch(1)
        var bitmap: Bitmap? = null
        var failure: String? = null
        takeScreenshot(Display.DEFAULT_DISPLAY, mainExecutor, object : TakeScreenshotCallback {
            override fun onSuccess(result: ScreenshotResult) {
                val buffer = result.hardwareBuffer
                bitmap = Bitmap.wrapHardwareBuffer(buffer, result.colorSpace)?.copy(Bitmap.Config.ARGB_8888, false)
                buffer.close()
                latch.countDown()
            }

            override fun onFailure(errorCode: Int) {
                failure = when (errorCode) {
                    ERROR_TAKE_SCREENSHOT_INTERVAL_TIME_SHORT -> "screenshots can't be taken that often: wait a second"
                    ERROR_TAKE_SCREENSHOT_SECURE_WINDOW -> "this screen is marked secure by its app and can't be captured"
                    ERROR_TAKE_SCREENSHOT_INVALID_DISPLAY -> "no display to capture"
                    else -> "the phone could not take a screenshot (code $errorCode)"
                }
                latch.countDown()
            }
        })
        if (!latch.await(5, TimeUnit.SECONDS)) throw PhoneToolException("the screenshot timed out")
        failure?.let { throw PhoneToolException(it) }
        val full = bitmap ?: throw PhoneToolException("the screenshot was empty")
        val scale = minOf(1f, maxEdge.toFloat() / maxOf(full.width, full.height))
        val shot = if (scale < 1f) Bitmap.createScaledBitmap(full, (full.width * scale).toInt(), (full.height * scale).toInt(), true) else full
        val bytes = ByteArrayOutputStream().also { shot.compress(Bitmap.CompressFormat.JPEG, 70, it) }.toByteArray()
        return JSONObject().put("jpegBase64", Base64.encodeToString(bytes, Base64.NO_WRAP))
            .put("width", shot.width).put("height", shot.height)
            .put("screenWidth", full.width).put("screenHeight", full.height)
            .put("package", pkg ?: JSONObject.NULL)
    }

    private fun gesture(path: Path, durationMs: Long): Boolean {
        PhoneUnlock.ours(durationMs + 1500)
        val gesture = GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(path, 0, durationMs.coerceAtLeast(1))).build()
        val done = CountDownLatch(1)
        var ok = false
        val accepted = dispatchGesture(gesture, object : GestureResultCallback() {
            override fun onCompleted(g: GestureDescription?) {
                ok = true
                done.countDown()
            }

            override fun onCancelled(g: GestureDescription?) = done.countDown()
        }, null)
        if (!accepted) return false
        done.await(durationMs + 3000, TimeUnit.MILLISECONDS)
        return ok
    }

    // ── the lock screen ──────────────────────────────────────────────────────

    /** Swipes up from the bottom of the screen: opens the lock screen's PIN pad. */
    fun swipeUpFromBottom() {
        val m = resources.displayMetrics
        val path = Path().apply { moveTo(m.widthPixels / 2f, m.heightPixels * 0.88f); lineTo(m.widthPixels / 2f, m.heightPixels * 0.30f) }
        gesture(path, 300)
    }

    fun pressBack() { performGlobalAction(GLOBAL_ACTION_BACK) }

    fun lockScreen(): Boolean = performGlobalAction(GLOBAL_ACTION_LOCK_SCREEN)

    /**
     * Presses the digits of [pin] on the lock screen's PIN pad (the keys are found by their ids `key0`…`key9`, or a
     * clickable digit), then Enter if there is one. False if there is no PIN pad to be found: nothing was pressed.
     * The digits are never logged.
     */
    fun enterPin(pin: String): Boolean {
        var keys: Map<Char, AccessibilityNodeInfo> = emptyMap()
        for (attempt in 0 until 6) {
            keys = pinKeys()
            if (keys.keys.containsAll(pin.toSet())) break
            Thread.sleep(500)
        }
        if (!keys.keys.containsAll(pin.toSet())) return false
        PhoneUnlock.ours(pin.length * 400L + 2500)
        for (digit in pin) {
            val key = keys[digit] ?: return false
            press(key)
            Thread.sleep(180)
        }
        Thread.sleep(300)
        // Most PIN pads unlock on Enter (some at once, when the length is known).
        pinEnter()?.let { press(it) }
        return true
    }

    private fun press(node: AccessibilityNodeInfo) {
        var n: AccessibilityNodeInfo? = node
        while (n != null && !n.isClickable) n = n.parent
        if (n != null && n.performAction(AccessibilityNodeInfo.ACTION_CLICK)) return
        val b = Rect().also(node::getBoundsInScreen)
        val path = Path().apply { moveTo(b.exactCenterX(), b.exactCenterY()) }
        gesture(path, 60)
    }

    private fun pinKeys(): Map<Char, AccessibilityNodeInfo> {
        val found = HashMap<Char, AccessibilityNodeInfo>()
        val root = rootInActiveWindow ?: return found
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)
        while (queue.isNotEmpty()) {
            val n = queue.removeFirst()
            val byId = n.viewIdResourceName?.let { Regex(".*:id/key([0-9])$").matchEntire(it)?.groupValues?.get(1)?.get(0) }
            val byText = (n.text?.toString() ?: n.contentDescription?.toString())?.takeIf { it.length == 1 && it[0].isDigit() && n.packageName == root.packageName }?.get(0)
            val digit = byId ?: byText
            if (digit != null && n.isVisibleToUser && digit !in found) found[digit] = n
            for (i in 0 until n.childCount) n.getChild(i)?.let(queue::add)
        }
        return found
    }

    private fun pinEnter(): AccessibilityNodeInfo? {
        val root = rootInActiveWindow ?: return null
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)
        while (queue.isNotEmpty()) {
            val n = queue.removeFirst()
            val id = n.viewIdResourceName.orEmpty()
            val label = (n.contentDescription ?: n.text)?.toString().orEmpty()
            if (n.isVisibleToUser && (id.endsWith(":id/key_enter") || label.equals("Enter", ignoreCase = true) || label.equals("OK", ignoreCase = true))) return n
            for (i in 0 until n.childCount) n.getChild(i)?.let(queue::add)
        }
        return null
    }

    companion object {
        @Volatile
        var instance: PhoneAccessibilityService? = null
            private set

        /** Apps that are never read or driven: this one, and the system's own prompts that grant power. */
        val BLOCKED_PACKAGES = setOf("sh.zeron.android", "com.android.packageinstaller", "com.google.android.packageinstaller")
    }
}
