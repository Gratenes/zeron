package sh.zeron.android.ui

import android.content.Context
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.Stable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.HarnessMark
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.AgentAccount
import uniffi.zeron_core.AgentAccountsSnapshot
import uniffi.zeron_core.AgentLoginMode
import uniffi.zeron_core.AgentLoginOption
import uniffi.zeron_core.AgentLoginStatus
import uniffi.zeron_core.AgentProvider
import uniffi.zeron_core.AgentUsageWindow
import uniffi.zeron_core.ComposerState
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.agentProviders
import uniffi.zeron_core.isOpenableLoginUrl

/** Forced probes hit the provider; never ask more often than this (desktop parity). */
private const val FORCE_MIN_INTERVAL_MS = 30_000L
/** Background re-probe while a session is open: usage moves as turns run. */
private const val POLL_INTERVAL_MS = 5 * 60_000L
private const val LOGIN_POLL_MS = 1_500L

private val PROVIDERS: List<AgentProvider> by lazy { agentProviders() }

fun agentProvider(harness: String?): AgentProvider? = PROVIDERS.firstOrNull { it.harness == harness }

/** What a host error reads as, without the RPC plumbing around it. */
private fun friendly(e: Throwable): String {
    val raw = e.message ?: return "Something went wrong"
    return raw.removePrefix("reason=").substringAfter(": ", raw).trim().ifEmpty { raw }
}

/** One sign-in in progress (start → browser or pasted code → done). */
@Stable
class AgentLogin(val harness: String, val option: AgentLoginOption) {
    var loginId by mutableStateOf<String?>(null)
    var url by mutableStateOf<String?>(null)
    var mode by mutableStateOf(AgentLoginMode.BROWSER)
    var callbackPort by mutableStateOf<UShort?>(null)
    var message by mutableStateOf<String?>(null)
    var failed by mutableStateOf<String?>(null)
    var submitting by mutableStateOf(false)

    val title: String
        get() {
            val name = agentProvider(harness)?.name ?: harness
            return if (option.provider != null) "Sign in to ${option.label} for $name" else "Sign in to $name"
        }
}

/**
 * The agent logins of one execution device: loads (cached first, then a
 * forced usage probe), switches, forgets and signs in. Logins belong to the
 * device, so every session of an agent on it shares the live one.
 */
@Stable
class AgentAccountsState(private val core: CoreClient, val deviceId: String, private val scope: CoroutineScope) {
    var snapshot by mutableStateOf<AgentAccountsSnapshot?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var refreshing by mutableStateOf(false)
        private set
    /** Account id with a switch/forget in flight. */
    var busy by mutableStateOf<String?>(null)
        private set
    var login by mutableStateOf<AgentLogin?>(null)
        private set
    private var lastForced = 0L
    private var generation = 0

    fun accounts(harness: String): List<AgentAccount> = snapshot?.accounts.orEmpty().filter { it.harness == harness }

    fun active(harness: String?): AgentAccount? = snapshot?.accounts?.firstOrNull { it.harness == harness && it.active }

    suspend fun load(force: Boolean, explicit: Boolean = false) {
        val now = System.currentTimeMillis()
        val forced = force && (explicit || now - lastForced >= FORCE_MIN_INTERVAL_MS)
        if (forced) lastForced = now
        val gen = ++generation
        if (explicit) refreshing = true
        try {
            val next = core.listAgentAccounts(deviceId, forced)
            if (gen == generation) {
                snapshot = next
                error = null
            }
        } catch (e: Exception) {
            if (gen == generation) error = friendly(e)
        } finally {
            if (explicit) refreshing = false
        }
    }

    fun refresh() {
        scope.launch { load(force = true, explicit = true) }
    }

    /** A throttled usage probe without the spinner (opening the quick switcher). */
    fun refreshQuietly() {
        scope.launch { load(force = true) }
    }

