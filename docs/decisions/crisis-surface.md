# Decision: Crisis Surface — Deferred

**Status: open. Almost nothing here is decided.**

This file exists because the questions below were identified and deliberately postponed,
not because they were answered. Writing down what we do not know is the point of the
document. If you are picking this up, assume none of it is settled and that the reasoning
in [formation-model.md](formation-model.md) is the only thing constraining you.

## What *is* decided

Two things, both of them limits rather than designs:

1. **Nothing beyond the one-tap self-report ships until this document is resolved.** The
   tap (*this was a stumble* / *this was fine*) is safe and may proceed. Free-text
   journaling, generated prompts, and any expansion of the reflection surface are gated on
   the questions below. See "The reflection surface is deliberately small" in
   [formation-model.md](formation-model.md).
2. **The interim rules hold unconditionally**, whatever gets decided later: prompts are
   pre-written and closed; nothing is generated; no user data leaves the device; the tool
   renders no verdict about a person.

Everything else is open.

---

## Why this is deferred rather than solved

The intended user is a person who believes they are failing God, in secret, repeatedly.
That is an elevated-risk population, and a tool that invites them to reflect on their
failures is operating in a place where getting it wrong has consequences no amount of
careful architecture can undo.

This is the point where engineering judgment runs out. The remaining questions are
clinical and pastoral, and they need someone qualified — not a design document, and not a
coding agent. Deferring is the correct move. Guessing would not be.

---

## Open questions

### The crisis surface itself

- Should the tool attempt to detect crisis language at all? `text-policy` is technically
  shaped for it (lexicon + scorer), but capability is not permission.
- What is the cost of a false positive here? Surfacing a hotline to someone who is fine is
  not free — it is a startling thing to be shown, and it may read as an accusation of
  instability.
- What is the cost of a false negative? Almost certainly higher. But the tool is not a
  safety net and must not be presented as one, or people will rely on it as one.
- Which resources, for which regions? `988` covers the US. Everything else needs actual
  research, and the answer must ship in-bundle (per the local-first promise), which means
  it goes stale and has no update path short of a release.
- Is a crisis match logged? If yes, it is the most sensitive record in the system and
  contradicts the release rule. If no, there is no way to calibrate it — ever.
- Does the tool say anything at all beyond showing the resource? Almost certainly not, but
  "almost certainly" is not a decision.

### Partner mode

**The whole of partner mode is reopened, including whether it should exist.**
[accountability.md](accountability.md) currently specifies it as a shipping feature. That
document has not been retracted, but it should be treated as unsettled until this one is
resolved — do not build against it without checking here first.

- Should partner mode exist at all?
- If a partner exists, are they told about a crisis signal? "He tried to disable the
  blocker" and "he may be in danger" are categorically different disclosures, and the
  consent that covers the first does not cover the second.
- Can a person meaningfully precommit to that disclosure in a calm moment? Precommitment is
  the mechanism this project trusts everywhere else. It is not obvious it transfers here.
- What happens when the partner is the wrong person — a spouse, a parent, someone whose
  reaction makes things worse? The tool cannot know this and cannot ask.
- The counts-only weekly digest already conflicts with "show shape, never sum" (flagged as
  an open tension in [formation-model.md](formation-model.md)). Resolve both together or
  neither.

### The journal

- Should free-text entry exist at all? It is the highest-risk surface and the least certain
  feature. The current answer is no, by deferral.
- If it ever ships, what supervises it? Nothing local can, and nothing remote is permitted.
  That may simply be the end of the idea.
- How would release-on-confession interact with an entry that contains a crisis signal?
  Deleting it is right by the release rule and may be wrong by every other standard.

---

## Who should answer these

Not this project's authors alone, and not an agent:

- A pastor or counselor who **actually works with this population** — compulsive use,
  sexual shame, religious scrupulosity. Their read on the reflection prompts is worth more
  than every rule in `formation-model.md`.
- **Safe messaging guidelines** for suicide and self-harm are an established body of
  clinical guidance from suicide-prevention organizations, with specific direction on
  wording, framing, and what not to show. Find the current ones and follow them rather
  than reasoning from first principles.
- **Prior art.** Other products in this space have had to solve this and their answers are
  publicly visible. Look before inventing.

---

## What must not happen in the meantime

- Do not ship free-text reflection.
- Do not add a language model that speaks to the person.
- Do not treat the absence of a crisis feature as a crisis feature. If someone asks whether
  the tool helps in a crisis, the honest answer today is **no**, and the tool should not
  imply otherwise anywhere in its copy.
