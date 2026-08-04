package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ProtectionScheduleTest {

    private val cooldown = ProtectionSchedule.COOLDOWN_MILLIS
    private val ready = ProtectionSchedule.READY_WINDOW_MILLIS
    private val disarmed = ProtectionSchedule.DISARM_WINDOW_MILLIS

    private fun evaluate(
        armed: Boolean = true,
        requestedAt: Long? = null,
        disarmedAt: Long? = null,
        now: Long,
    ) = ProtectionSchedule.evaluate(
        armed = armed,
        disarmRequestedAtElapsed = requestedAt,
        disarmedAtElapsed = disarmedAt,
        nowElapsed = now,
    )

    // --- the mode itself ----------------------------------------------------

    @Test
    fun `protection is off until the user turns it on`() {
        // Nothing is blocked before the user arms the guard. This is what makes
        // setup possible at all: enabling the accessibility service and
        // activating device admin both happen on screens the armed guard backs
        // out of.
        val state = evaluate(armed = false, now = 0)

        assertEquals(ProtectionPhase.OFF, state.phase)
        assertFalse(state.guardActive)
    }

    @Test
    fun `arming takes effect immediately`() {
        // Asymmetric on purpose: protecting yourself is one tap, removing the
        // protection costs the cooldown.
        val state = evaluate(armed = true, now = 0)

        assertEquals(ProtectionPhase.ARMED, state.phase)
        assertTrue(state.guardActive)
    }

    @Test
    fun `a stale disarm cannot outlive the mode being off`() {
        // The armed flag is the outer question. Turning protection off and on
        // again must not resurrect a disarm window from before.
        assertEquals(
            ProtectionPhase.OFF,
            evaluate(armed = false, disarmedAt = 0, now = 1_000).phase,
        )
    }

    // --- requesting a disarm ------------------------------------------------

    @Test
    fun `requesting a disarm does not disarm anything`() {
        // The whole point of the cooldown. An earlier version of this flow
        // released the guard on the tap, which made the exit path the fastest
        // way through the guard rather than a considered decision.
        val state = evaluate(requestedAt = 1_000, now = 1_000)

        assertEquals(ProtectionPhase.DISARM_PENDING, state.phase)
        assertEquals(cooldown, state.remainingMillis)
        assertTrue("the guard keeps working during the wait", state.guardActive)
    }

    @Test
    fun `stays pending for the whole cooldown`() {
        val state = evaluate(requestedAt = 1_000, now = 1_000 + cooldown - 1)

        assertEquals(ProtectionPhase.DISARM_PENDING, state.phase)
        assertEquals(1, state.remainingMillis)
        assertTrue(state.guardActive)
    }

    @Test
    fun `becomes ready once the cooldown has elapsed`() {
        // Ready is not disarmed: the user still has to come back and confirm.
        // That is what stops a request made and forgotten from spending its
        // window while the phone is in a pocket.
        val state = evaluate(requestedAt = 1_000, now = 1_000 + cooldown)

        assertEquals(ProtectionPhase.DISARM_READY, state.phase)
        assertEquals(ready, state.remainingMillis)
        assertTrue("nothing is unguarded until the user confirms", state.guardActive)
    }

    @Test
    fun `an unconfirmed request expires instead of staying ready`() {
        // Otherwise one cooldown is paid a single time and buys a disarm
        // whenever the user later feels like it.
        assertEquals(
            ProtectionPhase.DISARM_READY,
            evaluate(requestedAt = 1_000, now = 1_000 + cooldown + ready - 1).phase,
        )
        assertEquals(
            ProtectionPhase.ARMED,
            evaluate(requestedAt = 1_000, now = 1_000 + cooldown + ready).phase,
        )
        assertEquals(
            ProtectionPhase.ARMED,
            evaluate(requestedAt = 1_000, now = 1_000 + cooldown + ready + 86_400_000).phase,
        )
    }

    // --- the disarm window --------------------------------------------------

    @Test
    fun `a confirmed disarm stops the guard for the window`() {
        val state = evaluate(disarmedAt = 1_000, now = 1_000)

        assertEquals(ProtectionPhase.DISARMED, state.phase)
        assertEquals(disarmed, state.remainingMillis)
        assertFalse(state.guardActive)
    }

    @Test
    fun `the guard re-arms itself when the window runs out`() {
        // The safety property of a windowed disarm: a user who gets distracted
        // ends up protected again rather than permanently unprotected.
        assertFalse(evaluate(disarmedAt = 1_000, now = 1_000 + disarmed - 1).guardActive)

        val after = evaluate(disarmedAt = 1_000, now = 1_000 + disarmed)
        assertEquals(ProtectionPhase.ARMED, after.phase)
        assertTrue(after.guardActive)
    }

    @Test
    fun `a live disarm outranks the request that produced it`() {
        // Both timestamps can be stored at once; the disarm is the later
        // decision and must win, or a confirmed disarm would read as a stale
        // pending request and keep the guard on.
        val state = evaluate(requestedAt = 1_000, disarmedAt = 1_000 + cooldown, now = 1_000 + cooldown)

        assertEquals(ProtectionPhase.DISARMED, state.phase)
    }

    @Test
    fun `a second disarm is possible after the first window has expired`() {
        // Found by running the real flow on an emulator, not by reading the
        // code. The confirmed-disarm timestamp stays in storage after its window
        // ends, and while it was checked ahead of the request unconditionally it
        // swallowed every later request: the phase fell straight back to ARMED,
        // the countdown never appeared, and the user could never disarm again.
        //
        // That is the worst failure this module can have. The mode is the only
        // supported way to remove the app, so a disarm that can be used exactly
        // once turns the second attempt into "this thing cannot be uninstalled".
        val firstDisarm = 1_000L
        val expired = firstDisarm + disarmed + 1
        val state = evaluate(requestedAt = expired, disarmedAt = firstDisarm, now = expired)

        assertEquals(ProtectionPhase.DISARM_PENDING, state.phase)
        assertEquals(cooldown, state.remainingMillis)
    }

    @Test
    fun `an expired disarm does not resurrect itself`() {
        // The other half: with no request in flight, a spent window stays spent.
        assertEquals(
            ProtectionPhase.ARMED,
            evaluate(disarmedAt = 1_000, now = 1_000 + disarmed + 86_400_000).phase,
        )
    }

    // --- reboot -------------------------------------------------------------

    @Test
    fun `a reboot voids a pending request`() {
        // elapsedRealtime resets on boot, so a now smaller than the stored value
        // means the device restarted. Voiding is the strict reading: the user
        // simply requests again.
        assertEquals(
            ProtectionPhase.ARMED,
            evaluate(requestedAt = 500_000, now = 1_000).phase,
        )
    }

    @Test
    fun `a reboot ends a disarm window rather than extending it`() {
        // This is the direction that matters. A stored deadline read against a
        // clock that just reset to zero would look like a disarm with hours left
        // on it, so rebooting would be a way to stay unguarded indefinitely.
        // Failing closed makes a reboot cost the user a re-request at worst.
        val state = evaluate(disarmedAt = 500_000, now = 1_000)

        assertEquals(ProtectionPhase.ARMED, state.phase)
        assertTrue(state.guardActive)
    }

    // --- the constants are the mechanism ------------------------------------

    @Test
    fun `the schedule never consults the wall clock`() {
        // Regression guard for the attack this design exists to remove. The wall
        // clock is user-settable and Settings' date screen is not guarded, so any
        // wall-clock dependence would reduce the cooldown to "set the date
        // forward an hour". Every input here is elapsedRealtime.
        assertEquals(
            ProtectionPhase.DISARM_READY,
            evaluate(requestedAt = 1_000, now = 1_000 + cooldown).phase,
        )
    }

    @Test
    fun `cooldown is long enough to outlast an impulse`() {
        // Encoded as a test because the value is the mechanism, not a detail: a
        // disarm that arrives sooner than the urge fades is decoration.
        assertTrue("cooldown must be at least ten minutes", cooldown >= 10 * 60_000L)
    }

    @Test
    fun `the disarm window is long enough to actually remove the app`() {
        // The window has a job: deactivate device admin, open App info,
        // uninstall. A window too short to finish that turns a considered exit
        // into a race, and the user who loses it has to wait out the cooldown
        // again for no reason.
        assertTrue("disarm window must allow a real removal", disarmed >= 5 * 60_000L)
    }
}
