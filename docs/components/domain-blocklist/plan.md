# Domain Blocklist — Implementation Plan

The sourcing, merging, liveness, and storage design this crate implements is recorded in
[domain-blocklist-sourcing.md](../../decisions/domain-blocklist-sourcing.md). Read it first — this
plan is the build order, not the argument.

## Current state

`packages/domain-blocklist/` **does not exist**. Nothing has been built.

`net-shield`'s `DomainFilter` (`packages/net-shield/src/radix.rs`) already accepts "a flat slice of
rule strings" at construction time and has no opinion about where those strings come from — it is
currently exercised only with placeholder/test rule sets. This crate is what produces the real
list, offline, as a build pipeline, and hands `net-shield` (and, via `net-shield-ffi`, the Android
DNS path) a signed compact artifact to load instead of a hand-written rule slice.

## Why this is a separate crate, not logic inside `net-shield`

`net-shield`'s job is to evaluate a decision against an already-loaded ruleset at line rate, on
every OS this project targets, including a Windows TUN loop and an Android `VpnService`. The work
this crate does — fetching and parsing three different upstream list formats, normalizing entries,
tracking provenance, running a periodic DNS liveness sweep, building an FST — has a completely
different lifecycle (a periodic offline batch job, not a per-packet hot path) and a completely
different dependency footprint (HTTP client, DNS resolution, an FST builder) that `net-shield`
should not carry into every daemon that links it. Keeping them separate also means `net-shield`
never needs network access itself to answer a filter query — it stays a pure consumer of a file.

## Module 0 — `packages/domain-normalize`, a shared crate, built first

Specification: [modules/00-domain-normalize.md](modules/00-domain-normalize.md).

## Modules to add

### 1. `sources` — per-source fetch and parse

Specification: [modules/01-sources.md](modules/01-sources.md).

### 2. `merge` — normalize, scope, union, provenance, category separation

Specification: [modules/02-merge.md](modules/02-merge.md).

### 3. `liveness` — DNS-only revalidation with a persistent TTL cache

Specification: [modules/03-liveness.md](modules/03-liveness.md).

### 4. `fst_build` — the on-device artifact

Specification: [modules/04-fst-build.md](modules/04-fst-build.md).

### 5. `gates` — the publish gates

Specification: [modules/05-gates.md](modules/05-gates.md).

### 6. `net-shield` integration

Specification: [modules/06-net-shield.md](modules/06-net-shield.md).

### 7. `cli` — the pipeline entry point

Specification: [modules/07-cli.md](modules/07-cli.md).

### 8. `overlay` — a small, fast-cadence tier for urgent additions and removals

Specification: [modules/08-overlay.md](modules/08-overlay.md).

## Implementation order

