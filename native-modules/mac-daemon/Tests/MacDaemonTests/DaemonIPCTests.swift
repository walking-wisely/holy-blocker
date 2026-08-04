import Foundation
import Testing

@testable import MacDaemon

// Covers the pure framing/parsing/validation layer of module 14 (`DaemonIPC`). No socket is
// opened anywhere in this file — every test operates on `Data` buffers directly, simulating what
// a real Unix domain socket would deliver (whole frames, split frames, garbage, oversized
// prefixes) without needing one.

@Suite("ScanEvent round trip")
struct ScanEventRoundTripTests {
    @Test("encodes and decodes back to an equal value, with an empty regions array")
    func roundTripEmptyRegions() {
        let event = ScanEvent(
            action: .block, score: 0.92, source: .text, ts: 1_722_000_000, regions: [])

        let frame = DaemonIPCCodec.encode(event)
        let result = DaemonIPCCodec.decode(from: frame)

        #expect(
            result == .message(.scanEvent(event), bytesConsumed: frame.count))
    }

    @Test("round-trips populated regions")
    func roundTripWithRegions() {
        let event = ScanEvent(
            action: .warn, score: 0.6, source: .image, ts: 1_722_000_001,
            regions: [
                WireRegion(
                    label: "explicit", confidence: 0.81,
                    box: WireRect(x: 0.1, y: 0.2, width: 0.3, height: 0.4)),
                WireRegion(
                    label: "explicit", confidence: 0.5,
                    box: WireRect(x: 0.0, y: 0.0, width: 1.0, height: 1.0)),
            ])

        let frame = DaemonIPCCodec.encode(event)
        let result = DaemonIPCCodec.decode(from: frame)

        #expect(result == .message(.scanEvent(event), bytesConsumed: frame.count))
    }

    @Test("serializes an empty regions array as [] rather than omitting the field")
    func regionsFieldNeverOmitted() {
        let event = ScanEvent(action: .allow, score: 0.0, source: .ocr, ts: 0, regions: [])
        let frame = DaemonIPCCodec.encode(event)

        // Strip the 4-byte length prefix and inspect the raw JSON body directly, since the
        // "field present but empty" property is exactly what a round trip through Codable could
        // silently launder (Codable does not distinguish "empty array" from "absent" once it has
        // gone back through `ScanEvent`, so this test reads the wire bytes instead of the value).
        let body = frame.dropFirst(DaemonIPCFraming.lengthPrefixSize)
        let json = String(data: body, encoding: .utf8)!
        #expect(json.contains("\"regions\":[]"))
    }

    @Test("action, source and protection mode enums encode as their documented wire strings")
    func enumWireStrings() {
        let event = ScanEvent(action: .warn, score: 0.5, source: .ocr, ts: 0, regions: [])
        let frame = DaemonIPCCodec.encode(event)
        let body = frame.dropFirst(DaemonIPCFraming.lengthPrefixSize)
        let json = String(data: body, encoding: .utf8)!

        #expect(json.contains("\"action\":\"warn\""))
        #expect(json.contains("\"source\":\"ocr\""))
        #expect(json.contains("\"type\":\"scan_event\""))
    }
}

@Suite("ConfigUpdate round trip")
struct ConfigUpdateRoundTripTests {
    @Test("encodes and decodes back to an equal value")
    func roundTrip() {
        let update = ConfigUpdate(blockThreshold: 0.8, warnThreshold: 0.5, protectionMode: .full)

        let frame = DaemonIPCCodec.encode(update)
        let result = DaemonIPCCodec.decode(from: frame)

        #expect(result == .message(.configUpdate(update), bytesConsumed: frame.count))
    }

    @Test("serializes threshold fields under their documented snake_case keys")
    func snakeCaseKeys() {
        let update = ConfigUpdate(blockThreshold: 0.8, warnThreshold: 0.5, protectionMode: .warn)
        let frame = DaemonIPCCodec.encode(update)
        let body = frame.dropFirst(DaemonIPCFraming.lengthPrefixSize)
        let json = String(data: body, encoding: .utf8)!

        #expect(json.contains("\"block_threshold\":0.8"))
        #expect(json.contains("\"warn_threshold\":0.5"))
        #expect(json.contains("\"protection_mode\":\"warn\""))
        #expect(json.contains("\"type\":\"config_update\""))
    }
}

