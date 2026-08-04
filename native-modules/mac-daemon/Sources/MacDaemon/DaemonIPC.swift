import Foundation

// Message shapes, framing and validation for the Unix-domain-socket channel between
// `holy-blocker-macd` and the Electron desktop app. Module 14 of the mac-daemon plan.
//
// This file is deliberately the *pure* half of the module: everything here operates on `Data`
// buffers and produces values, with no socket, file handle, or other I/O edge. Opening a real
// Unix domain socket in the user's container is explicitly out of scope here — see the plan's
// module 14 note: "Framing, parsing and message validation are pure and get tests first; the
// socket is the edge."
//
// Wire shape mirrors the Windows daemon's named-pipe protocol (docs/components/win-daemon/plan.md)
// so `daemon-ipc.ts` needs one transport adapter, not a second protocol: a flat JSON object with a
// `type` discriminator field alongside the payload fields, rather than a nested envelope —
//   outbound `{"type": "scan_event", "action", "score", "source", "ts", "regions"}`
//   inbound  `{"type": "config_update", "block_threshold", "warn_threshold", "protection_mode"}`
//
// `ts` is encoded as a Double (Unix epoch seconds) here for the simplest round trip through
// Swift's `Codable`. The Windows plan documents `ts` as an ISO-8601 string; reconciling the two
// daemons on one timestamp representation is left as a follow-up (see this module's report) since
// it is a cross-platform protocol decision, not something to default on quietly in one daemon.

// MARK: - Message types

/// The effective action attached to a `scan_event`, after `ProtectionMode` has been applied.
/// Mirrors `ScanAction` in the plan's module 9, defined locally here rather than imported: this
/// module is being built concurrently with `Scanner.swift` in another worktree and must not
/// depend on it.
public enum ScanEventAction: String, Codable, Sendable {
    case allow
    case warn
    case block
}

/// Which stage of the pipeline produced a `scan_event`'s score. Mirrors `ScanSource` (plan
/// module 9), defined locally for the same reason as `ScanEventAction`.
public enum ScanEventSource: String, Codable, Sendable {
    case ocr
    case image
    case text
}

/// `off`/`warn`/`full` as carried on `config_update`. Mirrors `ProtectionMode` (plan module 9).
public enum WireProtectionMode: String, Codable, Sendable {
    case full
    case warn
    case off
}

/// A frame-relative bounding box, each component normalized to 0.0–1.0. Mirrors
/// `NormalizedRect` (plan module 9).
public struct WireRect: Codable, Equatable, Sendable {
    public let x: Double
    public let y: Double
    public let width: Double
    public let height: Double

    public init(x: Double, y: Double, width: Double, height: Double) {
        self.x = x
        self.y = y
        self.width = width
        self.height = height
    }
}

/// One localized detection carried on `scan_event.regions`. Mirrors `ImageDetection` (plan
/// module 9).
public struct WireRegion: Codable, Equatable, Sendable {
    public let label: String
    public let confidence: Double
    public let box: WireRect

    public init(label: String, confidence: Double, box: WireRect) {
        self.label = label
        self.confidence = confidence
        self.box = box
    }
}

/// Outbound: one verdict, emitted every time the scan loop produces a non-`nil` result.
///
/// `regions` is always present, never omitted — an empty array is the "nothing localized, or
/// this verdict has no image component" case, and per the plan's module 9, an empty array on a
/// non-`allow` verdict means "cover the whole surface," not "nothing to do." That distinction is
/// carried by `action`/`score`, not by whether `regions` is present, so the field is required
/// rather than optional on the wire.
public struct ScanEvent: Equatable, Sendable {
    public let action: ScanEventAction
    public let score: Double
    public let source: ScanEventSource
    public let ts: Double
    public let regions: [WireRegion]

    public init(
        action: ScanEventAction, score: Double, source: ScanEventSource, ts: Double,
        regions: [WireRegion]
    ) {
        self.action = action
        self.score = score
        self.source = source
        self.ts = ts
        self.regions = regions
    }
}

/// Inbound: the desktop app pushing new thresholds/mode down to the daemon.
public struct ConfigUpdate: Equatable, Sendable {
    public let blockThreshold: Double
    public let warnThreshold: Double
    public let protectionMode: WireProtectionMode

