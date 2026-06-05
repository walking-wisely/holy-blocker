# Video Watchdog — Implementation Plan

The design rationale and pipeline context live in [network-pipeline.md](../network-pipeline.md) (Phase 5).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

## Current state

The package `packages/video-watchdog/` does not exist yet. Nothing has been built for this area. This plan describes what to create from scratch as a Rust library crate.

What is missing is the entire Phase 5 pipeline: tapping a live video byte stream without stalling it, identifying HLS and DASH segment boundaries, extracting a representative frame from each sampled segment, and asynchronously classifying that frame using `ImageSandbox` from `packages/image-sandbox/`.

## Modules to add

### 1. `tee` — stream tee

```
src/tee.rs
```

Intercepts bytes flowing from the proxy to the browser and silently copies them to a background channel, without adding any latency to the forward path.

Responsibilities:

- Wrap an outbound `tokio::io::AsyncWrite` and a `tokio::sync::mpsc::Sender<Bytes>`.
- Each `poll_write` call writes to the underlying writer and, if the channel has capacity, also sends a clone of the bytes to the channel.
- If the channel is full, drop the copy rather than blocking. The hot path — forwarding to the browser — must never stall waiting for the background sampler.

Key types and signatures:

```rust
use bytes::Bytes;
use tokio::io::AsyncWrite;
use tokio::sync::mpsc;

pub struct StreamTee<W: AsyncWrite + Unpin> {
    inner:  W,
    sender: mpsc::Sender<Bytes>,
}

impl<W: AsyncWrite + Unpin> StreamTee<W> {
    pub fn new(inner: W, sender: mpsc::Sender<Bytes>) -> Self
}

impl<W: AsyncWrite + Unpin> AsyncWrite for StreamTee<W> {
    // Delegates to inner; best-effort copy to sender.
}
```

Backpressure policy: use `try_send` rather than `send` so the copy is dropped immediately when the channel is at capacity. This keeps the hot path O(1) and ensures the video player never experiences proxy-induced stalls.

### 2. `segment` — segment type detection

```
src/segment.rs
```

Identifies the container format of an intercepted response so the extractor can apply the right decode strategy. Pure function — no I/O.

Responsibilities:

- Inspect the HTTP `Content-Type` header and the URL path suffix.
- Return a `SegmentKind` indicating the likely container format, or `None` if the response is not a recognised video segment.

Key types and signatures:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    /// MPEG-2 Transport Stream (.ts), used by HLS.
    HlsTs,
    /// MPEG-4 fragment (.m4s), used by DASH and fMP4-based HLS.
    DashM4s,
    /// Recognised as some video segment, but format is not specifically handled.
    Unknown,
}

/// Detect the segment kind from response metadata.
/// Returns `None` if the response does not look like a video segment at all.
pub fn detect_segment_kind(content_type: &str, url_path: &str) -> Option<SegmentKind>
```

Detection rules:

```text
Content-Type "video/mp2t"                  -> HlsTs
URL path ends with ".ts"                   -> HlsTs
Content-Type "video/iso.segment"
  or "video/mp4" with ".m4s" in path       -> DashM4s
URL path ends with ".m4s"                  -> DashM4s
Content-Type starts with "video/"
  and none of the above                    -> Unknown
Otherwise                                  -> None
```

### 3. `extractor` — frame extraction

```
src/extractor.rs
```

Attempts to extract one representative frame from a binary segment buffer. Returns `None` when extraction is not yet implemented for the given format.

Responsibilities:

- Accept a byte buffer and a `SegmentKind`.
- For the first implementation, return a stub `None` for all inputs.
- Document the intended real strategy so the next implementer knows where to pick up.

Key types and signatures:

```rust
pub struct RawFrame {
    pub pixels: Vec<u8>,   // RGB8, row-major
    pub width:  u32,
    pub height: u32,
}

/// Extract one representative frame from `segment_bytes`.
/// Returns `None` if extraction fails or is not yet implemented.
pub fn extract_frame(segment_bytes: &[u8], kind: SegmentKind) -> Option<RawFrame>
```

Intended strategy for a future real implementation:

```text
HlsTs:
  -> parse MPEG-2 TS packet headers to locate the video PID
  -> scan for an IDR (I-frame) NAL unit start code (0x00 0x00 0x01)
  -> pass the NAL to a decoder (ffmpeg binding or pure-Rust h264 crate)
  -> return the decoded YUV frame converted to RGB8

DashM4s:
  -> parse the ISO BMFF box structure to locate the `mdat` box
  -> find the first sample entry and decode the first video sample
  -> same decoder path as above

