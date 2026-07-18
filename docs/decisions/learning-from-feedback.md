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
  data** (developer-curated explicit corpora, public NSFW benchmarks). Users
  have no path into it, so there is nothing to poison. This direction is
  **structurally clean by construction.**
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

## Evaluation (Decided strategy)

The asymmetry works in our favor: **the quantity users can corrupt
(over-blocking) is exactly the quantity we can measure safely and abundantly on
a benign corpus, with zero exposure to explicit material; the quantity they
cannot corrupt (recall) is the one needing guarded, independent test data.**

- **False-positive rate** — measured on a curated benign corpus (medical,
  sex-ed, biology, news, ordinary browsing; plus, on-device and private, the
  user's own real clean traffic). Every item is clean by construction, safe to
  view and to publish. This is the first buildable harness.
- **Recall** — measured only on held-out independent public NSFW benchmarks,
  kept out of the training/feedback loop and out of the public repo.
- **Gate every update on a recall guardrail**: recall on the held-out benchmark
  must not regress (auto-rollback = CRITICAL); FPR improvement on the benign
  corpus is the secondary metric (WARNING if it fails to improve). Staged canary
  rollout.
- For federated data the developer cannot inspect, use **label-free evaluation**
  (semi-supervised model evaluation, AutoEval, approximate-ground-truth bounds);
  devices DP-aggregate scalar metrics, never content.

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
  (arXiv:2007.02915); Approximate Ground Truth Refinement.
- **Public NSFW evaluation references:** NudeNet; Pornography-2k; I2P.
- **Incentive-compatible ML:** strategic-classification line (Hardt et al.,
  ITCS 2016) and incentive-aware ML surveys.

Confidence, per the review: high on what fits / doesn't fit, on the FL/privacy
mechanics, on the deferral methods, and on the evaluation strategy; medium on how
much the corruptible channel can be dampened; the "genuinely unsolved" claim is
from targeted search, not exhaustive proof. All legal/possession questions are
deferred to counsel.
