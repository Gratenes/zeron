package sh.zeron.android.car

import androidx.car.app.CarContext
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
import androidx.car.app.model.Template
import androidx.core.graphics.drawable.IconCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.launch
import sh.zeron.android.R
import uniffi.zeron_core.InputRequest
import uniffi.zeron_core.UserInputQuestion

private fun CarContext.icon(res: Int) = CarIcon.Builder(IconCompat.createWithResource(this, res)).build()

/** The agents, most urgent first, with Talk (the orchestrator) and Call. */
class HomeScreen(context: CarContext, private val session: CarSession) : Screen(context) {
    private var agents: List<CarAgent>? = null

    init {
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                session.agentList.collect {
                    agents = it
                    invalidate()
                }
            }
        }
    }

    override fun onGetTemplate(): Template {
        val talk = Action.Builder()
            .setTitle("Talk")
            .setIcon(carContext.icon(R.drawable.ic_car_mic))
            .setOnClickListener { session.talk(null) }
            .build()
        // The header takes one titled action: Call is its icon alone.
        val call = Action.Builder()
            .setIcon(carContext.icon(if (session.callLive) R.drawable.ic_car_call_end else R.drawable.ic_car_call))
            .setOnClickListener {
                session.call()
                invalidate()
            }
            .build()
        val builder = ListTemplate.Builder()
            .setTitle("Zeron")
            .setHeaderAction(Action.APP_ICON)
            .setActionStrip(ActionStrip.Builder().addAction(talk).addAction(call).build())
        val agents = agents
        if (agents == null) return builder.setLoading(true).build()
        val list = ItemList.Builder().setNoItemsMessage("No agents yet. Tap Talk and say what to work on.")
        for (agent in agents) {
            val row = Row.Builder()
                .setTitle(agent.title)
                .addText(agent.detail)
                .setOnClickListener { screenManager.push(AgentScreen(carContext, session, agent.id)) }
            when (agent.state) {
                AgentState.NeedsInput -> row.setImage(CarIcon.ALERT)
                AgentState.Failed -> row.setImage(CarIcon.ERROR)
                else -> Unit
            }
            list.addItem(row.build())
        }
        return builder.setSingleList(list.build()).build()
    }
}

/** One agent: what it said last, Reply, Read aloud and Stop. */
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
    }

    override fun onGetTemplate(): Template {
        val agent = agent
        val title = agent?.title ?: session.agentTitle(chatId)
        val pane = Pane.Builder()
        val reply = reply
        if (reply == null) {
            pane.setLoading(true)
        } else {
            pane.addRow(Row.Builder().setTitle(agent?.status ?: "").build())
            val asked = question?.questions?.firstOrNull()
            if (asked != null) {
                // The question, not the reply around it: that's what needs an answer.
                pane.addRow(Row.Builder().setTitle(Spoken.summary(asked.question, 200)).build())
                if (asked.options.isNotEmpty()) pane.addRow(Row.Builder().setTitle(asked.options.joinToString(" · ")).build())
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
            val choices = question?.takeIf { q -> q.questions.size == 1 && q.questions[0].options.isNotEmpty() }
            if (choices != null) {
                pane.addAction(
                    Action.Builder().setTitle("Choices").setOnClickListener {
                        screenManager.push(ChoicesScreen(carContext, session, chatId, choices.questions[0]))
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

/** A question's options, one tap each (the voice answer is on the agent screen). */
class ChoicesScreen(
    context: CarContext,
    private val session: CarSession,
    private val chatId: String,
    private val question: UserInputQuestion,
) : Screen(context) {
    override fun onGetTemplate(): Template {
        val list = ItemList.Builder()
        for (option in question.options) {
            list.addItem(
                Row.Builder().setTitle(option).setOnClickListener {
                    session.answer(chatId, option)
                    screenManager.pop()
                }.build(),
            )
        }
        return ListTemplate.Builder()
            .setTitle(Spoken.summary(question.question, 60))
            .setHeaderAction(Action.BACK)
            .setSingleList(list.build())
            .build()
    }
}

/** Shown while the microphone is open; Cancel stops listening. */
class ListeningScreen(context: CarContext, private val session: CarSession, private val to: String) : Screen(context) {
    private var done = false

    override fun onGetTemplate(): Template =
        MessageTemplate.Builder("Listening…")
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
