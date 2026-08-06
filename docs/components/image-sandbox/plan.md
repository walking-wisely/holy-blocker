# Image Sandbox — Implementation Plan

The design rationale and pipeline context live in [network-pipeline.md](../../architecture/network-pipeline.md) (Phase 4) and [content-classification.md](../../architecture/content-classification.md) (image classifier strategy).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

## Current state

**v0 is built and wired.** `packages/image-sandbox/` exists as a Rust library crate and
`mitm-proxy` calls it at the Phase 4 hook. Verified live: an HTTPS PNG fetched through the
tunnel returns `403 Blocked` at `--image-threshold 0` and 200 at `1`.

**This crate is the Windows/desktop half of a deliberate per-platform split, and is not the
Android path.** The runtime decision recorded in
[learning-from-feedback.md](../../decisions/learning-from-feedback.md#on-device-runtime-and-where-the-head-lives-decided-2026-07-18)
is LiteRT on Android, ONNX Runtime here, and the head — the part after the embedding — hand-written
in Rust behind UniFFI and shared by both (see [classifier-head/plan.md](../classifier-head/plan.md)).
Wiring `apps/mobile` to this crate would collapse that split; `ort` also ships no prebuilt runtime
for two of the three Android ABIs the app targets, measured 2026-07-26. When the head crate lands,
`classifier.rs` should hand its score contract over rather than keep a second copy of it.

What shipped, and how it differs from the order below:

- **ONNX first, hashing deferred.** The plan builds pHash and the SQLite blocklist before the
  model. No hash database exists or can be populated in-repo — the plan itself calls curation
  an out-of-repo operation — so that path blocks nothing on day one. It is a cache in front of
  the model, not a prerequisite for it. `hash.rs` and `db.rs` are still unbuilt.
- **`preprocess.rs` replaced the transform this plan specified.** See the correction below.
- **`classifier.rs`, not `onnx.rs`**, and it exposes the score rather than a label, because the
  threshold comparison belongs to the caller.

### Correction: the preprocessing in this plan was wrong

Sections 3 and 4 below say to "resize to 224×224 (bilinear)". That is a squash, and it matches
neither the training nor the evaluation transform. The model was evaluated under
`dataset.build_transform(224, augment=False)`:

```
Resize(int(224 * 1.14))   # shorter side to 255, aspect preserved
CenterCrop(224)
ToTensor(); Normalize(IMAGENET_MEAN, IMAGENET_STD)
```

`torchvision.transforms.Resize` with a scalar resizes the **shorter** side. Every number in
[results.md](../machine-learning/results.md) was measured under that pipeline, so a squash
would mean the published accuracy describes something other than what ships.

`tests/parity.rs` guards this against a torchvision-generated fixture, and it immediately caught
a second-order version of the same class of bug: torchvision's `CenterCrop` computes
`int(round((extent - crop) / 2.0))`, rounding a half-pixel offset **up**, where the obvious Rust
integer division truncates. That one-pixel shift moved the tensor mean by 0.0085 — systematic,
not noise, and invisible to any test that only checks Rust against itself.

### Correction, part two: the centre crop is not what ships either

The deployed geometry is **tile-max**, not the centre crop described above. The
[input-handling experiment](../machine-learning/experiments/input-handling.md) measured four
candidates against off-centre composites and the centre crop lost badly: it caught **41%** of
explicit content where tiling caught **62%** at each arm's own calibrated threshold, because it
discards ~23% of a wide image and explicit content in the side of a banner is simply never seen.
Tiling costs nothing on ordinary imagery — a near-square image yields exactly one tile, and plain
validation AUC is unchanged at 0.9806 against 0.9796.

`preprocess_tiles` resizes the shorter side to **224 exactly** — not `224 × 1.14`, since no crop
follows — then slides 224 windows along the longer axis at a half-window stride, with the final
window flush against the far edge. The caller takes the **maximum** score: averaging dilutes a
small explicit region into a large safe background, which is the failure being fixed.

`preprocess` (the centre crop) is kept, because every published figure was measured under it and
`tests/parity.rs` pins it against torchvision. `tests/tile_parity.rs` does the same for the tiled
path against `holy_blocker_ml.inputs.tile_geometry`, using a fixture whose green channel ramps
left-to-right so that tile *position* is visible in each tile's mean.

### Measured constants

Both were guesses on the first pass and both were wrong.

- **Threshold** — has no shipped default at all: `SandboxConfig`, the FFI surface, and the
  daemon's `HOLY_BLOCKER_IMAGE_THRESHOLD` all require the caller to supply one explicitly. A
  threshold belongs to a model *and* a geometry, because the max over tiles shifts the whole
  score distribution relative to a centre crop, and this project has already shipped the wrong
  pairing more than once with nothing failing to indicate it. See
  [the operating point decision](../../decisions/classifier-operating-point.md).
- **96px size floor** — the smallest measured arm still at or above 0.93 combined ROC-AUC. It
  replaces a provisional **32px**. Degradation is smooth rather than cliff-edged, so this is a
  chosen point on a curve. A floor is also a bypass: content served just under it is unfiltered.

The 8:1 aspect clamp remains, now bounding *inference count* rather than allocation — the widest
admissible image resizes to 1792×224 and costs 15 forward passes.

## Modules to add

### 1. `hash` — perceptual hashing

```
src/hash.rs
```

Pure computation module — no I/O, no state, easy to unit test.

Responsibilities:

- Compute a DCT-based perceptual hash (pHash) from raw pixel data.
- Compare two hashes by Hamming distance.
- Expose the default similarity threshold used by the lookup layer.

Key types and signatures:

```rust
/// Compute a 64-bit DCT-based perceptual hash from raw pixel data.
/// `pixels` is an RGB or greyscale flat buffer, row-major.
pub fn perceptual_hash(pixels: &[u8], width: u32, height: u32) -> u64

/// Number of bits that differ between two hashes.
pub fn hamming_distance(a: u64, b: u64) -> u32

/// Images whose Hamming distance is at or below this value are treated as
/// visually identical for blocking purposes.
pub const BLOCK_THRESHOLD: u32 = 10;
```

The pHash algorithm: reduce to 32×32 greyscale, apply a 2-D DCT, retain the top-left 8×8 DC coefficients (excluding the DC mean), compare each coefficient to the mean of the 64 values, encode the comparison results as one bit per coefficient. This yields a stable 64-bit fingerprint that is robust to JPEG re-encoding, minor crops, and colour shifts.

### 2. `db` — SQLite hash lookup

```
src/db.rs
```

Wraps a `rusqlite` connection to the local hash database and exposes a Hamming-distance–aware lookup.

Responsibilities:

- Open and hold a connection to `data/hash-db/hashes.sqlite`.
- Provide a schema migration that creates the `hashes` table on first run.
- Query for stored hashes within a configurable Hamming distance of a probe hash.

Key types and signatures:

```rust
pub struct HashDb {
    conn: rusqlite::Connection,
}

/// Result returned when a stored hash is within `threshold` bits of the probe.
pub struct DbMatch {
    pub stored_hash: u64,
    pub label:       String,
    pub distance:    u32,
}

impl HashDb {
    /// Open (or create) the database at `path`.
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self>

    /// Return the closest matching entry within `threshold` Hamming bits,
    /// or `None` if no entry is close enough.
    pub fn lookup(&self, hash: u64, threshold: u32) -> Option<DbMatch>
}
```

Schema:

```sql
CREATE TABLE hashes (
    hash  INTEGER PRIMARY KEY,
    label TEXT    NOT NULL
);
```

Hamming lookup strategy: SQLite does not support a native bitwise Hamming distance operator efficiently. The chosen approach for the first implementation is a full-table scan with a computed `(hash ^ probe) POPCOUNT` equivalent applied in the Rust layer after fetching all hashes. For databases up to several million rows this remains fast enough because each row is a single 8-byte integer. A neighbourhood-index optimisation (BK-tree pre-built at load time or a Vantage-Point tree over hashes) is deferred and noted with a TODO comment.

### 3. `onnx` — ONNX inference fallback

```
src/onnx.rs
```

Wraps an ONNX Runtime session behind a Cargo feature flag. When the `onnx` feature is disabled the module compiles to a zero-cost stub that always reports safe.

Responsibilities:

- Load a quantized ONNX vision model from `data/models/web-image-v1/model.onnx` (see [architecture.md](../../architecture/overview.md)).
- Resize and normalise an image to 224×224 using ImageNet mean/std before inference.
- Return a label and confidence score.

Key types and signatures:

```rust
pub struct ClassifyResult {
    pub label:      String,
    pub confidence: f32,
}

pub struct ImageClassifier {
    #[cfg(feature = "onnx")]
    session: ort::Session,
}

impl ImageClassifier {
    /// Load the model from `model_path`. Requires the `onnx` feature.
    #[cfg(feature = "onnx")]
    pub fn load(model_path: &std::path::Path) -> ort::Result<Self>

    /// Run inference on raw pixel data.
    /// When the `onnx` feature is disabled, always returns
    /// `ClassifyResult { label: "safe", confidence: 1.0 }`.
    pub fn classify(&self, pixels: &[u8], width: u32, height: u32) -> ClassifyResult
}
```

Model input preparation: resize to 224×224 (bilinear), convert to `f32`, apply per-channel normalisation with ImageNet mean `[0.485, 0.456, 0.406]` and std `[0.229, 0.224, 0.225]`, arrange as `NCHW`. The `ort` crate (ONNX Runtime Rust bindings) is the intended dependency, gated behind the `onnx` feature so that builds without a local ONNX Runtime install remain functional as stubs.

### 4. `sandbox` — decision entry point

```
src/sandbox.rs
```

Wires decoding → hashing → DB lookup → optional ONNX inference into a single public call.

Responsibilities:

- Accept raw compressed image bytes (JPEG, PNG, WebP, or GIF).
- Decode using the `image` crate.
- Compute the perceptual hash.
- Query `HashDb`; on a hit, return a block verdict immediately.
- On a miss, run `ImageClassifier` if the `onnx` feature is enabled; if the model returns an unholy label above its confidence threshold, return a block verdict.
- If neither the DB nor ONNX identifies the image, return Allow.

Key types and signatures:

```rust
pub enum ImageVerdict {
    Allow,
    Block { reason: String },
}

pub struct ImageSandbox {
    db:         HashDb,
    classifier: Option<ImageClassifier>,
}

impl ImageSandbox {
    pub fn new(db: HashDb, classifier: Option<ImageClassifier>) -> Self

    /// Decode `image_bytes`, hash, look up, and optionally classify.
    /// Returns `Allow` or `Block { reason }`.
    pub fn check(&self, image_bytes: &[u8]) -> ImageVerdict
}
```

Decision pipeline:

```text
image_bytes
  -> image::load_from_memory (JPEG / PNG / WebP / GIF)
  -> convert to RGB8 flat buffer
  -> perceptual_hash(pixels, width, height)
  -> db.lookup(hash, BLOCK_THRESHOLD)
       Some(match) -> Block { reason: match.label }
       None        -> continue
  -> classifier.classify(pixels, width, height)   [if Some]
       unholy above confidence threshold -> Block { reason: label }
       safe or classifier is None        -> Allow
```

Decode errors (unsupported format, truncated buffer) return Allow and log a warning rather than blocking, to avoid false positives on malformed but harmless images.

### 5. `lib` — crate root and re-exports

```
src/lib.rs
```

Re-exports the public API surface:

```rust
pub use sandbox::{ImageSandbox, ImageVerdict};
pub use db::{HashDb, DbMatch};
pub use hash::{perceptual_hash, hamming_distance, BLOCK_THRESHOLD};
pub use onnx::{ImageClassifier, ClassifyResult};
```

## Implementation order

The order below is the original plan. What was actually built is recorded under
**Current state** — steps 1 and 2 are deferred, and the model path was built first.

1. `hash.rs` — pHash computation and Hamming distance; unit test with known image pairs and known hash values to pin the algorithm output. **Deferred** — no hash database exists to look into.
2. `db.rs` — SQLite wrapper; test with an in-memory database (`rusqlite::Connection::open_in_memory()`), insert known hashes, verify lookup returns the correct match and correct distance. **Deferred** for the same reason.
3. ~~`sandbox.rs`~~ **Done**, though with `db: None` rather than `classifier: None` — the model is what blocks, so the stub went on the other side.
4. ~~`onnx.rs` behind the `onnx` feature flag~~ **Done** as `classifier.rs`. Non-default feature; without it the crate compiles and allows everything, so a build without an ONNX Runtime is a functioning build. `tests/inference.rs` exercises the real exported artifact and skips when it is absent, since `data/models/` is gitignored.
5. ~~Wire `ImageSandbox` into `packages/mitm-proxy` at the Phase 4 hook~~ **Done.** Inference runs under `tokio::task::spawn_blocking` — `image_scanner` is a sync `Fn` invoked inside the async handler, so running a MobileNetV3 forward pass inline would hold a tokio worker and stall every other connection it drives.

6. ~~Replace the provisional threshold and size floor with measured values, and adopt whichever
   geometry the input-handling measurement selects.~~
   **Done.** The size floor is now **96px**, the geometry is **tile-max**, and the threshold has
   no shipped default at all — the caller supplies it explicitly. See
   [Measured constants](#measured-constants).

Still to do:

7. `hash.rs` + `db.rs` as a short-circuit cache, if and when a hash database exists.
8. A **fully convolutional** equivalent of tile-max, if the per-image cost of up to 15 forward
   passes becomes a problem. MobileNetV3's `AdaptiveAvgPool2d(1) → Linear(576,1024) →
   Linear(1024,2)` head converts mechanically to 1×1 convolutions with identical weights, so one
   pass over a larger input yields a spatial logit grid whose max equals the tiled max — and, as
   a side effect, the coarse heatmap the screen-capture path will need for localisation.

## What this does not cover

- Video frame handling — that is `packages/video-watchdog` (Phase 5 of the network pipeline).
- Screen-capture image classification — that path goes through the daemon's `IScanner` interface and is independent of this package.
- Training or updating the ONNX model — that is `machine-learning/models/web-image-v1/` (see [architecture.md](../../architecture/overview.md)).
- Hash database population — data curation and hash ingestion are out-of-repo operations; this package only reads an already-populated `hashes.sqlite`.
- Serving the 1×1 transparent pixel response to the browser — that is the responsibility of `packages/mitm-proxy`, which calls `ImageSandbox::check` and acts on the returned verdict.