1. ~~`packages/domain-normalize` — pure, no I/O, tested first and hardest. `normalize()` against IDNs
   (both directions of the UTS #46 → punycode ordering), trailing dots, mixed case, `www.` variants,
   and post-conversion length limits; `classify_scope` against public suffixes, provider suffixes,
   the shared-hosting denylist, and deep hostnames. Every apex-widening bug this design fears is
   caught here or nowhere.~~ **Done.** `normalize()` uses the `idna` crate's UTS46 pass
   (`AsciiDenyList::URL`, `Hyphens::Allow`, `DnsLength::Ignore`, with label/name length validated
   explicitly afterward per RFC 1035 §2.3.4/RFC 5891 §4.4) and `classify_scope()` uses the `psl`
   crate (compiled-in Public Suffix List data — no I/O, no network fetch) plus a caller-supplied
   shared-hosting denylist slice. **`classify_scope` now downgrades to `ExactHost` rather than
   dropping the entry entirely when the normalized domain is itself a public suffix** (`com`,
   `co.uk`, or a wildcard-PSL tenant boundary like `pornslut.cn.st` — structurally the same as
   `someone.blogspot.com`, just one PSL level lower, since the boundary rule that matched was a
   wildcard rather than a plain suffix) — `Apex` is still refused (claiming the whole shared suffix
   would blackhole every other tenant), but the literal string itself can never match anything but
   that one exact query, so there's no scope-widening reason to drop it. A live run against the
   same ~4.78M real merged domains found ~20 entries being dropped this way (`ec2-*.compute-1.
   amazonaws.com`, `pornslut.cn.st`-style wildcard-tenant hosts, bare gTLDs like `xxx`/`adult`);
   after the fix, only an actual `shared_hosting_denylist` match still drops an entry outright
   (renamed `dropped_public_suffix_or_denylisted` → `dropped_shared_hosting_denylisted` to match),
   and the same real merge: `dropped_shared_hosting_denylisted` 20 → 0, entries 4,721,732 →
   4,721,752. 25 tests, including every named case from this plan (`com`,
   `co.uk`, `blogspot.com`, `s3.amazonaws.com` never `Apex`; `someone.blogspot.com` is `Apex`;
   `www.example.com` normalizes to itself and scopes `ExactHost`, distinct from the `example.com`
   `Apex` key). **`AsciiDenyList::STD3` (the initial choice) was replaced with `AsciiDenyList::URL`
   after a live run against ~4.78M real merged domains found ~1,029 entries silently dropped for
   containing an underscore** — DNS-wire-legal (RFC 1035 §2.3.1: labels are arbitrary octets, not
   LDH-restricted; `_dmarc.google.com` is a ubiquitous real example) and, confirmed against the
   WHATWG URL Standard (`beStrict = false` on the URL host parser, "due to web compatibility"), a
   hostname a real browser's address bar actually resolves and navigates to. `AsciiDenyList::URL`
   is the same deny list the URL Standard's own domain-to-ASCII step applies — denies glyphless/
   control characters and URL-structural punctuation (`%#/:<>?@[\]^|`, which is also how an IPv6
   literal's `:`/`[`/`]` stay rejected, unchanged from before) but not underscore. Rerunning the
   same ~4.78M-domain merge afterward: `dropped_normalization_failed` 1029 → 0, all 1,027 distinct
   previously-rejected domains now present in the merged set. Consumed by `domain-blocklist`
   (module 1+, done — see that row) and `net-shield` (module 6, done — see that row). <!-- step: domain-blocklist.domain-normalize -->
2. ~~`merge.rs` — pure functions (the union/provenance merge, scope resolution, category filtering,
   `flag_personal_name`) with no I/O. Test against hand-built `RawEntry` fixtures.~~ **Done.**
   `packages/domain-blocklist` created — `types.rs` defines `SourceId`, `Category` (`Adult`,
   `Gambling`, `Dating`), `ScopeHint`, `RawEntry` and `MergedEntry` ahead of module 1/`sources`
   (unbuilt), since `merge.rs` needs somewhere to import them from. `resolve_scope()` combines
   `domain_normalize::classify_scope` with the parser's `ScopeHint`: `classify_scope == None`
   (public suffix or denylisted) refuses the entry regardless of hint; only a wildcard
   (`ScopeHint::Apex`) whose base is itself eTLD+1 ever produces `RuleScope::Apex`; a **plain**
   entry that happens to literally be a registrable domain is still scoped `ExactHost` — this
   fills a gap the plan's three explicit bullets leave implicit, deliberately mirroring the
   `www.`-stripping section's logic: widening without an explicit wildcard would silently over-block
   exactly the way stripping `www.` would. `merge()` dedupes `RawEntry`s by normalized domain into a
   `BTreeMap`, unioning `sources`/`categories` (never overwriting) and taking the wider scope on a
   collision (`Apex` beats `ExactHost`), with two separate drop counters (`MergeReport`) for
   normalization failures vs. public-suffix/denylist refusals so a parser regression and a PSL
   surprise never look like the same event. `filter_by_category()` is a pure any-of filter over the
   plan's "adult-only build still ships a gambling-flagged domain" rule. `flag_personal_name()`
   matches the plan's own named cases exactly (`red-panda` flagged, `janedoe` missed) — 2–4
   alphabetic tokens split on `-`/`.`/`_`. **An adversarial (Opus) review of this module found four
   real gaps, all fixed:** (1) `merge()` normalizes `shared_hosting_denylist` once up front —
   `classify_scope`'s contract assumes its denylist entries are already normalized, and a
   hand-authored checked-in file is exactly where a stray trailing dot, capitalization, or a
   Unicode-typed IDN would otherwise silently fail to match and reopen the shared-hosting hole the
   parameter exists to close; (2) IP-literal entries (`"0.0.0.0"`) are dropped and counted
   (`dropped_ip_literal`) rather than passed through as an ordinary domain rule — DNS labels are
   digit-legal so `normalize()` accepts an IP literal as itself, and the plan assigns dropping these
   to the unbuilt `sources` module, so this is defense in depth, not a redundant check; (3)
   `merge()`'s output `sources`/`categories` are sorted before returning, since both are logical
   sets and an input-order-dependent `Vec` would make the eventually-signed artifact
   non-reproducible across a source-fetch order CI makes no promises about — `merge_output_does_not_
   depend_on_raw_entry_order` pins this; (4) `flag_personal_name()` rejects any `xn--`-prefixed
   (RFC 3492 §5 ACE) label outright — post-`normalize()` punycode fragments like `xn--mller-kva`
   (`müller`) tokenize into three all-alphabetic pieces and would otherwise systematically
   false-positive across the entire IDN corpus. The review's fifth claim — that `resolve_scope`
   should grant `Apex` to any plain entry that is literally eTLD+1, not just wildcard-hinted ones —
   was investigated and **rejected**: the plan's module 2 text states outright that "merging two
   entries with different scopes takes the wider scope," which is only meaningful if a plain entry's
   individual scope *can* differ from a wildcard entry's for the same domain — exactly what the
   current `ScopeHint`-gated implementation does. 36 tests, including the plan's named cases (a
   domain flagged `adult` by one source and `gambling` by another keeps both categories; the bare
   `example.com`/`www.example.com` pair stays two distinct entries with distinct scopes) and the
   review-driven regression tests above. Not yet consumed by `sources`, `gates`, or `fst_build`
   (modules 1, 3, 4 — all still unbuilt). <!-- step: domain-blocklist.merge -->
3. ~~`gates.rs` — pure functions against synthetic counts and key sets. Built early precisely because
   they are cheap, pure, and are what stops every category of bad build; leaving them for last means
   the first real run has no guardrail.~~ **Done, including an adversarial (Opus) review's fixes.**
   `shrinkage_gate`/`growth_gate` take `prev_count`/`prev_keys` as `PreviousBuild<T>`
   (`None`/`Existing`), not a bare `u64`/`Option` — the review's sharpest finding was that a caller
   who failed to load the previous manifest and a caller correctly reporting a genuine first build
   both produced the same zero/empty value, which silently disabled both gates' percentage checks
   on the easiest mistake to make; `PreviousBuild` makes "no data" an explicit, named variant a
   caller must consciously choose rather than an ambiguous default (only `shrinkage_gate`'s
   `absolute_floor` still gates a first build). Every threshold-taking gate now validates its
   fraction is finite and in `[0, 1]` first — a `NaN` threshold previously made every `>` comparison
   silently `false`, passing the gate unconditionally on a config mistake rather than refusing to
   run.

   `false_positive_gate` is a substantially different design from the first version, not just a bug
   fix: the plan originally specified subtracting a hand-maintained exclusion file from Tranco, but
   Tranco carries no content-category metadata at all (it's a pure traffic ranking) and genuinely
   contains adult sites at real density (~1–2.5%, not "a handful" — measured against live data), so
   an exclusion file large enough to matter would need hundreds of entries reviewed on every re-pin.
   The fix is **cross-source corroboration**: `FalsePositiveHit::is_corroborated()` treats two or
   more independently-maintained sources agreeing a domain is sensitive as sufficient evidence (a
   single source's tag is that source's own file-level curation choice — see the plan's module 1 —
   so one source alone proves less than two agreeing does), and `review_queue()` narrows the actual
   manual-triage surface to just the uncorroborated, not-yet-excluded hits — see the decision doc's
   "cross-source corroboration" section for the full rationale, including the accepted limitation
   that this assumes the sources curate independently, which hasn't been separately verified. Two
   further fixes, both confirmed live before being fixed: `false_positive_hits` now normalizes
   `control_set`/`exclusions` the same way `merged`'s own keys already are (an unnormalized control
   set previously matched nothing and reported a 0% rate against a list blocking 100% of it — the
   exact class of bug `merge.rs`'s own earlier adversarial review already fixed once, reintroduced
   here); and lookup is now a `HashMap` index built once (O(control + merged)) rather than a nested
   linear scan (confirmed to cost ~10¹² comparisons at realistic multi-million-entry list sizes).
   `false_positive_gate` also takes a `min_control_size` floor — an empty or truncated control-set
   fetch previously passed silently, indistinguishable from "measured, and clean." The direct
   string-equality match against `MergedEntry.domain` (not the `Apex`-scope subdomain-covering
   lookup `fst_build`/`net-shield`, modules 4/6, implement at query time) is kept and now correctly
   justified: it's safe specifically because Tranco ranks registrable, pay-level domains, not
   because "it's unlikely to matter."

   `license_gate` gained a `merged` parameter and now fails when a source that contributed entries
   has **no** snapshot at all — the license-only version of this check passed trivially on an
   *omitted* snapshot, which is the cheaper mistake and the one this exists to catch, since module
   1's fetch-time check re-verified here is exactly what should have made an omission unreachable.
   It also fails on duplicate snapshots for one source (ambiguous coverage) and compares licenses
   case/whitespace-insensitively via the new `LicenseId::spdx_matches`, matching SPDX's own
   identifier rule (an earlier exact-string comparison failed *closed*, correctly, but on a
   confusing message that invited "just add the lowercase spelling too" instead of fixing the
   comparison). **Explicitly not yet covered, and tracked as a gap rather than silently dropped:**
   a source's snapshot license drifting from its `SourceConfig.expected_license` pin, and the
   manifest's `output_license` field's own consistency — both need `SourceConfig`/`Manifest`
   (modules 1/4, still unbuilt) to exist before they can be checked. `SourceSnapshot`/`LicenseId`
   were defined in `types.rs` ahead of `sources` (module 1) the same way `RawEntry`/`MergedEntry`
   were ahead of it for module 2.

   Every gate returns `GateResult::Pass`/`Fail(String)` — the failure message names the evidence
   (which control domains hit and under which sources, which license and source, the measured
   percentage against the limit, capped at 20 named hits with a count of the remainder) rather than
   a bare boolean, per the plan's "refuses publication and requires explicit human sign-off"
   framing. 80 tests. Not yet consumed by `cli` (module 7, unbuilt) — the pipeline that would call
   these gates in sequence and act on a `Fail` doesn't exist yet. <!-- step: domain-blocklist.gates -->
4. ~~`sources/` — one parser per source against small fixture files first (a few lines of each
   source's real format, not a live fetch), covering every row of the input-shape table above, then
   wire in the real pinned `SourceFetcher` HTTP path last.~~ **Done, minus the real HTTP client.**
   `sources/mod.rs` holds what all three parsers share: `SourceConfig` (checked-in fetch
   coordinates — `source`/`url`/`pinned_revision`/`expected_license`), the `SourceFetcher` trait,
   and `fetch_source()`, which checks a fetch's served revision against the pin, its served license
   against `expected_license` (catching a silent relicense even when the new license would itself
   be allowlisted — `FetchError::LicenseChanged`, distinct from `LicenseNotAllowed`), and its
   license against the build's allowlist, in that order, before any parser sees the bytes — every
   failure is a `FetchError` variant meant to abort the whole build, never to be caught and skipped,
   per the plan's "a failed fetch aborts the whole build" rule. Parsing is one shared
   `parse_document()` taking a `LineFormat` (`HostsFile` or `PlainDomain`, the only two native
   formats these three sources ship in — hosts-file syntax differs from plain domain-per-line only
   in whether an address column precedes the domain); `stevenblack.rs`/`hagezi.rs`/`ut1.rs` are thin
   wrappers over it naming their `SourceId`/`Category`/`LineFormat`, matching the plan's one-file-
   per-source layout. Implements every row of the input-shape table: comment/blank skip (including
   an inline `# note` trailing a domain), hosts-format address-column discard, wildcard
   (`*.example.com`) base extraction with `ScopeHint::Apex`, and bare-IP-literal drop-and-count
   (`ParseReport::dropped_ip_literal`) — no normalization happens here, per the plan, since that
   stays `domain_normalize::normalize`, applied once in `merge` (module 2). UT1's `category`
   parameter is supplied by the caller per directory, since UT1 gives no in-band signal of which
   category a `domains` file belongs to; `ut1::count_urls_entries()` is the pure counter the plan's
   "documented coverage limitation, not a silent drop" rule calls for, so the size of the skipped-
   `urls` gap stays visible in the build's metrics without this crate ever extracting a URL entry.
   19 new tests (99 total). The real pinned HTTP `SourceFetcher` implementation, and UT1's per-
   category `SourceConfig` list, are left to `cli` (module 7, unbuilt) per this module's own text
   ("the pipeline binary wires in a real HTTP client") — nothing here needs live network access to
   test. Not yet consumed by `liveness`, `fst_build`, or `cli` (modules 3, 4, 7 — all still
   unbuilt). <!-- step: domain-blocklist.sources -->
5. ~~`liveness.rs` — `due_for_check` first as a pure function against a fake clock and fake cache
   entries with cadence ≠ TTL (this is the part with the trickiest edge cases: reappeared-stale vs.
   genuinely-revived entries). Then `canary_check` against a fake resolver that simulates a
   family-filtering resolver, a wildcard-sink resolver, and a healthy one — all three must be
   distinguishable. The real DNS lookup is a thin, separately-tested edge behind a trait so none of
   this needs live network access to test.~~ **Done, minus the real DNS client, persistent cache
   I/O, and the qps-paced sweep loop — all left to `cli` (module 7, unbuilt), the same split
   module 1 draws for its real HTTP fetcher.** `LookupResult`/`UnknownReason` model one
   single-QTYPE answer; `Verdict` is the combined per-domain outcome `combine()` produces from one
   A and one AAAA `LookupResult` via `check()`, per the plan's three-step ordering (either
   resolving → `Alive`; both `NxDomain` → `Dead`; otherwise, at least one `Unknown` and neither
   `Resolved` → `Unknown`, covering the named mixed `NxDomain`/`Unknown` case explicitly). Real
   lookups go through the `DnsLookup` trait, exactly as `SourceFetcher` does for module 1's HTTP
   fetch — nothing here needs live network access to test. `due_for_check(cache_entry, now,
   ttl_seconds)` takes only the TTL, not a cadence parameter: cadence is the caller's coarser
   scheduling concern, deliberately kept out of this signature so a caller can't conflate the two
   the plan is careful to decouple; `now.saturating_sub(last_checked)` reads clock skew (`now`
   before `last_checked`) as "just checked" rather than underflowing into a bogus multi-decade gap
   that would read as due regardless of TTL. `should_prune()` is the one-line rule "only `Dead`
   prunes," tested separately for first-seen-`Unknown` vs. previously-cached-`Unknown` per the
   plan's own split. `canary_check()` runs every `alive_controls` entry then every `dead_controls`
   entry through `check()` and returns `CanaryResult::Failed { detail }` naming which control
   domain misbehaved and what was expected — distinguishing a family-filtering resolver (an alive
   control comes back `Dead`), a wildcard-sink resolver (a dead control comes back `Alive`), and
   an inconclusive resolver (a control comes back `Unknown`, which fails the canary even though a
   real sweep would keep an `Unknown` domain, since the control set is supposed to be
   unambiguous) as three distinct, separately tested failures; it reports the mismatch only —
   discarding the sweep and refusing to write the cache back on any canary failure is `cli`'s job,
   per the plan. **`dead_controls` is a `Vec`, symmetric with `alive_controls`**, not a single
   `String` — a lone dead control is one resolver quirk away from a false pass (e.g. a rewrite
   rule scoped to one reserved name and not another), and running the full set costs nothing extra
   since `canary_check` already loops. 30 tests (129 total). Not yet consumed by `fst_build` or
   `cli` (modules 4, 7 — both still unbuilt). **Five items are explicitly deferred design decisions
   for `cli` (module 7, still unbuilt), not gaps silently dropped from this module.** An
   interleaved canary — the canary only runs once at t=0 before a ~24h sweep, and proving the
   resolver was honest at t=0 doesn't prove it stayed honest for the whole window — is deferred
   because `CanaryResult` has no timestamp field for `cli` to build the interleaving on top of yet,
   and that's worth deciding before module 7 rather than patching in here. RFC 8914 Extended DNS
   Errors (https://www.rfc-editor.org/rfc/rfc8914) — a filtering resolver often self-declares via
   EDE 15/16/17 (Blocked/Censored/Filtered) alongside its NXDOMAIN, the strongest available signal
   for exactly the threat `canary_check` guards against — is deferred because `LookupResult` has no
   variant for it yet. An aggregate "Unknown rate" health signal is deferred: at sustained
   real-world query rates a public resolver may start rate-limiting, turning every rate-limited
   query into `Unknown(Timeout)` (kept, by design), which could let a whole sweep silently become a
   no-op with nothing in `gates.rs` positioned to catch it, since `gates.rs`'s shrinkage gate
   watches entry counts, not per-domain Unknown rate. Keeping `DnsLookup` synchronous versus
   switching it to async is deferred as a real API-stability tradeoff — it's `pub`, re-exported
   from `lib.rs`, and both `check`/`canary_check` take `&R: DnsLookup` — that should be decided
   deliberately before module 7 is built rather than discovered mid-implementation, given the
   plan's own pacing design (many in-flight concurrent queries at a target qps, above) points
   toward an async-first client like hickory-resolver. RFC 8020
   (https://www.rfc-editor.org/rfc/rfc8020, "NXDOMAIN: There Really Is Nothing Underneath") — an
   NXDOMAIN at `example.com` implies every `*.example.com` entry is also dead, with zero extra
   queries — is deferred because nothing in this module's shape expresses shared context between
   per-domain `check()` calls, which a real optimization lever for module 7's per-domain sweep
   would need. <!-- step: domain-blocklist.liveness -->
6. ~~`fst_build.rs` — pin against a small hand-built key set first (a handful of domains sharing
   prefixes and suffixes, mixed `Apex` and `ExactHost`) and assert exact-match, label-boundary
   scoping, and **anchored** prefix-streaming behavior before building the real list.~~ **Done.**
   `reverse_key()` reverses a normalized domain's labels (`example.com` → `com.example`), keeping
   an `ExactHost` `www` key distinct from the apex key so the consumer's label-boundary lookup
   never conflates them; `assign_provenance()` enumerates the distinct `{sources, categories,
   scope}` combinations across the merged entries into a **deterministically sorted**
   `provenance_table` and maps every reversed key to its combination's compact `u32` ID (a
   `BTreeMap` gives the sorted, deduplicated key set `fst::MapBuilder` requires for free);
   `build()` builds the `fst::Map` from key → provenance ID, computes the SHA-256 `fst_digest` of
   the `.fst` bytes, assembles the `Manifest`, signs the manifest's **signable bytes** (the
   manifest with its own `signatures` field emptied, so adding a second signature during rotation
   can't invalidate the first) with every `(KeyId, SigningKey)` pair via `sign()`, and reports
   measured `bytes_per_entry` in `FstBuildReport`. `FstBuildError` aborts on any FST/serialize
   failure and refuses a build with no signing keys (`NoSigningKeys`) — a build with no signer
   must never publish. `verify_manifest()` mirrors the decision doc's client rule (verify against
   any one `{key_id, signature}` entry whose `key_id` is in the trusted set) and is used to
   round-trip the output in tests. `Manifest`/`ProvenanceEntry`/`Signature` carry serde derives
   (the 64-byte `Signature.bytes` gets a manual `Serialize`/`Deserialize`, since this serde build
   doesn't implement the traits for `[u8; 64]`), and `RuleScope`/`SourceId`/`Category`/
   `LicenseId`/`SourceSnapshot` gained the serde/`Ord` derives needed to encode them. 14 tests,
   covering label-boundary scoping, a two-category domain surviving merge into one provenance
   combination, manifest bincode round-trip, a two-signature rotation manifest verifying against
   old-key-only/new-key-only/both clients, and tampering with `fst_digest` after signing
   invalidating every signature entry. Not yet consumed by `cli` (module 7, unbuilt) — the
   pipeline that would call `build` and act on the manifest doesn't exist yet. <!-- step: domain-blocklist.fst-build -->
7. ~~`cli` — wire the whole pipeline together; the first real run is a dry run against the three
   fixture sources, not a live fetch.~~ **Done, with the real qps-paced sweep's production hardening
   (24h pacing at real scale, a genuinely random per-run nonce, a vetted in-category canary control)
   left as explicit follow-up — see below.** `packages/domain-blocklist/src/main.rs` (+ `cli.rs`,
   `cache_store.rs`, `slots.rs`, `sweep.rs`, and `src/fetchers/{github,ut1}.rs`) run the whole
   pipeline end to end and were verified **live, at runtime, not just compiling**: a fixture-mode
   dry run passes every gate; a real (non-dry) publish writes `current/{artifact.fst,manifest.bin}`;
   a second run correctly rotates the first build into `previous/`, auto-increments `version`, and
   passes `shrinkage_gate`/`growth_gate` against the loaded-and-verified previous manifest; and a
   deliberately shrunk fixture set correctly fails `shrinkage_gate` and **leaves the previously
   published `current/` untouched** (confirmed by file mtimes) rather than partially overwriting it.
   `src/fetchers/github.rs`/`ut1.rs` are the real `SourceFetcher`s the module's own assumption audit
   found were needed — a bounded-retry `reqwest`/rustls GitHub client (git-database commit-SHA
   revision resolution, the license API's `NOASSERTION` sentinel, `X-RateLimit-Reset`-bounded
   403 handling) and a UT1 tarball fetcher (in-memory `flate2`/`tar` extraction of `<category>/
   {domains,urls}`, and an HTML-comment-stripping `rel="license"` scraper so the page's dead,
   commented-out CC BY-NC-SA badge can never be read as the live CC BY-SA 4.0 license) — both
   reworked from an earlier drafting pass's scratch files with no functional changes needed beyond
   fixing their import paths for this crate's real module layout; all of both files' own tests pass
   unmodified. `src/liveness/net.rs` is the real `DnsLookup`, gated behind a new `net` Cargo feature
   so `cargo test` stays fully offline by default (verified: `cargo test`, default features, is 100%
   network-free; `cargo build --all-features` and `cargo test --all-features` are both green) —
   built directly against `hickory-proto` 0.26.1's message-level API (`Message`/`Query`/`Edns`
   public fields, DO-bit query construction, hand-decoded RFC 8914 EDE from `EdnsOption::
   Unknown(15, _)`, TC=1 UDP→TCP retry per RFC 1035 §4.2.1/RFC 7766, and CNAME-chain walking capped
   at 16 hops with cycle detection — DNAME is not a distinct case here, since this crate surfaces a
   DNAME hop as a synthesized CNAME, exactly as the plan's own net-client audit found) rather than
   the high-level `hickory-resolver` convenience crate the `DnsLookup` trait's contract rules out.
   `src/sweep.rs` drives it with qps-paced, concurrency-bounded dispatch (`futures::stream::
   buffer_unordered` gated by a per-domain start-time schedule), corroboration across two
   independently-configured resolvers, canary re-checks between chunks (aborting and discarding the
   **entire** sweep's results on any failure, never a partial commit — the plan's own "no way to
   know when a lying resolver started lying" rule), and hysteresis/`first_dead_at` bookkeeping
   exactly as `CacheEntry`'s doc comment assigns to this module. `src/cache_store.rs` persists the
   liveness cache as hand-written serde DTOs with an exhaustive, compiler-checked match to and from
   the real `CacheEntry`/`Verdict`/`UnknownReason` — deliberately **not** derived directly on those
   types, since doing so would mean editing an already-shipped pure module's file for a concern that
   belongs to this caller. `src/slots.rs` mirrors `net-shield::blocklist`'s two-slot layout **by
   value** (the dependency runs the other way: `net-shield` depends on `domain-blocklist`, not the
   reverse) — cold-start load hard-aborts (never silently reports "no previous build") on a
   signature or digest failure, and publish rotates `current/` → `previous/` before an atomic
   temp-then-rename write per file. One real integration bug found and fixed while wiring this up,
   not by a test: `gates::license_gate` requires exactly one `SourceSnapshot` per `SourceId`, but
   this pipeline fetches UT1 as three separate per-category tarballs (adult/gambling/dating) that
   all carry the identical `SourceId::Ut1` — `main.rs`'s `collapse_snapshots_by_source` merges them
   into one snapshot per source (asserting every grouped snapshot agrees on license, joining their
   individually pin-verified revisions) rather than changing `license_gate`'s own, correctly strict,
   one-entry-per-source invariant. **Left explicitly undone, not silently skipped:** (1) ~~the
   sweep's qps pacing is dispatch-time (a domain-check *starts* every `1/qps` seconds, concurrency-
   capped at `--concurrency`) rather than a raw-query-count limiter, and canary re-checks happen at
   `--canary-every`-sized chunk boundaries rather than continuously interleaved — both are the
   plan's own stated starting-value shape, but neither has been run at the plan's real ~1,000,000-
   domain, 24-hour scale, only against small fixture sets~~ **Partially measured, 2026-08-16 — see
   `docs/decisions/domain-blocklist-sourcing.md`'s "Measured 2026-08-15/16" section for the full
   numbers.** The real merged corpus is already ~4.75M domains from under half the planned source
   list (not ~1,000,000), so the original `~23 qps`/24h sizing is stale on corpus size alone. The
   `canary_every`-chunk-boundary concern was tested directly and cleared at the shipped default
   (`--canary-every 2000`: 1.03x wall-clock overhead against a real dead/lame-domain sample) — it
   only showed up (1.83x) at an artificially small `--canary-every 50` used for a quick local test.
   A naive qps bump sized to the corpus-growth factor (`--qps 57.5`, ~5x) was tested against a real
   corpus sample and produced a real degradation signal (`Unknown` rate 0.7%→10.6%, mostly `Timeout`
   and cross-resolver `UncorroboratedDead`, with local resource exhaustion checked and ruled out) —
   still not run at the true ~4.75M-domain/24h scale, and the CLI's qps/concurrency defaults are
   deliberately left unchanged pending an incremental, canary-monitored ramp on the real deployment
   host rather than a single guessed jump; (2) `--signing-key`/`--trust-key` take a
   `key_id:path-to-32-byte-file` argument, not the `--signing-key-env` comma-separated-hex CI-secret
   design a drafting pass's notes recommended — a real deployment wiring this into CI should add
   that env-var path rather than writing key material to a file on the runner; (3) the nonce dead
   control `liveness::nonce_dead_control` exists and is tested, but nothing in this binary generates
   a random nonce and threads it through `--canary-dead` automatically — an operator must do that
   themselves per run; (4) the vetted in-category alive control the module 3 canary doc comment
   names as the single most dangerous unclosed gap (a resolver that filters only the swept category
   passes every control this design can currently supply) is still not sourced — this crate's own
   content-policy conventions forbid fabricating or checking one in, so it remains a deployment-time
   data-sourcing decision, exactly as `liveness::canary`'s own doc comment already flagged; (5)
   ~~the real DNS client's live behavior was verified only through its 12 offline unit tests against
   hand-built `Message` fixtures — no `#[ignore]`d live smoke test against a real resolver over the
   network was added~~ **Done, in a follow-up pass.** `sweep::tests::live_sweep_smoke_test_against_
   real_resolvers` (`#[ignore]`d, `cargo test --bin domain-blocklist --features cli,net --release
   -- --ignored`) runs one real sweep — real UDP/53 to 1.1.1.1/8.8.8.8, real canary corroboration,
   real qps pacing — over a handful of domains in ~1 second, precisely so this exact code path can
   be exercised without waiting on a 24-hour production run. **It caught a real, 100%-reproducible
   defect on its first run**: `HickoryDnsLookup::lookup_async` built its query `Name` via
   `Name::from_ascii(domain)`, which leaves `is_fqdn` false for a bare domain string (no trailing
   dot); `hickory_proto::rr::Name`'s `PartialEq` treats that flag as significant, so
   `check_response_echo`'s echoed-name comparison rejected **every real DNS response, from every
   domain, unconditionally**, as a "mismatch" — meaning the live client had never completed a
   single successful lookup, on any prior run, offline test, or review, despite passing every
   fixture-based unit test (which never exercises a wire-decoded `Name`). Fixed with one
   `name.set_fqdn(true)` before the query is sent — a question name on the wire is always fully
   qualified per RFC 1035 §4.1.2 regardless of how the caller typed it, so the distinction the flag
   exists for (relative vs. absolute, for a stub resolver's search-list behavior) is moot once a
   name is about to be sent as one specific query. `cargo test --all-features` (253
   domain-blocklist + 92 net-shield tests) stays green; (6) the license
   allowlist a real run needs (`--allow-license`) defaults, with a loud warning, to an unratified
   starting set (MIT, CC0-1.0, GPL-3.0, CC-BY-SA-4.0) — ratifying that list, including whether
   copyleft (GPL-3.0) inputs are acceptable for this project, is a human licensing decision this
   code deliberately does not make silently; (7) ~~the sweep held its entire cache in memory and
   wrote it to disk exactly once, at the end~~ **Done, in a follow-up pass, prompted by a real
   ~14-hour VPS run with no checkpointing.** `sweep::CheckpointSink` + `--checkpoint-every` (default
   10,000, rounding up to the next `--canary-every` chunk boundary, since only a canary-validated
   chunk is trustworthy enough to persist) periodically flushes the accumulated cache via
   `cache_store::save` mid-sweep, so a crash loses at most one interval instead of the whole run — a
   restart resumes for free through `due_for_check`'s existing TTL logic, no separate resume state
   needed. A checkpoint failure aborts the sweep rather than logging and continuing. Also fixed in
   the same pass: `due_domains` was a second full-corpus `Vec<String>` cloned alongside `entries`,
   roughly doubling the resident set at multi-million-domain scale — replaced with a `Vec<usize>` of
   indices, cloning only one chunk's worth of domain strings at a time. Holding the full merged
   `Vec<MergedEntry>` in memory for the sweep's duration (~500-600MB at the measured 4.75M-domain
   corpus size) is unchanged and not yet addressed — paging it from an on-disk intermediate file
   would also require `build()`/`gates.rs`/`review_queue()` to stop assuming a resident `Vec`, which
   is a larger change than this pass's scope. **That follow-up is done, in a second pass**:
   `entries_store.rs` writes the merged corpus to a length-prefixed bincode-per-record scratch file
   right after merge, and `sweep::run_sweep_streaming` reads due-domain batches from it
   `canary_every` at a time (never pre-collecting a full due list) plus a second reopened pass for
   final pruning — sufficient because `entries` only ever needs sequential access, never random,
   so no index or mmap is needed, just a plain reopened file reader. `main.rs` writes and drops
   `entries` before the sweep starts and only reconstructs it (filtered by `pruned_domains`)
   afterward, for the fast gates/build phase. `run_sweep`/`run_sweep_with_resolvers` are kept
   unchanged for tests and small-corpus callers, sharing the per-batch DNS/cache logic
   (`process_batch`) with the new streaming driver so the two can't silently diverge; a new test
   asserts they produce byte-identical outcomes over the same input. **Measured, and the honest
   result is smaller than expected**: a fresh-process stress test shows RSS barely moves
   immediately after writing `entries` to disk and dropping it (516.1MB vs. 515.9MB) — `drop()`
   returns memory to the process allocator, not the OS, and macOS's `malloc` doesn't eagerly return
   millions of small fragmented `String` allocations, so the freed space is mostly silently reused
   by the cache's own later growth rather than showing as a visible RSS drop. A real but modest
   ~86MB reduction in final RSS (1960MB → 1874MB) was measured across two independent fresh
   processes — not the ~516MB a naive "the Vec is gone" story predicts. This is very likely a
   macOS-allocator artifact (Linux's `ptmalloc` typically returns large freed regions more
   eagerly) but that is an unverified claim about the real deployment host, not a measured one.
   What's guaranteed regardless of allocator behavior: no live reference to the full corpus exists
   during the sweep's duration, a structural fact independent of what `ps` reports. The cache
   remains the dominant driver of the ~1.9-2GB peak either way — an embedded KV store bounding
   cache RSS by a page budget instead of corpus size (replacing the `HashMap` +
   whole-file-bincode design) is the real lever for that number. ~~Now spec'd, not yet built~~
   **Built.** `docs/decisions/domain-blocklist-sourcing.md`'s "Measured 2026-08-16: the cache's
   in-process representation — redb, not a `HashMap` blob" section measured the actual production
   VPS (RAM, real `O_DIRECT` disk-latency percentiles, real corpus) and a real redb file built from
   the project's actual pinned sources (4,767,348 domains → 515MB compacted, vs. 2.71GB observed
   live for the then-current in-memory `HashMap`). `cache_store::CacheStore` is now the redb-backed
   store that section speced: a single-file table (domain → bincode `CacheEntryDto`, the same DTO
   `load`/`save` already used, kept unchanged for tests/small-corpus callers) behind a new
   `CacheBackend` trait, with `set` buffering into an in-memory pending map and `flush` committing
   one batched write transaction — sorted by key first, the free B-tree-locality win the plan named
   — rather than one transaction per domain. **Correction: redb is not mmap-backed** — it keeps its
   own in-process page cache (`redb::Builder`, 1 GiB default), which is anonymous process heap the
   kernel can only swap, not drop the cheap way it drops a clean mmap; `CacheStore::open` now passes
   `set_cache_size(cache_store::DEFAULT_CACHE_SIZE_BYTES)` (128 MiB, see that constant's own doc
   comment) explicitly rather than taking the unbounded default, which is the actual mechanism that
   bounds this cache's RSS — see the sourcing doc's corrected redb section for the full story and
   why the earlier mmap-reclaim framing was wrong. `sweep::run_sweep_streaming` (the function
   `main.rs` calls for a real run) is now generic over `CacheBackend`, so `main.rs` passes a real
   `CacheStore` there instead of a resident `HashMap`; checkpointing is `cache.flush()` at each
   `canary_every` boundary plus one **unconditional** final flush — no longer gated on
   `checkpoint_every > 0` (a real bug found and fixed: that gate made `--checkpoint-every 0` on a
   real run silently buffer every verdict since the sweep began and never reach disk, since the "a
   dry run never persists `--cache`" guarantee is already enforced upstream, by `main.rs` choosing an
   in-memory cache for a dry run rather than a real `CacheStore` at all). **On-disk migration was
   decided, not discovered mid-migration, per this section's own instruction**: no automatic
   bincode→redb converter was built, because no real production `cache.bin` exists to migrate — a
   deployment switching to this starts one cold sweep. ~~One real narrowing this decision costs: a
   dry run can no longer cheaply seed itself from an existing cache file's exact prior state
   (`CacheBackend::for_each` has no key-yielding variant to reconstruct a `HashMap` from), so a dry
   run now always starts cold.~~ **Closed in the adversarial-review pass below**: a new
   `cache_store::read_snapshot(path)` reads an existing redb file read-only (never creates it,
   never opens a write transaction) into a plain `HashMap`, and `main.rs`'s dry-run branch now
   seeds `LivenessCache::Memory` from it when `--cache` is given, restoring the "sees exactly the
   gate decisions a real run would" guarantee `--dry-run`'s own doc comment makes. **Measured
   against the same 4.8M-synthetic-domain stress harness the streaming-corpus pass
   above used, same machine, same process shape**: `HashMap`-backed streaming final RSS 1353.8MB;
   redb with the unbounded 1 GiB default, 1126.7MB (a modest ~17% reduction the sourcing doc
   originally, and wrongly, attributed to mmap reclaim); redb with the fixed 128 MiB bounded cache,
   **942.8MB** (cache file still 514.0MB on disk, unchanged — this is a resident-memory fix, not a
   format change). See the sourcing doc's corrected section for a separately-reproduced run that
   measured a larger gap (down to 613.9MB) under conditions not fully pinned down between the two
   runs — both agree the bounded-cache-size fix is real and correctly targeted, neither is confirmed
   against the real VPS yet. Periodic compaction cadence for steady-state churn remains this
   section's own named open gap — still unmeasured, not built here. **The same review also closed
   a silent-error gap in `CacheStore::get`**: four chained `.ok()?` calls previously collapsed a
   real error (a transaction/table failure, an undecodable row) into the same `None` a genuine
   cache miss returns, which mattered concretely in `sweep::process_batch` — a transient read
   error on an already-`Dead` domain silently reset its `first_dead_at` quarantine clock,
   indistinguishable from a fresh death. `get` now logs and counts real errors distinctly (a new
   `AtomicU64 error_count` field, exposed via `CacheStore::error_count()` and logged as
   `cache_errors` on the "liveness sweep complete" line) while leaving the fallback behavior
   (treat as due/unknown) unchanged for every caller. <!-- step: domain-blocklist.cli -->
 8. ~~`net-shield` integration — the precedence table and the shared `normalize()`, per module 6.~~
    **Done.** `packages/net-shield/src/blocklist.rs` — `BlocklistArtifact` (a `fst::Map<Mmap>` that
    owns the mapping, avoiding the plan's `Arc<Mmap>`-plus-`Map` self-reference while keeping the
    reference-counted/reclaimable property), `load()` implementing the two-slot (`current/`→`previous/`
    fallback) cold-start verification of signature then `fst_digest` then FST validity, and
    `lookup()` doing the decision doc's scope-aware label-boundary exact-`get` lookups (never prefix
    streaming, so `examplezzz.com` can't match an `example.com` apex). `DnsShield` now resolves the
    module-6 precedence table (allowlist → explicit `DomainFilter` rules → FST → default) and
    normalizes the query with `domain_normalize::normalize` — the *same* function the build uses.
    `DomainFilter` gained `rule_for()` (distinguishing an explicit `Proxy` rule from the trie's
    miss-default, which `lookup()` alone cannot) while `from_rules`/`lookup`/existing tests stay
    untouched. The lookup-budget worker (`BlocklistLookup`) owns the artifact on one thread, answers
    over a bounded channel keyed by the normalized domain, waits at most a caller-set budget (plan's
    starting value 2 ms), falls back to a small LRU then the default on expiry while counting the
    event, and deduplicates in-flight lookups. **Two explicit narrowings, recorded in
    `docs/engineering/coverage.md`:** (1) the SNI/Wintun path (`NetShield::process_packet`) still
    uses `DomainFilter` alone — the FST is wired into the DNS path only, which is the plan's own
    budget-section target; (2) `load()` has only ever met the two-slot layout written by this
    module's test helper, never by module 7 `cli` (unbuilt), so the layout contract is defined and
    tested against but not yet produced by the real pipeline. 25 new tests (22 in `blocklist.rs`
    covering load verification/fallback, label-boundary scoping, precedence, the budget worker, and
    end-to-end DNS, plus precedence tests; net-shield total 91 as of this pass — see the adversarial
    review below, which adds one more test and brings the final total to 92). Consumed by nothing yet — the
    daemon that constructs a `DnsShield` with a loaded artifact is a later wiring step.

    **An adversarial review round (four parallel angle-scoped passes, one independently re-verifying
    all four) found and fixed two real concurrency bugs in `BlocklistLookup`/`Worker`, neither caught
    by the shipped tests because none of them induce a panic.** `Worker::run` had no `catch_unwind`
    around the FST lookup: a panic there would kill the whole worker thread, silently degrading every
    uncached domain to the level-5 default for the rest of the process's life, and would permanently
    strand the panicking domain's `inflight` waiters (each subsequent `lookup()` call for that domain
    appends another `Responder` that is never drained). Fixed by wrapping the lookup in `catch_unwind`
    — a caught panic now answers that one request as `None` ("no rule") and the worker keeps running
    — plus a `panic_count()` metric, the same "no silently absorbed path" convention the fallback
    counter already followed. Separately, the cache was populated *after* draining `inflight`, not
    before, opening a narrow window for a duplicate worker request on a domain that had just resolved
    — fixed by reordering. Two more findings were documentation-only, not code changes: the review
    confirmed the SNI narrowing above is real and traced (not just asserted), and flagged that
    `DomainFilter::insert` stores rules unnormalized while module 6 queries normalized — pre-existing,
    dormant (no non-test caller builds a `DomainFilter` from untrusted input yet, since `cli` is still
    unbuilt), now called out in `radix.rs`'s own doc comment rather than left implicit. One test-
    coverage gap was closed: `rule_for` returning `None` at a branch node that has children but no
    action of its own (the shape the precedence table's `Some(Proxy)`-vs-miss distinction depends on)
    had no direct test; one was added. Net-shield total now 92. <!-- step: domain-blocklist.net-shield-integration -->
9. **The mmap benchmark** (below) — after there is a real artifact to map. <!-- step: domain-blocklist.mmap-benchmark -->
10. `overlay.rs` — after `fst_build` and `net-shield` integration both exist, since it reuses
    module 4's manifest/signing contract wholesale and slots into module 6's precedence table rather
    than defining either from scratch. Pin the size and publish-size gates against synthetic entry
    sets first, the same way `gates.rs` was built ahead of a real artifact to test against. <!-- step: domain-blocklist.overlay -->

## Benchmarking plan — the mmap major-fault tail

The decision doc's mmap tradeoff (a rare, bounded major fault in exchange for reclaimability) and
the 2 ms lookup budget are **unmeasured**. The point of this plan is that measuring a warm page
cache and calling it a fault is the easy mistake, and the whole question is what a *cold* page
costs.

**Primary measurement: the project's existing Android emulator.**

- Force genuine kernel-level page cache eviction between measurements with root plus
  `echo 3 > /proc/sys/vm/drop_caches`. Nothing short of this measures a real major fault; unmapping
  and remapping, or simply waiting, does not reliably evict.
- Measure the distribution of single-lookup latency on a cold mapping against a real built artifact,
  not a synthetic one — the whole effect depends on where the FST's states physically land in the
  file.
- **Caveat to record with the numbers:** the emulator's virtual disk is host-SSD-backed, so absolute
  I/O time is optimistic against real eMMC or low-end UFS. The *fault mechanism* is authentic; the
  absolute milliseconds are not a floor for real hardware.

**Secondary sanity check: macOS.** `sudo purge` plus an Instruments VM-Tracker pass, used only to
confirm the measurement harness is actually observing faults and not measuring a warm cache.
**Absolute latency from a Mac is not trusted for this question at all** — Mac NVMe is far faster
than budget Android storage, and a good number here proves nothing.

**The decision rule:** a comfortable emulator result validates the 2 ms budget. A **borderline**
emulator result is the trigger to **acquire a real low-end Android device and measure there before
shipping** — not to round the number down and hope. This mirrors the project's existing refusal to
ship a guessed constant.

## Reference documents

### DNS liveness checks

- [RFC 1035 — Domain Names, Implementation and Specification](https://www.rfc-editor.org/rfc/rfc1035)
  — §4.1.1 defines the header's QDCOUNT field (why "one question per message" is a real-world
  resolver convention, not a protocol requirement); §3.2.2 lists the A record type; §4.1.3 the
  RDATA format used to tell a real answer from NXDOMAIN. §2.3.4 gives the 63-byte label and
  255-byte name size limits the normalization step validates against.
- [RFC 3596 — DNS Extensions to Support IPv6](https://www.rfc-editor.org/rfc/rfc3596) — §2.1
  defines the AAAA record type, needed alongside A for a complete liveness signal — and the reason
  a liveness check is **two** queries per domain, not one.
- [RFC 8482 — Providing Minimal-Sized Responses to DNS Queries That Have QTYPE=ANY](https://www.rfc-editor.org/rfc/rfc8482)
  — why `QTYPE=ANY` is not a way to collapse the A and AAAA queries into one.
- [RFC 2606 — Reserved Top Level DNS Names](https://www.rfc-editor.org/rfc/rfc2606) and
  [RFC 6761 — Special-Use Domain Names](https://www.rfc-editor.org/rfc/rfc6761) — the names the
  canary check uses as guaranteed-not-to-resolve controls.

### IDN normalization

The normalization step must be traceable to all four of these, not just the protocol document —
RFC 5891 alone specifies neither the mapping rules nor the encoding.

- [UTS #46 — Unicode IDNA Compatibility Processing](https://www.unicode.org/reports/tr46/) — the
  mapping and case-folding rules applied **before** encoding. Getting the order wrong makes two
  spellings of the same name encode differently, which is a silent duplicate.
- [RFC 3492 — Punycode](https://www.rfc-editor.org/rfc/rfc3492) — the actual U-label → A-label
  encoding. Storage and comparison are always in A-label form.
- [RFC 5892 — The Unicode Code Points and IDNA](https://www.rfc-editor.org/rfc/rfc5892) — the
  code-point eligibility tables that decide whether a label is convertible at all.
- [RFC 5891 — IDNA: Protocol](https://www.rfc-editor.org/rfc/rfc5891) — the protocol framing that
  ties the three together, and the length constraints that must be validated **after** A-label
  expansion.

### Registrable-domain / apex scoping

- [Public Suffix List](https://publicsuffix.org/) — the authority for whether an entry is a public
  suffix, a registrable domain (eTLD+1), or something below one. This is what stands between a
  source entry naming a shared-hosting host and a rule that black-holes every tenant on it. Note
  the list is community-maintained and incomplete, which is why a checked-in shared-hosting
  denylist backstops it.

### Negative control set

- [Tranco](https://tranco-list.eu/) — the research-oriented top-sites ranking used as the
  false-positive control. Pinned to a specific daily release, like any other source.

### FST / succinct data structures

- [`fst` crate documentation](https://docs.rs/fst/) — `Map`/`MapBuilder` API, the sorted-insertion
  requirement, and the `Automaton` trait used for prefix-streaming secondary use cases. Note that
  `Map` outputs inhibit state merging, which is why the provenance-ID space is kept small.
- [`memmap2` crate documentation](https://docs.rs/memmap2/) — the mmap wrapper used for the
  reference-counted, reclaimable on-device loading path described in the decision doc.

## What this does not cover

- **Any list purporting to identify illegal content (CSAM or otherwise)** — out of scope by the
  hard boundary in the decision doc, not a later phase.
- **UT1's path-level `urls` files** — a documented coverage limitation, not a silent drop; see
  module 1.
- **IP-literal blocking** — `net-shield`'s `IpFilter`, fed by its own rule source, not a
  domain-keyed FST.
- **Incremental patching of the bulk `.fst` itself** — the FST ships monolithically and always will;
  `fst::Map` has no edit API and mutating a live mmap concurrently read by lookups is a correctness
  hazard the design avoids rather than solves. What *is* now specified is a small, separately-signed
  fast-cadence tier alongside it (module 8, `overlay`) for additions/removals that can't wait for the
  next bulk rebuild — see module 8 and the decision doc's on-device-storage/distribution sections.
  Bulk pruning's justification is unchanged by this: bounded artifact size and reduced stale-hit
  surface, not smaller downloads.
- **The on-device runtime lookup itself** — that is `net-shield`'s `DomainFilter`
  ([its plan](../net-shield/plan.md)); this crate only produces the artifact `DomainFilter` loads,
  plus the shared `normalize()` both sides use.
- **UI for category selection or the allowlist override** — the allowlist is a local, per-device
  concern layered on top of whatever this pipeline ships. The exact mechanics (does an allowlist
  entry match subdomains, how it's persisted) belong in `net-shield`'s own plan; the precedence it
  sits at is fixed in module 6 above.
- **The operator-facing dispute channel's implementation** — the decision doc requires a public
  general notice and a dispute/rectification channel. Standing those up (a form or contact address,
  the published statement, the record of Art. 21 balancing judgments) is a project-level
  responsibility, not a Rust module. What this crate owes it is the source-level correction file
  module 1 reads, so a rectified entry is not reinstated by the next source refresh.
- **Windows/Android-specific packaging of the artifact** — how the signed `.fst` file reaches an
  installed device (bundled at build time vs. fetched at runtime, per platform), including the
  Android storage path, loader ownership, initial-install behavior, the staleness indicator, and
  last-known-good rollback with its persisted `version` high-water mark, is a distribution concern
  for each daemon/app. It needs a home in `apps/mobile`'s and the mac-daemon's own plans before
  implementation — tracked as a gap here rather than designed here, since packaging decisions for
  one platform don't belong in a cross-platform pipeline's plan.
