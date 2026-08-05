# Classifier Head — Implementation Plan

The decision this crate exists to serve is
[learning-from-feedback.md § On-device runtime and where the head lives](../../decisions/learning-from-feedback.md#on-device-runtime-and-where-the-head-lives-decided-2026-07-18).
Read it first: this plan is the build order, not the argument.

## Current state

`packages/classifier-head/` **does not exist**. Nothing has been built, and neither has
`packages/classifier-head-ffi/`.

The 2026-07-18 research called the eventual crate `feedback-head`, after the federated
learning that motivated it. It is named for the classification head here because that is
what it holds on day one — feedback and gradients are a later phase of the same crate, and
naming it after the phase it does not have yet would misdescribe every early commit.

**The export side does not exist either, and producing it is a deliverable of this plan.**
`machine-learning`'s export contract is *decided* — see
[machine-learning/plan.md § Export contract (decided)](../machine-learning/plan.md#export-contract-decided)
— but it is **not implemented**. `export_tflite.py` today calls `create_classifier(...)`,
loads the full checkpoint, and converts the entire model *including its head*, so the
flatbuffer emits logits rather than an embedding. Nothing anywhere writes a head-weights
file. Read "terminates at the embedding" as an intention with no code behind it yet.

## Why there is a crate here at all

A classifier splits in two:

```
image → [ backbone ] → embedding → [ head ] → logits → allow / block
         frozen,        1024 f32     Linear(1024→2)
         exported       (Small)      2,050 params, 8.2 KB
         per platform
```

The backbone is adopted pretrained, fine-tuned once offline, and exported per platform.
**The head is the only part that encodes what this product considers blockable**, it is
under 0.1% of the parameters, and it must be able to change on a device without shipping a
new model file. That rules out baking it into the exported graph — TFLite weights are
immutable flatbuffer constants — which is what the export contract exists for.

Everything else follows from that:

- **It is arithmetic, so it belongs in Rust behind UniFFI**, exactly as `text-policy` does.
  One implementation serves Android and Windows, and the platforms cannot drift apart on the
  threshold, the softmax index, or the fail-open rules.
- **It is the only viable on-device training surface.** Every framework path was evaluated
  and rejected — see the decision doc. `dlogits = softmax(logits) − onehot(y)`,
  `dW = dlogits ⊗ e` is a hundred lines, and no framework offers it on Android at all.
- **Federated aggregation needs a flat parameter vector**, which is what this produces
  natively.

## Where the backbone ends and the head begins (decided)

This is the load-bearing decision of the plan. It fixes the width of the embedding, the size
of the head, the shape of the backward pass, and what `machine-learning` has to export.

`torchvision`'s `mobilenet_v3_small` — the architecture `model.py` builds — has this
classifier, read off a constructed model rather than from documentation:

```
model.avgpool          → 576-d                     (BACKBONE_FEATURE_DIM in model.py)
model.classifier[0]    Linear(576 → 1024)
model.classifier[1]    Hardswish()
model.classifier[2]    Dropout(p=0.2, inplace=True)
model.classifier[3]    Linear(1024 → 1000)         ← create_classifier replaces this
                                                     with Linear(1024 → 2)
```

`create_classifier` replaces **only `classifier[-1]`**. Everything before it is stock.

**Decision: the cut lands after the Hardswish.** The embedding is the penultimate
activation, and the head is the single final affine layer.

| | embedding | head | params | f32 bytes |
|---|---|---|---|---|
| MobileNetV3-**Small** (deployed) | 1024-d | `Linear(1024 → 2)` | 2,050 | 8,200 |
| MobileNetV3-**Large** (not adopted) | 1280-d | `Linear(1280 → 2)` | 2,562 | 10,248 |

Dropout is inference-inactive and disappears from the exported graph, so in practice it sits
on neither side of the cut.

### The rejected alternative: cutting after `avgpool`

The other natural cut is at `avgpool` — 576-d on Small, 960-d on Large — which makes the
head the **whole** `classifier` block: `Linear(576→1024)` + `Hardswish` + `Linear(1024→2)`,
**592,898 parameters, 2.37 MB in f32**, a two-layer MLP.

It has real advantages, and they are written down rather than dismissed:

- **It is the interface this repo already speaks.** `model.py` names the constant
  `BACKBONE_FEATURE_DIM = 576`; `BackboneFeatures` already exists as an `nn.Module` emitting
  exactly that vector; and `features.py` / `extract.py` cache 576-d vectors as the
  *permanent evaluation asset* — the artifact that lets the source corpus be deleted after a
  single pass. Choosing the Hardswish cut means training the head offline requires pushing
  those cached 576-d features through the frozen `classifier[0:2]` first. That is a few
  lines, but it is a real cost paid by every offline head fit, forever.
- **Post-`avgpool` activations quantize better.** They come straight out of a global pooling
  layer and are comparatively well behaved. Post-Hardswish activations are unbounded above
  and long tailed, which is the harder tensor to pick an int8 scale for — and the embedding
  dtype and scale are part of the head's input contract.

It is rejected anyway, for three reasons:

1. **The training math holds exactly for one affine layer.** `dlogits = softmax(logits) −
   onehot(y)`, `dW = dlogits ⊗ e`, `db = dlogits` is the entire backward pass, and it is
   finite-difference-checkable in a single test. The avgpool cut needs the Hardswish
   derivative in the backward pass — a piecewise function with two kink points — plus a
   cached hidden activation per sample. That is not hard; it is several times more surface
   to get wrong in the one component that has no framework checking it.
2. **2,050 parameters is the regime personalization works in.** Fitting a couple of thousand
   parameters to a few dozen user corrections behaves like logistic regression. Fitting 593k
   to the same data overfits instantly, and the mitigation (regularization schedules,
   per-user held-out splits) is a research problem rather than a feature.
3. **8 KB ships; 2.37 MB is an engineering problem.** 8 KB fits inside the app, updates over
   any channel, and is a sane federated-aggregation payload. 593k floats per device per
   round is a bandwidth-and-masking problem before it is anything else.

### The objection that settles it

The obvious objection to the Hardswish cut is that it ties every field head to one specific
checkpoint's `classifier[0]`, so re-exporting the backbone invalidates them all.

That is true, and it is **not a new failure class**, because the avgpool cut is invalidated
just as thoroughly. The deployed recipe is **full unfreeze** — `features` itself trains — so
a new checkpoint moves the 576-d vector too. That is exactly why `features.py`'s docstring
warns that cached artifacts silently stop matching, and why `extract.py` carries a
`--from-checkpoint` flag.

The repo has already measured how badly this class of breakage bites. The full-unfreeze run
was [replicated](../machine-learning/experiments/full-unfreeze.md) from scratch under an
identical recipe and identical seed: the two checkpoints agree on **ranking to within 0.0005
AUC**, while their 5%-miss thresholds differ by **27%** (0.2717 against 0.1980). Two runs of
the same recipe are already not interchangeable at the operating point.

So both cuts need the same answer, and the repo already has it: **refuse to score on a
backbone-identity mismatch rather than scoring anyway.** That is `weights.rs`'s job below.
Since the answer is shared, the cut is chosen on the merits above.

### The hedge

**The weights format carries a format version, and v1 describes exactly one affine layer.**
A future v2 may describe a hidden layer — which is the avgpool cut, and any other
"head with capacity" variant. A v1 reader **refuses** an unknown version rather than
guessing at the layout. Choosing the simple cut now therefore does not foreclose the other
later; it costs one format bump and a re-provision.

### Why `embedding_dim` is read from the file, never compiled in

Both MobileNetV3-**Small** and MobileNetV3-**Large** checkpoints exist under
`machine-learning/artifacts/` today (`finetuned-v0.pt` and `unfreeze-full/` for Small, the
`large-s0/ s1/ s2/` seed runs for Large), with embeddings of **1024** and **1280**
respectively. A constant compiled into the Rust crate would already be wrong for one of two
artifact families sitting on the same disk, and the failure mode is a dimension-mismatch
error at best and a silently mis-strided dot product at worst. This is a live requirement,
not a precaution.

**MobileNetV3-Large is not adopted.** It scores better, but its pre-registered decision rule
returned inconclusive and adoption is gated on an int8 export that has never been run for it
(fp32 is 16.8 MB against a 15 MB budget). It appears in this plan **only** as the reason
`embedding_dim` must be data. Nothing here proposes adopting it.

## The contract it must not break

`packages/image-sandbox/src/classifier.rs` already pins the score contract against
`machine-learning`'s `labels.py` / `eval.py`. This crate takes it over for the part after
the backbone, and must agree exactly:

| Rule | Value | Source |
|---|---|---|
| Class order | index 0 `safe`, index 1 `explicit` | `labels.py` — `BINARY_LABELS`, pinned there rather than derived from sorted directory names precisely so it cannot silently invert |
| Score | `softmax(logits)[1]` | `labels.py` / `eval.py:collect_predictions` |
| Block | `score >= threshold` | `eval.py`; `>=`, not `>` |
| On any error | Allow | `image-sandbox/src/sandbox.rs` — a fault here is not evidence about an image |

**The default threshold is not a constant, and this crate ships no number.** See
[classifier-operating-point.md](../../decisions/classifier-operating-point.md): a threshold
belongs to a **(checkpoint, geometry) pair**, and this repo has already been burned by
treating one as universal. `image-sandbox` shipped a provisional **0.20** — the 5%-miss
threshold of the *superseded unfreeze-3 model* — while the deployed full-unfreeze checkpoint
operates at **0.2717** under the evaluation centre crop and **0.4650** under the deployed
tile-max geometry. Reusing the centre-crop number under tile-max over-blocks 14.73% against
10.09%, with nothing failing to indicate it.

So: the threshold is a **configured field**, supplied by whoever knows which checkpoint and
which geometry are in play. Any number written into this crate would be a guess about the
caller's geometry. When the mobile image path picks its geometry, its threshold is
re-derived from the miss-budget table for *that* pairing and passed in.

**The embedding is a versioned interface, not a float array.** An int8 backbone's scale and
zero-point are part of the head's input contract, so a silently re-exported backbone
invalidates every head in the field. Weights therefore carry the backbone identity they were
trained against, and a mismatch is refused rather than scored.

## Modules to add

### 1. `head` — the forward pass

```
src/head.rs
```

Pure. `logits = W·e + b`, then softmax, then the explicit score.

```rust
pub struct Head {
    weights: Vec<f32>,      // row-major [classes, embedding_dim]
    bias: Vec<f32>,         // [classes]
    embedding_dim: usize,
    classes: usize,
    backbone_id: String,
}

impl Head {
    pub fn logits(&self, embedding: &[f32]) -> Result<Vec<f32>, HeadError>;
    pub fn score(&self, embedding: &[f32]) -> Result<f32, HeadError>;
    pub fn embedding_dim(&self) -> usize;
    pub fn backbone_id(&self) -> &str;
}
```

- `score` returns `softmax(logits)[EXPLICIT_INDEX]`, with `EXPLICIT_INDEX = 1` and
  `CLASS_COUNT = 2` mirroring `image-sandbox`'s constants of the same names.
- **A dimension mismatch is a typed error — never a panic, never a silent truncation.** This
  runs on every frame of a capture session and on every image in a page: a panic takes the
  guard process down, and truncating to the shorter of the two lengths produces a plausible
  number computed from the wrong vector. `HeadError::EmbeddingDim { expected, actual }`.
- **A non-finite embedding is a typed error too.** A NaN arriving from a broken backbone
  propagates through the dot product, and `NaN >= threshold` is `false` — i.e. it fails
  *open* by accident rather than by decision.
- **Subtract the row max before `exp`.** A 1024-dimensional dot product with unbounded
  weights overflows `f32::exp` readily, and `inf/inf` is NaN, which lands in the same
  accidental fail-open. `image-sandbox`'s `explicit_score` already does this; the two must
  not diverge.

Tests first: a hand-computed 2-class example; symmetric logits pinned at 0.5; the direction
test (`[0, 1]` scores above `[1, 0]`, so an inverted label order is caught rather than
merely assumed); the overflow case; a NaN embedding; an empty embedding; both directions of
dimension mismatch; and agreement with `image_sandbox::classifier::explicit_score` on a
shared logit fixture.

### 2. `weights` — provisioning and persistence

```
src/weights.rs
```

The head is data, and unlike the backbone it is small enough to ship, replace, or update.
The encoding is specified here to the byte, because two implementations write and read it
(Python exports, Rust consumes) and "whatever `struct.pack` did" is not a contract.

**All multi-byte fields are little-endian.** `f32` values are IEEE 754 binary32 in the byte
order of `f32::to_le_bytes` / `f32::from_le_bytes`, which is exactly NumPy's `'<f4'` — see
[Reference documents](#reference-documents).

```
offset  size             field             notes
------  ---------------  ----------------  --------------------------------------------
0       4                magic             ASCII "HBHD"  (Holy Blocker HeaD)
4       2                format_version    u16 = 1
6       2                classes           u16 = 2
8       4                embedding_dim     u32 = 1024 (Small) / 1280 (Large)
12      2                backbone_id_len   u16, byte length of the UTF-8 string
14      2                reserved          u16 = 0, MUST be zero in v1
16      backbone_id_len  backbone_id       UTF-8, no NUL terminator
        pad              padding           0x00 up to the next 4-byte boundary
        4*C*D            weights           f32, row-major [classes, embedding_dim]
        4*C              bias              f32, [classes]
```

Total length is `16 + backbone_id_len + pad + 4 * classes * (embedding_dim + 1)`. A decoder
that computes that and compares it against the buffer catches every truncation in one check.

The parts that are decisions rather than incidentals:

- **Row-major `[classes, embedding_dim]` is PyTorch's own layout.** `nn.Linear.weight` has
  shape `[out_features, in_features]` and is contiguous, so the export side is a straight
  `layer.weight.detach().cpu().numpy().astype("<f4").tobytes()` with **no transpose** — and
  the one bug this format could plausibly carry (a transposed matrix, which for a 2×1024
  matrix is not even detectable by element count) never gets a chance to happen.
- **The padding after `backbone_id` exists so the float arrays start 4-byte aligned.** Rust
  reads them via `from_le_bytes` over chunks rather than a transmute, so this is not a
  soundness requirement — it is there so a zero-copy reader stays possible without a format
  bump.
- **`reserved` must be zero.** It is where a v1.x additive change lands.
- **An unknown `format_version` is refused, not guessed at.** This is the hedge from the cut
  decision made concrete: a v2 file describing a hidden layer must never be read as v1 and
  reinterpreted as one very wide affine layer, which is precisely what a lenient reader
  would do.
- **Non-finite weights are refused at decode.** A single NaN in `W` makes every score NaN
  and every verdict a silent allow. Checking 2,050 floats once at load is free; discovering
  it per frame is not.
- **The threshold is deliberately not in the file.** The file names one half of the pair
  (the checkpoint) but not the other (the geometry), which the caller chooses at runtime. A
  threshold baked in here would be correct only for whichever geometry the exporter guessed
  — the failure [classifier-operating-point.md](../../decisions/classifier-operating-point.md)
  records.

```rust
pub fn decode(bytes: &[u8]) -> Result<Head, WeightsError>;
pub fn encode(head: &Head) -> Vec<u8>;
```

`WeightsError` distinguishes `BadMagic`, `UnsupportedVersion(u16)`,
`Truncated { expected, actual }`, `TrailingBytes`, `NonFinite`, `BadUtf8`, `ReservedNonZero`
and `ZeroDimension` — separate variants because these are exactly what a support log has to
tell apart.

**Backbone identity, and what a mismatch does.** Construction takes the identity the loaded
backbone reports and compares it against the file's `backbone_id`. On a mismatch the head
**refuses**: it is constructed in a disabled state that allows everything, and says so. The
identity string is produced by the exporter (module 6) and derived from the exported
artifact's own bytes, so a silent re-export cannot keep the old identity.

**A refusal to load is not a failure to block.** Missing, truncated, version-mismatched or
backbone-mismatched weights produce a head that allows everything, mirroring
`ImageSandbox::disabled()`. Both platforms already have the pattern for where the bytes come
from: `filesDir` on Android, exactly as `BlocklistStore` reads `filesDir/blocklist.txt`.

Tests first — this module is where the test-first rule earns its keep, and they are written
against the table above before any code exists: a round trip through `encode`/`decode`; a
**byte-literal fixture** asserting each header field lands at its documented offset (a field
reorder is invisible to a round trip); wrong magic; version 2; truncation at every boundary
(mid-header, mid-id, mid-`W`, mid-`b`, one byte short); trailing bytes; `embedding_dim = 0`;
`classes = 0`; non-zero `reserved`; invalid UTF-8 in the id; a NaN weight and an inf weight;
an id length that is not a multiple of 4 (pinning the padding); a backbone-id mismatch
producing a disabled head that allows; and a 1280-d file decoding correctly in the same
build as a 1024-d one.

#### Reference documents

- [IEEE 754-2019](https://standards.ieee.org/ieee/754/6210/) — binary32, the encoding every `f32` in this format uses
- [`f32::from_le_bytes`](https://doc.rust-lang.org/std/primitive.f32.html#method.from_le_bytes) / [`f32::to_le_bytes`](https://doc.rust-lang.org/std/primitive.f32.html#method.to_le_bytes) — the Rust side, and why no transmute is needed
- [NumPy byte-order and dtype strings](https://numpy.org/doc/stable/reference/arrays.dtypes.html#specifying-and-constructing-data-types) — `'<f4'` is little-endian binary32, which is what makes the Python side a `.tobytes()` call
- [`torch.nn.Linear`](https://pytorch.org/docs/stable/generated/torch.nn.Linear.html) — `weight` has shape `[out_features, in_features]`; that is the row-major layout this format adopts so no transpose is ever needed

### 3. `verdict` — threshold to decision

```
src/verdict.rs
```

Trivial, and separate on purpose: the threshold is a **(checkpoint, geometry)** property, so
it is a configured field rather than a constant read at the call site.

```rust
pub struct HeadConfig { pub threshold: f32 }        // no Default carrying a number
pub enum HeadVerdict { Allow, Block }
pub fn decide(score: f32, config: &HeadConfig) -> HeadVerdict;   // Block iff score >= threshold
```

Mirrors `image_sandbox::ImageVerdict` closely enough that a caller can hold one mental model
across both platforms. There is deliberately **no** `Default for HeadConfig` supplying a
number: the caller must state which operating point it is running at, and a default is how a
wrong one travels silently.

Tests first: `score == threshold` blocks (the `>=` boundary, probed from both sides by an
epsilon); a `0.0` threshold blocks everything and a `1.0` threshold blocks only a score of
exactly 1.0; a NaN score allows, and is documented as a path that `head` should have
rejected before `decide` ever sees it.

### 4. `train` — the backward pass, behind a feature

```
src/train.rs
```

**Not day one.** Built when the feedback channel exists, gated behind a non-default feature
(`train`) so the inference path never pays for it — the same pattern as `image-sandbox`'s
`onnx` feature and the FFI crates' `bindgen` feature.

- `dlogits = softmax(logits) − onehot(y)`, `dW = dlogits ⊗ e`, `db = dlogits`, then SGD.
- Verified **both** ways, not either:
  - against PyTorch `autograd` on committed fixtures, to ~1e-5;
  - **and** by a finite-difference gradient check in `cargo test` — central difference,
    `(L(w + h) − L(w − h)) / 2h`, over a sampled subset of coordinates.

  Fixtures catch a wrong formula. Finite differences catch a fixture generated by the same
  wrong formula — which is the realistic failure here, because the same person writes both.

The fixtures obey the same rule as the forward-pass fixtures below: seeded-random weights
and seeded-random embeddings, never anything trained or corpus-derived.

### 5. `packages/classifier-head-ffi` — the UniFFI surface

```
packages/classifier-head-ffi/
```

Third of its kind, after `packages/text-policy-ffi` and `packages/net-shield-ffi`, and it
follows them exactly. Concretely, from those two crates:

- `Cargo.toml` with `crate-type = ["cdylib", "lib"]`,
  `uniffi = { version = "0.32.0", default-features = false }`, and a path dependency on
  `classifier-head`.
- A **non-default `bindgen` feature** (`bindgen = ["uniffi/cli"]`) plus a
  `src/bin/uniffi-bindgen.rs` marked `required-features = ["bindgen"]`. Both existing crates
  carry the same comment explaining why: the bindgen CLI drags in `cargo_metadata`, whose
  transitive `cargo-platform` needs a newer rustc than the toolchain CI pins, so `cargo test`
  must not build it.
- `apps/mobile/scripts/build-ffi.sh` **already loops** over a `crates=(...)` array of
  `"<crate-dir>:<lib_name>"` entries and does both jobs per crate — host cdylib → Kotlin
  bindings, then `cargo ndk` for `arm64-v8a`, `armeabi-v7a` and `x86_64`. Adding this crate
  is one array entry: `"classifier-head-ffi:classifier_head_ffi"`. Note the bindings output
  directory is cleared **once, before** the loop, because each crate generates into its own
  `uniffi/<namespace>` subtree.
- Follow `net-shield-ffi`'s use of an **enum-with-fields** where a sum type is wanted, so the
  Kotlin binding comes out as a sealed class — that is the shape to use if `HeadVerdict` ever
  carries the score alongside the decision.
- `text-policy-ffi` ships a placeholder dictionary and `net-shield-ffi` ships one RFC 2606
  reserved name. This crate ships **no weights at all**: there is no sensible placeholder for
  a trained head, because a random one would score, and scoring nonsense is worse than
  refusing. Absent weights means the disabled head, which allows.
- Note `text-policy-ffi` also has a **Swift** consumer now (`native-modules/mac-daemon`),
  which makes an FFI crate's API a cross-platform contract. Keep this surface small for the
  same reason.

Surface, keeping every decision in the Rust core:

```
constructor Head::from_bytes(weights: bytes, expected_backbone_id: String) -> Head
method      score(embedding: sequence<f32>) -> f32          (throws)
method      classify(embedding: sequence<f32>, threshold: f32) -> HeadVerdict
method      is_enabled() -> bool
method      backbone_id() -> String
method      embedding_dim() -> u32
```

`embedding_dim()` is on the surface because the Kotlin side must size the LiteRT output
buffer from the *file*, not from a constant — see the Small/Large note above.

#### Reference documents

- [UniFFI manual](https://mozilla.github.io/uniffi-rs/) — the procedural-macro surface, read alongside `packages/text-policy-ffi` and `packages/net-shield-ffi`, which are the working examples in this repo
- [LiteRT for Android](https://developers.google.com/edge/litert/android) — the interpreter that produces the embedding this surface consumes

### 6. The export side — a `machine-learning` deliverable

New work in Python, and a prerequisite for anything in this crate ever running against a
real model. Cross-referenced from
[machine-learning/plan.md § Export contract (decided)](../machine-learning/plan.md#export-contract-decided).

Two artifacts come out of one checkpoint, and they must be produced **together** or they
cannot be trusted to match.

**(a) The backbone, terminating at the penultimate activation.**

A new module `holy_blocker_ml/embedding.py` supplying an `nn.Module` in the shape of the
existing `BackboneFeatures`:

```python
class PenultimateEmbedding(nn.Module):
    """MobileNetV3 up to and including classifier[1] (the Hardswish).

    Emits the 1024-d (Small) / 1280-d (Large) vector that packages/classifier-head
    consumes. Deliberately *not* BackboneFeatures, which stops at avgpool and emits
    576-d — see docs/components/classifier-head/plan.md for why the cut moved.
    """
```

`export_tflite.py` gains an embedding-only path (`export_backbone_tflite`, reached as
`--embedding-only`, target artifact `data/models/baseline-v0-embedding.tflite`) converting
this module rather than `create_classifier(...)`. **The existing whole-model export stays** —
`packages/image-sandbox` is a shipped consumer of a logits-emitting ONNX graph, and nothing
in this plan changes it.

Alongside the flatbuffer, a sidecar `baseline-v0-embedding.json`:

```json
{ "backbone_id": "mobilenet_v3_small-classifier.1-<sha256[:16]>",
  "embedding_dim": 1024, "arch": "mobilenet_v3_small",
  "cut": "classifier.1", "sha256": "..." }
```

**The `backbone_id` is derived from the exported artifact's own bytes.** That is the whole
mechanism: a silent re-export produces different bytes, therefore a different id, therefore
a head that refuses rather than one that scores a stale-shaped embedding. An id typed in by
hand would defeat it.

**(b) The head weights file.**

A new module `holy_blocker_ml/export_head.py` that reads the same checkpoint, takes
`model.classifier[-1]` — the only layer `create_classifier` replaced — and writes the format
specified in module 2:

```python
def encode_head(layer: nn.Linear, backbone_id: str) -> bytes: ...
def export_head(checkpoint_path: Path, backbone_id: str, output_path: Path) -> Path: ...
```

`W` is `layer.weight.detach().cpu().numpy().astype("<f4").tobytes()` — no transpose, per the
layout note — and `b` is the same for `layer.bias`. Target artifact
`data/models/baseline-v0.head`.

**CLI entry points**, in the style of the existing `holy-blocker-*` console scripts in
`machine-learning/pyproject.toml` (`holy-blocker-train`, `-export`, `-eval`, `-extract`,
`-finetune`, `-score`, `-anime`, `-inputs`) — each a thin `main()` using `argparse` with
`Path`-typed options defaulted from `TrainingConfig`, per the "keep orchestration thin" rule
in AGENTS.md:

```toml
holy-blocker-export-head = "holy_blocker_ml.export_head:main"
```

with the backbone half reached through the existing `holy-blocker-export`
(`--embedding-only`) rather than a fourth export script. A single `--out-dir` run should
write the `.tflite`, the `.json` and the `.head` in one pass, computing the id once —
producing them separately is how they get out of sync.

**Python tests first**, under `machine-learning/tests`: that `PenultimateEmbedding` emits
1024 columns for Small and 1280 for Large; that its output equals the reference model's
`classifier[1]` activation to floating-point tolerance; that `encode_head` produces exactly
`16 + len(id) + pad + 4*2*(D+1)` bytes with the documented header values at the documented
offsets; and that decoding the file back with `numpy` reconstructs `layer.weight` exactly.

#### Reference documents

- [Convert PyTorch to LiteRT](https://developers.google.com/edge/litert/models/convert_pytorch) — the conversion path the embedding-only backbone goes through
- [litert-torch](https://github.com/google-ai-edge/litert-torch) — the converter `export_tflite.py` already uses
- [torchvision `mobilenetv3.py`](https://github.com/pytorch/vision/blob/main/torchvision/models/mobilenetv3.py) — the classifier block the cut point is defined against
- [`torch.nn.Linear`](https://pytorch.org/docs/stable/generated/torch.nn.Linear.html) — the `[out, in]` weight layout the file format matches

## Parity fixtures — what they may and may not contain

The Rust forward pass is pinned against PyTorch by a committed fixture, the same way
`image-sandbox`'s `tests/parity.rs` and `tests/tile_parity.rs` are pinned against
torchvision.

**The fixture must not embed trained weights or anything derived from a corpus.** Use a
seeded-random head and seeded-random embeddings:

- a generator script committed under `machine-learning/tests/` that sets an explicit seed,
  builds a `Linear(1024, 2)` and a batch of random embeddings, and writes the inputs
  alongside PyTorch's `logits` and `softmax(logits)[1]` into a JSON fixture;
- the fixture committed under `packages/classifier-head/tests/fixtures/`;
- a `tests/parity.rs` that decodes it and asserts agreement to ~1e-6.

That pins arithmetic, memory layout, and the softmax index — all the fixture is for. It pins
nothing about what the product blocks. Real weights ship at runtime from `filesDir`, as
`BlocklistStore` does, and `data/models/` is gitignored precisely so trained artifacts never
enter the repo. A fixture built from a trained head would be a small checked-in piece of the
component that decides what is blockable, which is the one thing this repo keeps out of
version control.

Note the trap the tile-parity fixture already taught: build the fixture so a *positional* or
*layout* error shows up. Random embeddings do this for free — a transposed or mis-strided
`W` gives a visibly different logit — whereas a constant embedding would hide it.

## Implementation order

1. `head` + `weights` + `verdict` with unit tests, no FFI. Pure Rust, no artifact needed —
   this is the whole reason to start here rather than at the runtime.
2. The parity fixture and its generator. Needs Python, but no corpus and no checkpoint.
3. **The export side (module 6)** — the embedding-only backbone export, the sidecar
   identity, and `holy-blocker-export-head`. This is what turns "fails open correctly" into
   "scores".
4. `classifier-head-ffi` and the Kotlin bindings; add the crate to `build-ffi.sh`'s array.
5. Wire `apps/mobile`'s `FrameSink` to it behind the LiteRT interpreter — see
   [mobile/plan.md](../mobile/plan.md) §9 and step 13. Model provisioning from `filesDir`
   comes with it.
6. Wire the Windows path to the same crate, replacing the head half of `image-sandbox`'s
   ONNX graph **if and when** that export also terminates at the embedding. It does not
   today, and there is no reason to force it before the mobile path proves the seam.
7. `train`, when there is a feedback channel to train from.

Steps 1 and 2 are unblocked right now. Step 3 needs a checkpoint (several exist under
`machine-learning/artifacts/`, gitignored). Step 5 is blocked on an exported artifact for
*verification* only — it can be built and shown to fail open before one exists.

## What this does not cover

- **The backbone.** LiteRT on Android, ONNX Runtime on Windows — a deliberate split, not an
  inconsistency. This crate never loads a model file and never links a runtime.
- **Preprocessing and geometry.** `image-sandbox` owns the parity-tested transforms
  (`preprocess` for the evaluation centre crop, `preprocess_tiles` for the deployed tile-max
  geometry), each guarded by a torchvision-generated fixture. Do not reimplement either
  here. Note tile-max means the *caller* runs the head once per tile and takes the max; this
  crate scores one embedding at a time and has no opinion about how many there were.
- **Choosing the threshold.** That is a `machine-learning` output per (checkpoint, geometry)
  pair — see [classifier-operating-point.md](../../decisions/classifier-operating-point.md).
- **The federated protocol.** Clipping, DP noise, masking, and the server side are a
  separate concern and stay opt-in and off by default, per the local-first rule in AGENTS.md.

## Reference documents

Per-module reference lists sit with their modules above. The documents below are the ones
that apply to the crate as a whole.

- [learning-from-feedback.md](../../decisions/learning-from-feedback.md) — the runtime split and why the head is hand-written
- [classifier-operating-point.md](../../decisions/classifier-operating-point.md) — where the threshold comes from and why it belongs to a (checkpoint, geometry) pair
- [machine-learning/plan.md](../machine-learning/plan.md) — the export contract, and the deliverable module 6 adds to it
- [image-sandbox/plan.md](../image-sandbox/plan.md) — the ONNX half, the deployed geometry, and the score contract this crate must match
- [mobile/plan.md](../mobile/plan.md) §9 — the consumer on Android
- [Bonawitz et al., Practical Secure Aggregation](https://eprint.iacr.org/2017/281) — what the flat parameter vector is eventually for
