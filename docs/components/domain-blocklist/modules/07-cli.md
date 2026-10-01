# 7. `cli` — the pipeline entry point

Part of the [Domain Blocklist plan](../plan.md).

```text
src/main.rs
```

Responsibilities:

- One subcommand that runs the whole pipeline (fetch all pinned sources → license gate → merge →
  canary → liveness-prune → build → publish gates → sign → write output) end to end, intended to
  run on a monthly schedule from CI or a small persistent job runner — never on an end-user device.
- Flags for a dry run (build and run all gates without publishing) and for running against fixture
  sources instead of live fetches, so most of the pipeline is testable without network access.
- Every abort path — fetch failure, license gate, canary failure, any publish gate — exits non-zero
  with the specific reason. Nothing about a refused build should require reading a log to notice.

## Assumption audit

Run 2026-08-15, against the live sources, before implementation. Local-account/version data
trimmed to the decisive value per the skill's rule.

| Claim | Falsifier | Observed | Verdict |
|---|---|---|---|
| `raw.githubusercontent.com` can be pinned to an exact commit SHA (not just a moving branch) and GitHub's REST API returns a clean SPDX license id for a repo | `curl -o /dev/null -w '%{http_code}' https://raw.githubusercontent.com/StevenBlack/hosts/<latest-sha>/hosts`; `curl https://api.github.com/repos/StevenBlack/hosts/license` | `200`; `{"license":{"spdx_id":"MIT"},"sha":"8745f246..."}`. Same shape for `hagezi/dns-blocklists` → `GPL-3.0` | **TRUE** — `SourceConfig.pinned_revision` should be a commit SHA in the URL path, not `master`/`main`; `expected_license`/served-license comparison for these two sources should come from the GitHub license API, not from parsing the fetched document (neither ships an in-band license string) |
| Hagezi's list content lives in a plain-domain-per-line format compatible with `LineFormat::PlainDomain`, at the URL the plan implies (`domains/pro.txt` or similar) | `curl -o /dev/null -w '%{http_code}' https://raw.githubusercontent.com/hagezi/dns-blocklists/main/domains/pro.txt`; enumerate the repo tree | `404` — no `domains/` directory exists. The repo's actual top-level dirs are `adblock/`, `adguard/`, `controld/`, `dnsmasq/`, `ips/`, `rpz/`, `wildcard/`. `adblock/*.txt` is AdBlock filter syntax (`\|\|domain^`) — a third `LineFormat` this parser doesn't have. `wildcard/pro-onlydomains.txt` (200, comment-headed, one bare domain per line, no `*.` markers) is the format that actually matches `LineFormat::PlainDomain` | **PARTIALLY FALSE** — the plan's implied path is wrong; `SourceConfig` for every Hagezi list must point at the `wildcard/<name>-onlydomains.txt` variant specifically, never `adblock/`. Also note: this variant contains zero wildcard (`*.`) syntax in practice, so Hagezi will never emit `ScopeHint::Apex` through the extraction path — every Hagezi entry scopes through `classify_scope`'s own PSL/registrable-domain logic instead, not through wildcard hinting |
| UT1 category downloads (`.tar.gz`) unpack into a `domains` file that is itself plain-domain-per-line text, matching `LineFormat::PlainDomain` and `fetch_source`'s `Vec<u8>` document contract | `curl -o adult.tar.gz https://dsi.ut-capitole.fr/blacklists/download/adult.tar.gz && tar tzf adult.tar.gz && tar xzOf adult.tar.gz adult/domains \| head` | Archive contains `adult/domains`, `adult/urls`, `adult/expressions`, `adult/usage`; `adult/domains` is plain domain-per-line (`0-12kids.com`, …), matching `PlainDomain` | **TRUE for the format, FALSE for the transport shape** — `fetch_source`/`parse_document` assume a fetch hands back one already-plain-text document, but UT1 actually serves a **tar.gz archive with four files**. The CLI's UT1 `SourceFetcher` must decompress and extract `<category>/domains` itself (a `tar`/`flate2` or equivalent dependency, not yet in `Cargo.toml`) before the shared parser ever sees bytes — handing the raw archive bytes to `parse_document` would silently parse to all-`dropped_malformed` garbage rather than erroring explicitly |
| UT1 publishes a single, unambiguous, machine-checkable license for the fetched content | `curl https://dsi.ut-capitole.fr/blacklists/` and inspect the license section | The visible badge and link are CC BY-SA 4.0. The page's raw HTML *also* contains a second RDF block naming CC BY-NC-SA — but it sits inside an `<!-- -->` HTML comment (a stale leftover from the CC badge generator), not live content | **TRUE (BY-SA 4.0), with a fragility trap** — UT1 has no license API; a naive `grep`/regex-based license scraper for the CLI's UT1 fetcher must not match the commented-out NC-SA text, or it will misreport `LicenseChanged`/`LicenseNotAllowed` on every run against a page that hasn't actually changed. Strip HTML comments before matching, or match only the `rel="license"` anchor `href` |
| `hickory-proto` (or an equivalent crate reachable from this environment) exposes message-level access to set the DNSSEC OK (DO) bit on a query and read the `AD` flag / EDE options off a response — required because module 3's `DnsLookup` contract rules out a `getaddrinfo`-style high-level resolver API | `curl -o /dev/null -w '%{http_code}' https://docs.rs/hickory-proto/latest/hickory_proto/op/struct.Edns.html` (DO bit), `.../op/struct.Header.html` (AD flag), `.../rr/rdata/opt/enum.EdnsOption.html` (EDE) | All three `200` | **TRUE, but not the crate's convenience path** — `hickory-resolver`'s high-level `Resolver`/`AsyncResolver` API (simple `lookup_ip`) does not surface these; the real `DnsLookup` impl needs to build/send queries and parse responses through `hickory-proto`'s lower-level message API directly, which is more implementation surface than "add a resolver crate and call `.lookup()`" |
| The Rust crate registry (crates.io) is reachable from this dev/CI environment to add new dependencies (`reqwest`/`ureq`, `hickory-proto`, `tar`, `flate2`) | `curl -A cargo -o /dev/null -w '%{http_code}' https://index.crates.io/hi/ck/hickory-resolver` | `200` (a bare `curl https://crates.io` without a user agent returns `403` — Cloudflare bot-blocking the root page, not a registry outage; the actual index/download endpoints are unaffected) | **TRUE** |
| Outbound plain UDP/53 to public resolvers (1.1.1.1/8.8.8.8/9.9.9.9) and outbound HTTPS to GitHub/UT1 are both reachable from this dev environment, so most of the pipeline is exercisable live rather than only against fixtures | `dig @1.1.1.1 example.com A`; the source fetches above | Both answered normally | **TRUE** |
| A steady ~23 qps DNS sweep rate stays under public resolvers' undocumented abuse thresholds | *(no cheap falsifier — resolver operators don't publish a hard per-IP qps limit, and deliberately testing one via sustained load is itself the abuse this claim is trying to avoid causing)* | Not run | **Unverifiable here.** Already treated as a considered decision in the sourcing doc (¶"Sweep pacing") rather than a fresh claim for this module; flagging only that it remains unmeasured against real sustained load, and the module 3 audit's own deferred item (an aggregate Unknown-rate health signal) is the mechanism that would catch a rate limit being hit in production |

