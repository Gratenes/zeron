package sh.zeron.android.car

import android.Manifest
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.util.Log
import androidx.car.app.CarAppService
import androidx.car.app.CarToast
import androidx.car.app.Screen
import androidx.car.app.Session
import androidx.car.app.SessionInfo
import androidx.car.app.media.CarAudioRecord
import androidx.car.app.model.Action
import androidx.car.app.model.Alert
import androidx.car.app.model.AlertCallback
import androidx.car.app.model.CarText
import androidx.car.app.validation.HostValidator
import androidx.car.app.versioning.CarAppApiLevels
import androidx.core.content.ContextCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import sh.zeron.android.ZeronApplication
import sh.zeron.android.core.LaunchOptions
import uniffi.zeron_core.UserInputAnswer
import kotlin.coroutines.resume

/** Android Auto's entry point: one [CarSession] per connected car screen. */
class ZeronCarAppService : CarAppService() {
    override fun createHostValidator(): HostValidator =
        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) {
            HostValidator.ALLOW_ALL_HOSTS_VALIDATOR
        } else {
            HostValidator.Builder(applicationContext)
                .addAllowedHosts(androidx.car.app.R.array.hosts_allowlist_sample)
                .build()
        }

    override fun onCreateSession(sessionInfo: SessionInfo): Session = CarSession()
}

/** Whether a car screen is connected (voice may run with the app behind it). */
object CarLink {
    @Volatile
    var connected = false
        internal set
}

/**
 * A connected car: keeps the workspace live, announces agent reports, and
 * runs talk turns (listen → send → the reply read aloud).
 */
class CarSession : Session() {
    private val model get() = (carContext.applicationContext as ZeronApplication).model
    private val agents get() = model.car
    internal lateinit var voice: CarVoice
        private set

    /** A talk turn is running (listening or sending). */
    private var talking = false

    val agentList: StateFlow<List<CarAgent>> get() = agents.agents

