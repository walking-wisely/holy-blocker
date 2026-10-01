# 1. `PrivilegedCommand` — the process-execution edge

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/PrivilegedCommand.swift
```

Both Layer 1 modules work by invoking Apple command-line tools. To keep them testable without
mutating the real keychain or network configuration, execution sits behind one protocol:

```swift
struct CommandResult {
    let exitCode: Int32
    let standardOutput: String
    let standardError: String
}

protocol CommandRunner {
    func run(_ executable: String, _ arguments: [String]) throws -> CommandResult
}
```

Responsibilities:

- `SystemCommandRunner` — the real implementation, wrapping `Foundation.Process`. Uses absolute
  executable paths (never `PATH` lookup) so behaviour cannot be hijacked by the protected user's
  environment.
- `FakeCommandRunner` — test double. Records the exact argument vectors it was handed and returns
  scripted `CommandResult` values.

Every module below is constructed with a `CommandRunner` and never touches `Process` directly. This
is what makes "did we build the right argv?" and "did we parse the output correctly?" unit-testable
with no side effects, which is the bulk of the logic in this layer.
