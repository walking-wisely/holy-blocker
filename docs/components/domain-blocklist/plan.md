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

**`normalize()` cannot live only in this crate.** The pipeline normalizes at build time and
`net-shield` must normalize identically at query time; any divergence between the two silently
breaks matching in a way no test on either side catches, because each side is internally consistent.
This is the same failure the mac-daemon avoided by consuming `packages/text-policy-ffi` instead of
writing a third implementation of the mode→action mapping — **one implementation, no drift**. A
second copy of `normalize()` is the wrong call for exactly that reason.

So the first thing built is a tiny, pure, dependency-light crate:

```text
packages/domain-normalize/src/lib.rs
```

- `normalize(domain: &str) -> Result<String, NormalizeError>` — the comparison key.
- `RuleScope { Apex, ExactHost }` and `classify_scope(normalized: &str, psl: &PublicSuffixList,
  shared_hosting_denylist: &[&str]) -> Option<RuleScope>` — the apex-eligibility decision.
- Nothing else. No I/O, no HTTP, no FST, no DNS.

Consumed by `domain-blocklist` at build time and by `net-shield` at query time. `net-shield` picks
up one small pure dependency, which is a far cheaper cost than a normalization mismatch.

The normalization contract, in order (the ordering is not optional — mapping and case-folding must
happen **before** punycode encoding, or two spellings of the same name encode differently):

1. Strip a trailing dot.
2. Apply **UTS #46** mapping and case folding (this subsumes plain ASCII lowercasing).
3. Convert U-labels to A-labels — punycode encoding per **RFC 3492**, with code-point eligibility
   per **RFC 5892**. Storage and comparison are always in A-label form.
4. **Validate lengths after conversion** — a label that exceeds 63 bytes or a full name that
   exceeds 253 bytes *after* A-label expansion is rejected as malformed. Pre-conversion length is
   not a valid check; punycode expands.

**There is no `www.`-stripping step, deliberately.** An earlier draft of this design stripped a
leading `www.` label before comparison, on the theory that it was "just a comparison convenience."
It wasn't: `classify_scope` runs on the *same* string `normalize()` produces, so a stripped
`www.example.com` and a bare `example.com` collapsed onto the identical comparison key and could
not be told apart downstream — a source entry meant to name only the `www` host silently became
indistinguishable from one naming the apex, and a query for the bare domain incorrectly matched a
rule that was only ever supposed to cover its `www` subdomain. `www.example.com` is therefore
normalized, scoped, and stored exactly like any other three-label hostname — it produces its own
`ExactHost` entry that matches `www.example.com` and nothing else. If a source separately lists the
bare `example.com` as `Apex`, that Apex entry already covers `www.example.com` (and every other
subdomain) at lookup time — see [module 4](#4-fst_build--the-on-device-artifact) — so no aliasing
step is needed to get the common case right, and the uncommon case (only the `www` host is listed)
no longer silently over-blocks.

`classify_scope` returns `Apex` only when the normalized entry is exactly the registrable domain
(eTLD+1) per the Public Suffix List; `ExactHost` otherwise; and `None` (refuse the entry as an apex
rule) when the entry *is* a public suffix or appears on the shared-hosting denylist. Test this
first and hard: `com`, `co.uk`, `blogspot.com`, `s3.amazonaws.com` must never produce `Apex`, while
`someone.blogspot.com` must. Also test the `www.` case directly: an `ExactHost` entry for
`www.example.com` must match a query for `www.example.com`, and must **not** match `example.com` or
`cdn.example.com`.

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
  common `RawEntry { domain: String, source: SourceId, category: Category, scope_hint: ScopeHint }`.
  `ScopeHint { None, Apex }` records whether the source line was an ordinary entry or a wildcard —
  see the input-shape table below for how each parser sets it. `category` is populated here, not
  inferred later — it's the one thing only the source-specific parser knows (UT1 hands it out per
  directory, hagezi's NSFW list and StevenBlack's `porn` extension are each a single fixed
  category), and module 2's merge step needs both fields on every `RawEntry` it consumes.
- **Each source is fetched at a pinned revision** — a release tag or commit hash from the checked-in
  source configuration, never a branch HEAD. A `SourceConfig { source: SourceId, url: String,
  pinned_revision: String, expected_license: LicenseId }` is part of the repository, and moving a
  pin is a reviewed pull request. The fetcher verifies the revision it actually got matches the pin.
