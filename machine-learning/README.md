# Machine Learning

Local pipeline for the baseline image classifier: fine-tune MobileNetV3-Small,
evaluate it for false positives and false negatives, and export runtime
artifacts for the platform daemons.

- Windows: ONNX (optionally int8-quantized)
- Android: TFLite

Everything runs on-device. No training data, evaluation imagery, or model
artifact is ever committed — see [Data layout](#data-layout).

## Setup

Python 3.11–3.13. **3.14 does not work**: `litert-torch` pulls in `torchao`,
which fails to import on 3.14.

```bash
cd machine-learning
python3.13 -m venv .venv
.venv/bin/pip install -e ".[test,tflite]"
```

## Data layout

All of these directories are gitignored.

```
data/train/<label>/<image>    # fine-tuning set
data/val/<label>/<image>      # validation, scored after each epoch
data/eval/<label>/<image>     # held-out set for the FP/FN harness
```

`<label>` must be `safe` or `explicit`. The class order is pinned in
`labels.py`, not inferred from directory sort order — exported artifacts bake
the output index order in, and alphabetical sorting would put `explicit` first
and silently invert every false-positive/false-negative reading.

## Commands

```bash
holy-blocker-train                        # fine-tune, write artifacts/baseline-v0.pt
holy-blocker-export                       # -> artifacts/baseline-v0.onnx
holy-blocker-eval --data-dir data/eval    # false positive / false negative report
.venv/bin/pytest                          # full suite
.venv/bin/pytest -m "not slow"            # skip full MobileNetV3 conversions
```

## Evaluating without keeping images

`holy-blocker-extract` converts an explicit corpus into a `.npz` of feature
vectors and deletes the source archive. After one pass the vectors are the
permanent evaluation asset — every later run reads numbers, never pixels.

```bash
cp .env.example .env      # then paste your token into it; .env is gitignored
pip install -e ".[data]"

holy-blocker-extract --inspect   # preflight: print the archive layout, extract nothing
holy-blocker-extract --out data/eval/nsfw_detect.npz
holy-blocker-eval --checkpoint artifacts/baseline-v0.pt --features data/eval/nsfw_detect.npz
```

`--archive path/to.zip` skips the download if the file was fetched by hand.

`HF_TOKEN` is read from `.env`, searched upward from the working directory so it
resolves from either the repo root or `machine-learning/`. A real shell export
always wins over the file. Copy `.env.example` to get the expected keys — it is
the only tracked env file, and nothing in it is used by the shipped product.

The archive's published layout is `<root>/<class>/<image>`, but that is
unverified against the real download. If it differs, extraction **fails with the
paths it actually found** rather than writing a confident report over zero
samples — `--inspect` shows the same diagnostic without spending a full pass.

The dataset's five classes collapse onto the binary decision via
`features.DEFAULT_LABEL_POLICY`. That mapping *is* the FP/FN definition:

| source class | maps to | why |
|---|---|---|
| `porn`, `hentai` | `explicit` | the content being filtered, drawn or not |
| `sexy` | `safe` | the hard negative — over-blocking it should count as a false positive, not a win |
| `drawing`, `neutral` | `safe` | ordinary content |

Use `--strict-sexy` to move `sexy` to the blocked side.

### What this does and does not guarantee

- Images are decoded from the zip **in memory**. The archive is never unpacked
  and is deleted afterwards, so no viewable image file is written.
- The downloaded archive is on disk until the pass completes. That window is the
  one moment the material exists in a decodable form.
- Feature vectors cannot be casually viewed, but they are not a redaction
  primitive and are not cryptographically one-way.
- `--features` never surfaces file paths, so there is nothing to open.
  `--examples` defaults to `0` for the image path too.

### Freeze the backbone

Cached features are produced by a specific backbone. If training fine-tunes the
whole network, the checkpoint's backbone drifts away from the one that made the
vectors and the head ends up scoring features it never saw. Train with
`frozen_backbone=True` to keep them aligned — `holy-blocker-eval --features`
warns loudly when a checkpoint was not. Changing backbone entirely means
re-running extraction against the source corpus.

## Evaluation

`holy-blocker-eval` is the FP/FN harness. It prints per-class precision and
recall, a confusion matrix, a threshold sweep, and the individual files the
model got most wrong so they can be inspected by hand:

```
false positives (safe blocked):    3   rate 0.0188
false negatives (explicit missed): 7   rate 0.0438

 thresh     FP     FN      FPR      FNR      acc
   0.50      3      7   0.0188   0.0438   0.9688
   0.80      0     14   0.0000   0.0875   0.9563
```

The two error kinds are reported separately on purpose. A false positive blocks
something harmless and erodes trust; a false negative lets through exactly what
the user asked to be shielded from. Accuracy alone hides that asymmetry,
especially on a class-imbalanced evaluation set. The sweep exists so the
deployed threshold is a deliberate choice rather than an implicit 0.5.

## Notes

- Train from pretrained weights (the default). With `pretrained=False` on a
  small dataset the BatchNorm running statistics never stabilize, and the model
  scores near-perfectly in train mode while collapsing to a single constant
  output in eval mode. That is a property of the regime, not a bug — the tests
  use `pretrained=False` only to stay offline, never to assert accuracy.
- TFLite export is float32. `litert-torch` offers PT2E quantization, but that is
  static int8 and needs a calibration set; the float32 artifact is ~6 MB against
  a 15 MB budget, so it can wait.
- ONNX export pins the legacy TorchScript exporter (`dynamo=False`). The dynamo
  exporter's MobileNetV3 graph carries inconsistent shape metadata that
  `onnxruntime`'s quantizer rejects; see the comment in `export.py`.
