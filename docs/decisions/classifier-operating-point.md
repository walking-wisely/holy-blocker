# Decision: Classifier Operating Point

## What was decided

For the image classifier, **false negatives are the budget and false positives
are the price**. The deployed threshold is chosen by fixing an acceptable miss
rate and accepting whatever over-blocking that costs — not by maximising
accuracy.

Concretely, for the current fine-tuned model:

| max miss rate | threshold | resulting over-block rate |
|---|---|---|
| 10% | 0.44 | 7.1% |
| **5%** | **0.20** | **11.4%** |
| 2% | 0.07 | 19.7% |

The 5% row is the current default. **0.5 is not the threshold** and never was —
it is an artefact of `argmax` over two logits.

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
- The threshold is model-specific. It must be re-derived from the miss-budget
  table after any retraining, because scores are not calibrated across
  checkpoints.
