package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Records calls so tests can assert the gate actually suppressed evaluation. */
private class FakePolicy(
    var action: PolicyAction = PolicyAction.ALLOW,
    var score: UInt = 0u,
) : TextPolicy {
    val seen = mutableListOf<String>()

    override fun evaluate(text: String, source: PolicySource): PolicyVerdict {
        seen += text
        assertEquals(PolicySource.ACCESSIBILITY_TREE, source)
        return PolicyVerdict(action, score)
    }
}

class ScanGateTest {
    private val self = "com.holyblocker.mobile"

    private fun gate(policy: TextPolicy) = ScanGate(policy = policy, selfPackage = self)

    @Test
    fun `evaluates text from another app and reports the verdict`() {
        val policy = FakePolicy(PolicyAction.BLOCK, 90u)
        val outcome = gate(policy).onScreenText("com.other.app", listOf("explicit act"), 0)

        val scanned = outcome as GateOutcome.Scanned
        assertEquals(PolicyAction.BLOCK, scanned.verdict.action)
        assertEquals(90u, scanned.verdict.score)
        assertEquals(CoverState.COVER, scanned.cover)
        assertEquals(listOf("explicit act"), policy.seen)
    }

    @Test
    fun `never scans our own overlay`() {
        // Guards a feedback loop: our own warning UI contains policy-relevant
        // words, and scanning it would re-trigger the cover indefinitely.
        val policy = FakePolicy()
        val outcome = gate(policy).onScreenText(self, listOf("explicit act"), 0)

        assertEquals(GateOutcome.Skipped(SkipReason.SELF_PACKAGE), outcome)
        assertTrue(policy.seen.isEmpty())
    }

    @Test
    fun `shouldHarvest rejects our own package and accepts others`() {
        val g = gate(FakePolicy())
        assertFalse(g.shouldHarvest(self))
        assertTrue(g.shouldHarvest("com.other.app"))
    }

    @Test
    fun `skips when the node tree yields no text`() {
        val policy = FakePolicy()
        val outcome = gate(policy).onScreenText("com.other.app", listOf("", "   "), 0)

        assertEquals(GateOutcome.Skipped(SkipReason.NO_TEXT), outcome)
        assertTrue(policy.seen.isEmpty())
    }

    @Test
    fun `skips identical text within the same app`() {
        val policy = FakePolicy()
        val g = gate(policy)

        g.onScreenText("com.other.app", listOf("same text"), 0)
        val second = g.onScreenText("com.other.app", listOf("same text"), 10_000)

        assertEquals(GateOutcome.Skipped(SkipReason.DUPLICATE), second)
        assertEquals(1, policy.seen.size)
    }

    @Test
    fun `debounces rapid changing text within the same app`() {
        val policy = FakePolicy()
        val g = gate(policy)

        g.onScreenText("com.other.app", listOf("first"), 1_000)
        val second = g.onScreenText("com.other.app", listOf("second"), 1_100)

        assertEquals(GateOutcome.Skipped(SkipReason.DEBOUNCED), second)
        assertEquals(1, policy.seen.size)
    }

    @Test
    fun `rescans once the debounce interval has elapsed`() {
        val policy = FakePolicy()
        val g = gate(policy)

        g.onScreenText("com.other.app", listOf("first"), 1_000)
        val second = g.onScreenText("com.other.app", listOf("second"), 1_300)

        assertTrue(second is GateOutcome.Scanned)
        assertEquals(listOf("first", "second"), policy.seen)
    }

    @Test
    fun `app switch bypasses the debounce`() {
        // The whole screen just changed — the previous app's timer must not
        // leave the new app unscanned.
        val policy = FakePolicy()
        val g = gate(policy)

        g.onScreenText("com.a", listOf("first"), 1_000)
        val second = g.onScreenText("com.b", listOf("second"), 1_001)

        assertTrue(second is GateOutcome.Scanned)
        assertEquals(listOf("first", "second"), policy.seen)
    }

    @Test
    fun `app switch bypasses the duplicate check`() {
        val policy = FakePolicy()
        val g = gate(policy)

        g.onScreenText("com.a", listOf("same text"), 0)
        val second = g.onScreenText("com.b", listOf("same text"), 10_000)

        assertTrue(second is GateOutcome.Scanned)
        assertEquals(2, policy.seen.size)
    }

    @Test
    fun `returning to an app after a switch rescans identical text`() {
        // dedupe state tracks only the most recent app, so a → b → a rescans.
        val policy = FakePolicy()
        val g = gate(policy)

        g.onScreenText("com.a", listOf("text"), 0)
        g.onScreenText("com.b", listOf("other"), 1_000)
        val third = g.onScreenText("com.a", listOf("text"), 2_000)

        assertTrue(third is GateOutcome.Scanned)
        assertEquals(3, policy.seen.size)
    }

    @Test
    fun `reset clears dedupe state`() {
        val policy = FakePolicy()
        val g = gate(policy)

        g.onScreenText("com.other.app", listOf("text"), 0)
        g.reset()
        val second = g.onScreenText("com.other.app", listOf("text"), 10)

        assertTrue(second is GateOutcome.Scanned)
        assertEquals(2, policy.seen.size)
    }

    @Test
    fun `a skipped scan does not advance the debounce clock`() {
        // Only real evaluations move the timer; otherwise a stream of duplicates
        // would hold the window open forever and starve a genuine change.
        val policy = FakePolicy()
        val g = gate(policy)

        g.onScreenText("com.other.app", listOf("first"), 1_000)
        g.onScreenText("com.other.app", listOf("first"), 1_200) // duplicate
        val third = g.onScreenText("com.other.app", listOf("second"), 1_300)

        assertTrue(third is GateOutcome.Scanned)
    }

    @Test
    fun `cover state maps from every action`() {
        assertEquals(CoverState.COVER, ScanGate.coverFor(PolicyAction.BLOCK))
        assertEquals(CoverState.COVER, ScanGate.coverFor(PolicyAction.BLUR))
        assertEquals(CoverState.WARN, ScanGate.coverFor(PolicyAction.WARN))
        assertEquals(CoverState.CLEAR, ScanGate.coverFor(PolicyAction.LOG))
        assertEquals(CoverState.CLEAR, ScanGate.coverFor(PolicyAction.ALLOW))
    }

    @Test
    fun `clean screen after a blocked one clears the cover`() {
        val policy = FakePolicy(PolicyAction.BLOCK, 90u)
        val g = gate(policy)

        val blocked = g.onScreenText("com.other.app", listOf("explicit act"), 0)
        assertEquals(CoverState.COVER, (blocked as GateOutcome.Scanned).cover)

        policy.action = PolicyAction.ALLOW
        policy.score = 0u
        val clean = g.onScreenText("com.other.app", listOf("harmless"), 1_000)
        assertEquals(CoverState.CLEAR, (clean as GateOutcome.Scanned).cover)
    }
}
