# MITM Proxy — Implementation Plan

The design rationale and phase definitions live in [../network-pipeline.md](../network-pipeline.md).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

## Current state

The package at `packages/mitm-proxy/` already has:

- `forward.rs` — plain HTTP forwarding: opens a TCP connection to the origin, rewrites the request to origin-form, strips hop-by-hop headers, sends the request, and returns the response.
- `proxy.rs` — connection dispatcher: serves HTTP/1.1 connections via hyper, routes plain HTTP to `forward_http`, and returns `501 Not Implemented` for CONNECT requests.
- `main.rs` — `tokio` TCP listener that spawns a `proxy::handle` task per accepted connection.
- `tls.rs` — certificate generation and TLS state (step 1 complete). Loads a local root CA from PEM files, generates per-SNI leaf certificates on demand with `rcgen`, caches them in a `Mutex<HashMap<String, Arc<ServerConfig>>>`, and builds a `ClientConfig` backed by the system root store. `rcgen`, `rustls`, `tokio-rustls`, and `rustls-native-certs` are in `Cargo.toml`. Tests cover SAN correctness and cache-hit behaviour.
- Unit tests for hop-by-hop header stripping (both the fixed set and `Connection`-named extensions).

What is missing is everything required to complete HTTPS interception and route responses through the filter phases:

- No CONNECT handler (Phase 2): CONNECT tunnels still return 501 — `connect.rs` not yet written.
- No HTTP loop over decrypted streams: `tunnel.rs` not yet written.
- No text-policy integration (Phase 3): URL, metadata, and body are never scanned.
- No image interception hook (Phase 4): image responses pass through unexamined.
- No video segment tee (Phase 5): video segments pass through unexamined.

## Modules to add

### 1. `tls` — certificate generation and TLS termination

```
src/tls.rs
```

Responsibilities:

- Load the local root CA key pair from `data/ca/` at startup (PEM files: `ca.crt` and `ca.key`).
- Generate a per-SNI leaf certificate on demand using `rcgen`: new key pair, X.509 cert with the correct `SubjectAlternativeName` (`DNS:<hostname>`), signed by the local CA private key.
- Cache generated certificates in a `HashMap<String, Arc<CertifiedKey>>` keyed by SNI hostname so each domain pays the generation cost only once per proxy session.
- Expose a `TlsState` struct that holds the CA material and cache, and a method to produce a `ServerConfig` for a given SNI.
- The origin-facing TLS client must use the system root store for real certificate validation (not the local CA).

Key types and signatures:

```rust
pub struct TlsState { /* CA cert, CA key, cert cache */ }

impl TlsState {
    pub fn load(ca_dir: &Path) -> anyhow::Result<Self>
    pub fn server_config(&self, sni: &str) -> anyhow::Result<Arc<ServerConfig>>
    pub fn client_config() -> Arc<ClientConfig>   // validates real certs
}
```

Add to `Cargo.toml`:

```toml
rcgen          = "0.13"
rustls         = { version = "0.23", features = ["ring"] }
tokio-rustls   = "0.26"
rustls-native-certs = "0.8"
```

Tests to write:

- Generate a synthetic root CA with `rcgen`, call `server_config` for `"example.com"`, and verify the returned `ServerConfig` presents a certificate with the correct SAN and a chain that validates under the synthetic root.
- Call `server_config` twice for the same hostname and assert only one generation occurred (cache hit counter or `Arc` pointer equality).

### 2. `connect` — HTTPS CONNECT handler

```
src/connect.rs
```

Responsibilities:

- Receive the raw `TcpStream` that hyper hands back after parsing a CONNECT request (via the hyper `upgrade` mechanism).
- Send `200 Connection Established` to the browser to signal that the TCP pipe is open.
- Peek the first bytes of the raw stream to find the TLS `ClientHello` and extract the SNI hostname without consuming bytes.
- Use `TlsState` from `tls.rs` to complete a TLS handshake with the browser (client-facing leg).
- Open a new TCP connection to the real origin host:port and complete a TLS handshake with the origin (origin-facing leg), using `TlsState::client_config()` for real certificate validation.
- Hand the two resulting `TlsStream` handles to `tunnel::run` for the HTTP loop.
- Replace the current `StatusCode::NOT_IMPLEMENTED` branch in `proxy.rs` with a call into this module.

Key types and signatures:

```rust
pub async fn handle_connect(
    target: Authority,          // host:port from the CONNECT request
    upgraded: TokioIo<Upgraded>,
    tls: Arc<TlsState>,
) -> anyhow::Result<()>
```

Tests to write:

- Construct a minimal raw TLS `ClientHello` byte sequence carrying a known SNI value and assert that the SNI extractor returns the correct hostname.
- Assert that a malformed or SNI-absent `ClientHello` returns a descriptive error rather than panicking.

### 3. `tunnel` — HTTP/1.1 loop over decrypted HTTPS

```
src/tunnel.rs
```

Responsibilities:

