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

Both were guesses on the first pass and both were wrong. They remain
`SandboxConfig`/`PreprocessConfig` fields rather than hardcoded values.

- **Threshold 0.4650** — the 5%-miss-budget operating point for the full-unfreeze checkpoint
  *under tile-max*, costing 10.09% over-blocking. It replaces a provisional **0.20**, which was
  the unfreeze-3 model's operating point. Note the same checkpoint under a centre crop operates
  at **0.2717**: a threshold belongs to a model *and* a geometry, because the max over tiles
  shifts the whole score distribution upward. See
  [the operating point decision](../../decisions/classifier-operating-point.md).
- **96px size floor** — the smallest measured arm still at or above 0.93 combined ROC-AUC. It
  replaces a provisional **32px**, which sits at 0.8619. Degradation is smooth rather than
  cliff-edged (0.9255 at 64px, still 0.7640 at 16px), so this is a chosen point on a curve.
  A floor is also a bypass: content served just under it is unfiltered.

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

6. ~~Replace the provisional threshold and size floor with measured values from
   `holy_blocker_ml.inputs`, and adopt whichever geometry that experiment selects.~~
   **Done.** Threshold 0.20 → **0.4650**, floor 32px → **96px**, and the centre crop replaced by
   **tile-max**. See [Measured constants](#measured-constants) and the
   [experiment](../machine-learning/experiments/input-handling.md).

Still to do:

7. `hash.rs` + `db.rs` as a short-circuit cache, if and when a hash database exists.
8. A **fully convolutional** equivalent of tile-max. MobileNetV3's `AdaptiveAvgPool2d(1) →
   Linear(576,1024) → Linear(1024,2)` head converts mechanically to 1×1 convolutions with
   identical weights, so one pass over a larger input yields a spatial logit grid whose max equals
   the tiled max — and, as a side effect, the coarse heatmap the screen-capture path needs for
   localisation. **Promoted from "if cost becomes a problem" to Stage 1 of
   [The screen path](#the-screen-path)**: on a screen frame it is not a cost optimisation but the
   option that removes the region-proposal problem entirely, since it scores every pixel and so
   carries no silent-miss class.

## The screen path

**Added after the macOS daemon's `ImageScanner` (module 18) shipped and was run live.** The
screen path is no longer independent of this crate — `check_raw` is its entry point, and
`packages/image-sandbox-ffi` carries it to the daemon. This section is the plan for making that
path correct, and its first instruction is **not to build anything yet**.

### The problem

`ImageScanner` hands `check_raw` a whole screen frame. Tile-max then cuts 224 windows across
3000×2000 of mostly application chrome — toolbars, flat colour fields, walls of monospace text —
none of which exists anywhere in the training distribution, and takes the **max** over those
out-of-distribution scores. Simultaneously, a 300px thumbnail inside that frame is a small patch of
one tile once the shorter side is resized to 224, so the detail that would decide the verdict is
discarded before inference.

Both live observations point here. Ordinary non-sexual photos firing is the first effect; drawn
content not firing is partly the second and partly a genuine concept gap in the checkpoint.

Note the three failures are separate and only one is a model problem:

1. **Geometry** — chrome scored, small regions destroyed. No amount of retraining touches this.
2. **Taxonomy** — a binary head cannot express "suggestive". Addressed by the three-tier contract.
3. **Medium** — a photo-trained concept does not transfer to drawn content. Needs positives to fix
   by training, which [image-corpus-custody.md](../../decisions/image-corpus-custody.md) forbids,
   so it is addressed by *delegation* to a second pretrained expert instead.

### What the literature actually calls this

Surveyed 2026-08-08. The nearest field is **screen content detection** in video coding, not
document analysis and not image forensics.

- **libaom** ships `estimate_screen_content()` in `av1/encoder/encoder.c`: tile the **luma** plane
  into 16×16 blocks, count **distinct luma values** per block, flag a block screen-ish at
  `n_colors ≤ 4`, tie-break on per-pixel variance. Frame verdict by block count.
- **SVT-AV1** adds an anti-aliasing-aware mode (`--scm 3`) with a three-way block taxonomy —
  *simple* (2–4 values), *complex* (5–40, re-counted after dilation), *photo-like* (>40) — and its
  design document states the failure of the naive form outright: screen content "has too many
  colors in total, and so the detection algorithm can get easily fooled into thinking it's actually
  natural content."
- Google (US7657089), Intel (US11399187) and Microsoft's RemoteFX tile classifier converge
  independently on the same two features: colour-frequency histogram plus spatial variance.

Two conclusions from that survey are load-bearing:

- **Nobody publishes a recall number.** libaom, SVT-AV1, Intel and Microsoft all ship or patent one
  of these and none reports precision or recall; SVT-AV1's doc says the thresholds were "determined
  experimentally". There is no operating point to inherit — any figure this project uses has to be
  measured here.
- Every published instance answers *"is this **frame** screen content?"*, never *"which
  **rectangles** are pictorial?"* The spatial version of the question is not solved anywhere
  public.

Rejected branches, recorded so they are not re-litigated: **document page segmentation / MRC /
Leptonica** (contract is 1bpp at 300–400ppi, and its halftone detector looks for print screening
that does not exist on a screen); **CG-vs-photo forensics** (95–98% headline accuracy, but the
signal is sensor noise and demosaicing residue, destroyed by the display pipeline — and its "CG"
class is content we must *keep*); **VIPS and web page segmentation** (DOM-driven, we have pixels);
**UIED / OmniParser / DocLayout-YOLO** (wrong target class — interactable widgets, not pictorial
regions — plus AGPL on the useful weights and, for PP-DocLayout-L, 760 ms on CPU).

### The finding that constrains the design

**A palette-cardinality discard rule would drop cel-shaded and anime artwork**, and that is not a
bug in someone's implementation — it is the design intent of the entire photo-vs-graphic
literature. Lienhart & Hartmann's canonical taxonomy places comics and cartoons on the *graphics*
side; Google's patent states the rationale ("a synthetic/graphical image is likely to contain a
limited range of colors compared to a natural photograph"); cel shading by construction quantises
light into 2–3 flat bands.

Drawn explicit content is precisely the class this path is already missing. So:

> **No discard rule keyed on colour cardinality alone may ship.** Any such rule needs a per-block
> variance/linework term, and even then the aggregation must be asymmetric — a 224 tile holds 196
> 16×16 blocks; discard only if *essentially all* of them are simple *and* low-variance.

And specifically: **do not port SVT-AV1's dilation step.** It exists to reclassify anti-aliased
text as screen content, and it would take flat art with it.

Anti-aliased text fails in the safe direction — it reads as photographic and is kept, costing an
inference. That makes it a cost problem, not a miss problem, but a large one, since a code editor
is mostly anti-aliased text.

**Evidence gap, do not treat as settled:** no published distribution of distinct-colour-per-block
for cel-shaded art vs UI vs photos, and no study of any of these statistics on modern translucent /
vibrancy / gradient OS UI — the standard screen-content corpora (SIQAD, SCID) predate it.

### The reframing that decides the order of work

This crate already tiles and takes a max. A region proposer is therefore **not proposing regions —
it is choosing which of the tiles we were going to run anyway to skip.** Measured cost is 5.3 ms
for one tile, 9.6 ms at 1024², and a 1.54-aspect display frame costs three tiles.

So a content gate buys roughly 15 ms twice a second, and pays for it with a **silent-miss class
that produces no log line**. That is the worst risk/reward shape in the system, and two
alternatives dominate it:

- **Dirty-rectangle change detection.** Per-16×16-block hash against the last frame *analysed* —
  the same rule `apps/mobile`'s `FrameGate` already learned — skips tiles because nothing changed,
  not because something was judged boring. Same single pass, no silent-miss class. A static editor
  costs nothing; a playing video costs full price.
- **The fully convolutional pass already recorded as step 8 below.** One forward pass whose spatial
  max equals the tiled max, no recall liability at all, overlap computed once instead of twice, and
  it yields the heatmap that would finally populate the daemon's reserved
  `ScanVerdict.regions` and let `Overlay` cover a rectangle rather than hide a whole application.
  Open question is purely empirical: a stride-32 pass over 3000×2000 is roughly 50× a single 224²
  pass.

### Staged plan

Each stage is independently shippable, and each one's result decides whether the next is worth
doing. **Nothing here starts by writing a gate.**

**Stage 0 — the transport-shift measurement.** Build it in `machine-learning` (see that plan's
`synth_composite.py` and `transport.py`) and run it against the pipeline *as it exists today*.
This is the baseline every later claim is measured against, it needs only benign imagery, and it
splits the observed misses into "geometry, fixable and measurable" versus "concept, needs a
different expert". Right now that split is unknown.

**Stage 1 — the FCN cost measurement.** Convert the head to 1×1 convolutions, run one pass over a
real frame size, and time it. If it fits the ~500 ms tick budget, build it and **stop** — the gate
never gets written, and the region-proposal problem and its corpus both disappear.

**Stage 2 — dirty-rectangle dedup**, if cost still needs reducing. No recall liability, so it can
land regardless of what Stages 0 and 1 say.

**Stage 3 — a content gate, only if Stages 1 and 2 are insufficient.** Build libaom's, not a
region-level colour-concentration statistic: luma only; **every pixel** of every 16×16 block (see
the striding trap below); a 256-entry stack histogram; the three-way block taxonomy; aggregate per
224 tile; discard only when essentially every block is *simple* and low-variance. No dilation. Zero
new dependencies, pure Rust, single pass, and the thresholds are `SandboxConfig` fields with no
built-in defaults, exactly as the existing constants are.

**Stage 4 — a second expert for drawn content**, routed non-exclusively (below confidence, run
both and take the max — fail toward more inference, never fewer). This is the only stage that
addresses failure (3), and its recall cannot be measured here; see *What stays unmeasurable*.

### Traps to carry into implementation

- **Do not subsample by striding.** Two mechanisms, both mechanical: striding can lock phase
  against ordered dithering or a subpixel-AA glyph grid and return an artificially *low* colour
  count; and a 96×96 thumbnail — exactly the existing size floor — contributes ~144 samples to a
  region histogram of tens of thousands and is statistically invisible. libaom reads every pixel;
  its speed comes from one linear pass with a stack histogram, not from sampling.
- **Any resize that interpolates destroys the exact-palette signal**, making flat UI look
  photographic. Same class of bug as the `bytesPerRow` padding trap.
- **Block granularity, not region granularity.** Anything additive that covers part of a region —
  a caption bar, letterbox bars, a solid product-shot background — moves a region-wide statistic
  that is supposed to describe the rest of it. Per-block classification plus "any sufficiently
  large connected cluster" handles all three cases with one rule, and the block map *is* the region
  proposal, so no separate growing step is needed.
- **A text overlay is not a reason to discard anything.** Explicit imagery routinely carries
  captions, subtitles, watermarks and meme text. The predicate must be *absence of pictorial
  evidence*, never *presence of text*. Text coverage is legitimately a **geometry selector** —
  heavy coverage argues for sub-tiling within the region so the unoccluded part gets a
  full-resolution look — but never a reject predicate.
- **Instrument every discard.** Log the gate's decision and its margin exactly as the classifier's
  score is logged, and **defeat the gate on 1 frame in 20**, running the full tile set and
  recording any gated-out tile that would have scored above threshold. ~5% extra cost, and it is
  the only route to a false-discard number, since no published source has one. A silently
  discarding gate is indistinguishable from a clean screen — the same failure the `pixelFormat`
  incident cost a session on.

### What stays unmeasurable

Under [image-corpus-custody.md](../../decisions/image-corpus-custody.md) these have no measurement
available here, and must not be given a number:

- **Recall on drawn explicit content.** A drawn-content expert can be validated on false positives
  and on latency; its recall is inherited from its publisher's evaluation plus live observation.
- **The "suggestive" tier's boundary.** The band has no measurable edge without positives spanning
  it.
- **Any threshold's absolute correctness.** Thresholds are chosen against the false-positive curve,
  which is observable; the miss side is inherited.

What substitutes for an end-to-end miss rate is an **argument, labelled as one**: the checkpoint's
published image-level miss rate, times a *measured* near-zero transport shift, implies a comparable
screen miss rate. Its validity rests entirely on that shift being measured to be near zero and on
the transformation being content-blind — crop, scale, composite and overlay do the same thing to a
photo of a dog as to anything else. Everything else in the measurement table is a real number.

## What this does not cover

- Video frame handling — that is `packages/video-watchdog` (Phase 5 of the network pipeline).
- Screen-capture *scheduling and response* — the cadence, dedup and overlay decisions live in the
  daemon's `ScanLoop`/`ImageScanner`. The *pixels-to-score* half is this crate's `check_raw`, and
  the plan for it is [The screen path](#the-screen-path) above.
- Training or updating the ONNX model — that is `machine-learning/models/web-image-v1/` (see [architecture.md](../../architecture/overview.md)).
- Hash database population — data curation and hash ingestion are out-of-repo operations; this package only reads an already-populated `hashes.sqlite`.
- Serving the 1×1 transparent pixel response to the browser — that is the responsibility of `packages/mitm-proxy`, which calls `ImageSandbox::check` and acts on the returned verdict.