- Every fetch also produces one `SourceSnapshot { source: SourceId, revision: String, license: LicenseId, fetched_at: Timestamp }`, independent of the entries themselves — this is the provenance module 2 attaches to the merged output and what module 4's manifest ends up carrying. Provenance is recorded per top-level source pulled directly (StevenBlack/hagezi/UT1); it does not attempt to unwind a source's own further upstream aggregation (StevenBlack's `porn` extension is itself built from smaller lists) — if a source is later found to embed something improperly licensed, the fix is dropping that whole source, not attributing individual entries within it.
- **A failed fetch aborts the whole build.** Not a warning, not a skip, not "continue with the other
  two." The error propagates out of the pipeline and nothing is published. See the decision doc for
  why partial coverage that looks like a successful build is the worst available outcome.
- **License gate.** `SourceSnapshot.license` is checked against a checked-in allowlist of compatible
  license identifiers. A source whose current license is not on the allowlist fails the build. The
  build also computes the least-permissive input license and hands it to module 4 as the merged
  artifact's own license.

#### Input-shape handling (defined behavior for every case these formats actually contain)

`RawEntry` carries a `scope_hint` alongside the domain so the parser can express what the source
meant, which module 2 then resolves against the PSL:

| Input | Behavior |
|---|---|
| Comment line, blank line | Skipped, counted |
| `0.0.0.0 example.com` / `127.0.0.1 example.com` (hosts format) | Domain extracted, sink address discarded |
| **Wildcard `*.example.com`** | **Base domain extracted** (`example.com`), `scope_hint = Apex`. A wildcard is meaningful, not malformed — under this design's subdomain-covering semantics it is equivalent to listing the base domain. Discarding it, as an earlier draft of this plan implied, would silently drop real coverage |
| **Bare IP-literal entry** (a hosts line whose "domain" field is an IP) | **Dropped, counted.** IP-based blocking is out of scope for a domain-keyed artifact — that is `net-shield`'s separate `IpFilter`, fed by its own rule source. Stating the boundary is the point; leaving it ambiguous invites someone to stuff IPs into an FST keyed on reversed labels |
| Empty label, leading/trailing dot beyond the single trailing dot, invalid characters | Dropped, counted |
| Fails IDN/punycode conversion, or exceeds DNS length limits after A-label expansion | Dropped, counted |

- **UT1's path-level `urls` files are out of scope.** UT1 ships both `domains` and `urls` per
  category; only `domains` is consumed. A URL path cannot be expressed in a domain-keyed FST, and
  blocking the whole host because one path was listed would be a large, silent over-block. This is
  a **documented coverage limitation**, not a silent drop: the count of skipped `urls` entries is
  reported in the build's metrics so the size of the gap is visible.
- The default for anything a parser can't turn into a valid domain is to **drop the line and count
  it**, never to panic or silently mis-normalize it. Every drop category above is a separate
  counter in the build report, because a parser regression shows up as one counter exploding.
- No normalization here — that is module 0's shared function, applied once in module 2, so every
  source is normalized identically rather than each parser re-implementing it slightly differently.
- Fetching is behind a trait (`SourceFetcher`) so tests can supply fixture bytes instead of hitting
  the network; the pipeline binary wires in a real HTTP client.

### 2. `merge` — normalize, scope, union, provenance, category separation

```text
src/merge.rs
```

Responsibilities:

- Applies `domain_normalize::normalize` (module 0) to every `RawEntry`. This crate does **not**
  define its own normalization.
- Resolves each entry's final `RuleScope` via `domain_normalize::classify_scope`, combined with the
  parser's `scope_hint`:
  - `scope_hint = Apex` (a wildcard) plus `classify_scope = Apex` → `Apex`.
  - `scope_hint = Apex` on something that is not eTLD+1 → the *base* is kept as `ExactHost`, and
    the widening is refused. A wildcard cannot conjure apex coverage the PSL says isn't the
    tenant's.
  - `classify_scope = None` (the entry is a public suffix or on the shared-hosting denylist) → the
    entry is **dropped**, counted, and named in the build report. This is the case that would
    otherwise black-hole an entire hosting provider.
- `MergedEntry { domain: String, scope: RuleScope, sources: Vec<SourceId>, categories: Vec<Category> }`
  — the union type. **`categories` is a set, not a single value**, because two sources can
  legitimately disagree about what kind of site a domain is (one flags it `adult`, another flags the
  same domain `gambling`) and both are true statements about it, not a conflict to resolve. Merging
  two `RawEntry`s for the same normalized domain unions their `sources` the same way, rather than
  picking one arbitrarily, so a later "why is this blocked" query can answer with every source that
  flagged it and every category it was flagged under. Merging two entries with different scopes
  takes the **wider** scope (`Apex` wins), since one source legitimately naming the apex is a
  stronger statement than another naming one host under it.
- Categories are never blended into a single value: a source's own category (`adult`, `gambling`,
  `dating`, ...) is added to the entry's `categories` set, never overwrites it, and an entry ships to
  module 3 if **any** of its categories is one this build is configured to ship (an `adult`-only
  build still includes a domain that is also flagged `gambling`, because the `adult` flag alone
  qualifies it). Test a domain classified as both `adult` and `gambling` by two different sources and
  assert both categories survive merge.
