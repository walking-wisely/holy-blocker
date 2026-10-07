import Foundation

/// One line appended per event. Every failure is swallowed: a log that cannot be written must not
/// take the guard down with it.
public final class TamperLogStore: @unchecked Sendable {
    public static let fileName = "tamper-log.tsv"

    private let directory: URL
    private let file: URL
    private let maxEntries: Int
    private let trimSlack: Int
    private let lock = NSLock()
    private var loaded = false
    private var lineCount = 0

    public init(directory: URL, maxEntries: Int = TamperLog.maxEntries, trimSlack: Int = 200) {
        self.directory = directory
        self.file = directory.appendingPathComponent(Self.fileName)
        self.maxEntries = maxEntries
        self.trimSlack = trimSlack
    }

    @discardableResult
    public func record(
        _ code: TamperCode, detail: String = "",
        wallMillis: Int64 = Int64(Date().timeIntervalSince1970 * 1_000),
        uptimeMillis: Int64 = Int64(ProcessInfo.processInfo.systemUptime * 1_000)
    ) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        loadLocked()
        let entry = TamperEntry(
            wallMillis: wallMillis, uptimeMillis: uptimeMillis, code: code, detail: detail)
        return appendLocked(entry)
    }

    public func recentEntries(count: Int = TamperLog.classifyTail) -> [TamperEntry] {
        lock.lock()
        defer { lock.unlock() }
        return TamperLog.readAll(Array(readLinesLocked().suffix(count)))
    }

    public func entries() -> [TamperEntry] {
        lock.lock()
        defer { lock.unlock() }
        return TamperLog.readAll(readLinesLocked())
    }

    private func appendLocked(_ entry: TamperEntry) -> Bool {
        let line = Data((TamperLog.format(entry) + "\n").utf8)
        do {
            try FileManager.default.createDirectory(
                at: directory, withIntermediateDirectories: true)
            if !FileManager.default.fileExists(atPath: file.path) {
                guard
                    FileManager.default.createFile(
                        atPath: file.path, contents: nil, attributes: [.posixPermissions: 0o600])
                else { return false }
            }
            let handle = try FileHandle(forWritingTo: file)
            defer { try? handle.close() }
            try handle.seekToEnd()
            try handle.write(contentsOf: line)
        } catch {
            return false
        }
        lineCount += 1
        if lineCount >= maxEntries + trimSlack { trimLocked() }
        return true
    }

    /// Replaces the file by rename rather than truncating it: a kill partway through a rewrite
    /// must not leave the log empty.
    private func trimLocked() {
        let result = TamperLog.trim(readLinesLocked(), maxEntries: maxEntries)
        guard result.dropped > 0 else { return }
        let temporary = directory.appendingPathComponent(Self.fileName + ".tmp")
        do {
            try Data((result.kept.joined(separator: "\n") + "\n").utf8)
                .write(to: temporary, options: .atomic)
            try FileManager.default.setAttributes(
                [.posixPermissions: 0o600], ofItemAtPath: temporary.path)
            _ = try FileManager.default.replaceItemAt(file, withItemAt: temporary)
        } catch {
            try? FileManager.default.removeItem(at: temporary)
            return
        }
        lineCount = result.kept.count
        _ = appendLockedWithoutTrim(
            TamperEntry(
                wallMillis: Int64(Date().timeIntervalSince1970 * 1_000),
                uptimeMillis: Int64(ProcessInfo.processInfo.systemUptime * 1_000),
                code: .logTrimmed, detail: "dropped=\(result.dropped)"))
    }

    private func appendLockedWithoutTrim(_ entry: TamperEntry) -> Bool {
        guard let handle = try? FileHandle(forWritingTo: file) else { return false }
        defer { try? handle.close() }
        do {
            try handle.seekToEnd()
            try handle.write(contentsOf: Data((TamperLog.format(entry) + "\n").utf8))
        } catch {
            return false
        }
        lineCount += 1
        return true
    }

    private func loadLocked() {
        guard !loaded else { return }
        loaded = true
        lineCount = readLinesLocked().count
    }

    private func readLinesLocked() -> [String] {
        guard let data = try? Data(contentsOf: file) else { return [] }
        return String(decoding: data, as: UTF8.self)
            .split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
    }
}
