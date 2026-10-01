# 5. `gates` — the publish gates

Part of the [Domain Blocklist plan](../plan.md).

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
  see the decision doc's [Distribution](../../../decisions/domain-blocklist-sourcing.md#distribution)
  trust contract for what "the previous manifest" means.
- `growth_gate(prev_keys: PreviousBuild<&BTreeSet<String>>, new_keys, max_add_pct) -> GateResult` —
  fails when *added* entries exceed **10%** of the previous published build. Catches a compromised
  or mis-bumped upstream injecting bulk entries. Shrinkage and growth are separate gates because
  they catch opposite failures.
- `false_positive_gate(merged, control_set, exclusions, max_fp_rate, min_control_size) ->
  GateResult` — evaluates the merged list against a **known-good negative control**: the
  [Tranco](https://tranco-list.eu/) top-N list, pinned to a specific daily release like any other
  source. See the decision doc's ["A measured false-positive gate on every
  build"](../../../decisions/domain-blocklist-sourcing.md#3-a-measured-false-positive-gate-on-every-build)
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