- **Personal-name triage** — `flag_personal_name(second_level_label: &str) -> bool`. Pure function:
  returns true when the second-level label decomposes into **2–4 alphabetic tokens** separated by
  `-`, `.`, or `_`, in any order. Optionally raises confidence against an embedded first-name /
  surname frequency list. Matches are written to a review-queue file; **no verdict changes**. This
  is a triage filter to bound the review workload demanded by the decision doc's GDPR section, not
  a completeness guarantee, and its tests should assert that framing (it will flag `red-panda.com`
  and miss `janedoe.com`, and both are acceptable).

### 3. `liveness` — DNS-only revalidation with a persistent TTL cache

```text
src/liveness.rs
```

Responsibilities:

- `LivenessCache` — a `domain → { last_checked, verdict, sources }` table. **Its storage is a named,
  persistent, non-repository location**: a private object-store bucket, a release asset on the
  pipeline's own distribution channel, or a CI artifact cache with retention configured well beyond
  the TTL. It is fetched at run start and written back at run end. A CI runner's filesystem is
  ephemeral and cannot hold it; "the pipeline's own storage" is not an answer. It is **not**
  committed into the public repository tree, consistent with the project's convention of gitignoring
  corpora and model artifacts. A run that cannot load the cache starts cold; a run that cannot write
  it back **fails loudly** rather than discarding months of accumulated state.
- `check(domain: &str) -> Verdict` — **two lookups per domain, A and AAAA.** Not one: they are
  separate QTYPEs, and `QTYPE=ANY` is not a shortcut (RFC 8482). `Verdict` is **not** a boolean:

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

  **Reducing the A and AAAA lookups to one `Verdict`** — the two queries are independent and can
  disagree (e.g. A returns `NXDOMAIN` while AAAA times out), so `check()` applies this order,
  evaluated top to bottom:

  1. If **either** lookup (or its CNAME chain) resolves to an address, the result is `Alive`. One
     working address family is enough for the domain to be reachable.
  2. Otherwise, if **both** lookups unambiguously return `NXDOMAIN`, the result is `Dead`.
  3. Otherwise — at least one lookup is `Unknown` and neither is `Alive` — the result is `Unknown`.
     This covers a mixed `NXDOMAIN`/`Unknown` pair: one family failing to resolve while the other
     family's answer is merely inconclusive must never be conflated with both families cleanly
     saying the domain doesn't exist.

  A domain checked for the first time that comes back `Unknown` is **included by default**, the
  same as an existing domain whose cached verdict is `Unknown` — a build never treats "we couldn't
  tell" as a reason to omit a domain it has no prior information about. Test the mixed-verdict case
  explicitly, and test first-seen-`Unknown` retention separately from previously-cached-`Unknown`
  retention.
- **An explicitly configured resolver, never the host's system resolver.** The address is
  configuration, defaulting to Cloudflare's unfiltered `1.1.1.1` / `2606:4700:4700::1111` —
  explicitly *not* the `1.1.1.2` or `1.1.1.3` filtering variants. A CI provider's default resolver
  is frequently a filtering one, and that failure is silent and total.
