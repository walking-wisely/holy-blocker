# Baseline classifier — measured results

Measurements for the MobileNetV3-Small binary classifier, July 2026. Reproduce
with the commands in [`machine-learning/README.md`](../../../machine-learning/README.md).

Numbers here are measured, not estimated. Where something is inferred or
unverified it says so.

Pre-registered experiments — their arms, decision rules, and verdicts — are
indexed in [experiments/](experiments/README.md).

## Corpus

[`deepghs/nsfw_detect`](https://huggingface.co/datasets/deepghs/nsfw_detect) —
28,000 images, 5,600 per class, MIT licensed, gated with automatic approval.
Layout is `nsfw_dataset_v1/<class>/<image>`; the classes are `neutral`,
`drawings`, `sexy`, `hentai`, `porn`.

Collapsed to binary by `features.DEFAULT_LABEL_POLICY`: `hentai` and `porn` are
`explicit`, the rest `safe`. That mapping is the false-positive/false-negative
definition — see [the operating point decision](../../decisions/classifier-operating-point.md).

No imagery is retained. The corpus is converted once into feature vectors
(58 MB `.npz` from a 1.7 GB archive) and the archive is deleted; all evaluation
below runs on vectors.

## Models

| Model | Description | Artifact |
|---|---|---|
| baseline | Linear probe: frozen ImageNet backbone, trained head | `linear-probe-v0.pt` |
| fine-tuned | Last 3 feature blocks unfrozen, 6 epochs, cosine LR | `finetuned-v0.pt` |

## Headline numbers

Measured on **1,147 samples held out from both models' training**. The two
models used different splits, so this is their intersection — earlier
side-by-side figures in the commit history are not strictly comparable.

| | baseline | fine-tuned |
|---|---|---|
| ROC-AUC | 0.9707 | **0.9766** |
| PR-AUC | 0.9579 | **0.9684** |
| confident errors | 36.9% | **16.8%** |

**Separation is strong.** An accuracy of ~92% at threshold 0.5 understates the
model badly: it ranks explicit above safe 97.7% of the time. The gap between
those two figures is threshold placement, not capability.

## Cost of a miss budget

False negatives are the failure that matters, so the miss rate is the budget and
over-blocking is the price. Read this table, not accuracy.

| max FN | baseline FP | fine-tuned FP | threshold |
|---|---|---|---|
| 20% | 4.2% | **2.9%** | 0.73 |
| 10% | 8.5% | **7.1%** | 0.44 |
| 5% | 15.3% | **11.4%** | 0.20 |
| 2% | 24.7% | **19.7%** | 0.07 |
| 0% | 50.4% | 70.1% | 0.00 |

Fine-tuning wins at every usable budget — at a 5% miss rate it cuts
over-blocking by a quarter. **It loses badly at 0%**: a handful of explicit
images receive very low scores, so the extreme tail regressed. Zero-miss is not
a usable target for either model at 50%+ over-blocking.

## Per-class difficulty

AUC of each class against the opposite-label pool, fine-tuned model, threshold
free:

| class | AUC | role |
|---|---|---|
| `neutral` | 0.9885 | easiest |
| `sexy` | 0.9797 | |
| `porn` | 0.9760 | |
| `hentai` | 0.9735 | |
| `drawings` | 0.9561 | hardest |

By medium, as independent sub-problems:

| medium | n | AUC |
|---|---|---|
| photographic (`neutral`, `sexy`, `porn`) | 3,360 | 0.9844 |
| drawn (`drawings`, `hentai`) | 2,240 | 0.9530 |

## Where the errors are

At threshold 0.20 (the 5% miss budget):

- **False negatives, 127 total** — `hentai` 67 (6.0% of class), `porn` 60 (5.4%).
  Split roughly evenly. Drawn explicit content is *not* meaningfully harder to
  catch than photographic explicit content.
- **False positives, 397 total** — `drawings` 247 (62%), `sexy` 107 (27%),
  `neutral` 43 (11%).

The drawn weakness is concentrated in false positives — over-blocking legitimate
artwork — not in misses.

## Findings

**Accuracy at 0.5 is the wrong headline.** It conflates two errors of different
cost and moves when the score distribution shifts even if ranking is unchanged.
It also selected the wrong fine-tuning checkpoint: epoch 6 (FP 5.3% / FN 11.7%)
scored higher than epoch 5's better-balanced 7.2% / 9.2%.

**The model underfits; it does not overfit.** Train/validation gap is 2.4pp
accuracy (94.6% vs 92.2%), 0.013 AUC. Training accuracy below 95% means it
cannot fit even the training data — three unfrozen blocks is a capacity limit. A
full unfreeze is the cheapest remaining improvement.

> **Confirmed.** The full unfreeze was run and improved every reported metric on
> both evaluation sets, with nothing regressing: drawn AUC 0.9530 → 0.9604,
> photographic 0.9844 → 0.9883, and the FP rate at a 5% miss budget 13.2% →
> 10.1% on the validation split. See
> [experiments/full-unfreeze.md](experiments/full-unfreeze.md). The numbers
> throughout this document describe the unfreeze-3 model and are kept as the
> record it was measured against.

**More drawn data does not fix the drawn axis — and the wrong drawn data makes
it worse.** Adding 4,480 subsampled `anime_dbrating` images to the drawn
training half *lowered* drawn AUC 0.9604 → 0.9526, while an ablation control
that removed 4,480 in-distribution drawn images lowered it to 0.9458. Since
removing volume hurts, volume has positive marginal value, so the anime data's
contribution net of volume is negative rather than merely absent. Inconclusive
by the experiment's pre-registered rule; nothing supports adopting it. See
[experiments/anime-subsampling.md](experiments/anime-subsampling.md).

**Two confusion axes, not one.** A 5-way probe on the same features shows
`drawings→hentai` (64% of drawings errors) and `sexy→porn` (62% of sexy errors)
dominate, while cross-medium confusion is near zero (`drawings→sexy` = 1). The
model separates *medium* almost perfectly and struggles with *explicitness
within* each medium — which is exactly the binary task.

**Label noise is not the established ceiling.** It was hypothesised, and the
evidence is against it being dominant: fine-tuning halved the confident-error
share (36.9% → 16.8%), which label noise would not permit. Not fully ruled out;
no relabelling study has been run.

## Not established

- ~~Whether a full unfreeze closes the remaining gap.~~ **Done** — it narrows it
  (drawn-to-photographic 0.0314 → 0.0279) but does not close it. See
  [experiments/full-unfreeze.md](experiments/full-unfreeze.md).
- The label-noise rate. No annotator agreement study or relabelling pass exists.
- Whether *any* better-labelled drawn corpus helps. The anime experiment showed
  that one corpus, mapped one way, at one volume, does not — most likely domain
  mismatch rather than label quality. It did not test the general claim.
- Whether per-medium routing beats mixing. It is the follow-up the anime
  experiment's failure branch points to, and nothing has been built.
- Whether these numbers transfer to real traffic. This corpus is a scraped
  taxonomy, not a sample of what users actually encounter.
- Calibration. Scores are used as a ranking; they are not known to be
  probabilities.

## Reproducing

```bash
holy-blocker-extract --out data/eval/nsfw_detect.npz
holy-blocker-finetune --archive <corpus.zip> --epochs 6 --unfreeze 3
holy-blocker-extract --from-checkpoint artifacts/finetuned-v0.pt --archive <corpus.zip> \
  --out data/eval/nsfw_detect_ft.npz
holy-blocker-eval --checkpoint artifacts/finetuned-v0.pt --features data/eval/nsfw_detect_ft.npz
```