@Suite("partial reads and reassembly")
struct PartialReadTests {
    @Test("needsMoreData when nothing has arrived yet")
    func emptyBuffer() {
        #expect(DaemonIPCCodec.decode(from: Data()) == .needsMoreData)
    }

    @Test("needsMoreData when only part of the length prefix has arrived")
    func partialPrefix() {
        let event = ScanEvent(action: .block, score: 0.9, source: .text, ts: 1, regions: [])
        let frame = DaemonIPCCodec.encode(event)

        #expect(DaemonIPCCodec.decode(from: frame.prefix(2)) == .needsMoreData)
    }

    @Test("needsMoreData when the prefix is complete but the body is still arriving")
    func partialBody() {
        let event = ScanEvent(action: .block, score: 0.9, source: .text, ts: 1, regions: [])
        let frame = DaemonIPCCodec.encode(event)

        // Prefix (4 bytes) plus a few bytes of body, short of the full frame.
        let partial = frame.prefix(DaemonIPCFraming.lengthPrefixSize + 3)
        #expect(DaemonIPCCodec.decode(from: partial) == .needsMoreData)
    }

    @Test("reassembles one message delivered across several simulated socket reads")
    func splitAcrossReads() {
        let event = ScanEvent(
            action: .warn, score: 0.55, source: .image, ts: 42,
            regions: [
                WireRegion(
                    label: "x", confidence: 0.7,
                    box: WireRect(x: 0.1, y: 0.1, width: 0.2, height: 0.2))
            ])
        let frame = DaemonIPCCodec.encode(event)

        // Simulate a socket delivering the frame in three arbitrary-sized chunks, including a
        // split that lands inside the length prefix itself.
        let chunks = [
            frame.prefix(2),
            frame[2..<(frame.count / 2)],
            frame[(frame.count / 2)...],
        ]

        var buffer = Data()
        var decoded: DaemonIPCCodec.StreamDecodeResult = .needsMoreData
        for chunk in chunks {
            buffer.append(chunk)
            decoded = DaemonIPCCodec.decode(from: buffer)
            if case .needsMoreData = decoded { continue }
        }

        #expect(decoded == .message(.scanEvent(event), bytesConsumed: frame.count))
    }

    @Test("decodes a second message after consuming the first from a shared buffer")
    func twoMessagesInOneBuffer() {
        let first = ScanEvent(action: .allow, score: 0.1, source: .text, ts: 1, regions: [])
        let second = ScanEvent(action: .block, score: 0.95, source: .ocr, ts: 2, regions: [])

        var buffer = DaemonIPCCodec.encode(first)
        buffer.append(DaemonIPCCodec.encode(second))

        guard case .message(let firstDecoded, let firstConsumed) = DaemonIPCCodec.decode(
            from: buffer)
        else {
            Issue.record("expected the first message to decode")
            return
        }
        #expect(firstDecoded == .scanEvent(first))

        buffer.removeFirst(firstConsumed)
        #expect(
            DaemonIPCCodec.decode(from: buffer)
                == .message(.scanEvent(second), bytesConsumed: buffer.count))
    }
}

