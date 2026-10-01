# 1. `sources` — per-source fetch and parse

Part of the [Domain Blocklist plan](../plan.md).

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

## Input-shape handling (defined behavior for every case these formats actually contain)

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