    override fun onCreateScreen(intent: Intent): Screen {
        voice = CarVoice(carContext)
        lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onCreate(owner: LifecycleOwner) {
                // Android Auto may start the process with the app never opened.
                if (model.client.value == null) model.boot(LaunchOptions())
                CarLink.connected = true
                model.onCarConnected(true)
                agents.start()
            }

            override fun onDestroy(owner: LifecycleOwner) {
                agents.stop()
                voice.release()
                CarLink.connected = false
                model.onCarConnected(false)
            }
        })
        lifecycleScope.launch { agents.reports.collect { announce(it) } }
        return HomeScreen(carContext, this)
    }

    internal fun agentTitle(chatId: String?): String =
        if (chatId == null) "Zeron" else model.row(chatId)?.title ?: "Agent"

    suspend fun latestReply(chatId: String): String = agents.latestReply(chatId)

    /** Something in this chat changed (new text, a tool, the turn ending). */
    fun changes(chatId: String): Flow<String> = model.sessionEvents.filter { it == chatId }

    fun markSeen(chatId: String) = agents.markSeen(chatId)

    suspend fun openQuestion(chatId: String) = agents.openQuestion(chatId)

    // ── talking ─────────────────────────────────────────────────────────────

    /**
     * Listen and send the words to [chatId], or to the orchestrator when null.
     * An open question is asked instead, one question at a time.
     */
    fun talk(chatId: String?) = turn {
        val target = chatId ?: agents.liveOrchestrator
        val input = target?.let { agents.openQuestion(it) }
        if (target != null && input != null && input.questions.isNotEmpty()) {
            ask(target, QuestionFlow(input), prompted = false)
        } else {
            val heard = listen(chatId)
            if (heard != null) send(chatId, heard)
        }
    }

    /** One talk turn at a time; speech in progress stops. */
    private fun turn(block: suspend () -> Unit) {
        if (talking) return
        talking = true
        voice.stopSpeaking()
        lifecycleScope.launch {
            try {
                block()
            } finally {
                talking = false
            }
        }
    }

    /** What the driver said, or null for silence, a cancel or no recognizer. */
    private suspend fun listen(chatId: String?, detail: String? = null): String? {
        val screen = ListeningScreen(carContext, this, agentTitle(chatId), detail)
        carContext.getCarService(androidx.car.app.ScreenManager::class.java).push(screen)
        val heard = try {
            voice.listen(microphone(), hints())
        } finally {
            screen.close()
        }
        return when (heard) {
            is Heard.Words -> heard.text
            Heard.NoRecognizer -> null.also { say("Speech recognition isn't available on this phone.") }
            Heard.Silence -> null
        }
    }

    /**
     * [flow]'s questions by voice, one at a time ([prompted]: the first was
     * already read aloud), then every answer sent together. Silence or a
     * cancel sends nothing and leaves the question open.
     */
    private suspend fun ask(chatId: String, flow: QuestionFlow, prompted: Boolean) {
        var step = flow.step()
        var first = true
        while (step is QuestionFlow.Step.Ask) {
            if (!first || !prompted) say(step.prompt)
            first = false
            val heard = listen(chatId, detail(flow, step))
            if (heard == null) {
                if (flow.index > 0) toast("Not sent: the question stays open")
                return
            }
            toast("“$heard”")
            step = flow.hear(heard)
        }
        respond(chatId, flow, (step as QuestionFlow.Step.Submit).answers, spoken = flow.questions.size > 1)
    }

    /** The listening screen's line: "1 of 2 · Which harnesses…". */
    private fun detail(flow: QuestionFlow, ask: QuestionFlow.Step.Ask) =
        listOf(flow.progress, Spoken.summary(ask.question.question, 120)).filter { it.isNotEmpty() }.joinToString(" · ")

    /** The current question of [flow] answered by voice (a tap on the choices screen). */
    fun hear(chatId: String, flow: QuestionFlow, then: (QuestionFlow.Step) -> Unit) = turn {
        val ask = flow.step() as? QuestionFlow.Step.Ask ?: return@turn
        val heard = listen(chatId, detail(flow, ask)) ?: return@turn
        toast("“$heard”")
        then(flow.hear(heard))
    }

    /** Tapped answers: every answer of [flow], sent together. */
    fun submit(chatId: String, flow: QuestionFlow, answers: List<UserInputAnswer>) {
        lifecycleScope.launch { respond(chatId, flow, answers, spoken = false) }
    }

    /** [answers] to [flow]'s request ([spoken]: read back first). */
    private suspend fun respond(chatId: String, flow: QuestionFlow, answers: List<UserInputAnswer>, spoken: Boolean) {
        if (spoken) say(flow.confirmation(answers))
        val outcome = agents.respond(chatId, flow.input.requestId, answers)
        Log.i(TAG, "answer $chatId: ${answers.size} answers → $outcome")
        reportOutcome(chatId, outcome)
    }

    fun cancelTalk() = voice.cancel()

    /** Names worth biasing the recognizer toward: Zeron, the agents, projects and computers. */
    private fun hints(): List<String> {
        val ws = model.workspace.value
        return (listOf("Zeron") + agents.agents.value.map { it.title } +
            ws?.projects.orEmpty().map { it.name } + ws?.devices.orEmpty().map { it.name })
            .map { it.trim() }.filter { it.isNotEmpty() }.distinct()
    }

    private suspend fun send(chatId: String?, text: String) {
        toast("“$text”")
        val outcome = if (chatId == null) agents.command(text) else agents.reply(chatId, text)
        Log.i(TAG, "talk to ${chatId ?: "orchestrator"}: ${text.length} chars → $outcome")
        reportOutcome(chatId, outcome)
    }

    private suspend fun reportOutcome(chatId: String?, outcome: CarAgents.Outcome) {
        when (outcome) {
            is CarAgents.Outcome.Sent -> if (chatId != null) toast("Sent to ${agentTitle(chatId)}")
            CarAgents.Outcome.NoHost -> say("None of your computers is online.")
            CarAgents.Outcome.SignedOut -> say("Sign in to Zeron on your phone first.")
            is CarAgents.Outcome.Failed -> say(outcome.message)
        }
    }

    /** Read an agent's latest reply aloud. */
    fun readAloud(chatId: String) {
        // The reply may take a few seconds to load: show that something is happening.
        toast("Reading aloud…")
        lifecycleScope.launch {
            val text = agents.latestReply(chatId)
            Log.i(TAG, "read aloud: ${text.length} chars")
            say(if (text.isEmpty()) "${agentTitle(chatId)} hasn't replied yet." else Spoken.summary(text, 1_200))
        }
    }

    fun interrupt(chatId: String) {
        agents.interrupt(chatId)
        toast("Stopped ${agentTitle(chatId)}")
    }

    /** The Codex realtime call (conversation mode); media pauses while it runs. */
    fun call() {
        val voiceCall = model.voice
        if (voiceCall.live) {
            voiceCall.stop()
            toast("Call ended")
        } else if (!voiceCall.available || !voiceCall.start()) {
            lifecycleScope.launch { say("Conversation mode needs Codex voice on an online computer.") }
        }
    }

    val callLive: Boolean get() = model.voice.live

    /** Conversation mode is offered only where Codex voice runs (or a call is on). */
    val callAvailable: Boolean get() = model.voice.live || model.voice.available

    /** Speak, or show it when the phone's text-to-speech can't. */
    private suspend fun say(text: String) {
        if (!voice.speak(text)) toast("Text-to-speech isn't working on this phone")
    }

    private fun toast(text: String) = CarToast.makeText(carContext, text, CarToast.LENGTH_LONG).show()

    // ── reports ─────────────────────────────────────────────────────────────

    private suspend fun announce(report: Report) {
        // A realtime call speaks for itself.
        if (model.voice.live) return
        if (!report.orchestrator) alert(report)
        voice.speak(report.spoken)
        // Questions are answered right away; the microphone closes again if nothing is said.
        val question = report.question
        if (question != null && question.questions.isNotEmpty()) {
            // The first question was part of the report.
            turn { ask(report.chatId, QuestionFlow(question), prompted = true) }
            return
        }
        val asks = report.kind == ReportKind.NeedsInput || report.orchestrator && report.text.trimEnd().endsWith("?")
        if (asks && !talking) talk(report.chatId)
    }

    private fun alert(report: Report) {
        if (carContext.carAppApiLevel < CarAppApiLevels.LEVEL_5) return
        val title = when (report.kind) {
            ReportKind.Finished -> "${report.title} is done"
            ReportKind.NeedsInput -> "${report.title} needs you"
            ReportKind.Failed -> "${report.title} failed"
        }
        val subtitle = Spoken.summary(report.text, 120)
        val alert = Alert.Builder(report.chatId.hashCode(), CarText.create(title), ALERT_DURATION)
            .apply { if (subtitle.isNotEmpty()) setSubtitle(CarText.create(subtitle)) }
            .addAction(Action.Builder().setTitle("Reply").setOnClickListener { talk(report.chatId) }.build())
            .addAction(
                Action.Builder().setTitle("Open").setOnClickListener {
                    carContext.getCarService(androidx.car.app.ScreenManager::class.java)
                        .push(AgentScreen(carContext, this, report.chatId))
                }.build(),
            )
            .setCallback(object : AlertCallback {
                override fun onCancel(reason: Int) = Unit
                override fun onDismiss() = Unit
            })
            .build()
        runCatching { carContext.getCarService(androidx.car.app.AppManager::class.java).showAlert(alert) }
            .onFailure { Log.w(TAG, "alert failed", it) }
    }

    // ── the car's microphone ────────────────────────────────────────────────

    /** The car's microphone (Car API 5+), asking for the permission on the phone once. */
    private suspend fun microphone(): (() -> AudioStream?)? {
        if (carContext.carAppApiLevel < CarAppApiLevels.LEVEL_5) return null
        if (!granted() && !requestPermission()) return null
        return {
            runCatching {
                val record = CarAudioRecord.create(carContext)
                record.startRecording()
                object : AudioStream {
                    override fun read(buffer: ByteArray, offset: Int, length: Int) = record.read(buffer, offset, length)
                    override fun stop() {
                        runCatching { record.stopRecording() }
                    }
                }
            }.onFailure { Log.w(TAG, "car microphone failed", it) }.getOrNull()
        }
    }

    private fun granted() =
        ContextCompat.checkSelfPermission(carContext, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    private suspend fun requestPermission(): Boolean = suspendCancellableCoroutine { cont ->
        toast("Allow the microphone on your phone")
        carContext.requestPermissions(listOf(Manifest.permission.RECORD_AUDIO)) { approved, _ ->
            if (cont.isActive) cont.resume(Manifest.permission.RECORD_AUDIO in approved)
        }
    }

    companion object {
        private const val TAG = "ZeronCar"
        private const val ALERT_DURATION = 10_000L
    }
}
