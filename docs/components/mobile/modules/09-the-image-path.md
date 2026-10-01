# 9. The image path — backbone on LiteRT + the head crate — not yet created

Part of the [Mobile plan](../plan.md).

§6 delivers frames. This is what looks at them, and it is the half of step 12 that is still
open. **It does not go through `packages/image-sandbox`**, and the reason is a decision, not an
oversight:
[learning-from-feedback.md § On-device runtime and where the head lives](../../../decisions/learning-from-feedback.md#on-device-runtime-and-where-the-head-lives-decided-2026-07-18)
settled on 2026-07-18 that the frozen backbone runs on **LiteRT on Android and ONNX Runtime on
Windows**, with the head — embedding → score → verdict — hand-written in Rust behind UniFFI and
shared by both. `image-sandbox` is the ONNX half of that split, built for `mitm-proxy`.

The short version of why: nothing can train on Android. LiteRT's on-device training needs
TensorFlow-authored `train`/`infer` signatures that a `litert-torch` model cannot emit,
ExecuTorch's Android AAR has no training binding at all, and `onnxruntime-training-android` has
not published since 2024 while its inference sibling ships monthly. A head that is 2,050 numbers
is a hundred lines of arithmetic instead, and it is the only shape that leaves room for the
per-device updates the product design calls for. The decision doc carries the evidence.

**Measured here, so nobody re-derives it:** `image-sandbox` *does* build for
`aarch64-linux-android` with `--features onnx` (ONNX Runtime 1.24.2, statically linked, plus a
`libc++_shared.so` to package), but `ort` ships **no prebuilt runtime for
`x86_64-linux-android` or `armv7-linux-androideabi`** — the build fails outright. Taking that
route would cost two of the three ABIs `build-ffi.sh` targets, which is a reason to prefer the
decided path rather than merely an inconvenience.

Three pieces, in dependency order:

1. **`packages/classifier-head`** — the head and its FFI wrapper. Pure arithmetic, needs no
   model artifact, and is therefore the piece that can be built and tested first. See
   [classifier-head/plan.md](../../classifier-head/plan.md).
2. **The backbone on LiteRT.** `machine-learning`'s `export_tflite.py` already produces a
   verified 5.90 MB flatbuffer, and its export contract terminates at the embedding on purpose
   — **after the Hardswish**, i.e. `classifier[1]`, so the graph emits the **1024-d**
   penultimate activation and the head is the single remaining `Linear(1024 → 2)`. But that
   contract is *decided and not yet implemented*: the export today converts the whole model
   including its head and emits logits, so an embedding-only export is outstanding work on the
   `machine-learning` side — see
   [classifier-head/plan.md](../../classifier-head/plan.md) module 6. The Android side is then the
   LiteRT interpreter over that file, emitting the embedding the head consumes, sized from the
   artifact's sidecar rather than a constant. Note the tensor layout question is open:
   `preprocess.rs` produces NCHW for ONNX, and what `litert-torch` emits has to be read off the
   actual artifact rather than assumed.
3. **Model provisioning.** `data/models/` is gitignored and **no artifact exists on disk today**,
   so this needs the `BlocklistStore` treatment — read from `filesDir`, and with nothing there
   behave exactly as `ImageSandbox::disabled()` does: allow, and say so.

`FrameSink` is the seam all three land behind; §6's `CapturedFrame` already carries tightly
packed RGBA at capture size, which is the input the preprocessing step wants.

**What this cannot claim until an artifact exists.** With no model on disk, the whole path can
be unit tested and shown to fail open, and nothing more. A smoke test asserting "capture →
embedding → verdict" needs a `.tflite` exported from a trained checkpoint; until then the
emulator can only confirm that a missing model is handled the way it says it is.

## Reference documents

- [learning-from-feedback.md](../../../decisions/learning-from-feedback.md) — the runtime split, with the evidence for each rejected framework
- [classifier-head/plan.md](../../classifier-head/plan.md) — the head crate
- [LiteRT for Android](https://developers.google.com/edge/litert/android) — the interpreter this needs
- [Convert PyTorch to LiteRT](https://developers.google.com/edge/litert/models/convert_pytorch) — the export side, already exercised by `export_tflite.py`
- [machine-learning/plan.md](../../machine-learning/plan.md) — the export contract, and why it stops at the embedding
