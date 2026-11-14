package sh.zeron.android.car

import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.SoundPool
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.os.ParcelFileDescriptor
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import android.util.Log
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import sh.zeron.android.R
import java.util.Locale
import java.util.UUID
import kotlin.concurrent.thread
import kotlin.coroutines.resume

/** 16 kHz mono 16-bit PCM from a microphone (the car's, through the host). */
interface AudioStream {
    /** Bytes read; -1 once the stream ended (the driver dismissed the microphone). */
    fun read(buffer: ByteArray, offset: Int, length: Int): Int
    fun stop()
}

/** What listening heard. */
sealed interface Heard {
    data class Words(val text: String) : Heard
    /** Nothing was said, or the driver cancelled. */
    data object Silence : Heard
    data object NoRecognizer : Heard
}

/**
 * The car's speech, all on the phone: its text-to-speech and its (on-device
 * when there is one) speech recognizer. Audio focus is held only for the
 * moment it's needed, so the driver's music keeps playing — paused while
 * they speak, ducked while Zeron speaks — and never on the call stream.
 * One at a time: speaking waits for listening and the other way round.
 * Main thread.
 */
class CarVoice(
    context: Context,
    /** Pause the music while speaking, on the media stream (cars whose navigation-voice volume is low). */
    private val pausesMusic: () -> Boolean = { false },
    /** On-device dictation, and which engine the driver chose; Android's recognizer is the fallback. */
    private val local: LocalAsr? = null,
    private val dictationEngine: () -> DictationEngine = { DictationEngine.Android },
) {
    private val context = context.applicationContext
    private val audio = this.context.getSystemService(AudioManager::class.java)
    private val turn = Mutex()
    private val main = Handler(Looper.getMainLooper())

    /** The stream Android Auto mixes over media (like navigation prompts). */
    private val attributes = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_ASSISTANCE_NAVIGATION_GUIDANCE)
        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
        .build()

    /** The voice when it pauses the music: the media stream, at the music's volume. */
    private val loud = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
        .build()

    private val sounds = SoundPool.Builder().setMaxStreams(1).setAudioAttributes(attributes).build()
    private val startSound = sounds.load(this.context, R.raw.voice_start, 1)
    private val endSound = sounds.load(this.context, R.raw.voice_end, 1)

    private var tts: TextToSpeech? = null
    private var ttsReady: CompletableDeferred<Boolean>? = null
    private var speaking: CompletableDeferred<Unit>? = null
    /** The engine reported an error for the current utterance. */
    @Volatile private var failed = false
    private var recognizer: SpeechRecognizer? = null
    private var cancelListening: (() -> Unit)? = null
    private var modelRequested = false

    private fun focus(gain: Int, attributes: AudioAttributes = this.attributes, onLoss: () -> Unit): AudioFocusRequest? {
        val request = AudioFocusRequest.Builder(gain)
            .setAudioAttributes(attributes)
            .setOnAudioFocusChangeListener { change ->
                if (change == AudioManager.AUDIOFOCUS_LOSS || change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT) onLoss()
            }
            .build()
        return if (audio.requestAudioFocus(request) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) request else null
    }

    // ── speaking ────────────────────────────────────────────────────────────

    private suspend fun engine(): TextToSpeech? {
        tts?.let { if (ttsReady?.await() == true) return it }
        val ready = CompletableDeferred<Boolean>()
        ttsReady = ready
        val engine = TextToSpeech(context) { status -> ready.complete(status == TextToSpeech.SUCCESS) }
        tts = engine
        if (withTimeoutOrNull(5_000) { ready.await() } != true) {
            engine.shutdown()
            tts = null
            return null
        }
        engine.setAudioAttributes(attributes)
        // The phone's language when the engine has its voice, else the engine's default voice.
        val language = engine.setLanguage(Locale.getDefault())
        if (language < TextToSpeech.LANG_AVAILABLE) Log.w(TAG, "text-to-speech has no ${Locale.getDefault()} voice ($language)")
        engine.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
            override fun onStart(utteranceId: String?) {
                progressAt = SystemClock.elapsedRealtime()
            }

            override fun onDone(utteranceId: String?) {
                progressAt = SystemClock.elapsedRealtime()
                // Earlier pieces ending is no end: focus stays held until the last.
                if (utteranceId == lastPiece) speaking?.complete(Unit)
            }

            @Deprecated("Deprecated in Java")
            override fun onError(utteranceId: String?) {
                failed = true
                speaking?.complete(Unit)
            }

            override fun onError(utteranceId: String?, errorCode: Int) {
                Log.w(TAG, "text-to-speech error $errorCode")
                failed = true
                speaking?.complete(Unit)
            }

            override fun onStop(utteranceId: String?, interrupted: Boolean) {
                speaking?.complete(Unit)
            }
        })
        return engine
    }

    /** One focus for the whole readback: see [SpeechFocus]. */
    private val speechFocus: SpeechFocus<AudioFocusRequest> = SpeechFocus(
        acquire = { pause ->
            focus(
                if (pause) AudioManager.AUDIOFOCUS_GAIN_TRANSIENT else AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK,
                if (pause) loud else attributes,
            ) {
                tts?.stop()
                speaking?.complete(Unit)
                speechFocus.drop()
            }
        },
        abandon = { audio.abandonAudioFocusRequest(it) },
        later = { delayMs, block ->
            val run = Runnable(block)
            main.postDelayed(run, delayMs)
            val cancel: () -> Unit = { main.removeCallbacks(run) }
            cancel
        },
    )

    /** The last piece being said: its end is the reply's end. */
    @Volatile private var lastPiece: String? = null
    /** When the engine last reported progress (a piece started or ended). */
    @Volatile private var progressAt = 0L

    /**
     * Say [text] over ducked media; returns once it was said (or stopped):
     * true, or false when the phone's text-to-speech couldn't say it. The
     * text goes to the engine in pieces and audio focus is held from the
     * first to the last, however long the engine takes between pieces.
     */
    suspend fun speak(text: String): Boolean {
        if (text.isBlank()) return true
        return turn.withLock {
            val engine = engine()
            if (engine == null) {
                Log.w(TAG, "text-to-speech unavailable")
                return@withLock false
            }
            val done = CompletableDeferred<Unit>()
            failed = false
            speaking = done
            val pause = pausesMusic()
            engine.setAudioAttributes(if (pause) loud else attributes)
            speechFocus.hold(pause)
            if (!speechFocus.isHeld) Log.w(TAG, "speaking without audio focus")
            try {
                val ids = speechPieces(text).map { UUID.randomUUID().toString() to it }
                lastPiece = ids.last().first
                progressAt = SystemClock.elapsedRealtime()
                for ((i, piece) in ids.withIndex()) {
                    // An engine that refuses the text never calls back: don't wait on it.
                    val queued = engine.speak(piece.second, if (i == 0) TextToSpeech.QUEUE_FLUSH else TextToSpeech.QUEUE_ADD, Bundle(), piece.first)
                    if (queued != TextToSpeech.SUCCESS) {
                        Log.w(TAG, "text-to-speech refused ${piece.second.length} chars ($queued)")
                        return@withLock false
                    }
                }
                // Only a silent engine ends the wait early: a slow gap between pieces
                // (a network voice on mobile data) keeps focus and keeps waiting.
                while (!done.isCompleted) {
                    withTimeoutOrNull(1_000) { done.await() }
                    if (SystemClock.elapsedRealtime() - progressAt > STALL_MS) {
                        Log.w(TAG, "text-to-speech stalled")
                        return@withLock false
                    }
                }
                !failed
            } finally {
                engine.stop()
                speaking = null
                lastPiece = null
                speechFocus.settle()
            }
        }
    }

    fun stopSpeaking() {
        tts?.stop()
        speaking?.complete(Unit)
        speechFocus.drop()
    }

    // ── listening ───────────────────────────────────────────────────────────

    /**
     * Listen for one utterance: music pauses, a tone marks the start, and
     * the phone's recognizer transcribes [microphone] (the car's), or the
     * phone's own microphone without one or before Android 13.
     */
    suspend fun listen(microphone: (() -> AudioStream?)?, hints: List<String> = emptyList(), onPartial: (String) -> Unit = {}): Heard = turn.withLock {
        val useLocal = local != null &&
            engineFor(dictationEngine(), local.model.isReady(), local.usable) == DictationEngine.Local
        if (!useLocal && !SpeechRecognizer.isRecognitionAvailable(context)) return@withLock Heard.NoRecognizer
        var lost = false
        speechFocus.drop()
        val focus = focus(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE) { lost = true; cancel() }
        try {
            chime(startSound)
            var heard: Heard? = null
            var triedLocal = false
            if (useLocal) {
                // The car's microphone, or the phone's own without one.
                val stream = (if (Build.VERSION.SDK_INT >= 33) microphone?.invoke() else null) ?: PhoneMicrophone.open(context)
                triedLocal = true
                heard = recognizeLocal(stream, onPartial)
                if (heard == null) Log.w(TAG, "on-device dictation failed: falling back to the Android recognizer")
            }
            if (heard == null) {
                if (triedLocal && !SpeechRecognizer.isRecognitionAvailable(context)) return@withLock Heard.NoRecognizer
                val stream = if (Build.VERSION.SDK_INT >= 33) microphone?.invoke() else null
                heard = recognize(stream, onDevice = true, hints, onPartial)
                // Some on-device recognizers refuse an app-supplied stream or the language.
                if (heard == null && !lost) heard = recognize(if (Build.VERSION.SDK_INT >= 33) microphone?.invoke() else null, onDevice = false, hints, onPartial)
            }
            heard ?: Heard.Silence
        } finally {
            focus?.let(audio::abandonAudioFocusRequest)
            if (!lost) sounds.play(endSound, 0.6f, 0.6f, 1, 0, 1f)
        }
    }

    /**
     * One utterance on the phone itself: streaming partials while the driver talks, the
     * whole utterance decoded again at its end. Null when it can't (no microphone, the
     * model won't load, an error before any words): the caller falls back.
     */
    private suspend fun recognizeLocal(stream: AudioStream?, onPartial: (String) -> Unit): Heard? {
        val asr = local ?: return null
        if (stream == null) return null
        val cancelled = java.util.concurrent.atomic.AtomicBoolean(false)
        val session = try {
            withContext(Dispatchers.Default) { asr.session() }
        } catch (e: Throwable) {
            Log.w(TAG, "on-device dictation can't start", e)
            stream.stop()
            return null
        }
        cancelListening = { cancelled.set(true) }
        // On a thread of its own: a read from a microphone that delivers nothing blocks, and only
        // stopping the stream (below) frees it.
        val done = CompletableDeferred<LocalResult>()
        val listening = LocalListening(session)
        val started = SystemClock.elapsedRealtime()
        thread(name = "car-local-asr", isDaemon = true) {
            done.complete(listening.run(stream, { text -> main.post { onPartial(text) } }, { cancelled.get() }))
        }
        return try {
            // Only a guard: the end of speech ends a normal utterance. A microphone that delivers
            // nothing for a while is stuck, not quiet (quiet audio still arrives).
            while (!done.isCompleted) {
                withTimeoutOrNull(1_000) { done.await() }
                val silentMs = (System.nanoTime() - listening.lastAudioNanos) / 1_000_000
                if (SystemClock.elapsedRealtime() - started > LISTEN_LIMIT || silentMs > MIC_STALL_MS) {
                    Log.w(TAG, "on-device dictation: the microphone stalled or ran past the limit, stopping")
                    cancelled.set(true)
                    stream.stop()
                    break
                }
            }
            val result = withTimeoutOrNull(5_000) { done.await() } ?: LocalResult.Cancelled
            Log.i(TAG, "on-device dictation: ${result.javaClass.simpleName} after ${SystemClock.elapsedRealtime() - started} ms, ${listening.bytesRead / 32} ms of audio" + ((result as? LocalResult.Text)?.let { ", ${it.text.length} chars" } ?: ""))
            when (result) {
                is LocalResult.Text -> result.text.takeIf { it.isNotBlank() }?.let { Heard.Words(it) } ?: Heard.Silence
                LocalResult.Cancelled -> Heard.Silence
                is LocalResult.Failed -> {
                    Log.w(TAG, "on-device dictation failed", result.error)
                    result.partial?.takeIf { it.isNotBlank() }?.let { Heard.Words(it) }
                }
            }
        } finally {
            cancelListening = null
            // The native session can't be closed under a feed still running (a stuck microphone read).
            if (done.isCompleted) session.close()
        }
    }

    /** Stop listening now (Cancel on the car screen, focus lost). */
    fun cancel() {
        cancelListening?.invoke()
    }

    private suspend fun chime(sound: Int) {
        sounds.play(sound, 0.6f, 0.6f, 1, 0, 1f)
        // Let the tone finish before the microphone opens, so it isn't transcribed.
        delay(300)
    }

    private fun recognizerIntent(pipe: ParcelFileDescriptor?, hints: List<String> = emptyList()) = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
        putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
        putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
        putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
        putExtra(RecognizerIntent.EXTRA_SPEECH_INPUT_COMPLETE_SILENCE_LENGTH_MILLIS, 2_500L)
        putExtra(RecognizerIntent.EXTRA_SPEECH_INPUT_POSSIBLY_COMPLETE_SILENCE_LENGTH_MILLIS, 2_000L)
        // Words the driver is likely to say ("Zeron", agent titles) that a general model mishears.
        if (hints.isNotEmpty() && Build.VERSION.SDK_INT >= 33) {
            putStringArrayListExtra(RecognizerIntent.EXTRA_BIASING_STRINGS, ArrayList(hints.take(MAX_HINTS)))
        }
        if (pipe != null && Build.VERSION.SDK_INT >= 33) {
            putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE, pipe)
            putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_CHANNEL_COUNT, 1)
            putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
            putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_SAMPLING_RATE, 16_000)
        }
    }

    /** One recognition; null when this recognizer can't do it (try another). */
    private suspend fun recognize(stream: AudioStream?, onDevice: Boolean, hints: List<String>, onPartial: (String) -> Unit): Heard? {
        val recognizer = when {
            onDevice && Build.VERSION.SDK_INT >= 31 && SpeechRecognizer.isOnDeviceRecognitionAvailable(context) ->
                SpeechRecognizer.createOnDeviceSpeechRecognizer(context)
            onDevice -> {
                stream?.stop()
                return null
            }
            else -> SpeechRecognizer.createSpeechRecognizer(context)
        }
        this.recognizer = recognizer
        // The car's audio flows into the recognizer through a pipe.
        val pipe = stream?.let { ParcelFileDescriptor.createPipe() }
        var pumping = pipe != null
        // The recognizer doesn't endpoint an app-supplied stream: it answers once the
        // stream ends, so the pump ends it after the driver stops talking.
        var streamEnded: (() -> Unit)? = null
        if (pipe != null && stream != null) {
            thread(name = "car-mic", isDaemon = true) {
                val endpointer = Endpointer()
                ParcelFileDescriptor.AutoCloseOutputStream(pipe[1]).use { out ->
                    val buffer = ByteArray(BUFFER)
                    try {
                        while (pumping) {
                            val n = stream.read(buffer, 0, buffer.size)
                            if (n < 0) break
                            if (n > 0) out.write(buffer, 0, n)
                            if (endpointer.feed(buffer, n)) break
                        }
                    } catch (_: Exception) {
                        // The recognizer closed its end.
                    } finally {
                        stream.stop()
                    }
                }
                main.post { streamEnded?.invoke() }
            }
        }
        var heardAnything = false
        var lastPartial: String? = null
        return try {
            // Boxed: null inside means "use another recognizer", a null box is the time limit.
            withTimeoutOrNull(LISTEN_LIMIT) {
                Attempt(suspendCancellableCoroutine<Heard?> { cont ->
                    fun finish(result: Heard?) {
                        if (cont.isActive) cont.resume(result)
                    }
                    cancelListening = { recognizer.cancel(); finish(Heard.Silence) }
                    // Final results normally follow the end of the stream at once; the last partial stands in.
                    streamEnded = {
                        main.postDelayed({ finish(lastPartial?.let { Heard.Words(it) } ?: Heard.Silence) }, RESULT_GRACE)
                    }
                    cont.invokeOnCancellation { recognizer.cancel() }
                    recognizer.setRecognitionListener(object : RecognitionListener {
                        override fun onReadyForSpeech(params: Bundle?) = Unit
                        override fun onBeginningOfSpeech() {
                            heardAnything = true
                        }

                        override fun onRmsChanged(rmsdB: Float) = Unit
                        override fun onBufferReceived(buffer: ByteArray?) = Unit
                        override fun onEndOfSpeech() = Unit
                        override fun onEvent(eventType: Int, params: Bundle?) = Unit
                        override fun onPartialResults(partialResults: Bundle?) {
                            best(partialResults)?.let {
                                lastPartial = it
                                onPartial(it)
                            }
                        }

                        override fun onResults(results: Bundle?) {
                            // Some recognizers deliver the final text as the last partial and an empty bundle here.
                            finish(heardText(best(results), lastPartial)?.let { Heard.Words(it) } ?: Heard.Silence)
                        }

                        override fun onError(error: Int) {
                            Log.i(TAG, "recognizer error $error (on-device $onDevice)")
                            // No language pack yet: fetch it for next time (the other recognizer answers now).
                            if (onDevice && error == SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE && !modelRequested && Build.VERSION.SDK_INT >= 33) {
                                modelRequested = true
                                runCatching { recognizer.triggerModelDownload(recognizerIntent(null)) }
                            }
                            val unsupported = error == SpeechRecognizer.ERROR_CLIENT ||
                                error == SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED ||
                                error == SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE ||
                                error == SpeechRecognizer.ERROR_CANNOT_CHECK_SUPPORT
                            // Words already recognized aren't thrown away by a late error (a timeout after a pause).
                            finish(if (onDevice && unsupported && !heardAnything) null else lastPartial?.let { Heard.Words(it) } ?: Heard.Silence)
                        }
                    })
                    recognizer.startListening(recognizerIntent(pipe?.get(0), hints))
                })
            }.let { it?.heard ?: if (it == null) lastPartial?.let { p -> Heard.Words(p) } ?: Heard.Silence else null }
        } finally {
            pumping = false
            cancelListening = null
            recognizer.destroy()
            this.recognizer = null
            pipe?.get(0)?.close()
        }
    }

    private fun best(results: Bundle?): String? =
        results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()?.trim()?.takeIf { it.isNotEmpty() }

    fun release() {
        cancel()
        stopSpeaking()
        tts?.shutdown()
        tts = null
        sounds.release()
    }

    private class Attempt(val heard: Heard?)

    companion object {
        private const val TAG = "ZeronCar"
        private const val BUFFER = 512
        /** Only a guard: the end of speech, not this, ends a normal reply (a long one takes a minute). */
        private const val LISTEN_LIMIT = 90_000L
        /** The final text can take a few seconds after a long utterance; the last partial is cut short. */
        private const val RESULT_GRACE = 8_000L
        /** No word from the engine for this long: it is stuck, not slow. */
        private const val STALL_MS = 30_000L
        private const val MAX_HINTS = 50
        /** No audio at all from the microphone for this long: it is stuck. */
        private const val MIC_STALL_MS = 10_000L
    }
}

