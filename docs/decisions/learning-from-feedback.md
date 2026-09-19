# Decision: Learning From User Feedback

**Status:** Design direction. Not yet implemented. This records where the
classification strategy is heading and why, so the reasoning survives outside a
chat log. Sections are marked **Decided**, **Open**, or **Speculative**.

This decision reframes parts of
[content-classification.md](../architecture/content-classification.md): the
deterministic text scorer described there is demoted from *decider* to, at most,
a cheap local gate. It builds on [accountability.md](accountability.md) (the
partner) and [content-interception.md](content-interception.md) (the on-screen
capture/ML path).

## Summary of the pivot (Decided)

1. **Text scoring is not the primary blocking mechanism.** A deterministic
   lexicon scorer over page text is context-blind: one flagged word in a long,
   clean article either over-blocks (tuned loose) or finds nothing (tuned
   tight), with no operating point in between. This is a modelling problem, not
   a tuning problem — the same failure email spam filtering abandoned decades
   ago when it moved from hand-tuned rule weights to learned weights.
2. **Domain blocklists do the real work.** The most widely deployed open-source
   adult list (UT1, behind squidGuard / e2guardian / pfSense) ships ~4.6M
   domains and **zero** text expressions. Set-membership on hostnames needs no
   corpus, no taxonomy, no thresholds, and has near-zero false positives. This
   is the floor and the near-term MVP.
3. **Personalization is local.** Per-user corrections live on-device (kNN over
   embeddings + an override table), never centralized.
4. **The goal that justifies a learned model is false-positive reduction** — not
   blocking a person out of genuinely okay content. That is the thing worth
   improving, and it is uniquely measurable without any explicit corpus (see
   Evaluation).

## The core problem (Decided framing)

We want to improve the filter from user feedback — specifically user-flagged
**false positives** (e.g. the on-screen scanner blurring a clean image inside a
mixed-content app like Telegram, where the domain tells us nothing). But a
user-flagged "false positive" is ambiguous:

- **Genuine FP** — the scanner blocked clean content (swimwear, medical, art).
  Learning from this is the goal.
- **Temptation-unblock** — the user, in a weak moment, flags genuinely explicit
  content as "not a violation" to unblock it. Learning from this poisons the
  model toward under-blocking.

The user is their own adversary (see [accountability.md](accountability.md)), so
the labeler is **compromised in one specific direction**: temptation biases
mislabels toward *allow / block-less*, essentially never toward *block-more*.

## What the research says works

Findings below are from a literature review (see References). Directional and
load-bearing conclusions first.

### Directional trust (Decided)

Temptation cannot produce "you should have blocked this" flags. So:

- **The recall / "block-more" side is sourced entirely from labeler-independent
  data** — a theoretical framing, not a claim that such data exists here: per
  the 2026-09-19 revision under Evaluation below, this project holds no
  developer-curated explicit corpus and no public NSFW benchmark, so this
  channel is currently empty rather than merely uncorruptible. The framing
  still matters for *whatever* labeler-independent recall signal is eventually
  used (see the revised Evaluation section) — none of it is user-supplied, so
  none of it is poisonable. This direction is **structurally clean by
  construction**, whatever fills it.
- **The precision / "allow" side is treated as positive-unlabeled**: a user
  "not a violation" flag is *unlabeled evidence that shifts a prior*, never a
  trusted "this is clean" label. This maps to the one-sided-label-noise / PU
  learning regime.

This is the same asymmetry throughout the design: the direction users *can*
corrupt is bounded and dampened; the direction they *cannot* corrupt carries the
safety-critical guarantee.

### The honest limit (Decided)

The corruptible ("allow") channel **cannot be cleaned from the biased labeler
alone.** The theory that would let you recover a one-sidedly-corrupted signal
(Menon et al. 2015) assumes *class-conditional* noise — independent of the
content. Temptation noise is almost certainly *instance-dependent* (worst on the
most borderline/tempting content), which breaks that guarantee. So on the allow
side we can only **bound and dampen**, and hand the residual to a human. This is
a limit to design around, not a bug to fix.

