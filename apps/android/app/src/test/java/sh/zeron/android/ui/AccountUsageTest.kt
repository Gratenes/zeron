package sh.zeron.android.ui

import androidx.compose.runtime.snapshots.Snapshot
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import uniffi.zeron_core.ContextUsage
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.DeviceAccountInfo
import uniffi.zeron_core.NoHandle
import uniffi.zeron_core.UsageWindowInfo

private fun w(label: String, used: Float) = UsageWindowInfo(label, used, null)

private fun login(
    id: String,
    active: Boolean = false,
    usage: List<UsageWindowInfo> = emptyList(),
    provider: String? = null,
    harness: String = "claude-code",
    plan: String? = null,
    switchable: Boolean = true,
) = DeviceAccountInfo(
    id = id,
    harness = harness,
    email = "$id@example.com",
    displayName = null,
    planLabel = plan,
    active = active,
    switchable = switchable,
    apiKey = false,
    organization = null,
    provider = provider,
    usage = usage,
    usageError = null,
)

class AccountUsageTest {
    // The cache is app-wide: each test starts signed out.
    @Before fun signedOut() = DeviceAccountsCache.reset()

    @Test fun noLoginInUseWhenNoneIsActive() {
        assertNull(loginInUse(emptyList()))
        assertNull(loginInUse(listOf(login("a"), login("b"))))
    }

    @Test fun theActiveLoginIsInUse() {
        val logins = listOf(login("a"), login("b", active = true), login("c"))
        assertEquals("b", loginInUse(logins)?.id)
    }

    @Test fun perProviderLoginsLeadWithTheOneReportingUsage() {
        // OpenCode: one login in use per model provider; only some report plan usage.
        val logins = listOf(
            login("anthropic-key", active = true, provider = "anthropic", harness = "opencode"),
            login("openai-1", active = true, provider = "openai", harness = "opencode", usage = listOf(w("5h", 0.4f))),
            login("openai-2", provider = "openai", harness = "opencode", usage = listOf(w("5h", 0.1f))),
        )
        assertEquals("openai-1", loginInUse(logins)?.id)
        // None reporting: the first in use still drives the pill (it shows the plan's name).
        assertEquals("a", loginInUse(listOf(login("a", active = true, provider = "x"), login("b", active = true, provider = "y")))?.id)
    }

    @Test fun peakUsageIsTheBusiestWindowClamped() {
        assertNull(login("a").peakUsage)
        assertEquals(0.78f, login("a", usage = listOf(w("Session", 0.42f), w("Week", 0.78f))).peakUsage!!, 0f)
        assertEquals(1f, login("a", usage = listOf(w("Session", 1.3f))).peakUsage!!, 0f)
        assertEquals(0f, login("a", usage = listOf(w("Session", -0.2f))).peakUsage!!, 0f)
    }

    @Test fun pillReadsTheBusiestWindowElseThePlan() {
        assertEquals("78%", pillLabel(login("a", usage = listOf(w("Session", 0.42f), w("Week", 0.78f)))))
        assertEquals("Max", pillLabel(login("a", plan = "Max")))
        assertEquals("Plan", pillLabel(login("a")))
    }

    @Test fun usageLevelThresholds() {
        assertEquals(UsageLevel.Normal, usageLevel(0f))
        assertEquals(UsageLevel.Normal, usageLevel(0.69f))
        assertEquals(UsageLevel.High, usageLevel(0.7f))
        assertEquals(UsageLevel.High, usageLevel(0.89f))
        assertEquals(UsageLevel.Critical, usageLevel(0.9f))
        assertEquals(UsageLevel.Critical, usageLevel(1f))
    }

    @Test fun switcherListsTheInUseLoginsGroup() {
        val a = login("a", active = true)
        val logins = listOf(a, login("b"), login("c", switchable = false))
        assertEquals(listOf("a", "b", "c"), switchChoices(logins, a, switches = true).map { it.id })
    }

    @Test fun switcherIsEmptyForASingleLoginOrAnAgentThatCantSwitch() {
        val a = login("a", active = true)
        assertEquals(emptyList<DeviceAccountInfo>(), switchChoices(listOf(a), a, switches = true))
        assertEquals(emptyList<DeviceAccountInfo>(), switchChoices(listOf(a, login("b")), a, switches = false))
    }

    @Test fun switcherKeepsToTheInUseLoginsProvider() {
        val openai = login("openai-1", active = true, provider = "openai", harness = "opencode")
        val logins = listOf(
            login("anthropic-1", active = true, provider = "anthropic", harness = "opencode"),
            openai,
            login("openai-2", provider = "openai", harness = "opencode"),
        )
        assertEquals(listOf("openai-1", "openai-2"), switchChoices(logins, openai, switches = true).map { it.id })
        // Alone for its provider: nothing to switch to.
        assertEquals(emptyList<DeviceAccountInfo>(), switchChoices(logins, logins[0], switches = true))
    }

    @Test fun switchingMovesInUseWithinItsGroupOnly() {
        val list = listOf(
            login("c1", active = true),
            login("c2"),
            login("x1", active = true, harness = "codex"),
            login("o1", active = true, provider = "openai", harness = "opencode"),
            login("o2", provider = "openai", harness = "opencode"),
            login("a1", active = true, provider = "anthropic", harness = "opencode"),
        )
        val active = { l: List<DeviceAccountInfo> -> l.filter { it.active }.map { it.id } }
        assertEquals(listOf("c2", "x1", "o1", "a1"), active(withActive(list, list[1])))
        assertEquals(listOf("c1", "x1", "o2", "a1"), active(withActive(list, list[4])))
    }

    @Test fun contextChipShowsFromHalfFullUnlessThePillCarriesIt() {
        val half = ContextUsage(100_000uL, 200_000uL)
        assertEquals(0.5f, contextChipFraction(half, pillShown = false)!!, 0f)
        assertNull(contextChipFraction(half, pillShown = true))
        assertNull(contextChipFraction(ContextUsage(90_000uL, 200_000uL), pillShown = false))
        assertNull(contextChipFraction(ContextUsage(null, 200_000uL), pillShown = false))
        assertNull(contextChipFraction(ContextUsage(10uL, 0uL), pillShown = false))
    }

    @Test fun aReaderThatFoundNothingIsToldWhenTheListLands() {
        // A page opened before any list (a cold start on the accounts page) must still recompose when one lands.
        val core = CoreClient(NoHandle)
        val reads = mutableListOf<Any>()
        val before = Snapshot.observe(readObserver = { reads += it }) { DeviceAccountsCache.get(core, "dev") }
        assertNull(before)
        assertTrue(reads.isNotEmpty())
        DeviceAccountsCache.put(core, "dev", listOf(login("a", active = true)))
        assertEquals("a", DeviceAccountsCache.get(core, "dev")?.single()?.id)
        // Another sign-in: nothing of the last one carries over.
        assertNull(DeviceAccountsCache.get(CoreClient(NoHandle), "dev"))
    }

    @Test fun signOutForgetsTheListsTheThrottleAndTheClient() {
        val core = CoreClient(NoHandle)
        DeviceAccountsCache.put(core, "dev", listOf(login("a", active = true)))
        assertTrue(DeviceAccountsCache.takeForce("dev", explicit = false))
        DeviceAccountsCache.reset()
        // Even for the very same client: nothing of the signed-out account answers.
        assertNull(DeviceAccountsCache.get(core, "dev"))
        assertTrue(DeviceAccountsCache.takeForce("dev", explicit = false))
    }
}
