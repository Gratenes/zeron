package sh.zeron.android.car

import androidx.car.app.CarContext
import androidx.car.app.CarToast
import androidx.car.app.Screen
import androidx.car.app.model.Action
import androidx.car.app.model.ActionStrip
import androidx.car.app.model.CarColor
import androidx.car.app.model.CarIcon
import androidx.car.app.model.ItemList
import androidx.car.app.model.ListTemplate
import androidx.car.app.model.MessageTemplate
import androidx.car.app.model.Pane
import androidx.car.app.model.PaneTemplate
import androidx.car.app.model.Row
import androidx.car.app.model.SectionedItemList
import androidx.car.app.model.Template
import androidx.car.app.model.Toggle
import androidx.core.graphics.drawable.IconCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.launch
import sh.zeron.android.R
import uniffi.zeron_core.InputRequest

private fun CarContext.icon(res: Int) = CarIcon.Builder(IconCompat.createWithResource(this, res)).build()

/** One chat's row: its title, then its status or what it last said. */
private fun agentRow(screen: Screen, session: CarSession, agent: CarAgent): Row {
    val row = Row.Builder()
        .setTitle(agent.title)
        .addText(agent.detail)
        .setOnClickListener { screen.screenManager.push(AgentScreen(screen.carContext, session, agent.id)) }
    when (agent.state) {
        AgentState.NeedsInput -> row.setImage(CarIcon.ALERT)
        AgentState.Failed -> row.setImage(CarIcon.ERROR)
        else -> Unit
    }
    return row.build()
}

/** Follows the car's chat list while the screen is up. */
private fun Screen.watchAgents(session: CarSession, onChange: (List<CarAgent>) -> Unit) {
    lifecycleScope.launch {
        repeatOnLifecycle(Lifecycle.State.STARTED) {
            session.agentList.collect {
                onChange(it)
                invalidate()
            }
        }
    }
}

/** Chats newest first (the drawer's order), with Projects as a folder on top; Talk and Call. */
class HomeScreen(context: CarContext, private val session: CarSession) : Screen(context) {
    private var agents: List<CarAgent>? = null

    init {
        watchAgents(session) { agents = it }
    }

    override fun onGetTemplate(): Template {
        val talk = Action.Builder()
            .setTitle("Talk")
            .setIcon(carContext.icon(R.drawable.ic_car_mic))
            .setOnClickListener { session.talk(null) }
            .build()
        val strip = ActionStrip.Builder().addAction(talk)
        // The header takes one titled action: Call is its icon alone, and only where Codex voice runs.
        if (session.callAvailable) {
            strip.addAction(
                Action.Builder()
                    .setIcon(carContext.icon(if (session.callLive) R.drawable.ic_car_call_end else R.drawable.ic_car_call))
                    .setOnClickListener {
                        session.call()
                        invalidate()
                    }
                    .build(),
            )
        }
        val builder = ListTemplate.Builder()
            .setTitle("Zeron")
            .setHeaderAction(Action.APP_ICON)
            .setActionStrip(strip.build())
        val agents = agents
        if (agents == null) return builder.setLoading(true).build()
        val list = ItemList.Builder().setNoItemsMessage("No chats yet. Tap Talk and say what to work on.")
        val folders = agents.map { it.folder }.distinct()
        if (folders.isNotEmpty()) {
            list.addItem(
                Row.Builder()
                    .setTitle("Projects")
                    .addText(if (folders.size == 1) folders[0].name else "${folders.size} folders")
                    .setImage(carContext.icon(R.drawable.ic_car_folder))
                    .setBrowsable(true)
                    .setOnClickListener { screenManager.push(FoldersScreen(carContext, session)) }
                    .build(),
            )
        }
        for (agent in agents) list.addItem(agentRow(this, session, agent))
        return builder.setSingleList(list.build()).build()
    }
}

/** Projects (and computers' homes), the most recently active first. */
class FoldersScreen(context: CarContext, private val session: CarSession) : Screen(context) {
    private var agents: List<CarAgent> = session.agentList.value

