package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Window selection for the guard — backlog item 2.
 *
 * The bug being pinned down here is narrow: picking the *first* window whose
 * root matches the package is fine when there is one, and wrong the moment
 * split screen puts two settings-adjacent windows on the display at once. The
 * guard then evaluates a window the user is not looking at, the self-mention
 * catch-all does not fire, and the screen sits unguarded.
 */
class WindowResolverTest {
    private val settings = "com.android.settings"
    private val other = "com.example.reader"

    private fun window(
        id: Int,
        packageName: String? = settings,
        active: Boolean = false,
        focused: Boolean = false,
    ) = WindowCandidate(id = id, packageName = packageName, isActive = active, isFocused = focused)

    @Test
    fun `returns null when no window matches the package`() {
        val windows = listOf(window(1, packageName = other, active = true, focused = true))

        assertNull(WindowResolver.choose(windows, settings, eventWindowId = null))
    }

    @Test
    fun `ignores windows whose root package is unknown`() {
        // A window with no readable root is not evidence of anything; matching it
        // by position would be matching on nothing at all.
        val windows = listOf(window(1, packageName = null, active = true, focused = true))

        assertNull(WindowResolver.choose(windows, settings, eventWindowId = null))
    }

    @Test
    fun `single match is chosen regardless of flags`() {
        // The ordinary case, and the one the device-admin list produced:
        // windows=1, neither active nor focused as far as the caller knows.
        val windows = listOf(window(7))

        assertEquals(7, WindowResolver.choose(windows, settings, eventWindowId = null)?.id)
    }

    @Test
    fun `prefers the window the event came from`() {
        // The event names its own window, which is better evidence than focus:
        // it is the window that actually changed.
        val windows = listOf(
            window(1, active = true, focused = true),
            window(2),
        )

        assertEquals(2, WindowResolver.choose(windows, settings, eventWindowId = 2)?.id)
    }

    @Test
    fun `falls back to focus when the event window is gone`() {
        // Events outlive the windows they describe. A stale id must not lose the
        // match altogether -- that would be the old bug with extra steps.
        val windows = listOf(
            window(1),
            window(2, active = true, focused = true),
        )

        assertEquals(2, WindowResolver.choose(windows, settings, eventWindowId = 99)?.id)
    }

    @Test
    fun `prefers focused over active on the event-less path`() {
        // evaluateCurrentScreen is deliberately event-less, so there is no
        // windowId to match and it needs its own criterion. Focus wins because
        // GLOBAL_ACTION_BACK follows focus.
        val windows = listOf(
            window(1, active = true),
            window(2, focused = true),
        )

        assertEquals(2, WindowResolver.choose(windows, settings, eventWindowId = null)?.id)
    }

    @Test
    fun `prefers active when nothing is focused`() {
        val windows = listOf(
            window(1),
            window(2, active = true),
        )

        assertEquals(2, WindowResolver.choose(windows, settings, eventWindowId = null)?.id)
    }

    @Test
    fun `split screen does not let an unfocused pane win by position`() {
        // The actual reported shape: two settings windows, the one the user is
        // driving is second in the list.
        val windows = listOf(
            window(10),
            window(11, active = true, focused = true),
        )

        assertEquals(11, WindowResolver.choose(windows, settings, eventWindowId = null)?.id)
    }
}
