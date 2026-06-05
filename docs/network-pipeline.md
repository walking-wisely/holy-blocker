# Network Pipeline

The network pipeline is a complementary blocking mode to the screen-capture path described in [Content Classification](content-classification.md). Instead of inspecting what is already visible on screen, it intercepts web traffic at the network layer before content reaches the browser.

The two modes are independent but composable. The network pipeline blocks content at the cheapest layer possible and hands off ambiguous cases to progressively more expensive analysis. The screen-capture daemon acts as a backstop for content that slips through or arrives outside the browser.

---

## Overview

```text
outbound TCP/UDP
  -> network shield (packet filter / SNI / IP radix tree)
       known unholy  -> drop packet                       [Phase 1]
       unknown       -> route into proxy
  -> decryption engine (TLS MITM)                        [Phase 2]
  -> text gauntlet (metadata + body scan)                [Phase 3]
       triggers threshold -> sever TCP connection
       passes -> continue
  -> image sandbox (perceptual hash + ONNX fallback)     [Phase 4]
       known unholy  -> serve blank pixel
       unknown       -> ONNX ML inference (~50 ms)
       known safe    -> serve real bytes
  -> video watchdog (async stream sampling)              [Phase 5]
       frame passes ML -> stream continues
       frame flagged  -> kill TCP connection
```

The fast path exits at Phase 1 in O(1) time for known domains. Only content that passes all prior phases and cannot be classified by deterministic rules reaches the ML models.

---

## Phase 1 — Network Shield

**Privilege requirement:** System / Root. The OS must route traffic through the virtual adapter.

The network shield operates at the packet level, before any TLS decryption. It catches the largest share of bad traffic with near-zero CPU cost.

### 1.1 Packet Interception

All outbound traffic is routed through a virtual TUN adapter:

- Windows: [Wintun](https://www.wintun.net/) kernel driver, managed from `native-modules/win-network/`
- macOS: `NetworkExtension` `NEPacketTunnelProvider`, managed from `native-modules/mac-network/`

The adapter hands raw IP packets to the userspace Rust process in `packages/net-shield/`.

### 1.2 Protocol and Port Filtering

Packets are evaluated before any domain lookup:

```text
UDP (non-DNS, non-QUIC-allowed)
  -> drop immediately
  reason: catches BitTorrent, non-standard game traffic, HTTP/3 QUIC bypass attempts

TCP port != 80 and != 443
  -> configurable: pass through, log, or drop

TCP port 80 or 443
  -> proceed to domain lookup
```

QUIC (UDP 443) handling requires a separate policy. It can be blocked entirely to force HTTP/2 fallback, or allowed selectively by domain.

### 1.3 Domain and IP Lookup

The shield extracts an identifier from each connection:

- **HTTPS:** reads the `server_name` extension from the TLS `ClientHello` (SNI), before the handshake proceeds
- **HTTP:** reads the `Host` header or raw destination IP
- **IP-only fallback:** uses the destination IP when SNI is absent

The identifier is looked up in an in-memory radix tree of known domains and CIDR blocks. Lookup complexity is O(k) where k is the length of the domain label sequence — effectively constant for a bounded hostname length.

```text
SNI / IP extracted
  -> radix tree lookup
     match: known unholy domain or IP
       -> drop packet immediately
       -> browser displays a connection refused error
     match: known safe domain
       -> pass through to OS network stack (not proxied)
     no match
       -> route TCP stream into the MITM proxy (Phase 2)
```

The radix tree is loaded from the hash-db blocklist at startup and updated when the user installs new rule packs. It lives entirely in userspace memory and requires no kernel modifications beyond the TUN adapter itself.

---

## Phase 2 — Decryption Engine

Unknown or partially-trusted traffic is forcibly routed from the TUN adapter into the Rust proxy server (`packages/mitm-proxy/`).

### 2.1 TLS Termination

The proxy presents a dynamically generated certificate for the target domain, signed by a locally-trusted root CA. The root CA certificate is installed into the OS and browser trust stores during initial setup.

```text
browser -> proxy (presents generated cert for target.com, signed by local CA)
proxy   -> origin (establishes real TLS connection to target.com)
```

The local CA private key never leaves the device. Certificate generation uses the target SNI so that the browser's address bar continues to show the correct domain name.

### 2.2 Payload Extraction

Once the TLS session is established on both sides, the proxy holds the raw plaintext:

```text
HTTP headers
  -> Host, Content-Type, Set-Cookie, redirects
HTML body
  -> raw HTML string before browser parsing
Media bytes
  -> image buffers, video segment buffers
```

These are passed to the filter phases below. If all phases pass, the proxy forwards the bytes to the browser normally.

---

## Phase 3 — Text Gauntlet

Text analysis runs synchronously before any image or media bytes are forwarded to the browser. A single positive signal at this phase prevents all subsequent media from loading.

### 3.1 Metadata Scan

The proxy extracts high-signal text fields immediately on receiving response headers and the first bytes of the HTML body:

```text
URL path and query string
<title> tag content
<meta name="description"> content
<meta name="keywords"> content
Open Graph og:title, og:description
```

These fields are passed to `packages/text-policy/` for rapid scoring. Because this text is compact, the deterministic rule pass is fast enough to block before the rest of the body arrives.

### 3.2 Body Scan

If metadata passes, the proxy buffers the HTML body and scans for dense clusters of forbidden keywords across the full text:

```text
raw HTML text
  -> strip tags (keep visible text and alt attributes)
  -> feed to text-policy scoring pipeline
  -> evaluate against configured threshold
```

See [Content Classification](content-classification.md) for the full text-policy pipeline including normalization, tokenization, evasion handling, and scoring.

### 3.3 Action

```text
score >= block threshold
  -> sever TCP connection immediately
  -> optionally serve a custom "Blocked" HTML response before severing

score < block threshold
  -> allow body bytes to pass through to browser
  -> proceed to Phase 4 for images
```

---

## Phase 4 — Image Sandbox

If the text passes, the browser begins requesting embedded images. The proxy intercepts each image request and holds the bytes in memory before forwarding them. The browser waits for the proxy response and does not render anything until Phase 4 completes.

### 4.1 Perceptual Hashing

For each intercepted image buffer, the proxy generates a perceptual hash (e.g., dHash — an 8-byte difference hash of the image's visual structure). This hash is content-addressed rather than file-addressed: the same visual image produces the same hash regardless of encoding or minor crops.

### 4.2 Local SQLite Lookup

The hash is looked up in the local SQLite database (`data/hash-db/hashes.sqlite`):

```text
hash lookup result: known unholy
  -> discard image buffer
  -> serve a transparent 1x1 pixel to the browser

hash lookup result: known safe
  -> forward real image bytes to the browser immediately

hash not found
  -> proceed to ONNX fallback (Phase 4.3)
```

The SQLite database can hold millions of hashes. Lookup by hash column with an index is fast enough to run synchronously per image request.

### 4.3 ONNX Fallback (Zero-Day)

Unknown images are passed to the local quantized ML vision model (e.g., MobileNetV2 INT8 in ONNX format):

```text
image buffer
  -> resize and normalize to model input shape
  -> ONNX Runtime inference (~50 ms on CPU)
  -> model output: category probabilities

result: unholy (above configured confidence threshold)
  -> discard image buffer
  -> serve transparent 1x1 pixel
  -> save hash to local SQLite blocklist as known unholy

result: safe
  -> forward real image bytes to browser
  -> optionally save hash to local SQLite as known safe
```

The ONNX model is loaded once at proxy startup and kept in memory. Inference runs on a thread pool to avoid blocking the main proxy loop on slow images.

---

## Phase 5 — Video Watchdog

Video streams cannot be buffered synchronously without causing the player to stall indefinitely. This phase uses a "sniff and sever" approach: content flows immediately while a background sampler checks frames asynchronously.

### 5.1 Stream Passthrough

The proxy forwards HLS `.ts` segments and DASH `.m4s` fragments to the browser as they arrive. The browser's media player buffers and plays these normally.

### 5.2 Frame Extraction

The proxy maintains a silent background sampler per active video stream:

```text
every 3–5 seconds (configurable):
  -> copy one arriving segment buffer (without blocking the forward path)
  -> extract a single representative raw frame
  -> enqueue the frame for ML inspection
```

Frame extraction should use a lightweight in-process decoder (e.g., a minimal FFmpeg binding or a pure-Rust decoder for common container formats). It does not need to decode the full segment — one I-frame per sample interval is sufficient.

### 5.3 ML Inspection

The extracted frame is passed to the same ONNX vision model used in Phase 4:

```text
raw frame
  -> resize and normalize to model input shape
  -> ONNX Runtime inference

result: safe
  -> stream continues uninterrupted

result: unholy
  -> proxy kills the active TCP connection immediately
  -> browser's video player freezes and throws a network error
  -> no further segment requests are fulfilled for this stream
```

The kill happens via the proxy's active connection table. The proxy does not need to modify the TCP stack directly — closing the proxy's own socket to the browser is sufficient.

---

## Component Map

The network pipeline maps onto the following workspace locations:

```text
packages/
  text-policy/        (existing) Rust text classification and rule engine
  net-shield/         TUN packet reader, SNI extractor, radix-tree domain/IP filter
  mitm-proxy/         TLS termination, HTTP/HTTPS proxying, phase routing
  image-sandbox/      Perceptual hashing, SQLite hash lookup, ONNX inference
  video-watchdog/     Async segment sampler, frame extraction, ML gate

native-modules/
  win-daemon/         (existing) Win32 screen-capture daemon
  win-network/        Wintun driver binding, Windows routing policy (netsh / WFP)
  mac-network/        macOS NetworkExtension NEPacketTunnelProvider

data/                 (gitignored — private runtime assets)
  ca/                 Local root CA private key and self-signed certificate
  hash-db/            SQLite perceptual hash database
  models/             Quantized ONNX and TFLite model files

machine-learning/     (existing) Training and export pipeline
  models/
    image-v1/         Screen-capture classifier (existing)
    web-image-v1/     Web image classifier for Phases 4 and 5 (proposed)
```

### Package Responsibilities

| Package | Language | Key dependency | Role |
|---|---|---|---|
| `net-shield` | Rust | `tun` crate, `etherparse` | Packet parsing, SNI extraction, radix tree |
| `mitm-proxy` | Rust | `hyper`, `rustls`, `rcgen` | TLS termination, HTTP proxy, phase dispatch |
| `image-sandbox` | Rust | `image`, `rusqlite`, `ort` | dHash, SQLite lookup, ONNX inference |
| `video-watchdog` | Rust | `ffmpeg-next` or `minimpeg` | Segment buffering, frame extraction, ML gate |
| `text-policy` | Rust | `aho-corasick`, `unicode-normalization` | Text scoring (shared with screen-capture path) |
| `win-network` | C++ / Rust | Wintun SDK | Adapter lifetime, Windows routing rules |
| `mac-network` | Swift | NetworkExtension | `NEPacketTunnelProvider` implementation |

---

## Privilege and Trust Model

The network pipeline requires two distinct privilege levels:

| Component | Privilege | Reason |
|---|---|---|
| `win-network` / `mac-network` | System / Root | Installing and managing the virtual TUN adapter |
| Proxy process (`mitm-proxy` + phases) | User | TLS termination, hashing, ML inference |
| Local CA installation | One-time admin | Adding the root cert to OS and browser trust stores |

The elevated network adapter runs as a minimal native module and does not execute ML inference or parse HTTP. All filtering logic runs in the unprivileged user-space proxy.

The local root CA private key should be stored in the OS credential manager (Windows Credential Manager, macOS Keychain) rather than as a plain file, and regenerated on first run rather than shipped with the software.

---

## Relationship to Screen-Capture Path

The network and screen-capture pipelines are independent and can run simultaneously:

| Network pipeline | Screen-capture pipeline |
|---|---|
| Blocks before content reaches the browser | Catches content already visible on screen |
| Requires TUN adapter + root CA setup | Requires accessibility / screen recording permission |
| Works only for browser traffic | Works for any visible application window |
| Blind to local files and cached content | Can inspect anything currently rendered |
| High precision (inspects actual bytes) | Handles PDF viewers, media players, native apps |

Running both together provides defense in depth: the network pipeline stops most bad content before it renders, and the screen-capture daemon handles edge cases such as cached pages, native apps, and content loaded before the proxy was active.
