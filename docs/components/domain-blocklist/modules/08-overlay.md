# 8. `overlay` — a small, fast-cadence tier for urgent additions and removals

Part of the [Domain Blocklist plan](../plan.md).

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
new wire format or a new OS interface. See [module 4](04-fst-build.md) and the
decision doc's [Distribution](../../../decisions/domain-blocklist-sourcing.md#distribution) section.
