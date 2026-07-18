package com.holyblocker.mobile.policy

/**
 * Decides when to take a second look at a settings screen that did not match.
 *
 * `SettingsGuard` only ever runs on an accessibility event, and that is not
 * enough on its own. A screen emits its events *while it renders*, and a list
 * that populates asynchronously is still empty when they fire — so the tree the
 * guard walks has none of the rows that would identify the screen. Once the
 * screen is static no further event is delivered, and observed on an android-36
 * emulator, none is: the device-admin list sat showing this app's own row, with
 * the service bound and subscribed to `TYPE_WINDOW_CONTENT_CHANGED`, completely
 * unguarded. Even scrolling produced nothing, because a list that does not
 * scroll emits no scroll event.
 *
 * The fix is to look again shortly after the events stop. This holds the policy
 * for *when*, so the service is left with only the posting and cancelling.
 *
 * Two properties matter and both are tested:
 *
 * - **Trailing edge.** Each new event restarts the wait, so a render burst costs
 *   one re-look after it settles rather than one per event.
 * - **Bounded.** The offsets run out. An unmatched settings screen must not be
 *   re-walked forever — the settings app is exactly where a user idles, and the
 *   tree walk is the expensive part of the event path.
 *
 * Not thread-safe: driven from the accessibility callback and its handler, both
 * on the main thread.
 */
class RescanSchedule(
    private val offsetsMillis: List<Long> = DEFAULT_OFFSETS_MILLIS,
) {
    private var nextIndex = 0
    private var elapsedMillis = 0L

    /**
     * An event arrived on a watched screen and did not match.
     *
     * @return how long to wait before looking again, or null when the budget for
     *   this screen is spent.
     */
    fun onWatchedEvent(): Long? {
        reset()
        return advance()
    }

    /** A scheduled re-look ran and still did not match. */
    fun onRescanMissed(): Long? = advance()

    /** The screen matched, or the user left it. */
    fun reset() {
        nextIndex = 0
        elapsedMillis = 0
    }

    /**
     * The wait until the next look, measured from now.
     *
     * [offsetsMillis] are absolute — measured from the event that armed the
     * schedule — but the caller feeds this straight to `Handler.postDelayed`,
     * which is relative. Returning the offsets verbatim therefore stacked each
     * wait on the previous one and pushed the looks out to 0.4s, 1.4s and 3.4s.
     * The delta is what keeps the offsets meaning what they say.
     */
    private fun advance(): Long? {
        val offset = offsetsMillis.getOrNull(nextIndex) ?: return null
        nextIndex++
        // coerce guards a misordered offsets list: a negative delay would make
        // postDelayed fire immediately and burn the budget in one frame.
        val delay = (offset - elapsedMillis).coerceAtLeast(0)
        elapsedMillis = offset
        return delay
    }

    companion object {
        /**
         * Measured against the device-admin list, which is the slowest populating
         * guarded screen found so far.
         *
         * The first look is soon enough that a user cannot act on the screen
         * before it lands; the last is late enough to catch a list still
         * assembling. Beyond ~2s the screen is either matched or genuinely is
         * not one of ours.
         */
        val DEFAULT_OFFSETS_MILLIS = listOf(400L, 1_000L, 2_000L)
    }
}
