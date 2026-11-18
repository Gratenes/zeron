package sh.zeron.android.phone

import org.junit.Assert.assertEquals
import org.junit.Test

class QueuePolicyTest {
    private val life = PhoneSendQueue_LIFETIME

    @Test fun waitsWhileThePersonIsOnThePhone() =
        assertEquals(QueuePolicy.Decision.WAIT, QueuePolicy.decide(1_000, 0, true, life))

    @Test fun goesOnceTheyAreIdle() =
        assertEquals(QueuePolicy.Decision.GO, QueuePolicy.decide(40_000, 0, false, life))

    @Test fun givesUpAfterTheTimeLimitEvenIfTheyAreStillBusy() {
        assertEquals(QueuePolicy.Decision.EXPIRED, QueuePolicy.decide(life + 1, 0, true, life))
        assertEquals(QueuePolicy.Decision.EXPIRED, QueuePolicy.decide(life + 1, 0, false, life))
        assertEquals(QueuePolicy.Decision.WAIT, QueuePolicy.decide(life, 0, true, life))
    }

    @Test fun aBusyRetryStaysQueuedAndAnythingElseFails() {
        assertEquals("queued", QueuePolicy.afterAttempt(true))
        assertEquals("failed", QueuePolicy.afterAttempt(false))
    }
}

private const val PhoneSendQueue_LIFETIME = 10 * 60_000L
