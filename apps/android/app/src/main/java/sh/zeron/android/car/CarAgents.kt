package sh.zeron.android.car

import android.app.Application
import android.util.Log
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.onSubscription
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import sh.zeron.android.core.AppModel
import uniffi.zeron_core.BusyPolicy
import uniffi.zeron_core.ChatConfig
import uniffi.zeron_core.DeviceView
import uniffi.zeron_core.InputRequest
import uniffi.zeron_core.NewSession
import uniffi.zeron_core.SandboxLevel
import uniffi.zeron_core.SendRequest
import uniffi.zeron_core.SessionHandle
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.SessionTarget
import uniffi.zeron_core.UserInputAnswer
import uniffi.zeron_core.WorkspaceSnapshot

/** One row of the car's agent list. */
data class CarAgent(
    val id: String,
    val title: String,
    val state: AgentState,
    /** "Working · 4m", "Done · 2h ago" (the agent screen's status line). */
    val status: String,
    /** The list's second line: the status, or for finished work what it said. */
    val detail: String,
    val unseen: Boolean,
    /** Its folder: the project, or its computer's home for a chat without one. */
    val folder: Folder,
    val lastActivityMs: Long,
    /** When the running turn started (Working or waiting for an answer). */
    val workingSinceMs: Long?,
)

/** A project (or a computer's home) the car groups chats under, like the drawer's project picker. */
data class Folder(val id: String, val name: String)

/** An agent finished, wants an answer, or failed — what the car says about it. */
data class Report(
    val chatId: String,
    val title: String,
    val kind: ReportKind,
    /** The reply as prose (screens); empty when there was none. */
    val text: String,
    /** What is read aloud. */
    val spoken: String,
    /** The car orchestrator answering a spoken command. */
    val orchestrator: Boolean,
    val question: InputRequest? = null,
    val createdAtMs: Long = 0,
)

/**
 * The car's view of the workspace, independent of the car screens: which
 * agents are working or waiting, what they report when they finish, and the
 * car orchestrator chat spoken commands go to. Main thread.
 */
class CarAgents(app: Application, private val model: AppModel) {
    private val scope = MainScope()
    private val prefs = app.getSharedPreferences("car", 0)
    private val states = AgentStates()
    private var watching: Job? = null

    private val _agents = MutableStateFlow<List<CarAgent>>(emptyList())
    val agents: StateFlow<List<CarAgent>> = _agents.asStateFlow()

    private val _reports = MutableSharedFlow<Report>(extraBufferCapacity = 16)
    val reports: SharedFlow<Report> = _reports

    /** The chat spoken commands go to (made on the first command). */
    var orchestratorId: String?
        get() = prefs.getString("orchestrator", null)
        private set(value) = prefs.edit().putString("orchestrator", value).apply()

    /** [orchestratorId] while its chat is still around. */
    val liveOrchestrator: String?
        get() = orchestratorId?.takeIf { id -> model.row(id)?.archived == false }

    /** Watch the workspace while a car is connected. */
    fun start() {
        if (watching != null) return
        states.reset()
        watching = scope.launch {
            model.workspace.collect { ws -> if (ws != null) onWorkspace(ws) }
        }
    }

    fun stop() {
        watching?.cancel()
        watching = null
    }

    private fun rows(ws: WorkspaceSnapshot): List<SessionRow> =
        (ws.front.pinned + ws.front.sections.flatMap { it.sessions } + ws.front.recent)
            .filter { !it.archived }
            .distinctBy { it.id }

    private fun onWorkspace(ws: WorkspaceSnapshot) {
        val rows = rows(ws)
        val orchestrator = orchestratorId
        // Checkouts of one repository on several computers share a name: those folders say where.
        val clashing = rows.mapNotNull { it.project }.distinctBy { it.id }.groupBy { it.name }.filterValues { it.size > 1 }.keys
        _agents.value = rows.filter { it.id != orchestrator }
            // Newest activity first, like the drawer.
            .sortedByDescending { it.lastActivityMs }
            .map { agent(it, clashing) }
        // An offline host's chat reads Idle once stale: that isn't finishing.
        val reports = states.update(rows.filter { it.deviceOnline }.associate { it.id to it.indicator.agentState() })
        for ((id, kind) in reports) {
            val row = rows.firstOrNull { it.id == id } ?: continue
            scope.launch { _reports.emit(report(row, kind)) }
        }
    }