    init {
        watchAgents(session) { agents = it }
    }

    override fun onGetTemplate(): Template {
        val list = ItemList.Builder().setNoItemsMessage("No projects yet.")
        // The chat list is newest first, so each folder's first chat is its latest.
        for ((folder, chats) in agents.groupBy { it.folder }) {
            val waiting = chats.count { it.state == AgentState.NeedsInput }
            val summary = listOfNotNull(
                if (chats.size == 1) "1 chat" else "${chats.size} chats",
                if (waiting > 0) "$waiting waiting" else null,
                chats.first().status.substringAfter(" · ", "").ifEmpty { null },
            ).joinToString(" · ")
            list.addItem(
                Row.Builder()
                    .setTitle(folder.name)
                    .addText(summary)
                    .setBrowsable(true)
                    .setOnClickListener { screenManager.push(FolderScreen(carContext, session, folder)) }
                    .build(),
            )
        }
        return ListTemplate.Builder()
            .setTitle("Projects")
            .setHeaderAction(Action.BACK)
            .setSingleList(list.build())
            .build()
    }
}

/** One project's chats, newest first. */
class FolderScreen(context: CarContext, private val session: CarSession, private val folder: Folder) : Screen(context) {
    private var agents: List<CarAgent> = session.agentList.value

    init {
        watchAgents(session) { agents = it }
    }

    override fun onGetTemplate(): Template {
        val list = ItemList.Builder().setNoItemsMessage("No chats in ${folder.name}.")
        for (agent in agents.filter { it.folder == folder }) list.addItem(agentRow(this, session, agent))
        return ListTemplate.Builder()
            .setTitle(folder.name)
            .setHeaderAction(Action.BACK)
            .setSingleList(list.build())
            .build()
    }
}

/**
 * One agent: what it said last, Reply, Read aloud and Stop. While it works,
 * the screen stays live: how long it has run, and its latest message
 * ("I'll look into this…"; thinking stays hidden).
 */
class AgentScreen(context: CarContext, private val session: CarSession, private val chatId: String) : Screen(context) {
    private var reply: String? = null
    private var question: InputRequest? = null
    private var agent: CarAgent? = null

    private suspend fun load() {
        question = session.openQuestion(chatId)
        reply = session.latestReply(chatId)
    }

