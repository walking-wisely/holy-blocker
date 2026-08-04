# Security Backlog

This document turns the current security discussion into a concrete backlog for Holy Blocker.
It is intentionally scoped to the architecture that exists or is already planned in this
repository: a local-first desktop app, Windows daemon, local MITM proxy, policy engine,
and local ML pipeline.

The backlog is ordered by engineering priority, not by abstract severity labels. Items in
`Now` are the controls that should be established early because they reduce risk across the
entire repository and release process. Items in `Soon` depend on more runtime code existing.
Items in `Later` are important, but they should follow once the relevant components stabilize.

## Security objectives

- Preserve the local-first model: no screenshots, OCR text, browsing history, or decrypted
  traffic should leave the device unless a future feature explicitly requires it.
- Prevent silent disablement: local processes and local users should not be able to turn off
  protection through weak IPC, weak file permissions, or unsafe defaults.
- Prevent secret leakage: CA private keys, signing credentials, private eval packs, and model
  artifacts must not leak through the repo, CI logs, or public workflows.
- Keep security decisions explainable: policy results, mode changes, and override attempts
  should be auditable without logging raw sensitive content.

## Now

### 1. Branch protection and required checks

Add branch protection for the default branch (`master`) and require passing status checks before merge.

Why now:

- This is the lowest-cost way to prevent bypassing future security checks.
- It creates a single place to enforce typecheck, tests, and security scans.

Suggested checks:

- JavaScript workspace install/build/typecheck
- Rust tests
- Python tests
- Native Windows build/test job
- Secret scanning
- SAST

### 2. Secret scanning in CI — **Done.**

~~Add repository-wide secret scanning with `gitleaks` or `trufflehog`.~~

Implemented with `gitleaks` in two layers: a `pre-commit` hook
(`.pre-commit-config.yaml`) running `gitleaks protect --staged` on every local
commit, and a `secret-scan` CI job (`.github/workflows/ci.yml`) running
`gitleaks detect` against full git history on every PR and push. The project
allowlist baseline lives in `.gitleaks.toml`; false positives are added there
with a documented reason rather than suppressed inline.

### 3. CodeQL SAST across TypeScript, Rust, Python, and C++

Enable GitHub CodeQL for all supported languages in this repository.

Why now:

- The repo is polyglot and crosses high-risk trust boundaries.
- It is more useful to start with broad language coverage than to overfit one package.

Focus areas:

- Electron IPC and preload exposure
- Unsafe filesystem handling
- Rust parsing and proxy input handling
- C++ memory and handle lifetime issues
- Python artifact loading and shelling/conversion flows

### 4. Dependency update and supply-chain hygiene

Enable Dependabot or Renovate for `pnpm`, Cargo, Python, and GitHub Actions.

Why now:

- Electron, Rust TLS crates, Python ML tooling, and test dependencies will age quickly.
- GitHub Actions are part of the supply chain and should be pinned and updated deliberately.

Definition of done:

- Lockfiles are committed and reviewed.
- Action versions are pinned to immutable references where practical.
- Update PRs run the normal CI checks.

### 5. Public vs private CI separation

Define which jobs are safe for untrusted pull requests and which are trusted-only.

Why now:

- The repo already expects private eval packs and sensitive artifacts later.
- If the trust model is not defined early, secrets tend to leak into generic workflows.

Public CI:

- Build, typecheck, unit tests, sanitized fixtures, static scans

Trusted-only CI:

- Private eval-pack download
- Signing
- Release packaging
- Any job that touches secrets, private models, or private corpora

### 6. SECURITY.md and disclosure process — **Done.**

~~Add a top-level `SECURITY.md`.~~

Added top-level `SECURITY.md` covering supported versions, private reporting via
GitHub private vulnerability reporting (with email fallback), what to include and
what not to include in a report, local-first scope and out-of-scope cases, and a
48h-acknowledge / 7-day-triage response commitment.

Follow-up: enable **Private vulnerability reporting** in the repo settings
(Settings → Security) so the channel `SECURITY.md` points to is live.

### 7. STRIDE threat models for current trust boundaries

Create initial threat models for the two main runtime paths:

- screen-capture path
- network interception path

Why now:

- The risk in Holy Blocker is not generic web risk; it is local trust-boundary failure.
- STRIDE should drive design controls, tests, and CI policy rather than sit as a standalone doc.

Minimum threat-model scope:

