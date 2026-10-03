# mitm-proxy claims

The `mitm-proxy.*` rows in [coverage.md](../../engineering/coverage.md), the check that observes each, and
what a pass says nothing about. Contract: [e2e-scenario-contract.md](../../decisions/e2e-scenario-contract.md).

## What the hermetic check proves

`packages/mitm-proxy/tests/hermetic_claims.rs`, layer `ci`: an in-process CA, loopback origins, the hook
wiring `main.rs` ships (`scan::build_hooks`), and a `reqwest`/`rustls` client that shares no code with the proxy.

| Row | Test | Asserts |
|---|---|---|
| `mitm-proxy.https-scan-block` | `https_url_match_*`, `https_body_match_*`, `https_clean_page_*`, `https_image_verdict_*` | 403 `Blocked`, the scanner saw the exact input and returned `block` with a non-zero score, origin hit count |
| `mitm-proxy.protection-mode-text` | `text_verdicts_follow_protection_mode_changes_at_runtime` | Full blocks, WarnOnly and Off pass, Full blocks again on one client |
| `mitm-proxy.scan-body-limit` | `body_exactly_at_the_scan_limit_*`, `body_over_the_scan_limit_*` | Boundary on both sides |
| `mitm-proxy.plain-http-scan` | `known_gap_plain_http_*` + `control_https_blocks_*` | Plain HTTP reaches the origin with the body intact and no hook runs; the same content over HTTPS is 403 |
| `mitm-proxy.image-protection-mode` | `known_gap_image_scan_*` + `control_text_gate_*` | Image is blocked in Off and WarnOnly; text honours Off and the image blocks in Full |
| `mitm-proxy.leaf-aki` | `known_gap_leaf_*` + `control_leaf_*` | The presented leaf has no AKI; the same leaf verifies against the CA and carries the SAN |

A `known_gap_*` test asserts today's wrong behaviour and goes red when the gap is fixed. The fix PR flips
the assertion and moves the row.

## What a pass does not prove

- Anything about a real browser, a real trust store or a real CA install. See `mitm-proxy.firefox-https` (`human`).
- That OpenSSL-based clients reject the leaf. The check sees the missing extension only (`mitm-proxy.leaf-aki`).
- Image classification. The image scanner is a stand-in that blocks one byte string; a model is never loaded.
  The macOS and Android image rows stay `Unverified`.
- Text-policy quality. The proxy is tested against the placeholder dictionary in `scan::build_default_engine`.
- The Android and macOS consumers of `text-policy`. A pass here says nothing about them.
- HTTP/2, pinned clients, or interception under a TUN route.

## Mutation testing

`cargo mutants` runs in `packages/mitm-proxy` against `hermetic_claims` only (config in
`.cargo/mutants.toml`), so a mutant the unit tests happen to kill still counts as missed here. Scope: the scan
hooks and `ProtectionMode` gate (`scan.rs`, `tunnel.rs`), the dispatch (`proxy.rs`, `connect.rs`) and
certificate generation (`tls.rs`). A surviving mutant fails the job unless it is excluded in the config and
listed here.

### Known survivors

| Excluded (`exclude_re`) | Why it is not this claim |
|---|---|
| `replace \* with . in scan_image` | The score is a log diagnostic; no hermetic check loads a model |
| `in TlsState::build_client_config` | Reads the machine's native roots; not hermetic |
| `replace bad_gateway ` | Only reached on an origin body-read error; no check provokes one |
| `replace text -> ResBody` | Body text of the CONNECT replies; no check reads it |

## Owner checklist (three minutes)

1. Open `tests/hermetic_claims.rs` and read the six `known_gap_*` / `control_*` pairs: does each control
   succeed through the same harness as its gap?
2. Read the "does not prove" list above: is anything in it something you were counting on this check for?
3. Read the three `Uncovered` rows: is `Uncovered` still the right word for each?
