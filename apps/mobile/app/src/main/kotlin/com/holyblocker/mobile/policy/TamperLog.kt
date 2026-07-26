package com.holyblocker.mobile.policy

/**
 * What happened to the guard. Never what was on the screen.
 *
 * The [code] is the on-disk representation and is part of the file format:
 * renaming a constant here is free, changing its code orphans every entry
 * already written. Codes are asserted unique and content-free by
 * `TamperLogTest`.
 */
enum class TamperEvent(
    val code: String,
    /**
     * Whether a repeat of this event within [TamperLog.COALESCE_MILLIS] should
     * be dropped.
     *
     * True only for the events that fire from the accessibility-event path,
     * which arrive several times per screen while it renders. State transitions
     * are never coalesced — two of them in a second are two real things the user
     * did, and the pair is the record.
     */
    val coalescing: Boolean = false,
) {
    /** The accessibility service was bound and is running. */
    SERVICE_CONNECTED("service_on"),

    /**
     * The service was unbound cleanly — the supported way to stop the guard.
     *
     * Its *absence* is the interesting case; see [SessionStart.UNCLEAN_STOP].
     */
    SERVICE_DISCONNECTED("service_off"),

    PROTECTION_ARMED("armed"),
    DISARM_REQUESTED("disarm_requested"),
    DISARM_CONFIRMED("disarm_confirmed"),
    DISARM_CANCELLED("disarm_cancelled"),

    ADMIN_ENABLED("admin_on"),

    /** `onDisableRequested` — removal is under way and cannot be prevented. */
    ADMIN_DISABLE_REQUESTED("admin_disable_requested"),
    ADMIN_DISABLED("admin_off"),

    /** The guard backed the user out of a screen that would remove it. */
    REMOVAL_SCREEN_BLOCKED("screen_blocked", coalescing = true),

    /** The back-out bound tripped; the guard fell back to covering only. */
    GUARD_DEGRADED("guard_degraded", coalescing = true),

    /**
     * Inferred at connect time: the previous session ended without an unbind.
     *
     * Force-stop, a process kill, or a recents swipe on an OEM that kills the
     * service. See [SessionStart].
     */
    GUARD_STOPPED_UNCLEANLY("unclean_stop"),

    /** The log reached its cap and the oldest entries were dropped. */
    LOG_TRIMMED("log_trimmed"),
    ;

    companion object {
        private val byCode = entries.associateBy { it.code }

        fun fromCode(code: String): TamperEvent? = byCode[code]
    }
}

/**
 * One line of the record.
 *
 * **Both clocks are stored, and each answers a different question.**
 * [wallMillis] is the only one a human can read, and it is what the log is *for*
 * — but it is user-settable, so it cannot be reasoned with. [elapsedMillis] is
 * `SystemClock.elapsedRealtime()`, which the user cannot move, and every
 * inference in [TamperLog] is made against it. Its one discontinuity is reboot,
 * and detecting that discontinuity is itself one of the inferences.
 */
data class TamperEntry(
    val wallMillis: Long,
    val elapsedMillis: Long,
    val event: TamperEvent,
    val detail: String = "",
)

/** How a session began, judged from the last entry of the previous one. */
enum class SessionStart(
    /** What to record about the gap, if anything. */
    val notableEvent: TamperEvent? = null,
) {
    /** No prior entries. */
    FIRST_RUN,

    /** The previous session ended with an unbind. Nothing to see. */
    CLEAN_RESTART,

    /**
     * The monotonic clock went backwards, so the device restarted.
     *
     * Not notable, and deliberately so: a shutdown does not deliver an unbind
     * either, so without this every reboot would read as a removal attempt and
     * the signal would be worth nothing.
     */
    AFTER_REBOOT,

    /**
     * The previous session stopped without unbinding, on the same boot.
     *
     * This is the one thing this classification exists to find, and it is the
     * only observation available for the recents-swipe kill that cannot be
     * reproduced on the emulator — see
     * `docs/components/mobile/plan.md`, "Recents is blocked on hardware".
     */
    UNCLEAN_STOP(TamperEvent.GUARD_STOPPED_UNCLEANLY),
}

/**
 * The append-only record of what happened to the guard.
 *
 * It is the backstop for what the guard cannot prevent — guest user, safe mode,
 * `adb`, and the OEM recents kill — rather than a mechanism of its own. Nothing
 * here blocks anything; it records, so that a removal shows up afterwards
 * instead of being silent.
 *
 * **It records the guard, never the screen.** No scanned text, no verdicts, no
 * scores, no package the user merely visited. [TamperEntry.detail] carries
 * identifiers the maintainer already has — a guarded surface name, our own
 * package — and a test asserts the event vocabulary stays content-free. The
 * whole value of an on-device accountability tool is that it does not become a
 * log of what its user looked at.
 *
 * Pure and unit tested. The file edge is `TamperLogStore`.
 */
object TamperLog {

    /** Field separator. Stripped from details, so it cannot appear in one. */
    private const val SEPARATOR = '\t'

