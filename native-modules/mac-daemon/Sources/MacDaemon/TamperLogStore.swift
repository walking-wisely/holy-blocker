import Darwin
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
        _ code: TamperCode, detail: TamperDetail = .none,
        wallMillis: Int64 = Int64(Date().timeIntervalSince1970 * 1_000),
        uptimeMillis: Int64 = Int64(ProcessInfo.processInfo.systemUptime * 1_000)
    ) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        loadLocked()
        let entry = TamperEntry(
            wallMillis: wallMillis, uptimeMillis: uptimeMillis, code: code, detail: detail.text)
        guard appendLocked(entry) else { return false }
        if lineCount >= maxEntries + trimSlack { trimLocked() }
        return true
    }

    /// From the last session boundary onward, however many entries that is: a fixed-size tail would
    /// lose the boundary in a session busy enough to matter.
    public func sessionEntries() -> [TamperEntry] {
        lock.lock()
        defer { lock.unlock() }
        let all = TamperLog.readAll(readLinesLocked())
        guard let boundary = all.lastIndex(where: { $0.code.isSessionBoundary }) else { return [] }
        return Array(all[boundary...])
    }

    public func entries() -> [TamperEntry] {
        lock.lock()
        defer { lock.unlock() }
        return TamperLog.readAll(readLinesLocked())
    }

    private func appendLocked(_ entry: TamperEntry) -> Bool {
        guard prepareDirectory() else { return false }
        let descriptor = open(
            file.path, O_RDWR | O_APPEND | O_CREAT | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard descriptor >= 0 else { return false }
        defer { close(descriptor) }
        guard isOwnRegularFile(descriptor) else { return false }

        var line = TamperLog.format(entry) + "\n"
        if !endsWithNewlineOrEmpty(descriptor) { line = "\n" + line }
        guard writeAll(descriptor, Array(line.utf8)) else { return false }
        lineCount += 1
        return true
    }

    /// Replaces the file by rename rather than truncating it: a kill partway through a rewrite
    /// must not leave the log empty. A failed trim backs off to the cap so it is not retried, with
    /// a full re-read, on every following append.
    private func trimLocked() {
        let result = TamperLog.trim(readLinesLocked(), maxEntries: maxEntries)
        guard result.dropped > 0 else {
            lineCount = result.kept.count
            return
        }
        let temporary = directory.appendingPathComponent(Self.fileName + ".tmp")
        unlink(temporary.path)
        let descriptor = open(
            temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard descriptor >= 0 else {
            lineCount = maxEntries
            return
        }
        let wrote = writeAll(descriptor, Array((result.kept.joined(separator: "\n") + "\n").utf8))
        close(descriptor)
        guard wrote, rename(temporary.path, file.path) == 0 else {
            unlink(temporary.path)
            lineCount = maxEntries
            return
        }
        lineCount = result.kept.count
        _ = appendLocked(
            TamperEntry(
                wallMillis: Int64(Date().timeIntervalSince1970 * 1_000),
                uptimeMillis: Int64(ProcessInfo.processInfo.systemUptime * 1_000),
                code: .logTrimmed, detail: TamperDetail.dropped(result.dropped).text))
    }

    private func prepareDirectory() -> Bool {
        try? FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o755])
        var status = stat()
        guard lstat(directory.path, &status) == 0,
            (status.st_mode & S_IFMT) == S_IFDIR,
            status.st_uid == geteuid(),
            status.st_mode & (S_IWGRP | S_IWOTH) == 0
        else { return false }
        return true
    }

    private func isOwnRegularFile(_ descriptor: Int32) -> Bool {
        var status = stat()
        return fstat(descriptor, &status) == 0 && (status.st_mode & S_IFMT) == S_IFREG
            && status.st_uid == geteuid()
    }

    private func endsWithNewlineOrEmpty(_ descriptor: Int32) -> Bool {
        var status = stat()
        guard fstat(descriptor, &status) == 0, status.st_size > 0 else { return true }
        var byte: UInt8 = 0
        return pread(descriptor, &byte, 1, status.st_size - 1) == 1 && byte == UInt8(ascii: "\n")
    }

    private func writeAll(_ descriptor: Int32, _ bytes: [UInt8]) -> Bool {
        var offset = 0
        while offset < bytes.count {
            let written = bytes[offset...].withUnsafeBytes {
                write(descriptor, $0.baseAddress, $0.count)
            }
            if written < 0 && errno == EINTR { continue }
            guard written > 0 else { return false }
            offset += written
        }
        return true
    }

    private func loadLocked() {
        guard !loaded else { return }
        loaded = true
        lineCount = readLinesLocked().count
    }

    private func readLinesLocked() -> [String] {
        let descriptor = open(file.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        guard descriptor >= 0 else { return [] }
        defer { close(descriptor) }
        guard isOwnRegularFile(descriptor) else { return [] }
        guard
            let data = try? FileHandle(fileDescriptor: descriptor, closeOnDealloc: false).readToEnd()
        else { return [] }
        return String(decoding: data, as: UTF8.self)
            .split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
    }
}
