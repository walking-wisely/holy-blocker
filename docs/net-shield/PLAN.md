# Net Shield — Implementation Plan

The design rationale and phase-by-phase pipeline specification live in [network-pipeline.md](../network-pipeline.md).
This document is the build plan for `packages/net-shield/`: what modules to add, in what order, and what each one is responsible for.

## Current state

The package `packages/net-shield/` does not exist yet. Nothing has been scaffolded — no `Cargo.toml`, no source files, no tests. This plan describes building it from scratch.

What the design calls for (per [network-pipeline.md](../network-pipeline.md) Phase 1 and [architecture.md](../architecture.md)):

- A TUN adapter that reads raw IP packets from a Wintun virtual device.
- An in-memory domain and IP filter (radix tree / CIDR matcher) that classifies each outbound connection as block, allow, or proxy.
- A TLS ClientHello parser that extracts the SNI extension without completing the handshake.
- A top-level `NetShield` struct that wires the three pieces into a running filter loop.

None of these exist. The entire package is new work.

## Modules to add

### 1. `radix` — domain trie and IP CIDR filter

```
src/radix.rs
```

Responsibilities:

- `DomainFilter` — trie-based matcher for domain names. A rule for `ads.example.com` matches that exact domain and all subdomains. Rules are stored label-by-label from the TLD inward so a single traversal answers the query.
- `IpFilter` — CIDR range matcher for IPv4 and IPv6. Backed by a sorted `Vec` of prefix/mask pairs; binary search by prefix gives O(log n) lookup. An interval tree or Patricia trie can replace this later if rule counts grow large, but a sorted Vec is sufficient for Phase 1.
- `FilterAction` — the shared output type for both filters:

```rust
pub enum FilterAction {
    Block,
    Allow,
    Proxy,   // forward to mitm-proxy for deep inspection
}
```

- Both filters are constructable from a flat slice of rule strings and expose a single query method. There is no file I/O in either type — callers load rules and pass them in.

Key signatures:

```rust
pub struct DomainFilter { ... }

impl DomainFilter {
    pub fn from_rules(rules: &[(&str, FilterAction)]) -> Self
    pub fn lookup(&self, domain: &str) -> FilterAction
}

pub struct IpFilter { ... }

impl IpFilter {
    pub fn from_rules(rules: &[(&str, FilterAction)]) -> anyhow::Result<Self>
    pub fn lookup(&self, addr: IpAddr) -> FilterAction
}
```

Lookup complexity is O(k) for `DomainFilter` where k is the number of labels in the hostname, and O(log n) for `IpFilter` where n is the number of CIDR rules. Both are effectively constant for realistic inputs.

### 2. `sni` — TLS ClientHello SNI extraction

```
src/sni.rs
```

Responsibilities:

- `extract_sni(buf: &[u8]) -> Option<String>` — parse the `server_name` extension from a raw TLS 1.x ClientHello record without completing or intercepting the handshake.
- Returns `None` on partial buffers (not enough bytes to complete parsing), on malformed records, or when the SNI extension is absent.
- No external TLS library dependency. This is a targeted byte-level parse of the TLS record and handshake header structures, which are stable and well-specified. The implementation reads only as far as is needed to extract the SNI hostname.

Parsing outline (no public types beyond the function):

1. Check the TLS record header: content type `0x16` (Handshake), protocol version, and record length.
2. Check the Handshake header: message type `0x01` (ClientHello).
3. Skip the legacy version, random, session ID, cipher suites, and compression methods.
4. Walk the extensions list until `extension_type == 0x0000` (server_name) or the list is exhausted.
5. Extract the first `host_name` entry from the SNI extension.
6. Validate that the result is valid UTF-8 and return it.

All length-field bounds checks return `None` rather than panicking.

### 3. `tun` — TUN adapter interface (Windows)

```
src/tun.rs
```

Responsibilities:

- `TunAdapter` — wraps a Wintun virtual adapter. Windows target only; the entire module is gated behind `#[cfg(target_os = "windows")]`. On other targets the module exposes only the `PacketSink` trait and stub types so the rest of the crate compiles for cross-platform development and testing.
- Reads raw IP packets from the Wintun ring buffer.
- Extracts the 5-tuple from each packet: src IP, dst IP, src port, dst port, protocol.
- For TCP SYN packets to port 80 or 443: runs the extracted destination through `DomainFilter` / `IpFilter` (after SNI is available on 443) and routes the result:
  - `FilterAction::Block` — drop the packet.
  - `FilterAction::Allow` — pass the packet back to the OS network stack unchanged.
  - `FilterAction::Proxy` — redirect the TCP stream to the local mitm-proxy listener port via a userspace splice (connect a new socket to the proxy and relay bytes). The full WFP callout path is deferred.