    /**
     * Cap on a detail, in characters.
     *
     * Details are short identifiers; anything longer is a bug upstream, and this
     * bounds what one such bug can cost on disk.
     */
    const val MAX_DETAIL_CHARS = 120

    /**
     * How long a repeat of a [TamperEvent.coalescing] event is treated as the
     * same visit.
     *
     * A guarded screen emits three window-state events in ~800 ms while it
     * renders (`plan.md` §7), each of which can produce a block. Long enough to
     * fold one visit into one entry, short enough that coming back to the screen
     * is counted again — the count of attempts is most of what makes this
     * readable.
     */
    const val COALESCE_MILLIS = 10_000L

    /**
     * How many entries are kept.
     *
     * This file is written from the accessibility-event path, on storage the
     * user did not choose to give up. At ~60 bytes an entry this is well under
     * 150 KB, and the oldest entries are the ones a user is least likely to
     * need.
     */
    const val MAX_ENTRIES = 2_000

    fun format(entry: TamperEntry): String = buildString {
        append(entry.wallMillis)
        append(SEPARATOR)
        append(entry.elapsedMillis)
        append(SEPARATOR)
        append(entry.event.code)
        append(SEPARATOR)
        append(sanitize(entry.detail))
    }

    /**
     * Reads one line back, or null if it cannot be read.
     *
     * **Tolerant on purpose.** A process killed mid-append leaves a partial
     * line, and being killed mid-append is precisely the tamper this log exists
     * to record — so a damaged tail must cost that one entry and nothing else.
     * Same for a code written by a newer build: unknown, therefore skipped,
     * never fatal.
     */
    fun parse(line: String): TamperEntry? {
        val fields = line.split(SEPARATOR)
        if (fields.size < 4) return null

        val wall = fields[0].toLongOrNull() ?: return null
        val elapsed = fields[1].toLongOrNull() ?: return null
        val event = TamperEvent.fromCode(fields[2]) ?: return null

        return TamperEntry(
            wallMillis = wall,
            elapsedMillis = elapsed,
            event = event,
            // Rejoined rather than taking [3]: a detail that reached the file
            // with a separator in it (from an older build, or a bug) should read
            // back oddly, not truncate the entry.
            detail = fields.drop(3).joinToString(SEPARATOR.toString()),
        )
    }

    /** Every readable entry, in file order. Unreadable lines are dropped. */
    fun readAll(lines: List<String>): List<TamperEntry> = lines.mapNotNull(::parse)

    /**
     * Whether [candidate] should be written, given the entry before it.
     *
     * See [TamperEvent.coalescing]. Note the clock check: a candidate that
     * appears to predate [previous] means the device rebooted, and suppressing
     * there would drop the first entry of every boot session.
     */
    fun shouldRecord(previous: TamperEntry?, candidate: TamperEntry): Boolean {
        if (previous == null) return true
        if (!candidate.event.coalescing) return true
        if (previous.event != candidate.event || previous.detail != candidate.detail) return true

        val since = candidate.elapsedMillis - previous.elapsedMillis
        if (since < 0) return true // reboot

        return since >= COALESCE_MILLIS
    }

    data class TrimResult(val kept: List<String>, val dropped: Int) {
        val trimmed: Boolean get() = dropped > 0
    }

    /** Drops the oldest lines once the file exceeds [maxEntries]. */
    fun trim(lines: List<String>, maxEntries: Int = MAX_ENTRIES): TrimResult {
        if (lines.size <= maxEntries) return TrimResult(lines, dropped = 0)
        return TrimResult(
            kept = lines.takeLast(maxEntries),
            dropped = lines.size - maxEntries,
        )
    }

    /**
     * What the gap before this connect means. See [SessionStart].
     *
     * @param last the final entry of the previous session, if there was one.
     * @param nowElapsed `SystemClock.elapsedRealtime()` at connect.
     */
    fun classifyConnect(last: TamperEntry?, nowElapsed: Long): SessionStart = when {
        last == null -> SessionStart.FIRST_RUN

        // **The monotonic clock is the only evidence of a reboot, and this is
        // deliberate.** An earlier version also treated a `BOOT` entry written
        // by a boot receiver as proof of one, which measured worse than useless
        // on an android-36 emulator: `ACTION_BOOT_COMPLETED` is delivered every
        // time the app leaves the force-stopped state, reproducibly, on a device
        // three minutes into its uptime. A force-stop — the removal-shaped event
        // this classification exists to catch — therefore wrote a boot marker
        // and then read itself back as an ordinary restart. Anything the system
        // can deliver outside a boot cannot be what identifies one.
        nowElapsed < last.elapsedMillis -> SessionStart.AFTER_REBOOT

        last.event == TamperEvent.SERVICE_DISCONNECTED -> SessionStart.CLEAN_RESTART
        else -> SessionStart.UNCLEAN_STOP
    }

    private fun sanitize(detail: String): String = detail
        // Separators and newlines would shift every field of the following line
        // and corrupt the file from that point on.
        .replace(Regex("[\\t\\r\\n]+"), " ")
        .trim()
        .take(MAX_DETAIL_CHARS)
}
