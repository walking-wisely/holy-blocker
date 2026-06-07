# MITM Proxy — Implementation Plan

The design rationale and phase definitions live in [../network-pipeline.md](../network-pipeline.md).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

## Related flows

- [../flows/block.md](../flows/block.md) — what happens when a scan returns Block
- [../flows/warn-interstitial.md](../flows/warn-interstitial.md) — HTML overlay injection on Warn verdict
- [../flows/protection-mode-change.md](../flows/protection-mode-change.md) — how ProtectionMode propagates to the proxy at runtime

## Current state

The package at `packages/mitm-proxy/` already has:

- `forward.rs` — plain HTTP forwarding: opens a TCP connection to the origin, rewrites the request to origin-form, strips hop-by-hop headers, sends the request, and returns the response.
- `proxy.rs` — connection dispatcher: serves HTTP/1.1 connections via hyper, routes plain HTTP to `forward_http`, and returns `501 Not Implemented` for CONNECT requests.
- `main.rs` — `tokio` TCP listener that spawns a `proxy::handle` task per accepted connection.
- `tls.rs` — certificate generation and TLS state (step 1 complete). Loads a local root CA from PEM files, generates per-SNI leaf certificates on demand with `rcgen`, caches them in a `Mutex<HashMap<String, Arc<ServerConfig>>>`, and builds a `ClientConfig` backed by the system root store. `rcgen`, `rustls`, `tokio-rustls`, and `rustls-native-certs` are in `Cargo.toml`. Tests cover SAN correctness and cache-hit behaviour.
- Unit tests for hop-by-hop header stripping (both the fixed set and `Connection`-named extensions).

Additional modules are now complete:

- `connect.rs` — CONNECT handler: sends `200 Connection Established`, peeks the TLS ClientHello for SNI, performs the two-leg TLS handshake, and hands streams to `tunnel::run`. **Done.**
- `tunnel.rs` — HTTP/1.1 loop over decrypted HTTPS with phase 3 URL/body scan call sites, phase 4 image hook, and phase 5 video tee. **Done.**
- `scan.rs` — policy hook wiring real `PolicyEngine` calls for URL and body scans; image and video hooks are stubs returning `Allow` until `image-sandbox` and `video-watchdog` are ready. **Done.**

What remains is `ProtectionMode` runtime switching (step 7 in the implementation order).

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
- ~~Stub all three functions to return `Allow` unconditionally until `packages/text-policy` exposes a stable library interface or FFI surface.~~
- Wire `scan_url` and `scan_body` to a `PolicyEngine` from `packages/text-policy` (path dep). Expose `build_default_engine()` so `main.rs` can construct an `Arc<PolicyEngine>` at startup.
- Map `Action::Block → ScanResult::Block { score }` and all other actions to `ScanResult::Allow`. The mapping is mediated by `ProtectionMode` (see step 6 below).
- `scan_image` remains a stub returning `Allow` until `packages/image-sandbox` is ready.
- This module is the only place in the proxy that knows about `text-policy`. Replacing the dictionary with a config-loaded one is isolated to this file.

Key types and signatures:

```rust
pub enum ScanResult {
    Allow,
    Block { score: u32 },
}

pub fn build_default_engine() -> PolicyEngine
pub fn scan_url(engine: &PolicyEngine, url: &str) -> ScanResult
pub fn scan_body(engine: &PolicyEngine, html: &str) -> ScanResult
pub fn scan_image(bytes: &[u8]) -> ScanResult   // Phase 4 hook, still stub
```

Tests to write:

- Assert `scan_url` returns `Allow` for a clean URL.
- Assert `scan_url` returns `Block` when the URL contains a high-severity term.
- Assert `scan_body` returns `Allow` for innocuous HTML.
- Assert `scan_body` returns `Block` for HTML containing a high-severity term.
- Assert `scan_image` returns `Allow` for any input.

### 6. `ProtectionMode` — mode-aware verdict mapping

See [../flows/protection-mode-change.md](../flows/protection-mode-change.md) for the
end-to-end flow and [../decisions/protection-modes.md](../decisions/protection-modes.md)
for rationale.