- renderer -> preload -> Electron main
- Electron main -> named pipe -> Windows daemon
- daemon -> local storage
- browser/client -> MITM proxy -> origin
- proxy -> CA key material
- CI -> private eval assets -> release artifacts

## Soon

### 8. Named-pipe hardening and local peer trust

Harden desktop/daemon IPC before the named-pipe protocol becomes a control plane.

Why soon:

- The desktop plan already introduces persistent named-pipe IPC and config updates.
- A forged local client/server is one of the clearest spoofing risks in the repo.

Controls to implement:

- Restrictive pipe ACLs
- Explicit server/client ownership expectations
- Defensive message validation
- Versioned message schema
- Tests for malformed, oversized, replayed, or unauthorized messages

### 9. File-permission and secret-material policy

Define how local sensitive files are stored and protected.

Applies to:

- `data/ca/`
- local policy/config files
- stats/event stores
- future private model artifacts
- downloaded private eval packs on trusted runners

Controls:

- keep secrets out of the repo by default
- document allowed locations
- minimize retained sensitive content
- set restrictive filesystem permissions where the platform allows it

### 10. Sensitive logging and redaction rules

Add explicit rules for what may never appear in logs, test output, or CI artifacts.

Must not log by default:

- screenshots
- OCR text
- decrypted HTTP bodies
- raw private eval cases
- root CA private key material

Preferred alternatives:

- opaque case IDs
- aggregate metrics
- hashes or redacted summaries
- structured event types without raw content

### 11. Release artifact signing and provenance

Define the release trust chain before end-user distribution starts.

Why soon:

- This is a local security product. Users need confidence that binaries, native modules, and
  model artifacts are authentic.

Scope:

- desktop binaries/installers
- native daemon binaries
- proxy binaries
- shipped model artifacts

Controls:

- signing for release artifacts
- published checksums
- provenance or attestation from trusted CI

### 12. Security-focused test cases for abuse paths

Turn the highest-value STRIDE findings into tests.

Examples:

- forged `config_update` attempts
- malformed IPC message parsing
- proxy parser stress cases
- oversized body buffering limits
- config tampering and invalid threshold files
- model artifact checksum mismatch

The goal is not exhaustive pentesting. The goal is preventing silent regressions on known abuse
paths.

### 13. SBOM generation for releases

Generate a software bill of materials for release artifacts.

Why soon:

- The repo is polyglot and will ship security-relevant binaries.
- SBOMs make dependency review and incident response easier when a library vulnerability lands.

## Later

### 14. DAST for any future hosted surfaces

If the project later adds hosted services, admin panels, docs apps, or APIs, add DAST such as
OWASP ZAP against staging.

Why later:

- The current core product is local-first, not an internet-facing SaaS app.
- DAST is lower value than IPC, artifact, and local-storage hardening at the current stage.

### 15. Infrastructure-as-code scanning

If the project introduces Terraform, Pulumi, or cloud deployment manifests, add IaC scanning
with tools such as Checkov or Trivy.

Why later:

- This only becomes relevant once cloud infrastructure exists.
- It should be added immediately when infrastructure code appears, not before.

### 16. Fuzzing and property-based testing on parsers and scanners

Add fuzzing where untrusted input is parsed or normalized.

Strong candidates:

- proxy request/response handling
- TLS ClientHello/SNI parsing
- text normalization and matcher inputs
- IPC framing and JSON decoding

### 17. Reproducible-build improvements

Improve the ability to reproduce release artifacts from source.

Why later:

- Valuable for trust and incident response.
- Usually easier after the packaging pipeline stabilizes.

### 18. Incident response and key rotation playbooks

Write operational runbooks for:

- leaked signing credentials
- leaked CA material
- compromised private eval storage
- malicious dependency introduction

These do not need to be elaborate at first, but they should exist before external distribution
becomes broad.

## Recommended rollout order

If only a few items can be started immediately, do them in this order:

1. Branch protection and required checks
2. Secret scanning
3. CodeQL
4. Dependency update automation
5. Public vs private CI separation
6. SECURITY.md
7. Initial STRIDE documents

That sequence gives the repository a real baseline without waiting for all runtime components to
be finished.

## Audit findings

Findings from `holy-blocker-security` audit passes, newest package first. Unlike the sections
above — which are program-level controls — each entry here is a specific defect with a concrete
attack and a file reference. The gate for a PR is that it introduces no *new* finding; an entry
marked `accepted-baseline` is known debt scheduled against the owning package's plan, not a pass.

