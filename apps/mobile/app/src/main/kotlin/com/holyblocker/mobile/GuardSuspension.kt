package com.holyblocker.mobile

import android.content.Context
import android.os.SystemClock
import com.holyblocker.mobile.policy.ReleasePhase
import com.holyblocker.mobile.policy.ReleaseSchedule
import com.holyblocker.mobile.policy.ReleaseState

/**
 * The exit path: a delayed, time-limited release of
 * [com.holyblocker.mobile.policy.SettingsGuard].
 *
 * `MainActivity` requests it and `ScreenGuardService` reads it. They are separate
 * objects in the same process, so this goes through `SharedPreferences` — which
 * also means a pending request survives the service being restarted.
 *
 * Why an exit path exists at all: with the guard running there is no other way to
 * reach the accessibility toggle, so without a deliberate release a matcher that
 * is wrong on an untested device would leave the user locked out of their own
 * settings with `adb` or safe mode as the only recovery. A tool that cannot be
 * removed is not an accountability tool.
 *
 * Why it is delayed rather than immediate: see [ReleaseSchedule]. This class is
 * only storage; the decision lives there and is unit tested without Android.
 *
 * **What is stored is a request, not a grant.** Clearing app data therefore
 * removes a pending request and makes the guard stricter, never weaker — worth
 * preserving, since clear-data is reachable from a screen we can only guard, not
 * prevent.
 */
class GuardSuspension(context: Context) {

    private val prefs =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /** Starts the cooldown. Re-requesting while one is live does not restart it. */
    fun requestRelease(nowElapsed: Long = SystemClock.elapsedRealtime()) {
        if (state(nowElapsed).phase != ReleasePhase.IDLE) return
        prefs.edit().putLong(KEY_REQUESTED_AT_ELAPSED, nowElapsed).apply()
    }

    fun state(nowElapsed: Long = SystemClock.elapsedRealtime()): ReleaseState {
        val requested = prefs.getLong(KEY_REQUESTED_AT_ELAPSED, NO_REQUEST)
        return ReleaseSchedule.evaluate(
            requestedAtElapsed = requested.takeIf { it != NO_REQUEST },
            nowElapsed = nowElapsed,
        )
    }

    /**
     * Wall-clock instant the open window ends, or 0 when the guard is not
     * released.
     *
     * Converted here because [com.holyblocker.mobile.policy.SettingsGuard] tracks
     * suspension against the same clock it timestamps events with. The window is
     * short and re-derived from the monotonic schedule on every event, so a
     * wall-clock change cannot extend it by more than one event's latency.
     */
    fun releasedUntilWallMillis(
        nowWall: Long = System.currentTimeMillis(),
        nowElapsed: Long = SystemClock.elapsedRealtime(),
    ): Long {
        val state = state(nowElapsed)
        return if (state.phase == ReleasePhase.OPEN) nowWall + state.remainingMillis else 0L
    }

    /** Drops a pending or open request, e.g. once the user is done. */
    fun clear() {
        prefs.edit().remove(KEY_REQUESTED_AT_ELAPSED).apply()
    }

    companion object {
        private const val PREFS_NAME = "guard_suspension"
        private const val KEY_REQUESTED_AT_ELAPSED = "requested_at_elapsed"
        private const val NO_REQUEST = -1L
    }
}
