# ADR: Tiered On-Device Inference Pipeline

| Field | Value |
|---|---|
| **Status** | Proposed |
| **Date** | 2026-06-14 |
| **Owner** | Ivan Dutov |
| **Stakeholders** | Platform engineering, ML, UX |
| **Supersedes** | — |
| **Superseded by** | — |

---

## Context

A proposed "Adaptive On-Device Content Filtering Pipeline" blueprint described a
three-tier inference cascade (perceptual-hash gate → cheap detector → multimodal
judge), an adaptive NPU→GPU→CPU execution fallback, and a set of user controls.
The shape is sound and aligns with the project's existing design, but as drafted it
proposed technology choices that conflict with the committed stack and omitted parts
of the system that are already built or specified.

This ADR records the blueprint reconciled against the committed architecture: which
parts are adopted, which are changed, which are deferred, and the open work items the
reconciliation creates. It is scoped to the **inference cascade and its runtime** — the
classification "brain" that both the network path and the render path feed. It does
**not** re-decide the interception surfaces; those are fixed in
[content-interception.md](content-interception.md). Where the cascade runs (network
proxy Phase 4 vs render-path capture) is that ADR's concern; *how it classifies* is
this one's.

The cascade is the shared, platform-neutral component the whole ecosystem depends on
(see [content-interception.md](content-interception.md) § Highest-leverage components).
Getting its model and runtime choices right matters more than any single platform
adapter.

---

## Decision: a three-tier cascade on ONNX Runtime, gated by cost

Content (a decoded image plus its surrounding text context) passes through tiers of
increasing cost. Each tier may resolve the verdict and short-circuit the rest. Cheap,
deterministic, high-volume filtering happens first; the expensive multimodal model runs
only on the residual uncertain band.

```text
[ decoded image + surrounding text (before / after / OCR-on-image) ]
        │
        ▼
┌──────────────────────────────────────────────────────────────┐
│ TIER 1 — Hash gate (two tables, blocklist checked first)       │
│   • Blocklist  (shared, distributable hashes.sqlite)           │
│       hit within BLOCK_THRESHOLD → Block                       │
│   • Cleared-cache (local, per-device, tighter radius)          │
│       hit → skip cascade (already-cleared, e.g. page refresh)  │
└──────┬─────────────────────────────────────────────────────────┘
       │ miss
       ▼
┌──────────────────────────────────────────────────────────────┐
│ TIER 2 — Visual tripwire (MobileNetV3-Small, ONNX/TFLite)      │
│   high-recall threshold; clearly-safe → Allow, else escalate   │
└──────┬─────────────────────────────────────────────────────────┘
       │ uncertain / trigger
       ▼
┌──────────────────────────────────────────────────────────────┐
│ TIER 3 — Multimodal judge (CLIP-family, zero-shot first)       │
│   image + concatenated text context → label similarities       │
│   verdict is one evidence source; text-policy engine is another │
└──────────────────────────────────────────────────────────────┘
```

In parallel and independent of the image cascade, OCR-extracted and
browser/accessibility text always runs through the deterministic **text-policy engine**
([content-classification.md](../architecture/content-classification.md)). The cascade
above decides the *image* verdict; the text-policy engine decides the *text* verdict;
the final decision combines both (a phrase plus a matching image category is an existing
context amplifier). The multimodal judge does not replace the rule engine — see the text
note below.

### Tier 1 — two tables, not one

The original blueprint framed Tier 1 as a "whitelist" that passes matches instantly.
That is the wrong primitive on its own: the project's perceptual-hash database is a
**blocklist** ([image-sandbox/plan.md](../components/image-sandbox/plan.md) — hash hit →
`Block`, `BLOCK_THRESHOLD = 10`). Tier 1 therefore holds **two** tables:

- **Blocklist** — the shared, distributable `hashes.sqlite`. Optimized for wide
  distribution and fast Hamming lookup. Hit within `BLOCK_THRESHOLD` → `Block`.
- **Cleared-cache** — a local, per-device memoization table populated at runtime by
  prior `Allow` verdicts. Its purpose is to avoid re-running the whole cascade on
  content the user has already seen cleared — page refreshes in a browser, repeated
  native imagery, redraws. Hit → skip the cascade.

Rules that keep the cleared-cache from becoming a hole:

1. **Blocklist is checked first, cleared-cache second.** A stale `Allow` must never
   shadow a blocklist hit after the blocklist is updated.
2. **Invalidate on policy change.** Tag cleared-cache entries with the blocklist version
   and the active pessimism threshold; treat a version/threshold mismatch as a miss (or
   flush). Otherwise newly-blockable images stay whitelisted after an update.
3. **Tighter radius than the blocklist.** The cleared-cache matches near-exact
   re-encodings (e.g. Hamming ≤ 5), while the blocklist keeps its own wider radius
   (≤ 10). Because the blocklist is checked first at its own radius, a loose cleared
   radius cannot swallow a should-block near-duplicate.
4. **Cache the pixel decision, not the page decision.** pHash is pixels only. The same
   image can be safe in one context and flagged-by-surrounding-text in another, so a
   cleared-cache hit skips the *image* cascade but text-policy still runs on the new
   context.
5. **Bounded with eviction.** LRU with an entry/age cap; native imagery churns and an
   unbounded cache grows without limit.

### Tier 2 — MobileNetV3-Small, not YOLO

The committed model is the MobileNetV3-Small classifier already in
[machine-learning/plan.md](../components/machine-learning/plan.md), exported to ONNX
(Windows) and TFLite (Android). The blueprint's YOLOv8-Nano object detector is **not**
adopted: a detector needs a bounding-box-labelled dataset that does not exist, and the
ML pipeline is built around a classifier, not a detector. Tier 2 runs at a high-recall
(low) threshold so the benign majority is allowed cheaply and anything uncertain
escalates.

### Tier 3 — CLIP-family multimodal judge, zero-shot first

Some content is only judgeable with its surrounding context — text before the image,
text after it, and OCR'd words on the image itself. A CLIP-family dual-encoder model is
the right tool for this band, and it stays at Tier 3 (rare, gated) because it is heavier
than the Tier 2 CNN.

The important consequence: **start with zero-shot, defer the supervised head.**

- **Zero-shot** compares the joint image+text embedding against label *prompts* using
  cosine similarity. It needs **no labelled training data** to add a category — which
  directly sidesteps the project's hardest blocker (datasets do not exist; see § Datasets
  deferred). This is the primary reason to adopt CLIP, not merely "context helps."
- A **supervised multilabel fusion head** over `[text-before, text-after, OCR-words,
  image]` is the eventual goal, but it is supervised — it needs exactly the labelled
  data the project lacks. It is deferred until a useful eval/training set exists.

Mechanics to record so the build is not surprised:

- CLIP does not natively ingest "two sentences plus an image." Textual context is
  concatenated into one string, tokenized, and run through the **text encoder**; the
  image runs through the **image encoder**; the two embeddings meet in the joint space.
- This requires shipping a **tokenizer on-device** (BPE vocab + merges) and implementing
  text tokenization in the native/Rust layer. No current plan covers this — it is a new
  work item.

### Text-policy stays a separate evidence source

OCR output feeds **both** the deterministic text-policy engine (always, cheap,
explainable, evasion-aware) **and** the Tier 3 text input (uncertain cases only). The
multimodal model is not folded into the text engine.
[content-classification.md](../architecture/content-classification.md) is explicit that
deterministic rules must remain an independent evidence source so the model never becomes
the only source of truth. CLIP is the judge for the middle band; the rule engine is the
fast, inspectable floor.

---

## Runtime: ONNX Runtime + Execution Providers (not ExecuTorch)

The blueprint proposed ExecuTorch with explicit QNN / CoreML / XNNPACK backend mapping.
This is **deferred to future research**, for three reasons specific to a Windows-first
build:

1. **QNN and CoreML do not help the first target.** QNN is Qualcomm-SoC-specific
   (mobile, or Snapdragon-X Windows only); CoreML is Apple-only. The first prototype is
   x86 Windows with the existing Electron host and `win-daemon`. Neither backend fires
   there — the device would run CPU or DirectML regardless.
2. **It adds a third export pipeline.** The ML plan already maintains torch→ONNX
   (Windows) and torch→TFLite (Android). ExecuTorch is a third lowering path with its own
   quantization story and op-coverage gaps; partial delegation silently falls back to
   portable CPU ops — the very "runtime blindly partitions the model" problem the
   blueprint wanted to avoid, relocated rather than solved.