    public init(blockThreshold: Double, warnThreshold: Double, protectionMode: WireProtectionMode)
    {
        self.blockThreshold = blockThreshold
        self.warnThreshold = warnThreshold
        self.protectionMode = protectionMode
    }
}

// MARK: - Wire type tags

/// The `type` discriminator every frame carries, so the decoder knows which shape to parse
/// before committing to one — `Codable` alone cannot branch on a sibling field the way a
/// `switch` can.
enum WireMessageType {
    static let scanEvent = "scan_event"
    static let configUpdate = "config_update"
}

// MARK: - Codable (flat `type` + fields, matching the Windows daemon's wire shape)

extension ScanEvent: Codable {
    private enum CodingKeys: String, CodingKey {
        case type, action, score, source, ts, regions
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        guard type == WireMessageType.scanEvent else {
            throw DecodingError.dataCorruptedError(
                forKey: .type, in: container, debugDescription: "expected \"scan_event\"")
        }
        action = try container.decode(ScanEventAction.self, forKey: .action)
        score = try container.decode(Double.self, forKey: .score)
        source = try container.decode(ScanEventSource.self, forKey: .source)
        ts = try container.decode(Double.self, forKey: .ts)
        regions = try container.decode([WireRegion].self, forKey: .regions)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(WireMessageType.scanEvent, forKey: .type)
        try container.encode(action, forKey: .action)
        try container.encode(score, forKey: .score)
        try container.encode(source, forKey: .source)
        try container.encode(ts, forKey: .ts)
        try container.encode(regions, forKey: .regions)
    }
}

extension ConfigUpdate: Codable {
    private enum CodingKeys: String, CodingKey {
        case type
        case blockThreshold = "block_threshold"
        case warnThreshold = "warn_threshold"
        case protectionMode = "protection_mode"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        guard type == WireMessageType.configUpdate else {
            throw DecodingError.dataCorruptedError(
                forKey: .type, in: container, debugDescription: "expected \"config_update\"")
        }
        blockThreshold = try container.decode(Double.self, forKey: .blockThreshold)
        warnThreshold = try container.decode(Double.self, forKey: .warnThreshold)
        protectionMode = try container.decode(WireProtectionMode.self, forKey: .protectionMode)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(WireMessageType.configUpdate, forKey: .type)
        try container.encode(blockThreshold, forKey: .blockThreshold)
        try container.encode(warnThreshold, forKey: .warnThreshold)
        try container.encode(protectionMode, forKey: .protectionMode)
    }
}

/// Reads just the `type` field, to route a decoded blob to the right full decode without
/// committing to a shape first.
private struct TypeTag: Decodable {
    let type: String
}

// MARK: - Framing

/// Length-prefixed JSON framing for a stream socket.
///
/// A Unix domain socket (like TCP) delivers a byte stream with no message boundaries of its own:
/// a single `read` can return part of a message, exactly one message, or several concatenated
/// together. JSON alone is not reliably self-delimiting on an arbitrary read boundary (a brace
/// can appear inside a string value), so each message is prefixed with its own length. This is
/// the simplest correct framing for a stream socket, and is what this module uses.
///
/// Frame layout: a 4-byte big-endian (network byte order) unsigned length, followed by exactly
/// that many bytes of UTF-8 JSON. Big-endian matches conventional wire-length encoding
/// (`htonl`/network byte order) and keeps the format readable from any other language without
/// depending on host endianness.
public enum DaemonIPCFraming {
    /// Bytes in the length prefix.
    static let lengthPrefixSize = 4

    /// Refuses to decode a length prefix claiming a body larger than this. Without a cap, a
    /// corrupted or hostile 4-byte prefix (up to ~4 GiB) would make the decoder tell its caller
    /// to buffer gigabytes of memory waiting for bytes that may never arrive.
    public static let maxMessageSize = 16 * 1024 * 1024  // 16 MiB — comfortably above a JPEG-backed warn frame.

