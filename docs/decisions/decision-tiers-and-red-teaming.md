# ADR: Decision tiers and red-teaming

| Field | Value |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-10-01 |
| **Owner** | Ivan Dutov |
| **Supersedes** | — |
| **Superseded by** | — |

---

## Context

The owner wants to be present where only a human can decide, and absent everywhere else.
The pipeline until now escalated *doctrinal* claims (faith, product, legal) and left every
engineering claim to the agent, verified by a falsifying command. That leaves two gaps:

1. **No trigger for architecture.** Choices such as the LiteRT/ONNX split, a cross-platform
   FFI contract, or an ML operating point are engineering claims an agent can verify with a
   command and still get wrong as *decisions*. Nothing routes them to the owner.
2. **Nobody attacks a decision before it is recorded.** `adversarial-review` attacks code.
   `plan-inception` traces a plan's claims to citations but never asks how the decision is
   bypassed, abused, or fails silently.

## Decision

### 1. Three decision tiers

| Tier | What it is | Who decides |
|---|---|---|
| **Product** | What the product does and refuses to do: protection modes, the CSAM/legal boundary, verse and accountability policy, privacy trade-offs, what a feature is *for* | The owner, always, after red-teaming |
| **Architecture** | A cross-cutting or hard-to-reverse structural choice: a cross-platform contract (the FFI crate APIs), the ML approach and operating point, a new trust boundary, the system's decomposition | The owner when it meets any trigger below; otherwise the agent, recorded |
| **Implementation** | Platform plumbing, data structures, test design, SAST/DST wiring, anything reversible inside one package | The agent. Never escalated for its own sake |

An architecture decision reaches the owner when it meets **any** of: it is an ML decision;
it changes a contract two or more platforms consume; it adds or moves a trust boundary; it
is a one-way door (cannot be undone by a later PR). Everything else in the tier is decided by
the agent and written down in the plan.

The owner does not review code. The owner reviews decisions and feels the finished feature
(see [feature-demos-and-local-e2e.md](feature-demos-and-local-e2e.md)).

### 2. Red-team tiering

A decision is red-teamed **before** it is written back as an ADR. The depth scales with the
tier and the cost of being wrong:

| Red-team tier | Applies to | What runs |
|---|---|---|
| **0** | Implementation decisions; reversible product tweaks inside an existing ADR | Nothing |
| **1** | Architecture decisions that meet no escalation trigger, and product decisions that extend an existing ADR | One fresh-context subagent on the single most relevant lens |
| **2** | A new product decision; any architecture decision that meets a trigger; any one-way door; anything touching the tamper model, protection modes, the legal boundary, or an ML operating point | Three fresh-context subagents in parallel, one per lens |

The three lenses:

- **Bypass** — a motivated user, or someone with their device, defeating it: the cheapest
  route that is both easy and silent, what the decision does not cover, how it fails open.
- **Intent** — whether it serves the mission in `docs/mission.md`: the faith/covenant
  rationale, what it does to the person it protects, the false-positive and false-negative
  cost on the axis in `classifier-operating-point.md`.
- **Legal and privacy** — the boundaries in `domain-blocklist-sourcing.md` and
  `image-corpus-custody.md`, local-first and what data the decision creates or moves.

A tier-1 run picks the lens from the domain tags on the plan's claims. A claim tagged
`faith` draws Intent; `legal` draws Legal and privacy; a trust-boundary change draws Bypass.

Each lens returns findings under the rules `adversarial-review` already enforces —
**reproduce or label, cite or omit** — and the findings are folded into the ADR's rejected-
alternatives and "what this does not cover" sections, so the record shows what was attacked
and not only what was chosen. Findings are handed to the owner as a short list; the
escalation packet format in [`docs/engineering/review-triage.md`](../engineering/review-triage.md)
§4 applies verbatim.

### 3. Spawning authorisation

`confirm-before-subagents` makes an agent ask before spawning. Tier-2 red-teaming spawns
three agents by design. The `plan-inception` skill states that tier-2 and tier-1 red-team
spawns are **pre-authorised by this ADR**, so the skill does not stop to ask on each one.
Nothing else gains that authorisation.

## Rejected

- **Red-team every decision at tier 2.** Most decisions are reversible implementation
  detail; three agents each time would train the owner to skim the output.
- **Let the agent classify tier silently.** The classification is written in the plan next
  to each claim, where the owner can overrule it.
- **Escalate every architecture decision.** That reinstates the babysitting this ADR exists
  to remove.

## What this does not cover

- Red-teaming *code*. That remains `adversarial-review`, now followed by the unconditional
  security and privacy gates in `docs/engineering/plan.md` step 8.
- A mechanism that forces `plan-inception` to be entered for new work. That remains a
  convention in `AGENTS.md`.
