package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RescanScheduleTest {

    private fun schedule(vararg offsets: Long) =
        RescanSchedule(offsetsMillis = offsets.toList())

    @Test
    fun `an unmatched watched screen asks for a second look`() {
        assertEquals(400L, schedule(400, 1000).onWatchedEvent())
    }

    @Test
    fun `a further event restarts the settle timer`() {
        // A screen emits a burst of events while rendering. Each one means the
        // tree is still moving, so the useful moment to look again is after the
        // burst stops — not once per event.
        val s = schedule(400, 1000)

        assertEquals(400L, s.onWatchedEvent())
        assertEquals(400L, s.onWatchedEvent())
        assertEquals(400L, s.onWatchedEvent())
    }

    @Test
    fun `successive misses walk the offsets outward`() {
        // Lists populate at very different speeds; a single fixed delay either
        // fires too early to see the rows or wastes a wait on every screen.
        val s = schedule(400, 1000, 2000)

        // Deltas, not the offsets themselves — see the cumulative test below.
        assertEquals(400L, s.onWatchedEvent())
        assertEquals(600L, s.onRescanMissed())
        assertEquals(1000L, s.onRescanMissed())
    }

    @Test
    fun `the offsets are absolute — the looks land where they say they do`() {
        // Regression. The returned values are fed straight to postDelayed, which
        // schedules relative to now, so returning the offsets verbatim made each
        // wait stack on the last: [400, 1000, 2000] fired at 0.4s, 1.4s and 3.4s.
        // Measured on device at 0.4s/1.5s/3.5s before the fix, against a KDoc
        // promising the last look lands by ~2s.
        val s = schedule(400, 1000, 2000)

        var elapsed = 0L
        elapsed += s.onWatchedEvent()!!
        assertEquals(400L, elapsed)
        elapsed += s.onRescanMissed()!!
        assertEquals(1000L, elapsed)
        elapsed += s.onRescanMissed()!!
        assertEquals(2000L, elapsed)
    }

    @Test
    fun `restarting the settle timer restarts the elapsed budget`() {
        // A fresh event means a fresh screen, so the next look is 400ms from
        // then — not 400ms from an event two screens ago.
        val s = schedule(400, 1000, 2000)

        s.onWatchedEvent()
        s.onRescanMissed()

        assertEquals(400L, s.onWatchedEvent())
    }

    @Test
    fun `re-looking stops once the offsets are exhausted`() {
        // The bound is the point: without it an unmatched settings screen would
        // be re-walked forever, which is a battery drain on the one package the
        // user is most likely to sit in.
        val s = schedule(400, 1000)

        s.onWatchedEvent()
        s.onRescanMissed()

        assertNull(s.onRescanMissed())
        assertNull(s.onRescanMissed())
    }

    @Test
    fun `a new event re-arms an exhausted schedule`() {
        val s = schedule(400, 1000)

        s.onWatchedEvent()
        s.onRescanMissed()
        assertNull(s.onRescanMissed())

        // The user navigated somewhere else in Settings — that is a fresh screen
        // and gets the full budget again.
        assertEquals(400L, s.onWatchedEvent())
    }

    @Test
    fun `reset re-arms the schedule`() {
        val s = schedule(400, 1000)

        s.onWatchedEvent()
        s.reset()

        assertEquals(400L, s.onWatchedEvent())
    }

    @Test
    fun `an empty offset list never schedules anything`() {
        // The disable switch: a profile that wants no re-looking at all.
        val s = schedule()

        assertNull(s.onWatchedEvent())
        assertNull(s.onRescanMissed())
    }

    @Test
    fun `the default offsets are bounded and increasing`() {
        // Guards the shape rather than the exact numbers: they back off, they
        // stop, and the last look is soon enough to matter.
        val offsets = RescanSchedule.DEFAULT_OFFSETS_MILLIS

        assertTrue("must re-look at least once", offsets.isNotEmpty())
        assertEquals(offsets.sorted(), offsets)
        assertTrue("must give up", offsets.size <= 4)
        assertTrue("last look must land within a few seconds", offsets.last() <= 3_000)
    }

    @Test
    fun `the default schedule terminates`() {
        val s = RescanSchedule()

        assertNotNull(s.onWatchedEvent())
        repeat(RescanSchedule.DEFAULT_OFFSETS_MILLIS.size - 1) { s.onRescanMissed() }

        assertNull(s.onRescanMissed())
    }
}