@Suite("garbage and truncated input")
struct MalformedInputTests {
    @Test("rejects non-JSON bytes as invalid rather than crashing")
    func nonJSONBody() {
        let body = "not json at all".data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.malformedJSON, bytesConsumed: frame.count))
    }

    @Test("rejects JSON with an unrecognized type tag")
    func unknownType() {
        let body = "{\"type\":\"mystery\"}".data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.malformedJSON, bytesConsumed: frame.count))
    }

    @Test("rejects JSON missing the type field entirely")
    func missingType() {
        let body = "{\"action\":\"block\"}".data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.malformedJSON, bytesConsumed: frame.count))
    }

    @Test("rejects an oversized length prefix without buffering or crashing")
    func oversizedPrefix() {
        var frame = Data()
        var length = UInt32(DaemonIPCFraming.maxMessageSize + 1).bigEndian
        withUnsafeBytes(of: &length) { frame.append(contentsOf: $0) }
        // No body follows — a real attacker would not necessarily send one either, and the
        // decoder must reject on the prefix alone rather than waiting for gigabytes of bytes
        // that may never arrive.

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .oversized(declaredLength: UInt32(DaemonIPCFraming.maxMessageSize + 1)))
    }

    @Test("a truncated frame (declared length longer than the buffer) reports needsMoreData, not a crash")
    func truncatedFrame() {
        let event = ScanEvent(action: .block, score: 0.9, source: .text, ts: 1, regions: [])
        let frame = DaemonIPCCodec.encode(event)

        // Drop the last byte: prefix says N bytes of body, only N-1 are present.
        let truncated = frame.dropLast()
        #expect(DaemonIPCCodec.decode(from: truncated) == .needsMoreData)
    }

    @Test("random noise never crashes the decoder, regardless of length")
    func randomNoise() {
        for length in [0, 1, 3, 4, 5, 10, 100] {
            let noise = Data((0..<length).map { _ in UInt8.random(in: 0...255) })
            // No assertion on the outcome beyond "returns", but calling it at all is the point:
            // a `fatalError`/trap anywhere in the decode path fails the whole test run.
            _ = DaemonIPCCodec.decode(from: noise)
        }
    }
}

@Suite("config_update validation")
struct ConfigUpdateValidationTests {
    @Test("rejects a block_threshold above 1.0")
    func blockThresholdTooHigh() {
        let body = """
            {"type":"config_update","block_threshold":1.5,"warn_threshold":0.5,"protection_mode":"full"}
            """.data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.invalidField("block_threshold"), bytesConsumed: frame.count))
    }

    @Test("rejects a negative warn_threshold")
    func warnThresholdNegative() {
        let body = """
            {"type":"config_update","block_threshold":0.8,"warn_threshold":-0.1,"protection_mode":"full"}
            """.data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.invalidField("warn_threshold"), bytesConsumed: frame.count))
    }

    @Test("rejects an unrecognized protection_mode string")
    func unknownProtectionMode() {
        let body = """
            {"type":"config_update","block_threshold":0.8,"warn_threshold":0.5,"protection_mode":"sleep"}
            """.data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.malformedJSON, bytesConsumed: frame.count))
    }

    @Test("accepts boundary threshold values 0.0 and 1.0")
    func boundaryThresholds() {
        let update = ConfigUpdate(blockThreshold: 1.0, warnThreshold: 0.0, protectionMode: .off)
        let frame = DaemonIPCCodec.encode(update)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .message(.configUpdate(update), bytesConsumed: frame.count))
    }
}

@Suite("scan_event validation")
struct ScanEventValidationTests {
    @Test("rejects a score above 1.0")
    func scoreTooHigh() {
        let body = """
            {"type":"scan_event","action":"block","score":1.2,"source":"text","ts":1,"regions":[]}
            """.data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.invalidField("score"), bytesConsumed: frame.count))
    }

    @Test("rejects a region confidence below 0.0")
    func regionConfidenceNegative() {
        let body = """
            {"type":"scan_event","action":"warn","score":0.5,"source":"image","ts":1,
             "regions":[{"label":"x","confidence":-0.2,"box":{"x":0,"y":0,"width":1,"height":1}}]}
            """.data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.invalidField("regions[].confidence"), bytesConsumed: frame.count))
    }

    @Test("rejects a region box component outside 0.0-1.0")
    func regionBoxOutOfRange() {
        let body = """
            {"type":"scan_event","action":"warn","score":0.5,"source":"image","ts":1,
             "regions":[{"label":"x","confidence":0.5,"box":{"x":0,"y":0,"width":1.4,"height":1}}]}
            """.data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.invalidField("regions[].box"), bytesConsumed: frame.count))
    }

    @Test("rejects an unrecognized action string")
    func unknownAction() {
        let body = """
            {"type":"scan_event","action":"panic","score":0.5,"source":"text","ts":1,"regions":[]}
            """.data(using: .utf8)!
        let frame = DaemonIPCFraming.encodeFrame(jsonBody: body)

        #expect(
            DaemonIPCCodec.decode(from: frame)
                == .invalid(.malformedJSON, bytesConsumed: frame.count))
    }
}