3. **Distribution cost.** A QNN `.so` per chip generation; a CoreML `.mlpackage` needs
   the macOS toolchain in CI.

**The adaptive fallback the blueprint wanted is native to ONNX Runtime.** ORT's
Execution Provider mechanism already does NPU→GPU→CPU selection: DirectML EP (GPU/NPU on
Windows), QNN EP, CoreML EP, and XNNPACK/CPU as the portable floor. The Phase 3 adaptive
execution chain is therefore implemented as **EP priority ordering within ONNX Runtime**,
not a separate runtime. This delivers the blueprint's adaptive behavior on the committed
stack, with no new dependency.

ExecuTorch is revisited only if ORT's on-device NPU throughput proves insufficient on
mobile — a measured decision, not an upfront one.

Reference documents:

- ONNX Runtime Execution Providers — <https://onnxruntime.ai/docs/execution-providers/>
- DirectML EP — <https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html>
- CLIP (Radford et al., 2021) — <https://arxiv.org/abs/2103.00020>
- MobileCLIP — <https://arxiv.org/abs/2311.17049>

---

## User controls: pessimism is a threshold band within a mode

The blueprint's "pessimism slider" and "hardware performance profiles" are adopted, but
kept **orthogonal** to the protection-mode state machine in
[protection-modes.md](protection-modes.md). Two independent controls:

- **Protection mode** (`full` / `warn` / `off`) — decides *what action* a flag produces
  and gates the voice gate and partner accountability. Unchanged by this ADR. The slider
  must never move a mode transition; in particular it must not influence the transition
  to `off` that fires the voice gate.
- **Pessimism threshold band** — decides *how sensitive* the cascade is. The slider moves
  a confidence threshold **within a range defined by the active mode**, with a
  mode-specific floor and ceiling (e.g. a "Max Guard" mode cannot slide above a lax
  ceiling; an "Eco" mode cannot drop below a minimum). Lowering the threshold raises
  Tier 2 recall and therefore Tier 3 wake-ups.

Hardware profiles (Eco / Balanced / Max Guard) map to EP selection and resolution
scaling: Eco may cap at Tier 2 or CPU-only and shrink Tier 2 input resolution; Balanced
uses the full EP fallback chain; Max Guard locks the highest-accuracy EP path and accepts
cover/blur friction.

---

## Rejected / deferred

- **ExecuTorch + QNN / CoreML as the prototype runtime** — deferred to future research;
  ORT Execution Providers cover the adaptive fallback on the committed stack. Revisit only
  if measured mobile NPU throughput on ORT is insufficient.
- **YOLOv8-Nano object detector at Tier 2** — rejected for the prototype; needs a
  bounding-box dataset that does not exist, and the ML pipeline is classifier-based.
- **Supervised multimodal fusion head** — deferred until labelled training/eval data
  exists; zero-shot CLIP is the data-free starting point.
- **Browser-extension content ingestion (DOM scan as a capture source)** — out of scope
  for this ADR and not adopted as an ingestion path; the network path already inspects
  content via the MITM proxy. (This is distinct from the extension's overlay-applicator
  role discussed in [content-interception.md](content-interception.md).)

## Datasets deferred

Sourcing and curating training/eval data is the project's hardest unsolved problem and is
explicitly out of scope for this ADR. The `data/` tree is gitignored and curation is an
out-of-repo process ([machine-learning/plan.md](../components/machine-learning/plan.md)).
The decision to start Tier 3 zero-shot is partly a consequence: it lets the cascade reach
the broad Philippians 4:8 categories without first solving the dataset problem.

---

## Open questions / work items

- **CLIP tokenizer on-device** — shipping the BPE vocab/merges and implementing text
  tokenization in the native/Rust layer. Not covered by any current plan.
- **Cleared-cache table** — add a second table to `packages/image-sandbox` alongside the
  blocklist, with policy-version tagging, a tighter Hamming radius, and LRU eviction.
- **ORT EP fallback chain** — wiring DirectML / QNN / CoreML / CPU EP priority and the
  resolution-scaling step that triggers when the cascade falls back to a slower EP.
- **Zero-shot label prompt set** — the prompts the Tier 3 judge compares against; this is
  effectively the policy surface for the broad categories and needs its own review.
- **Tier 2 high-recall threshold tuning** — the lower the threshold, the more Tier 3
  wake-ups; needs validation against latency/battery targets once the cascade runs.
