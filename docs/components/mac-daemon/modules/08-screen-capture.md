# 8. `ScreenCapture` — `ScreenCaptureKit`

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/ScreenCapture.swift
```

**Built, except the live PNG-by-hand check — see the notes at the end of this section.**

Produces the same logical frame the Windows `capture` module produces, so the downstream scan
interface is shared:

```swift
struct CapturedFrame {
    let pixels: [UInt8]   // BGRA, row-major, tightly packed (no row padding)
    let width:  Int
    let height: Int
    let captured: Date
}
```

Responsibilities:

- Enumerate with `SCShareableContent`, build an `SCContentFilter`, run an `SCStream` with an
  `SCStreamOutput` delegate on a dedicated dispatch queue.
- Filter to the **frontmost window's application** rather than the whole display where possible, so
  the daemon's own overlay is not captured and re-scanned — a feedback loop that would otherwise
  re-trigger on the blurred backdrop it just drew. `SCContentFilter(display:excludingApplications:
  exceptingWindows:)` is the mechanism; excluding our own bundle identifier is not optional.
- Return an empty frame (zero width/height) when capture is unavailable, and let callers check —
  the same contract as the Windows module. Fail open.

Four traps, all of which produce plausible-looking wrong output rather than an error:

1. **`CVPixelBuffer` rows are padded.** `bytesPerRow` is aligned (commonly to 64 bytes) and is
   `≥ width * 4`. Copying `bytesPerRow * height` bytes and calling it a `width * height` image
   yields a progressively sheared frame that still decodes as an image and still scores as
   *something* on the classifier. Copy row by row using `CVPixelBufferGetBytesPerRow`, and lock with
   `CVPixelBufferLockBaseAddress(_:.readOnly)` first.
2. **The stream is change-driven, so a static image starves it.** `SCStreamFrameInfo.status` is one
   of `.complete`, `.idle`, `.blank`, `.suspended`; only `.complete` carries new pixels, and a
   motionless screen emits `.idle` frames with a nil surface. A still image is precisely the content
   this project most needs to catch, so the module must **retain the last `.complete` frame** and
   serve it to the scheduler on demand rather than requiring a fresh delivery per scan tick.
3. **`SCStreamConfiguration` defaults to point dimensions, not pixels.** On a Retina display that is
   half the real resolution, and the capture is a blurry downscale. Set `width`/`height` from the
   display's pixel dimensions and set `scalesToFit` deliberately. This interacts with the image
   model: `packages/image-sandbox` resizes the *shorter side* to 255 then centre-crops 224, so a
   wrong aspect ratio here silently changes what the model sees — the same class of error the
   `parity.rs` fixture caught on the Rust side.
4. **DRM-protected content captures as black.** Netflix, Apple TV+ and any HDCP-protected surface
   yield black frames by design. This is not a bug to fix; it is a coverage limit to record (below)
   and, more usefully, a detectable one — an all-black frame from a window that is playing video is
   a signal in itself.

`CGWindowListCreateImage` is obsoleted in macOS 15 and must not be used, including as a fallback.

## What was built, and what is still outstanding

`ScreenCapture.swift` ships `CapturedFrame` as specified, plus `PixelBufferCopy.depad` (the
row-depadding fix for trap 1), `FrameDeliveryStatus` + `FrameCache` (the retention fix for trap 2,
kept independent of `ScreenCaptureKit` types so it is testable without linking the framework), and
`FrameAnalysis.isAllBlack` (the DRM-black-as-signal note, turned into a callable check rather than
left as a comment). `ScreenCapturing` is the edge protocol, mirroring `CommandRunner` and
`PermissionProbing`; `FakeScreenCapture` is its test double, and `SCShareableContentCapture` is the
real implementation — `SCShareableContent` → `SCContentFilter` (excluding our own bundle
identifier) → pixel-dimensioned `SCStreamConfiguration` → `SCStream`, with frames pushed through
`FrameCache` from the `SCStreamOutput` delegate callback. 20 tests, all on the pure logic (depad,
retention, black-detection); the edge is exercised only through `FakeScreenCapture` in tests, per
the `CommandRunner` pattern.

A `capture` CLI verb was added alongside `permissions` for live verification. Run from a shell
today it reaches the real `SCStream` setup and fails at the expected point — the process has no
Screen Recording grant — with `SCStreamErrorDomain Code=-3801`, confirming the request actually
reaches TCC rather than silently returning nothing. This is the same responsible-process gap
`PermissionGate` recorded: a real capture, and the PNG-by-hand check the plan calls for, are
**blocked on module 0's outstanding item** (a stable signing identity) and a human granting Screen
Recording to it, not on anything in this module.

## Reference documents

- [Apple — ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit) — `SCStream`, `SCShareableContent`, `SCContentFilter`, and the `SCStreamOutput` delegate contract.
- [Apple — `SCStreamFrameInfo.status`](https://developer.apple.com/documentation/screencapturekit/scstreamframeinfo) — the `.complete`/`.idle`/`.blank`/`.suspended` attachment that governs trap 2.
- [Apple — `CVPixelBuffer`](https://developer.apple.com/documentation/corevideo/cvpixelbuffer) — `CVPixelBufferGetBytesPerRow`, base-address locking, and the padding rule behind trap 1.
- [Apple — `CGWindowListCreateImage` deprecation](https://developer.apple.com/documentation/coregraphics/1455730-cgwindowlistcreateimage) — confirms the obsoletion this plan routes around.

---
