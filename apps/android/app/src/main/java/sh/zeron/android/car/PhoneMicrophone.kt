package sh.zeron.android.car

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.util.Log
import androidx.core.content.ContextCompat

/** The phone's own microphone as a 16 kHz mono PCM stream, for when the car's isn't available. */
object PhoneMicrophone {
    private const val TAG = "ZeronCar"

    fun open(context: Context): AudioStream? {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) return null
        return runCatching {
            val minimum = AudioRecord.getMinBufferSize(16_000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
            @Suppress("MissingPermission")
            val record = AudioRecord(
                MediaRecorder.AudioSource.VOICE_RECOGNITION,
                16_000,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                maxOf(minimum, 16_000),
            )
            check(record.state == AudioRecord.STATE_INITIALIZED) { "microphone busy" }
            record.startRecording()
            object : AudioStream {
                override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
                    val n = record.read(buffer, offset, length)
                    return if (n < 0) -1 else n
                }

                override fun stop() {
                    runCatching { record.stop() }
                    record.release()
                }
            }
        }.onFailure { Log.w(TAG, "phone microphone failed", it) }.getOrNull()
    }
}
