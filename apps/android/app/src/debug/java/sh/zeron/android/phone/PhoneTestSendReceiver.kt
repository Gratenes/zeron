package sh.zeron.android.phone

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/** Debug builds: `am broadcast -a sh.zeron.android.TEST_SEND --es to N --es text T --es via sms|auto|rcs` runs phone_sms_send; the result is in logcat tag ZeronSend. */
class PhoneTestSendReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pending = goAsync()
        Thread {
            val out = runCatching {
                val sms = PhoneSms(context)
                when (intent.getStringExtra("tool")) {
                    "attachment" -> return@runCatching sms.attachment(intent.getStringExtra("message_id") ?: "", intent.getStringExtra("part_id") ?: "").toString().take(600)
                    "setting" -> {
                        val st = PhoneSettings(context)
                        val name = intent.getStringExtra("name") ?: ""
                        val on = intent.getBooleanExtra("value", true)
                        if (name == "confirm-installs") st.confirmInstalls = on else PhoneSettings.Group.entries.firstOrNull { it.key == name }?.let { st.setGroup(it, on) }
                        return@runCatching "set $name=$on"
                    }
                    "sys" -> {
                        val sys = PhoneSystem(context, PhoneSettings(context))
                        return@runCatching when (intent.getStringExtra("op")) {
                            "install" -> sys.install(intent.getStringExtra("path") ?: "", intent.getStringExtra("sha"), "Test chat", "chat-test-1").toString()
                            "grant" -> sys.grant(intent.getStringExtra("pkg") ?: "", intent.getStringExtra("perm") ?: "", intent.getBooleanExtra("revoke", false)).toString()
                            "stop" -> sys.forceStop(intent.getStringExtra("pkg") ?: "").toString()
                            "get" -> sys.settingsGet(intent.getStringExtra("ns") ?: "", intent.getStringExtra("key") ?: "").toString()
                            "probe" -> sys.probe().toString()
                            "shzreq" -> { PhoneShizuku.requestPermission(); PhoneShizuku.state(context).let { "requested; running=${it.running} permitted=${it.permitted}" } }
                            "shzstate" -> PhoneShizuku.state(context).let { "installed=${it.installed} running=${it.running} permitted=${it.permitted} uid=${it.uid} v=${it.version}" }
                            else -> "bad op"
                        }
                    }
                    "queue" -> {
                        val mode = intent.getStringExtra("mode") ?: "ok"
                        var tries = 0
                        return@runCatching PhoneSendQueue.enqueue(context, "5590", "Test person", intent.getStringExtra("text") ?: "queued hello", "busy", false) { force ->
                            tries++
                            when {
                                mode == "busy2" && tries <= 2 && !force -> throw PhoneBusyException("still busy")
                                mode == "fail" -> throw PhoneToolException("Messages didn't open")
                                mode == "never" && !force -> throw PhoneBusyException("always busy")
                                else -> org.json.JSONObject().put("ok", true).put("tries", tries).put("force", force)
                            }
                        }.toString()
                    }
                    "queueact" -> { PhoneSendQueue.act(context, intent.getIntExtra("id", -1), intent.getStringExtra("do") ?: ""); return@runCatching PhoneSendQueue.status(null).toString() }
                    "queuestatus" -> return@runCatching PhoneSendQueue.status(null).toString()
                    "files" -> {
                        val f = PhoneFiles(context)
                        val path = intent.getStringExtra("path") ?: ""
                        return@runCatching when (intent.getStringExtra("op")) {
                            "list" -> f.list(path, null).toString()
                            "stat" -> f.stat(path).toString()
                            "hash" -> f.hash(path).toString()
                            "read" -> f.read(path, intent.getLongExtra("offset", 0), intent.getIntExtra("len", 2400)).toString()
                            "write" -> f.writePart(intent.getStringExtra("id") ?: "t", path, intent.getLongExtra("offset", 0), android.util.Base64.decode(intent.getStringExtra("data") ?: "", android.util.Base64.DEFAULT), intent.getBooleanExtra("final", true), intent.getStringExtra("sha"), intent.getBooleanExtra("overwrite", false), null).toString()
                            else -> "bad op"
                        }
                    }
                    "read" -> return@runCatching sms.read(intent.getStringExtra("thread_id"), null, 10, null).toString()
                }
                sms.send(intent.getStringExtra("to"), null, intent.getStringExtra("text") ?: "test", intent.getStringExtra("via") ?: "auto", null).toString()
            }.getOrElse { "ERROR ${it.message}" }
            Log.i("ZeronSend", out.take(3800))
            pending.finish()
        }.start()
    }
}