- **`canary_check() -> CanaryResult`, run before every sweep and gating it.** A small fixed control
  set, checked in both directions:
  - Several known-always-alive, definitely-non-adult domains must each return `Alive`.
  - At least one reserved never-resolving name (RFC 2606 / RFC 6761 — `invalid.`, a name under
    `test.`) must return `Dead`, which catches a resolver that NXDOMAIN-rewrites to a wildcard sink.

  **Any canary result other than expected aborts the entire sweep.** Every verdict from that run is
  discarded — not merged, not written back to the cache — and nothing is published. There is no way
  to know at which query a lying resolver started lying, so a partial sweep is not salvaged. This is
  the guard against the single worst failure in the design: a content-filtering resolver returning
  NXDOMAIN for the whole list, pruning it to near-zero, and shipping that signed.
- `due_for_check(cache_entry, now, ttl) -> bool` — pure function deciding whether a cached verdict is
  still trusted or needs a fresh lookup. **TTL is decoupled from the run cadence and is a multiple of
  it: cadence monthly, TTL 3 cadences (~3 months).** With TTL equal to the cadence, every cached
  entry is due on every run and the cache saves nothing while being exquisitely sensitive to clock
  jitter — a run a minute early or late swings the check volume wildly. At 3× only about one third of
  the previously-dead cohort is due on any run, which is where the saving actually comes from, and a
  genuine revival is still caught within one TTL. Both numbers are configuration, and the tests
  should cover cadence ≠ TTL explicitly.
- **Pacing: a steady low rate across a bounded ~24-hour sweep window**, not a burst and not smeared
  across the whole month. A cold sweep of ~1,000,000 domains is ~2,000,000 queries, which over 24
  hours is **~23 qps**. Concurrency (many in-flight queries, rate-limited to the target qps) is the
  lever; batching is not available, since a single DNS message answering multiple questions is not
  something real-world resolvers implement despite RFC 1035's QDCOUNT nominally allowing it.
- A `Dead` verdict removes the entry from the next build's output; it does not delete it from the
  cache, so a later revival is still detected once its TTL expires rather than being permanently
  invisible.
- Note the accepted limitation, stated in the decision doc and worth restating where it is
  implemented: **DNS liveness measures registration, not content.** A parked domain still resolves.
  Erring toward keeping an entry is the intended direction — false negatives are the budget, false
  positives are the price — which is why only an unambiguous NXDOMAIN prunes.

(See [Reference documents](#reference-documents) below for the DNS response codes this matrix is
built from.)

### 4. `fst_build` — the on-device artifact

```text
src/fst_build.rs
```

Responsibilities:

- Reverses each surviving domain's labels (`example.com` → `com.example`) so subdomain matching
  becomes prefix/exact-match at label boundaries on the consumer side. The key is the *normalized*
  form with no host-identity stripping (module 0): a source entry of `www.example.com` produces the
  key `com.example.www`, distinct from the apex key `com.example` — an `ExactHost` rule stored under
  the `www` key matches only a query for `www.example.com`, never the bare domain or any other
  subdomain. A separate `Apex` entry for `example.com`, if one exists, already covers `www` (and
  every other subdomain) through the label-boundary lookup below, without needing the two keys to
  collide.
- Sorts the reversed, deduplicated key set (an `fst::MapBuilder` requirement — the merge/dedupe step
  upstream already produces this property, so this is not new work, just an ordering constraint on
  output).
- Builds the FST, mapping each key to a small **provenance ID** (`u32`). The ID encodes the
  canonicalized combination of `{sources, categories, scope}` — **an enumerated set, not a
  per-domain value.** `categories` is the same set `MergedEntry` carries (module 2): a domain
  flagged under two categories keeps both in the combination it's assigned. Most domains share one
  of a few dozen combinations, and keeping the value space small is
  what limits the damage a `Map`'s per-key value does to suffix-sharing (states with differing
  outputs cannot be merged). The ID is only meaningful alongside the manifest entry that decodes it,
  so a future "why is this blocked" surface needs no second on-device lookup structure.
