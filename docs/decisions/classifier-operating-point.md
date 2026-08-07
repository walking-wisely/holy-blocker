# Decision: Classifier Operating Point

## What was decided

For the image classifier, **false negatives are the budget and false positives
are the price**. The deployed threshold is chosen by fixing an acceptable miss
rate and accepting whatever over-blocking that costs — not by maximising
accuracy.

Concretely, for the **deployed full-unfreeze model under the tile-max geometry**,
on the validation split:

| max miss rate | threshold | resulting over-block rate |
|---|---|---|
| 10% | 0.7523 | 4.94% |
| **5%** | **0.4650** | **10.09%** |
| 2% | 0.1634 | 19.67% |

The 5% row is the current default. **0.5 is not the threshold** and never was —
it is an artefact of `argmax` over two logits.

### A threshold belongs to a model *and* a geometry

Both halves of that are load-bearing, and getting it wrong has already happened
here. `packages/image-sandbox` shipped a provisional **0.20**, which was the
5%-miss threshold of the superseded *unfreeze-3* model — a different checkpoint
whose scores were never comparable. The deployed checkpoint's centre-crop
threshold is **0.2717**, and under tile-max it is **0.4650**, because taking a
max over overlapping tiles shifts the whole score distribution upward. Applying
any of those three numbers to the wrong pairing silently changes the operating
point, in the direction of either missing content or over-blocking, with no
error anywhere.

How little this transfers is measurable: the full-unfreeze run was
[replicated](../components/machine-learning/experiments/full-unfreeze.md#replicated)
from scratch under an identical recipe and seed, and the two checkpoints agree on
ranking to within 0.0005 AUC while their 5%-miss thresholds differ by 27% (0.2717
against 0.1980). Two equally good models can need very different cuts.

Superseded values, kept so an old number found in code can be identified rather
than guessed at:

| model | geometry | 5%-miss threshold |
|---|---|---|
| unfreeze-3 | centre crop | 0.20 |
| full-unfreeze | centre crop | 0.2717 |
| **full-unfreeze** | **tile-max** | **0.4650** |

## Why the miss rate is the budget

The two errors are not symmetric for this product.

A false positive blocks something harmless. The user sees an interstitial, is
mildly annoyed, and — under [protection modes](protection-modes.md) — can pass
through in `warn`. It costs trust, and trust is recoverable.

A false negative delivers exactly the content the user asked to be shielded
from. It is the single failure the product exists to prevent, and no other part
of the system compensates for it.

So the miss rate is what gets specified, and the over-blocking rate is reported
as its cost. This inverts the usual framing, where a model is tuned for accuracy
and the error split falls out incidentally.

## Why accuracy is not the headline metric

Three concrete failures observed while building this:

1. **It hides the asymmetry.** 92% accuracy says nothing about which errors were
   made. The same figure covers a model missing 3% and over-blocking 13%, or the
   reverse — which are very different products.
2. **It moves when the score distribution shifts, even if ranking does not.**
   Across fine-tuning epochs, accuracy stayed near 90–92% while the FP/FN split
   swung from 8.1/11.8 to 5.3/11.7. Comparisons at a fixed threshold read those
   shifts as quality changes when they were not.
3. **It selected the wrong checkpoint.** Best-accuracy selection picked epoch 6
   (FP 5.3% / FN 11.7%) over epoch 5 (7.2% / 9.2%), which is better under this
   decision.

Accuracy is still reported. It is not what anything is chosen by.

## What is used instead

- **ROC-AUC / PR-AUC** for comparing models. Ranking metrics are invariant to
  where the cut sits, so two models are comparable without arguing about
  thresholds. The current model's 0.9766 AUC against ~92% accuracy is the whole
  point: separation is strong, 0.5 is simply a bad operating point.
- **`fpr_at_fnr`** for choosing the deployed threshold — the table above.
- **Error confidence** as a label-noise diagnostic: a model that is confidently
  wrong is being contradicted by its labels, not running out of capacity.

Implemented in [`metrics.py`](../../machine-learning/src/holy_blocker_ml/metrics.py).

## What was rejected

**Maximising accuracy or F1.** Both bake in a symmetry that does not hold here.
F-beta with β > 1 would encode the asymmetry, but a miss-rate budget states the
same preference in units that can be reasoned about directly — "5% of explicit
content gets through" is a product decision; "β = 2" is not.

**Zero misses.** Achievable, at 70% over-blocking for the fine-tuned model. That
is not a usable product, and the extreme tail is where the fine-tuned model is
*worse* than the baseline. There is no threshold at which misses are free.

**A fixed 0.5 threshold.** It is the default only because `argmax` over two
logits implies it, and it happens to sit near the worst part of the trade for an
FN-averse product.

## Consequences

- Roughly **11% of safe content is over-blocked** at the default. Most of it is
  illustrated artwork — `drawings` is 62% of all false positives. That is a real
  product cost and the motivation for the
  [anime subsampling experiment](../components/machine-learning/experiments/anime-subsampling.md).
- The `warn` mode matters more under this decision than it would under a
  balanced one. At an 11% over-block rate, a pass-through path for ambiguous
  verdicts is doing real work.
- The threshold is model- **and geometry-** specific. It must be re-derived from
  the miss-budget table after any retraining *or* any change to how images are
  fitted into the 224×224 input, because scores are not calibrated across either.
- **Below 96px on the shorter side, nothing is classified at all** — the verdict
  is `Allow` without inference. That is a coverage decision made in
  [experiments/input-handling.md](../components/machine-learning/experiments/input-handling.md),
  and it is not a threshold: no operating point applies, because the model is
  never consulted. Content served under that size is unfiltered.

## The screen path operates outside this measurement (added 2026-08-08)

Every figure above is measured on a corpus of *images*. The macOS daemon hands
the same classifier, under the same tile-max geometry, a **screen frame** — a
distribution nothing in this repository covers. The two differ in ways that
plausibly move the operating point in opposite directions: a screen frame is
mostly application chrome, so "small region in a large safe background" becomes
the typical case rather than the tail case tile-max was measured on; and screen
content is rendered rather than photographed, while the over-block rate at any
usable threshold is already concentrated in drawn imagery.

**A labelled corpus of explicit screen frames would close this, and it will
never exist** — [image-corpus-custody.md](image-corpus-custody.md) rules out
acquiring one, and nothing else would serve.

What replaces it is a **paired transport-shift measurement**: `s(X)` against
`s(pipeline(X))` over benign imagery composited into rendered UI. Because the
pipeline's geometry — crop, scale, composite, overlay — is content-blind, a
shift measured on benign images describes what it does to *any* image. A shift
measured near zero licenses transferring the checkpoint's own operating point
onto the screen path; a shift that is not near zero is the quantified loss.

That is an argument resting on one measurement, not a measurement of the thing
itself, and it is labelled as such wherever it is used. The harness is planned
in
[machine-learning/plan.md](../components/machine-learning/plan.md#synth_compositepy-and-transportpy--the-screen-path-measurement);
the work it gates is in
[image-sandbox/plan.md](../components/image-sandbox/plan.md#the-screen-path).

Until it runs, the daemon reports the score on **every** verdict including
allows, so the margin under the configured cut is observable rather than
invisible — and an allow that never reached the model stays distinguishable
from one the model scored at zero.
