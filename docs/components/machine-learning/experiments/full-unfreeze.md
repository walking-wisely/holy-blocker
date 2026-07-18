# Experiment: does unfreezing the whole backbone beat unfreezing three blocks?

**Status:** run. Accepted.

Prerequisite for the
[anime subsampling experiment](anime-subsampling.md), which requires a model
that is not capacity-limited before it can measure whether more data helps.

## Question

The fine-tuned model reached 94.6% training accuracy against 92.2% validation.
Training accuracy below 95% means the model cannot fit even the data it has
seen, which is a capacity limit rather than a data limit. Does removing that
limit — unfreezing all of the backbone instead of the last three blocks —
improve the drawn and photographic sub-problems?

## Method

Identical to the fine-tuning run in every respect except unfreeze depth:
6 epochs, cosine schedule, backbone LR 1e-4, head LR 1e-3, batch 32,
`seed=0`, `val_fraction=0.2`. Only `--unfreeze` was dropped, taking trainable
tensors from 33 to 142.

The shared seed and validation fraction matter: they make the validation split
sample-for-sample identical to the baseline run's, so the comparison is not
confounded by a different split.

```
holy-blocker-finetune --archive <corpus>.zip --output-dir artifacts/unfreeze-full \
  --epochs 6 --head-lr 1e-3 --backbone-lr 1e-4 --batch-size 32
holy-blocker-score --archive <corpus>.zip --common-idx data/eval/common_idx.npy \
  --checkpoint artifacts/unfreeze-full/finetuned-v0.pt
```

Best epoch was 5 of 6, selected by validation accuracy. Checkpoint size is
unchanged at 5.92 MB — unfreezing changes which weights move, not how many
exist, so the 15 MB on-device budget is unaffected.

## Results

Both evaluation sets are reported, because the baselines for this model were
[recorded across two of them](anime-subsampling.md#which-set-to-score-on).

Validation split, 5,600 samples:

| metric | unfreeze 3 | full unfreeze | Δ |
|---|---|---|---|
| photographic AUC | 0.9844 | **0.9883** | +0.0039 |
| drawn AUC | 0.9530 | **0.9604** | +0.0074 |
| combined AUC | 0.9748 | **0.9796** | +0.0048 |
| accuracy @ 0.5 | 0.9216 | **0.9311** | +0.0095 |
| FP at 5% miss budget | 13.24% | **10.09%** | −3.15pp |

Common holdout, 1,147 samples held out from every model's training:

| metric | unfreeze 3 | full unfreeze | Δ |
|---|---|---|---|
| photographic AUC | 0.9848 | **0.9881** | +0.0033 |
| drawn AUC | 0.9566 | **0.9699** | +0.0133 |
| combined AUC | 0.9766 | **0.9832** | +0.0066 |
| accuracy @ 0.5 | 0.9119 | **0.9346** | +0.0227 |
| FP at 5% miss budget | 11.42% | **8.24%** | −3.18pp |

Nothing regressed on either set.

Per-source error rate at threshold 0.5, validation split, at each run's best
epoch:

| source | unfreeze 3 | full unfreeze |
|---|---|---|
| `drawings` | 10.5% | 12.4% |
| `hentai` | 13.1% | **8.8%** |
| `neutral` | 1.2% | 1.3% |
| `porn` | 10.3% | **7.7%** |
| `sexy` | 4.0% | 4.2% |

The gain is concentrated in the two explicit classes — misses, the error kind
[the operating point treats as the budget](../../decisions/classifier-operating-point.md).
False negatives fall from 262 to 185 while false positives rise from 177 to 201,
which is the trade the threshold is supposed to make and was previously
unavailable at any threshold.

`drawings` is the one source class that got worse at 0.5. This is a threshold
artifact rather than a ranking regression: drawn AUC improved on both
evaluation sets, and the whole score distribution shifted upward, so a fixed
0.5 cut now falls at a different place on it. At the 5% miss budget the model
over-blocks less overall, not more.

## Was the underfitting diagnosis right?

Yes, but it is not fully resolved.

| | unfreeze 3 | full unfreeze |
|---|---|---|
| train accuracy | 94.6% | **95.9%** |
| validation accuracy | 92.2% | **93.1%** |
| accuracy gap | 2.4pp | 2.8pp |
| train AUC | 0.9881 | 0.9927 |
| validation AUC | 0.9748 | 0.9796 |
| AUC gap | 0.0133 | 0.0131 |

**The diagnosis was right.** Capacity was the binding constraint: relieving it
raised training accuracy above the 95% line the diagnosis used as its criterion,
and — the part that matters — validation improved with it. Extra capacity spent
on memorisation would have moved training accuracy alone.

**The constraint is not gone.** At 95.9% the model still cannot fit its own
training data. Whatever remains is not a shortage of trainable parameters,
because every parameter is now trainable; the ceiling has moved somewhere else —
architecture, input resolution, optimisation schedule, or label noise.

**This is not the onset of overfitting.** The accuracy gap widened by 0.4pp
while the AUC gap was flat (0.0133 → 0.0131) and every validation metric
improved. A model beginning to overfit shows the opposite: a widening gap with
validation flat or falling. The accuracy movement is a threshold effect on a
shifted score distribution, which is also what the `drawings` row above reflects.

The practical reading: the cheap capacity gain has been taken, and a further
unfreeze is not available — there is nothing left to unfreeze.

## Consequences for the anime subsampling experiment

**Its prediction is now supported by direct evidence.** The pre-registration
predicted that "the binding constraint is capacity ... rather than data volume."
Removing the capacity limit improved drawn AUC by 0.0074 on the validation split
and 0.0133 on the common holdout — the latter exceeding the +0.010 threshold the
experiment set for accepting a *data* intervention, achieved with no new data at
all.

**The baselines must be re-fixed before that experiment runs.** They were taken
from the unfreeze-3 model, which is no longer the model to beat. Re-baselining
here is not post-hoc reinterpretation: the pre-registration named this run as its
prerequisite, so the new numbers are still fixed *before* the anime run starts.
The decision rule and its thresholds are unchanged — only the reference model is.

**Drawn content is still the weaker sub-problem**, 0.9604 against 0.9883 on the
validation split. The gap narrowed from 0.0314 to 0.0279 but did not close, so
the question the experiment asks remains open.