    private fun agent(row: SessionRow, clashing: Set<String>): CarAgent {
        val state = row.indicator.agentState()
        val preview = row.preview?.let { Spoken.summary(it, 80) }.orEmpty()
        val ago = if (row.timeLabel == "now") "just now" else "${row.timeLabel} ago"
        val status = when (state) {
            AgentState.NeedsInput -> "Needs your answer"
            AgentState.Working -> "Working" + (row.workingSinceMs?.let { " · " + elapsedLabel(it) } ?: "")
            AgentState.Failed -> "Failed · $ago"
            AgentState.Done, AgentState.Idle -> "Done · $ago"
        }
        val detail = when (state) {
            AgentState.Done, AgentState.Idle -> listOf(if (state == AgentState.Done) "Done" else row.timeLabel, preview).filter { it.isNotEmpty() }.joinToString(" · ")
            else -> status
        }
        val project = row.project
        val folder = if (project != null) {
            Folder("project:${project.id}", project.name + (row.deviceName?.takeIf { project.name in clashing }?.let { " · $it" } ?: ""))
        }
        else Folder("home:${row.deviceId}", "No project" + (row.deviceName?.let { " · $it" } ?: ""))
        return CarAgent(row.id, row.title, state, status, detail, row.unseen, folder, row.lastActivityMs, row.workingSinceMs)
    }


    // ── reading a chat ──────────────────────────────────────────────────────

    private fun handle(chatId: String): SessionHandle? {
        val client = model.client.value ?: return null
        return client.session(chatId) ?: runCatching { client.openSession(chatId) }.getOrNull()
    }

    /** The chat's transcript, once it has loaded (a few seconds at most). */
    private suspend fun loaded(chatId: String): SessionHandle? {
        val handle = handle(chatId) ?: return null
        if (!handle.transcriptStatus().hydrated) {
            withTimeoutOrNull(HYDRATE_TIMEOUT) {
                model.sessionEvents.onSubscription { emit(chatId) }
                    .first { it == chatId && handle.transcriptStatus().hydrated }
            }
        }
        return handle
    }

    /** The agent's latest reply, as prose. */
    suspend fun latestReply(chatId: String): String =
        loaded(chatId)?.lastReply()?.let(Spoken::clean).orEmpty()

    suspend fun openQuestion(chatId: String): InputRequest? = loaded(chatId)?.composer()?.openInput

    private suspend fun report(row: SessionRow, kind: ReportKind): Report {
        val orchestrator = row.id == orchestratorId
        val handle = loaded(row.id)
        val reply = handle?.lastReply().orEmpty()
        val text = Spoken.clean(reply)
        val title = if (orchestrator) "Zeron" else row.title
        val question = if (kind == ReportKind.NeedsInput) handle?.composer()?.openInput else null
        val spoken = when (kind) {
            ReportKind.Finished ->
                if (orchestrator) Spoken.summary(reply, 400).ifEmpty { "Done." }
                else "$title is done. " + Spoken.summary(reply)
            ReportKind.NeedsInput -> {
                val ask = (question?.let { QuestionFlow(it).step() } as? QuestionFlow.Step.Ask)?.prompt ?: Spoken.summary(reply)
                if (orchestrator) ask else "$title needs you. $ask"
            }
            ReportKind.Failed -> "$title ran into an error."
        }.trim()
        return Report(row.id, title, kind, text, spoken, orchestrator, question, row.createdAtMs)
    }

    // ── speaking to agents ──────────────────────────────────────────────────

    sealed interface Outcome {
        data class Sent(val chatId: String) : Outcome
        data object NoHost : Outcome
        data object SignedOut : Outcome
        data class Failed(val message: String) : Outcome
    }