### Robust aggregation is the wrong tool (Decided)

Byzantine-robust federated aggregation (Krum, trimmed-mean, median) assumes a
malicious *minority* pushing in *arbitrary* directions. Our adversary is a
near-*universal majority* biased the *same* way — precisely the regime where
those methods provably fail (cf. the "A Little Is Enough" attack). Keep robust
aggregation only for ordinary outliers / the occasional truly-malicious client;
it is **not** the defense against temptation.

### Discriminating genuine FP from temptation (Decided mechanism, Open thresholds)

Use the model's own confidence on the flagged frame, via **learning-to-defer /
selective prediction** (predictor + rejector):

- Model was **unsure** it was explicit **+** user flags "not a violation" →
  likely genuine FP → **auto-accept**.
- Model was **confident** it was explicit **+** user disagrees → likely
  temptation → **defer to the human** (accountability partner).

This routes only the ambiguous minority to a human. Its quality is a measurable
**risk–coverage curve**, so "how much human review can we remove while holding
quality" has an empirical answer, not a guess.

### Incentive-compatible friction (Speculative, most promising unexploited lever)

Make the temptation-unblock *costly or delayed* so honest reporting dominates: an
"allow" flag only loosens blocking after partner co-sign, a cool-down, or
independent confirmation. This is the [protection-modes.md](protection-modes.md)
override gate and the [accountability.md](accountability.md) partner, applied to
the feedback channel. Tightening flags stay instant/self-service; loosening
flags are gated.

## Privacy (Decided constraints, Open engineering)

- **Raw explicit content never leaves the device, and is never in developer
  custody.** For this content type that is not only a privacy stance but a legal
  firewall (CSAM custody / mandatory-reporting exposure). **Any legal question
  here needs qualified counsel; this doc does not opine on law.**
