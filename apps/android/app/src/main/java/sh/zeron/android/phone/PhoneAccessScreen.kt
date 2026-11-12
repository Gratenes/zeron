package sh.zeron.android.phone

import android.Manifest
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilterChip
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import java.text.DateFormat
import java.util.Date
import sh.zeron.android.R
import sh.zeron.android.core.AppModel
import sh.zeron.android.ui.IconTile
import sh.zeron.android.ui.TonalCircleButton
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.segmentedShapes

/**
 * Settings → Agent phone access: whether agents may use this phone, which of its abilities, what they
 * need from Android, how it spares the battery, the question before a text is sent, the wake rule
 * for new texts, and the log of what agents did.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun PhoneAccessScreen(model: AppModel, onBack: () -> Unit) {
    val context = LocalContext.current
    val settings = remember { PhoneSettings(context) }
    // Android's own screens (accessibility, permissions, battery) change things behind our back.
    var tick by remember { mutableIntStateOf(0) }
    val owner = LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_RESUME) tick++ }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer) }
    }
    val refresh = tick
    var enabled by remember(refresh) { mutableStateOf(settings.enabled) }
    var paused by remember(refresh) { mutableStateOf(settings.paused) }
    var confirmSms by remember(refresh) { mutableStateOf(settings.confirmSmsSend) }
    var keepalive by remember(refresh) { mutableIntStateOf(settings.keepaliveSecs) }
    var wake by remember(refresh) { mutableStateOf(settings.wakeEnabled) }
    var senders by remember(refresh) { mutableStateOf(settings.wakeSenders) }
    var picking by remember { mutableStateOf(false) }
    var unlockOn by remember(refresh) { mutableStateOf(settings.unlockEnabled) }
    var unlockFailed by remember(refresh) { mutableStateOf(settings.unlockFailed) }
    var pinStored by remember(refresh) { mutableStateOf(PhonePinVault.has(context)) }
    var enteringPin by remember { mutableStateOf(false) }
    var folders by remember(refresh) { mutableStateOf(PhoneFolders.all(context)) }
    var confirmWrites by remember(refresh) { mutableStateOf(settings.confirmFileWrites) }
    val allFiles = remember(refresh) { Build.VERSION.SDK_INT >= 30 && android.os.Environment.isExternalStorageManager() }
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { tree ->
        if (tree != null) {
            runCatching { context.contentResolver.takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION) }
            PhoneFolders.pathOfTree(tree)?.let { path ->
                PhoneFolders.add(context, path, tree.toString(), write = false)
                PhoneActionLog.add(context, "files_setup", "the person allowed the folder $path (read only)", true, null)
                folders = PhoneFolders.all(context)
            }
        }
    }
    val deviceSecure = remember(refresh) { context.getSystemService(android.app.KeyguardManager::class.java).isDeviceSecure }
    val sms = remember { PhoneSms(context) }
    val accessibilityOn = remember(refresh) { PhoneAccessibilityService.instance != null }
    val smsGranted = remember(refresh) {
        sms.has(Manifest.permission.READ_SMS) && sms.has(Manifest.permission.SEND_SMS) && sms.has(Manifest.permission.RECEIVE_SMS)
    }
    val contactsGranted = remember(refresh) { sms.has(Manifest.permission.READ_CONTACTS) }
    val listenerOn = remember(refresh) {
        androidx.core.app.NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName)
    }
    val notificationsGranted = remember(refresh) {
        Build.VERSION.SDK_INT < 33 || ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == android.content.pm.PackageManager.PERMISSION_GRANTED
    }
    val unrestricted = remember(refresh) {
        context.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName)
    }
    val permissions = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { tick++ }
    val log by PhoneActionLog.entries.collectAsState()
    androidx.compose.runtime.LaunchedEffect(Unit) { PhoneActionLog.init(context) }
    val workspace by model.workspace.collectAsState()

    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding() + 8.dp,
                bottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding() + 32.dp,
            ),
        ) {
            item {
                Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    TonalCircleButton(R.drawable.zi_arrow_left, "Back", onClick = onBack, container = MaterialTheme.colorScheme.surfaceContainerHighest)
                    Spacer(Modifier.size(12.dp))
                    Text("Agent phone access", style = MaterialTheme.typography.titleLargeEmphasized)
                }
            }
            item {
                Text(
                    "Lets the agents you run in Zeron use this phone: read and tap its screen, open apps, read and send text messages. Only you can turn this on, and you choose what they may do. A notice with a Stop button shows whenever one does.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 28.dp, vertical = 8.dp),
                )
            }
            group {
                SettingRow(0, 1, R.drawable.zi_smartphone, "Let agents use this phone", if (enabled) "On" else "Off", trailing = {
                    Switch(enabled, { on ->
                        settings.enabled = on
                        enabled = on
                        if (on) {
                            settings.pausedUntil = 0
                            paused = false
                            model.restartPhoneHost()
                        } else model.stopPhoneHost()
                    })
                }) {}
            }
            if (paused) group {
                SettingRow(0, 1, R.drawable.zi_stop, "Stopped", "Agents are refused until ${DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(settings.pausedUntil))}. Tap to let them back in.") {
                    settings.pausedUntil = 0
                    paused = false
                }
            }

            section("Android needs")
            group {
                SettingRow(0, 5, R.drawable.zi_keyboard, "Accessibility service", if (accessibilityOn) "On: reading and driving the screen works" else "Off: tap to turn on “Zeron” in Accessibility. On Android 13+, first allow restricted settings in the app's info page.") {
                    context.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                }
                SettingRow(1, 5, R.drawable.zi_chats, "Text messages", if (smsGranted) "Allowed (covers SMS, picture messages and Google Messages chats)" else "Tap to allow reading, sending and receiving messages") {
                    permissions.launch(arrayOf(Manifest.permission.READ_SMS, Manifest.permission.SEND_SMS, Manifest.permission.RECEIVE_SMS))
                }
                SettingRow(2, 5, R.drawable.zi_bot, "Contacts", if (contactsGranted) "Allowed" else "Tap to allow, so texts show names") {
                    permissions.launch(arrayOf(Manifest.permission.READ_CONTACTS))
                }
                SettingRow(3, 5, R.drawable.zi_bell, "Notifications", if (notificationsGranted) "Allowed" else "Tap to allow: the notice and the question before sending need them") {
                    if (Build.VERSION.SDK_INT >= 33) permissions.launch(arrayOf(Manifest.permission.POST_NOTIFICATIONS))
                }
                SettingRow(4, 5, R.drawable.zi_chats, "Notification access", if (listenerOn) "On: Zeron can answer a Google Messages chat" else "Optional: tap to turn on “Zeron”, so agents can reply in Google Messages chats (RCS)") {
                    context.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                }
            }

            section("What agents may do")
            group {
                val groups = PhoneSettings.Group.entries
                groups.forEachIndexed { i, g ->
                    var on by remember(refresh) { mutableStateOf(settings.groupOn(g)) }
                    SettingRow(i, groups.size, R.drawable.zi_check, g.label, g.about, trailing = {
                        Switch(on, { on = it; settings.setGroup(g, it) })
                    }) {}
                }
            }

            section("Unlocking")
            group {
                SettingRow(0, if (unlockOn && deviceSecure) 2 else 1, R.drawable.zi_key_minimalistic, "Let agents unlock this phone",
                    when {
                        !unlockOn -> "Off. When on, an agent that needs the screen can unlock it with your PIN, and it locks again 60 s after it stops. Anyone who controls an agent could use this: leave it off unless you trust them."
                        unlockFailed -> "Switched off after a failed attempt: enter the PIN again"
                        !deviceSecure -> "On (this phone has no PIN: agents only swipe)"
                        pinStored -> "On. PIN stored in ${PhonePinVault.protection(context).ifEmpty { "the Keystore" }}; only used when the phone is locked and you haven't touched it for 3 minutes"
                        else -> "On, but no PIN is stored yet: tap to enter it"
                    },
                    trailing = {
                        Switch(unlockOn, { on ->
                            if (on) {
                                if (deviceSecure && !pinStored) enteringPin = true else { settings.unlockEnabled = true; unlockOn = true }
                            } else {
                                settings.unlockEnabled = false
                                unlockOn = false
                                PhonePinVault.forget(context)
                                pinStored = false
                            }
                        })
                    }) { if (unlockOn && deviceSecure) enteringPin = true }
                if (unlockOn && deviceSecure) {
                    SettingRow(1, 2, R.drawable.zi_trash_bin_minimalistic, "Forget the PIN", "Deletes it and its key from this phone, and switches unlocking off") {
                        settings.unlockEnabled = false; unlockOn = false
                        PhonePinVault.forget(context); pinStored = false
                    }
                }
            }

            section("Files")
            group {
                SettingRow(0, 3, R.drawable.zi_folder, "All files access", if (allFiles) "On: allowed folders are read and written as plain files" else "Off: tap to turn it on for Zeron. Without it only folders you pick below work, one tree at a time.") {
                    context.startActivity(
                        Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, Uri.parse("package:${context.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    )
                }
                SettingRow(1, 3, R.drawable.zi_folder_with_files, "Add a folder", "Agents and your other devices can only use folders you add here. Add read-only first; allow writing per folder.") {
                    pickFolder.launch(null)
                }
                SettingRow(2, 3, R.drawable.zi_check, "Ask me before a file is written", if (confirmWrites) "A notification asks once per transfer" else "Allowed folders accept writes without asking: only turn this off if you trust them", trailing = {
                    Switch(confirmWrites, { confirmWrites = it; settings.confirmFileWrites = it })
                }) {}
            }
            if (folders.isNotEmpty()) group {
                folders.forEachIndexed { i, f ->
                    SettingRow(i, folders.size, R.drawable.zi_folder, f.path, if (f.write) "Read and write. Tap to remove this folder." else "Read only. Tap to remove this folder.", trailing = {
                        Switch(f.write, { on ->
                            PhoneFolders.setWrite(context, f.path, on)
                            folders = PhoneFolders.all(context)
                            PhoneActionLog.add(context, "files_setup", "the person ${if (on) "allowed writing in" else "made read-only"} ${f.path}", true, null)
                        })
                    }) {
                        PhoneFolders.remove(context, f.path)
                        folders = PhoneFolders.all(context)
                    }
                }
            }

            section("Text messages")
            group {
                SettingRow(0, 2, R.drawable.zi_chats, "Ask me before each message is sent", if (confirmSms) "A notification shows the whole text with Send and Don't send" else "Agents send without asking: only turn this off if you trust them", trailing = {
                    Switch(confirmSms, { confirmSms = it; settings.confirmSmsSend = it })
                }) {}
                SettingRow(1, 2, R.drawable.zi_bot, "Wake a chat on a new message", if (wake) "A new message from a number below is sent into “${settings.wakeChatTitle ?: "(pick a chat)"}”" else "Off: new messages are not passed to any agent", trailing = {
                    Switch(wake, { wake = it; settings.wakeEnabled = it })
                }) { picking = true }
            }
            if (wake) item {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    OutlinedTextField(
                        value = senders,
                        onValueChange = { senders = it; settings.wakeSenders = it },
                        label = { Text("Numbers that wake the chat") },
                        supportingText = { Text("Comma-separated. Anyone can message you, and it goes to an agent as it is: list only numbers you trust. Covers texts, picture messages and Google Messages chats (RCS). * means anyone.") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }

            section("Battery")
            group {
                SettingRow(0, 2, R.drawable.zi_clock_circle, "Keep-alive", "How often the link checks in with Zeron: longer saves battery, an agent waits a little longer for a sleeping phone.") {}
                SettingRow(1, 2, R.drawable.zi_settings_minimalistic, "Battery optimization", if (unrestricted) "Zeron is exempt, so Android doesn't pause it" else "Tap to let Zeron run without Android pausing it") {
                    context.startActivity(
                        Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:${context.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    )
                }
            }
            item {
                Row(Modifier.padding(horizontal = 24.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf(30 to "30 s", 60 to "1 min", 120 to "2 min").forEach { (secs, label) ->
                        FilterChip(selected = keepalive == secs, onClick = {
                            keepalive = secs
                            settings.keepaliveSecs = secs
                            if (settings.enabled) model.restartPhoneHost()
                        }, label = { Text(label) })
                    }
                }
            }

            section("What agents did")
            if (log.isEmpty()) item {
                Text("Nothing yet.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 28.dp, vertical = 4.dp))
            } else {
                val recent = log.takeLast(40).reversed()
                items(recent.size) { i ->
                    val e = recent[i]
                    Column(Modifier.fillMaxWidth().padding(horizontal = 28.dp, vertical = 4.dp)) {
                        Text(
                            "${DateFormat.getTimeInstance(DateFormat.MEDIUM).format(Date(e.at))} · ${e.who ?: "no chat"}",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            e.summary,
                            style = MaterialTheme.typography.bodyMedium,
                            color = if (e.ok) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.error,
                        )
                    }
                }
                item { TextButton({ PhoneActionLog.clear() }, Modifier.padding(horizontal = 16.dp)) { Text("Clear the log") } }
            }
        }
    }

    if (enteringPin) {
        var pin by remember { mutableStateOf("") }
        var again by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { enteringPin = false },
            title = { Text("Your phone's PIN") },
            text = {
                Column {
                    Text("It is encrypted with a key that never leaves this phone's secure hardware and is only used to unlock it for an agent. It is never shown, logged or sent anywhere.", style = MaterialTheme.typography.bodySmall)
                    OutlinedTextField(
                        value = pin, onValueChange = { pin = it.filter(Char::isDigit).take(16) }, label = { Text("PIN") }, singleLine = true,
                        visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(),
                        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.NumberPassword),
                        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                    )
                    OutlinedTextField(
                        value = again, onValueChange = { again = it.filter(Char::isDigit).take(16) }, label = { Text("PIN again") }, singleLine = true,
                        visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(),
                        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.NumberPassword),
                        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                    )
                }
            },
            confirmButton = {
                TextButton(enabled = pin.length >= 4 && pin == again, onClick = {
                    runCatching { PhonePinVault.store(context, pin) }.onSuccess {
                        settings.unlockEnabled = true; settings.unlockFailed = false
                        unlockOn = true; unlockFailed = false; pinStored = true
                        PhoneActionLog.add(context, "unlock_setup", "the person stored a PIN for agents", true, null)
                    }
                    enteringPin = false
                }) { Text("Save") }
            },
            dismissButton = { TextButton({ enteringPin = false }) { Text("Cancel") } },
        )
    }

    if (picking) {
        val rows = ((workspace?.front?.pinned.orEmpty()) + (workspace?.front?.recent.orEmpty())).distinctBy { it.id }.take(40)
        AlertDialog(
            onDismissRequest = { picking = false },
            confirmButton = { TextButton({ picking = false }) { Text("Cancel") } },
            title = { Text("Send new texts into which chat?") },
            text = {
                androidx.compose.foundation.lazy.LazyColumn {
                    if (rows.isEmpty()) item { Text("No chats yet.") }
                    items(rows.size) { i ->
                        TextButton({
                            settings.wakeChatId = rows[i].id
                            settings.wakeChatTitle = rows[i].title
                            wake = true
                            settings.wakeEnabled = true
                            picking = false
                        }, Modifier.fillMaxWidth()) { Text(rows[i].title, maxLines = 1) }
                    }
                }
            },
        )
    }
}

private fun LazyListScope.section(title: String) {
    item {
        Text(
            title,
            style = MaterialTheme.typography.titleSmallEmphasized,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 28.dp, top = 24.dp, bottom = 8.dp),
        )
    }
}

/** One card of rows. */
private fun LazyListScope.group(content: @Composable () -> Unit) {
    item {
        Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
            content()
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun SettingRow(
    index: Int,
    count: Int,
    icon: Int,
    title: String,
    support: String,
    trailing: (@Composable () -> Unit)? = null,
    onClick: () -> Unit,
) {
    SegmentedListItem(
        onClick = onClick,
        shapes = segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
        leadingContent = { IconTile(icon) },
        supportingContent = { Text(support) },
        trailingContent = trailing,
    ) { Text(title) }
}
