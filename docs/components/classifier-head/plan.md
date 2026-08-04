# Classifier Head — Implementation Plan

The decision this crate exists to serve is
[learning-from-feedback.md § On-device runtime and where the head lives](../../decisions/learning-from-feedback.md#on-device-runtime-and-where-the-head-lives-decided-2026-07-18).
Read it first: this plan is the build order, not the argument.

## Current state

`packages/classifier-head/` **does not exist**. Nothing has been built.

The 2026-07-18 research called the eventual crate `feedback-head`, after the federated
learning that motivated it. It is named for the classification head here because that is
what it holds on day one — feedback and gradients are a later phase of the same crate, and
naming it after the phase it does not have yet would misdescribe every early commit.

## Why there is a crate here at all

A classifier splits in two:

```
image → [ backbone ] → embedding → [ head ] → logits → allow / block
        ~30M params    1024 floats  ~2K params
```

The backbone is frozen, adopted pretrained, and exported per platform. **The head is the only
part that encodes what this product considers blockable**, it is 0.007% of the model, and it
must be able to change on a device without shipping a new model file. That rules out baking it
into the exported graph — TFLite weights are immutable flatbuffer constants — which is why
`machine-learning`'s export contract terminates at the embedding.

Everything else follows from that:

- **It is arithmetic, so it belongs in Rust behind UniFFI**, exactly as `text-policy` does. One
  implementation serves Android and Windows, and the platforms cannot drift apart on the
  threshold, the softmax index, or the fail-open rules.
- **It is the only viable on-device training surface.** Every framework path was evaluated and
  rejected — see the decision doc. `dlogits = softmax(logits) − onehot(y)`, `dW = dlogits ⊗ e`
  is a hundred lines, and no framework offers it on Android at all.
- **Federated aggregation needs a flat parameter vector**, which is what this produces natively.

## The contract it must not break

`packages/image-sandbox/src/classifier.rs` already pins the score contract against
`machine-learning`'s `labels.py` / `eval.py`. This crate takes it over for the part after the
backbone, and must agree exactly:

| Rule | Value | Source |
|---|---|---|
| Score | `softmax(logits)[1]` | `labels.py` — index 1 is the explicit class |
| Block | `score >= threshold` | `eval.py`; `>=`, not `>` |
| Default threshold | `0.20`, **provisional** | [classifier-operating-point.md](../../decisions/classifier-operating-point.md) — derived from the *unfreeze-3* model, not the shipped one |
| On any error | Allow | `image-sandbox/src/sandbox.rs` — a fault here is not evidence about an image |

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

- `Head { weights: Vec<f32>, bias: Vec<f32>, embedding_dim: usize, classes: usize }`.
- `fn score(&self, embedding: &[f32]) -> Result<f32, HeadError>` — returns
  `softmax(logits)[EXPLICIT_CLASS]`.
- Dimension mismatch is a typed error, never a panic and never a silent truncation: this runs
  on every frame of a capture session and on every image in a page.
- **Subtract the row max before `exp`.** A 1024-dimensional dot product with unbounded weights
  overflows `f32::exp` readily, and an `inf/inf` NaN scored against a threshold compares false
  — i.e. it fails *open* by accident rather than by decision, which is the wrong way to be
  right.

Tests first: a hand-computed 2-class example, a symmetric-logits case pinned at 0.5, the
overflow case above, dimension mismatch, and agreement with `image-sandbox`'s
`explicit_score` on shared fixtures.

### 2. `weights` — provisioning and persistence

```
src/weights.rs
```

The head is data, and unlike the backbone it is small enough to ship, replace, or update.

- A versioned flat encoding: format version, backbone identity, `embedding_dim`, `classes`,
  then `W` and `b` as little-endian `f32`.
- `fn decode(bytes: &[u8]) -> Result<Head, WeightsError>` / `fn encode(&self) -> Vec<u8>`.
- **A refusal to load is not a failure to block.** Missing, truncated, or backbone-mismatched
  weights produce a head that allows everything, mirroring `ImageSandbox::disabled()`. Both
  platforms already have the pattern for where the bytes come from: `filesDir` on Android, as
  `BlocklistStore` does.

### 3. `verdict` — threshold to decision

```
src/verdict.rs
```

Trivial, and separate on purpose: the threshold is provisional and model-specific, so it is a
configured field rather than a constant read at the call site. Mirrors
`image_sandbox::ImageVerdict` so a caller can hold one type across both platforms.

### 4. `train` — the backward pass, behind a feature

```
src/train.rs
```

**Not day one.** Built when the feedback channel exists, gated behind a non-default feature so
the inference path never pays for it.

- `dlogits = softmax(logits) − onehot(y)`, `dW = dlogits ⊗ e`, `db = dlogits`, SGD.
- Verified against PyTorch `autograd` on committed fixtures to ~1e-5, **plus** a
  finite-difference gradient check in `cargo test`. Both, not either: fixtures catch a wrong
  formula, finite differences catch a fixture generated by the same wrong formula.

### 5. `classifier-head-ffi` — the UniFFI surface

```
packages/classifier-head-ffi/
```

Third of its kind, after `text-policy-ffi` and `net-shield-ffi`, and it follows them exactly —
including being built by `apps/mobile/scripts/build-ffi.sh`, which already loops over the FFI
crates. Exposes construction from weight bytes, `score`, and the verdict; keeps every decision
in the Rust core.

## Implementation order

1. `head` + `weights` + `verdict` with unit tests, no FFI. Pure Rust, no artifact needed —
   this is the whole reason to start here rather than at the runtime.
2. `classifier-head-ffi` and the Kotlin bindings.
3. Wire `apps/mobile`'s `FrameSink` to it, once a backbone exists to produce embeddings —
   see [mobile/plan.md](../mobile/plan.md) §9.
4. Wire the Windows path to the same crate, replacing the head half of `image-sandbox`'s ONNX
   graph if and when that export also terminates at the embedding.
5. `train`, when there is a feedback channel to train from.

## What this does not cover

- **The backbone.** LiteRT on Android, ONNX Runtime on Windows — a deliberate split, not an
  inconsistency. This crate never loads a model file.
- **Preprocessing.** `image-sandbox/src/preprocess.rs` owns the parity-tested transform and is
  guarded by a torchvision-generated fixture. Do not reimplement it here.
- **The federated protocol.** Clipping, DP noise, masking, and the server side are a separate
  concern and stay opt-in and off by default, per the local-first rule in AGENTS.md.

## Reference documents

- [learning-from-feedback.md](../../decisions/learning-from-feedback.md) — the runtime split and why the head is hand-written
- [classifier-operating-point.md](../../decisions/classifier-operating-point.md) — where the threshold comes from and why it is model-specific
- [machine-learning/plan.md](../machine-learning/plan.md) — the export contract that terminates at the embedding
- [image-sandbox/plan.md](../image-sandbox/plan.md) — the ONNX half, and the score contract this crate must match
- [Bonawitz et al., Practical Secure Aggregation](https://eprint.iacr.org/2017/281) — what the flat parameter vector is eventually for