    fun activate(account: AgentAccount, onError: (String) -> Unit = {}) {
        if (account.active || busy != null) return
        val before = snapshot
        // Optimistic: the tapped login becomes live within its group.
        snapshot = before?.copy(
            accounts = before.accounts.map {
                if (it.harness == account.harness && it.provider == account.provider) it.copy(active = it.id == account.id) else it
            },
        )
        busy = account.id
        generation++
        scope.launch {
            try {
                snapshot = core.activateAgentAccount(deviceId, account.harness, account.id)
            } catch (e: Exception) {
                snapshot = before
                onError("Couldn't switch: ${friendly(e)}")
            } finally {
                busy = null
            }
        }
    }

    fun forget(account: AgentAccount, onError: (String) -> Unit = {}) {
        if (busy != null) return
        busy = account.id
        generation++
        scope.launch {
            try {
                snapshot = core.forgetAgentAccount(deviceId, account.harness, account.id)
            } catch (e: Exception) {
                onError("Couldn't remove: ${friendly(e)}")
            } finally {
                busy = null
            }
        }
    }

    fun startLogin(context: Context, harness: String, option: AgentLoginOption) {
        login?.let { cancelLogin() }
        val flow = AgentLogin(harness, option)
        login = flow
        scope.launch {
            try {
                val start = core.startAgentLogin(deviceId, harness, option.provider)
                if (login !== flow) {
                    runCatching { core.cancelAgentLogin(deviceId, start.loginId) }
                    return@launch
                }
                flow.loginId = start.loginId
                flow.mode = start.mode
                flow.callbackPort = start.callbackPort
                if (start.url.isNotEmpty()) {
                    flow.url = start.url
                    openLoginUrl(context, start.url)
                }
                if (start.mode == AgentLoginMode.BROWSER) poll(context, flow)
            } catch (e: Exception) {
                if (login === flow) flow.failed = friendly(e)
            }
        }
    }

    private suspend fun poll(context: Context, flow: AgentLogin) {
        val id = flow.loginId ?: return
        while (scope.isActive && login === flow) {
            delay(LOGIN_POLL_MS)
            if (login !== flow) return
            val poll = try {
                core.pollAgentLogin(deviceId, id)
            } catch (e: Exception) {
                flow.failed = friendly(e)
                return
            }
            when (poll.status) {
                AgentLoginStatus.PENDING -> {
                    poll.message?.let { flow.message = it }
                    val late = poll.url
                    if (late != null && flow.url == null) {
                        flow.url = late
                        flow.callbackPort = poll.callbackPort
                        openLoginUrl(context, late)
                    }
                }
                AgentLoginStatus.DONE -> {
                    login = null
                    load(force = true, explicit = true)
                    return
                }
                AgentLoginStatus.ERROR -> {
                    flow.failed = poll.message ?: "Sign-in failed"
                    return
                }
            }
        }
    }

    fun submitCode(code: String) {
        val flow = login ?: return
        val id = flow.loginId ?: return
        if (code.isBlank() || flow.submitting) return
        flow.submitting = true
        flow.message = null
        scope.launch {
            try {
                snapshot = core.completeAgentLogin(deviceId, id, code.trim())
                if (login === flow) login = null
                load(force = true, explicit = true)
            } catch (e: Exception) {
                flow.message = friendly(e)
            } finally {
                flow.submitting = false
            }
        }
    }

    fun retryLogin(context: Context) {
        val flow = login ?: return
        startLogin(context, flow.harness, flow.option)
    }

    fun cancelLogin() {
        val flow = login ?: return
        login = null
        val id = flow.loginId ?: return
        scope.launch { runCatching { core.cancelAgentLogin(deviceId, id) } }
    }
}

fun openLoginUrl(context: Context, url: String) {
    if (!isOpenableLoginUrl(url)) return
    runCatching { CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(context, Uri.parse(url)) }
}

/**
 * Accounts of `deviceId`, loaded once with cached usage then probed, and
 * re-probed every few minutes while on screen.
 */
