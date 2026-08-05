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
    /// Where embedded dylibs go. Not a free choice: the executable is linked against
    /// `@rpath/libtext_policy_ffi.dylib` and its only bundle-relative rpath is
    /// `@executable_path/../Frameworks`, so dyld looks here and nowhere else.
    public let frameworks: URL
    public let infoPlist: URL
    public let executable: URL

    /// In creation order — `Contents` must exist before anything under it.
    public var directories: [URL] { [contents, macOS, resources, frameworks] }

    public func embeddedLibrary(named name: String) -> URL {
        frameworks.appendingPathComponent(name)
    }
}

// MARK: - Assembly

/// The `.app` wrapper that makes a TCC grant durable.
///
/// A SwiftPM executable target produces a bare Mach-O, and a grant made against one is keyed to an
/// ad-hoc identity derived from its `cdhash` — which changes on every rebuild. A bundle with a
/// stable signature is the only artifact TCC will hold a lasting grant against.
public enum AppBundle {
    /// The dylibs the daemon links and must therefore carry.
    ///
    /// Kept here rather than in `scripts/bundle.sh` so the name the linker resolves and the name
    /// the bundler copies cannot drift apart: a mismatch produces a bundle that builds, signs and
    /// installs cleanly and then fails at launch with a dyld error.
    public static let embeddedLibraryNames = [
        "libtext_policy_ffi.dylib", "libimage_sandbox_ffi.dylib",
    ]

    /// The classifier model, looked up under `Contents/Resources` at runtime.
    ///
    /// A single well-known name rather than a configurable path: the model is sealed by the
    /// bundle's signature, and a path the daemon reads from somewhere else would be a file anyone
    /// could swap for one that scores everything zero — the whole point of putting it inside a
    /// signed bundle. See docs/components/mac-daemon/plan.md, module 18.
    public static let classifierModelName = "baseline-v0.onnx"

    public static func layout(root: URL, identity: BundleIdentity) -> BundleLayout {
        let contents = root.appendingPathComponent("Contents")
        let macOS = contents.appendingPathComponent("MacOS")
        return BundleLayout(
            root: root,
            contents: contents,
            macOS: macOS,
            resources: contents.appendingPathComponent("Resources"),
            frameworks: contents.appendingPathComponent("Frameworks"),
            infoPlist: contents.appendingPathComponent("Info.plist"),
            executable: macOS.appendingPathComponent(identity.executableName))
    }

    /// Everything inside the bundle that carries its own signature, in the order `codesign` has to
    /// take it: innermost first, because signing the bundle seals whatever is in it at that moment.
    public static func nestedCode(
        in layout: BundleLayout, fileManager: FileManager = .default
    ) throws -> [URL] {
        guard fileManager.fileExists(atPath: layout.frameworks.path) else { return [] }
        return try fileManager
            .contentsOfDirectory(at: layout.frameworks, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "dylib" }
            .sorted { $0.path < $1.path }
    }

    /// Build `root` into a complete, unsigned `.app` around `executable`.
    ///
    /// Any existing bundle at `root` is removed first rather than written over: a `_CodeSignature`
    /// directory left from the previous build makes `codesign` refuse the new signature, and a
    /// half-replaced bundle is the kind of thing that fails much later and somewhere else.
    /// `libraries` are copied into `Contents/Frameworks`. It defaults to empty so the bundle verb
    /// still works before `scripts/build-ffi.sh` has ever run — the resulting bundle launches only
    /// as far as the first FFI call, which is a better failure than refusing to assemble.
    ///
    /// `resources` are copied into `Contents/Resources` — today, the ONNX classifier model. It
    /// defaults to empty for the same reason: `data/models/` is gitignored, so a fresh checkout has
    /// no artifact, and a daemon with no model must still assemble and run its text path. Resources
    /// are *sealed* by the signature but are not code, so unlike `libraries` they need no separate
    /// `codesign` pass — replacing one after signing invalidates the bundle, which is the point.
    public static func assemble(
        at root: URL,
        identity: BundleIdentity,
        executable: URL,
        libraries: [URL] = [],
        resources: [URL] = [],
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
        for library in libraries {
            try fileManager.copyItem(
                at: library, to: layout.embeddedLibrary(named: library.lastPathComponent))
        }
        for resource in resources {
            try fileManager.copyItem(
                at: resource, to: layout.resources.appendingPathComponent(resource.lastPathComponent)
            )
        }
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
