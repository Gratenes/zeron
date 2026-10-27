package sh.zeron.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.DropdownMenuGroup
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenuPopup
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import sh.zeron.android.core.CloudStore
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.AccountAutoSwitchInfo
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.CoreException
import uniffi.zeron_core.agentLoginRules
import java.text.DateFormat
import java.util.Date
import kotlin.coroutines.cancellation.CancellationException

/** The thresholds offered, in percent (the desktop's). */
val AUTO_SWITCH_THRESHOLDS = listOf(70, 75, 80, 85, 90, 95, 98)

/** The thresholds the menu lists: the offered ones, plus a saved one set elsewhere so it stays shown. */
fun autoSwitchThresholds(current: Int): List<Int> =
    if (current in AUTO_SWITCH_THRESHOLDS) AUTO_SWITCH_THRESHOLDS else (AUTO_SWITCH_THRESHOLDS + current).sorted()

/** What the switch row says under its title: the threshold, or — while on — the last switch. */
fun autoSwitchDetail(
    status: AccountAutoSwitchInfo,
    agentName: (String) -> String = { agentLoginRules(it).name.ifEmpty { it } },
    clock: (Long) -> String = { DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(it)) },
): String {
    val last = status.lastSwitch
    return if (last != null && status.enabled) {
        "Switched ${agentName(last.harness)} to ${last.toEmail ?: "another account"} at ${clock(last.atMs)}"
    } else {
        "At ${status.thresholdPercent}% of a usage limit, switch to the saved account with the most left"
    }
}

/** An engine from before automatic switching answers Unsupported. */
fun autoSwitchUpdateNote(deviceName: String) = "Update Zeron on $deviceName to switch accounts automatically."

/**
 * One computer's automatic account switching (the desktop's Settings →
 * Providers card): read once, saved optimistically, put back with an error
 * when the computer refuses. Cloud has no such setting.
 */
@Stable
class AccountAutoSwitchState(private val core: CoreClient, val deviceId: String, private val scope: CoroutineScope, private val deviceName: () -> String) {
    /** The computer's setting (null until read). */
    var status by mutableStateOf<AccountAutoSwitchInfo?>(null)
        private set
    /** Why it couldn't be read. */
    var error by mutableStateOf<String?>(null)
        private set
    /** The computer's Zeron predates the setting: the control stays off. */
    var unsupported by mutableStateOf(false)
        private set
    var saving by mutableStateOf(false)
        private set

    val interactive: Boolean
        get() = status != null && !saving && !unsupported

    suspend fun load() {
        try {
            status = core.deviceAccountAutoSwitch(deviceId)
            error = null
            unsupported = false
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            unsupported = e is CoreException.Unsupported
            error = CloudStore.message(e)
        }
    }

    fun setEnabled(on: Boolean, onError: (String) -> Unit) {
        status?.let { save(it.copy(enabled = on), onError) }
    }

    fun setThreshold(percent: Int, onError: (String) -> Unit) {
        status?.let { save(it.copy(thresholdPercent = percent.toUInt()), onError) }
    }

    private fun save(next: AccountAutoSwitchInfo, onError: (String) -> Unit) {
        val before = status ?: return
        if (!interactive || next == before) return
        // Optimistic: the reply replaces it.
        status = next
        saving = true
        scope.launch {
            try {
                status = core.deviceSetAccountAutoSwitch(deviceId, next.enabled, next.thresholdPercent)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                status = before
                unsupported = e is CoreException.Unsupported
                onError(if (unsupported) autoSwitchUpdateNote(deviceName()) else "Couldn't save: ${CloudStore.message(e)}")
            } finally {
                saving = false
            }
        }
    }
}

/** The switch, and — while on — the threshold it switches at. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun AccountAutoSwitchRows(state: AccountAutoSwitchState, deviceName: String, online: Boolean, onError: (String) -> Unit) {
    val status = state.status
    val enabled = status?.enabled == true
    val detail = when {
        state.unsupported -> autoSwitchUpdateNote(deviceName)
        status != null -> autoSwitchDetail(status)
        !online -> "$deviceName is offline."
        else -> state.error ?: "Loading…"
    }
    val count = if (enabled) 2 else 1
    CloudRow(
        0, count, "Switch accounts automatically", detail,
        leading = { IconTile(ZIcons.Refresh) },
        trailing = {
            if (status == null && online && state.error == null) {
                LoadingIndicator(Modifier.size(32.dp))
            } else {
                Switch(enabled, { state.setEnabled(it, onError) }, enabled = state.interactive)
            }
        },
        onClick = if (state.interactive) ({ state.setEnabled(!enabled, onError) }) else null,
    )
    if (status != null && enabled) {
        var menu by remember { mutableStateOf(false) }
        val current = status.thresholdPercent.toInt()
        CloudRow(
            1, count, "Switch at", "The most-used window counts. Chats pick up the new account on their next turn.",
            leading = { IconTile(ZIcons.Context) },
            trailing = {
                Box {
                    FilledTonalButton(onClick = { menu = true }, enabled = state.interactive) { Text("$current%") }
                    DropdownMenuPopup(expanded = menu, onDismissRequest = { menu = false }) {
                        val options = autoSwitchThresholds(current)
                        DropdownMenuGroup(shapes = MenuDefaults.groupShape(0, 1)) {
                            options.forEachIndexed { i, t ->
                                DropdownMenuItem(
                                    onClick = {
                                        menu = false
                                        state.setThreshold(t, onError)
                                    },
                                    text = { Text("$t%") },
                                    shape = MenuDefaults.itemShape(i, options.size).shape,
                                    trailingIcon = if (t == current) {
                                        { ZIcon(ZIcons.Check, "Selected", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurface) }
                                    } else {
                                        null
                                    },
                                )
                            }
                        }
                    }
                }
            },
            onClick = if (state.interactive) ({ menu = true }) else null,
        )
    }
}
