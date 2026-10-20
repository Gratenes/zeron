package sh.zeron.android.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ChatBubble
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.outlined.ChatBubbleOutline
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.ShortNavigationBar
import androidx.compose.material3.ShortNavigationBarItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import sh.zeron.android.design.ZIcons
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.navigation.NavController
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.compose.foundation.layout.imePadding
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import kotlinx.coroutines.launch
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.key
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.ZeronTheme
import android.net.Uri
import androidx.navigation.navArgument
import sh.zeron.android.tools.BrowserScreen
import sh.zeron.android.tools.FileScreen
import sh.zeron.android.tools.FilesScreen
import sh.zeron.android.tools.TerminalScreen
import sh.zeron.android.tools.WorkspaceRef

@Composable
fun ZeronRoot(model: AppModel) {
    val appearance by model.appearance.collectAsState()
    ZeronTheme(appearance) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            val client by model.client.collectAsState()
            Box(Modifier.fillMaxSize()) {
                AnimatedContent(client != null, transitionSpec = { fadeIn() togetherWith fadeOut() }, label = "root") { signedIn ->
                    if (signedIn) MainNav(model) else SignInScreen(model)
                }
                // Mic prompt, keep-awake and the off-stage "call ended" snackbar.
                sh.zeron.android.voice.VoiceHost(model.voice)
            }
        }
    }
}

object Routes {
    const val HOME = "home"
    const val CHAT = "chat/{id}?subagents={subagents}"
    const val SUBAGENT = "subagent/{chat}/{doc}"
    const val NEW = "new"
    const val SEARCH = "search"
    const val ARCHIVED = "archived"
    const val VOICE = "voice"
    const val VOICE_SETTINGS = "voice-settings"
    const val AGENTS = "agents"
    const val FILES = "files/{ws}"
    const val FILE = "file/{ws}?path={path}"
    const val TERMINAL = "terminal/{ws}"
    const val BROWSER = "browser?ws={ws}&url={url}"
    fun chat(id: String) = "chat/$id"

    /** The chat with its Subagents panel open. */
    fun chatSubagents(id: String) = "chat/$id?subagents=true"

    /** One subagent of [chat], read-only. */
    fun subagent(chat: String, doc: String) = "subagent/${Uri.encode(chat)}/${Uri.encode(doc)}"

    /** Developer tools address a workspace by chat id, or `space:<id>` for a project. */
    fun files(ws: String) = "files/${Uri.encode(ws)}"
    fun file(ws: String, path: String) = "file/${Uri.encode(ws)}?path=${Uri.encode(path)}"
    fun terminal(ws: String) = "terminal/${Uri.encode(ws)}"
    fun browser(ws: String?, url: String?) = "browser?ws=${Uri.encode(ws ?: "")}&url=${Uri.encode(url ?: "")}"
}

private fun AppModel.refFor(ws: String): WorkspaceRef? =
    if (ws.startsWith("space:")) projectRef(ws.removePrefix("space:")) else workspaceRef(ws)

