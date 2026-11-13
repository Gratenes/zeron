package sh.zeron.android.phone

import android.app.KeyguardManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import org.json.JSONObject

/**
 * Letting an agent unlock the phone, only if the person turned it on (Settings → Agent phone access → Let agents
 * unlock this phone, off by default) and gave the phone their PIN ([PhonePinVault]).
 *
 * - It wakes the screen, swipes up, finds the PIN pad on the lock screen (through the accessibility service)
 *   and presses the keys, then checks the phone is really unlocked.
 * - One attempt only. If the phone is still locked afterwards, unlocking is switched off until the person enters
 *   the PIN again: no second try, so a wrong PIN can never lock the person out.
 * - Never over the person: if they touched the phone or woke the screen in the last 3 minutes, it refuses.
 * - It locks again ([GLOBAL_ACTION_LOCK_SCREEN]) after 60 s with no phone_* call, but only if the agent was the one
 *   that unlocked it and the person hasn't touched the phone since.
 * - Every unlock, lock and failure is in the action log; the PIN never is.
 */
object PhoneUnlock {
    private const val RECENT_USER_MS = 3 * 60_000L
    private const val RELOCK_IDLE_MS = 60_000L

    private val main = Handler(Looper.getMainLooper())

    /** When the person last touched the phone or woke it (ours excluded); 0 = not since the app started. */
    @Volatile
    private var lastUser = 0L

    /** Until when touches and screen-ons are ours, not the person's. */
    @Volatile
    private var oursUntil = 0L

    @Volatile
    private var unlockedAt = 0L

    @Volatile
    private var lastToolCall = 0L

    private var receiver: BroadcastReceiver? = null

    fun ours(forMs: Long) {
        oursUntil = maxOf(oursUntil, System.currentTimeMillis() + forMs)
    }

    /** A touch the accessibility service saw: the person's, unless it is one of ours. */
    fun onTouch() {
        if (System.currentTimeMillis() > oursUntil) lastUser = System.currentTimeMillis()
    }