    init {
        lifecycleScope.launch {
            load()
            session.markSeen(chatId)
            invalidate()
        }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                session.agentList.collect { list ->
                    val now = list.firstOrNull { it.id == chatId }
                    if (now != null && now.state != agent?.state) {
                        val changed = agent != null
                        agent = now
                        // A turn that ended has a new reply; a question came or went.
                        if (changed) load()
                        invalidate()
                    }
                }
            }
        }
        // While it works: its newest message as it writes (at most every couple of seconds) …
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                session.changes(chatId).conflate().collect {
                    if (agent?.state == AgentState.Working) {
                        reply = session.latestReply(chatId)
                        invalidate()
                    }
                    delay(PROGRESS_INTERVAL)
                }
            }
        }
        // … and the clock.
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                while (true) {
                    delay(CLOCK_INTERVAL)
                    if (agent?.state == AgentState.Working) invalidate()
                }
            }
        }
    }

    override fun onGetTemplate(): Template {
        val agent = agent
        val title = agent?.title ?: session.agentTitle(chatId)
        val pane = Pane.Builder()
        val reply = reply
        if (reply == null) {
            pane.setLoading(true)
        } else if (agent?.state == AgentState.Working) {
            // Two rows throughout, so each update is a refresh of the same screen.
            val since = agent.workingSinceMs
            pane.addRow(Row.Builder().setTitle("Working" + if (since != null) " · " + elapsedLabel(since) else "").build())
            pane.addRow(Row.Builder().setTitle(Spoken.latest(reply, 200).ifEmpty { "Starting…" }).build())
            pane.addAction(
                Action.Builder()
                    .setTitle("Reply")
                    .setIcon(carContext.icon(R.drawable.ic_car_mic))
                    .setFlags(Action.FLAG_PRIMARY)
                    .setBackgroundColor(CarColor.BLUE)
                    .setOnClickListener { session.talk(chatId) }
                    .build(),
            )
            pane.addAction(Action.Builder().setTitle("Read aloud").setOnClickListener { session.readAloud(chatId) }.build())
        } else {
            pane.addRow(Row.Builder().setTitle(agent?.status ?: "").build())
            val asked = question?.questions.orEmpty()
            if (asked.size == 1) {
                // The question, not the reply around it: that's what needs an answer.
                pane.addRow(Row.Builder().setTitle(Spoken.summary(asked[0].question, 200)).build())
                if (asked[0].options.isNotEmpty()) pane.addRow(Row.Builder().setTitle(asked[0].options.joinToString(" · ")).build())
            } else if (asked.isNotEmpty()) {
                // A pane takes four rows: the status and up to three questions.
                asked.take(3).forEachIndexed { i, q ->
                    pane.addRow(Row.Builder().setTitle("${i + 1} of ${asked.size} · " + Spoken.summary(q.question, 120)).build())
                }
            } else {
                val sentences = Spoken.summary(reply, 600).ifEmpty { "No reply yet." }
                // Short rows: the host shows only a couple of lines of each.
                for (chunk in chunks(sentences, 3)) pane.addRow(Row.Builder().setTitle(chunk).build())
            }
            pane.addAction(
                Action.Builder()
                    .setTitle(if (question != null) "Answer" else "Reply")
                    .setIcon(carContext.icon(R.drawable.ic_car_mic))
                    .setFlags(Action.FLAG_PRIMARY)
                    .setBackgroundColor(CarColor.BLUE)
                    .setOnClickListener { session.talk(chatId) }
                    .build(),
            )
            val choices = question?.takeIf { input -> input.questions.any { it.options.isNotEmpty() } }
            if (choices != null) {
                pane.addAction(
                    Action.Builder().setTitle("Choices").setOnClickListener {
                        screenManager.push(ChoicesScreen(carContext, session, chatId, QuestionFlow(choices)))
                    }.build(),
                )
            } else {
                pane.addAction(Action.Builder().setTitle("Read aloud").setOnClickListener { session.readAloud(chatId) }.build())
            }
        }
        val builder = PaneTemplate.Builder(pane.build())
            .setTitle(title)
            .setHeaderAction(Action.BACK)
        if (agent?.state == AgentState.Working || agent?.state == AgentState.NeedsInput) {
            builder.setActionStrip(
                ActionStrip.Builder()
                    .addAction(Action.Builder().setTitle("Stop").setOnClickListener { session.interrupt(chatId) }.build())
                    .build(),
            )
        }
        return builder.build()
    }

    private companion object {
        const val PROGRESS_INTERVAL = 2_000L
        const val CLOCK_INTERVAL = 15_000L
    }

    /** [text] split at sentence ends into at most [count] similar-sized rows. */
    private fun chunks(text: String, count: Int): List<String> {
        val sentences = text.split(Regex("(?<=[.!?…])\\s+"))
        if (sentences.size <= count) return sentences
        val target = text.length / count
        val out = mutableListOf<String>()
        val current = StringBuilder()
        for (s in sentences) {
            if (current.isNotEmpty() && current.length + s.length > target && out.size < count - 1) {
                out += current.toString()
                current.clear()
            }
            if (current.isNotEmpty()) current.append(' ')
            current.append(s)
        }
        if (current.isNotEmpty()) out += current.toString()
        return out
    }
}

/**
 * An agent's questions by touch, one at a time: a tap picks a single
 * option, toggles and Next pick several, the microphone answers by voice.
 * Each next question replaces this screen rather than stacking on it (the
 * host allows few template steps per task); the answers go together at
 * the end, and Back sends nothing.
 */
