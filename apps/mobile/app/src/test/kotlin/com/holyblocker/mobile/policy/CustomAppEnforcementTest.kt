package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CustomAppEnforcementTest {

    private val armed = ProtectionState(ProtectionPhase.ARMED, 0)
    private val state = CustomAppState(apps = setOf("com.listed.app", "com.other.listed"))

    private fun decide(
        enforcement: CustomAppEnforcement,
        visible: List<String>,
        now: Long = 0,
        protection: ProtectionState = armed,
        protected: Set<String> = emptySet(),
        listed: CustomAppState = state,
    ) = enforcement.decide(listed, visible, protected, protection, now)

    @Test
    fun `an unlisted app is untouched`() {
        val action = decide(CustomAppEnforcement(), listOf("com.unlisted.app"))

        assertFalse(action.cover)
        assertFalse(action.sendHome)
    }

    @Test
    fun `a listed app in any visible window is covered and sent home`() {
        val action = decide(CustomAppEnforcement(), listOf("com.unlisted.app", "com.listed.app"))

        assertTrue(action.cover)
        assertTrue(action.sendHome)
    }

    @Test
    fun `no windows means nothing to do`() {
        val action = decide(CustomAppEnforcement(), emptyList())

        assertFalse(action.cover)
        assertFalse(action.sendHome)
    }

    @Test
    fun `send home is throttled while the app stays up`() {
        val enforcement = CustomAppEnforcement(minGapMillis = 1_000)

        assertTrue(decide(enforcement, listOf("com.listed.app"), now = 5_000).sendHome)
        val second = decide(enforcement, listOf("com.listed.app"), now = 5_400)
        assertTrue(second.cover)
        assertFalse(second.sendHome)
        assertTrue(decide(enforcement, listOf("com.listed.app"), now = 6_000).sendHome)
    }

    @Test
    fun `a clock that moves backwards does not suppress send home`() {
        val enforcement = CustomAppEnforcement(minGapMillis = 1_000)
        decide(enforcement, listOf("com.listed.app"), now = 9_000)

        assertTrue(decide(enforcement, listOf("com.listed.app"), now = 100).sendHome)
    }

    @Test
    fun `a protected package is never acted on even when listed`() {
        val action = decide(
            CustomAppEnforcement(),
            listOf("com.listed.app"),
            protected = setOf("com.listed.app"),
        )

        assertFalse(action.cover)
        assertFalse(action.sendHome)
    }

    @Test
    fun `a never-rated package is never acted on even when listed`() {
        val action = decide(
            CustomAppEnforcement(),
            listOf("com.android.settings"),
            listed = CustomAppState(apps = setOf("com.android.settings")),
        )

        assertFalse(action.cover)
    }

    @Test
    fun `enforcement stops when protection is disarmed or off`() {
        for (phase in listOf(ProtectionPhase.DISARMED, ProtectionPhase.OFF)) {
            val action = decide(
                CustomAppEnforcement(),
                listOf("com.listed.app"),
                protection = ProtectionState(phase, 0),
            )

            assertFalse("$phase", action.cover)
            assertFalse("$phase", action.sendHome)
        }
    }

    @Test
    fun `enforcement continues through a pending or ready disarm request`() {
        for (phase in listOf(ProtectionPhase.DISARM_PENDING, ProtectionPhase.DISARM_READY)) {
            val action = decide(
                CustomAppEnforcement(),
                listOf("com.listed.app"),
                protection = ProtectionState(phase, 0),
            )

            assertTrue("$phase", action.cover)
        }
    }

    @Test
    fun `an app with a pending removal is still enforced until it is confirmed`() {
        val pending = state.copy(removals = mapOf("com.listed.app" to 0L))

        assertEquals(true, decide(CustomAppEnforcement(), listOf("com.listed.app"), listed = pending).cover)
    }
}
