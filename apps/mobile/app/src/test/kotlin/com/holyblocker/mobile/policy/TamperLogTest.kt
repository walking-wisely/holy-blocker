package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TamperLogTest {

    private fun entry(
        wall: Long = 1_700_000_000_000,
        elapsed: Long = 10_000,
        event: TamperEvent = TamperEvent.SERVICE_CONNECTED,
        detail: String = "",
    ) = TamperEntry(wallMillis = wall, elapsedMillis = elapsed, event = event, detail = detail)

    // ---- format and parse ---------------------------------------------------

    @Test
    fun `an entry survives a round trip`() {
        val original = entry(event = TamperEvent.REMOVAL_SCREEN_BLOCKED, detail = "ACCESSIBILITY")

        assertEquals(original, TamperLog.parse(TamperLog.format(original)))
    }

    @Test
    fun `every event code round trips`() {
        // The codes are the on-disk format. Renaming an enum constant is free;
        // changing its code silently orphans every entry already written, which
        // for an append-only record is data loss rather than a migration.
        for (event in TamperEvent.entries) {
            assertEquals(event, TamperLog.parse(TamperLog.format(entry(event = event)))?.event)
        }
    }

    @Test
    fun `event codes are unique`() {
        assertEquals(
            TamperEvent.entries.size,
            TamperEvent.entries.map { it.code }.toSet().size,
        )
    }

    @Test
    fun `a detail carrying separators cannot break the line`() {
        // Details come from surface names and package names, but the format has
        // to hold regardless: one entry that swallowed a newline would shift
        // every field of the next line and corrupt the record from that point on.
        val written = TamperLog.format(entry(detail = "a\tb\nc\r\nd"))

        assertEquals(1, written.lines().size)
        assertEquals("a b c d", TamperLog.parse(written)?.detail)
    }

    @Test
    fun `a long detail is truncated rather than written whole`() {
        val parsed = TamperLog.parse(TamperLog.format(entry(detail = "x".repeat(500))))

        assertTrue((parsed?.detail?.length ?: 0) <= TamperLog.MAX_DETAIL_CHARS)
    }

    @Test
    fun `an unreadable line is skipped, not fatal`() {
        // A process killed mid-append leaves a partial line, and the killed-mid-
        // append case is exactly the tamper this log exists to record. Refusing
        // to read the file because its last line is short would lose the whole
        // history to the event it was there to capture.
        assertNull(TamperLog.parse(""))
        assertNull(TamperLog.parse("garbage"))
        assertNull(TamperLog.parse("1700000000000\t10000"))
        assertNull(TamperLog.parse("not-a-number\t10000\tservice_on\t"))
        assertNull(TamperLog.parse("1700000000000\t10000\tno_such_event\t"))
    }

    @Test
    fun `readAll keeps the good lines from a damaged file`() {
        val good = TamperLog.format(entry())
        val lines = listOf(good, "1700000000000\t10000\ttrunc", good)

        assertEquals(2, TamperLog.readAll(lines).size)
    }

    @Test
    fun `an unknown code from a newer build is skipped without dropping the rest`() {
        // Downgrades are not a supported flow, but a log written by a newer build
        // and read by an older one must degrade to "some entries I do not
        // understand", never to an unreadable file.
        val lines = listOf(
            "1700000000000\t10000\tfrom_the_future\tdetail",
            TamperLog.format(entry()),
        )

        assertEquals(listOf(TamperEvent.SERVICE_CONNECTED), TamperLog.readAll(lines).map { it.event })
    }

    // ---- coalescing ---------------------------------------------------------

    @Test
    fun `a repeated identical event within the window is suppressed`() {
        // A guarded screen fires several events while it renders, and each one
        // that matches produces a block. Without this the log fills with one
        // screen visit and the entries that matter age out of the cap.
        val first = entry(elapsed = 10_000, event = TamperEvent.REMOVAL_SCREEN_BLOCKED, detail = "ACCESSIBILITY")
        val again = first.copy(elapsedMillis = 10_500)

        assertFalse(TamperLog.shouldRecord(previous = first, candidate = again))
    }

    @Test
    fun `the same event on a different surface is always recorded`() {
        val first = entry(elapsed = 10_000, event = TamperEvent.REMOVAL_SCREEN_BLOCKED, detail = "ACCESSIBILITY")
        val other = first.copy(elapsedMillis = 10_500, detail = "APP_INFO")

        assertTrue(TamperLog.shouldRecord(previous = first, candidate = other))
    }

    @Test
    fun `a repeat after the window is a fresh visit`() {
        // Coming back to the same screen a minute later is a second attempt, and
        // the count of attempts is most of what makes this log worth keeping.
        val first = entry(elapsed = 10_000, event = TamperEvent.REMOVAL_SCREEN_BLOCKED, detail = "ACCESSIBILITY")
        val later = first.copy(elapsedMillis = 10_000 + TamperLog.COALESCE_MILLIS)

        assertTrue(TamperLog.shouldRecord(previous = first, candidate = later))
    }

    @Test
    fun `coalescing never suppresses across a reboot`() {
        // elapsedRealtime resets to zero on boot, so a candidate that appears to
        // predate the last entry is a new boot session, not a repeat. Suppressing
        // there would drop the first entry after every reboot — including the one
        // recording that the guard came back up.
        val first = entry(elapsed = 900_000, event = TamperEvent.SERVICE_CONNECTED)
        val afterBoot = first.copy(elapsedMillis = 4_000)

        assertTrue(TamperLog.shouldRecord(previous = first, candidate = afterBoot))
    }

    @Test
    fun `the first entry is always recorded`() {
        assertTrue(TamperLog.shouldRecord(previous = null, candidate = entry()))
    }

    @Test
    fun `state transitions are never coalesced`() {
        // Only the noisy events are eligible. An arm/disarm pair inside the
        // window is two real transitions, and the pair is the whole record of
        // what the user did.
        val first = entry(elapsed = 10_000, event = TamperEvent.DISARM_REQUESTED)
        val again = first.copy(elapsedMillis = 10_100)

        assertTrue(TamperLog.shouldRecord(previous = first, candidate = again))
    }

    // ---- trimming -----------------------------------------------------------

    @Test
    fun `trimming keeps the newest entries`() {
        val lines = (1..10).map { TamperLog.format(entry(elapsed = it * 1_000L)) }

        val result = TamperLog.trim(lines, maxEntries = 4)

        assertEquals(4, result.kept.size)
        assertEquals(6, result.dropped)
        assertEquals(
            listOf(7_000L, 8_000L, 9_000L, 10_000L),
            result.kept.mapNotNull { TamperLog.parse(it)?.elapsedMillis },
        )
    }

    @Test
    fun `a log under the cap is left alone`() {
        val lines = (1..3).map { TamperLog.format(entry(elapsed = it * 1_000L)) }

        val result = TamperLog.trim(lines, maxEntries = 4)

        assertEquals(lines, result.kept)
        assertEquals(0, result.dropped)
        assertFalse(result.trimmed)
    }

    @Test
    fun `the cap is bounded so the log cannot grow without limit`() {
        // This file is written from the accessibility-event path on a device the
        // user did not choose to give storage to. A cap that is not enforced is
        // the same bug as no cap.
        assertTrue(TamperLog.MAX_ENTRIES in 100..5_000)
    }

    // ---- session classification --------------------------------------------

    @Test
    fun `a first run is recognised`() {
        assertEquals(SessionStart.FIRST_RUN, TamperLog.classifyConnect(emptyList(), nowElapsed = 5_000))
    }

    @Test
    fun `a log with no session of its own is a first run`() {
        // The status service and the mode both write before the accessibility
        // service has ever been enabled. Entries exist, but no session does.
        val recent = listOf(
            entry(elapsed = 1_000, event = TamperEvent.PROTECTION_ARMED),
            entry(elapsed = 2_000, event = TamperEvent.ADMIN_ENABLED),
        )

        assertEquals(SessionStart.FIRST_RUN, TamperLog.classifyConnect(recent, nowElapsed = 5_000))
    }

    @Test
    fun `a clean stop and restart is unremarkable`() {
        // Turning the service off in Settings unbinds it, which is the supported
        // way to stop the guard and must not be recorded as tampering.
        val recent = listOf(
            entry(elapsed = 10_000, event = TamperEvent.SERVICE_CONNECTED),
            entry(elapsed = 60_000, event = TamperEvent.SERVICE_DISCONNECTED),
        )

        assertEquals(SessionStart.CLEAN_RESTART, TamperLog.classifyConnect(recent, nowElapsed = 70_000))
    }

    @Test
    fun `a stop with no unbind is an unclean stop`() {
        // The signal this whole classification exists for: force-stop, a process
        // kill, or a recents swipe on an OEM that kills the service. No unbind
        // arrives, so the session's last entry is whatever it happened to be
        // doing.
        val recent = listOf(
            entry(elapsed = 10_000, event = TamperEvent.SERVICE_CONNECTED),
            entry(elapsed = 60_000, event = TamperEvent.REMOVAL_SCREEN_BLOCKED, detail = "APP_INFO"),
        )

        assertEquals(SessionStart.UNCLEAN_STOP, TamperLog.classifyConnect(recent, nowElapsed = 70_000))
    }

    @Test
    fun `entries written after a clean unbind do not make it look unclean`() {
        // The regression that made this take a list. The status service outlives
        // the accessibility service by design — noticing that the guard stopped
        // is its job — so it keeps writing after the unbind. Reading only the
        // last line would classify every clean off-and-on as a kill, and the one
        // observation that catches a real kill would be worth nothing.
        val recent = listOf(
            entry(elapsed = 10_000, event = TamperEvent.SERVICE_CONNECTED),
            entry(elapsed = 60_000, event = TamperEvent.SERVICE_DISCONNECTED),
            entry(elapsed = 61_000, event = TamperEvent.GUARD_UNPROTECTED),
        )

        assertEquals(SessionStart.CLEAN_RESTART, TamperLog.classifyConnect(recent, nowElapsed = 70_000))
    }

    @Test
    fun `connecting after a reboot is not an unclean stop`() {
        // The clock reset is the evidence. Shutting the phone down never delivers
        // an unbind either, so without this every reboot would be logged as a
        // removal attempt and the signal would be worthless.
        val recent = listOf(
            entry(elapsed = 500_000, event = TamperEvent.SERVICE_CONNECTED),
            entry(elapsed = 900_000, event = TamperEvent.REMOVAL_SCREEN_BLOCKED),
        )

        assertEquals(SessionStart.AFTER_REBOOT, TamperLog.classifyConnect(recent, nowElapsed = 8_000))
    }

    @Test
    fun `a reboot is still seen through entries written before the service connects`() {
        // The boot receiver starts the status service, which checks and can write
        // before the accessibility service is bound. Those entries carry a small
        // elapsed value, so the newest line no longer post-dates the connect —
        // and comparing against it alone would hide every reboot behind them.
        val recent = listOf(
            entry(elapsed = 900_000, event = TamperEvent.SERVICE_CONNECTED),
            entry(elapsed = 4_000, event = TamperEvent.GUARD_UNPROTECTED),
        )

        assertEquals(SessionStart.AFTER_REBOOT, TamperLog.classifyConnect(recent, nowElapsed = 8_000))
    }

    @Test
    fun `an earlier boot in the tail is not read as a fresh reboot`() {
        // The tail spans more than one boot once the log has any history. Only a
        // clock reset *after* the last session ended is a reboot; an older one is
        // already accounted for.
        val recent = listOf(
            entry(elapsed = 900_000, event = TamperEvent.SERVICE_CONNECTED),
            entry(elapsed = 5_000, event = TamperEvent.SERVICE_CONNECTED),
            entry(elapsed = 60_000, event = TamperEvent.SERVICE_DISCONNECTED),
        )

        assertEquals(SessionStart.CLEAN_RESTART, TamperLog.classifyConnect(recent, nowElapsed = 70_000))
    }

    @Test
    fun `only the clock identifies a reboot`() {
        // Regression, measured on an android-36 emulator. A boot receiver was
        // recording a BOOT entry and this classification treated it as proof of
        // a restart — but ACTION_BOOT_COMPLETED is delivered every time the app
        // leaves the force-stopped state, on a device minutes into its uptime.
        // A force-stop wrote a boot marker and then read itself back as benign,
        // which is the failure this whole classification exists to prevent. No
        // entry the app writes may stand in for the clock going backwards.
        val forceStopped = listOf(entry(elapsed = 60_000, event = TamperEvent.SERVICE_CONNECTED))

        assertEquals(
            SessionStart.UNCLEAN_STOP,
            TamperLog.classifyConnect(forceStopped, nowElapsed = 70_000),
        )
    }

    @Test
    fun `the classification tail covers more than a single session`() {
        // A tail that could hold only one session would make the reboot check
        // blind exactly when the log is busiest.
        assertTrue(TamperLog.CLASSIFY_TAIL >= 16)
    }

    @Test
    fun `only an unclean stop is worth an entry of its own`() {
        assertNull(SessionStart.FIRST_RUN.notableEvent)
        assertNull(SessionStart.CLEAN_RESTART.notableEvent)
        assertNull(SessionStart.AFTER_REBOOT.notableEvent)
        assertEquals(TamperEvent.GUARD_STOPPED_UNCLEANLY, SessionStart.UNCLEAN_STOP.notableEvent)
    }

    // ---- what the log may contain ------------------------------------------

    @Test
    fun `no event describes scanned content`() {
        // The hard rule for this file: it records what happened to the guard,
        // never what was on the screen. A cover event carrying its trigger text
        // would turn an on-device accountability record into a log of everything
        // the user looked at.
        val codes = TamperEvent.entries.map { it.code }

        for (banned in listOf("text", "score", "verdict", "content", "url")) {
            assertFalse("$banned has no place in this log", codes.any { it.contains(banned) })
        }
    }
}
