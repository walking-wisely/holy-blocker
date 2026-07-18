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
and `porn` (5.4%). Since [false negatives are the budget](../../../decisions/classifier-operating-point.md),
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

> **These numbers come from two different evaluation sets.** No baseline value
> below has been changed — this note records which set each was measured on,
> because the original table did not say, and reading them as one set
> manufactures a regression that is not there. See
> [Which set to score on](#which-set-to-score-on).

Validation split — 5,600 samples, `seed=0`, `val_fraction=0.2`:

| holdout | n | AUC to beat / preserve |
|---|---|---|
| photographic (`neutral`, `sexy`, `porn`) | 3,360 | **0.9844** — must not degrade |
| drawn (`drawings`, `hentai`) | 2,240 | **0.9530** — target of the experiment |
| combined | 5,600 | 0.9748 |
| FP rate at 5% miss budget | 5,600 | 13.2% |

Common holdout — the 1,147 samples held out from *both* the linear probe's and
the fine-tuned model's training, which is what [results.md](../results.md)
reports its headline figures on:

| holdout | n | AUC to beat / preserve |
|---|---|---|
| photographic | 692 | 0.9848 |
| drawn | 455 | 0.9566 |
| combined | 1,147 | 0.9766 |
| FP rate at 5% miss budget | 1,147 | **11.4%** |

### Which set to score on

The **decision rule below is evaluated on the validation split**, because that
is where its three AUC thresholds were fixed. The common holdout is reported
alongside as a secondary read.

The two are not interchangeable: the same checkpoint scores 0.9530 drawn on one
and 0.9566 on the other, and its FP rate at a 5% miss budget is 13.2% against
11.4%. Both are correct measurements of the same model — they differ because
the sets differ, not because anything changed. Scoring a new run on the
validation split and comparing its miss-budget figure to the 11.4% baseline
would show a ~1.8pp regression that is purely an artifact of the swap.

The common holdout is a strict subset of the validation split, so any run
sharing `seed=0` and `val_fraction=0.2` can be scored on both.

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

~~Run the **full unfreeze** first. Training accuracy of 94.6% says the current
model underfits, and an underfitting model is the wrong instrument for measuring
whether more data helps — extra data cannot be absorbed by a model that already
cannot fit what it has.~~ **Done** — see [full-unfreeze.md](full-unfreeze.md).

It changed the picture in two ways.

**The capacity hypothesis was confirmed, and it moved the drawn axis on its
own.** Drawn AUC improved by 0.0074 on the validation split and 0.0133 on the
common holdout with no new data — the latter larger than the +0.010 gain this
experiment set as its bar for accepting a *data* intervention. Photographic
improved too, and nothing regressed.

**The prerequisite is satisfied in practice but not on its own terms.** The
stated rationale was that "an underfitting model is the wrong instrument for
measuring whether more data helps." Training accuracy rose from 94.6% to 95.9%,
but a model that still cannot fit its own training data is still, strictly,
underfitting. The difference is that capacity is no longer the lever — there is
nothing left to unfreeze — so waiting for a fully-fitting model before running
this would mean waiting on an architecture change, not a cheaper knob. Run it,
and read a null drawn result as "data did not help *this* architecture" rather
than "data does not help."

**The baselines above are superseded.** They describe the unfreeze-3 model,
which is no longer the model to beat. The table below re-fixes them against the
full-unfreeze model. This is still a pre-registration: this experiment named the
full unfreeze as its prerequisite, so these numbers are fixed *before* the anime
run begins, and the decision rule and its thresholds are unchanged.

Validation split — the set the decision rule is evaluated on:

| holdout | n | AUC to beat / preserve |
|---|---|---|
| photographic | 3,360 | **0.9883** — must not degrade |
| drawn | 2,240 | **0.9604** — target of the experiment |
| combined | 5,600 | 0.9796 |
| FP rate at 5% miss budget | 5,600 | 10.1% |

Common holdout, reported alongside:

| holdout | n | AUC |
|---|---|---|
| photographic | 692 | 0.9881 |
| drawn | 455 | 0.9699 |
| combined | 1,147 | 0.9832 |
| FP rate at 5% miss budget | 1,147 | 8.2% |

Method step 3 changes accordingly: fine-tune with a **full unfreeze**, not
`--unfreeze 3`, so the comparison holds architecture fixed and varies only data.

This also raises the bar. The drawn sub-problem now starts at 0.9604 rather than
0.9530, and the remaining headroom to photographic has narrowed from 0.0314 to
0.0279 — so a +0.010 drawn gain from data alone is a larger share of what is
left to win than it was when this was written.
