package sh.zeron.android.phone

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import sh.zeron.android.ZeronApplication

/**
 * When this phone's link to its agents (the device-room host) is up.
 *
 * - While Zeron's accessibility service is bound (Android keeps it bound, and restarts it after a
 *   kill or reboot) the link stays up: no foreground service, no permanent notification.
 * - Without it, the link is up only for a while after a wake (a push, [onWake]) and drops after
 *   [IDLE_MS] with no call, so an idle phone holds no socket.
 */
object PhoneLink {
    private const val TAG = "ZeronPhone"
    const val IDLE_MS = 60_000L

    private val main = Handler(Looper.getMainLooper())

    @Volatile
    private var lastCall = 0L
    private var context: Context? = null

    private val idle: Runnable = Runnable { checkIdle() }

    private fun checkIdle() {
        val ctx = context ?: return
        if (PhoneAccessibilityService.instance != null) return
        val quiet = System.currentTimeMillis() - lastCall
        if (quiet >= IDLE_MS) {
            Log.i(TAG, "idle for ${quiet / 1000}s with no accessibility service: dropping the host")
            model(ctx).stopPhoneHost()
        } else {
            main.postDelayed(idle, IDLE_MS - quiet)
        }
    }

    private fun model(context: Context) = (context.applicationContext as ZeronApplication).model

    fun onAccessibilityBound(context: Context) {
        this.context = context
        main.removeCallbacks(idle)
        Log.i(TAG, "accessibility bound: starting the host")
        main.post { model(context).bootHeadless() }
        watchMessages(context)
        PhoneUnlock.watch(context)
    }

    fun onAccessibilityGone(context: Context) {
        this.context = context
        scheduleIdle()
    }

    /** A push (or anything that has to reach the agents' tools) woke the app. */
    fun onWake(context: Context, reason: String) {
        this.context = context
        lastCall = System.currentTimeMillis()
        Log.i(TAG, "wake ($reason) at $lastCall")
        main.post { model(context).bootHeadless() }
        watchMessages(context)
        scheduleIdle()
    }

    /** Chat and picture messages arrive in the provider: watch it while the person has agent access on. */
    private fun watchMessages(context: Context) {
        if (PhoneSettings(context).enabled) PhoneMessageWatcher.start(context) else PhoneMessageWatcher.stop()
    }

    /** An agent's call arrived: the link has work. */
    fun touch(context: Context) {
        this.context = context
        lastCall = System.currentTimeMillis()
        PhoneUnlock.noteToolCall(context)
        if (PhoneAccessibilityService.instance == null) scheduleIdle()
    }

    private fun scheduleIdle() {
        main.removeCallbacks(idle)
        main.postDelayed(idle, IDLE_MS)
    }
}