@Composable
private fun MainNav(model: AppModel) {
    val nav = rememberNavController()
    // Large screens (tablets, unfolded foldables): sessions beside the open chat.
    androidx.compose.foundation.layout.BoxWithConstraints(Modifier.fillMaxSize()) {
    // (Width alone would split a phone turned sideways.)
    val wide = maxWidth >= 840.dp && maxHeight >= 480.dp
    var selected by rememberSaveable { mutableStateOf<String?>(null) }
    var tab by rememberSaveable { mutableStateOf(if (model.launch.route == "settings" || model.launch.route == "more") Tab.Settings else Tab.Sessions) }
    val history = remember { HistoryDrawerState() }
    val scope = rememberCoroutineScope()
    val entry by nav.currentBackStackEntryAsState()
    val route = entry?.destination?.route
    val activeChat = if (route == Routes.CHAT) entry?.arguments?.getString("id") else null
    val openHistory: () -> Unit = { scope.launch { history.open() } }
    val open: (String) -> Unit = { id -> if (wide) selected = id else nav.navigate(Routes.chat(id)) }
    // Folding back to a phone keeps the open chat, full screen.
    LaunchedEffect(wide) {
        val id = selected
        if (!wide && id != null) {
            selected = null
            nav.navigate(Routes.chat(id))
        }
    }
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    // A screen stays composed through NavHost's exit fade (700 ms), and so
    // does its focused text field — the IME only went away once the field was
    // disposed, a second after leaving the chat. Drop focus and the keyboard
    // the moment the destination changes instead.
    DisposableEffect(nav, focus, keyboard) {
        val listener = NavController.OnDestinationChangedListener { _, _, _ ->
            focus.clearFocus(force = true)
            keyboard?.hide()
        }
        nav.addOnDestinationChangedListener(listener)
        onDispose { nav.removeOnDestinationChangedListener(listener) }
    }
    LaunchedEffect(Unit) {
        when (val route = model.launch.route) {
            null -> Unit
            "new" -> nav.navigate(Routes.NEW)
            "search" -> nav.navigate(Routes.SEARCH)
            "archived" -> nav.navigate(Routes.ARCHIVED)
            "agents" -> nav.navigate(Routes.AGENTS)
            else -> when {
                route.startsWith("chat:") -> nav.navigate(Routes.chat(route.removePrefix("chat:")))
                // subagents:<chat> opens its panel; subagent:<chat>|<doc> one subagent.
                route.startsWith("subagents:") -> nav.navigate(Routes.chatSubagents(route.removePrefix("subagents:")))
                route.startsWith("subagent:") -> route.removePrefix("subagent:").split('|', limit = 2).let {
                    if (it.size == 2) nav.navigate(Routes.subagent(it[0], it[1]))
                }
                // Developer tools at launch: files:<chat> / terminal:<chat> / browser:<chat>|<url> / file:<chat>|<path>
                route.startsWith("files:") -> nav.navigate(Routes.files(route.removePrefix("files:")))
                route.startsWith("terminal:") -> nav.navigate(Routes.terminal(route.removePrefix("terminal:")))
                route.startsWith("file:") -> route.removePrefix("file:").split('|', limit = 2).let { nav.navigate(Routes.file(it[0], it.getOrElse(1) { "" })) }
                route.startsWith("browser:") -> route.removePrefix("browser:").split('|', limit = 2).let { nav.navigate(Routes.browser(it[0], it.getOrNull(1))) }
            }
        }
    }
    // The notification, a snackbar Retry or `voice-stage` asks for the call.
    LaunchedEffect(Unit) {
        model.voice.stageRequests.collect {
            if (nav.currentDestination?.route != Routes.VOICE) nav.navigate(Routes.VOICE) { launchSingleTop = true }
        }
    }
    Box(
        Modifier.fillMaxSize().onPreviewKeyEvent { e ->
            // Hardware keyboards: Ctrl+N new session, Ctrl+Shift+F search (iPad ⌘N / ⌘⇧F).
            if (e.type != KeyEventType.KeyDown || !(e.isCtrlPressed || e.isMetaPressed)) return@onPreviewKeyEvent false
            when {
                e.key == Key.N -> nav.navigate(Routes.NEW).let { true }
                e.key == Key.F && e.isShiftPressed -> nav.navigate(Routes.SEARCH).let { true }
                else -> false
            }
        },
    ) {
    HistoryDrawerHost(
        history,
        // Wide layouts already show sessions beside the chat.
        enabled = !wide && (route == Routes.HOME || route == Routes.CHAT),
        drawer = {
            HistoryDrawerContent(
                model,
                activeId = activeChat,
                onNewChat = {
                    scope.launch { history.close() }
                    nav.navigate(Routes.NEW) { popUpTo(Routes.HOME) }
                },
                onPick = { id ->
                    scope.launch { history.close() }
                    if (id != activeChat) nav.navigate(Routes.chat(id)) { popUpTo(Routes.HOME) }
                },
                onSearch = {
                    scope.launch { history.close() }
                    nav.navigate(Routes.SEARCH)
                },
                onSettings = {
                    scope.launch { history.close() }
                    tab = Tab.Settings
                    nav.popBackStack(Routes.HOME, inclusive = false)
                },
            )
        },
    ) {
    NavHost(nav, startDestination = Routes.HOME) {
        composable(Routes.HOME) {
            if (!wide) {
                Home(model, nav, open, tab = tab, onTab = { tab = it })
            } else {
                Row(Modifier.fillMaxSize()) {
                    Box(Modifier.width(400.dp).fillMaxHeight()) { Home(model, nav, open, tab, { tab = it }, selected) }
                    androidx.compose.material3.VerticalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    Box(Modifier.weight(1f).fillMaxHeight()) {
                        val id = selected
                        if (id != null) {
                            key(id) {
                                SessionScreen(
                                    model,
                                    id,
                                    onBack = { selected = null },
                                    onOpenVoice = { nav.navigate(Routes.VOICE) { launchSingleTop = true } },
                                    onVoiceSettings = { nav.navigate(Routes.VOICE_SETTINGS) },
                                    onNavigate = { nav.navigate(it) },
                                )
                            }
                        } else {
                            Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
                                sh.zeron.android.design.ZIcon(ZIcons.Chat, null, Modifier.size(40.dp), tint = MaterialTheme.colorScheme.outline)
                                androidx.compose.foundation.layout.Spacer(Modifier.size(12.dp))
                                Text("Pick a session, or start a new one", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                    }
                }
            }
        }
        composable(Routes.ARCHIVED) {
            ArchivedScreen(model, onBack = { nav.popBackStack() }, onOpen = { nav.navigate(Routes.chat(it)) })
        }
        composable(Routes.CHAT, arguments = listOf(navArgument("subagents") { defaultValue = "false" })) { entry ->
            val id = entry.arguments?.getString("id") ?: return@composable
            val subagents = entry.arguments?.getString("subagents") == "true"
            SessionScreen(
                model,
                id,
                onBack = { nav.popBackStack() },
                onNavigate = { nav.navigate(it) },
                showSubagents = subagents,
                onOpenVoice = { nav.navigate(Routes.VOICE) { launchSingleTop = true } },
                onVoiceSettings = { nav.navigate(Routes.VOICE_SETTINGS) },
                onOpenHistory = if (wide) null else openHistory,
            )
        }
        composable(Routes.VOICE) {
            sh.zeron.android.voice.VoiceStageScreen(
                model.voice,
                onClose = { nav.popBackStack() },
                onOpenChat = { chatId ->
                    nav.popBackStack()
                    open(chatId)
                },
                wallpaper = model.wallpaper,
            )
        }
        composable(Routes.VOICE_SETTINGS) {
            sh.zeron.android.voice.VoiceSettingsScreen(model.voice, onBack = { nav.popBackStack() })
        }
        composable(Routes.SUBAGENT) { entry ->
            val chat = entry.arguments?.getString("chat") ?: return@composable
            val doc = entry.arguments?.getString("doc") ?: return@composable
            SubagentScreen(model, chat, doc, onBack = { nav.popBackStack() }, onNavigate = { nav.navigate(it) })
        }
        composable(Routes.FILES) { entry ->
            val ws = entry.arguments?.getString("ws") ?: return@composable
            val ref = remember(ws) { model.refFor(ws) } ?: return@composable Unavailable { nav.popBackStack() }
            FilesScreen(
                model,
                ref,
                onBack = { nav.popBackStack() },
                onOpenFile = { nav.navigate(Routes.file(ws, it)) },
                onTerminal = { nav.navigate(Routes.terminal(ws)) },
                onBrowser = { nav.navigate(Routes.browser(ws, it)) },
            )
        }
        composable(Routes.FILE, arguments = listOf(navArgument("path") { defaultValue = "" })) { entry ->
            val ws = entry.arguments?.getString("ws") ?: return@composable
            val path = entry.arguments?.getString("path").orEmpty()
            val ref = remember(ws) { model.refFor(ws) } ?: return@composable Unavailable { nav.popBackStack() }
            FileScreen(model, ref, path, onBack = { nav.popBackStack() }, onBrowser = { nav.navigate(Routes.browser(ws, it)) })
        }
        composable(Routes.TERMINAL) { entry ->
            val ws = entry.arguments?.getString("ws") ?: return@composable
            val ref = remember(ws) { model.refFor(ws) } ?: return@composable Unavailable { nav.popBackStack() }
            TerminalScreen(model, ref, onBack = { nav.popBackStack() })
        }
        composable(
            Routes.BROWSER,
            arguments = listOf(navArgument("ws") { defaultValue = "" }, navArgument("url") { defaultValue = "" }),
        ) { entry ->
            val ws = entry.arguments?.getString("ws").orEmpty()
            val url = entry.arguments?.getString("url").orEmpty()
            val ref = remember(ws) { ws.ifEmpty { null }?.let { model.refFor(it) } }
            BrowserScreen(model, ref, url.ifEmpty { null }, onBack = { nav.popBackStack() }, onOpenFile = { if (ws.isNotEmpty()) nav.navigate(Routes.file(ws, it)) })
        }
        composable(Routes.NEW) {
            NewSessionScreen(model, onClose = { nav.popBackStack() }, onCreated = { id ->
                nav.popBackStack()
                open(id)
            })
        }
        composable(Routes.SEARCH) {
            SearchScreen(model, onBack = { nav.popBackStack() }, onOpen = { id ->
                if (wide) nav.popBackStack()
                open(id)
            })
        }
        composable(Routes.AGENTS) { AgentsScreen(model, onBack = { nav.popBackStack() }) }
    }
    }
    Toasts(model, nav)
    }
    }
}

/** App-wide snackbars ("Archived · Undo"), just above whatever bottom chrome the page has. */
@Composable
private fun androidx.compose.foundation.layout.BoxScope.Toasts(model: AppModel, nav: NavHostController) {
    val host = remember { SnackbarHostState() }
    LaunchedEffect(Unit) {
        model.toasts.collect { toast ->
            // One at a time: a new toast replaces the one showing.
            host.currentSnackbarData?.dismiss()
            launch {
                val result = host.showSnackbar(toast.message, actionLabel = toast.action, withDismissAction = toast.action != null, duration = SnackbarDuration.Long)
                if (result == SnackbarResult.ActionPerformed) toast.run?.invoke()
            }
        }
    }
    val entry by nav.currentBackStackEntryAsState()
    val inset = when (entry?.destination?.route) {
        Routes.HOME -> 176.dp
        Routes.CHAT, Routes.NEW -> 96.dp
        else -> 16.dp
    }
    SnackbarHost(host, Modifier.align(Alignment.BottomCenter).imePadding().navigationBarsPadding().padding(bottom = inset))
}

private enum class Tab { Sessions, Settings }

@Composable
private fun Home(model: AppModel, nav: NavHostController, open: (String) -> Unit, tab: Tab, onTab: (Tab) -> Unit, selected: String? = null) {
    val workspace by model.workspace.collectAsState()
    val summary = remember(workspace) { workspace?.let { liveSummary(it) } }
    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        when (tab) {
            Tab.Sessions -> SessionsScreen(model, onOpen = open, onArchived = { nav.navigate(Routes.ARCHIVED) }, selected = selected)
            Tab.Settings -> SettingsScreen(model, onArchived = { nav.navigate(Routes.ARCHIVED) }, onOpenVoiceSettings = { nav.navigate(Routes.VOICE_SETTINGS) }, onOpen = { nav.navigate(it) })
        }
        // Floating chrome over a soft scrim: new session, then the nav capsule.
        Column(
            Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .background(Brush.verticalGradient(0f to Color.Transparent, 0.25f to MaterialTheme.colorScheme.background.copy(alpha = 0.94f), 0.6f to MaterialTheme.colorScheme.background))
                .navigationBarsPadding()
                .padding(top = 28.dp, bottom = 8.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (tab == Tab.Sessions) {
                val voiceLive = model.voice.snapshot.collectAsState().value.live
                NewSessionBar(summary, onClick = { nav.navigate(Routes.NEW) }, voiceLive = voiceLive) {
                    sh.zeron.android.voice.VoiceNewSessionAccessory(
                        model.voice,
                        onOpenStage = { nav.navigate(Routes.VOICE) { launchSingleTop = true } },
                        onOpenSettings = { nav.navigate(Routes.VOICE_SETTINGS) },
                        modifier = if (voiceLive) Modifier.weight(1f) else Modifier,
                    )
                }
            }
            FloatingNavBar(
                listOf(
                    NavItem("Sessions", ZIcons.TabSessions, tab == Tab.Sessions) { onTab(Tab.Sessions) },
                    NavItem("Settings", ZIcons.TabSettings, tab == Tab.Settings) { onTab(Tab.Settings) },
                ),
                trailing = {
                    TonalCircleButton(ZIcons.Search, "Search", onClick = { nav.navigate(Routes.SEARCH) }, size = 72.dp)
                },
            )
        }
    }
}

@Composable
private fun Unavailable(onBack: () -> Unit) {
    Column(Modifier.fillMaxSize().navigationBarsPadding().padding(32.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
        Text("This workspace isn't available.", style = MaterialTheme.typography.titleMedium)
        androidx.compose.material3.TextButton(onClick = onBack) { Text("Back") }
    }
}
