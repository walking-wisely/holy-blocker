# Decision: Image Corpus Custody

## Status

Decided. Constrains every corpus the image classification path may use, on any platform.

Companion to [domain-blocklist-sourcing.md](domain-blocklist-sourcing.md), which settles the same
class of question for *domain lists*. That document's rule — this project never builds, holds, or
infers a CSAM list, and never inspects content to produce one — is assumed here and not restated.
This document covers **imagery**: the corpora used to measure and calibrate the image classifier.

## What was decided

**No third-party imagery is ingested, at any scale, for any purpose.**

Not filtered, not screened, not sampled, not streamed-and-deleted. The corpora the image path is
measured against are built from three sources only:

1. **Generated** — `synth_ui.py`'s rendered UI, procedural textures, and 3D renders.
2. **The developer's own material** — their screen, their camera roll.
3. **Small, hand-annotated academic sets that are small enough to look at** — COCO `val2017`
   scale (5,000 images), used for photographic realism only.

The explicit-content *concept* is never acquired as data. It is acquired as **third-party
pretrained weights**, which are not the material.

## Why the usual mitigations were rejected

The obvious alternatives all reduce to trust, and each fails for a specific reason.

**Source reputation is not a control.** "Scrape reputable domains" has no failure bound and no way
to verify. It fails precisely where it cannot be observed — an ad slot, an embed, a
user-contributed image on an otherwise unremarkable site. Wikipedia is actively moderated and
still hosts explicit adult imagery in its sexuality articles; if the safest available pick fails
the property, the property was never being satisfied by the picking.

**A published "filtered" release proves less than it appears to.** WebUI's Hugging Face release
was filtered with an *explicit-words list* — a screen over page **text**, which cannot see the
images at all. It misses the dominant case at crawl scale: a page whose own words are
unremarkable while a third-party embed renders something else. Filtering lowers a base rate by an
unknown amount; it does not bound anything.

**Screening with our own classifier is circular.** It fails exactly where the model fails, which
is the drawn content and the small in-page thumbnails this project is trying to stop missing. A
corpus screened by a model is only as clean as that model's recall, and the recall is the thing
under investigation.

**"Transient, in-memory, deleted afterwards" does not help here.** The prior version of this rule
(recorded in [../components/machine-learning/plan.md](../components/machine-learning/plan.md))
permitted reading a corpus archive in memory and deleting it. That remains acceptable for a corpus
*of known provenance*, but it is not a mitigation for an uncurated one: the exposure sits in the
acquisition, not the storage duration.

## The evidence this rests on

This is not a hypothetical risk profile. In December 2023 the Stanford Internet Observatory found
**3,226 suspected and 1,008 externally validated instances of CSAM in LAION-5B**. LAION withdrew
the dataset and released Re-LAION-5B in August 2024 with 2,236 links removed after working with
IWF and the Canadian Centre for Child Protection. The report's lead author stated the consequence
directly: anyone who downloaded the full dataset has the material unless they took extraordinary
measures.

Three facts from that episode are load-bearing here:

- **LAION distributed URLs and embeddings, not images.** Whoever ran the downloader did the
  acquiring. The thing that protected the distributor is precisely what a crawler gives up.
- **The remediation required institutional access.** Hash-matching against IWF/NCMEC sets is
  licensed to qualifying providers, for the obvious reason that the hashes derive from the
  material. A solo developer cannot obtain it, and therefore **cannot perform the diligence that
  knowledge of the base rate implies.**
- **The knowledge defence is closed.** Contamination of web-scale image crawls is a published,
  cited property of the method. "I did not know" was arguably available before that report. It is
  not available now.

Those three together produce the actual reasoning: the exposure is not created by ingesting, it is
created by ingesting **after** you know, without the control the knowledge implies — and that
control is not obtainable at this scale of operation. There is no diligence step available that
reaches "I screened it." So the rule is not a risk appetite. It is the only posture with a
defensible answer.

