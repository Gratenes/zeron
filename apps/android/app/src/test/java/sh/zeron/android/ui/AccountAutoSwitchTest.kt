package sh.zeron.android.ui

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.yield
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.zeron_core.AccountAutoSwitchInfo
import uniffi.zeron_core.AccountSwitchInfo
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.CoreException
import uniffi.zeron_core.NoHandle

private fun status(enabled: Boolean, threshold: Int = 90, last: AccountSwitchInfo? = null) =
    AccountAutoSwitchInfo(enabled, threshold.toUInt(), last)

/** A client whose switching calls answer from the test (UniFFI's fake-object constructor). */
private class SwitchingCore : CoreClient(NoHandle) {
    val calls = mutableListOf<String>()
    var get: suspend () -> AccountAutoSwitchInfo = { status(false) }
    var set: suspend (Boolean, UInt) -> AccountAutoSwitchInfo = { on, t -> status(on, t.toInt()) }

    override suspend fun deviceAccountAutoSwitch(deviceId: String): AccountAutoSwitchInfo {
        calls += "get"
        return get()
    }

    override suspend fun deviceSetAccountAutoSwitch(deviceId: String, enabled: Boolean, thresholdPercent: UInt): AccountAutoSwitchInfo {
        calls += "set $enabled $thresholdPercent"
        return set(enabled, thresholdPercent)
    }
}

class AccountAutoSwitchTest {
    @Test fun aThresholdSetElsewhereStaysListed() {
        assertEquals(AUTO_SWITCH_THRESHOLDS, autoSwitchThresholds(90))
        assertEquals(listOf(70, 75, 80, 85, 90, 93, 95, 98), autoSwitchThresholds(93))
    }

    @Test fun theRowNamesTheThresholdUntilASwitchHappens() {
        val name = { h: String -> if (h == "codex") "Codex" else h }
        val clock = { _: Long -> "3:04 PM" }
        assertEquals(
            "At 85% of a usage limit, switch to the saved account with the most left",
            autoSwitchDetail(status(true, 85), name, clock),
        )
        val last = AccountSwitchInfo("codex", "a@example.com", "b@example.com", 0.9f, 0)
        assertEquals("Switched Codex to b@example.com at 3:04 PM", autoSwitchDetail(status(true, 85, last), name, clock))
        assertEquals("Switched Codex to another account at 3:04 PM", autoSwitchDetail(status(true, 85, last.copy(toEmail = null)), name, clock))
        // Turned off since: back to the threshold.
        assertTrue(autoSwitchDetail(status(false, 85, last), name, clock).startsWith("At 85%"))
    }

    @Test fun aSaveShowsAtOnceAndTakesTheComputersReply() = runBlocking {
        val core = SwitchingCore()
        // The computer clamps what it keeps.
        core.set = { on, _ -> status(on, 99) }
        val state = AccountAutoSwitchState(core, "dev", this) { "Studio" }
        state.load()
        state.setEnabled(true) { error(it) }
        assertTrue(state.status!!.enabled)
        assertFalse(state.interactive)
        while (state.saving) yield()
        assertEquals(status(true, 99), state.status)
        assertEquals(listOf("get", "set true 90"), core.calls)
    }

    @Test fun aRefusedSavePutsTheSettingBack() = runBlocking {
        val core = SwitchingCore()
        core.get = { status(true, 90) }
        val answer = CompletableDeferred<AccountAutoSwitchInfo>()
        core.set = { _, _ -> answer.await() }
        val errors = mutableListOf<String>()
        val state = AccountAutoSwitchState(core, "dev", this) { "Studio" }
        state.load()
        state.setThreshold(80) { errors += it }
        assertEquals(80u, state.status!!.thresholdPercent)
        // No second save while one is in flight.
        state.setEnabled(false) { errors += it }
        answer.completeExceptionally(CoreException.HostException("disk full"))
        while (state.saving) yield()
        assertEquals(status(true, 90), state.status)
        assertEquals(listOf("Couldn't save: disk full"), errors)
        assertEquals(listOf("get", "set true 80"), core.calls)
        assertTrue(state.interactive)
    }

    @Test fun anOlderComputerAsksForAnUpdateAndStaysOff() = runBlocking {
        val core = SwitchingCore()
        core.get = { throw CoreException.Unsupported("GetAccountAutoSwitch on dev") }
        val state = AccountAutoSwitchState(core, "dev", this) { "Studio" }
        state.load()
        assertTrue(state.unsupported)
        assertFalse(state.interactive)
        state.setEnabled(true) { error(it) }
        assertEquals(listOf("get"), core.calls)
        assertEquals("Update Zeron on Studio to switch accounts automatically.", autoSwitchUpdateNote("Studio"))
    }
}
