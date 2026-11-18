package sh.zeron.android.phone

import android.content.Context
import android.content.pm.PackageManager
import java.security.MessageDigest
import org.json.JSONArray
import org.json.JSONObject

/**
 * The typed system actions: install an .apk, grant or revoke a runtime permission, force-stop an app, read a
 * setting, and a read-only probe. The shell ones go through Shizuku ([PhoneShizuku]); installing falls back to
 * Android's PackageInstaller (the person taps Install) when Shizuku isn't available. Every install is confirmed
 * (unless the person turned that off or said "always" for the chat) and written to the audit log.
 */
class PhoneSystem(private val context: Context, private val settings: PhoneSettings) {
    private val own = context.packageName

    private fun shell(argv: List<String>, timeoutMs: Int = 30_000): PhoneShizuku.Result {
        if (!settings.groupOn(PhoneSettings.Group.SHELL)) throw PhoneToolException("Allow Shizuku shell is switched off on this phone (Zeron → Settings → Agent phone access)")
        return PhoneShizuku.run(context, argv, timeoutMs)
    }

    private fun sha256(f: java.io.File): String {
        val d = MessageDigest.getInstance("SHA-256")
        f.inputStream().use { i -> val b = ByteArray(64 * 1024); while (true) { val n = i.read(b); if (n < 0) break; d.update(b, 0, n) } }
        return d.digest().joinToString("") { "%02x".format(it) }
    }

    fun install(path: String, expectedSha: String?, who: String?, chatId: String?): JSONObject {
        ShellArgs.requireApkPath(path)
        val file = PhoneFiles(context).readableFile(path)
        if (file.length() > 600_000_000L) throw PhoneToolException("the apk is over 600 MB")
        val actual = sha256(file)
        if (!expectedSha.isNullOrBlank() && !actual.equals(expectedSha.trim(), ignoreCase = true)) {
            audit(path, null, null, actual, who, chatId, "refused: sha256 is $actual, not the one given")
            throw PhoneToolException("the apk's sha256 is $actual, not the one given: nothing was installed")
        }
        val pm = context.packageManager
        @Suppress("DEPRECATION")
        val info = pm.getPackageArchiveInfo(file.path, PackageManager.GET_SIGNING_CERTIFICATES)
            ?: run { audit(path, null, null, actual, who, chatId, "refused: not a valid apk"); throw PhoneToolException("that file isn't a valid, readable apk") }
        info.applicationInfo?.let { it.sourceDir = file.path; it.publicSourceDir = file.path }
        val pkg = info.packageName
        if (ShellArgs.protectedPackage(pkg, own)) { audit(path, pkg, null, actual, who, chatId, "refused: protected package"); throw PhoneToolException("$pkg is never installed or replaced by agents") }
        val label = info.applicationInfo?.loadLabel(pm)?.toString() ?: pkg
        val signer = info.signingInfo?.apkContentsSigners?.firstOrNull()?.let { s -> MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).joinToString("") { "%02x".format(it) } }
        val version = info.versionName ?: info.longVersionCode.toString()
        val installed = runCatching { pm.getPackageInfo(pkg, 0).versionName }.getOrNull()

        val key = chatId ?: who ?: "?"
        if (settings.confirmInstalls && key !in settings.trustedInstallChats) {
            val text = "$label ($pkg) $version${installed?.let { ", replacing $it" } ?: ""}\nsigned by ${signer?.take(16) ?: "?"}…\nfile sha256 ${actual.take(16)}…\n$path"
            val choice = PhoneConfirm.choose(context, "${who ?: "An agent"} wants to install an app", text, "Install", "Deny", if (chatId != null || who != null) "Always for this chat" else null)
            if (choice == PhoneConfirm.NO) { audit(path, pkg, signer, actual, who, chatId, "denied by the person"); throw PhoneToolException("the person denied the install (or didn't answer in ${PhoneConfirm.WAIT_SECONDS} s)") }
            if (choice == PhoneConfirm.ALWAYS) settings.trustedInstallChats = settings.trustedInstallChats + key
        }

