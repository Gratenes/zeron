package sh.zeron.android.phone

import android.content.Context

/**
 * What the person lets agents do on this phone. Off by default: nothing works until the master
 * switch is on, and each tool group has its own. Checked in [PhoneTools] on the phone, never
 * trusted to the caller.
 */
class PhoneSettings(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("phone-access", Context.MODE_PRIVATE)

    var enabled: Boolean
        get() = prefs.getBoolean("enabled", false)
        set(value) = prefs.edit().putBoolean("enabled", value).apply()

    /** The Stop button: agent access is refused until this time, then comes back by itself. */
    var pausedUntil: Long
        get() = prefs.getLong("paused-until", 0L)
        set(value) = prefs.edit().putLong("paused-until", value).apply()

    val paused: Boolean get() = System.currentTimeMillis() < pausedUntil

    fun groupOn(group: Group): Boolean = prefs.getBoolean("group-${group.key}", group.defaultOn)

    fun setGroup(group: Group, on: Boolean) = prefs.edit().putBoolean("group-${group.key}", on).apply()

    /** Ask the person before each text is sent (a notification with Send / Don't send). */
    var confirmSmsSend: Boolean
        get() = prefs.getBoolean("confirm-sms-send", true)
        set(value) = prefs.edit().putBoolean("confirm-sms-send", value).apply()

    /**
     * How often the link pings its room. Each ping wakes the radio; 60 s is far inside what a
     * mobile NAT keeps open, and the room is told so it keeps routing meanwhile.
     */
    var keepaliveSecs: Int
        get() = prefs.getInt("keepalive-secs", 60)
        set(value) = prefs.edit().putInt("keepalive-secs", value.coerceIn(30, 300)).apply()

    /** A new text from one of [wakeSenders] is sent to chat [wakeChatId] (an agent can then act on it). */
    var wakeEnabled: Boolean
        get() = prefs.getBoolean("wake-enabled", false)
        set(value) = prefs.edit().putBoolean("wake-enabled", value).apply()

    var wakeChatId: String?
        get() = prefs.getString("wake-chat-id", null)
        set(value) = prefs.edit().putString("wake-chat-id", value).apply()

    var wakeChatTitle: String?
        get() = prefs.getString("wake-chat-title", null)
        set(value) = prefs.edit().putString("wake-chat-title", value).apply()

    /** Comma-separated phone numbers (any formatting) or `*` for anyone. Empty wakes nothing. */
    var wakeSenders: String
        get() = prefs.getString("wake-senders", "") ?: ""
        set(value) = prefs.edit().putString("wake-senders", value).apply()

    enum class Group(val key: String, val label: String, val about: String, val defaultOn: Boolean) {
        SCREEN_READ("screen-read", "Read the screen", "What an app shows, as text", true),
        SCREENSHOT("screenshot", "Take screenshots", "A picture of the screen", true),
        CONTROL("control", "Tap, type and swipe", "Use apps, open apps, press Back and Home", true),
        SMS_READ("sms-read", "Read text messages", "Conversations and messages", true),
        SMS_SEND("sms-send", "Send text messages", "From your number (see the question below)", true),
        CONTACTS("contacts", "Search contacts", "Names and numbers", true),
    }
}
