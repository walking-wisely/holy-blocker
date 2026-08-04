package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class FrameGateTest {

    private fun gate() = FrameGate(
        minIntervalMillis = 1_000,
        changeThresholdBits = 6,
    )

    /** A hash differing from [from] in exactly [bits] places. */
    private fun differing(from: Long, bits: Int): Long {
        var out = from
        for (i in 0 until bits) out = out xor (1L shl i)
        return out
    }

    @Test
    fun `the first frame of a session is always analysed`() {
        assertEquals(FrameOutcome.Analyse, gate().onFrame(hash = 0x1234L, nowMillis = 0))
    }

    @Test
    fun `a frame arriving too soon is dropped however different it is`() {
        val gate = gate()
        gate.onFrame(hash = 0L, nowMillis = 0)

        // -1 differs from 0 in all 64 bits: as different as a frame can be, and
        // still not worth a model pass 40ms after the last one.
        assertEquals(
            FrameOutcome.Skipped(FrameSkipReason.TOO_SOON),
            gate.onFrame(hash = -1L, nowMillis = 40),
        )
    }

    @Test
    fun `a near-identical frame after the interval is dropped as unchanged`() {
        val gate = gate()
        gate.onFrame(hash = 0L, nowMillis = 0)

        // A blinking cursor or a ticking clock moves a handful of bits.
        assertEquals(
            FrameOutcome.Skipped(FrameSkipReason.UNCHANGED),
            gate.onFrame(hash = differing(0L, 3), nowMillis = 2_000),
        )
    }

    @Test
    fun `a changed frame after the interval is analysed`() {
        val gate = gate()
        gate.onFrame(hash = 0L, nowMillis = 0)

        assertEquals(FrameOutcome.Analyse, gate.onFrame(hash = differing(0L, 9), nowMillis = 2_000))
    }

    @Test
    fun `the threshold is a floor, not a ceiling`() {
        val gate = gate()
        gate.onFrame(hash = 0L, nowMillis = 0)

        // Exactly at the threshold counts as changed; one below does not. Stated
        // as a test because "6 bits different" is otherwise ambiguous.
        assertEquals(FrameOutcome.Analyse, gate.onFrame(hash = differing(0L, 6), nowMillis = 2_000))
    }

    @Test
    fun `a dropped frame does not become the baseline`() {
        val gate = gate()
        gate.onFrame(hash = 0L, nowMillis = 0)
        // Dropped for arriving too soon — and it is wildly different, so if it
        // were taken as the baseline the next frame would look like a change.
        gate.onFrame(hash = -1L, nowMillis = 40)

        assertEquals(
            FrameOutcome.Skipped(FrameSkipReason.UNCHANGED),
            gate.onFrame(hash = differing(0L, 2), nowMillis = 5_000),
        )
    }

    @Test
    fun `an unchanged screen does not keep resetting the clock`() {
        val gate = gate()
        gate.onFrame(hash = 0L, nowMillis = 0)
        gate.onFrame(hash = 0L, nowMillis = 5_000) // unchanged
        // A static screen redraws for its own reasons. When it finally does
        // change, the change is analysed at once rather than waiting out another
        // interval measured from the last *skip*.
        assertEquals(FrameOutcome.Analyse, gate.onFrame(hash = -1L, nowMillis = 5_100))
    }

    @Test
    fun `the interval is measured from the last analysed frame`() {
        val gate = gate()
        gate.onFrame(hash = 0L, nowMillis = 0)
        gate.onFrame(hash = -1L, nowMillis = 2_000) // analysed

        assertEquals(
            FrameOutcome.Skipped(FrameSkipReason.TOO_SOON),
            gate.onFrame(hash = 0L, nowMillis = 2_500),
        )
    }

    @Test
    fun `a new session starts from nothing`() {
        val gate = gate()
        gate.onFrame(hash = 0L, nowMillis = 0)
        gate.reset()

        // Consent is per session, so a second session is a second grant, and the
        // screen behind it is not assumed to be the one this session ended on.
        assertEquals(FrameOutcome.Analyse, gate.onFrame(hash = 0L, nowMillis = 10))
    }

    @Test
    fun `defaults are sane enough to run with`() {
        val gate = FrameGate()

        assertEquals(FrameOutcome.Analyse, gate.onFrame(hash = 0L, nowMillis = 0))
        assertTrue(gate.onFrame(hash = 0L, nowMillis = 1) is FrameOutcome.Skipped)
    }
}