Add `ProtectionMode` to `scan.rs` and thread it through `ScanHooks` construction in
`main.rs`.

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectionMode { Full, WarnOnly, Off }

pub fn apply_mode(mode: ProtectionMode, action: Action, score: u32) -> ScanResult
```

Mode mapping:

| Engine `Action` | `Full`           | `WarnOnly` | `Off`   |
|-----------------|------------------|------------|---------|
| `Block`         | `Block { score }`| `Allow`    | `Allow` |
| any other       | `Allow`          | `Allow`    | `Allow` |

In `WarnOnly` the scan still runs and events are still emitted; only the HTTP verdict
is downgraded. In `Off` the scan closures short-circuit before calling the engine.

`main.rs` wraps the mode in `Arc<AtomicU8>` so it can be updated at runtime from a
desktop `config_update` without rebuilding `ScanHooks`.

Tests to write:

- `apply_mode(Full, Block, 90)` → `Block { score: 90 }`.
- `apply_mode(WarnOnly, Block, 90)` → `Allow`.
- `apply_mode(Off, Block, 90)` → `Allow`.
- `apply_mode(Full, Action::Warn, 60)` → `Allow`.

## Implementation order

1. ~~`tls.rs` — cert generation and two-leg TLS setup; add `rcgen`, `tokio-rustls`, `rustls`, and `rustls-native-certs` to `Cargo.toml`; test with a synthetic CA and SNI round-trip.~~ **Done.**
2. ~~`connect.rs` — CONNECT handler replacing the current 501 branch; test SNI extraction from a raw `ClientHello` byte sequence.~~ **Done.**
3. ~~`tunnel.rs` — HTTP loop with phase 3/4/5 hook call sites (all stubs, always Allow for now); test header forwarding and block-on-URL-scan behavior using injected hook closures.~~ **Done.**
4. ~~`scan.rs` — policy hook stub with correct types; unit test the stub contracts.~~ **Done.**
5. ~~Wire phase 4 image stub and phase 5 tee stub into `tunnel`; confirm existing tests still pass with no real inference running.~~ **Done.**
6. ~~Wire `text-policy` into `scan.rs`; replace stubs with real `PolicyEngine` calls; test clean/blocked URL and body paths.~~ **Done.**
7. `ProtectionMode` — add enum and `apply_mode` to `scan.rs`; thread an `Arc<AtomicU8>` through `ScanHooks` closures in `main.rs` so mode can be changed at runtime without rebuilding hooks.
8. ~~Add Criterion benchmark suite (`benches/tunnel.rs`, `benches/tls_cert.rs`, `benches/headers.rs`); expose a `[lib]` target so benches can import from the crate.~~ **Done.**
9. ~~Optimize `TlsState::server_config` cold miss — reuse a single pre-generated leaf `KeyPair` instead of calling `KeyPair::generate()` on every cache miss. Benchmarks showed cold cert generation at ~13.6 ms (dominated by ECDSA key generation); reusing the leaf `KeyPair` drops it below 2 ms.~~ **Done.**
10. ~~Add end-to-end integration tests (`tests/proxy_integration.rs`): spin up real TCP listeners on ephemeral ports and drive them with a `reqwest` client. Covers plain HTTP forwarding and the full CONNECT → TLS interception → tunnel → origin round trip.~~ **Done.**

## What this does not cover

- Actual text-policy scoring — deferred until `packages/text-policy` has a stable library surface or FFI wrapper (`src/ffi.rs`); see the text-policy plan for that work.
- Image perceptual hashing and ONNX inference — that is `packages/image-sandbox`; the phase 4 hook in `tunnel.rs` will call into it once it exists.
- Video frame extraction and ML gate — that is `packages/video-watchdog`; the phase 5 tee in `tunnel.rs` feeds the queue that watchdog will consume.
- HTTP/2 support — only HTTP/1.1 is handled for now; hyper's `http2` feature is not enabled.
- Transparent TUN routing — that is `packages/net-shield`; the proxy expects either manual browser proxy configuration or a TCP stream forwarded from the TUN adapter; it does not manage the routing policy itself.
- Local CA installation into the OS or browser trust stores — that is a one-time setup step outside the proxy binary.
