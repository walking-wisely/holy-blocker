# 6. `net-shield` integration

Part of the [Domain Blocklist plan](../plan.md).

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

## Assumption audit

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
