# Experiment: does subsampled anime data improve the drawn axis?

**Status:** tooling built, protocol amended, run pending.
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
   **Amended before the run — see [Protocol amendment](#protocol-amendment).**
   As written this step admits only n=0, because the corpus is already exactly
   40% drawn. Substitution replaces addition.
2. **Map ratings to the binary policy.** `general` + `sensitive` → `safe`,
   `explicit` → `explicit`. ~~Hold `questionable` out of training entirely for
   the first run — its whole value is as an evaluation set for the boundary, and
   assigning it a side would re-import the arbitrariness being fixed.~~
   **Reversed before the run:** `questionable` → `safe`. Holding it out biases
   drawn AUC downward for reasons unrelated to label quality — see
   [`questionable` maps to safe](#questionable-maps-to-safe-reversing-method-step-2).
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

## Protocol amendment

Fixed **before** the run. The decision rule, its thresholds, and the baselines
are untouched.

An earlier version of this amendment proposed *substitution* — hold drawn volume
fixed and swap half of it for anime data. It was withdrawn after review, before
any run. The reasoning and the reason it fails are recorded in
[Withdrawn: the substitution amendment](#withdrawn-the-substitution-amendment),
because the failure is instructive.

### The defect in method step 1

Step 1 says to sample so the combined set holds 40% drawn / 60% photographic.
The corpus is 5,600 images in each of five classes: `drawings` + `hentai` =
11,200 of 28,000, which is **already exactly 40%**. Adding any drawn data pushes
it above 40%, so the rule as written is satisfied only by adding nothing.

But its purpose is stated in the risk table: *"Volume imbalance swamps
photographic content → subsample to the 40:60 ratio."* That is a guard against
the naive 40:1 mix, not a conservation law. The repair consistent with that
intent is to bound the drift rather than forbid it.

### The bound

**Drawn may grow but must not exceed photographic.** At 4,480 added images the
drawn share of the training half goes 40% → exactly 50%, and drawn never
outweighs photographic in a gradient step. Recorded here so the number is fixed
in advance rather than chosen once results are visible.

### Three arms

Addition alone cannot separate "better labels helped" from "more data helped",
so a control arm runs beside it. The two are symmetric around the baseline —
4,480 added, 4,480 removed — so their deltas share a scale.

| arm | drawn (train) | photographic (train) | drawn share | what it measures |
|---|---|---|---|---|
| **baseline** | 8,960 original | 13,440 | 40% | the full-unfreeze run, already measured |
| **A — addition** | 8,960 original + 4,480 anime | 13,440 | 50% | the pre-registered question |
| **B — ablation** | 4,480 original | 13,440 | 25% | how much drawn AUC depends on drawn volume at all |

Arm B is what makes arm A readable. If arm A moves drawn AUC by less than the
slope implied by arm B, the anime data is doing no more than generic volume
would; if it moves more, the labels are contributing something.

The validation half is 5,600 frozen original samples in every arm.

### `questionable` maps to safe, reversing method step 2

Step 2 held `questionable` out of training entirely. That is withdrawn, because
it biases the result in a way unrelated to what is being measured.

`drawings` is a **residual** class — every drawn image that is not `hentai` —
so on the Danbooru scale it spans `general`, `sensitive`, and much of
`questionable`. Holding `questionable` out of the anime safe class while the
validation set keeps such content inside `drawings` truncates the safe class's
borderline support on the training side only. The predicted effect is more false
positives on exactly the `drawings` images this experiment calls 62% of all
over-blocks — a drawn AUC drop **by construction**, with no bearing on label
quality.

Mapping it to safe is also the choice consistent with the established policy,
which already puts `sexy` (photographic suggestive) on the safe side.

The original rationale — that assigning it a side "would re-import the
arbitrariness being fixed" — mistakes the level. The arbitrariness this dataset
fixes is *per-image*: `drawings` commits this content to the safe side wholesale,
while Danbooru rates each image against published criteria. Keeping
`questionable` and labelling it safe preserves that per-image judgement in the
training signal; dropping it discards the images the experiment most wants.

### Recorded deviation

Method step 3's "identical hyperparameters" means those of the full-unfreeze
run, as the Prerequisite section established.

### What this does not fix

**A single run per arm cannot decide the rule.** The thresholds are ±0.003 to
0.010; run-to-run variance from the training seed alone is plausibly the same
size. Against sampling noise the thresholds are defensible — the drawn AUC
standard error at n=2,240 and AUC 0.9604 is 0.0042 (Hanley–McNeil), so a paired
+0.010 is roughly 3σ at a score correlation of 0.7 — but seed variance is
unmeasured and unaccounted for.

Three seeds per arm would fix this and is the right design; it is not what is
being run. **The verdict from this run is therefore a screen, not a test.** A
result near a threshold should be read as "worth replicating", never as
"accepted" or "rejected". Recorded in advance so the write-up cannot quietly
claim more than the design supports.

## Interim result: arm B, and what it says about the accept bar

**Recorded before arm A was run.** Arm B finished first because it needs no
supplement. Its numbers change how arm A must be read, so they are written down
while arm A's are still unknown.

Arm B — 4,480 drawn training images removed, nothing added, everything else
identical:

| | baseline | arm B | Δ |
|---|---|---|---|
| photographic (val) | 0.9883 | 0.9883 | **0.0000** |
| drawn (val) | 0.9604 | 0.9458 | **−0.0146** |
| combined (val) | 0.9796 | 0.9757 | −0.0039 |
| FP at 5% miss (val) | 10.1% | 13.5% | +3.4pp |
| drawn (common) | 0.9699 | 0.9466 | −0.0233 |

### Two things this establishes

**The medium invariant holds empirically, not just by construction.**
Photographic AUC is *identical to four decimal places* after removing a quarter
of the training set. The drawn and photographic sub-problems really do occupy
near-disjoint regions, which is what makes the decision rule's "preserve
photographic" clause measurable at all.

**The volume slope, which is the scale arm A has to be read against.** Halving
the drawn training half costs 0.0146 drawn AUC. Fitting the usual log-linear
learning curve (AUC deficit linear in log n) through the two points gives
0.0211 AUC per natural-log unit of drawn training volume.

Arm A adds 4,480 images, taking drawn volume 8,960 → 13,440. That is +0.406
log-units, so the same curve predicts:

> **+0.0085 drawn AUC — if the anime data were exactly as useful as more
> in-distribution data.**

### The accept bar may be unreachable, and that is not a result about anime

The pre-registered accept threshold is **+0.010**. The extrapolation above puts a
*perfect* 50% data increase at **+0.0085**, below it. Anime data is
out-of-distribution relative to a validation set made entirely of `drawings` and
`hentai`, so the realistic ceiling is lower still.

So arm A can very plausibly return "inconclusive" while the anime data is doing
everything data of its size could do. The threshold was fixed when the cost of a
data intervention was unknown; arm B is the first measurement of that cost, and
it suggests +0.010 asks for more than +4,480 images of *any* provenance can
deliver.

Consequences, fixed now:

- A drawn gain in the **+0.005 to +0.010** band must not be written up as a
  failure of the anime data. Against the slope it is close to the ceiling for
  this volume, and the honest reading is "the intervention worked about as well
  as data can at this scale, and the bar was set above that scale."
- A gain **at or above +0.0085** means the anime data is performing at least as
  well per image as in-distribution data — the strongest outcome available, and
  a direct answer to the label-quality question, whether or not it clears
  +0.010.
- A gain **near zero or negative** is the genuine null: the data is not
  contributing even what its volume would predict.

The decision rule itself is **unchanged** — it still reports accept / reject /
inconclusive on its original thresholds. This section constrains the *narrative*
around a verdict, not the verdict.

### Caveats on the extrapolation

One interval, two points, one seed. The log-linear form is a convention, not a
law, and the slope is measured on the *downward* side where the curve is
steepest — so +0.0085 is more likely an over-estimate than an under-estimate of
what +50% buys. It is a calibration for reading a number, not a prediction to be
scored.

### Withdrawn: the substitution amendment

The first amendment held drawn volume fixed at 11,200 and replaced 4,480
original drawn training images with anime ones. The argument was that adding
data confounds label quality with volume, so holding volume fixed isolates
label quality.

It is wrong, for three reasons:

1. **It confounds worse, not better.** Substitution changes label provenance,
   visual domain, *and* in-distribution drawn volume simultaneously. Volume is
   at least monotone and ablatable; domain shift is neither.
2. **It is biased toward a decline.** The validation half is 100% original
   `drawings`/`hentai`. Halving matched training data and replacing it with
   mismatched data should lower in-distribution drawn AUC on priors, making the
   +0.010 accept bar close to unreachable and the modal outcome a drop.
3. **It answers a different question.** The pre-registration asks whether
   *adding* better-labelled drawn data helps. A null from substitution would
   have been written up as "data did not help" — a claim the run never tested.

The general lesson: finding a real defect in a protocol does not license
replacing the question. The minimal repair consistent with the stated intent is
the one to make.

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

## How to run it

`CORPUS=data/eval/.archive/nsfw_dataset_v1.zip` throughout.

```sh
# Inspect either arm's plan without fetching anything.
holy-blocker-anime --archive $CORPUS --dry-run                          # arm A
holy-blocker-anime --archive $CORPUS --anime-count 0 --drop-fraction 0.5 --dry-run

# Build the supplement for arm A. Ranged reads pull only the selected members,
# so the 68 GB of rating archives is never downloaded whole.
holy-blocker-anime --archive $CORPUS --out data/eval/anime_supplement.zip

# Arm A — addition.
holy-blocker-finetune --archive $CORPUS \
                      --supplement data/eval/anime_supplement.zip \
                      --output-dir artifacts/anime-addition \
                      --epochs 6 --backbone-lr 1e-4 --head-lr 1e-3

# Arm B — ablation control. No supplement; --drop-fraction alone builds the plan.
holy-blocker-finetune --archive $CORPUS --drop-fraction 0.5 \
                      --output-dir artifacts/anime-ablation \
                      --epochs 6 --backbone-lr 1e-4 --head-lr 1e-3

# Score each arm on the frozen holdouts — the command the baselines came from.
holy-blocker-score --archive $CORPUS --common-idx data/eval/common_idx.npy \
                   --checkpoint artifacts/anime-addition/finetuned-v0.pt
```

`anime_dbrating` is ungated, so no `HF_TOKEN` is needed — unlike `nsfw_detect`.

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