    /**
     * Words for one agent: the answer to its open question when it asks just
     * one, otherwise a message (queued behind a running turn). Several
     * questions go through a [QuestionFlow] and [respond].
     */
    suspend fun reply(chatId: String, text: String): Outcome {
        if (model.client.value == null) return Outcome.SignedOut
        val handle = loaded(chatId) ?: return Outcome.Failed("That chat isn't available.")
        val input = handle.composer().openInput
        if (input != null) {
            val step = QuestionFlow(input).hear(text) as? QuestionFlow.Step.Submit
                ?: return Outcome.Failed("It asked several questions. Tap Answer to go through them.")
            return respond(chatId, input.requestId, step.answers)
        }
        return deliver(chatId, "reply") { handle.send(SendRequest(text, emptyList(), null, BusyPolicy.QUEUE)) }
    }

    /** Every answer to the open request [requestId], sent together. */
    suspend fun respond(chatId: String, requestId: String, answers: List<UserInputAnswer>): Outcome {
        if (model.client.value == null) return Outcome.SignedOut
        val handle = loaded(chatId) ?: return Outcome.Failed("That chat isn't available.")
        if (handle.composer().openInput?.requestId != requestId) return Outcome.Failed("That question was already answered.")
        return deliver(chatId, "answer") { handle.respondInput(requestId, answers) }
    }

    private fun deliver(chatId: String, what: String, send: () -> Unit): Outcome = try {
        send()
        model.noteDeliveryActivity()
        model.refreshWorkspace()
        Outcome.Sent(chatId)
    } catch (e: Exception) {
        Log.w(TAG, "$what failed", e)
        Outcome.Failed("Couldn't send that.")
    }

    /** A spoken command for the orchestrator (made on the host the first time). */
    suspend fun command(text: String): Outcome {
        val client = model.client.value ?: return Outcome.SignedOut
        val existing = liveOrchestrator
        if (existing != null) return reply(existing, text)
        val host = orchestratorHost() ?: return Outcome.NoHost
        val draft = model.lastDraft
        val sameHost = draft.hostId == host.id
        val config = ChatConfig(
            draft.harness,
            if (sameHost) draft.model else null,
            if (sameHost) draft.effort else null,
            emptyMap(),
            SandboxLevel.WORKSPACE_WRITE,
        )
        return try {
            val chatId = client.createSession(NewSession(SessionTarget.Projectless(host.id), config, null, null, ORCHESTRATOR_TITLE))
            orchestratorId = chatId
            client.openSession(chatId).send(SendRequest(ORCHESTRATOR_BRIEF + text, emptyList(), null, BusyPolicy.QUEUE))
            model.noteDeliveryActivity()
            model.refreshWorkspace()
            Outcome.Sent(chatId)
        } catch (e: Exception) {
            Log.w(TAG, "orchestrator failed", e)
            Outcome.Failed("Couldn't reach ${host.name}.")
        }
    }

    /** The new-session page's host when it's online, else any online computer. */
    private fun orchestratorHost(): DeviceView? {
        val devices = model.workspace.value?.devices.orEmpty().filter { it.isExecutionHost && it.online }
        return devices.firstOrNull { it.id == model.lastDraft.hostId } ?: devices.firstOrNull()
    }

    fun interrupt(chatId: String) {
        runCatching { handle(chatId)?.interrupt() }.onFailure { Log.w(TAG, "interrupt failed", it) }
    }

    fun markSeen(chatId: String) = model.markSeen(chatId)

    companion object {
        private const val TAG = "ZeronCar"
        private const val HYDRATE_TIMEOUT = 6_000L
        const val ORCHESTRATOR_TITLE = "Car"

        /** Sent ahead of the first command: the chat's standing instructions. */
        val ORCHESTRATOR_BRIEF = """
            [Zeron car mode] I'm driving. My messages are dictated by speech recognition (expect misheard words) and your replies are read aloud to me.
            You are my orchestrator. Use the Zeron MCP tools to do the work: list and read chats, create chats (kind: chat) with the right agent, project and device, send them messages, and check on their progress. Delegate coding and file changes to those chats instead of doing them yourself. Don't wait for delegated work to finish; Zeron tells me when a chat finishes and I'll answer it directly.
            Reply in one or two short spoken sentences: no markdown, lists, code, file paths or URLs. Ask me before anything destructive.

            My request:
        """.trimIndent() + " "
    }
}
