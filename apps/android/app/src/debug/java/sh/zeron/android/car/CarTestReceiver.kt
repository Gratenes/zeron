package sh.zeron.android.car

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlin.concurrent.thread
import kotlin.math.PI
import kotlin.math.sin

/**
 * Debug only: drives the car's speech and notifications without a signed-in chat, for the DHU.
 *
 * - `am broadcast -a sh.zeron.android.TEST_CAR_REPORT --es text "…" [--es kind needs|failed]` posts a report notification.
 * - `am broadcast -a sh.zeron.android.TEST_CAR_SPEAK [--es text "…"]` reads text aloud like a readback.
 * - `am broadcast -a sh.zeron.android.TEST_CAR_REPLY [--es text "…"]` fires the reply service as Android Auto does for a dictated reply.
 * - `am broadcast -a sh.zeron.android.TEST_CAR_MUSIC --es op start|stop` plays a tone under its own media session (a stand-in music app).
 */
class CarTestReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val app = context.applicationContext
        when (intent.action) {
            ACTION_REPORT -> {
                val kind = when (intent.getStringExtra("kind")) {
                    "needs" -> ReportKind.NeedsInput
                    "failed" -> ReportKind.Failed
                    else -> ReportKind.Finished
                }
                val text = intent.getStringExtra("text") ?: "I fixed the login redirect and added a test. Anything else?"
                val title = intent.getStringExtra("title") ?: "Fix login"
                val spoken = if (kind == ReportKind.NeedsInput) "$title needs you. $text" else text
                val ok = CarNotifications(app).post(Report("test-chat", title, kind, text, spoken, orchestrator = false, createdAtMs = System.currentTimeMillis()))
                Log.i(TAG, "test report posted: $ok")
            }
            ACTION_SPEAK -> {
                val text = intent.getStringExtra("text") ?: LONG
                val voice = voice ?: CarVoice(app) { false }.also { voice = it }
                CoroutineScope(Dispatchers.Main).launch { Log.i(TAG, "test speak: ${voice.speak(text)}") }
            }
            ACTION_REPLY_TEST -> {
                // What Android Auto does with a dictated reply: results in the reply intent, then fire the service.
                val results = android.os.Bundle().apply { putCharSequence(CarMessaging.REPLY_KEY, intent.getStringExtra("text") ?: "yes please open the pull request") }
                val fillIn = Intent()
                androidx.core.app.RemoteInput.addResultsToIntent(
                    arrayOf(androidx.core.app.RemoteInput.Builder(CarMessaging.REPLY_KEY).build()), fillIn, results,
                )
                // Through a PendingIntent, as Android Auto fires it (that is what lets a background service start).
                android.app.PendingIntent.getService(
                    app, 0, CarReplyService.intent(app, CarReplyService.ACTION_REPLY, intent.getStringExtra("chat") ?: "test-chat"),
                    android.app.PendingIntent.FLAG_UPDATE_CURRENT or android.app.PendingIntent.FLAG_MUTABLE,
                ).send(app, 0, fillIn)
            }
            ACTION_MUSIC -> if (intent.getStringExtra("op") == "stop") stopMusic() else startMusic(app)
        }
    }

    private fun startMusic(context: Context) {
        stopMusic()
        val audio = context.getSystemService(AudioManager::class.java)
        val attrs = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build()
        val track = AudioTrack.Builder()
            .setAudioAttributes(attrs)
            .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(22_050).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
            .setBufferSizeInBytes(22_050 * 2)
            .build()
        val sess = MediaSession(context, "FakeMusic")
        var playing = true
        fun state() = PlaybackState.Builder()
            .setActions(PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_SKIP_TO_NEXT)
            .setState(if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED, 0, 1f).build()
        sess.setCallback(object : MediaSession.Callback() {
            override fun onPlay() { Log.i(TAG, "FakeMusic onPlay"); playing = true; track.play(); sess.setPlaybackState(state()) }
            override fun onPause() { Log.i(TAG, "FakeMusic onPause"); playing = false; track.pause(); sess.setPlaybackState(state()) }
            override fun onSkipToNext() { Log.i(TAG, "FakeMusic onSkipToNext") }
            override fun onMediaButtonEvent(mediaButtonIntent: Intent): Boolean {
                Log.i(TAG, "FakeMusic media button ${mediaButtonIntent.getParcelableExtra<android.view.KeyEvent>(Intent.EXTRA_KEY_EVENT)}")
                return super.onMediaButtonEvent(mediaButtonIntent)
            }
        })
        val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(attrs)
            .setOnAudioFocusChangeListener { change -> Log.i(TAG, "FakeMusic focus change $change") }
            .build()
        audio.requestAudioFocus(focus)
        sess.setPlaybackState(state())
        sess.isActive = true
        track.play()
        val samples = ShortArray(22_050) { (sin(2 * PI * 440 * it / 22_050) * 6_000).toInt().toShort() }
        music = Music(track, sess, focus, audio, thread(isDaemon = true) { while (!Thread.interrupted()) { track.write(samples, 0, samples.size) } })
        Log.i(TAG, "FakeMusic started")
    }

    private fun stopMusic() {
        music?.let {
            it.thread.interrupt()
            runCatching { it.track.stop(); it.track.release() }
            it.session.isActive = false
            it.session.release()
            it.audio.abandonAudioFocusRequest(it.focus)
        }
        music = null
    }

    private class Music(val track: AudioTrack, val session: MediaSession, val focus: AudioFocusRequest, val audio: AudioManager, val thread: Thread)

    companion object {
        private const val TAG = "ZeronCarTest"
        const val ACTION_REPORT = "sh.zeron.android.TEST_CAR_REPORT"
        const val ACTION_SPEAK = "sh.zeron.android.TEST_CAR_SPEAK"
        const val ACTION_REPLY_TEST = "sh.zeron.android.TEST_CAR_REPLY"
        const val ACTION_MUSIC = "sh.zeron.android.TEST_CAR_MUSIC"
        private var voice: CarVoice? = null
        private var music: Music? = null
        private val LONG = (1..14).joinToString(" ") { "This is sentence number $it of a long reply that Zeron reads back to the driver." }
    }
}
