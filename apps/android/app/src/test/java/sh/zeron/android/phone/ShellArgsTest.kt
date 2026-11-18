package sh.zeron.android.phone

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class ShellArgsTest {
    private val own = "sh.zeron.android"

    private fun refused(block: () -> Unit) {
        try { block(); fail("should have been refused") } catch (_: PhoneToolException) { }
    }

    @Test fun packageNames() {
        assertTrue(ShellArgs.isPackage("com.example.app"))
        assertTrue(ShellArgs.isPackage("org.fdroid.fdroid"))
        for (bad in listOf("", "app", "com.example app", "com.example;rm -rf /", "com.example\nid", "-r", "../x.y", "com..x")) assertFalse(bad, ShellArgs.isPackage(bad))
    }

    @Test fun installPathsStayInStorageAndAreApks() {
        assertEquals(listOf("pm", "install", "-r", "--user", "0", "-S", "1234"), ShellArgs.install("/storage/emulated/0/Download/a.apk", 1234))
        for (bad in listOf("/data/local/tmp/a.apk", "/storage/emulated/0/a.txt", "/storage/emulated/0/../../data/a.apk", "/storage/emulated/0/a.apk\nrm", "relative.apk")) refused { ShellArgs.install(bad, 10) }
        refused { ShellArgs.install("/storage/emulated/0/a.apk", 0) }
    }

    @Test fun grantAndRevokeAreTypedArgvs() {
        assertEquals(listOf("pm", "grant", "--user", "0", "com.example.app", "android.permission.CAMERA"), ShellArgs.grant("com.example.app", "android.permission.CAMERA", false, own))
        assertEquals("revoke", ShellArgs.grant("com.example.app", "android.permission.CAMERA", true, own)[1])
        refused { ShellArgs.grant("com.example.app", "android.permission.CAMERA; id", false, own) }
        refused { ShellArgs.grant("com.example.app", "x", false, own) }
    }

    @Test fun zeronItselfAndTheInstallPromptsAreNeverTouched() {
        refused { ShellArgs.grant(own, "android.permission.READ_SMS", false, own) }
        refused { ShellArgs.forceStop(own, own) }
        refused { ShellArgs.forceStop("com.android.packageinstaller", own) }
        assertEquals(listOf("am", "force-stop", "--user", "0", "com.example.app"), ShellArgs.forceStop("com.example.app", own))
    }

    @Test fun settingsGet() {
        assertEquals(listOf("settings", "get", "global", "airplane_mode_on"), ShellArgs.settingsGet("global", "airplane_mode_on"))
        refused { ShellArgs.settingsGet("bogus", "x") }
        refused { ShellArgs.settingsGet("secure", "a; b") }
        refused { ShellArgs.settingsGet("secure", "") }
    }
}