@Composable
fun rememberAgentAccounts(core: CoreClient, deviceId: String, enabled: Boolean = true): AgentAccountsState {
    val scope = rememberCoroutineScope()
    val state = remember(core, deviceId) { AgentAccountsState(core, deviceId, scope) }
    LaunchedEffect(state, enabled) {
        if (!enabled || deviceId.isEmpty()) return@LaunchedEffect
        state.load(force = false)
        state.load(force = true)
        while (isActive) {
            delay(POLL_INTERVAL_MS)
            state.load(force = true)
        }
    }
    return state
}

// ── visuals ───────────────────────────────────────────────────────────────

private fun warningColor(dark: Boolean) = if (dark) Color(0xFFFBBF24) else Color(0xFFB45309)

@Composable
fun usageColor(fraction: Float): Color = when {
    fraction >= 0.9f -> MaterialTheme.colorScheme.error
    fraction >= 0.75f -> warningColor(LocalDarkTheme.current)
    else -> MaterialTheme.colorScheme.primary
}

private fun percent(fraction: Float) = "${(fraction.coerceIn(0f, 1f) * 100).toInt()}%"

private fun compactTokens(n: ULong): String {
    val v = n.toDouble()
    return when {
        v >= 1_000_000 -> String.format(java.util.Locale.US, "%.1fM", v / 1_000_000).replace(".0M", "M")
        v >= 1_000 -> "${(v / 1_000).toInt()}k"
        else -> n.toString()
    }
}

