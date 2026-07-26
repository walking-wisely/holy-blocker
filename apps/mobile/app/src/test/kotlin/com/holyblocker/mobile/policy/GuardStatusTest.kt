package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class GuardStatusTest {

    private fun conditions(
        serviceEnabled: Boolean = true,
        phase: ProtectionPhase = ProtectionPhase.ARMED,
        remainingMillis: Long = 0,
        deviceRecognised: Boolean = true,
    ) = GuardConditions(
        serviceEnabled = serviceEnabled,
        protection = ProtectionState(phase, remainingMillis),
        deviceRecognised = deviceRecognised,
    )

    // ---- health -------------------------------------------------------------

    @Test
    fun `armed with the service running is protecting`() {
        assertEquals(GuardHealth.PROTECTING, GuardStatus.health(conditions()))
    }

    @Test
    fun `armed with the service disabled is unprotected`() {
        // The whole reason this service exists. Nothing inside the accessibility
        // service can observe its own disabling by adb or by a settings screen
        // the guard does not recognise: it is already gone by then. A separate
        // still-alive process is the only thing that can notice.
        assertEquals(
            GuardHealth.UNPROTECTED,
            GuardStatus.health(conditions(serviceEnabled = false)),
        )
    }

    @Test
    fun `a disarm still in progress is still protecting`() {
        // The waiting phases guard exactly as ARMED does — that is what makes the
        // cooldown a cooldown rather than a notice period.
        for (phase in listOf(ProtectionPhase.DISARM_PENDING, ProtectionPhase.DISARM_READY)) {
            assertEquals(GuardHealth.PROTECTING, GuardStatus.health(conditions(phase = phase)))
            assertEquals(
                GuardHealth.UNPROTECTED,
                GuardStatus.health(conditions(serviceEnabled = false, phase = phase)),
            )
        }
    }

    @Test
    fun `a released or unarmed guard is idle, not unprotected`() {
        // Turning the service off inside a release window is the front door, and
        // recording it as a failure would make the supported exit path look like
        // an attack. Same for a user who has never armed anything.
        for (phase in listOf(ProtectionPhase.OFF, ProtectionPhase.DISARMED)) {
            assertEquals(
                GuardHealth.IDLE,
                GuardStatus.health(conditions(serviceEnabled = false, phase = phase)),
            )
        }
    }

    @Test
    fun `an unverified device is still protecting`() {
        // The settings screens are not guarded there, but the content scan and
        // the cover — the product's actual job — run regardless.
        assertEquals(
            GuardHealth.PROTECTING,
            GuardStatus.health(conditions(deviceRecognised = false)),
        )
    }

    // ---- what reaches the tamper log ---------------------------------------

    @Test
    fun `falling unprotected is recorded`() {
        assertEquals(
            TamperEvent.GUARD_UNPROTECTED,
            GuardStatus.noticeOn(previous = GuardHealth.PROTECTING, current = GuardHealth.UNPROTECTED),
        )
    }

    @Test
    fun `starting up already unprotected is recorded`() {
        // No previous observation is not the same as a healthy one. A device that
        // boots with protection armed and the service disabled has a gap worth a
        // line, and it is the shape an adb disable leaves behind after a reboot.
        assertEquals(
            TamperEvent.GUARD_UNPROTECTED,
            GuardStatus.noticeOn(previous = null, current = GuardHealth.UNPROTECTED),
        )
    }

    @Test
    fun `staying unprotected is recorded once`() {
        // This is polled on a timer, so without the transition check an evening
        // spent with the service off would write an entry a minute until it
        // pushed the rest of the history past the cap.
        assertNull(
            GuardStatus.noticeOn(previous = GuardHealth.UNPROTECTED, current = GuardHealth.UNPROTECTED),
        )
    }

    @Test
    fun `healthy transitions write nothing`() {
        // Every one of these already has a writer: the service records its own
        // connect and unbind, and ProtectionStore records each mode transition.
        // A second entry from here would be the same fact twice.
        for (previous in listOf(null, GuardHealth.PROTECTING, GuardHealth.IDLE, GuardHealth.UNPROTECTED)) {
            for (current in listOf(GuardHealth.PROTECTING, GuardHealth.IDLE)) {
                assertNull("$previous -> $current", GuardStatus.noticeOn(previous, current))
            }
        }
    }

    // ---- what the notification says ----------------------------------------

    @Test
    fun `a guard that is not running outranks every other message`() {
        // The one state the user has to be told about, so it wins over the
        // countdowns and over the unverified-device notice.
        for (phase in listOf(ProtectionPhase.ARMED, ProtectionPhase.DISARM_PENDING, ProtectionPhase.DISARM_READY)) {
            assertEquals(
                StatusMessage.GUARD_NOT_RUNNING,
                GuardStatus.message(
                    conditions(serviceEnabled = false, phase = phase, deviceRecognised = false),
                ),
            )
        }
    }

    @Test
    fun `each phase has its own message`() {
        assertEquals(StatusMessage.PROTECTING, GuardStatus.message(conditions(phase = ProtectionPhase.ARMED)))
        assertEquals(
            StatusMessage.DISARM_PENDING,
            GuardStatus.message(conditions(phase = ProtectionPhase.DISARM_PENDING)),
        )
        assertEquals(
            StatusMessage.DISARM_READY,
            GuardStatus.message(conditions(phase = ProtectionPhase.DISARM_READY)),
        )
        assertEquals(StatusMessage.DISARMED, GuardStatus.message(conditions(phase = ProtectionPhase.DISARMED)))
        assertEquals(StatusMessage.IDLE, GuardStatus.message(conditions(phase = ProtectionPhase.OFF)))
    }

    @Test
    fun `an unverified device is said so rather than claimed as protection`() {
        // Same rule as the onboarding screen: the guard genuinely does nothing on
        // an unrecognised build, and a notification claiming otherwise is worse
        // than no notification at all.
        assertEquals(
            StatusMessage.DEVICE_UNVERIFIED,
            GuardStatus.message(conditions(phase = ProtectionPhase.ARMED, deviceRecognised = false)),
        )
    }

    @Test
    fun `an unverified device is not mentioned while nothing is armed`() {
        // Nothing is being guarded either way, so the notice would be noise.
        assertEquals(
            StatusMessage.IDLE,
            GuardStatus.message(conditions(phase = ProtectionPhase.OFF, deviceRecognised = false)),
        )
    }

    @Test
    fun `a countdown outranks the unverified notice`() {
        // The release the user asked for is the more useful thing to show while
        // it is running; the device notice is static and is on the onboarding
        // screen too.
        assertEquals(
            StatusMessage.DISARM_READY,
            GuardStatus.message(
                conditions(phase = ProtectionPhase.DISARM_READY, deviceRecognised = false),
            ),
        )
    }

    // ---- when the service is worth running ---------------------------------

    @Test
    fun `it keeps running while there is anything to report`() {
        assertTrue(GuardStatus.shouldKeepRunning(conditions()))
        assertTrue(GuardStatus.shouldKeepRunning(conditions(serviceEnabled = false)))
        assertTrue(GuardStatus.shouldKeepRunning(conditions(phase = ProtectionPhase.DISARMED)))
        // Protection off but the guard still scanning: the ongoing notification
        // is how an accessibility service that reads the screen stays visible.
        assertTrue(GuardStatus.shouldKeepRunning(conditions(phase = ProtectionPhase.OFF)))
    }

    @Test
    fun `it stops once nothing is armed and nothing is running`() {
        // A permanent notification for a guard that is neither scanning nor
        // blocking is a notification with nothing behind it.
        assertFalse(
            GuardStatus.shouldKeepRunning(
                conditions(serviceEnabled = false, phase = ProtectionPhase.OFF),
            ),
        )
    }

    @Test
    fun `the health check runs often enough to be a check`() {
        // A ceiling rather than a value: the poll is what turns "the guard was
        // switched off" from something noticed at the next connect into
        // something noticed while it is happening. A floor as well, because this
        // wakes on a timer and reads SharedPreferences plus a secure setting.
        assertTrue(GuardStatus.CHECK_INTERVAL_MILLIS in 15_000..5 * 60_000L)
    }
}
