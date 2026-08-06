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
tracking provenance, running a monthly DNS liveness sweep, building an FST — has a completely
different lifecycle (a periodic offline batch job, not a per-packet hot path) and a completely
different dependency footprint (HTTP client, DNS resolution, an FST builder) that `net-shield`
should not carry into every daemon that links it. Keeping them separate also means `net-shield`
never needs network access itself to answer a filter query — it stays a pure consumer of a file.

## Modules to add

### 1. `sources` — per-source fetch and parse

```text
src/sources/stevenblack.rs
src/sources/hagezi.rs
src/sources/ut1.rs
src/sources/mod.rs
```

Responsibilities:

- One module per source, each parsing that source's native format (hosts-file syntax for
  StevenBlack, plain domain-per-line for hagezi, UT1's category directory structure) into a
  common `RawEntry { domain: String, source: SourceId, category: Category }`. `category` is
  populated here, not inferred later — it's the one thing only the source-specific parser knows
  (UT1 hands it out per directory, hagezi's NSFW list and StevenBlack's `porn` extension are each a
  single fixed category), and module 2's merge step needs it on every `RawEntry` it consumes.
- Every fetch also produces one `SourceSnapshot { source: SourceId, version: String, license: String, fetched_at: Timestamp }`, independent of the entries themselves — this is the provenance module 2 attaches to the merged output and what module 4's manifest ends up carrying (see below). Provenance is recorded per top-level source pulled directly (StevenBlack/hagezi/UT1); it does not attempt to unwind a source's own further upstream aggregation (StevenBlack's `porn` extension is itself built from smaller lists) — if a source is later found to embed something improperly licensed, the fix is dropping that whole source, not attributing individual entries within it.
- Parsers must have a defined, tested answer for the malformed-input cases every one of these
  formats actually contains: comment lines, bare IP-literal entries (no domain to normalize),
  wildcard entries (`*.example.com`), empty labels, and domains that fail IDN/punycode conversion.
  The default for anything a parser can't turn into a valid domain is to drop the line and count
  it, not to panic or silently mis-normalize it.
- No normalization here — that is a separate, shared step (module 2) so every source is normalized
  identically rather than each parser re-implementing it slightly differently.
- Fetching is behind a trait (`SourceFetcher`) so tests can supply fixture bytes instead of hitting
  the network; the pipeline binary wires in a real HTTP client.

### 2. `merge` — normalize, union, provenance, category separation

```text
src/merge.rs
```

Responsibilities:

- `normalize(domain: &str) -> String` — lowercase, strip a leading `www.`, punycode-normalize IDNs,
  strip a trailing dot. Pure function, the first thing to test: this is what makes "is domain X
  already in the list" a reliable comparison instead of a string-literal accident.
- `MergedEntry { domain: String, sources: Vec<SourceId>, category: Category }` — the union type.
  Merging two `RawEntry`s for the same normalized domain unions their `sources` rather than
  picking one arbitrarily, so a later "why is this blocked" query can answer with every source
  that flagged it.
- Categories are never blended silently: a source's own category (`adult`, `gambling`, `dating`,
  ...) is preserved through merge, and only entries in the categories this build is configured to
  ship are passed on to module 3.

### 3. `liveness` — DNS-only revalidation with a persistent TTL cache

```text
src/liveness.rs
```

Responsibilities:

- `LivenessCache` — a persistent `domain → { last_checked, verdict, sources }` table (a flat file
  or embedded key-value store; no server, this runs inside the pipeline's own storage).
- `check(domain: &str) -> Verdict` — a single DNS A/AAAA lookup. `Verdict` is **not** a boolean:

  ```rust
  enum Verdict {
      Alive,                 // A or AAAA record, or a CNAME chain resolving to one
      Dead,                  // NXDOMAIN
      Unknown(UnknownReason), // NODATA, SERVFAIL, REFUSED, timeout, or a malformed response
  }
  ```

  Only `Dead` prunes a domain from the next build. `Unknown` is left exactly as it was in the
  previous build — a resolver hiccup or a transient SERVFAIL must never be able to silently shrink
  the blocklist. A domain stuck in `Unknown` across repeated checks is a signal worth surfacing in
  the build's own logs/metrics, not a reason to change its inclusion.
  No HTTP fetch, no TCP connection to the domain's own server, no ICMP — see the decision doc's
  legal boundary for why an application-layer request against a listed domain is never made here.
- `due_for_check(cache_entry, now, ttl) -> bool` — pure function deciding whether a cached verdict
  is still trusted or needs a fresh lookup. Default TTL: one month. This is the piece that stops a
  source's own re-published stale entry from forcing an unbounded recheck every cycle while still
  catching a genuine revival eventually.
- Concurrency, not batching: many DNS queries are issued in flight at once (rate-limited against
  the resolver), since a single DNS message answering multiple questions is not something
  real-world resolvers implement despite the wire format nominally allowing it.
- A `Dead` verdict removes the entry from the next build's output; it does not delete it from the
  cache, so a later revival is still detected once its TTL expires rather than being permanently
  invisible.

(See [Reference documents](#reference-documents) below for the DNS response codes this matrix is
built from.)

### 4. `fst_build` — the on-device artifact

```text
src/fst_build.rs
```

Responsibilities:

- Reverses each surviving domain's labels (`www.example.com` → `com.example.www`) so subdomain
  matching becomes prefix/exact-match at label boundaries on the consumer side.
- Sorts the reversed, deduplicated key set (an `fst::MapBuilder` requirement — the merge/dedupe
  step upstream already produces this property, so this is not new work, just an ordering
  constraint on output).
- Builds the FST, mapping each key to a small **provenance ID** (`u32`) rather than building a bare
  set. The provenance ID is only meaningful alongside the manifest entry that decodes it — see
  below — so a future "why is this blocked" surface never needs a second on-device lookup
  structure, just this one plus the small manifest.
- Writes the `.fst` file plus a manifest:

  ```rust
  struct Manifest {
      build_time: Timestamp,
      sources: Vec<SourceSnapshot>,          // one per constituent source, from module 1
      provenance_table: Vec<ProvenanceEntry>, // provenance_id -> { sources: Vec<SourceId>, category: Category }
      fst_digest: [u8; 32],                  // SHA-256 of the .fst file, binding manifest to artifact
  }
  ```

  and signs `(fst_digest || manifest_bytes)` with the pipeline's Ed25519 key — see the decision
  doc's "Distribution" section for the exact trust contract this produces. Round-trip tests cover
  building a small provenance table, encoding it into the manifest, and decoding it back to the
  same `{sources, category}` per ID.

(See [Reference documents](#reference-documents) below for the FST/mmap references this module builds on.)

### 5. `cli` — the pipeline entry point

```text
src/main.rs
```

Responsibilities:

- One subcommand that runs the whole pipeline (fetch all sources → merge → liveness-prune → build
  → sign → write output) end to end, intended to run on a schedule (monthly, matching the liveness
  TTL) from CI or a small persistent job runner — never on an end-user device.
- Flags for a dry run (build without publishing) and for running against fixture sources instead
  of live fetches, so most of the pipeline is testable without network access.

## Implementation order

1. `merge.rs` — pure functions (`normalize`, the union/provenance merge, category filtering) with
   no I/O. Test first against hand-built `RawEntry` fixtures covering the normalization edge cases
   (IDNs, trailing dots, mixed case, `www.` variants) and the provenance-union behavior.
2. `sources/` — one parser per source against small fixture files first (a few lines of each
   source's real format, not a live fetch), then wire in the real `SourceFetcher` HTTP path last.
3. `liveness.rs` — `due_for_check` first as a pure function against a fake clock and fake cache
   entries (this is the part with the trickiest edge cases: reappeared-stale vs. genuinely-revived
   entries). The real DNS lookup is a thin, separately-tested edge behind a trait so the TTL logic
   never needs live network access to test.
4. `fst_build.rs` — pin against a small hand-built key set first (a handful of domains sharing
   prefixes and suffixes) and assert both the exact-match and label-boundary-prefix lookup
   behavior before building the real multi-million-entry list.
5. `cli` — wire the whole pipeline together; the first real run is a dry run against the three
   fixture sources, not a live fetch.
6. `net-shield` integration — teach `DomainFilter`'s loading path to accept the signed `.fst` file
   (mmap-backed, `Arc`-wrapped for atomic swap, per the decision doc) as an alternative to the
   existing flat rule-slice constructor, without changing the existing constructor's behavior or
   its tests.

## Reference documents

### DNS liveness checks

- [RFC 1035 — Domain Names, Implementation and Specification](https://www.rfc-editor.org/rfc/rfc1035)
  — §4.1.1 defines the header's QDCOUNT field (why "one question per message" is a real-world
  resolver convention, not a protocol requirement); §3.2.2 lists the A record type; §4.1.3 the
  RDATA format used to tell a real answer from NXDOMAIN.
- [RFC 3596 — DNS Extensions to Support IPv6](https://www.rfc-editor.org/rfc/rfc3596) — §2.1
  defines the AAAA record type, needed alongside A for a complete liveness signal.

### IDN normalization

- [RFC 5891 — Internationalized Domain Names in Applications (IDNA): Protocol](https://www.rfc-editor.org/rfc/rfc5891)
  — the punycode-normalization step `normalize()` must apply consistently, so two IDN spellings of
  the same domain merge into one entry instead of silently duplicating.

### FST / succinct data structures

- [`fst` crate documentation](https://docs.rs/fst/) — `Map`/`MapBuilder` API, the sorted-insertion
  requirement, and the `Automaton` trait used for prefix-streaming secondary use cases.
- [`memmap2` crate documentation](https://docs.rs/memmap2/) — the mmap wrapper used for the
  reference-counted, reclaimable on-device loading path described in the decision doc.

## What this does not cover

- **Any list purporting to identify illegal content (CSAM or otherwise)** — out of scope by the
  hard boundary in the decision doc, not a later phase.
- **The on-device runtime lookup itself** — that is `net-shield`'s `DomainFilter`
  ([its plan](../net-shield/plan.md)); this crate only produces the artifact `DomainFilter` loads.
- **UI for category selection or the allowlist override** — the allowlist is a local, per-device
  concern layered on top of whatever this pipeline ships, not something the pipeline itself
  renders or stores per-user. The exact mechanics (does an allowlist entry match subdomains, how
  it's persisted, that it's checked *before* the FST lookup so it always wins) belong in
  `net-shield`'s own plan, since that's the crate that actually evaluates a query — this plan only
  guarantees the pipeline never bakes an unconditional block past what a consumer's allowlist can
  override.
- **Windows/Android-specific packaging of the artifact** — how the signed `.fst` file reaches an
  installed device (bundled at build time vs. fetched at runtime, per platform), including the
  Android storage path, loader ownership, initial-install behavior, and last-known-good rollback,
  is a distribution concern for each daemon/app, not for this crate. It needs a home in
  `apps/mobile`'s and the mac-daemon's own plans before implementation — tracked as a gap here
  rather than designed here, since packaging decisions for one platform don't belong in a
  cross-platform pipeline's plan.
