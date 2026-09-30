# DNS review traps

Applies to `packages/domain-blocklist` (build-time list pipeline), `packages/net-shield` +
`packages/net-shield-ffi` (runtime filter), and `apps/mobile`'s `NetworkGuardService` /
`NetworkGuard` / `Blocklist`. These are three different things and a review must name which.

## Denial of existence is not uniform

- `.com`, `.net`, `.org` and most large TLDs sign with **NSEC3 opt-out** (RFC 5155 §6), so a
  validator cannot prove non-existence and `AD` is never set. Any logic gated on authenticated
  denial is unreachable for the majority of real domains.
- Cloudflare-hosted zones answer nonexistent names with **NODATA, not NXDOMAIN** (compact denial of
  existence). Pruning keyed on NXDOMAIN silently stops pruning under those zones.
- The root zone does **not** use opt-out, so controls under `invalid.` / `example.test.` come back
  authenticated and a canary built on them passes while the real sweep does nothing. A control that
  lives in a different part of the namespace than the swept data is not a control.
- RCODE in a CNAME/DNAME chain answer refers to the **last name in the chain** (RFC 6604 §3,
  RFC 6672), not the queried name. A live registration behind a dead target must not be pruned.

## Filtering resolvers

- A large class of resolvers (`1.1.1.3`, OpenDNS FamilyShield, filtered Quad9, most ISP defaults,
  many CI networks) returns NXDOMAIN **specifically for the category being swept**. A sweep that
  does not name its resolver is not reproducible, and a canary whose alive-controls are all out of
  category is structurally blind to exactly this.
- RFC 8914 Extended DNS Errors 15/16/17 are the strongest available self-declaration of filtering.
  Flattening them into plain NXDOMAIN discards the signal that would catch the failure.

## Response handling on the runtime path

- A forwarding socket must be `connect()`ed, and the response's transaction ID and question section
  must be checked before the answer is used. An unconnected `DatagramSocket` accepts a datagram
  from any source, and the wrapped result is written into the TUN as authoritative.
  **Known live defect** in `apps/mobile/.../NetworkGuardService.kt` (`ask()`), unfixed at the time
  of writing.
- Truncation (`TC=1`) requires a TCP retry (RFC 1035 §4.2.1, RFC 7766). Check what the code does
  when the TCP path is not routed — hang, fail open, and fail closed are three different bugs.
- Compression pointers in a QNAME must be rejected, never followed; fragments refused. This is
  already the pattern in `net-shield/src/dns.rs` — new parsers should match it.

## Coverage claims

- A port-53 plaintext filter does not cover DoH, DoT, hardcoded resolvers, DNS on a non-standard
  port, or direct-IP connections. Chrome auto-upgrades to DoH where available and Android's Private
  DNS is a system-wide DoT toggle, so this is the mainstream path, not the tail.
- The Android TUN claims a single `/32` route by platform necessity (no packet re-injection, so a
  permitted TCP flow would need a userspace TCP stack). Everything not addressed to that `/32` is
  unfiltered **by construction** — say so in any statement of what the guard does.
- A DNS blocklist blocks names, not content. Sinkholed, parked, and CDN-shared names all need
  explicit handling before "the domain is dead" means anything.

## Pipeline hygiene

- A missing or unreadable previous manifest must not read as a first build; that silently disables
  the shrinkage and growth gates.
- A blocklist source's rate limit and terms are external facts — check them, don't assume them.
- Normalisation must be identical on both sides of every comparison (lowercase, strip `www.`,
  punycode, trailing dot) or the union quietly under-merges.
