package com.holyblocker.mobile

import android.content.Context
import android.os.SystemClock
import android.util.Log
import com.holyblocker.mobile.policy.TamperEntry
import com.holyblocker.mobile.policy.TamperEvent
import com.holyblocker.mobile.policy.TamperLog
import java.io.File
import java.io.IOException

/**
 * Storage edge for [TamperLog]: one line appended per event, in `filesDir`.
 *
 * **What this survives, stated plainly.** `filesDir` outlives the accessibility
 * service being turned off, the app being force-stopped, the process being
 * killed, a reboot, and an app update — which is the set of removals the log
 * exists to record, and the reason step 9 of the plan asks for entries that
 * survive the app being disabled. It does **not** survive uninstall or
 * clear-data. Clear-data is reachable from App Info, which this product can
 * guard but cannot prevent, so a determined user can erase the record; that is
 * the same ceiling as everything else in `plan.md` §7 and it is not worth
 * pretending otherwise. Exporting entries somewhere that survives uninstall is
 * a real improvement and is deliberately not attempted here — it means writing
 * user-readable history to shared storage, which is a decision about the
 * product rather than about this class.
 *
 * A process-wide singleton because the writers are in the same process but not
 * in the same object: the service, the activity, the boot receiver and the admin
 * receiver all record, and separate instances would interleave partial lines and
 * each hold their own stale idea of the last entry.
 *
 * Every operation swallows IO failure. A log that cannot be written is worse
 * than useless if it can also take the guard down with it.
 */
class TamperLogStore private constructor(context: Context) {

    private val file = File(context.applicationContext.filesDir, FILE_NAME)

    /** Cached so the coalescing check does not re-read the file per event. */
    private var lastEntry: TamperEntry? = null
    private var lineCount = 0
    private var loaded = false

    /**
     * Appends an event, unless [TamperLog.shouldRecord] folds it into the last
     * one. Returns whether anything was written.
     *
     * Timestamps default to now and are parameters only so tests and callers
     * that already read the clock do not read it twice.
     */
    @Synchronized
    fun record(
        event: TamperEvent,
        detail: String = "",
        wallMillis: Long = System.currentTimeMillis(),
        elapsedMillis: Long = SystemClock.elapsedRealtime(),
    ): Boolean {
        load()

        val entry = TamperEntry(
            wallMillis = wallMillis,
            elapsedMillis = elapsedMillis,
            event = event,
            detail = detail,
        )
        if (!TamperLog.shouldRecord(previous = lastEntry, candidate = entry)) return false

        return append(entry)
    }

    /** The most recent readable entry, or null on an empty or missing log. */
    @Synchronized
    fun lastEntry(): TamperEntry? {
        load()
        return lastEntry
    }

    /** Everything readable, oldest first. For the in-app history view. */
    @Synchronized
    fun entries(): List<TamperEntry> = TamperLog.readAll(readLines())

    private fun append(entry: TamperEntry): Boolean {
        try {
            file.appendText(TamperLog.format(entry) + "\n")
        } catch (e: IOException) {
            Log.w(TAG, "could not append ${entry.event.code}", e)
            return false
        }

        lastEntry = entry
        lineCount++

        // Trimming rewrites the whole file, so it is done in batches rather than
        // on every append past the cap — otherwise a device that reaches the cap
        // pays a full rewrite for every event from then on, on the UI-event path.
        if (lineCount >= TamperLog.MAX_ENTRIES + TRIM_SLACK) trim()

        return true
    }

    private fun trim() {
        val result = TamperLog.trim(readLines())
        if (!result.trimmed) return

        // Rename rather than truncate-and-write: a kill partway through a
        // rewrite would otherwise leave the log empty, and being killed is one
        // of the things it is here to record.
        val temp = File(file.parentFile, "$FILE_NAME.tmp")
        try {
            temp.writeText(result.kept.joinToString("\n", postfix = "\n"))
            if (!temp.renameTo(file)) {
                Log.w(TAG, "could not replace the log while trimming")
                temp.delete()
                return
            }
        } catch (e: IOException) {
            Log.w(TAG, "could not trim the log", e)
            temp.delete()
            return
        }

        lineCount = result.kept.size
        // Recorded in the log itself: a reader who finds the record shorter than
        // the history should be able to tell dropped entries from a log that was
        // wiped.
        append(
            TamperEntry(
                wallMillis = System.currentTimeMillis(),
                elapsedMillis = SystemClock.elapsedRealtime(),
                event = TamperEvent.LOG_TRIMMED,
                detail = "dropped=${result.dropped}",
            ),
        )
    }

    private fun load() {
        if (loaded) return
        loaded = true

        val lines = readLines()
        lineCount = lines.size
        lastEntry = lines.asReversed().firstNotNullOfOrNull(TamperLog::parse)
    }

    private fun readLines(): List<String> = try {
        if (file.exists()) file.readLines().filter { it.isNotBlank() } else emptyList()
    } catch (e: IOException) {
        Log.w(TAG, "could not read the log", e)
        emptyList()
    }

    companion object {
        private const val TAG = "TamperLog"
        private const val FILE_NAME = "tamper-log.tsv"

        /** How far past the cap the log runs before a rewrite. See [append]. */
        private const val TRIM_SLACK = 200

        @Volatile
        private var instance: TamperLogStore? = null

        fun of(context: Context): TamperLogStore =
            instance ?: synchronized(this) {
                instance ?: TamperLogStore(context).also { instance = it }
            }
    }
}