This section is public deliberately. A finding that would give a stranger a working attack against a
**released** version never appears here — it goes straight into a private draft advisory, and this
file gets at most a placeholder naming the package and the advisory. See
[`SECURITY.md`](../../SECURITY.md). There are no releases yet, so nothing currently meets that bar.

### `packages/net-shield` and `packages/mitm-proxy` — audited 2026-08-04

Audited at commit `fb69d30`. Boundaries matched: untrusted input parsers, TLS interception and CA.
`dns.rs`, `udp.rs` and `radix.rs` were reviewed and produced no findings — bounds are checked
through `get()`/`checked_add()` throughout.

Three distinct failure paths in `net-shield` are easy to conflate, and all three happen to be
fail-safe, but for different reasons. Worth stating separately so a later change cannot quietly
rely on the wrong one:

- **Packet parse failure** — neither `parse_ipv4_packet` nor `parse_ipv6_packet` accepts the
  buffer. `process_packet` returns `false` and dispatches nothing at all, so the packet is not
  forwarded. Fail-closed, and not a `FilterAction` at any point.
- **SNI extraction failure** — the packet parses but `extract_sni` returns `None`. The decision
  falls back to `ip_filter.lookup(dst_ip)`, which is a real lookup and can return any action.
- **Unmatched lookup** — a name or address matches no rule. Both filters default to
  `FilterAction::Proxy`, so an unknown destination is inspected rather than allowed.

Only the third is the `Proxy` default. The finding below on split ClientHellos concerns the second,
and depends on the third to contain it.

#### [HIGH] net-shield: a 24-byte crafted packet panics the filter loop

- **Boundary:** untrusted input parsers
- **Attack:** `parse_ipv4_packet` guarantees only `buf.len() >= ihl + 4`, but `process_packet`
  then indexes `raw[ihl + 12]` directly to read the TCP data offset. A 24-byte IPv4/TCP packet to
  port 443 passes the parse and panics on the index. On the Wintun loop the panic propagates out
  of `run_windows`, so any host that can get one packet onto the TUN stops the filter — and a
  stopped filter is an open network.
- **Evidence:** `packages/net-shield/src/lib.rs:65`, guarantee at `packages/net-shield/src/tun.rs:63`
- **Repro:** build an IPv4/TCP packet, `p[0]=0x45`, total length 24, `p[9]=6`, dst port 443, and
  call `NetShield::process_packet`. Confirmed: `index out of bounds: the len is 24 but the index is 32`.
- **Status:** open

#### [HIGH] net-shield: IPv6 packets are parsed with IPv4 header arithmetic

- **Boundary:** untrusted input parsers
- **Attack:** `process_packet` accepts either address family, but the SNI branch unconditionally
  computes `ihl` from `raw[0] & 0x0f` — the low nibble of an IPv6 packet is part of the traffic
  class, not a header length. A 60-byte IPv6 packet to port 443 with `p[0] = 0x6f` yields
  `ihl = 60` and panics on `raw[72]`. Where it does not panic it reads the SNI from the wrong
  offset, so IPv6 HTTPS is filtered against garbage and falls through to the IP filter.
- **Evidence:** `packages/net-shield/src/lib.rs:64-66`
- **Repro:** confirmed — `index out of bounds: the len is 60 but the index is 72`.
- **Status:** open

#### [MED] net-shield: SNI hostnames are not case-normalised

- **Boundary:** untrusted input parsers
- **Attack:** `DomainFilter` keys its trie on raw label strings and `extract_sni` returns the
  hostname exactly as the client sent it. TLS SNI is case-insensitive, so a client offering
  `EXAMPLE.COM` misses a rule written as `example.com` and downgrades from `Block` to the default
  `Proxy`. `dns.rs` lowercases (`read_name`, per RFC 4343 §3) and the SNI path does not, so one
  rule set behaves differently depending on which path sees the name. `insert` does not normalise
  either, so any rule authored with uppercase is silently dead on both paths.
- **Evidence:** `packages/net-shield/src/sni.rs:117`, `packages/net-shield/src/radix.rs:35`
- **Status:** open

#### [MED] net-shield: a ClientHello split across TCP segments is not filtered by name

- **Boundary:** untrusted input parsers
- **Attack:** `extract_sni` requires the whole ClientHello in one buffer and returns `None`
  otherwise; `process_packet` then falls back to `ip_filter.lookup(dst_ip)`. Splitting the
  ClientHello across two segments — a one-line change in a client, and the standard SNI-filter
  evasion — therefore takes the name out of the decision entirely. The default `Proxy` action
  contains the damage today, but any rule set carrying an explicit `Allow` CIDR turns this into a
  silent bypass.
