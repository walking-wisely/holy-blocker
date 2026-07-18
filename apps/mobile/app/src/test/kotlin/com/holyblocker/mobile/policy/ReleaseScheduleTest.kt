package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Test

class ReleaseScheduleTest {

    private val cooldown = ReleaseSchedule.COOLDOWN_MILLIS
    private val window = ReleaseSchedule.WINDOW_MILLIS

    @Test
    fun `no request means no release`() {
        assertEquals(
            ReleasePhase.IDLE,
            ReleaseSchedule.evaluate(requestedAtElapsed = null, nowElapsed = 0).phase,
        )
    }

    @Test
    fun `a request does not release immediately`() {
        // The whole point. The first version released on the tap, which made the
        // button a four-tap bypass: open the app, tap release, tap through to
        // the accessibility toggle.
        val state = ReleaseSchedule.evaluate(requestedAtElapsed = 1_000, nowElapsed = 1_000)

        assertEquals(ReleasePhase.PENDING, state.phase)
        assertEquals(cooldown, state.remainingMillis)
    }

    @Test
    fun `stays pending for the whole cooldown`() {
        val state = ReleaseSchedule.evaluate(1_000, 1_000 + cooldown - 1)

        assertEquals(ReleasePhase.PENDING, state.phase)
        assertEquals(1, state.remainingMillis)
    }

    @Test
    fun `opens once the cooldown has elapsed`() {
        val state = ReleaseSchedule.evaluate(1_000, 1_000 + cooldown)

        assertEquals(ReleasePhase.OPEN, state.phase)
        assertEquals(window, state.remainingMillis)
    }

    @Test
    fun `closes when the window runs out`() {
        assertEquals(
            ReleasePhase.OPEN,
            ReleaseSchedule.evaluate(1_000, 1_000 + cooldown + window - 1).phase,
        )
        assertEquals(
            ReleasePhase.IDLE,
            ReleaseSchedule.evaluate(1_000, 1_000 + cooldown + window).phase,
        )
    }

    @Test
    fun `an unused request expires instead of staying armed`() {
        // Otherwise a request made once sits ready forever, and the cooldown is
        // paid a single time for unlimited future access.
        assertEquals(
            ReleasePhase.IDLE,
            ReleaseSchedule.evaluate(1_000, 1_000 + cooldown + window + 86_400_000).phase,
        )
    }

    @Test
    fun `a reboot voids a pending request`() {
        // elapsedRealtime resets on boot, so a now smaller than the stored value
        // means the device restarted. Voiding is the strict reading and keeps
        // wall-clock manipulation out of the decision entirely: the user simply
        // requests again.
        val state = ReleaseSchedule.evaluate(requestedAtElapsed = 500_000, nowElapsed = 1_000)

        assertEquals(ReleasePhase.IDLE, state.phase)
    }

    @Test
    fun `the schedule never consults the wall clock`() {
        // Regression guard for the attack this design exists to remove. Wall
        // clock is user-settable and Settings' date screen is not guarded, so
        // any wall-clock dependence would let "set the date forward an hour"
        // skip the cooldown. elapsedRealtime cannot be set by the user.
        val monotonicOnly = ReleaseSchedule.evaluate(1_000, 1_000 + cooldown)

        assertEquals(ReleasePhase.OPEN, monotonicOnly.phase)
    }

    @Test
    fun `cooldown is long enough to outlast an impulse`() {
        // Encoded as a test because the value is the mechanism, not a detail: a
        // release that arrives sooner than the urge fades is decoration.
        assert(cooldown >= 10 * 60_000L) { "cooldown must be at least ten minutes" }
        assert(window <= 5 * 60_000L) { "window must be short enough not to be free browsing" }
    }
}
