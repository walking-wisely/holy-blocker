package com.holyblocker.mobile.policy

import uniffi.text_policy_ffi.Action as FfiAction
import uniffi.text_policy_ffi.PolicyEngine as FfiPolicyEngine
import uniffi.text_policy_ffi.SourceKind as FfiSourceKind

/**
 * Bridges the app's domain types to the `text-policy` Rust engine over UniFFI.
 *
 * This is the only file that touches the generated bindings, and therefore the
 * only one that requires `libtext_policy_ffi.so` to be present. Constructing
 * [FfiPolicyEngine] compiles the lexicon automaton, so hold one instance for the
 * service's lifetime rather than building one per event.
 */
class NativeTextPolicy private constructor(
    private val engine: FfiPolicyEngine,
) : TextPolicy, AutoCloseable {

    override fun evaluate(text: String, source: PolicySource): PolicyVerdict {
        val verdict = engine.evaluate(text, source.toFfi())
        return PolicyVerdict(action = verdict.action.toDomain(), score = verdict.score)
    }

    override fun close() = engine.close()

    companion object {
        fun withBuiltinDictionary(): NativeTextPolicy =
            NativeTextPolicy(FfiPolicyEngine.withBuiltinDictionary())

        fun withThresholds(block: UInt, warn: UInt): NativeTextPolicy =
            NativeTextPolicy(FfiPolicyEngine.withThresholds(block, warn))

        private fun PolicySource.toFfi(): FfiSourceKind = when (this) {
            PolicySource.BROWSER_TITLE -> FfiSourceKind.BROWSER_TITLE
            PolicySource.BROWSER_URL -> FfiSourceKind.BROWSER_URL
            PolicySource.ACCESSIBILITY_TREE -> FfiSourceKind.ACCESSIBILITY_TREE
            PolicySource.OCR_HIGH -> FfiSourceKind.OCR_HIGH
            PolicySource.OCR_MEDIUM -> FfiSourceKind.OCR_MEDIUM
            PolicySource.OCR_LOW -> FfiSourceKind.OCR_LOW
        }

        private fun FfiAction.toDomain(): PolicyAction = when (this) {
            FfiAction.BLOCK -> PolicyAction.BLOCK
            FfiAction.BLUR -> PolicyAction.BLUR
            FfiAction.WARN -> PolicyAction.WARN
            FfiAction.LOG -> PolicyAction.LOG
            FfiAction.ALLOW -> PolicyAction.ALLOW
        }
    }
}