**This is engineering reasoning, not legal advice, and no part of it has been reviewed by
counsel.** The jurisdictional analysis in
[domain-blocklist-sourcing.md](domain-blocklist-sourcing.md#jurisdictional-scope-and-what-it-does-not-cover)
applies here unchanged, including that the project owner is the sign-off owner for anything that
ships on it.

## What the rule permits, and why it is sufficient

The corpora exist to answer questions about **geometry**, not about explicitness.

What is actually missing from this project's measurements is *screen composition* — pictures
embedded in real UI, at real scales, with chrome and text around them. None of that requires
explicit imagery. Benign photos in realistic layouts supply every property that matters: scale
distribution, occlusion, thumbnail sizes, caption overlays, surrounding chrome statistics.

The explicit-content concept comes from a pretrained checkpoint, which is not retrained here.

### Scale is what makes this tractable

The measurements need a few thousand images, not millions. At that scale:

- **The corpus can be looked at.** 5,000 thumbnails is roughly 50 contact sheets. This converts
  trust into verification, and it is unavailable at any web scale — LAION-5B was never viewed by
  anyone, which is why finding what was in it took a research team with hash lists.
- Hand-annotated academic sets are *already* once-reviewed: COCO's images were each hand-segmented
  by multiple annotators as part of construction. That is a different process from a blind crawl,
  not a smaller number attached to the same one.

A one-time visual sanity pass over a small, already-annotated set is **not** the human review path
this document forbids below. The forbidden thing is an ongoing pipeline that surfaces *suspected*
material out of an uncurated source for adjudication.

### Photo sources, ranked

| Source | Acquisition risk | Use |
|---|---|---|
| Developer's own camera roll | none — known provenance | primary |
| 3D renders / procedural textures (Blender, Hypersim-style) | none — generated | photographic statistics without any scraping |
| `synth_ui.py` output | none — generated, committed generator | rendered-UI backgrounds |
| COCO `val2017` (5k, hand-segmented) | low — human-annotated in construction, reviewable at this size | photographic realism, spanning the score range |
| ImageNet | low, with a caveat — the *person* subtree had documented problems and was partly withdrawn; prefer standard object classes | optional |
| Moderated stock platforms (Unsplash, Pexels) | moderated, not per-image reviewed for this | avoid unless a gap is shown |
| Blind web crawls (LAION, DataComp, CommonPool) | **prohibited** | never |

**RICO and WebUI are prohibited as pixels** and permitted only as *layout statistics* used to
parameterise the generator. Both ship as flat screenshots with third-party imagery baked in, so
taking them is ingestion. Both are also research agreements with indemnity clauses that bind a
for-profit employer, not open licences, so nothing derived from them is redistributable.

### If real page layouts are ever needed

Only if a measurement shows the synthetic backgrounds are insufficient, and then by **capture, not
download**: drive a browser over a chosen list of pages and **inject
`Content-Security-Policy: img-src 'none'; media-src 'none'` on every response**, so the renderer
refuses to paint any image regardless of how it would have arrived — including `data:` URIs and
CSS `background-image`, which resource-type request blocking alone does not cover. Pictorial
regions are then filled with our own photos, and the DOM supplies exact ground-truth boxes for
free.

Under that design **the domain list does no safety work at all** — it only affects layout realism.
That is the point: the control is categorical (no image bytes are fetched or painted) and
verifiable by reading the capture code, rather than probabilistic and resting on a list.

Known residual: `<canvas>` painting and inline SVG shapes are not covered by an image CSP. Stated
as a limitation rather than solved.

## Standing prohibitions

- **No human review queue** over uncurated material. A queue that surfaces suspected content is a
  machine for manufacturing actual knowledge paired with an inability to act on it.
- **No CSAM detection.** This product blocks legal-but-unwanted adult content. Detection is done
  by perceptual-hash matching against provider-licensed databases; building a classifier for it
  would require acquiring the training data, i.e. committing the offence. Incidentally blocking
  such an image as "explicit" is the design working; making it a feature is not.
- **No model inversion / data-free distillation** (DeepInversion, DAFL, ZSKD) against an NSFW
  teacher. It formally satisfies "train without the dataset", and it satisfies it by *generating*
  imagery in the class being inverted, from a teacher whose own data provenance cannot be
  verified. Rejected outright.
- **No dataset distillation or condensation.** The optimisation runs against the full real dataset;
  possession is required to distil it.
- **No persisted derivative corpus.** Composites are rendered in memory; only derived numbers are
  written.

## Retention surface

Retention hides in places that are not called a database. Each of these is an invariant to hold,
and several are already held for unrelated reasons:

- `FrameCache` retains the last complete frame **in memory** by design. In-memory transience is
  fine; a **core dump or hibernation image** is not. Disable core dumps for the agent, and treat
  making frame buffers non-swappable as open work.
- **Logs record measurements, never content.** The daemon already logs AX text *length only, never
  content*, and `TamperLog` records the guard rather than the screen. The image path logs scores.
  Extend the same rule explicitly to pixels.
- **Debug affordances are the realistic accident** — any `--dump-frame` flag, `capture` verb
  variant, or test fixture that writes a frame to disk.
- `packages/mitm-proxy` handles image **bytes in transit** and must never buffer them to disk.
- On macOS, anything written under a path a backup daemon walks.

## Consequences

- **Local-first is a legal asset, not only a privacy one.** With no server and no upload, the
  project plausibly never becomes a provider carrying the reporting duties that attach to one. Any
  future telemetry, cloud-assist, or escalation path is therefore a legal decision before it is a
  product one.
- **The no-explicit-corpus rule is the firewall, and it is now stated as such** rather than left
  as an ethical preference. It keeps training-data acquisition — the highest-risk operation in any
  ML pipeline — permanently out of the project, and it is the strongest argument for the
  delegate-to-pretrained-weights architecture: **weights are not data.**
- **Unresolvable diligence limit, recorded rather than hidden:** the provenance of third-party NSFW
  checkpoints cannot be verified. Prefer checkpoints with documented adult-only sourcing, and
  accept that this does not close.
- **Some questions become permanently unmeasurable here**, and are enumerated in
  [classifier-operating-point.md](classifier-operating-point.md) and in
  [../components/image-sandbox/plan.md](../components/image-sandbox/plan.md#what-stays-unmeasurable):
  recall on drawn explicit content, the boundary of a "suggestive" tier, and any threshold's
  absolute correctness. Those are inherited from a checkpoint's own evaluation or left open — never
  papered over with a number this project cannot produce.

## Sources

- Stanford Internet Observatory, *Identifying and Eliminating CSAM in Generative ML Training Data
  and Models* — <https://purl.stanford.edu/kh752sm9123>
- LAION, *Releasing Re-LAION-5B* — <https://laion.ai/blog/relaion-5b/>
- Rico terms of use — <https://www.interactionmining.org/archive/rico>
- WebUI dataset and COPYRIGHT.txt — <https://github.com/js0nwu/webui>