@Composable
private fun UsageMeter(label: String, fraction: Float, detail: String?) {
    val f = fraction.coerceIn(0f, 1f)
    Column(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = MaterialTheme.typography.labelLarge, modifier = Modifier.weight(1f))
            Text(percent(f), style = MaterialTheme.typography.labelLarge, color = if (f >= 0.75f) usageColor(f) else MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Spacer(Modifier.height(6.dp))
        LinearProgressIndicator(
            progress = { if (f > 0f) maxOf(f, 0.015f) else 0f },
            modifier = Modifier.fillMaxWidth().height(6.dp),
            color = usageColor(f),
            trackColor = MaterialTheme.colorScheme.surfaceContainerHighest,
            strokeCap = StrokeCap.Round,
            gapSize = 0.dp,
            drawStopIndicator = {},
        )
        detail?.let {
            Spacer(Modifier.height(4.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun PlanMeters(windows: List<AgentUsageWindow>) {
    windows.forEach { w -> UsageMeter(w.label, w.usedFraction, w.resetLabel?.replaceFirstChar { it.uppercase() }) }
}

private fun accountTitle(a: AgentAccount): String =
    a.email ?: a.displayName ?: if (a.apiKey) "API key" else "Signed in"

private fun accountDetail(a: AgentAccount, groups: Boolean): String? = listOfNotNull(
    a.planLabel,
    a.organization,
    if (groups) a.provider?.let { providerLabel(a.harness, it) } else null,
    if (a.apiKey && a.email != null) "API key" else null,
).joinToString(" · ").ifEmpty { null }

private fun providerLabel(harness: String, provider: String): String =
    agentProvider(harness)?.loginOptions?.firstOrNull { it.provider == provider }?.label ?: provider

@Composable
private fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmallEmphasized,
        color = MaterialTheme.colorScheme.primary,
        modifier = modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp),
    )
}

/** An "In use" pill. */
@Composable
private fun ActivePill() {
    Surface(shape = RoundedCornerShape(50), color = MaterialTheme.colorScheme.primaryContainer, contentColor = MaterialTheme.colorScheme.onPrimaryContainer) {
        Text("In use", style = MaterialTheme.typography.labelMedium, modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp))
    }
}

/** A row per account of one agent, switchable by tap; `showMeters` adds plan usage under each. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun AccountRows(
    state: AgentAccountsState,
    provider: AgentProvider,
    accounts: List<AgentAccount>,
    onError: (String) -> Unit,
    allowForget: Boolean,
    extraRows: Int = 0,
) {
    val groups = accounts.mapNotNull { it.provider }.distinct().size > 1 || provider.loginOptions.size > 1
    val total = accounts.size + extraRows
    var menuFor by remember { mutableStateOf<String?>(null) }
    var confirmForget by remember { mutableStateOf<AgentAccount?>(null) }
    accounts.forEachIndexed { i, a ->
        val canSwitch = provider.switchesAccounts && a.switchable && !a.active
        Box {
            SegmentedListItem(
                onClick = { if (canSwitch) state.activate(a, onError) },
                onLongClick = if (allowForget && provider.switchesAccounts && a.switchable && !a.active) ({ menuFor = a.id }) else null,
                shapes = segmentedShapes(i, total),
                colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                leadingContent = {
                    if (provider.switchesAccounts && accounts.size > 1) {
                        if (state.busy == a.id) LoadingIndicator(Modifier.size(24.dp))
                        else RadioButton(selected = a.active, onClick = if (canSwitch) ({ state.activate(a, onError) }) else null, enabled = a.switchable)
                    } else {
                        Box(Modifier.size(40.dp).clip(RoundedCornerShape(14.dp)).background(MaterialTheme.colorScheme.secondaryContainer), contentAlignment = Alignment.Center) {
                            HarnessMark(a.harness, 22.dp)
                        }
                    }
                },
                supportingContent = {
                    Column {
                        accountDetail(a, groups)?.let { Text(it) }
                        if (provider.reportsUsage) {
                            if (a.usageWindows.isNotEmpty()) {
                                Spacer(Modifier.height(4.dp))
                                PlanMeters(a.usageWindows)
                            }
                            a.usageError?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
                                ?: if (a.usageWindows.isEmpty() && a.active && !a.apiKey) Text("Usage unavailable", style = MaterialTheme.typography.bodySmall) else Unit
                        }
                    }
                },
            ) {
                // The pill rides the title line so every row's meters share one width.
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(accountTitle(a), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                    if (a.active) {
                        Spacer(Modifier.width(8.dp))
                        ActivePill()
                    }
                }
            }
            ActionMenu(
                menuFor == a.id,
                { menuFor = null },
                listOf(
                    MenuAction("Switch to this account", ZIcons.Check) { state.activate(a, onError) },
                    MenuAction("Remove from device", ZIcons.Delete, destructive = true) { confirmForget = a },
                ),
            )
        }
    }
    confirmForget?.let { a ->
        AlertDialog(
            onDismissRequest = { confirmForget = null },
            icon = { ZIcon(ZIcons.Delete, null) },
            title = { Text("Remove ${accountTitle(a)}?") },
            text = { Text("Its saved login is deleted from this device. You can sign in again any time.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmForget = null
                    state.forget(a, onError)
                }) { Text("Remove", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmForget = null }) { Text("Cancel") } },
        )
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun AddAccountRows(state: AgentAccountsState, provider: AgentProvider, empty: Boolean, startIndex: Int, total: Int) {
    val context = LocalContext.current
    if (provider.keepsOneLogin && !empty) return
    provider.loginOptions.forEachIndexed { j, option ->
        val label = when {
            option.provider == null && empty -> "Connect a ${provider.name} account"
            option.provider == null -> "Add account"
            empty -> "Connect ${option.label}"
            else -> "Add ${option.label} account"
        }
        SegmentedListItem(
            onClick = { state.startLogin(context, provider.harness, option) },
            shapes = segmentedShapes(startIndex + j, total),
            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
            leadingContent = { IconTile(ZIcons.Plus) },
        ) { Text(label) }
    }
}

private fun addRows(provider: AgentProvider, empty: Boolean): Int =
    if (provider.keepsOneLogin && !empty) 0 else provider.loginOptions.size

// ── session sheet ─────────────────────────────────────────────────────────

/**
 * The session's usage: its context window and the plan usage of the agent
 * login it runs as on its host, with that agent's other logins a tap away.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SessionUsageSheet(
    model: AppModel,
    state: AgentAccountsState,
    composer: ComposerState,
    harness: String?,
    harnessLabel: String?,
    onManage: () -> Unit,
    onDismiss: () -> Unit,
) {
    val provider = agentProvider(harness)
    val hostName = composer.host.name ?: "the host"
    LaunchedEffect(state) { state.load(force = true) }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp)
                .navigationBarsPadding(),
        ) {
            Row(Modifier.padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("Usage", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.weight(1f))
                if (state.refreshing) LoadingIndicator(Modifier.size(32.dp))
                else TonalCircleButton(ZIcons.Refresh, "Refresh usage", onClick = { state.refresh() }, size = 40.dp)
            }

            // Context window of this session.
            SectionLabel("Context")
            Surface(shape = RoundedCornerShape(24.dp), color = cardColor(), modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 10.dp)) {
                    val tokens = composer.contextUsage?.tokens
                    val window = composer.contextUsage?.window
                    if (tokens != null && window != null && window > 0u) {
                        val f = (tokens.toDouble() / window.toDouble()).toFloat()
                        val left = if (window > tokens) window - tokens else 0uL
                        UsageMeter("Context window", f, "${compactTokens(tokens)} of ${compactTokens(window)} tokens · ${compactTokens(left)} left")
                    } else if (tokens != null) {
                        Text("${compactTokens(tokens)} tokens used", style = MaterialTheme.typography.bodyLarge, modifier = Modifier.padding(vertical = 6.dp))
                    } else {
                        Text("Not reported yet — it appears after the agent's next turn.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(vertical = 6.dp))
                    }
                }
            }

            if (provider != null) {
                val accounts = state.accounts(provider.harness)
                SectionLabel("${provider.name} account on $hostName")
                val snapshot = state.snapshot
                when {
                    snapshot == null && state.error != null -> ErrorCard(state.error!!) { state.refresh() }
                    snapshot == null -> Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { LoadingIndicator() }
                    else -> {
                        val extra = addRows(provider, accounts.isEmpty())
                        Column(verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                            AccountRows(state, provider, accounts, onError = { model.toast(it) }, allowForget = false, extraRows = extra)
                            AddAccountRows(state, provider, accounts.isEmpty(), accounts.size, accounts.size + extra)
                        }
                        state.error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(12.dp)) }
                        snapshot.warnings.filter { it.harness == provider.harness }.forEach {
                            Text(it.message, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp))
                        }
                        val note = provider.note ?: if (provider.switchesAccounts && accounts.size > 1) {
                            "Switching changes the login every ${provider.name} session on $hostName uses."
                        } else null
                        note?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) }
                    }
                }
            } else if (harnessLabel != null) {
                SectionLabel("Account")
                Text("$harnessLabel has no zeron-managed login.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 12.dp))
            }

            Spacer(Modifier.height(12.dp))
            SegmentedListItem(
                onClick = onManage,
                shapes = segmentedShapes(0, 1),
                colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                leadingContent = { IconTile(ZIcons.TabSettings) },
                supportingContent = { Text("Every agent's logins on $hostName") },
                trailingContent = { ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.outline) },
            ) { Text("Manage accounts") }
        }
    }
    AgentLoginDialog(state, hostName)
}

@Composable
private fun ErrorCard(message: String, onRetry: () -> Unit) {
    Surface(shape = RoundedCornerShape(24.dp), color = MaterialTheme.colorScheme.errorContainer, contentColor = MaterialTheme.colorScheme.onErrorContainer, modifier = Modifier.fillMaxWidth()) {
        Row(Modifier.padding(start = 16.dp, end = 4.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Couldn't load accounts: $message", style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f).padding(vertical = 8.dp))
            TextButton(onClick = onRetry) { Text("Retry") }
        }
    }
}

// ── device accounts screen ────────────────────────────────────────────────

/** Settings → device → Accounts: every agent's logins on one device. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun AgentAccountsScreen(model: AppModel, deviceId: String, onBack: () -> Unit) {
    val client by model.client.collectAsState()
    val core = client ?: return
    val workspace by model.workspace.collectAsState()
    val device = workspace?.devices?.firstOrNull { it.id == deviceId }
    val deviceName = device?.name ?: "this device"
    val state = rememberAgentAccounts(core, deviceId)
    // Agents installed there (cached list first): the rest only show once they have a login.
    val installed by produceState<Set<String>?>(null, core, deviceId) {
        value = runCatching { core.listHarnesses(deviceId).filter { it.offered }.map { it.id }.toSet() }.getOrNull()
    }
    val snapshot = state.snapshot
    val providers = PROVIDERS.filter { p -> installed?.contains(p.harness) != false || state.accounts(p.harness).isNotEmpty() }
    val list = rememberLazyListState()
    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        LazyColumn(state = list, contentPadding = PaddingValues(bottom = 48.dp), modifier = Modifier.fillMaxSize()) {
            item("header") {
                Column(Modifier.statusBarsPadding()) {
                    Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 12.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        TonalCircleButton(ZIcons.Back, "Back", onClick = onBack)
                        Spacer(Modifier.weight(1f))
                        if (state.refreshing) LoadingIndicator(Modifier.size(40.dp))
                        else TonalCircleButton(ZIcons.Refresh, "Refresh usage", onClick = { state.refresh() })
                    }
                    ScreenHeader("Accounts", "Agent logins on $deviceName")
                }
            }
            if (device != null && !device.online && !device.isSelf) {
                item("offline") {
                    Box(Modifier.padding(horizontal = 16.dp)) { StatusBanner("$deviceName is offline", null) }
                }
            }
            when {
                snapshot == null && state.error != null -> item("error") {
                    Box(Modifier.padding(horizontal = 16.dp)) { ErrorCard(state.error!!) { state.refresh() } }
                }
                snapshot == null -> item("loading") {
                    Box(Modifier.fillMaxWidth().padding(48.dp), contentAlignment = Alignment.Center) { LoadingIndicator() }
                }
                else -> providers.forEach { provider ->
                    item(provider.harness) {
                        val accounts = state.accounts(provider.harness)
                        Column(Modifier.padding(horizontal = 16.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                HarnessMark(provider.harness, 16.dp, Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp))
                                SectionLabel(provider.name, Modifier.padding(start = 0.dp))
                            }
                            val extra = addRows(provider, accounts.isEmpty())
                            Column(verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                                AccountRows(state, provider, accounts, onError = { model.toast(it) }, allowForget = true, extraRows = extra)
                                AddAccountRows(state, provider, accounts.isEmpty(), accounts.size, accounts.size + extra)
                            }
                            snapshot.warnings.filter { it.harness == provider.harness }.forEach {
                                Text(it.message, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp))
                            }
                            provider.note?.let {
                                Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp))
                            }
                        }
                    }
                    if (provider.harness == "pi") item("pi-providers") { PiProvidersSection(model, deviceId) }
                }
            }
            if (snapshot != null) {
                item("footer") {
                    Text(
                        "Switching changes which login every session of that agent on $deviceName uses. Long-press a saved login to remove it.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 28.dp, vertical = 20.dp),
                    )
                }
            }
        }
        StatusBarScrim(scrolled = list.firstVisibleItemIndex > 0 || list.firstVisibleItemScrollOffset > 0)
    }
    AgentLoginDialog(state, deviceName)
}

// ── sign-in dialog ────────────────────────────────────────────────────────

/** Sign-in progress: waiting on the browser, or the code to paste back. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun AgentLoginDialog(state: AgentAccountsState, deviceName: String) {
    val flow = state.login ?: return
    val context = LocalContext.current
    var code by remember(flow) { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = {},
        icon = { HarnessMark(flow.harness, 28.dp) },
        title = { Text(flow.title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                val failed = flow.failed
                when {
                    failed != null -> Text(failed, color = MaterialTheme.colorScheme.error)
                    flow.loginId == null -> Row(verticalAlignment = Alignment.CenterVertically) {
                        LoadingIndicator(Modifier.size(28.dp))
                        Spacer(Modifier.width(12.dp))
                        Text("Starting on $deviceName…")
                    }
                    flow.mode == AgentLoginMode.PASTE_CODE -> {
                        Text("Approve the sign-in in your browser, then paste the code it shows here.")
                        OutlinedTextField(
                            code,
                            { code = it },
                            label = { Text("Code") },
                            singleLine = true,
                            enabled = !flow.submitting,
                            isError = flow.message != null,
                            supportingText = flow.message?.let { { Text(it) } },
                            modifier = Modifier.fillMaxWidth(),
                        )
                        flow.url?.let { url -> TextButton(onClick = { openLoginUrl(context, url) }) { Text("Open sign-in page again") } }
                    }
                    else -> {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            LoadingIndicator(Modifier.size(28.dp))
                            Spacer(Modifier.width(12.dp))
                            Text(flow.message ?: "Finish signing in in your browser.")
                        }
                        if (flow.callbackPort != null) {
                            // The redirect lands on the host's own localhost, which this phone's browser can't reach.
                            Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainerHighest) {
                                Text(
                                    "This sign-in returns to $deviceName (localhost:${flow.callbackPort}). If the page can't load after you approve, finish it from Zeron on $deviceName or another computer.",
                                    style = MaterialTheme.typography.bodySmall,
                                    modifier = Modifier.padding(12.dp),
                                )
                            }
                        }
                    }
                }
            }
        },
        confirmButton = {
            when {
                flow.failed != null -> TextButton(onClick = { state.retryLogin(context) }) { Text("Try again") }
                flow.mode == AgentLoginMode.PASTE_CODE && flow.loginId != null ->
                    TextButton(onClick = { state.submitCode(code) }, enabled = code.isNotBlank() && !flow.submitting) {
                        if (flow.submitting) LoadingIndicator(Modifier.size(20.dp)) else Text("Sign in")
                    }
                flow.url != null -> TextButton(onClick = { openLoginUrl(context, flow.url!!) }) { Text("Open page") }
                else -> {}
            }
        },
        dismissButton = { TextButton(onClick = { state.cancelLogin() }) { Text("Cancel") } },
    )
}

/**
 * The composer's plan chip: a ring + percent of the session agent's live
 * login (its busiest window). Tapping opens a quick switcher — that login's
 * windows on top, every login of the agent below as one compact row with its
 * own busiest window — and a tap on a row switches to it.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun PlanUsageChip(
    state: AgentAccountsState?,
    harness: String?,
    onError: (String) -> Unit,
    onDetails: () -> Unit,
    context: uniffi.zeron_core.ContextUsage? = null,
) {
    if (!planUsageShown(state, harness)) return
    val provider = agentProvider(harness) ?: return
    val active = state?.active(harness) ?: return
    val fraction = active.usedFraction
    var open by remember { mutableStateOf(false) }
    ContextChip(
        fraction?.let { percent(it) } ?: "Plan",
        leading = { UsageRing(fraction ?: 0f, Modifier.size(16.dp)) },
        onClick = {
            open = true
            state.refreshQuietly()
        },
        tint = if ((fraction ?: 0f) >= 0.9f) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
    ) {
        androidx.compose.material3.DropdownMenuPopup(expanded = open, onDismissRequest = { open = false }) {
            androidx.compose.material3.DropdownMenuGroup(shapes = androidx.compose.material3.MenuDefaults.groupShape(0, 1)) {
            Column(Modifier.width(300.dp).padding(vertical = 4.dp)) {
                // The live login's windows.
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        HarnessMark(active.harness, 16.dp)
                        Spacer(Modifier.width(8.dp))
                        Text(accountTitle(active), style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                        active.planLabel?.let { Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                    if (active.usageWindows.isEmpty()) {
                        Text(active.usageError ?: "Usage unavailable", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp))
                    } else {
                        active.usageWindows.forEach { w -> CompactMeter(w.label, w.usedFraction, w.resetLabel) }
                        active.usageError?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
                    }
                }
                // This chat's context window (the composer shows one usage pill, not two).
                if (ContextUsageText.hasWindow(context)) {
                    androidx.compose.material3.HorizontalDivider(Modifier.padding(vertical = 4.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    Column(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
                        Text("This chat", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.primary)
                        CompactMeter("Context", (ContextUsageText.fraction(context) ?: 0.0).toFloat(), ContextUsageText.details(context).firstOrNull())
                    }
                }
                val accounts = state.accounts(provider.harness).filter { it.provider == active.provider }
                if (provider.switchesAccounts && accounts.size > 1) {
                    androidx.compose.material3.HorizontalDivider(Modifier.padding(vertical = 4.dp), color = MaterialTheme.colorScheme.outlineVariant)
                    Text("Switch account", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(start = 16.dp, top = 4.dp, bottom = 2.dp))
                    accounts.forEach { a -> QuickAccountRow(a, busy = state.busy == a.id) { state.activate(a, onError) } }
                }
                androidx.compose.material3.HorizontalDivider(Modifier.padding(vertical = 4.dp), color = MaterialTheme.colorScheme.outlineVariant)
                androidx.compose.material3.DropdownMenuItem(
                    text = { Text("Usage & accounts") },
                    leadingIcon = { ZIcon(ZIcons.Context, null, Modifier.size(20.dp)) },
                    onClick = {
                        open = false
                        onDetails()
                    },
                )
            }
            }
        }
    }
}

/** A small progress ring tinted by level. */
@Composable
private fun UsageRing(fraction: Float, modifier: Modifier = Modifier) {
    val f = fraction.coerceIn(0f, 1f)
    androidx.compose.material3.CircularProgressIndicator(
        progress = { if (f > 0f) maxOf(f, 0.04f) else 0f },
        modifier = modifier,
        color = usageColor(f),
        trackColor = MaterialTheme.colorScheme.outlineVariant,
        strokeWidth = 2.5.dp,
        gapSize = 0.dp,
    )
}

/** One window as a single line: label, bar, percent (reset under it). */
@Composable
private fun CompactMeter(label: String, fraction: Float, reset: String?) {
    val f = fraction.coerceIn(0f, 1f)
    Column(Modifier.padding(top = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, modifier = Modifier.width(64.dp))
            LinearProgressIndicator(
                progress = { if (f > 0f) maxOf(f, 0.015f) else 0f },
                modifier = Modifier.weight(1f).height(4.dp),
                color = usageColor(f),
                trackColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                strokeCap = StrokeCap.Round,
                gapSize = 0.dp,
                drawStopIndicator = {},
            )
            Text(percent(f), style = MaterialTheme.typography.labelMedium, color = if (f >= 0.75f) usageColor(f) else MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.width(44.dp), textAlign = androidx.compose.ui.text.style.TextAlign.End)
        }
        reset?.let { Text(it.replaceFirstChar { c -> c.uppercase() }, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.outline, modifier = Modifier.padding(start = 64.dp)) }
    }
}

