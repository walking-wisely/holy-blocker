package com.holyblocker.mobile.policy

/**
 * A window, described without Android types so selection stays testable on the JVM.
 *
 * Mirrors the fields of `AccessibilityWindowInfo` the guard actually needs.
 * [packageName] is the package of the window's *root node*, which is null when
 * no root can be read — that window is not a candidate for anything.
 *
 * Reference:
 * https://developer.android.com/reference/android/view/accessibility/AccessibilityWindowInfo
 */
data class WindowCandidate(
    val id: Int,
    val packageName: String?,
    val isActive: Boolean,
    val isFocused: Boolean,
)

/**
 * Picks which window the guard should evaluate — backlog item 2.
 *
 * The previous rule was "first window whose root matches the package", which is
 * correct exactly as long as there is only one. Split screen puts two
 * settings-adjacent windows on the display at once, and then the guard can
 * evaluate a pane the user is not driving: the self-mention catch-all does not
 * fire, and the screen sits unguarded.
 *
 * Ordering, strongest evidence first:
 *
 *  1. **The event's own window.** An event names the window that changed, which
 *     is better evidence than focus — it is a statement about this event rather
 *     than about the display as a whole.
 *  2. **Focused.** The event-less re-look path has no window id to match, so it
 *     needs its own criterion. Focus is the right one because
 *     `GLOBAL_ACTION_BACK` follows focus, so the focused window is the only one
 *     the guard can actually act on.
 *  3. **Active.** Weaker than focus but still better than list position.
 *  4. **First match**, which is where this started.
 */
object WindowResolver {

    /**
     * The window to evaluate for [packageName], or null when none matches.
     *
     * [eventWindowId] is the id from the triggering `AccessibilityEvent`, or null
     * on the deliberately event-less re-look path. A stale id — the window has
     * since gone — falls through to the focus rules rather than losing the match,
     * since events routinely outlive the windows they describe.
     */
    fun choose(
        candidates: List<WindowCandidate>,
        packageName: String,
        eventWindowId: Int?,
    ): WindowCandidate? {
        val matching = candidates.filter { it.packageName == packageName }
        if (matching.isEmpty()) return null

        eventWindowId
            ?.let { id -> matching.firstOrNull { it.id == id } }
            ?.let { return it }

        return matching.firstOrNull { it.isFocused }
            ?: matching.firstOrNull { it.isActive }
            ?: matching.first()
    }
}
