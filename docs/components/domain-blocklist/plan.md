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

**Done.** Built as `src/liveness/{mod,lookup,cache,canary,corroboration}.rs`, then hardened across
three review rounds. The load-bearing correction from the third round: an earlier version of this
module required DNSSEC authentication (the AD bit) on both address families before trusting an
NXDOMAIN as `Dead`, and that requirement was measured live to be **the wrong design**, not a
stricter-but-safer one — most large TLDs sign with NSEC3 opt-out (RFC 5155 §6), under which a
validating resolver cannot construct authenticated denial-of-existence for an unsigned delegation
at all, so the gate made `Dead` almost unreachable rather than safer to reach, and the module's own
canary didn't catch it (`invalid.`/`test.` sit under the root zone, which uses plain NSEC rather
than NSEC3 opt-out, so they authenticated cleanly while the real sweep quietly pruned nothing). The
fix
is `liveness::corroboration::corroborate`/`check_corroborated`: pruning now requires **two
independently-configured resolvers** to each independently produce `Dead`, replacing the
DNSSEC-authentication requirement as the actual defence against a lying or hijacking resolver;
`authenticated` is kept as loggable evidence but no longer gates anything. `should_prune`/
`should_prune_with_hysteresis` (`cache.rs`) now require the corroborated verdict, not a single
resolver's own `check()` result. Also fixed in the same round: `is_filtering_ede` broadened to
cover RFC 8914 codes 13 (Cached Error) and 18 (Prohibited) alongside 19 (Stale NXDOMAIN Answer);
`nonce_dead_control`'s worked example and this file's own tests, which used the Cloudflare-hosted
`example.com` — measured to answer a nonexistent subdomain with NODATA ("compact denial of
existence") rather than NXDOMAIN, which broke the dead control and aborted every sweep
permanently — renamed away from it, with the trap now documented explicitly; the apparent tension
between `should_prune` (a `Dead` verdict on `.invalid`/`.test` proves nothing about registration)
and `canary_check` (the same verdict on the same names proves the resolver is honest) resolved with
cross-referencing doc comments explaining these are different questions about the same verdict, not
a contradiction; and the `DnsLookup` trait's contract doc now states explicitly that an
implementation must set the DNSSEC OK bit and read the AD/EDE evidence off the wire, ruling out a
`getaddrinfo`-style high-level resolver API for module 7.

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

- `shrinkage_gate(prev_count: PreviousBuild<u64>, new_count, max_drop_pct, absolute_floor) ->
  GateResult` — fails when the entry count drops more than **10%** below the previous published
  build, or below an absolute floor. Catches a bad liveness sweep the canary missed, a source that
  silently emptied, and parser regressions. `PreviousBuild<T>` (`None` | `Existing(T)`) is
  deliberate, not `u64`/`Option<u64>`: a caller that failed to load the previous manifest and a
  caller correctly reporting a genuine first build must never be able to produce the same input, or
  this gate (and `growth_gate` below) silently disables itself on the easiest-to-produce mistake —
  see the decision doc's [Distribution](../../decisions/domain-blocklist-sourcing.md#distribution)
  trust contract for what "the previous manifest" means.
- `growth_gate(prev_keys: PreviousBuild<&BTreeSet<String>>, new_keys, max_add_pct) -> GateResult` —
  fails when *added* entries exceed **10%** of the previous published build. Catches a compromised
  or mis-bumped upstream injecting bulk entries. Shrinkage and growth are separate gates because
  they catch opposite failures.
- `false_positive_gate(merged, control_set, exclusions, max_fp_rate, min_control_size) ->
  GateResult` — evaluates the merged list against a **known-good negative control**: the
  [Tranco](https://tranco-list.eu/) top-N list, pinned to a specific daily release like any other
  source. See the decision doc's ["A measured false-positive gate on every
  build"](../../decisions/domain-blocklist-sourcing.md#3-a-measured-false-positive-gate-on-every-build)
  for the full design; in short:
  - `false_positive_hits(merged, control_set) -> Vec<FalsePositiveHit>` finds every control-set
    domain the merged list blocks, normalizing `control_set` identically to `merged`'s own keys
    first (an earlier version of this function compared unnormalized control entries against
    normalized merged keys and silently matched nothing).
  - `FalsePositiveHit::is_corroborated()` is true when **two or more** independent sources flagged
    the domain — a corroborated hit never counts as a false positive, since it takes two separately
    curated projects independently agreeing rather than one project's own file-level mistake.
  - `review_queue(merged, control_set, exclusions) -> Vec<FalsePositiveHit>` is the bounded set that
    actually needs a human: uncorroborated hits not already covered by a small, reviewed,
    checked-in `exclusions` file. This is what replaces reviewing "every adult site Tranco ranks."
  - The gate itself fails when `review_queue`'s rate against the checked control set exceeds
    `max_fp_rate`, or when the checked control set falls below `min_control_size` (an empty or
    truncated control-set fetch must never look identical to "measured, and clean"). **Starting
    threshold: 0.5%.**
  - Reports which control entries were hit and under which sources, so a regression names the
    source that caused it. This mirrors `machine-learning`'s `gate.py` release guardrail — the
    project already refuses to ship a classifier with no measured quality signal, and a blocklist
    with no measured FP rate is the same gap.
- `size_gate(artifact_bytes, ceiling) -> GateResult` — **ceiling 32 MB** for the `.fst` file. The
  precedent for stating a ceiling at all is `image-sandbox`'s 15 MB model budget. Since the merged
  entry count is a planning assumption rather than a measurement, this gate is what turns "the list
  is much bigger than we assumed" into a decision instead of a surprise on a user's device.
- `license_gate(merged, snapshots, allowlist) -> GateResult` — module 1 enforces the
  license-on-allowlist half of this at fetch time; it is re-checked here so the published artifact
  can never carry a snapshot the allowlist doesn't cover. Also fails when a source that contributed
  entries to `merged` has **no** snapshot at all (an omission, not a bad license — the cheaper
  mistake to make and the one a license-only check misses entirely), or when `snapshots` carries
  more than one entry for the same source (ambiguous license coverage). License comparison is
  case/whitespace-insensitive, matching SPDX's own identifier rule. **Not yet checked here, and
  tracked as a gap until module 1/4 exist to check it against:** a source's snapshot `license`
  drifting from its `SourceConfig.expected_license` pin, and the manifest's own `output_license`
  field being consistent with what was actually measured.

Every threshold above is a **starting value to be re-derived from real builds**, not a defended
constant.

### 6. `net-shield` integration

`DomainFilter` (`packages/net-shield/src/radix.rs`) gains a **second, separate loading path**
alongside its existing one — it does not grow I/O or mmap logic of its own. A new type owns the
signed-artifact concerns and hands `DomainFilter` something it already knows how to consume:

**Superseded by the assumption-audit table below**: the sketch originally here declared
`map: Arc<memmap2::Mmap>` alongside a separate `fst::Map`, which is exactly the self-referential
`Mmap`/`Map` pair the audit's first row rejects. The implementation (and the shape to read as
current) owns a single `fst::Map<Mmap>`, which is `Send + Sync` on its own and needs no `Arc`:

```rust
// packages/net-shield/src/radix.rs — unchanged:
impl DomainFilter {
    pub fn from_rules(rules: &[(&str, FilterAction)]) -> Self   // existing, untouched
}

// new: a thin wrapper that owns the artifact, not DomainFilter itself
pub struct BlocklistArtifact {
    map: fst::Map<memmap2::Mmap>,  // owns the mmap-backed .fst directly — no Arc, no self-reference
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
| 3 | `OverlaySet`-sourced rules (module 8, unbuilt) | Additions/removals published since the last bulk `.fst` rebuild — see module 8. Checked before the bulk FST specifically because its whole purpose is to represent state the bulk artifact doesn't have yet |
| 4 | FST-sourced `Block` rules | Apply only to domains not covered by 1–3. Scope (`Apex` vs `ExactHost`) comes from the provenance table |
| 5 (lowest) | `DomainFilter`'s existing default | `Proxy`, unchanged |

The FST is consulted only after levels 1–3 miss, so loading a multi-million-entry blocklist can
never change the behavior of an existing explicit rule, an existing test, or an in-flight overlay
entry. **A matching level-3 `OverlayAction::Remove` terminates lookup right there**: it resolves to
`DomainFilter`'s default action (`Proxy`) at the matched `RuleScope`, and level 4 is never consulted
for that domain — a `Remove` entry exists specifically to unblock a domain the bulk FST still blocks,
so falling through to level 4 after matching one would silently keep the domain blocked. This is
strictly lower priority than levels 1–2: an explicit `net-shield` `Block`/`Allow` or the device-local
allowlist still wins over an overlay `Remove` the same way they win over the bulk FST. See module 8's
"On-device precedence and lookup" for the full contract.

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

**The overlay (priority 3, module 8) does not share this budget or this worker.** It is bounded to a
few thousand entries by module 8's own size cap, so it is held as a plain in-memory `HashMap` rather
than mmap-backed — there is no page fault to bound a budget against, and consulting it costs an
ordinary hash lookup on the calling thread before the bulk-FST worker is ever contacted. Module 8's
own load/verify/swap path is what needs the async, off-thread treatment; the query-time lookup does
not.

#### Assumption audit

Run 2026-08-10 before implementing this module. Claims verified against the shipped crate sources in
`~/.cargo/registry`, not against docs.

| Claim | Falsifier | Observed | Verdict |
|---|---|---|---|
| `fst::Map<D>` can own its backing bytes, so the artifact needs no self-referential `Mmap`/`Map` pair — `Map<Mmap>` owns the mapping and is `Send + Sync` | read `fst-0.4.7/src/raw/mod.rs` `struct Fst<D>` | `struct Fst<D> { meta: Meta, data: D }` — no raw pointer, no `PhantomData`; `Map::new(data: D)` takes ownership | **TRUE** — `Map<Mmap>` is `Send + Sync` iff `Mmap` is; this replaces the plan's `map: Arc<Mmap>` field with a single owned `fst::Map<Mmap>` inside the artifact (same reference-counting/reclaimability, no self-reference) |
| `memmap2::Mmap` is an `AsRef<[u8]>`/`Deref<Target=[u8]>` and `Send + Sync` | read `memmap2-0.9.11/src/lib.rs` | `impl Deref for Mmap` (l.918), `impl AsRef<[u8]> for Mmap` (l.927), `MmapOptions::map_copy_read_only` (l.593); `Mmap` holds only `ptr`/`len` | **TRUE** |
| `fst::Map::get(key) -> Option<u64>` is exact-match, usable for the decision doc's label-boundary lookups | read `fst-0.4.7/src/map.rs` | `pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Option<u64>` (l.133) | **TRUE** — the hot path is `get` at each label boundary, not prefix streaming (which would need anchoring, per decision doc l.674) |
| A `Manifest`/artifact produced by `fst_build::build` round-trips through bincode so a consumer can deserialize it | existing `fst_build` test `manifest_round_trips_through_bincode` | passes; `Manifest`/`ProvenanceEntry` carry serde derives | **TRUE** — net-shield deserializes the same `Manifest` type from `domain-blocklist` rather than redefining it (no drift) |
| The on-device two-slot layout has a defined file format net-shield can load | search decision doc / plan for a filename or byte layout | decision doc specifies `current/` + `previous/` slots, each a `.fst` + manifest pair, and a high-water-mark record, but **names no file format and module 7 `cli` (which writes it) is unbuilt** | **UNVERIFIED — gap** — `load` must define the slot layout it reads; the constants are specified in `blocklist.rs` and module 7 must write the same layout, tracked here until module 7 exists |
| Query-side normalization must not drift from build-side | `net-shield` calls `domain_normalize::normalize` directly (no reimplementation) | by construction, same function both sides | **TRUE** |

The one load-bearing gap is the undefined on-disk slot layout. `BlocklistArtifact::load` defines it as
a base directory with `current/artifact.fst` + `current/manifest.bin`, `previous/` with the same two
files, and a `high_water_mark` file (a u64, absent ⇒ 0). Cold-start `load` verifies `current/` then
`previous/` and fails closed to no artifact if both fail; the `version > high-water-mark` rollback
check is the update/accept path's job (module 7 / distribution), not the cold-start loader's. Module 7
must write exactly this layout for the two to interoperate.

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

#### Assumption audit

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

#### Net-client assumption audit (run 2026-08-15, before building the real `DnsLookup` + sweep loop in `src/liveness/`)

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
[module-7-cli-adversarial-review.md](module-7-cli-adversarial-review.md) — that file is a **pre-fix
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

### 8. `overlay` — a small, fast-cadence tier for urgent additions and removals

```text
src/overlay.rs
```

**The problem this solves.** The bulk `.fst` (module 4) can only ever be rebuilt whole — `fst::Map`
has no incremental-edit API, and the compression itself is a function of the *entire* sorted key
set, so one added or removed key can shift which states downstream get merged. That rebuild is cheap
on the pipeline's own build machine but is not the bottleneck: every *device* update, no matter how
small the actual change, still costs a full-artifact re-download and a full sequential hash-verify
(the decision doc's on-device storage section — verification reads every byte, by design, on every
load). Waiting for the next monthly bulk rebuild to propagate a single newly-flagged domain, or an
urgent takedown of a wrongly-blocked one, is too slow for either case, and shrinking the bulk
cadence to fix it would mean paying that full-artifact cost on every device far more often than the
bulk gates (canary, shrinkage, growth, false-positive) need to run.

The fix is not to make the bulk FST patchable — the `fst` crate offers no such thing, and mutating a
live mmap that lookups may be concurrently reading is its own hazard the on-device storage design
already avoids by never doing it (see the decision doc's rejected alternatives). The fix is a
**second, much smaller artifact** that is cheap enough to re-fetch and re-verify in full, often:

- `OverlayEntry { domain: String, scope: RuleScope, action: OverlayAction, categories: Vec<Category>, added_at: Timestamp }`,
  where `OverlayAction { Add, Remove }` — `Remove` is what makes an urgent takedown of a wrongly-block
  domain possible without waiting for the next bulk rebuild, the same way the bulk FST's `Apex`/
  `ExactHost` distinction already exists for `Add`. **A `Remove` entry that matches at its declared
  `RuleScope` unblocks the domain and stops lookup there** — it resolves to `DomainFilter`'s default
  action rather than falling through to the bulk FST, which still carries the rule this entry exists
  to override. It does not touch levels 1–2 of module 6's precedence table: a device-local allowlist
  entry or an explicit `net-shield` rule at higher priority still wins over a `Remove`, the same way
  either already wins over the bulk FST. See module 6's precedence table for the full ordering.
- `OverlaySet { entries: Vec<OverlayEntry> }`, capped at a **starting value of 5,000 entries** — an
  order of magnitude of headroom over what "urgent, between bulk rebuilds" should ever need, kept
  small on purpose so the whole set stays cheap to hold in memory unindexed (module 6's lookup-budget
  section) and cheap to transmit even on a slow connection.
- A manifest reusing **exactly** module 4's trust contract — monotonic `version`, `{key_id,
  signature}` list, a digest binding the manifest to the entry set, the same current/previous
  atomic-slot swap on disk. This is not a new mechanism to design or a new mechanism to audit; it is
  the same one, applied to a smaller payload. See module 4's manifest struct and the decision doc's
  Distribution section for the field-for-field contract this must match.

**Publish gates, scaled down, not skipped.** The bulk gates' statistical thresholds (10% shrinkage,
10% growth, 0.5% false-positive) don't mean anything against a few-entry diff, but "never an
automatic publish" still applies:

- `overlay_size_gate(current_len, added, removed, max_publish_size) -> GateResult` — refuses a
  publish that adds or removes more than a small fixed count in one go (starting value: **50**
  entries per publish). A legitimate urgent fix is a handful of domains; anything larger than that is
  either a mis-scoped batch that should go through the reviewed bulk pipeline instead, or a sign
  something is wrong with whatever produced the list of urgent entries.
- Every `OverlayEntry.domain` still goes through `domain_normalize::normalize`/`classify_scope`
  (module 0) and the same public-suffix/shared-hosting-denylist refusal `merge()` already applies —
  an overlay is not a bypass of the scoping rules that protect against black-holing a hosting
  provider, just of the monthly cadence.
- Signing and human sign-off are not waived for being small. The plan's framing — "every gate below
  refuses publication and requires explicit human sign-off to override" — applies here at a smaller
  scale, not a lower bar.

**Folding and draining.** Every bulk rebuild (module 7's `cli`) first folds the current overlay's
`Add` entries into the ordinary source-merge pipeline as if they had arrived from a source with that
build's cadence, and applies its `Remove` entries as exclusions against the freshly-merged set,
before running the bulk gates. Once a bulk build incorporating a given overlay entry publishes, that
entry is dropped from the next `OverlaySet` — the overlay is a bounded, actively-drained queue of
"not yet reflected in the bulk artifact," never a second permanent copy of the list living outside
the reviewed bulk pipeline. An overlay that is never drained (the bulk pipeline stops running, or
keeps failing its own gates) is exactly the kind of staleness the decision doc's "staleness must be
visible" rule already covers — surfaced the same way, not through a separate mechanism.

**Distribution stays inside the existing consent model — this does not become a background push.**
Per the decision doc, updates are opt-in and never silently polled; the overlay does not change that
contract, it changes what a check costs. A user who checks daily pays a few-KB, sub-second overlay
fetch and verify on days without a bulk update, instead of either doing nothing (today's only opt-in
outcome between monthly bulk releases) or paying a multi-megabyte re-fetch to catch one new domain.
"Fast" here means "cheap enough that the user's own chosen check cadence is enough," not a new
covert channel.

**On-device precedence and lookup.** Specified in module 6 above (priority 3, between `net-shield`'s
explicit rule set and the bulk FST) — checked before the bulk FST specifically because it exists to
cover exactly the domains the bulk artifact doesn't have yet, and held as a plain in-memory map since
its capped size makes mmap's page-fault tradeoff unnecessary. A matching `OverlayAction::Remove`
terminates the lookup at level 3 rather than falling through to level 4 — see module 6's precedence
table for why a fall-through would silently re-block a domain this entry exists to unblock. This
holds regardless of how the map itself is implemented (plain `HashMap`, a second mmap, or a
cache-through structure in front of the bulk FST); the storage choice is unmeasured and open, the
termination behavior is not.

**Reference documents**

Reuses module 4's manifest/signing contract and its reference documents (Ed25519 signing, the
Distribution section's rollback/rotation model) without restating them — nothing here introduces a
new wire format or a new OS interface. See [module 4](#4-fst_build--the-on-device-artifact) and the
decision doc's [Distribution](../../decisions/domain-blocklist-sourcing.md#distribution) section.

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
   these gates in sequence and act on a `Fail` doesn't exist yet.
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
   unbuilt).
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
   would need.
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
   pipeline that would call `build` and act on the manifest doesn't exist yet.
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
   is a larger change than this pass's scope.
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
    had no direct test; one was added. Net-shield total now 92.
9. **The mmap benchmark** (below) — after there is a real artifact to map.
10. `overlay.rs` — after `fst_build` and `net-shield` integration both exist, since it reuses
    module 4's manifest/signing contract wholesale and slots into module 6's precedence table rather
    than defining either from scratch. Pin the size and publish-size gates against synthetic entry
    sets first, the same way `gates.rs` was built ahead of a real artifact to test against.

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
