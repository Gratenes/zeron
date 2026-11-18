package sh.zeron.android.phone

import android.content.Context

/**
 * What the person lets agents do on this phone. Off by default: nothing works until the master
 * switch is on, and each tool group has its own. Checked in [PhoneTools] on the phone, never
 * trusted to the caller.
 */
class PhoneSettings(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("phone-access", Context.MODE_PRIVATE)

    init {
        // Files used to be a per-folder allow list with write flags; now it is one switch (same key, so a switch that
        // was on stays on). The old list and the old "ask before writing" default are dropped, once.
        if (!prefs.getBoolean("files-simple", false)) {
            context.applicationContext.getSharedPreferences("phone-folders", Context.MODE_PRIVATE).edit().clear().apply()
            prefs.edit().remove("confirm-file-writes").putBoolean("files-simple", true).apply()
        }
    }

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

    /** Ask before each app an agent wants to install (Install / Deny / Always for this chat). */
    var confirmInstalls: Boolean
        get() = prefs.getBoolean("confirm-installs", true)
        set(value) = prefs.edit().putBoolean("confirm-installs", value).apply()

    /** Chats the person said "always" to for installs: they are not asked again. */
    var trustedInstallChats: Set<String>
        get() = prefs.getStringSet("trusted-install-chats", emptySet()) ?: emptySet()
        set(value) = prefs.edit().putStringSet("trusted-install-chats", value).apply()

    /** Ask the person before a file is written to the phone (once per transfer). */
    var confirmFileWrites: Boolean
        get() = prefs.getBoolean("confirm-file-writes", false)
        set(value) = prefs.edit().putBoolean("confirm-file-writes", value).apply()

    /** Agents may unlock the phone with the PIN the person stored (off by default). */
    var unlockEnabled: Boolean
        get() = prefs.getBoolean("unlock-enabled", false)
        set(value) = prefs.edit().putBoolean("unlock-enabled", value).apply()

    /** An unlock attempt failed: agents are refused until the person enters the PIN again. */
    var unlockFailed: Boolean
        get() = prefs.getBoolean("unlock-failed", false)
        set(value) = prefs.edit().putBoolean("unlock-failed", value).apply()

    /** A new text from one of [wakeSenders] is sent to chat [wakeChatId] (an agent can then act on it). */
    var wakeEnabled: Boolean
        get() = prefs.getBoolean("wake-enabled", false)
        set(value) = prefs.edit().putBoolean("wake-enabled", value).apply()

    /** A new voicemail is sent into the same chat as new texts ([wakeChatId]). */
    var voicemailWake: Boolean
        get() = prefs.getBoolean("voicemail-wake", false)
        set(value) = prefs.edit().putBoolean("voicemail-wake", value).apply()

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
        SMS_READ("sms-read", "Read text and chat messages", "SMS, picture messages and Google Messages (RCS) chats", true),
        SMS_SEND("sms-send", "Send text and chat messages", "From your number, or in a Messages chat (see the question below)", true),
        CONTACTS("contacts", "Search contacts", "Names and numbers", true),
        ATTACH_READ("attachments-read", "Read attachments", "Save photos, videos and files from messages so agents can look at them", true),
        ATTACH_SEND("attachments-send", "Send attachments", "Photos, videos and files in a message (through Google Messages)", false),
        REACTIONS("reactions", "React to messages", "Send an emoji reaction to a message", false),
        VOICEMAIL("voicemail", "Read voicemails", "New voicemails announced by Google Voice or the Phone app, with their transcripts", true),
        INSTALL("install", "Let agents install apps", "Install an .apk an agent put on the phone, after you confirm it", false),
        SHELL("shell", "Allow Shizuku shell", "Let Zeron use Shizuku for a few fixed system actions (grant a permission, stop an app, read a setting)", false),
        FILES("files", "Full access to this phone's files", "List, read and write any file in shared storage", false),
    }
}
