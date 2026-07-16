package com.holyblocker.mobile.policy

/**
 * Domain types for the policy call.
 *
 * These deliberately mirror `text-policy`'s Rust types rather than reusing the
 * UniFFI-generated ones: the generated file pulls in JNA and loads the native
 * library on class init, which would drag the native `.so` into every JVM unit
 * test. Mapping happens in exactly one place ([NativeTextPolicy]), so the gate
 * logic below stays testable with no Android and no Rust in the test path.
 */
enum class PolicyAction {
    BLOCK,
    BLUR,
    WARN,
    LOG,
    ALLOW,
}

/** Provenance of the text; scales the score by how trustworthy the source is. */
enum class PolicySource {
    BROWSER_TITLE,
    BROWSER_URL,
    ACCESSIBILITY_TREE,
    OCR_HIGH,
    OCR_MEDIUM,
    OCR_LOW,
}

/**
 * A statement about content, never about the person reading it — see
 * docs/decisions/formation-model.md.
 */
data class PolicyVerdict(
    val action: PolicyAction,
    val score: UInt,
)

interface TextPolicy {
    fun evaluate(text: String, source: PolicySource): PolicyVerdict
}