- **Federated fine-tuning of a small head on a frozen backbone** is the proven
  pattern (Gboard). But "raw content never leaves" requires **Secure Aggregation
  *and* Differential Privacy** — SecAgg alone is breakable (recent attacks
  recover labels/inputs from the aggregate; even "this device had explicit
  content" leaking is a harm). DP has a real accuracy cost, worse for a small
  head with sparse signal; budget a permissive-but-defensible ε and many devices
  per round.
- **TEE / confidential computing** is viable for the recall/explicit side (which
  is developer-sourced anyway), but side-channels are a live, recurring threat —
  treat attestation as one trust input, not a guarantee.
- The clean / false-positive feedback, being clean content, **can** be handled
  and even centralized far more freely than the explicit side. The two channels
  should be structurally separated by content type.

### A personal/jurisdictional constraint tightens custody further (Decided 2026-09-19)

The maintainer's jurisdiction (Ukraine) treats holding or buffering NSFW-adjacent
image data as illegal even transiently, including for the purpose of running it
through a CSAM-detection/hash-matching screen (Thorn Safer, PhotoDNA) before use.
This removes an option some teams reach for — "screen the candidate corpus first,
then hold whatever passes" — because the screening step itself already requires a
custody window this jurisdiction does not permit. More generally, legally holding
such material for detection purposes requires registered-ESP-type status (e.g. 18
U.S.C. §2258A) this project does not have, so there is no favorable jurisdiction to
route around it by relocating infrastructure.

**Consequence: no self-assembled or hash-screened corpus of explicit imagery is
achievable in-house, ever — not on disk, not gitignored, not as cached feature
vectors, not transiently in memory, not for screening purposes.** This is stricter
than "never in developer custody" above: it forecloses even a bounded custody
window, which the wording above did not previously rule out on its own. As with
every legal statement in this document, this is not legal advice — qualified
counsel should confirm it before anything here is relied on.

Two corollaries worth stating explicitly, because both look like workarounds and
neither is one:

- **Perceptual hashes and embeddings are not a lower-risk substitute for raw
  images.** Both are similarity-preserving by design — that is their entire
  purpose, matching near-duplicates — which is the opposite of
  privacy-preserving, and both are documented as invertible to a meaningful
  degree: the 2021 community reversal of Apple's NeuralHash, and the
  model-inversion-attack literature against face/image embeddings generally. An
  encoding useful enough to train or match on is discriminative enough to
  reconstruct from; there is no encoding of explicit imagery that is
  simultaneously useful here and safely non-identifiable. This is the same
  utility–privacy tradeoff the Secure Aggregation / DP bullet above already
  names for gradients, not a new one.
- **Centralizing anything — full images, hashes, embeddings, or an individual
  (non-aggregated) gradient — to project-run infrastructure is a regression,
  not a mitigation.** It does not remove the custody question; it moves it from
  one person's own device (bounded exposure, one person's own traffic) to a
  server aggregating many devices' screens, at higher expected volume of
  anything illegal and a plausible step toward the same ESP-like
  mandatory-reporting obligations this project does not currently carry. If a
  federated design is ever built here, the flat parameter vector must be the
  only thing that ever crosses the device boundary, and only after Secure
  Aggregation and DP as already specified above — a bare individual gradient is
  subject to the same reconstruction-attack literature ("Deep Leakage from
  Gradients", Zhu et al., NeurIPS 2019) that motivates SecAgg+DP in the first
  place.

## On-device runtime and where the head lives (Decided 2026-07-18)

`docs/components/machine-learning/plan.md` cross-references this section for the
runtime findings behind its export contract; they are recorded here.

**The decision: the frozen backbone runs on the platform's inference runtime, and the
head — forward, and later backward — is hand-written in Rust behind UniFFI, shared by
Android and Windows.** Not a fallback. Every framework on-device-training path was
evaluated and each is inference-only, experimental and stagnant, or effectively
abandoned:

- **LiteRT / TFLite** — inference-only by positioning. On-device training requires a
  **TensorFlow SavedModel** exposing `train`/`infer`/`save`/`restore` `tf.function`
  signatures, driven through the legacy `Interpreter.runSignature`. A `litert-torch`
  model cannot produce those: the converter requires a `torch.export`-compliant module
  in `.eval()`, and its multi-signature API exposes several *inference* entry points
  over an immutable graph. Getting training signatures would mean authoring the head in
  TensorFlow — a second modelling stack, plus the API LiteRT 2.x is steering away from.
- **ExecuTorch** — the only torch-native training path, and stalled. The extension
  exists (`_export_forward_backward` → joint fwd/bwd `.pte`, C++ `TrainingModule`, SGD)
  but its own README still calls it experimental at v1.3, with no weight serialization
  and no delegate support; every 2026 commit under `extension/training` is janitorial.
  **There is no Android training binding at all** — the AAR is `Module`/`Tensor`/
  `EValue`, inference only.
- **ONNX Runtime** — the disappointing one, because ORT-everywhere would have paired
  neatly with the Windows path. `onnxruntime-training-android` last published **1.19.2
  on 2024-09-03** and never again, while the inference AAR tracks mainline monthly;
  `onnxruntime-training-examples` was archived by its owner on 2026-05-20. Read
  "unmaintained on mobile" as verified fact and "officially deprecated" as inference.
- **MediaPipe Model Maker** — no longer actively maintained, and server-side anyway.

**Why the hand-written head is the right call rather than a workaround.** For
`logits = W·e + b` with softmax cross-entropy, the backward pass is
`dlogits = softmax(logits) − onehot(y)`, `dW = dlogits ⊗ e`, `db = dlogits` — on the
order of 100–200 lines of Rust with an optimizer, textbook-derivable, and testable
against PyTorch `autograd` on fixed fixtures plus a finite-difference check. It is also
the only option with a real Android surface; it reuses the `text-policy` /
`text-policy-ffi` pattern this repo already runs on an emulator; it decouples training
from the export format entirely; and Secure Aggregation needs the flat parameter vector
it produces anyway, which ExecuTorch cannot even serialize. If the head ever outgrows
hand-written gradients, **Burn** is the in-language upgrade — not a day-one dependency.

**Consequences that bind other components:**

- The export contract is backbone-only, terminating at the embedding — see
  `machine-learning/plan.md`. The embedding dtype and scale are a versioned interface:
  a silently re-exported int8 backbone invalidates every head in the field.
- **The Android and Windows runtimes differ on purpose.** LiteRT on Android, ONNX
  Runtime on Windows, one head crate over both. `packages/image-sandbox` is the ONNX
  side of that split and is not the mobile path.
- Federated learning is **our own protocol over that parameter vector**, not a
  framework adoption. TensorFlow Federated is effectively dead (v0.88.0, 2024-09-26,
  simulation-only, no Android client); Flower's Android example is a 2022 demo resting
  on exactly the TFLite training signatures a PyTorch pipeline cannot emit; Google's
  `federated-compute` is production-proven but its server side is not open source. Any
  such path stays opt-in and off by default, per the local-first rule in AGENTS.md.

## Evaluation (Decided strategy)

The asymmetry works in our favor for the corruptibility question: **the
quantity users can corrupt (over-blocking) is exactly the quantity we can
measure safely and abundantly on a benign corpus, with zero exposure to
explicit material.** The quantity they cannot corrupt (recall) was originally
scoped as "the one needing guarded, independent test data" — **that scoping is
superseded by the 2026-09-19 revision below: this project holds no such test
data, guarded or otherwise, so recall is not measured against held data at
all**, only estimated (CBPE) or gated indirectly (the specificity-suite release
gate). The asymmetry in corruptibility still holds; the asymmetry in "which
side has a held dataset" no longer does.

- **False-positive rate** — measured on a curated benign corpus (medical,
  sex-ed, biology, news, ordinary browsing; plus, on-device and private, the
  user's own real clean traffic). Every item is clean by construction, safe to
  view and to publish. This is the first buildable harness.
- **Recall — revised 2026-09-19, tightened from the original design.** This
  bullet originally proposed measuring recall against held-out independent
  public NSFW benchmarks, kept private and out of the repo. That still assumes
  somewhere legal to hold such a benchmark; per the jurisdictional constraint
  above, this project's maintainer has none, so **no recall benchmark of any
  kind is held in-house, private or otherwise.** What remains achievable
  without holding any explicit imagery:
  - **Confidence-Based Performance Estimation (CBPE)**, computed entirely from
    the deployed model's own score stream — no images, no labels — under the
    assumption that the model stays calibrated. This is complementary to, not
    a replacement for, the SSME/AutoEval label-free methods already cited
    below; it is the more directly applicable one here because it needs only a
    score stream this project already produces. Its ceiling is exactly the
    assumption it cannot check: a silent calibration drift is invisible to
    CBPE because there is no real-positive stream to check it against.
  - **Third-party, vendor-published benchmarks** (Hive, Sightengine
    self-report ~95–98% accuracy) as an external reference point only —
    self-reported, unaudited, and measured on their own distribution rather
    than this project's screen-capture path, so never a substitute for an
    in-house number.
  - **This exact gap is field-wide, not particular to this project**:
    CSAM-hash-algorithm researchers face the identical custody restriction and
    validate only against benign data (see the analysis of PhotoDNA's own
    evaluation practice, ACM 2023, in References).
  - See "Release gating without a recall benchmark" immediately below for how
    a release is still gated without an absolute recall number.
- **Gate every update on a recall guardrail** remains the intent, but with no
  held recall benchmark to regress against, the guardrail is now the release
  gate below rather than a benchmark comparison; FPR improvement on the benign
  corpus stays the secondary, WARNING-only metric. Staged canary rollout.
- For federated data the developer cannot inspect, use **label-free evaluation**
  (semi-supervised model evaluation, AutoEval, approximate-ground-truth bounds,
  CBPE above); devices DP-aggregate scalar metrics, never content.

### Release gating without a recall benchmark (Decided methodology, 2026-09-19)

Given no recall benchmark is held, a release still needs a promote/hold
decision. The gate is built entirely from what is legally holdable and what
normal operation already produces:

1. **A cross-style specificity regression suite** — legally clean imagery
   chosen to probe known failure modes, run old-vs-candidate on every release:
   - classical/fine-art nudity (public domain — a well-documented
     false-positive class for NSFW classifiers generally, not specific to this
     project)
   - licensed swimwear/lingerie stock photography (probes the boundary a
     future "suggestive" tier would sit at)
   - safe-for-work anime/cartoon imagery — checks that the style-domain
     false-positive gap [classifier-operating-point.md](classifier-operating-point.md)
     already documents (over-blocking "concentrated in illustrated artwork")
     does not get worse, without claiming to measure whether explicit-anime
     recall improves
   - breastfeeding/medical imagery (another documented real-world
     false-positive class)
   - the bed-photo false positive and anime false negative from operator
     dogfooding of `Falconsai/nsfw_image_detection` on the macOS daemon
     (2026-08-07) — an observation from live use, not yet written up as its
     own repo doc; worth turning into one so a future regression suite has a
     citable source instead of this parenthetical
   A candidate that blocks meaningfully more of this suite than the current
   model is a real regression signal, obtained without holding anything
   illegal to hold.
2. **Shadow-mode comparison, dogfood-only or a small consenting opt-in beta —
   never shipped as permanent dual-inference to the general fleet.** The
   current model acts; the candidate scores the same live input in parallel;
   only score-deltas and a disagreement rate are logged, never images. This
   runs only in a bounded pre-release window on the maintainer's own device(s)
   (or a small opted-in cohort later), sampled (1-in-N frames, not every
   frame) rather than at full rate — the always-on Android AccessibilityService
   path in particular cannot absorb permanent double inference. Promote only
   when disagreement is low and the specificity suite above is clean.
3. There is still no absolute recall number produced by this gate, by design
   — see the revised Recall bullet above. This is a trend/regression check,
   not an accuracy certification, and should not be described as one.

## Open problems (be clear-eyed with contributors)

- **No off-the-shelf method solves our exact case** — "the data subject is their
  own adversary, corrupting labels in one direction, as a majority." It sits in
  the gap between Byzantine-robust FL (assumes minority), strategic
  classification (assumes test-time feature-gaming), and asymmetric-noise theory
  (assumes class-conditional noise). The resolution is therefore
  **architectural, not algorithmic**: deny the corruptible channel any path to
  under-blocking; accept a bounded, human-controlled residual on over-blocking.
- **Instance-dependent noise** on the allow side has only weak guarantees.
- **Corpus representativeness** — a public benign corpus is not representative of
  a given user's browsing; the on-device real-traffic corpus closes that gap
  per-user but stays local.
- **Domain-blocklist coverage (`packages/net-shield`) changes what this
  classifier actually has to catch, and by extension how slowly any live
  signal converges (noted 2026-09-19).** Once traditional NSFW domains are
  blocked, the residual distribution reaching the image classifier is narrower
  and rarer — messenger-shared images, non-blocklisted/non-traditional
  sources, suggestive anime on platforms typical blocklists miss. Passive,
  organic-usage signal (CBPE trend lines, override-based calibration,
  shadow-mode disagreement) converges slowly by construction, in proportion to
  how effectively the domain layer already did its job — a structural property
  of a layered filter, not a flaw in the evaluation methodology, and no amount
  of tooling makes a rare event observed sooner. Two mitigations, neither of
  which changes the underlying rate: (a) deliberately dogfood the specific
  residual channels (messaging apps, fan-art platforms) instead of waiting for
  organic incidence; (b) separate **coverage** ("does the scan pipeline even
  trigger for this app/surface") from **accuracy** ("does the model score it
  correctly") — coverage is fast, fully legal to test with arbitrary imagery,
  and testable immediately, and is plausibly now the higher-leverage
  investment given how much volume the domain layer already absorbs.

## Proposed experiments (Open — all on public/benign proxy data only)

No private corpus required; nothing explicit stored in the repo.

1. **Temptation-poison simulation & discrimination accuracy (the crux).** On a
   public NSFW set + a benign corpus, train frozen-backbone + head. Simulate
   temptation via *instance-dependent, one-sided* label flips (explicit →
   "not a violation", never the reverse, with probability rising in model
   confidence). Metric: AUC of the selective-prediction rejector at separating
   genuine-FP from temptation flips; the risk–coverage curve.
2. **One-directional aggregation ablation.** Compare (A) trust-all-flags
   (expect recall collapse), (B) robust-aggregation only (expect partial
   failure under high poison), (C) this design (PU allow-side + independent
   recall + guardrail). Show whether (C) reduces over-blocking without recall
   regression across poison rates 0–60%.
3. **DP utility-cost curve.** Sweep ε for the head-only federated fine-tune;
   find the ε where FPR gains vanish. Sets the privacy budget.
4. **Human-budget / deferral curve.** Sweep the deferral threshold: "to hold
   recall constant, X% of flags must go to the partner."

Recommended starting point: **Experiment 1** — smallest, public data only,
directly answers "can we tell the two kinds of flag apart, and how often do we
then need a human."

## References

Established frameworks to adopt by name:

- **PU / one-sided-noise:** Menon et al., *Learning from Corrupted Binary Labels
  via Class-Probability Estimation*, ICML 2015; Scott et al., *Classification
  with Asymmetric Label Noise*, COLT 2013.
- **Learning-to-defer / selective prediction:** Cortes, DeSalvo, Mohri,
  *Learning with Rejection*, ALT 2016.
- **Private federated fine-tuning:** DP-FTRL + Secure Aggregation (Gboard;
  arXiv:2306.14793). Gradient-leakage / SecAgg limits: arXiv:2406.15731,
  arXiv:2311.05808.
- **Robust-aggregation limits ("A Little Is Enough"):** see Byzantine-FL
  literature; do not rely on it for this threat.
- **Label-free evaluation:** SSME (arXiv:2501.11866); AutoEval
  (arXiv:2007.02915); Approximate Ground Truth Refinement; Confidence-Based
  Performance Estimation (CBPE) — NannyML,
  nannyml.com/blog/model-monitoring-without-ground-truth; label-free
  confusion-matrix estimation under distribution shift, arXiv:2507.22776 and
  arXiv:2505.05295; stratified-sampling accuracy estimation for content
  moderation under the EU DSA, arXiv:2305.09601.
- **Public NSFW evaluation references:** NudeNet; Pornography-2k; I2P. (Named
  here as the class of thing that exists in the literature; per the
  2026-09-19 revision above, this project does not itself hold any of them.)
- **CSAM-detection evaluation under custody restrictions:** analysis of
  PhotoDNA's own evaluation practice, ACM 2023,
  dl.acm.org/doi/fullHtml/10.1145/3600160.3605048.
- **Perceptual-hash / embedding invertibility:** the 2021 community reversal
  of Apple's NeuralHash; model-inversion-attack literature against face/image
  embeddings generally.
- **Vendor self-reported NSFW benchmarks (reference only, unaudited):** Hive,
  thehive.ai/blog/best-in-class-hive-model-benchmarks; Sightengine.
- **Incentive-compatible ML:** strategic-classification line (Hardt et al.,
  ITCS 2016) and incentive-aware ML surveys.

Confidence, per the review: high on what fits / doesn't fit, on the FL/privacy
mechanics, on the deferral methods, and on the evaluation strategy; medium on how
much the corruptible channel can be dampened; the "genuinely unsolved" claim is
from targeted search, not exhaustive proof. All legal/possession questions are
deferred to counsel.