class ChoicesScreen(
    context: CarContext,
    private val session: CarSession,
    private val chatId: String,
    private val flow: QuestionFlow,
) : Screen(context) {
    private val picked = LinkedHashSet<String>()

    override fun onGetTemplate(): Template {
        val ask = flow.step() as? QuestionFlow.Step.Ask
            ?: return MessageTemplate.Builder("Answered.").setHeaderAction(Action.BACK).build()
        val q = ask.question
        val last = ask.index == flow.questions.size - 1
        val list = ItemList.Builder()
        when {
            q.options.isEmpty() -> list.addItem(
                Row.Builder().setTitle("Say your answer").setImage(carContext.icon(R.drawable.ic_car_mic))
                    .setOnClickListener { session.hear(chatId, flow, ::next) }.build(),
            )
            q.multiSelect -> for (option in q.options) {
                list.addItem(
                    Row.Builder().setTitle(option).setToggle(
                        Toggle.Builder { on -> if (on) picked += option else picked -= option }.setChecked(option in picked).build(),
                    ).build(),
                )
            }
            else -> for (option in q.options) {
                list.addItem(Row.Builder().setTitle(option).setOnClickListener { next(flow.answer(listOf(option))) }.build())
            }
        }
        // The header takes one titled action: the microphone is its icon alone.
        val strip = ActionStrip.Builder().addAction(
            Action.Builder().setIcon(carContext.icon(R.drawable.ic_car_mic)).setOnClickListener { session.hear(chatId, flow, ::next) }.build(),
        )
        if (q.multiSelect && q.options.isNotEmpty()) {
            strip.addAction(
                Action.Builder().setTitle(if (last) "Send" else "Next").setOnClickListener {
                    if (picked.isEmpty()) {
                        CarToast.makeText(carContext, "Pick at least one", CarToast.LENGTH_SHORT).show()
                    } else {
                        next(flow.answer(q.options.filter { it in picked }))
                    }
                }.build(),
            )
        }
        val title = listOf(q.header.ifEmpty { "Question" }, flow.progress).filter { it.isNotEmpty() }.joinToString(" · ")
        return ListTemplate.Builder()
            .setTitle(title)
            .setHeaderAction(Action.BACK)
            .setActionStrip(strip.build())
            .addSectionedList(SectionedItemList.create(list.build(), Spoken.summary(q.question, 200)))
            .build()
    }

    /** The next question in this screen's place, or every answer sent and back to the agent. */
    private fun next(step: QuestionFlow.Step) {
        if (!lifecycle.currentState.isAtLeast(Lifecycle.State.CREATED)) return
        val manager = screenManager
        if (manager.top === this) manager.pop() else manager.remove(this)
        when (step) {
            is QuestionFlow.Step.Ask -> manager.push(ChoicesScreen(carContext, session, chatId, flow))
            is QuestionFlow.Step.Submit -> session.submit(chatId, flow, step.answers)
        }
    }
}

/** Shown while the microphone is open; Cancel stops listening. */
class ListeningScreen(
    context: CarContext,
    private val session: CarSession,
    private val to: String,
    /** What's being answered ("1 of 2 · Which harnesses…"), shown instead of "Listening…" (the host shows few lines). */
    private val detail: String? = null,
) : Screen(context) {
    private var done = false

    override fun onGetTemplate(): Template =
        MessageTemplate.Builder(if (detail.isNullOrEmpty()) "Listening…" else detail)
            .setTitle("To $to")
            .setIcon(carContext.icon(R.drawable.ic_car_mic))
            .setHeaderAction(Action.BACK)
            .addAction(Action.Builder().setTitle("Cancel").setOnClickListener { session.cancelTalk() }.build())
            .build()

    /** Leave once (back press or the turn ending). */
    fun close() {
        if (done) return
        done = true
        if (screenManager.top === this) screenManager.pop() else screenManager.remove(this)
    }

    init {
        lifecycle.addObserver(object : androidx.lifecycle.DefaultLifecycleObserver {
            override fun onDestroy(owner: androidx.lifecycle.LifecycleOwner) {
                // Back on the car screen also stops listening.
                if (!done) {
                    done = true
                    session.cancelTalk()
                }
            }
        })
    }
}
