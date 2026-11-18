package sh.zeron.android.phone

/**
 * The only shell commands an agent can cause, built from validated pieces (never a string): argv lists for
 * `pm install`, `pm grant`/`revoke`, `am force-stop` and `settings get`. Pure, so it is tested without Android.
 */
object ShellArgs {
    private val PACKAGE = Regex("^[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z0-9_]+)+$")
    private val PERMISSION = Regex("^[A-Za-z][A-Za-z0-9_.]{2,127}$")
    private val KEY = Regex("^[A-Za-z0-9_.:-]{1,100}$")
    val NAMESPACES = setOf("system", "secure", "global")

    fun isPackage(s: String) = s.length <= 255 && PACKAGE.matches(s)
    fun isPermission(s: String) = PERMISSION.matches(s)
    fun isSettingsKey(s: String) = KEY.matches(s)

    /** Packages agents may not act on: Zeron itself (it would be switching its own permissions) and the install prompts. */
    fun protectedPackage(pkg: String, own: String): Boolean = pkg == own || pkg in PhoneAccessibilityService.BLOCKED_PACKAGES

    fun requirePackage(pkg: String, own: String): String {
        if (!isPackage(pkg)) throw PhoneToolException("package is an app's id like com.example.app")
        if (protectedPackage(pkg, own)) throw PhoneToolException("$pkg is never changed by agents")
        return pkg
    }

    /** A file Shizuku's shell may be pointed at: shared storage only, no odd characters. */
    fun requireApkPath(path: String): String {
        val ok = path.startsWith("/storage/") && path.endsWith(".apk", ignoreCase = true) && path.none { it == '\n' || it == '\u0000' } && !path.split('/').contains("..")
        if (!ok) throw PhoneToolException("path is an .apk file in shared storage (/storage/emulated/0/…)")
        return path
    }

    /** `pm install` reading the apk from stdin (`-S <size>`): the caller feeds the file. */
    fun install(path: String, size: Long): List<String> {
        requireApkPath(path)
        if (size <= 0) throw PhoneToolException("the apk is empty")
        return listOf("pm", "install", "-r", "--user", "0", "-S", size.toString())
    }

    fun grant(pkg: String, permission: String, revoke: Boolean, own: String): List<String> {
        requirePackage(pkg, own)
        if (!isPermission(permission)) throw PhoneToolException("permission is an Android permission like android.permission.READ_CONTACTS")
        return listOf("pm", if (revoke) "revoke" else "grant", "--user", "0", pkg, permission)
    }

    fun forceStop(pkg: String, own: String): List<String> = listOf("am", "force-stop", "--user", "0", requirePackage(pkg, own))

    fun settingsGet(namespace: String, key: String): List<String> {
        if (namespace !in NAMESPACES) throw PhoneToolException("namespace is system, secure or global")
        if (!isSettingsKey(key)) throw PhoneToolException("key is a setting's name like screen_off_timeout")
        return listOf("settings", "get", namespace, key)
    }
}
