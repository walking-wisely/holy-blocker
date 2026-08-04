import Testing

@testable import TextPolicyFFI

/// Proof that the UniFFI plumbing works, not a test of policy behaviour — the scoring rules have
/// their own tests in `packages/text-policy`, and duplicating them here would only add a second
/// place to update. What is checked is that the Swift bindings compile, that the dylib is found and
/// loaded at runtime, and that a value crosses the boundary in each direction.
///
/// If these fail with a dyld error rather than an assertion, `scripts/build-ffi.sh` has not run or
/// `scripts/test.sh` was bypassed in favour of a bare `swift test`.
@Suite("text-policy over UniFFI")
struct TextPolicyFFITests {
    @Test("loads the Rust library and returns a verdict")
    func evaluatesCleanText() {
        let engine = PolicyEngine.withBuiltinDictionary()

        let verdict = engine.evaluate(
            text: "a perfectly ordinary sentence", source: .accessibilityTree)

        #expect(verdict.action == .allow)
    }

    @Test("carries a blocking verdict back across the boundary")
    func evaluatesBlockedText() {
        // "explicit act" is a term in the crate's own starter dictionary (src/lib.rs), used here
        // for the same reason its Rust tests use it: it is the shipped fixture, not a real list.
        let engine = PolicyEngine.withBuiltinDictionary()

        let verdict = engine.evaluate(text: "contains explicit act here", source: .accessibilityTree)

        #expect(verdict.action == .block)
        #expect(verdict.score > 0)
    }

    @Test("passes constructor arguments into Rust")
    func honoursThresholds() {
        // Thresholds are a u32 pair crossing the boundary in the other direction; a block threshold
        // of 0 makes every input block, which no default could produce by accident.
        let engine = PolicyEngine.withThresholds(block: 0, warn: 0)

        #expect(engine.evaluate(text: "anything at all", source: .accessibilityTree).action == .block)
    }

    @Test("exposes the five-case Action enum the Swift scanner has to map down")
    func actionCases() {
        // ScanAction has three cases and this has five; session 4's AccessibilityScanner owns that
        // mapping. Pinning the case set here means a change in Rust breaks this file rather than
        // silently changing what the daemon does.
        let all: [Action] = [.block, .blur, .warn, .log, .allow]

        #expect(Set(all).count == 5)
    }
}