Unknown:
  -> probe for a standalone JPEG or PNG magic number at offset 0
  -> if found, decode with the `image` crate and return the result
  -> otherwise return None
```

A real demuxer (e.g. `ffmpeg-next` bindings or a pure-Rust `mpeg2ts` crate) is deferred. The stub is marked with a `// TODO(extractor): wire real demuxer` comment.

### 4. `watchdog` — async worker

```
src/watchdog.rs
```

Spawns a background tokio task that drains the `StreamTee` channel, extracts frames from buffered segments, and classifies them.

Responsibilities:

- Hold a shared reference to an `ImageSandbox` from `packages/image-sandbox/`.
- Drain incoming `Bytes` chunks from the channel, accumulating them into a segment buffer.
- When a complete segment has been received (heuristic: channel quiet after the last chunk, or a configurable byte-count threshold), call `extract_frame`.
- If a frame is produced, call `sandbox.check()` on the frame pixels.
- On a `Block` verdict: record the verdict in the shared verdict queue. Socket drop (killing the TCP connection) is deferred — log the verdict for now.
- Expose the verdict queue so the proxy can check for recent verdicts.

Key types and signatures:

```rust
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use image_sandbox::{ImageSandbox, ImageVerdict};

pub struct VideoVerdict {
    pub stream_id: String,
    pub verdict:   ImageVerdict,
    pub timestamp: std::time::Instant,
}

pub struct VideoWatchdog {
    verdicts: Arc<Mutex<VecDeque<VideoVerdict>>>,
    // background task handle (joined on drop)
    _task: tokio::task::JoinHandle<()>,
}

impl VideoWatchdog {
    /// Spawn the background task and return the watchdog handle plus the tee
    /// end that should be inserted into the proxy's write path.
    pub fn new(
        classifier: Arc<ImageSandbox>,
        stream_id:  String,
        max_queue:  usize,
    ) -> (VideoWatchdog, tokio::sync::mpsc::Sender<bytes::Bytes>)

    /// Recent block or allow verdicts from this stream.
    pub fn verdicts(&self) -> Arc<Mutex<VecDeque<VideoVerdict>>>
}
```

The background task loop:

```text
loop {
    receive chunks from channel until timeout or channel closes
    if accumulated buffer is non-empty:
        detect_segment_kind from stored metadata (passed at construction time)
        extract_frame(buffer, kind)
        if Some(frame):
            classifier.check(frame.pixels)
            push verdict to VecDeque (cap at max_queue, evict oldest)
            if Block: log warn!("video block: {reason}, stream {stream_id}")
    clear buffer
}
```

### 5. `lib` — crate root and re-exports

```
src/lib.rs
```

Re-exports the public API surface:

```rust
pub use watchdog::{VideoWatchdog, VideoVerdict};
pub use tee::StreamTee;
pub use segment::{detect_segment_kind, SegmentKind};
pub use extractor::RawFrame;
```

## Implementation order

1. `tee.rs` — stream tee with backpressure; test with an in-memory `Vec<u8>` writer, verify that bytes reach the writer and are also delivered to the channel, and that the writer is not stalled when the channel is full.
2. `segment.rs` — kind detection; test with a matrix of Content-Type strings and URL path suffixes covering HLS, DASH, unknown video, and non-video cases.
3. `extractor.rs` — stub returning `None` for all inputs; mark the intended real demuxer strategy with `// TODO` comments; test that the stub compiles and returns `None`.
4. `watchdog.rs` — async worker with the stub extractor; test by sending synthetic `Bytes` chunks through the channel, confirm verdicts are recorded, and confirm the writer side is never blocked even when the sampler is slow.
5. Wire `VideoWatchdog` into `packages/mitm-proxy` at the Phase 5 hook, inserting `StreamTee` into the proxy's response-forwarding write path (see [network-pipeline.md](../network-pipeline.md) Phase 5).

## What this does not cover

- Real video demuxing or ffmpeg bindings — the frame extractor is a stub in the first implementation; a real demuxer is deferred until the stub is validated end-to-end.
- Socket drop and stream severing — actively killing the TCP connection to the browser on a Block verdict requires coordination with the proxy's connection table; this is deferred and replaced with a log warning for now (see [network-pipeline.md](../network-pipeline.md) Phase 5.3).
- Live screen-capture video analysis — that path goes through the daemon's `IScanner` interface and is independent of this package.
- Sampling policy (every N seconds vs. every N bytes) — the first implementation accumulates a full segment buffer; configurable time-based sampling is deferred.
- QUIC / HTTP-3 streams — the network pipeline blocks or downgrades QUIC at Phase 1; this package assumes TCP-delivered HLS and DASH only.