- Accept two decrypted `TlsStream` handles (browser-facing and origin-facing) and run the HTTP/1.1 request/response loop using hyper in client+server mode.
- **Phase 3 — metadata scan:** Before forwarding each request, pass the request URL to `scan::scan_url`. On a `Block` verdict, sever the browser-facing stream immediately; optionally send a small HTML "Blocked" response first.
- **Phase 3 — body scan:** Buffer the origin response body up to a configurable size limit, pass visible text to `scan::scan_body`. On a `Block` verdict, sever; on `Allow`, flush all buffered bytes to the browser.
- **Phase 4 hook:** For responses with `Content-Type: image/*`, pass the response bytes to `scan::scan_image` (stub returning `Allow` until `packages/image-sandbox` is ready).
- **Phase 5 hook:** For HLS `.ts` and DASH `.m4s` responses, tee bytes to the browser immediately and push a copy into a background `tokio::sync::mpsc` queue for the video watchdog (stub consumer that drops frames until `packages/video-watchdog` is ready).
- Connection severing is performed by dropping the browser-facing `TlsStream` handle; no additional TCP RST is needed.

Key types and signatures:

```rust
pub async fn run(
    browser: TlsStream<TcpStream>,
    origin:  TlsStream<TcpStream>,
    scan:    Arc<ScanHooks>,
) -> anyhow::Result<()>

pub struct ScanHooks {
    pub url_scanner:   Box<dyn Fn(&str) -> ScanResult + Send + Sync>,
    pub body_scanner:  Box<dyn Fn(&str) -> ScanResult + Send + Sync>,
    pub image_scanner: Box<dyn Fn(&[u8]) -> ScanResult + Send + Sync>,
    pub video_tx:      mpsc::Sender<Bytes>,
}
```

Tests to write:

- Build a `ScanHooks` that always returns `Allow`; assert that request headers and response bodies are forwarded without modification.
- Build a `ScanHooks` whose `url_scanner` returns `Block { score: 100 }` for a specific URL; assert the browser-facing connection is severed before any bytes from the origin are forwarded.
- Build a `ScanHooks` whose `body_scanner` returns `Block` after receiving an HTML body; assert the connection is severed and no body bytes reach the browser.
- Assert that body buffering stops at the configured size limit and continues forwarding after the limit is reached (no block triggered).

### 4. `scan` — text-policy integration hook

```
src/scan.rs
```

Responsibilities:

- Define `ScanResult` and expose the three hook functions that `tunnel.rs` calls.
- Stub all three functions to return `Allow` unconditionally until `packages/text-policy` exposes a stable library interface or FFI surface.
- This module is the only place in the proxy that knows about `text-policy`. Replacing the stubs with real calls is isolated to this file.

Key types and signatures:

```rust
pub enum ScanResult {
    Allow,
    Block { score: u32 },
}

pub fn scan_url(url: &str) -> ScanResult
pub fn scan_body(html: &str) -> ScanResult
pub fn scan_image(bytes: &[u8]) -> ScanResult   // Phase 4 hook
```

Tests to write:

- Assert `scan_url` returns `Allow` for any input (stub contract test).
- Assert `scan_body` returns `Allow` for any input.
- Assert `scan_image` returns `Allow` for any input.
- These tests are intentionally trivial; they exist so that replacing a stub with a real implementation forces a test update.

## Implementation order

1. ~~`tls.rs` — cert generation and two-leg TLS setup; add `rcgen`, `tokio-rustls`, `rustls`, and `rustls-native-certs` to `Cargo.toml`; test with a synthetic CA and SNI round-trip.~~ **Done.**
2. ~~`connect.rs` — CONNECT handler replacing the current 501 branch; test SNI extraction from a raw `ClientHello` byte sequence.~~ **Done.**
3. `tunnel.rs` — HTTP loop with phase 3/4/5 hook call sites (all stubs, always Allow for now); test header forwarding and block-on-URL-scan behavior using injected hook closures.
4. `scan.rs` — policy hook stub with correct types; unit test the stub contracts.
5. Wire phase 4 image stub and phase 5 tee stub into `tunnel`; confirm existing tests still pass with no real inference running.

## What this does not cover

- Actual text-policy scoring — deferred until `packages/text-policy` has a stable library surface or FFI wrapper (`src/ffi.rs`); see the text-policy plan for that work.
- Image perceptual hashing and ONNX inference — that is `packages/image-sandbox`; the phase 4 hook in `tunnel.rs` will call into it once it exists.
- Video frame extraction and ML gate — that is `packages/video-watchdog`; the phase 5 tee in `tunnel.rs` feeds the queue that watchdog will consume.
- HTTP/2 support — only HTTP/1.1 is handled for now; hyper's `http2` feature is not enabled.
- Transparent TUN routing — that is `packages/net-shield`; the proxy expects either manual browser proxy configuration or a TCP stream forwarded from the TUN adapter; it does not manage the routing policy itself.
- Local CA installation into the OS or browser trust stores — that is a one-time setup step outside the proxy binary.