    /** Starts watching the screen for the person's own use. */
    @Synchronized
    fun watch(context: Context) {
        if (receiver != null) return
        val app = context.applicationContext
        val pm = app.getSystemService(PowerManager::class.java)
        if (pm.isInteractive) lastUser = System.currentTimeMillis()
        val r = object : BroadcastReceiver() {
            override fun onReceive(c: Context, intent: Intent) {
                when (intent.action) {
                    Intent.ACTION_SCREEN_ON, Intent.ACTION_USER_PRESENT -> onTouch()
                    Intent.ACTION_SCREEN_OFF -> unlockedAt = 0L // it locked itself: nothing of ours is left to lock
                }
            }
        }
        receiver = r
        app.registerReceiver(r, IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_ON); addAction(Intent.ACTION_SCREEN_OFF); addAction(Intent.ACTION_USER_PRESENT)
        })
    }

    /** Whether the person used the phone (a touch, a click, the screen waking) within [ms]. */
    fun personActiveWithin(ms: Long): Boolean = lastUser != 0L && System.currentTimeMillis() - lastUser < ms

    fun isLocked(context: Context): Boolean = context.getSystemService(KeyguardManager::class.java).isKeyguardLocked

    /** An agent's phone_* call happened: the phone link has work, and the re-lock clock restarts. */
    fun noteToolCall(context: Context) {
        lastToolCall = System.currentTimeMillis()
        if (unlockedAt != 0L) scheduleRelock(context)
    }

    /** `phone_unlock`: unlocks (or says it is already), or throws why not. */
    fun unlock(context: Context): JSONObject {
        val was = isLocked(context)
        ensureUnlocked(context)
        return JSONObject().put("unlocked", true).put("wasLocked", was)
            .put("note", if (was) "Unlocked with the stored PIN; it locks again by itself after ${RELOCK_IDLE_MS / 1000} s without a phone_* call." else "The phone was already unlocked.")
    }

    /** For a tool that acts on the screen: the phone is unlocked after this, or the reason it can't be is thrown. */
    fun ensureUnlocked(context: Context) {
        if (!isLocked(context)) return
        val settings = PhoneSettings(context)
        if (!settings.unlockEnabled) {
            throw PhoneToolException("The phone is locked. Ask the person to unlock it, or they can turn on Zeron → Settings → Agent phone access → Let agents unlock this phone.")
        }
        if (settings.unlockFailed) {
            throw PhoneToolException("Unlocking by agents is switched off after a failed attempt: the person has to enter the PIN again in Zeron → Settings → Agent phone access.")
        }
        val secure = context.getSystemService(KeyguardManager::class.java).isDeviceSecure
        val pin = if (secure) PhonePinVault.load(context) ?: throw PhoneToolException("No PIN is stored for agents: the person has to enter it in Zeron → Settings → Agent phone access.") else null
        val sinceUser = System.currentTimeMillis() - lastUser
        if (lastUser != 0L && sinceUser < RECENT_USER_MS) {
            throw PhoneToolException("The person was using the phone ${sinceUser / 1000} s ago: not unlocking it over them. Try again in a few minutes.")
        }
        val service = PhoneAccessibilityService.instance
            ?: throw PhoneToolException("Zeron's accessibility service is off: it is what unlocks the phone.")
        val started = System.currentTimeMillis()
        // The screen has to be awake, then the lock screen swiped up to show the PIN pad.
        wake(context)
        Thread.sleep(700)
        // The pad appears after the swipe's animation: look for it, and swipe again (differently) if it doesn't show.
        for (attempt in 0 until 3) {
            if (!isLocked(context) || service.pinPadShown()) break
            service.swipeUpFromBottom(attempt)
            for (i in 0 until 6) {
                Thread.sleep(300)
                if (service.pinPadShown()) break
            }
        }
        if (pin != null && isLocked(context)) {
            val entered = service.enterPin(pin)
            if (!entered) {
                // Not even a PIN pad to press: nothing was tried, so this isn't a failed PIN.
                PhoneActionLog.add(context, "unlock", "could not find the PIN pad on the lock screen", false, null)
                throw PhoneToolException("Couldn't find the PIN pad on this phone's lock screen (swiping up didn't show one Zeron can read).")
            }
        }
        // Did it unlock?
        var open = false
        for (i in 0 until 12) {
            if (!isLocked(context)) { open = true; break }
            Thread.sleep(400)
        }
        if (!open) {
            // One attempt only: a wrong PIN is never repeated.
            settings.unlockFailed = true
            service.pressBack()
            PhoneActionLog.add(context, "unlock", "the stored PIN did not unlock the phone: unlocking by agents is switched off until the PIN is entered again", false, null)
            throw PhoneToolException("The stored PIN did not unlock the phone. Unlocking by agents is now off until the person enters the PIN again in Zeron → Settings → Agent phone access. Nothing else was tried.")
        }
        unlockedAt = System.currentTimeMillis()
        lastToolCall = unlockedAt
        PhoneActionLog.add(context, "unlock", "unlocked the phone (${unlockedAt - started} ms)", true, null)
        scheduleRelock(context)
    }

    private fun wake(context: Context) {
        val pm = context.getSystemService(PowerManager::class.java)
        if (pm.isInteractive) return
        ours(6_000)
        @Suppress("DEPRECATION")
        pm.newWakeLock(PowerManager.SCREEN_BRIGHT_WAKE_LOCK or PowerManager.ACQUIRE_CAUSES_WAKEUP or PowerManager.ON_AFTER_RELEASE, "zeron:unlock").acquire(5_000)
    }

    private fun scheduleRelock(context: Context) {
        val app = context.applicationContext
        main.removeCallbacksAndMessages(RELOCK)
        main.postAtTime({ relockIfIdle(app) }, RELOCK, android.os.SystemClock.uptimeMillis() + RELOCK_IDLE_MS + 1_000)
    }

    private val RELOCK = Any()

    private fun relockIfIdle(context: Context) {
        val at = unlockedAt
        if (at == 0L || isLocked(context)) { unlockedAt = 0L; return }
        val idle = System.currentTimeMillis() - lastToolCall
        if (idle < RELOCK_IDLE_MS) return scheduleRelock(context)
        // The person has used the phone since the agent unlocked it: it is theirs now, leave it.
        if (lastUser > at) { unlockedAt = 0L; return }
        unlockedAt = 0L
        ours(2_000)
        if (PhoneAccessibilityService.instance?.lockScreen() == true) {
            PhoneActionLog.add(context, "lock", "locked the phone again after ${idle / 1000} s without an agent", true, null)
        }
    }
}