- **Report the achieved bytes-per-entry in the build metrics.** The commonly quoted 2–4 bytes/entry
  figure describes a bare set; this builds a `Map`. The number must be measured, and the size budget
  in module 5 — not the estimate — is the gate.
- Writes the `.fst` file plus a manifest:

  ```rust
  struct Manifest {
      version: u64,                          // monotonic; the client's rollback high-water mark
      build_time: Timestamp,
      entry_count: u64,
      output_license: LicenseId,             // least-permissive of the input licenses
      sources: Vec<SourceSnapshot>,          // one per constituent source, from module 1
      provenance_table: Vec<ProvenanceEntry>, // provenance_id -> { sources, categories, scope }
      fst_digest: [u8; 32],                  // SHA-256 of the .fst file, binding manifest to artifact
      signatures: Vec<Signature>,            // { key_id: KeyId, bytes: [u8; 64] } — see below
  }
  ```

  `version` and `signatures` are load-bearing, not decoration: the decision doc's rollback defense
  rejects a bundle whose `version` is not strictly greater than the client's high-water mark, and
  each `signatures` entry's `key_id` is how a client picks which of its trusted keys to try instead
  of guessing. An earlier draft of this struct had a single `key_id`/signature pair, which is exactly
  what key rotation (decision doc, [Distribution](../../decisions/domain-blocklist-sourcing.md#distribution))
  cannot be built on — a single signer means the pipeline can only ever satisfy one generation of
  client trust set at a time. `signatures` is a list so a build can carry both an old-key and a
  new-key signature during a rotation's overlap window.

- **Signs `manifest_bytes` with every key currently active for signing** — one entry in `signatures`
  per key, each an Ed25519 signature over the manifest bytes with the `signatures` field itself
  excluded from what's signed (it's appended after signing each entry, not signed over — otherwise
  adding the second signature during rotation would invalidate the first). Outside a rotation window
  there is exactly one entry. Not `fst_digest || manifest_bytes` — `fst_digest` is already a field
  inside the manifest, so signing the manifest already binds the artifact transitively. The
  concatenation added a framing detail two implementations could disagree about and no security
  property.
- Round-trip tests cover building a small provenance table, encoding it into the manifest, decoding
  it back to the same `{sources, categories, scope}` per ID — including a domain assigned two
  categories — verifying that a manifest with two signature entries (simulating a rotation) validates
  against a client trusting only the old key, only the new key, or both, and that altering
  `fst_digest` after signing invalidates every signature entry.

(See [Reference documents](#reference-documents) below for the FST/mmap references this module builds on.)

### 5. `gates` — the publish gates

```text
src/gates.rs
```

Pure functions over the built artifact and the previous published build's manifest. **Every gate
below refuses publication and requires explicit human sign-off to override — never an automatic
publish.** They are pure and separately tested precisely because they are the last thing standing
between a bad build and a signed one.

- `shrinkage_gate(prev_count, new_count, max_drop_pct, absolute_floor) -> GateResult` — fails when
  the entry count drops more than **10%** below the previous published build, or below an absolute
  floor. Catches a bad liveness sweep the canary missed, a source that silently emptied, and parser
  regressions.
- `growth_gate(prev_keys, new_keys, max_add_pct) -> GateResult` — fails when *added* entries exceed
  **10%** of the previous published build. Catches a compromised or mis-bumped upstream injecting
  bulk entries. Shrinkage and growth are separate gates because they catch opposite failures.
- `false_positive_gate(merged, control_set, exclusions, max_fp_rate) -> GateResult` — evaluates the
  merged list against a **known-good negative control**: the [Tranco](https://tranco-list.eu/)
  top-N list, pinned to a specific daily release like any other source, minus a small reviewed,
  checked-in exclusion file of control entries that are legitimately adult. **Starting threshold:
  0.5%.** Reports which control entries were hit and under which provenance ID, so a regression
  names the source that caused it. This mirrors `machine-learning`'s `gate.py` release guardrail —
  the project already refuses to ship a classifier with no measured quality signal, and a blocklist
  with no measured FP rate is the same gap.
- `size_gate(artifact_bytes, ceiling) -> GateResult` — **ceiling 32 MB** for the `.fst` file. The
  precedent for stating a ceiling at all is `image-sandbox`'s 15 MB model budget. Since the merged
  entry count is a planning assumption rather than a measurement, this gate is what turns "the list
  is much bigger than we assumed" into a decision instead of a surprise on a user's device.
- `license_gate(snapshots, allowlist) -> GateResult` — module 1 enforces this at fetch time; it is
  re-checked here so the published artifact can never carry a snapshot the allowlist doesn't cover.

Every threshold above is a **starting value to be re-derived from real builds**, not a defended
constant.

### 6. `net-shield` integration

`DomainFilter` (`packages/net-shield/src/radix.rs`) gains a **second, separate loading path**
alongside its existing one — it does not grow I/O or mmap logic of its own. A new type owns the
signed-artifact concerns and hands `DomainFilter` something it already knows how to consume:

```rust
// packages/net-shield/src/radix.rs — unchanged:
impl DomainFilter {
    pub fn from_rules(rules: &[(&str, FilterAction)]) -> Self   // existing, untouched
}

// new: a thin wrapper that owns the artifact, not DomainFilter itself
pub struct BlocklistArtifact {
    map: Arc<memmap2::Mmap>,       // the mmap-backed .fst, per the decision doc
    provenance: Vec<ProvenanceEntry>,
}

impl BlocklistArtifact {
    /// Verifies the manifest signature and fst_digest against `path`'s current/previous
    /// slot layout (decision doc, on-device storage section) before returning. Fails closed:
    /// an Err here means "no artifact," never a partially-trusted one.
    pub fn load(path: &Path, trusted_keys: &[PublicKey]) -> Result<Self, ArtifactError>

    /// Looks up a provenance ID for a domain key, or None if absent. Pure, no I/O — the mmap
    /// access itself is what can major-fault, not this call.
    fn provenance_for(&self, reversed_key: &str) -> Option<&ProvenanceEntry>
}
```

`DomainFilter::from_rules` and its existing tests are untouched. A `BlocklistArtifact` is a
*separate* handle that `net-shield`'s top-level filter-query path consults in addition to
`DomainFilter`, per the precedence table below — it is not spliced into `DomainFilter`'s own
internal trie.

Three things must be specified here or matching breaks silently.

**Precedence.** The FST stores a provenance ID, not a `FilterAction`, and `net-shield`'s existing
`DomainFilter` has richer semantics than a flat block-only list: a specific `Allow` can override a
broader `Block`, and a domain matching nothing defaults to `Proxy`. FST rules therefore slot in at
the bottom, not on top:

| Priority | Source | Notes |
|---|---|---|
| 1 (highest) | Device-local user allowlist | Always wins, unconditionally. The user's no-appeal remedy |
| 2 | `net-shield`'s existing explicit rule set | `Allow` and `Proxy` entries, unchanged semantics, including a specific `Allow` beating a broader `Block` |
| 3 | FST-sourced `Block` rules | Apply only to domains not covered by 1 or 2. Scope (`Apex` vs `ExactHost`) comes from the provenance table |
| 4 (lowest) | `DomainFilter`'s existing default | `Proxy`, unchanged |

The FST is consulted only after levels 1 and 2 miss, so loading a multi-million-entry blocklist can
never change the behavior of an existing explicit rule or an existing test.

**Normalization.** The query side must normalize **identically** to the build side. Both consume
`packages/domain-normalize` (module 0); neither reimplements it. Reimplementing it in `net-shield`
would leave two copies that each pass their own tests and disagree in production — the exact drift
`packages/text-policy-ffi` exists to prevent, per the "one implementation, no drift" lesson recorded
in the mac-daemon row of `AGENTS.md`.

**Lookup budget.** Per the decision doc's mmap section, a `BlocklistArtifact` lookup must never block
packet delivery on a page fault, which `dns_shield.rs`'s current inline, synchronous
`self.domains.lookup(...)` call cannot guarantee once `domains` can be artifact-backed. The bounded
contract:

- A single-threaded worker owns the `BlocklistArtifact` and receives lookup requests over a bounded
  channel keyed by the **normalized** domain (module 0's `normalize()` — the packet-handling side
  must never normalize differently than the worker or a cache hit/miss becomes non-deterministic).
- `DnsShield::inspect` sends a request and waits at most **2 ms** (starting value, per the mmap
  section) on a response channel. Two outcomes:
  - The worker answers in time → its provenance-derived `FilterAction` is used and also written into
    a small in-memory LRU cache keyed by the same normalized domain, so a repeated query for the same
    domain during a burst doesn't re-enter the channel at all.
  - The budget expires first → `inspect` falls back to the LRU cache's last answer for that domain
    if one exists, otherwise to `DomainFilter`'s existing default action. The in-flight worker
    request is not cancelled; its eventual answer still populates the cache for the next query.
- **A request already in flight for the same domain is not duplicated** — a second `inspect` call for
  a domain the worker is already resolving attaches to the same in-flight future rather than queuing
  a second worker request, so a burst of packets to one blocked domain costs one worker round trip,
  not one per packet.
- Every fallback (timeout expiry) increments a counter exposed the same way other daemon health
  signals are — a metric, not a silently absorbed path — since a fallback that fires constantly means
  the 2 ms budget or the cache size needs revisiting.
- Tests cover: a fast worker response within budget, a slow worker forcing the timeout fallback (with
  and without a warm cache entry), and two concurrent lookups for the same domain collapsing to one
  worker request.

### 7. `cli` — the pipeline entry point

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

## Implementation order

1. ~~`packages/domain-normalize` — pure, no I/O, tested first and hardest. `normalize()` against IDNs
   (both directions of the UTS #46 → punycode ordering), trailing dots, mixed case, `www.` variants,
   and post-conversion length limits; `classify_scope` against public suffixes, provider suffixes,
   the shared-hosting denylist, and deep hostnames. Every apex-widening bug this design fears is
   caught here or nowhere.~~ **Done.** `normalize()` uses the `idna` crate's UTS46 pass
   (`AsciiDenyList::STD3`, `Hyphens::Allow`, `DnsLength::Ignore`, with label/name length validated
   explicitly afterward per RFC 1035 §2.3.4/RFC 5891 §4.4) and `classify_scope()` uses the `psl`
   crate (compiled-in Public Suffix List data — no I/O, no network fetch) plus a caller-supplied
   shared-hosting denylist slice. 22 tests, including every named case from this plan (`com`,
   `co.uk`, `blogspot.com`, `s3.amazonaws.com` never `Apex`; `someone.blogspot.com` is `Apex`;
   `www.example.com` normalizes to itself and scopes `ExactHost`, distinct from the `example.com`
   `Apex` key). Not yet consumed by `domain-blocklist` (module 1+, unbuilt) or `net-shield` (module
   6, unbuilt) — this is the shared crate only.
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
   (modules 1, 3, 4 — all still unbuilt).
3. `gates.rs` — pure functions against synthetic counts and key sets. Built early precisely because
   they are cheap, pure, and are what stops every category of bad build; leaving them for last means
   the first real run has no guardrail.
4. `sources/` — one parser per source against small fixture files first (a few lines of each
   source's real format, not a live fetch), covering every row of the input-shape table above, then
   wire in the real pinned `SourceFetcher` HTTP path last.
5. `liveness.rs` — `due_for_check` first as a pure function against a fake clock and fake cache
   entries with cadence ≠ TTL (this is the part with the trickiest edge cases: reappeared-stale vs.
   genuinely-revived entries). Then `canary_check` against a fake resolver that simulates a
   family-filtering resolver, a wildcard-sink resolver, and a healthy one — all three must be
   distinguishable. The real DNS lookup is a thin, separately-tested edge behind a trait so none of
   this needs live network access to test.
6. `fst_build.rs` — pin against a small hand-built key set first (a handful of domains sharing
   prefixes and suffixes, mixed `Apex` and `ExactHost`) and assert exact-match, label-boundary
   scoping, and **anchored** prefix-streaming behavior before building the real list.
7. `cli` — wire the whole pipeline together; the first real run is a dry run against the three
   fixture sources, not a live fetch.
8. `net-shield` integration — the precedence table and the shared `normalize()`, per module 6.
9. **The mmap benchmark** (below) — after there is a real artifact to map.

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
- **Delta/patch distribution of the artifact** — the FST ships monolithically. Noted in the decision
  doc as a future improvement; until it exists, pruning's justification is bounded artifact size and
  reduced stale-hit surface, not smaller downloads.
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