    /// Encodes `jsonBody` as a length-prefixed frame ready to write to the socket.
    static func encodeFrame(jsonBody: Data) -> Data {
        var frame = Data(capacity: lengthPrefixSize + jsonBody.count)
        var length = UInt32(jsonBody.count).bigEndian
        withUnsafeBytes(of: &length) { frame.append(contentsOf: $0) }
        frame.append(jsonBody)
        return frame
    }

    /// The result of attempting to decode one frame from the front of a buffer.
    public enum DecodeResult: Equatable {
        /// Fewer than `lengthPrefixSize` bytes are buffered, or the prefix is complete but the
        /// declared body has not fully arrived yet. Callers should append more bytes and retry.
        case needsMoreData

        /// One complete frame was parsed. `body` is the raw JSON payload; `bytesConsumed` is how
        /// many bytes of the input buffer it occupied (prefix + body) — callers drop that many
        /// bytes from the front of their buffer before decoding again.
        case message(body: Data, bytesConsumed: Int)

        /// The length prefix declares a body larger than `maxMessageSize`. The connection should
        /// be treated as corrupted; there is no recovery within the stream because the framing
        /// itself cannot be trusted (a garbage prefix will not point at a real frame boundary).
        case oversized(declaredLength: UInt32)
    }

    /// Attempts to decode one frame from the front of `buffer` without mutating it.
    static func decodeFrame(from buffer: Data) -> DecodeResult {
        guard buffer.count >= lengthPrefixSize else { return .needsMoreData }

        let prefixStart = buffer.startIndex
        let prefixBytes = [UInt8](buffer[prefixStart..<(prefixStart + lengthPrefixSize)])
        let length =
            (UInt32(prefixBytes[0]) << 24) | (UInt32(prefixBytes[1]) << 16)
            | (UInt32(prefixBytes[2]) << 8) | UInt32(prefixBytes[3])

        guard length <= UInt32(maxMessageSize) else { return .oversized(declaredLength: length) }

        let bodyLength = Int(length)
        let totalLength = lengthPrefixSize + bodyLength
        guard buffer.count >= totalLength else { return .needsMoreData }

        let bodyStart = prefixStart + lengthPrefixSize
        let body = buffer[bodyStart..<(bodyStart + bodyLength)]
        return .message(body: Data(body), bytesConsumed: totalLength)
    }
}

// MARK: - Encode / decode

/// Errors surfaced by `DaemonIPCCodec` for a complete-but-invalid frame. Distinct from
/// `DaemonIPCFraming.DecodeResult`, which only concerns itself with byte boundaries — a frame can
/// be a perfectly well-formed length-prefixed chunk and still fail here, either because the JSON
/// itself does not parse or because it parses but violates a field's semantic range.
public enum DaemonIPCDecodeError: Error, Equatable {
    /// The frame body is not valid JSON, does not carry a recognized `type`, or does not match
    /// the shape that `type` promises.
    case malformedJSON
    /// The JSON parsed, but a field is out of its documented range or names an unrecognized enum
    /// case — e.g. a threshold outside 0.0–1.0, or a `protection_mode` string that is not one of
    /// `full`/`warn`/`off`.
    case invalidField(String)
}

/// The outcome of decoding one complete frame's JSON body.
public enum DecodedMessage: Equatable {
    case scanEvent(ScanEvent)
    case configUpdate(ConfigUpdate)
}

/// Pure JSON encode/decode plus validation, layered on top of `DaemonIPCFraming`. This is the
/// surface `daemon-ipc.ts`'s macOS transport adapter and the Swift socket edge both call into.
public enum DaemonIPCCodec {
    private static let encoder = JSONEncoder()
    private static let decoder = JSONDecoder()

    // MARK: Outbound (scan_event)

    /// Encodes a `scan_event` as a complete length-prefixed frame ready to write to the socket.
    public static func encode(_ event: ScanEvent) -> Data {
        // Encoding a value this module itself defines as Codable, with no non-finite Double
        // fields under our control, cannot fail at runtime; a crash here would indicate a
        // programmer error, not a data problem.
        let json = try! encoder.encode(event)
        return DaemonIPCFraming.encodeFrame(jsonBody: json)
    }

    // MARK: Inbound (config_update)