/** A login in the quick switcher: who, plan, its busiest window; the live one checked. */
@Composable
private fun QuickAccountRow(a: AgentAccount, busy: Boolean, onSwitch: () -> Unit) {
    val f = a.usedFraction
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = !a.active && a.switchable && !busy, onClick = onSwitch)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) {
            when {
                busy -> LoadingIndicator(Modifier.size(20.dp))
                a.active -> ZIcon(ZIcons.Check, "In use", Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary)
            }
        }
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(accountTitle(a), style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, color = if (a.switchable || a.active) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.outline)
            a.planLabel?.let { Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
        if (f != null) {
            Spacer(Modifier.width(8.dp))
            LinearProgressIndicator(
                progress = { if (f > 0f) maxOf(f, 0.04f) else 0f },
                modifier = Modifier.width(44.dp).height(4.dp),
                color = usageColor(f),
                trackColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                strokeCap = StrokeCap.Round,
                gapSize = 0.dp,
                drawStopIndicator = {},
            )
            Text(percent(f), style = MaterialTheme.typography.labelMedium, color = if (f >= 0.75f) usageColor(f) else MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.width(44.dp), textAlign = androidx.compose.ui.text.style.TextAlign.End)
        }
    }
}

/** Whether [PlanUsageChip] shows for this agent; when it does it carries the chat's context window too. */
fun planUsageShown(state: AgentAccountsState?, harness: String?): Boolean {
    val provider = agentProvider(harness) ?: return false
    return state != null && provider.reportsUsage && state.active(harness) != null
}
