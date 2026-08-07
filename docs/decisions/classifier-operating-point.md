# Decision: Classifier Operating Point

## What was decided

For the image classifier, **false negatives are the budget and false positives
are the price**. The deployed threshold is chosen by fixing an acceptable miss
rate and accepting whatever over-blocking that costs — not by maximising
accuracy.

The miss rate is fixed as a budget and the threshold that achieves it is
derived from a miss-budget sweep against a held-out evaluation set, not chosen
by eye. **0.5 is not the threshold** and never was — it is an artefact of
`argmax` over two logits.

### A threshold belongs to a model *and* a geometry, and there is no built-in default

Both halves of that are load-bearing, and getting it wrong has already happened
here: an earlier constant baked into `packages/image-sandbox` had been carried
over from a superseded checkpoint whose scores were never comparable to the one
actually shipped, and the same checkpoint's threshold moves substantially
between a centre-crop and a tile-max geometry, because taking a max over
overlapping tiles shifts the whole score distribution upward. Applying a
threshold measured under one pairing to a different model or a different
geometry silently changes the operating point, in the direction of either
missing content or over-blocking, with no error anywhere.

How little this transfers is measurable: two checkpoints from an identical
training recipe and seed, differing only in run-to-run noise, agreed on ranking
to within 0.0005 AUC while their 5%-miss thresholds differed by over 25%. Two
equally good models can need very different cuts.

**The threshold is therefore a required runtime configuration value, not a
constant recorded in code or in this document.** `SandboxConfig` and every
caller across it (the `mitm-proxy` CLI, the `image-sandbox-ffi` UniFFI surface,
the macOS daemon's `HOLY_BLOCKER_IMAGE_THRESHOLD`) require it explicitly and
supply no fallback; a caller that omits it gets no image scanning at all rather
than a silently wrong cut.

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
  thresholds — strong separation with a high AUC against a middling
  fixed-threshold accuracy figure is the whole point: 0.5 is simply a bad
  operating point.
- A miss-budget sweep for choosing the deployed threshold, described above.
- **Error confidence** as a label-noise diagnostic: a model that is confidently
  wrong is being contradicted by its labels, not running out of capacity.

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

- A meaningful share of safe content is over-blocked at any usable miss budget,
  concentrated in illustrated artwork. That is a real product cost.
- The `warn` mode matters more under this decision than it would under a
  balanced one: at a non-trivial over-block rate, a pass-through path for
  ambiguous verdicts is doing real work.
- The threshold is model- **and geometry-** specific. It must be re-derived
  after any retraining *or* any change to how images are fitted into the
  model's input, because scores are not calibrated across either.
- **Below the shorter-side size floor, nothing is classified at all** — the
  verdict is `Allow` without inference. That is a deliberate coverage decision,
  not a threshold: no operating point applies, because the model is never
  consulted. Content served under that size is unfiltered.

## The screen path operates outside this measurement

**Added when `image-sandbox` was wired into the macOS daemon (module 18).**
Whatever threshold is configured is calibrated on a corpus of *images*. The
daemon hands the same classifier, under the same tile-max geometry, a **screen
frame** — and no measurement in this repository covers that distribution.

The two differ in ways that plausibly move the operating point in opposite
directions:

- A screen frame is mostly application chrome. Tile-max was adopted precisely so
  a small explicit region in a large safe frame still blocks, so the geometry is
  right for this shape — but "small region, large safe background" is the
  *typical* case on a screen rather than the tail case it was measured as.
- Screen content is rendered rather than photographed: browser UI, text, flat
  colour fields. Nothing in the training corpus looks like a toolbar, and the
  over-block rate at any usable image threshold is already concentrated in
  drawn imagery.

Neither effect is estimated, because estimating them needs a corpus of screen
frames and this project deliberately does not collect one. The daemon therefore
reports the score on every verdict — including allows — so the margin under
the configured cut is observable in the log line rather than invisible.
`ImageOutcome.Allow` carries an *optional* score for the same reason: a frame
that never reached the model is distinguishable from one the model scored at
zero.

**What would close this:** an operating point re-derived against screen
frames, which needs a labelled corpus of them. Until then, whatever threshold
is configured is a transfer from a neighbouring distribution, not one measured
for this path.

## The contract is now three classes, not two

**Added after live testing surfaced both failure modes this decision predicts.**
Running the two-class contract against real screen content produced both
directions of error the sections above warn about in the abstract: false
positives on ordinary non-sexual photos, and false negatives on illustrated
content the two-class model had no way to name — the safe/explicit cut has no
room for "suggestive but not explicit," and the specific checkpoint tested had
no exposure to drawn/anime style at all.

`packages/image-sandbox`'s classifier contract is therefore `safe`/`sexy`/
`explicit` (`SAFE_INDEX`/`SEXY_INDEX`/`EXPLICIT_INDEX` = 0/1/2), and
`SandboxConfig` takes **two** thresholds — `sexy_threshold` and
`explicit_threshold` — compared independently against tiles reduced per class,
with `explicit_threshold` checked first so a tile clearing both bars blocks
rather than warns. Everything in the "no built-in default" section above
applies to both: neither has a fallback, and both must be re-derived together
for a given model and geometry, not carried over independently.

This does not close the screen-path gap two sections up — it only gives the
warn tier a real class to draw its line against instead of the daemon
inventing one. No operating point has been measured for either threshold
against screen content, same as before.