    /// Encodes a `config_update` as a complete length-prefixed frame. The real sender is the
    /// Electron app, not this daemon; this exists so the round trip is testable from one side.
    public static func encode(_ update: ConfigUpdate) -> Data {
        let json = try! encoder.encode(update)
        return DaemonIPCFraming.encodeFrame(jsonBody: json)
    }

    // MARK: Streaming decode

    /// The result of attempting to decode one message from the front of a growing byte buffer.
    public enum StreamDecodeResult: Equatable {
        /// Not enough bytes have arrived yet to complete a frame. Append more bytes and retry.
        case needsMoreData
        /// One full frame was parsed and validated. `bytesConsumed` bytes should be dropped from
        /// the front of the caller's buffer.
        case message(DecodedMessage, bytesConsumed: Int)
        /// The frame boundary itself could not be trusted (declared length over the cap). The
        /// caller should close the connection rather than attempt to resynchronize, since a
        /// corrupted length has no reliable way back to a real frame boundary.
        case oversized(declaredLength: UInt32)
        /// The frame boundary was fine (and is reflected in `bytesConsumed`) but the body failed
        /// to parse or failed validation.
        case invalid(DaemonIPCDecodeError, bytesConsumed: Int)
    }

    /// Attempts to decode one message from the front of `buffer`, without mutating it.
    ///
    /// This is the incremental entry point: a real socket read arrives in arbitrary-sized
    /// chunks, so callers accumulate bytes into their own buffer, call this repeatedly, and drop
    /// `bytesConsumed` bytes after each `.message`/`.invalid` result before calling again. An
    /// `.invalid` frame still has a known length and byte boundary — only its content was wrong —
    /// so the stream can resynchronize past it; `.oversized` carries no `bytesConsumed` because
    /// the boundary itself is not trustworthy there and no resync is possible.
    public static func decode(from buffer: Data) -> StreamDecodeResult {
        switch DaemonIPCFraming.decodeFrame(from: buffer) {
        case .needsMoreData:
            return .needsMoreData
        case .oversized(let declaredLength):
            return .oversized(declaredLength: declaredLength)
        case .message(let body, let bytesConsumed):
            switch decodeBody(body) {
            case .success(let message):
                return .message(message, bytesConsumed: bytesConsumed)
            case .failure(let error):
                return .invalid(error, bytesConsumed: bytesConsumed)
            }
        }
    }

    private static func decodeBody(_ body: Data) -> Result<DecodedMessage, DaemonIPCDecodeError> {
        guard let tag = try? decoder.decode(TypeTag.self, from: body) else {
            return .failure(.malformedJSON)
        }

        switch tag.type {
        case WireMessageType.scanEvent:
            guard let event = try? decoder.decode(ScanEvent.self, from: body) else {
                return .failure(.malformedJSON)
            }
            return validate(event).map(DecodedMessage.scanEvent)
        case WireMessageType.configUpdate:
            guard let update = try? decoder.decode(ConfigUpdate.self, from: body) else {
                return .failure(.malformedJSON)
            }
            return validate(update).map(DecodedMessage.configUpdate)
        default:
            return .failure(.malformedJSON)
        }
    }

    // MARK: Validation

    private static let unitRange = 0.0...1.0

    private static func validate(_ event: ScanEvent) -> Result<ScanEvent, DaemonIPCDecodeError> {
        guard unitRange.contains(event.score) else {
            return .failure(.invalidField("score"))
        }
        for region in event.regions {
            guard unitRange.contains(region.confidence) else {
                return .failure(.invalidField("regions[].confidence"))
            }
            guard unitRange.contains(region.box.x), unitRange.contains(region.box.y),
                unitRange.contains(region.box.width), unitRange.contains(region.box.height)
            else {
                return .failure(.invalidField("regions[].box"))
            }
        }
        return .success(event)
    }

    private static func validate(_ update: ConfigUpdate) -> Result<
        ConfigUpdate, DaemonIPCDecodeError
    > {
        guard unitRange.contains(update.blockThreshold) else {
            return .failure(.invalidField("block_threshold"))
        }
        guard unitRange.contains(update.warnThreshold) else {
            return .failure(.invalidField("warn_threshold"))
        }
        return .success(update)
    }
}