**One more constraint, discovered while starting module 7's implementation, not from a fresh
falsifier run: module 6 (`net-shield` integration) shipped on this branch since this table was
first written, and its `BlocklistArtifact::load` (`packages/net-shield/src/blocklist.rs`) already
defines the on-disk contract module 7 must write to** — `<base>/current/artifact.fst` +
`<base>/current/manifest.bin`, mirrored under `<base>/previous/`, loaded with `current/` preferred
and falling back to `previous/` on a signature/digest failure. `load()`'s own doc comment states
the `version > high-water-mark` rollback check is deliberately **not** its job — that, and the
high-water-mark's own on-disk format, remain undefined and are module 7's to decide. Concretely,
module 7's publish step must: rotate the existing `current/` into `previous/` before writing a new
`current/` (so a rebuild doesn't destroy the fallback slot the previous build populated), write
both files atomically per slot, and track the high-water mark itself (e.g. read the current
manifest's `version` before rotating, refuse to publish a `version` that doesn't exceed it) rather
than inventing an on-disk high-water-mark file format that `net-shield` never reads.

**What this changes about module 7, before writing code:**

- StevenBlack and Hagezi `SourceConfig`s pin a commit SHA in the URL, and their `FetchedSource.license`/`.revision` come from GitHub's REST API (`/repos/{owner}/{repo}/license`, `/commits/{sha}`), not from parsing the fetched list body — neither source carries an in-band license or revision string.
- Hagezi `SourceConfig.url` values must target `wildcard/<name>-onlydomains.txt`, never `adblock/<name>.txt` (wrong format entirely) or a nonexistent `domains/<name>.txt` path.
- UT1's `SourceFetcher` needs a decompress-and-extract step (new `tar`+`flate2` — or one crate covering both — dependency) between the HTTP fetch and `parse_document`, and its own license check is an HTML scrape of `dsi.ut-capitole.fr` that must exclude commented-out markup, sourced from `<https://dsi.ut-capitole.fr/blacklists/>`.
- The real `DnsLookup` implementation is built on `hickory-proto`'s message-level API, not `hickory-resolver`'s convenience lookups — confirm this before scaffolding the CLI's DNS client so the dependency choice isn't revisited mid-implementation.
- Everything above is live-fetchable from this environment, so the CLI's fixture-mode flag is for deterministic tests, not a network-access workaround — a dry run against live sources is a real, runnable verification step, not merely aspirational.

## Net-client assumption audit (run 2026-08-15, before building the real `DnsLookup` + sweep loop in `src/liveness/`)

An assumption-audit round specifically for the real network client and the qps-paced sweep loop,
which module 3 deliberately deferred to `cli` (module 7, unbuilt). The earlier module-7 audit above
verified hickory's existence at the docs.rs level ("TRUE, but not the crate's convenience path");
this round starts implementation and pins the API-surface claim against the **shipped** crate
source, since the crate has moved to 0.26 since that row was written.

| Claim | Falsifier | Observed | Verdict |
|---|---|---|---|
| The resolvable `hickory-proto` exposes low-level message access: build a query with `Message`/`Query`/`Edns` (DO bit set), parse a response with `from_vec`, and read the AD flag, RCODE, TC flag, and EDNS options back | `cargo add hickory-proto@0.26.1`, then read `~/.cargo/registry/src/.../hickory-proto-0.26.1/src/{op/message.rs,op/header.rs,op/edns.rs,rr/record_data.rs}` for the exact names (the crate ships an API, not a promise) | `Message` exposes `pub` fields `metadata`/`queries`/`answers`/`edns`; `Metadata` exposes `authentic_data`/`truncation`/`response_code`; `Edns::option(EdnsCode) -> Option<&EdnsOption>`; `Message::from_vec`/`to_vec`; `RecordType::{A,AAAA,CNAME}`; `Record`/`Query` all readable | **TRUE with three shape corrections** — (a) access is by public field, not getter methods; (b) **RFC 8914 EDE has no native decode** — EDNS option code 15 lands in `EdnsOption::Unknown(15, Vec<u8>)` and the client must decode the 16-bit BE error code + optional text itself (a ~10-line parser, unit-testable with synthetic bytes); (c) **`RData` has no `DNAME` variant** — a DNAME redirection (RFC 6672 §2.2) surfaces in the answer section as a synthesized `CNAME`, so chain detection matches `RData::CNAME` presence and needs no DNAME-specific branch |
| A fresh, normal NXDOMAIN against the default unfiltered resolver (1.1.1.1) carries **no** EDE and no AD, so EDE parsing cannot false-trigger on ordinary negative answers and the `AuthenticatedDenial`-gate trap can't quietly re-engage | `dig +dnssec nonexistent-$RANDOM.com @1.1.1.1` (recipe: confirm the reply arrived first) | `status: NXDOMAIN`, flags `qr rd ra`, EDNS `flags: do`, **no `EDE:` line, no `ad`** | **TRUE** — re-confirms module 3's `.com` NSEC3-opt-out finding and adds no-EDE-on-clean-NXDOMAIN on top |
| Filtering resolvers self-declare with EDE 15/16/17 (the recipe's claim, which the client's `FilteredByResolver` path depends on live) | `dig +dnssec urban.hostafrican.ng @1.1.1.3`; `dig +dnssec malware.testcategory.com @1.1.1.2`; `dig +dnssec pagead2.googlesyndication.com @94.140.14.14` (AdGuard) | All resolve with **NO EDE option present**: 1.1.1.3 gives a signed NXDOMAIN with `ad`, 1.1.1.2 and AdGuard give `NOERROR` with an A answer (0.0.0.0 sink), no `EDE:` line anywhere | **FALSE / not reproducible via these public filters today** — those operators now sink via a 0.0.0.0 A record (which our pipeline reads as `Alive`, the safe direction) or a bare NXDOMAIN rather than self-declaring. Impact on safety: **none** — a resolver that never emits EDE simply never contributes `FilteredByResolver` evidence, and a filtering resolver's NXDOMAIN cannot be `Dead` on one resolver's word alone anyway (corroboration requires two). The EDE *read* path is still unit-tested with synthetic wire bytes; the "does EDE 15/19 occur in the wild today" half is **unverifiable here** and is recorded as such, same class as the qps-threshold row above |
| UDP/53 to public resolvers is reachable from this environment (needed for the client's live `#[ignore]`d smoke tests, and re-confirms the earlier row) | the `dig` probes above actually getting replies (vs timing out) | Every probe above returned a reply | **TRUE** |
| The `net`-gated dependency (feature-flagged `hickory-proto`) builds clean and keeps the default `cargo test` fully offline — the pure decision modules stay network-free per module 3's design | `cargo build --all-features` and `cargo test` (default) after wiring the feature | *(run after implementation)* | **PENDING — code build is the falsifier; reported after the module lands** |

**What these findings change about the net client, before writing code:**

- Read responses through `Message`'s public fields (`metadata`, `queries`, `answers`, `edns`) rather than getter methods.
- Decode RFC 8914 EDE by hand from `EdnsOption::Unknown(15, raw)` — no hickory EDE type exists to lean on; the decoder is a pure function kept next to the module's `is_filtering_ede` logic and unit-tested with synthetic option bytes.
- Chain detection (`NxDomainViaChain`) keys on `RData::CNAME` records in the answer section; no DNAME branch exists in this crate.
- 1.1.1.x's sink-by-0.0.0.0-now means a filtered domain reads `Alive` from those resolvers, which corroborates `Alive` on a genuinely-NXDOMAIN filtered domain — the `Alive`-wins rule keeps this in the safe (keep) direction, matching the plan's false-positives-are-the-price stance. Worth a regression test documenting that a 0.0.0.0 sink answer must NOT become evidence of anything other than `Alive`.

**Adversarial review (2026-08-15):** an Opus pass focused on transient error handling, error
handling/tracing, and performance/parallelization found two critical defects — both live-fetch and
live-DNS-sweep paths panic on completion (a `tokio::runtime::Runtime` dropped inside its own async
context), and the DNS client has no query retries, so the canary's near-certain exposure to one
dropped UDP packet across ~6,000 canary queries per sweep makes a real multi-hour sweep's most
likely outcome a false abort — plus several high/medium findings (an unintended `net-shield`
dependency on a full HTTP stack, silent-by-default logging, an unenforced false-positive gate,
and more). Full findings, reproduced where practical:
[module-7-cli-adversarial-review.md](../archive/module-7-cli-adversarial-review.md) — that file is a **pre-fix
snapshot** as of the date above; see its own note at the top for where each finding's disposition is
now tracked. **Fixed in this same branch's later commits**, per this row's own "module 7 (`cli`) is
now done" text in `CLAUDE.md`: the `Runtime`-dropped-inside-async-context panic (`main.rs`'s
`AlreadyFetched` wrapper now drives the real fetchers' async paths from outside `fetch_source`'s
synchronous trait method, rather than `block_on`-ing from inside an already-running runtime) and the
DNS client's missing retries (`liveness/net.rs`'s `UDP_RETRY_ATTEMPTS`/`UDP_RETRY_BACKOFF`, plus this
module's own later `TOTAL_QUERY_BUDGET` fix for the retry loop's total time bound). The unintended
`net-shield`-on-full-HTTP-stack dependency does not reproduce against current `Cargo.toml`:
`net-shield` depends on `domain-blocklist` with no `cli`/`net` features enabled, so `reqwest`/`tar`/
`clap` are not pulled in. **Neither the live-fetch path nor the live-DNS-sweep path has been run to
completion against real infrastructure** — every verification so far, including this fix pass, has
been fixture-mode/`--skip-liveness`/unit-test only; that gap is unchanged by the fixes above and is
still open.