        val via: String
        val output: String
        val ok: Boolean
        if (settings.groupOn(PhoneSettings.Group.SHELL) && PhoneShizuku.problem(context) == null) {
            via = "shizuku"
            val r = PhoneShizuku.run(context, ShellArgs.install(file.path), 120_000)
            ok = r.exit == 0 && r.output.contains("Success")
            output = r.output.trim().take(300)
        } else {
            via = "package installer"
            val r = PhoneInstaller(context).install(file, pkg)
            ok = r.first
            output = r.second
        }
        audit(path, pkg, signer, actual, who, chatId, "${if (ok) "installed" else "failed"} via $via: $output")
        if (!ok) throw PhoneToolException("the install failed ($via): $output")
        return JSONObject().put("ok", true).put("package", pkg).put("label", label).put("version", version).put("replaced", installed ?: JSONObject.NULL)
            .put("signerSha256", signer ?: JSONObject.NULL).put("apkSha256", actual).put("via", via).put("note", output)
    }

    private fun audit(path: String, pkg: String?, signer: String?, sha: String, who: String?, chatId: String?, result: String) {
        PhoneActionLog.add(context, "install_apk", "${pkg ?: path.substringAfterLast('/')} signer ${signer?.take(12) ?: "?"} sha256 ${sha.take(12)} chat ${chatId ?: who ?: "?"} → $result".take(300), result.startsWith("installed"), who)
    }

    fun grant(pkg: String, permission: String, revoke: Boolean): JSONObject {
        val r = shell(ShellArgs.grant(pkg, permission, revoke, own))
        if (r.exit != 0) throw PhoneToolException("pm ${if (revoke) "revoke" else "grant"} failed: ${r.output.trim().take(300)}")
        return JSONObject().put("ok", true).put("package", pkg).put("permission", permission).put("action", if (revoke) "revoked" else "granted")
    }

    fun forceStop(pkg: String): JSONObject {
        val r = shell(ShellArgs.forceStop(pkg, own))
        if (r.exit != 0) throw PhoneToolException("force-stop failed: ${r.output.trim().take(300)}")
        return JSONObject().put("ok", true).put("package", pkg).put("stopped", true)
    }

    fun settingsGet(namespace: String, key: String): JSONObject {
        val r = shell(ShellArgs.settingsGet(namespace, key))
        if (r.exit != 0) throw PhoneToolException("settings get failed: ${r.output.trim().take(300)}")
        val v = r.output.trim()
        return JSONObject().put("namespace", namespace).put("key", key).put("value", if (v == "null") JSONObject.NULL else v)
    }

    /**
     * Read-only: the carrier's name, and whether the shell user can read the voicemail provider and whether its rows
     * carry transcriptions. Only counts and column names come back, never a voicemail's content or number.
     */
    fun probe(): JSONObject {
        val carrier = shell(listOf("getprop", "gsm.sim.operator.alpha")).output.trim()
        val numeric = shell(listOf("getprop", "gsm.sim.operator.numeric")).output.trim()
        val vm = shell(listOf("content", "query", "--uri", "content://com.android.voicemail/voicemail"))
        val rows = vm.output.lines().filter { it.startsWith("Row:") }
        val columns = rows.firstOrNull()?.removePrefix("Row:")?.substringAfter(' ')?.split(", ")?.map { it.substringBefore('=').trim() }.orEmpty()
        fun populated(col: String) = rows.count { r -> Regex("[ ,]$col=([^,]*)").find(r)?.groupValues?.get(1)?.let { it.isNotBlank() && it != "NULL" } == true }
        val result = JSONObject().put("carrier", carrier).put("simOperatorNumeric", numeric)
        result.put("voicemail", JSONObject()
            .put("exitCode", vm.exit).put("rowCount", rows.size).put("columns", JSONArray(columns))
            .put("withTranscription", populated("transcription")).put("withSourceData", populated("source_data"))
            .put("error", if (vm.exit != 0 || rows.isEmpty()) vm.output.trim().take(300) else JSONObject.NULL))
        return result
    }
}
