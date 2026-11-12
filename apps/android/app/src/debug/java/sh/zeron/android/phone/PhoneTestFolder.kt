package sh.zeron.android.phone

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Debug builds: `am broadcast -n sh.zeron.android/.phone.PhoneTestFolderReceiver -a sh.zeron.android.TEST_FOLDER --es path /sdcard/X --ez write true` allows a folder and turns the Files group on. */
class PhoneTestFolderReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val path = intent.getStringExtra("path") ?: return
        PhoneFolders.add(context, path, null, intent.getBooleanExtra("write", false))
        PhoneSettings(context).setGroup(PhoneSettings.Group.FILES, true)
        android.util.Log.i("ZeronPhone", "TEST folder allowed: $path")
    }
}
