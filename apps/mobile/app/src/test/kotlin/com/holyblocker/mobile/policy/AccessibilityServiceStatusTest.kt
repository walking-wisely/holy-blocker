package com.holyblocker.mobile.policy

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AccessibilityServiceStatusTest {
    private val pkg = "com.holyblocker.mobile"
    private val cls = "com.holyblocker.mobile.ScreenGuardService"

    private fun enabled(setting: String?) =
        AccessibilityServiceStatus.isEnabled(setting, pkg, cls)

    @Test
    fun `detects a fully qualified entry`() {
        assertTrue(enabled("$pkg/$cls"))
    }

    @Test
    fun `detects a relative entry`() {
        assertTrue(enabled("$pkg/.ScreenGuardService"))
    }

    @Test
    fun `detects our entry among other services`() {
        assertTrue(enabled("com.other/com.other.Svc:$pkg/$cls:com.third/.Svc"))
    }

    @Test
    fun `tolerates whitespace and trailing separators`() {
        assertTrue(enabled(" $pkg/$cls : "))
    }

    @Test
    fun `null and blank settings mean disabled`() {
        assertFalse(enabled(null))
        assertFalse(enabled(""))
        assertFalse(enabled("   "))
    }

    @Test
    fun `another app's service does not count`() {
        assertFalse(enabled("com.other/com.other.ScreenGuardService"))
    }

    @Test
    fun `a different service in our package does not count`() {
        assertFalse(enabled("$pkg/.SomeOtherService"))
    }

    @Test
    fun `a package whose name merely contains ours does not count`() {
        // Guards the naive `contains` implementation this replaced.
        assertFalse(enabled("com.holyblocker.mobile.evil/com.holyblocker.mobile.evil.Svc"))
    }
}
