# Model and classifier review traps

Applies to `packages/image-sandbox`, `packages/image-sandbox-ffi`, `packages/classifier-head`,
and `machine-learning`.

## Thresholds and operating points

- **A threshold belongs to a model *and* a geometry *and* an input distribution.** This project has
  shipped the wrong pairing more than once with nothing failing to indicate it, which is why no
  default threshold exists anywhere in the stack and every caller must supply one explicitly. A
  change that reintroduces a default is a finding.
- A threshold measured on centre-cropped web images does not describe tiled screen frames. Where a
  path operates outside its measurement, that must be stated at the path, not inferred from a
  decision doc — and the score should be logged so the margin stays observable.
- Reduction across tiles is **max**, not mean. Averaging dilutes a small explicit region into a
  large safe background — the exact failure being fixed.
- If any one tile fails, abandon the whole image: a max over a subset scores a different image.

## Scores and outcomes

- "Nothing was classified" must stay representable separately from "classified as 0.0". Collapsing
  them makes a silently broken image path read as a clean screen — the same failure class as the
  `pixelFormat` incident.
- Construction is fallible, classification is not. A daemon reporting image scanning as on while
  classifying nothing is precisely what a fallible constructor exists to prevent; making every
  caller reimplement fail-open is how that rule gets got wrong.
- A tile is not a located object — a bounding-box field must not be populated from a classifier.

## Preprocessing parity

- Pin any preprocessing path against its reference implementation with a fixture whose structure
  exposes position and offset (a channel ramp, not a flat colour). A flat fixture cannot see a
  one-pixel crop shift.
- Where two entry points must agree (encoded bytes vs raw framebuffer), assert their equality on the
  **real model**, not a synthetic buffer, or every published figure stops describing the shipped path.
- Alpha is dropped for an RGB tensor; a short buffer is refused rather than read past; a longer one
  is accepted with its tail ignored only if the guard on the producing side matches.

## Runtime and packaging

- Build for **every** shipped ABI, not one. `ort` has no prebuilt runtime for `x86_64-linux-android`
  or `armv7-linux-androideabi`; this is why the runtime split exists (LiteRT on Android, ONNX on
  Windows/macOS, shared head in Rust). **Do not wire `apps/mobile` to `packages/image-sandbox`.**
- A bundled model must be sealed by the signature: appending one byte should make verification fail.
  If a swapped model silently disables the path instead, that is the finding.
- Inference cost is a main-thread question. Measure it against the tick cadence before assuming it
  fits, and dispatch off the drawing thread.

## Data, provenance, and evaluation

- No pornographic corpus anywhere carries a credible CSAM-screening claim; the category is empty.
  Any plan that assumes otherwise rests on a false premise.
- Do not add explicit corpora, generated adult-content fixtures, private blocklists, or evaluation
  samples to this repo. A review that asks for one as evidence is asking for the wrong evidence.
- An evaluation figure must name the geometry it was measured under. A number carried across a
  geometry change is not a measurement of the new path.
- Claims about a model's training data are provenance claims: cite the model card or mark them
  `UNVERIFIED`. Do not infer provenance from a model's behaviour.
