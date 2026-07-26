import Foundation

/// Result of running an external tool.
public struct CommandResult: Equatable, Sendable {
    public let exitCode: Int32
    public let standardOutput: String
    public let standardError: String

    public init(exitCode: Int32, standardOutput: String = "", standardError: String = "") {
        self.exitCode = exitCode
        self.standardOutput = standardOutput
        self.standardError = standardError
    }

    public var succeeded: Bool { exitCode == 0 }
}

/// The single edge through which this package touches the system.
///
/// Every module that configures the machine takes a `CommandRunner` rather than reaching for
/// `Process` directly, so argument construction and output parsing can be tested without
/// mutating the real keychain or network configuration.
public protocol CommandRunner: Sendable {
    func run(_ executable: String, _ arguments: [String]) throws -> CommandResult
}

public enum CommandRunnerError: Error, Equatable {
    /// The executable path was not absolute. Tools are always invoked by absolute path so a
    /// modified PATH cannot substitute a different binary.
    case executablePathNotAbsolute(String)
    case executableNotFound(String)
}

/// Real implementation, backed by `Foundation.Process`.
public struct SystemCommandRunner: CommandRunner {
    public init() {}

    public func run(_ executable: String, _ arguments: [String]) throws -> CommandResult {
        guard executable.hasPrefix("/") else {
            throw CommandRunnerError.executablePathNotAbsolute(executable)
        }
        guard FileManager.default.isExecutableFile(atPath: executable) else {
            throw CommandRunnerError.executableNotFound(executable)
        }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments

        let outPipe = Pipe()
        let errPipe = Pipe()
        process.standardOutput = outPipe
        process.standardError = errPipe

        try process.run()

        // Read before waiting: a tool that fills the 64 KiB pipe buffer would otherwise block
        // forever on write while we block on wait.
        let outData = outPipe.fileHandleForReading.readDataToEndOfFile()
        let errData = errPipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()

        return CommandResult(
            exitCode: process.terminationStatus,
            standardOutput: String(decoding: outData, as: UTF8.self),
            standardError: String(decoding: errData, as: UTF8.self)
        )
    }
}

/// Absolute paths to the system tools this package drives.
public enum SystemTool {
    /// `security(1)` — keychain and trust-settings manipulation.
    public static let security = "/usr/bin/security"
    /// `networksetup(8)` — network service and proxy configuration.
    public static let networksetup = "/usr/sbin/networksetup"
}
