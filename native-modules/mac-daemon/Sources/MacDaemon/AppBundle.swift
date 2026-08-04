import Foundation

// MARK: - Identity

/// What the bundle calls itself. TCC holds a grant against this identity, not against a path.
public struct BundleIdentity: Equatable, Sendable {
    public let identifier: String
    public let name: String
    public let executableName: String
    public let version: String
    public let build: String
    public let minimumSystemVersion: String

    public init(
        identifier: String,
        name: String,
        executableName: String,
        version: String,
        build: String,
        minimumSystemVersion: String
    ) {
        self.identifier = identifier
        self.name = name
        self.executableName = executableName
        self.version = version
        self.build = build
        self.minimumSystemVersion = minimumSystemVersion
    }

    /// The shipping identity. Changing `identifier` orphans every existing grant, so it is a
    /// constant rather than something a caller passes in.
    public static let holyBlocker = BundleIdentity(
        identifier: "com.holyblocker.daemon",
        name: "Holy Blocker",
        executableName: "holy-blocker-macd",
        version: "0.1.0",
        build: "1",
        minimumSystemVersion: "14.0")
}

/// Where each piece of an assembled `.app` lives.
public struct BundleLayout: Equatable, Sendable {
    public let root: URL
    public let contents: URL
    public let macOS: URL
    public let resources: URL
    public let infoPlist: URL
    public let executable: URL

    /// In creation order — `Contents` must exist before anything under it.
    public var directories: [URL] { [contents, macOS, resources] }
}

// MARK: - Assembly

/// The `.app` wrapper that makes a TCC grant durable.
///
/// A SwiftPM executable target produces a bare Mach-O, and a grant made against one is keyed to an
/// ad-hoc identity derived from its `cdhash` — which changes on every rebuild. A bundle with a
/// stable signature is the only artifact TCC will hold a lasting grant against.
public enum AppBundle {
    public static func layout(root: URL, identity: BundleIdentity) -> BundleLayout {
        let contents = root.appendingPathComponent("Contents")
        let macOS = contents.appendingPathComponent("MacOS")
        return BundleLayout(
            root: root,
            contents: contents,
            macOS: macOS,
            resources: contents.appendingPathComponent("Resources"),
            infoPlist: contents.appendingPathComponent("Info.plist"),
            executable: macOS.appendingPathComponent(identity.executableName))
    }

    /// Build `root` into a complete, unsigned `.app` around `executable`.
    ///
    /// Any existing bundle at `root` is removed first rather than written over: a `_CodeSignature`
    /// directory left from the previous build makes `codesign` refuse the new signature, and a
    /// half-replaced bundle is the kind of thing that fails much later and somewhere else.
    public static func assemble(
        at root: URL,
        identity: BundleIdentity,
        executable: URL,
        fileManager: FileManager = .default
    ) throws {
        let layout = layout(root: root, identity: identity)

        if fileManager.fileExists(atPath: root.path) {
            try fileManager.removeItem(at: root)
        }
        for directory in layout.directories {
            try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        }

        try fileManager.copyItem(at: executable, to: layout.executable)
        try infoPlist(for: identity).write(to: layout.infoPlist)
    }

    /// The `Info.plist` an assembled bundle ships.
    ///
    /// **It deliberately carries no `NS*UsageDescription` keys.** The three capabilities Layer 2
    /// needs have none: the key list `tccd` reads on macOS 26.5.2 contains no
    /// `NSScreenCaptureUsageDescription`, no `NSInputMonitoringUsageDescription`, and nothing for
    /// Accessibility. Those prompts use fixed system wording and interpolate only the bundle name,
    /// which is why `CFBundleName` carries the whole burden of saying who is asking.
    public static func infoPlist(for identity: BundleIdentity) throws -> Data {
        let entries: [String: Any] = [
            "CFBundleIdentifier": identity.identifier,
            "CFBundleName": identity.name,
            "CFBundleDisplayName": identity.name,
            "CFBundleExecutable": identity.executableName,
            // The four-character type that marks a directory as an application bundle.
            "CFBundlePackageType": "APPL",
            "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundleShortVersionString": identity.version,
            "CFBundleVersion": identity.build,
            "LSMinimumSystemVersion": identity.minimumSystemVersion,
            // An agent with no Dock icon that can still open windows. Not LSBackgroundOnly, which
            // would forbid the overlay Layer 2 is built around.
            "LSUIElement": true,
        ]

        // XML rather than binary so the file is reviewable in a diff.
        return try PropertyListSerialization.data(
            fromPropertyList: entries, format: .xml, options: 0)
    }
}
