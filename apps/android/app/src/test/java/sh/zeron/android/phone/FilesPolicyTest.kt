package sh.zeron.android.phone

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class FilesPolicyTest {
    private val roots = listOf("/storage/emulated/0", "/storage/1A2B-3C4D")
    private val priv = listOf("/data/user/0/sh.zeron.android", "/data/user_de/0/sh.zeron.android")

    @Test fun anywhereInSharedStorageIsReachable() {
        assertNull(FilesPolicy.refusal("/storage/emulated/0/Shiz/a.ogg", roots, priv))
        assertNull(FilesPolicy.refusal("/storage/emulated/0/DCIM/Camera", roots, priv))
        assertNull(FilesPolicy.refusal("/storage/1A2B-3C4D/Music/x.mp3", roots, priv))
        assertNull(FilesPolicy.refusal("/storage/emulated/0", roots, priv))
        assertNull(FilesPolicy.refusal("/storage/emulated/0/Android", roots, priv))
        assertNull(FilesPolicy.refusal("/storage/emulated/0/Android/media/x", roots, priv))
    }

    @Test fun androidDataAndObbAreClosed() {
        assertNotNull(FilesPolicy.refusal("/storage/emulated/0/Android/data", roots, priv))
        assertNotNull(FilesPolicy.refusal("/storage/emulated/0/Android/data/com.whatsapp/files", roots, priv))
        assertNotNull(FilesPolicy.refusal("/storage/emulated/0/Android/obb/x", roots, priv))
        assertNotNull(FilesPolicy.refusal("/storage/1A2B-3C4D/android/DATA/x", roots, priv))
    }

    @Test fun outsideStorageIsRefused() {
        for (p in listOf("/", "/data/data/sh.zeron.android/files/x", "/system/etc/hosts", "/storage", "/storage/emulated", "/storage/self/primary", "/sdcard2/x", "/storage/emulated/01/x")) {
            assertNotNull(p, FilesPolicy.refusal(p, roots, priv))
        }
    }

    @Test fun zeronsOwnPrivateDataIsRefusedEvenIfItIsLinkedIntoStorage() {
        assertNotNull(FilesPolicy.refusal("/data/user/0/sh.zeron.android/files/x", listOf("/data/user/0"), priv))
    }

    @Test fun dotDotCannotClimbOut() {
        assertEquals("/storage/emulated/0/x", FilesPolicy.clean("/storage/emulated/0/Shiz/../x"))
        assertEquals("/data", FilesPolicy.clean("/storage/emulated/0/../../../data"))
        assertNotNull(FilesPolicy.refusal(FilesPolicy.clean("/storage/emulated/0/../../../data"), roots, priv))
        assertNotNull(FilesPolicy.refusal(FilesPolicy.clean("/storage/emulated/0/Android/data/../data/x"), roots, priv))
    }
}
