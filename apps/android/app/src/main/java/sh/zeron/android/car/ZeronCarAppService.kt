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
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import sh.zeron.android.ZeronApplication
import sh.zeron.android.core.LaunchOptions
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

    fun markSeen(chatId: String) = agents.markSeen(chatId)

    suspend fun openQuestion(chatId: String) = agents.openQuestion(chatId)

    /** A tapped choice: the same path as a spoken one. */
    fun answer(chatId: String, text: String) {
        lifecycleScope.launch { send(chatId, text) }
    }

    // ── talking ─────────────────────────────────────────────────────────────

    /** Listen and send the words to [chatId], or to the orchestrator when null. */
    fun talk(chatId: String?) {
        if (talking) return
        talking = true
        voice.stopSpeaking()
        val screen = ListeningScreen(carContext, this, agentTitle(chatId))
        carContext.getCarService(androidx.car.app.ScreenManager::class.java).push(screen)
        lifecycleScope.launch {
            try {
                val heard = voice.listen(microphone(), hints())
                screen.close()
                when (heard) {
                    is Heard.Words -> send(chatId, heard.text)
                    Heard.NoRecognizer -> say("Speech recognition isn't available on this phone.")
                    Heard.Silence -> Unit
                }
            } finally {
                screen.close()
                talking = false
            }
        }
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
        when (outcome) {
            is CarAgents.Outcome.Sent -> if (chatId != null) toast("Sent to ${agentTitle(chatId)}")
            CarAgents.Outcome.NoHost -> say("None of your computers is online.")
            CarAgents.Outcome.SignedOut -> say("Sign in to Zeron on your phone first.")
            is CarAgents.Outcome.Failed -> say(outcome.message)
        }
    }

    /** Read an agent's latest reply aloud. */
    fun readAloud(chatId: String) {
        lifecycleScope.launch {
            val text = agents.latestReply(chatId)
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

    private suspend fun say(text: String) = voice.speak(text)

    private fun toast(text: String) = CarToast.makeText(carContext, text, CarToast.LENGTH_LONG).show()

    // ── reports ─────────────────────────────────────────────────────────────

    private suspend fun announce(report: Report) {
        // A realtime call speaks for itself.
        if (model.voice.live) return
        if (!report.orchestrator) alert(report)
        voice.speak(report.spoken)
        // Questions are answered right away; the microphone closes again if nothing is said.
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
