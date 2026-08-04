import Foundation

/// Test double for `CommandRunner`.
///
/// Records every invocation verbatim so tests can assert on the exact argument vector built, and
/// returns scripted results. Lives in the library rather than the test target so integration
/// harnesses can use it too.
public final class FakeCommandRunner: CommandRunner, @unchecked Sendable {
    public struct Invocation: Equatable, Sendable {
        public let executable: String
        public let arguments: [String]
    }

    private let lock = NSLock()
    private var _invocations: [Invocation] = []
    private var responses: [(match: (String, [String]) -> Bool, result: CommandResult)] = []
    private var defaultResult: CommandResult

    public init(defaultResult: CommandResult = CommandResult(exitCode: 0)) {
        self.defaultResult = defaultResult
    }

    public var invocations: [Invocation] {
        lock.lock()
        defer { lock.unlock() }
        return _invocations
    }

    /// Script a result for invocations whose argument vector contains `subcommand`.
    public func stub(subcommand: String, result: CommandResult) {
        lock.lock()
        defer { lock.unlock() }
        responses.append((match: { _, args in args.contains(subcommand) }, result: result))
    }

    /// Script a result using an arbitrary predicate over the executable and arguments.
    public func stub(
        where predicate: @escaping (String, [String]) -> Bool,
        result: CommandResult
    ) {
        lock.lock()
        defer { lock.unlock() }
        responses.append((match: predicate, result: result))
    }

    public func run(_ executable: String, _ arguments: [String]) throws -> CommandResult {
        lock.lock()
        defer { lock.unlock() }
        _invocations.append(Invocation(executable: executable, arguments: arguments))
        for response in responses where response.match(executable, arguments) {
            return response.result
        }
        return defaultResult
    }

    public func reset() {
        lock.lock()
        defer { lock.unlock() }
        _invocations.removeAll()
        responses.removeAll()
    }
}
