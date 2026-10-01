# Decision: External and Self-Hosted "System One" Classifiers for Content Detection

## What was decided

No cloud-hosted classifier — Jev/TypeSafe or any other hosted LLM/VLM judge —
is used on any live content-detection path in Holy Blocker, for text or for
images, for any user other than a single developer explicitly opting their own
personal instance in.

Self-hosted open-weight "System One" models are **deferred, not adopted**:

- **Laya** (text) is deferred behind Jev. Researching a model expected to sit
  below Jev's capability ceiling is not worth doing before Jev itself was
  already ruled out on architecture/legal grounds unrelated to capability.
- **VisionLaya** (image) is deferred for a stronger reason than "unbenchmarked" —
  see below.

The active investment for image-classification quality is **established,
purpose-built, locally-run classifiers** (NudeNet-style, and an
illustration/anime-specific classifier) added as second-opinion/ensemble
signals alongside `packages/image-sandbox`'s existing tile classifier — not a
general-purpose vision-language model.

Quality measurement for the image path is split in two, deliberately:

- **False-positive rate and the safe/sexy boundary** are measurable now, with
  a self-assembled eval set built entirely from legal, non-explicit imagery
  (clothed people, swimwear/lingerie advertising, anime already rated safe).
- **Recall on actual explicit content is never measured in-house.** It is
  cited from published third-party benchmarks (UnsafeBench, NudeNet's own
  reported eval, LSPD's paper) whose raw datasets are never downloaded — only
  their published numbers are used, never their images.

## Why cloud classifiers were rejected

- **Architecture.** `AGENTS.md`'s local-first rule: no cloud calls, no remote
  content analysis, unless explicitly requested by the user for their own
  instance.
- **Legal — special-category personal data.** Text or images revealing sex
  life/sexual orientation are special-category personal data under GDPR
  Art. 9 (and Ukraine's equivalent). Sending them to a third party requires
  either the purely-personal/household exemption or explicit consent from
  *the data subject* — and a startup a week old has no established DPA,
  audited retention policy, or subprocessor terms.
- **Legal — correspondence.** Self-consent only covers content *you*
  authored. Text sent by another person (chat) makes that person a second,
  non-consenting data subject; forwarding their message to a third-party
  classifier is closer to *Ryneš* (C-212/13) — processing that extends beyond
  the purely private sphere loses the household exemption — and separately
  implicates confidentiality-of-correspondence law (in Ukraine, constitutional
  Art. 31), independent of data-protection law entirely.
- **ZDR does not fix the transfer, and isn't cheap anyway.** TypeSafe only
  offers zero data retention on an enterprise contract, not the self-serve
  tier — and even with ZDR, the data still leaves the device before being
  discarded; ZDR is a retention promise, not a transfer-avoidance one.
- **Jev cannot be self-hosted.** Closed weights, hosted-API-only
  (`POST /v1/systemone`), no on-prem offering or timeline.

## Why Laya and VisionLaya are deferred specifically

**Laya:** expected to sit below Jev's capability ceiling, and Jev is already
out for reasons orthogonal to capability (architecture/legal, above). There is
no case for researching the weaker option before the stronger one was ruled
out on unrelated grounds.

**VisionLaya:**

- No NSFW-specific benchmark exists for it at all — the only published number
  is 69.1% on a 59,427-question *general* multi-task visual-QA suite,
  unrelated to explicit-content judgment.
- It is an independent fork, unaffiliated with Laya's original authors,
  published days before this decision was written, with its own evaluation
  infrastructure ("held-out evaluation sets for calibration and abstention
  testing") still under active construction at the time.
- **A structural reason to expect failure, not just an absence of evidence of
  success:** general-purpose VLM pretraining corpora (SmolVLM's included, by
  its own published training-data description) are commonly filtered to
  exclude explicit content for the publisher's own safety/liability reasons.
  If the backbone never saw the target class during pretraining, zero-shot
  prompting cannot make it recognize that class — this is a reason to expect
  the model is bad at this task, not merely untested on it.
- **The cheap export path is known-degraded by the model's own authors.**
  VisionLaya's fp32 ONNX export is faithful to the PyTorch reference (max
  abs diff 4.0e-6), but the quantized exports that would actually be cheap
  enough to run live diverge sharply — fp16 3.8e-3, q8 1.7e-2, q4 0.23 — and
  the authors pulled their own in-browser demo because those smaller exports
  "misrepresented the model's performance." Any future work must treat a
  quantized VisionLaya as an unvalidated, different model from the fp32
  reference, not a cheap version of the same one.

## What was rejected

- Adopting VisionLaya as a production second opinion now.
- Building a shadow-mode live-traffic comparison harness for VisionLaya as
  current work. (The design — fp32-only, sampled rather than per-frame,
  local-only, single-developer dogfood, scores-and-agreement-only logging,
  never the image — remains valid as a future bounded experiment if
  VisionLaya's evidence base improves; it just isn't being built now.)
- Self-hosting Laya for text nuance as current work.
- Downloading the raw datasets behind published NSFW benchmarks (LSPD,
  NudeNet's training/eval set) to reproduce their numbers locally — holding
  that corpus recreates the CSAM-adjacent possession risk this project has
  already ruled out for any self-assembled dataset. Published *numbers* are
  cited; the *images* behind them are never fetched.

## Consequences

- Near-term image-quality work goes into NudeNet-style and
  illustration/anime-specific local ONNX classifiers as ensemble signals next
  to `packages/image-sandbox`'s tile classifier — not a VLM.
- A safe+sexy eval corpus is legal and buildable now; recall on true explicit
  content stays permanently "borrowed" from external publications rather than
  independently verified. This is a structural limit of this project's legal
  position on holding evaluation data, not a temporary gap expected to close.
- This decision is a snapshot of the evidence available in September 2026.
  If TypeSafe (or another vendor) ships a genuinely self-hostable or
  ZDR-by-default product, or an independently-benchmarked NSFW-aware open
  vision judge appears, it should be re-evaluated against this record rather
  than assumed still true.
