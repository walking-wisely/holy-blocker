import Foundation
import Testing

@testable import MacDaemon

private func decodePlist(_ data: Data) throws -> [String: Any] {
    let object = try PropertyListSerialization.propertyList(from: data, format: nil)
    return object as? [String: Any] ?? [:]
}

@Suite("LaunchdJob.environment")
struct LaunchdJobEnvironmentTests {
    @Test("omits EnvironmentVariables entirely when none are given")
    func noEnvironmentKeyByDefault() throws {
        let job = LaunchdJob.agent(
            label: "com.holyblocker.agent",
            executable: URL(fileURLWithPath: "/Applications/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd"),
            arguments: ["agent"], home: URL(fileURLWithPath: "/Users/test"), uid: 501)
        let plist = try decodePlist(try job.plist())
        #expect(plist["EnvironmentVariables"] == nil)
    }

    @Test("carries explicit environment variables into the plist")
    func environmentVariablesRoundTrip() throws {
        let job = LaunchdJob.agent(
            label: "com.holyblocker.agent",
            executable: URL(fileURLWithPath: "/Applications/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd"),
            arguments: ["agent"], home: URL(fileURLWithPath: "/Users/test"), uid: 501,
            environment: ["HOLY_BLOCKER_IMAGE_THRESHOLD": "0.5"])
        let plist = try decodePlist(try job.plist())
        let environment = plist["EnvironmentVariables"] as? [String: String]
        #expect(environment == ["HOLY_BLOCKER_IMAGE_THRESHOLD": "0.5"])
    }

    @Test("a daemon job also accepts environment variables")
    func daemonEnvironment() throws {
        let job = LaunchdJob.daemon(
            label: "com.holyblocker.daemon",
            executable: URL(fileURLWithPath: "/Applications/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd"),
            arguments: ["run"], environment: ["HOLY_BLOCKER_PROXY_PORT": "8080"])
        let plist = try decodePlist(try job.plist())
        let environment = plist["EnvironmentVariables"] as? [String: String]
        #expect(environment == ["HOLY_BLOCKER_PROXY_PORT": "8080"])
    }
}
