# Experiments

Pre-registered experiments on the image classifier. Each page fixes its
question, baselines, and decision rule **before** the run, so a result cannot be
reinterpreted once the numbers are visible.

Headline model numbers live in [../results.md](../results.md); these pages hold
the reasoning, the arms, and the verdicts.

## Index

| experiment | question | status | verdict |
|---|---|---|---|
| [full-unfreeze](full-unfreeze.md) | Does unfreezing the whole backbone beat unfreezing three blocks? | run | **Accepted** — drawn 0.9530 → 0.9604, photographic 0.9844 → 0.9883, nothing regressed |
| [anime-subsampling](anime-subsampling.md) | Does drawn training data from a better-labelled source close the drawn/photographic gap? | run | **Inconclusive — not adopted.** Drawn 0.9604 → 0.9526; the added data was net-harmful |

## The current model

The full-unfreeze checkpoint: **0.9796** combined ROC-AUC on the validation
split, **0.9832** on the common holdout. The anime experiment did not displace
it.

## How these are run

**Baselines are frozen against a fixed split.** Every arm uses
`stratified_split(seed=0, val_fraction=0.2)` over the source archive, and the
validation half is never modified — an arm may only change the *training* half.
`holy-blocker-score` regenerates both holdouts from the archive so any published
figure can be reproduced, and it fails loudly if the common holdout ever escapes
the validation split.

**Amendments are recorded, not silently applied.** Where a protocol turned out
to be unrunnable as written, the defect, the repair, and the reasoning are kept
on the page — including repairs that were themselves withdrawn. The anime page
carries a
[withdrawn amendment](anime-subsampling.md#withdrawn-the-substitution-amendment)
and a [withdrawn extrapolation](anime-subsampling.md#what-arm-b-does-not-license)
for exactly this reason.

**Predictions are stated in advance and scored afterwards.** Running tally:
**three of five wrong**. The record is kept because the failures have been more
informative than the hits — the most recent one assumed better-labelled data
could not carry *negative* marginal value.

## Standing limitations

These apply to every result here and are not restated on each page.

- **One seed per arm.** Run-to-run variance from the training seed is plausibly
  0.003–0.010 AUC, which is the same order as the decision thresholds. Verdicts
  are therefore **screens, not significance tests**; a result near a threshold
  means "worth replicating." Three seeds per arm would fix this and has not been
  run.
- **One corpus.** `deepghs/nsfw_detect` is a scraped taxonomy, not a sample of
  real traffic. Nothing here establishes transfer.
- **Ranking, not calibration.** Scores are used as an ordering. They are not
  known to be probabilities.

## Adding an experiment

1. Write the page first: question, why it might not be worth doing, data,
   method, **frozen baselines**, decision rule, prediction, risks, and what to
   do if it fails.
2. Commit it *before* running anything.
3. Run the arms; score with `holy-blocker-score` against the frozen holdouts.
4. Add the result, apply the decision rule as written, score the prediction, and
   update the table above and [../results.md](../results.md).

Include a control arm wherever a result could be explained by something other
than the intervention. The anime experiment's ablation arm is the worked
example: it is what turned "drawn AUC went down" into "the added data was
net-harmful," a conclusion the two-arm design could not have reached.
