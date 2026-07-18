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

        assertEquals(400L, s.onWatchedEvent())
        assertEquals(1000L, s.onRescanMissed())
        assertEquals(2000L, s.onRescanMissed())
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
