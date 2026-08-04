// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "MacDaemon",
    // Testing.framework is built for macOS 14; ScreenCaptureKit's SCStream needs 13+.
    platforms: [.macOS(.v14)],
    targets: [
        .target(name: "MacDaemon"),
        .executableTarget(name: "holy-blocker-macd", dependencies: ["MacDaemon"]),
        // Do not add `unsafeFlags` to this target. SwiftPM silently stops discovering and running
        // tests when a test target carries them — `swift test` builds, prints nothing, and exits
        // 0. The Command Line Tools paths swift-testing needs are passed by scripts/test.sh.
        .testTarget(name: "MacDaemonTests", dependencies: ["MacDaemon"]),
    ]
)
