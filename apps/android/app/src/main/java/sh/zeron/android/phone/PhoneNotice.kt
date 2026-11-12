package sh.zeron.android.phone

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import sh.zeron.android.MainActivity
import sh.zeron.android.R

/**
 * The visible sign that an agent is using this phone, for as long as it is and a few seconds
 * after: a chip over the screen (drawn by the accessibility service, so no extra permission) and
 * a quiet notification, both with Stop. Stop refuses all agent access for [PAUSE_MS].
 */
object PhoneNotice {
    private const val CHANNEL = "phone-activity"
    private const val ID = 7201
    private const val LINGER_MS = 6_000L
    const val PAUSE_MS = 10 * 60_000L
    const val ACTION_STOP = "sh.zeron.android.PHONE_STOP"

    private val main = Handler(Looper.getMainLooper())
    private var chip: View? = null
    private var chipText: TextView? = null
    private val hide = Runnable { dismiss(appContext ?: return@Runnable) }
    private var appContext: Context? = null

    /** An agent's call is starting (or running): [who] is its chat's title, if known. */
    fun active(context: Context, who: String?, doing: String) {
        appContext = context.applicationContext
        val label = if (who.isNullOrBlank()) "An agent is using this phone" else "$who is using this phone"
        main.post {
            main.removeCallbacks(hide)
            notify(context.applicationContext, label, doing)
            showChip(label)
            main.postDelayed(hide, LINGER_MS)
        }
    }

    fun dismiss(context: Context) {
        main.post {
            main.removeCallbacks(hide)
            runCatching { context.getSystemService(NotificationManager::class.java).cancel(ID) }
            removeChip()
        }
    }

    private fun notify(context: Context, title: String, text: String) {
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(context, android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) return
        val manager = context.getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CHANNEL) == null) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, "Agents using this phone", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "Shown while an agent uses your phone, with a Stop button"
                    setShowBadge(false)
                },
            )
        }
        val stop = PendingIntent.getBroadcast(
            context, 0, Intent(ACTION_STOP).setPackage(context.packageName),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val open = PendingIntent.getActivity(
            context, 0, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
        )
        manager.notify(
            ID,
            NotificationCompat.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.zi_arrow_up)
                .setContentTitle(title)
                .setContentText(text)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .setCategory(Notification.CATEGORY_STATUS)
                .setOnlyAlertOnce(true)
                .setTimeoutAfter(LINGER_MS + 4_000)
                .setContentIntent(open)
                .addAction(0, "Stop", stop)
                .build(),
        )
    }

    private fun showChip(label: String) {
        val service = PhoneAccessibilityService.instance ?: return
        if (chip != null) {
            chipText?.text = label
            return
        }
        val density = service.resources.displayMetrics.density
        fun dp(v: Int) = (v * density).toInt()
        val text = TextView(service).apply {
            this.text = label
            setTextColor(Color.WHITE)
            textSize = 13f
            setPadding(dp(14), dp(8), dp(10), dp(8))
        }
        val stop = TextView(service).apply {
            this.text = "Stop"
            setTextColor(Color.parseColor("#FFB4AB"))
            textSize = 13f
            setPadding(dp(10), dp(8), dp(14), dp(8))
            setOnClickListener { stopAgents(service) }
        }
        val view = LinearLayout(service).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            background = GradientDrawable().apply {
                setColor(Color.parseColor("#E6202020"))
                cornerRadius = dp(22).toFloat()
            }
            addView(text)
            addView(stop)
        }
        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
            y = dp(36)
            title = "Zeron agent notice"
        }
        runCatching {
            service.getSystemService(WindowManager::class.java).addView(view, params)
            chip = view
            chipText = text
        }
    }

    private fun removeChip() {
        val view = chip ?: return
        chip = null
        chipText = null
        val service = PhoneAccessibilityService.instance
        runCatching { (service ?: return).getSystemService(WindowManager::class.java).removeView(view) }
    }

    /** Refuse agents for [PAUSE_MS], and end whatever is waiting on the person. */
    fun stopAgents(context: Context) {
        PhoneSettings(context).pausedUntil = System.currentTimeMillis() + PAUSE_MS
        PhoneConfirm.denyAll()
        PhoneActionLog.add(context, "stop", "the person stopped agents (paused for 10 min)", true, null)
        dismiss(context)
    }
}

/** The notification's Stop button. */
class PhoneStopReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == PhoneNotice.ACTION_STOP) PhoneNotice.stopAgents(context)
    }
}