/**
 * The text of one recognition: the final result, unless the last partial says
 * more (a final cut short), else the partial; null when neither has words.
 */
internal fun heardText(final: String?, lastPartial: String?): String? {
    val f = final?.trim()?.takeIf { it.isNotEmpty() }
    val p = lastPartial?.trim()?.takeIf { it.isNotEmpty() }
    if (f == null) return p
    if (p != null && p.length > f.length && p.startsWith(f, ignoreCase = true)) return p
    return f
}

/**
 * Where an utterance ends, from 16 kHz 16-bit mono PCM: [silenceMs] of quiet
 * after speech, or [waitMs] without any. Speech is well above the noise floor
 * (tracked while quiet), so road noise doesn't hold the microphone open.
 */
class Endpointer(private val silenceMs: Int = SILENCE_MS, private val waitMs: Int = WAIT_MS) {
    private var floor = -1.0
    private var elapsedMs = 0.0
    private var quietMs = 0.0
    var spoke = false
        private set

    /** One buffer of audio; true once the utterance is over. */
    fun feed(pcm: ByteArray, length: Int): Boolean {
        val samples = length / 2
        if (samples == 0) return false
        var sum = 0.0
        for (i in 0 until samples) {
            val v = ((pcm[2 * i].toInt() and 0xff) or (pcm[2 * i + 1].toInt() shl 8)).toShort().toDouble()
            sum += v * v
        }
        val rms = kotlin.math.sqrt(sum / samples)
        val ms = samples * 1000.0 / 16_000
        elapsedMs += ms
        if (floor < 0) floor = rms
        val threshold = maxOf(MIN_SPEECH, floor * 3)
        if (rms > threshold) {
            spoke = true
            quietMs = 0.0
        } else {
            floor = floor * 0.9 + rms * 0.1
            if (spoke) quietMs += ms
        }
        return if (spoke) quietMs >= silenceMs else elapsedMs >= waitMs
    }

    companion object {
        /** A driver pauses to think mid-sentence: only a long quiet ends the reply. */
        const val SILENCE_MS = 2_200
        const val WAIT_MS = 8_000
        private const val MIN_SPEECH = 500.0
    }
}