- **Evidence:** `packages/net-shield/src/lib.rs:67-71`, `packages/net-shield/src/sni.rs:21`
- **Status:** open

#### [SYSTEMIC] net-shield: the wire-format parsers have no fuzz or property tests

- **Boundary:** untrusted input parsers
- **Why the findings above were missed:** all 70 unit tests feed well-formed packets built by a
  helper; none feeds a truncated or hostile one. Both panics are reachable by the first malformed
  input a fuzzer would try, which is why a package with full unit coverage still shipped them.
- **Correction:** `proptest` over `dns::parse_query`, `udp::parse_ipv4_udp`, `sni::extract_sni`
  and `NetShield::process_packet` asserting only "does not panic" would have caught both. Track
  the implementation through the `test` skill; it is recorded here because the gap is what let two
  `[HIGH]`s through, not because writing tests is a security task.
- **Evidence:** `packages/net-shield/src/{dns,udp,sni,tun}.rs` test modules
- **Status:** open

#### [HIGH] mitm-proxy: response bodies are buffered without a cap when Content-Length is absent

- **Boundary:** untrusted input parsers
- **Attack:** the HTML/image scan path skips buffering when `Content-Length` exceeds
  `body_limit`, but a chunked or close-delimited response carries no `Content-Length`, so
  `content_length.is_some_and(..)` is false and `body.collect().await` reads the whole stream into
  memory. The size check at line 178 runs after the allocation. Any visited site can OOM the proxy
  by streaming a multi-gigabyte chunked response with an HTML or image content type.
- **Evidence:** `packages/mitm-proxy/src/tunnel.rs:164-178`
- **Status:** open

#### [HIGH] mitm-proxy: nothing in the repository owns CA private key generation or its permissions

- **Boundary:** TLS interception and CA
- **Attack:** `TlsState::load` reads `ca.crt` and `ca.key` from `ca_dir` and no code path in the
  repository creates them — the key is produced by hand, out of band, with undefined mode bits and
  undefined ownership. The corresponding root is installed into the macOS System keychain as
  trusted by `CATrust`, so any local user or process able to read that file can mint a trusted
  certificate for any host and MITM every TLS connection on the machine, including the ones this
  project is not intercepting.
- **Evidence:** `packages/mitm-proxy/src/tls.rs:55-59`; no generator anywhere under `packages/`,
  `scripts/`, or `native-modules/`
- **Status:** open — needs an owned generator that creates the key per-install at mode `0600`, plus
  a startup check that refuses to run if the key is group- or world-readable

#### [MED] mitm-proxy: padding a response past the body limit skips scanning entirely

- **Boundary:** untrusted input parsers
- **Attack:** a body over `body_limit` (default 1 MiB) is forwarded without a verdict. The
  fail-open is deliberate and documented in the code, but it is also a one-line evasion: a hostile
  site pads its HTML or image past 1 MiB and the scanner never runs. The trade-off is real —
  blocking unscannable content has its own cost — but the current default makes evasion cheaper
  than detection.
- **Evidence:** `packages/mitm-proxy/src/tunnel.rs:176-178`, default at
  `packages/mitm-proxy/src/tunnel.rs:34`
- **Status:** accepted-baseline — revisit with a streaming or prefix-scan path

#### [MED] mitm-proxy: the plain-HTTP path performs no scanning at all

- **Boundary:** untrusted input parsers
- **Attack:** `forward::forward_http` takes no `ScanHooks`, so URL, body and image scanning apply
  only to CONNECT-tunnelled traffic. Any `http://` resource bypasses every phase.
- **Evidence:** `packages/mitm-proxy/src/forward.rs`
- **Status:** accepted-baseline — already recorded in the root `CLAUDE.md`; carried here so the
  gate sees it

#### [LOW] mitm-proxy: the default CA directory is a relative path

- **Boundary:** TLS interception and CA
- **Attack:** `ca_dir` defaults to `data/ca`, resolved against the proxy's working directory.
  Under `ProxySupervisor` the working directory is whatever `launchd` supplies, so the CA the
  proxy signs with depends on where it was started from — and a process that can control the
  working directory can substitute its own CA.
- **Evidence:** `packages/mitm-proxy/src/cli.rs:37`
- **Status:** open