- `PacketSink` — a trait that decouples the routing logic from the real Wintun device so it can be tested with a fake in-memory sink:

```rust
pub trait PacketSink: Send {
    fn drop_packet(&mut self, pkt: &RawPacket);
    fn pass_packet(&mut self, pkt: &RawPacket);
    fn redirect_to_proxy(&mut self, pkt: &RawPacket, proxy_port: u16);
}

pub struct RawPacket {
    pub bytes:    Vec<u8>,
    pub src_ip:   IpAddr,
    pub dst_ip:   IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: u8,
}
```

`TunAdapter` implements `PacketSink` against the real Wintun device. Tests wire in a `Vec`-backed fake sink to exercise the routing decisions without hardware.

### 4. `lib` — public API

```
src/lib.rs
```

Responsibilities:

- Re-exports `DomainFilter`, `IpFilter`, `FilterAction` from `radix`, and `extract_sni` from `sni`.
- Defines `NetShield`, the top-level entry point that combines a `TunAdapter`, `DomainFilter`, and `IpFilter` into a running async filter loop:

```rust
pub struct NetShield { ... }

impl NetShield {
    pub fn new(
        domain_filter: DomainFilter,
        ip_filter:     IpFilter,
        proxy_port:    u16,
    ) -> Self

    pub async fn run(self) -> anyhow::Result<()>
}
```

`run` enters the Wintun read loop. For each packet it:

1. Parses the 5-tuple.
2. For TCP port 443 packets with enough bytes buffered, calls `extract_sni` to get the hostname.
3. Looks the hostname (or raw IP on fallback) up in `DomainFilter` / `IpFilter`.
4. Dispatches to the appropriate `PacketSink` action.

The loop runs until an unrecoverable adapter error occurs or the returned `Future` is dropped.

## Implementation order

1. `radix.rs` — pure data structures with no I/O. Build `DomainFilter` first (label trie), then `IpFilter` (sorted CIDR vec). Test both with synthetic rule sets covering exact matches, subdomain inheritance, CIDR containment, and default-allow behaviour.
2. `sni.rs` — pure byte parsing with no I/O or network state. Test with hand-constructed TLS record buffers covering: well-formed ClientHello with SNI, ClientHello without SNI extension, truncated buffers at each length-field boundary, and records with malformed extension lists.
3. `src/lib.rs` — public re-exports and the `NetShield` struct shell. At this point `run` can be a stub returning `Ok(())`.
4. `tun.rs` — `PacketSink` trait and `RawPacket` type first; test the routing dispatch logic using a fake sink against pre-built packet buffers. Then add the Wintun `TunAdapter` implementation behind `#[cfg(target_os = "windows")]`.
5. Wire `NetShield::run()` to the full loop: integrate `TunAdapter`, `DomainFilter`, `IpFilter`, and `extract_sni`; smoke-test by routing a known-block domain and confirming the packet is dropped.

## What this does not cover

- **WFP callout driver** — kernel-level packet interception via Windows Filtering Platform is deferred. Phase 1 uses userspace TUN routing (Wintun + socket splice). The `PacketSink` trait is the seam where a WFP-backed sink can be dropped in later without changing the routing logic.
- **macOS `NetworkExtension`** (`NEPacketTunnelProvider`) — handled separately in `native-modules/mac-network/`. The `PacketSink` trait can be reused, but the adapter layer is Swift and out of scope here.
- **DNS-over-HTTPS and DNS blocking** — not part of Phase 1. Domain lookup operates on SNI / Host header values extracted from live connections, not on DNS queries.
- **QUIC / HTTP3** — UDP port 443 QUIC handling requires a separate policy decision (block entirely to force HTTP/2 fallback, or allow selectively). This is deferred pending the QUIC policy design.
- **Android** — handled separately in `native-modules/android-service/`.
- **`win-network` adapter lifetime management** — installing and removing the Wintun driver, setting Windows routing rules, and managing the adapter across reboots are responsibilities of `native-modules/win-network/`, not this package.
