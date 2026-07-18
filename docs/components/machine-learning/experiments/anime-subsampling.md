# Experiment: does subsampled anime data improve the drawn axis?

**Status:** planned, not run.
**Pre-registered:** criteria and baselines below were fixed *before* running, so
the result cannot be reinterpreted after the fact.

## Question

The drawn sub-problem scores 0.9530 AUC against 0.9844 for photographic
([results](../results.md)). Does adding drawn training data from a
better-labelled source close that gap — **without degrading photographic
performance**?

## Why this is not obviously worth doing

Recorded so the result is judged against the real motivation, not a
retrospective one.

The drawn weakness sits almost entirely in **false positives** — `drawings` is
62% of over-blocks — while misses split evenly between `hentai` (6.0% of class)
and `porn` (5.4%). Since [false negatives are the budget](../../decisions/classifier-operating-point.md),
this experiment targets the error kind that matters *less*.

It is still worth running because over-blocking artwork at 62% of all false
positives is the largest single trust cost in the product, and because the
alternative (per-medium routing) is more expensive to build.

## Data

[`deepghs/anime_dbrating`](https://huggingface.co/datasets/deepghs/anime_dbrating)
— ungated, CC-BY-4.0, 1.2M+ images, four **ordinal** Danbooru ratings:
`general`, `sensitive`, `questionable`, `explicit`.

Two properties the current corpus lacks:

- `questionable` is an explicit boundary class. The current data forces every
  drawn image to safe-or-explicit, so ambiguous cases are assigned arbitrarily —
  precisely where `drawings↔hentai` errors live.
- Labels come from community moderation against published criteria rather than
  subreddit provenance.

Caveat: CC-BY-4.0 covers the curation, not the underlying third-party artwork.

## Method

1. **Subsample to preserve medium balance.** Current corpus is 40% drawn / 60%
   photographic. Sample `anime_dbrating` so the combined set holds that ratio.
   Naively mixing 1.2M against 28k is 40:1 and would let drawn content dominate
   gradient updates in a 2.5M-parameter backbone.
2. **Map ratings to the binary policy.** `general` + `sensitive` → `safe`,
   `explicit` → `explicit`. Hold `questionable` out of training entirely for the
   first run — its whole value is as an evaluation set for the boundary, and
   assigning it a side would re-import the arbitrariness being fixed.
3. Fine-tune with identical hyperparameters to the current run (`--unfreeze 3`,
   6 epochs, cosine, backbone LR 1e-4 / head 1e-3) so the only variable is data.
4. Score against the **frozen holdouts** below.

## Pre-registered baselines

Fixed from the current fine-tuned model. Do not recompute these after the run.

| holdout | n | AUC to beat / preserve |
|---|---|---|
| photographic (`neutral`, `sexy`, `porn`) | 3,360 | **0.9844** — must not degrade |
| drawn (`drawings`, `hentai`) | 2,240 | **0.9530** — target of the experiment |
| combined | 5,600 | 0.9748 |
| FP rate at 5% miss budget | — | 11.4% |

## Decision rule

Fixed in advance.

**Accept** if drawn AUC improves by ≥ 0.010 **and** photographic AUC drops by
≤ 0.003.

**Reject** if photographic AUC drops by > 0.005, regardless of drawn gain. The
photographic classes carry the error kind that matters most; a drawn improvement
does not buy a photographic regression.

**Inconclusive** otherwise — treat as a null result and do not adopt. A change
this cheap to run should not be adopted on an ambiguous reading.

Report all three AUCs and the miss-budget table either way, including on
rejection.

## Prediction

Stated in advance because two earlier predictions in this work were wrong.

Photographic performance will move by less than 0.003 — cross-medium confusion
is already near zero (`drawings→sexy` = 1 of 5,600), so the sub-problems occupy
largely disjoint regions of feature space and drawn data should land in the
drawn region. Drawn AUC will improve, but by less than the 0.010 threshold,
because the binding constraint is capacity (training accuracy is 94.6%, i.e.
underfitting) rather than data volume.

If that prediction holds, the experiment is **inconclusive by its own rule** and
the correct follow-up is a full unfreeze, not more data.

## Risks

| risk | mitigation |
|---|---|
| Volume imbalance swamps photographic content | Subsample to the 40:60 ratio |
| Label semantics differ between corpora | Hold `questionable` out; document the mapping |
| Shared backbone capacity is finite | Compare against the full-unfreeze run before concluding data was the cause |
| 63 GB download | Take a stratified subset; do not materialise the whole corpus |

## If it fails

Per-medium **routing** instead of mixing. Medium is ~99% separable, so a cheap
drawn-vs-photographic classifier plus one specialist per medium removes the
interference entirely. Cost is two on-device models — 2 × 6 MB against the 15 MB
budget in `TrainingConfig.max_model_mb`, which fits.

## Prerequisite

Run the **full unfreeze** first. Training accuracy of 94.6% says the current
model underfits, and an underfitting model is the wrong instrument for measuring
whether more data helps — extra data cannot be absorbed by a model that already
cannot fit what it has.
