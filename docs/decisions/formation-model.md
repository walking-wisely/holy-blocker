# Decision: Formation Model

The rationale for this document lives in [mission.md](../mission.md) under "What the
blocking is for." This file is the design consequence: what the tool is permitted to
judge, what only the person may judge, what may be shown, and what must be forgotten.

*(Scripture quotations are NIV. For licensing notes on embedding verse text in the
distributed application, see [verse-pools.md](verse-pools.md).)*

## What was decided

The tool notices, pauses, and asks. The person judges. These are separate jobs and they
are assigned to separate parties:

| Question | Who answers it |
|---|---|
| What is in this content? | the model — as a calibrated probability, never a label |
| Should it be covered? | the user's mode + threshold, set in advance |
| Was this a stumble? | the person, and only if they choose to say |
| Was this sin? | the person before God. Never the tool, in any mode, at any confidence |

Every rule below follows from that table. All formation features are **optional in the
product** — a person can disable every prompt and run this as a pure blocker — and
**non-optional in the design**. The tool must never be built in a way that accuses, even
if a user asks for it.

---

## Why the model never renders a verdict

The same image is a stumble for one person and nothing for another. That variance is not
noise to be trained away with more data — it is irreducible, because the meaning of the
content depends on the person seeing it. There is no fact in the pixels for the model to
be right about.

This is not a limitation to be engineered around. It is the structure of the problem, and
it means a classifier confident to four decimal places is confident about the wrong
question. It can tell you what is on the screen. It cannot tell you what happened in the
person.

So the model's output is a **calibrated probability about content**, and it stops there.
It never emits "you sinned," never emits "this was lust," and never phrases a prompt as a
finding about the person.

---

## Two thresholds, not one

Blocking and inviting have different costs when they are wrong, so they get different
operating points on the same score.

**Blocking — tune for recall.** A false positive costs a covered painting. A false
negative costs the thing the tool exists to prevent. Over-block freely; this is where the
mission's "a tool that almost blocks is not the same as one that blocks" applies.

**Inviting — tune for precision.** A false invitation is not a harmless extra nudge. It
costs three things:

1. **It manufactures the false negatives it was meant to prevent.** The tool's credibility
   is a depleting resource. Prompt wrongly often enough and the real prompt gets dismissed
   with the same reflex as the false one. This is not a tradeoff between error types — over-
   prompting *produces* the miss, just later and somewhere you are not measuring.
2. **It teaches a false picture of God.** Not by argument, but by repetition, which is how
   formation actually works. A thing that watches you and finds you guilty daily trains a
   pre-cognitive expectation of the eye on you. That job is taken, and it is not ours.
3. **It collapses temptation into sin.** Being shown a thing is not consenting to it.
   Christ was tempted and was sinless. The distinction between a thought arriving and a
   thought being entertained is the one spiritual direction spends years establishing, and
   a prompt that fires on mere exposure erases it — teaching bad moral theology every time
   it is right about the pixels and wrong about the person.

> "Therefore, there is now no condemnation for those who are in Christ Jesus."
> — Romans 8:1 (NIV)

**Requirement:** because a user-set threshold is meaningless on uncalibrated scores, model
outputs must be calibrated (temperature scaling or Platt on a held-out set) before any
threshold is exposed. Otherwise the dial is theater and "strict" means only "noisier."

---

## The strictness dial controls sensitivity, never certainty

A user choosing their own level is precommitment working as intended — the same mechanism
as the covenant itself. Job's covenant was self-imposed; so is this. The dial is
legitimate and should exist.

What it moves is **when a prompt fires**, never **what the prompt claims**.

- Legitimate: "prompt me on anything borderline." Nothing false is asserted. The person
  asked to be interrupted more often and is interrupted more often.
- Not legitimate: firing "you sinned" at 0.4 confidence because strict mode is on. That is
  not strictness. It is a falsehood with a slider attached, and no one can meaningfully
  precommit to being told a false thing about themselves.

Strict mode means *pause me on the ambiguous case*. It never means *convict me on the
ambiguous case*.

### Presets, not a continuous slider

A slider invites adjustment at the worst moment — the hard moment is exactly when a person
wants it lower, and exactly when past-them knew better. Ship three or four named levels,
chosen deliberately.

### Asymmetric change cost

**Tightening takes effect immediately. Loosening takes 24 hours.** That single rule is
most of what makes this a covenant rather than a preference pane, and it costs one
timestamp. An impulse does not survive a day; a considered decision does — which is also
the honest way to distinguish the 2am impulse from a real choice to leave, without ever
needing to interrogate the person about which one it is.

---

## The person labels, the model does not

We cannot read the heart. The person can tell us. That is the only honest access to
intent that exists, and it is also better training data than any teacher model produces.

The `warn` interstitial already lets a user flag an event (see
[protection-modes.md](protection-modes.md)). Extend that into the formation loop: after a
block or a pause, offer one tap — *this was a stumble* / *this was fine*. A question
asked, never a verdict delivered.

This single control does three things at once:

- **It is the ground truth we could not otherwise get.** No public teacher model encodes
  where a given person's line sits, because no dataset does. The person's own labels fit a
  personalized classifier head to *their* actual struggle, locally, from a few hundred
  examples.
- **It is already a practice.** Reviewing your own conduct before God is a thing the
  intended user does. A one-tap version is not a UX affordance bolted onto a classifier —
  it is the point, with the classifier demoted to its proper role: something that notices,
  and asks.
- **It puts the authority in the right place.** The machine reports what it saw. The person
  judges what it meant. That is the only arrangement that survives being wrong.

It must be skippable with no cost, no nag, and no "you have 12 unreviewed events" badge.
An unanswered prompt is an answer.

---

## Reflection follows the shape of the daily review — and the order is load-bearing

For the optional reflection feature, do not invent a structure. Use the one that already
exists and has been iterated on since the 1500s: the **Examen** (also called the daily
examen or examination of conscience), a short structured review of the day from the
Ignatian tradition, still practiced daily by many Christians. Naming it here so it can be
looked up; the design only needs its shape.

It has five movements, in this order:

1. **Gratitude** — begin with what was good in the day. Not as a warm-up. As the frame
   everything else is read inside.
2. **Ask for light** — ask God to show what matters, rather than trusting your own
   inventory of your day. An admission that self-assessment is unreliable.
3. **Review** — walk through the day as it actually went, noticing where you were drawn
   toward God and where away.
4. **Face what went wrong** — specifically, without flinching, and receive forgiveness for
   it. Sorrow that terminates in mercy, not in rumination.
5. **Resolve** — one concrete thing for tomorrow. Forward-facing, small, actionable.

**Gratitude is first and the review is third. This ordering is the entire anti-despair
mechanism, and it is not stylistic.** Build the same data with the review first and you
have shipped an accusation log that opens with a list of a person's failures. Build it in
this order and you have shipped a practice. Identical inputs; opposite formation.

Movement 4 terminating in forgiveness rather than in a record is what makes the release
rule below necessary rather than optional.

**The ordering is also a safety mechanism, not only a formation one.** See "The reflection
surface is deliberately small" below — unstructured self-examination is the failure mode
this sequence was shaped against over several centuries. It is not decoration and it is
not reorderable for UX convenience.

---

## The reflection surface is deliberately small

The intended user is a person who believes they are failing God, in secret, repeatedly.
Shame, sexual struggle, conviction, and isolation is a genuinely elevated-risk cluster.
If this tool reaches any meaningful number of people, some of them will use it in crisis.
That is arithmetic, not speculation, and it constrains the reflection feature more than
anything else in this document.

The response is **less surface, not more capability.** A tool that cannot counsel, will
not generate, and hands off to a person cannot do this particular kind of damage. This is
what the restraint in "The tool points away from itself" is *for*.

### Ship the tap. Defer the journal.

One-tap self-report — *this was a stumble* / *this was fine* — carries almost no crisis
surface, and it already delivers the ground truth, the personalized head, and the practice.

Open free-text journaling carries a large surface that nobody is supervising at 3am. It is
deferred. See [crisis-surface.md](crisis-surface.md) before building it.

### Prompts are pre-written and closed. Never generated.

- **Permitted:** concrete, behavioral, answerable. *"You tend to stumble late at night —
  what usually comes before that?"*
- **Forbidden:** open-ended interrogation of the person's worth or state. *"Why do you keep
  failing?"*

A fixed pool is reviewed once by a human and is then safe forever. A generated prompt says
something new to someone in an unknown state with nobody watching. This is the same reason
[verse-selection.md](verse-selection.md) is deterministic: **determinism here is a safety
property, not an engineering preference.**

### AI may organize. It may never speak to the person about themselves.

- **Fine:** offline labeling for training (VLM-as-teacher for categories with no public
  dataset), classification, and locally restructuring text the person wrote themselves.
  None of it touches the person's interior life or leaves the device.
- **Not fine:** generating reflections, prompts, verdicts, or pastoral language.

A classifier emitting `0.87` is honest about being a machine. A language model saying *"it
sounds like you're carrying something heavy tonight"* is impersonating a pastor, and it is
more dangerous precisely because it is more convincing. Every rule in this document about
not rendering verdicts applies with more force to a fluent model, not less — a classifier
cannot pretend to see the heart, and a language model will pretend well.

This also settles the privacy question before it is asked: reflection content is more
sensitive than block events, and [mission.md](../mission.md) already promises none of it
leaves the device. Local or not at all.

### Solo mode needs a floor

[accountability.md](accountability.md) states that solo mode is complete and dignified,
and that stands. But dignified does not mean sufficient in every state a person may be in,
and solo mode has no human in it by definition.

The floor is not a partner — it is a resource. Crisis resources shipped in-bundle, exactly
the way [verse-pools.md](verse-pools.md) ships verse text, surfaced on a lexicon match that
`text-policy` is already shaped to do. No network call, no verdict, no diagnosis, no
logging of the match. Showing someone a number is not an intervention; it is declining to
be the only thing in the room.

Design and region coverage are open — see [crisis-surface.md](crisis-surface.md).

---

## Show shape, never sum

A person should be able to understand their own patterns. A person should never be handed
a number to be measured against. The line:

- **Shape — yes.** "Stumbles cluster on weeknights after 11pm, usually starting with the
  phone in bed." Diagnostic, actionable, self-knowledge. This is what movement 3 above is
  *for*.
- **Sum — no.** "47 this month, down from 62." A scoreboard. Same underlying data; the
  difference is whether it aggregates into a verdict.

**Permitted:** distributions, times of day, contexts, triggers, co-occurrences, "what
usually precedes this."
**Forbidden:** running totals, streaks, averages, month-over-month trends, and anything
with a target line.

If the sum is not shown, then not by works is not merely stated in the mission document —
it is true of the artifact. This rule is checkable at code review, which is why it is
written down.

---

## No streaks, no scores

Consequence of the above, called out separately because it is the single most likely thing
to be added in good faith by someone trying to help.

Streak mechanics work for language apps and are poison here. A streak converts a fall into
the **loss of accumulated value**, which is precisely the despair mechanism: break day 90
and the pull is *"it's ruined now"* → give up → binge. That is not motivation. It is a
despair engine with a flame icon.

> "Because of the LORD's great love we are not consumed, for his compassions never fail.
> They are new every morning; great is your faithfulness."
> — Lamentations 3:22–23 (NIV)

Nothing accumulates, so nothing can be lost. A fall must never cost the person anything
they had been storing up, because in the thing being modeled, it doesn't.

---

## Release deletes the episode and keeps the pattern

We will hold the most sensitive log a person has. Local-first (see
[mission.md](../mission.md), "Local-first and private") covers the privacy half. This is
the other half: an application that remembers a person's sins forever is asserting
something false about what happened to them.

> "as far as the east is from the west, so far has he removed our transgressions from us."
> — Psalm 103:12 (NIV)

Provide an explicit action — *confessed and released* — that genuinely deletes the episode
record. Not archived, not soft-deleted, not tombstoned. Gone.

The technical resolution that makes this free: **keep the trained head weights, drop the
episode.** The personalized classifier retains the *pattern* it learned; the *record* of
the event is destroyed. The tool goes on being useful without going on being a witness.
Nothing is sacrificed in either direction.

Retention default for unreleased episodes should be a rolling window, not forever.

---

## Leaving is frictioned, never shamed

A harness comes off. That is what distinguishes it from a cage, and per the mission it is
sometimes the goal rather than the failure.

The existing exits — voice gate, partner notification on attempt, install protection — are
**friction**, and friction stays. Friction is the covenant holding at 2am and it is
working as designed.

**Shame is different and is forbidden on every exit path.** No "are you sure you want to
give up?" No guilt-worded confirmation dialogs. No framing departure as failure or
relapse. No dark patterns on uninstall.

The 24-hour asymmetry already separates the impulse from the decision. Once a decision has
survived a day, the tool has no standing to editorialize about it. A person who leaves
well should be able to leave well.

---

## The tool points away from itself

> "After beginning by means of the Spirit, are you now trying to finish by means of the
> flesh?"
> — Galatians 3:3 (NIV)

The work is God's (mission.md, Philippians 1:6). Everything about ordinary product design
will fight this, and every one of those instincts is wrong here:

- **Do not optimize engagement.** Time in app is not a good. It is close to the opposite.
- **Do not make it sticky.** No daily-open incentives, no notifications engineered for
  return visits, no habit hooks.
- **Hand off.** Prompts should point to scripture, prayer, confession, the person's
  partner, their church, actual people — not deeper into the application.
- **Be quiet.** The mission cites Matthew 6:6. The tool should be about as loud as that
  room.

An application that becomes the center of someone's spiritual life has failed, even if the
blocking worked perfectly. The ambition belongs in the restraint.

---

## Open tension: the weekly partner digest

[accountability.md](accountability.md) specifies a counts-only weekly summary sent to the
partner. That is a sum, and "show shape, never sum" forbids sums. This is a real conflict
and it is not resolved here.

The argument that it is fine: a count delivered to a *person who knows you* is not a
scoreboard. The partner exercises judgment; the number is a prompt for a conversation, not
a verdict. Proverbs 27:17 and James 5:16 are load-bearing for that design and are not
retracted.

The argument that it is not fine: it is still a tally, it still invites month-over-month
comparison, and the user knows it is being generated — which makes it a scoreboard they
cannot see but can feel.

**Provisional rule pending a decision:** the digest should tell the partner *when to ask*,
not *how the person is doing*. That likely means a signal rather than an integer. Whoever
takes this up should update this section and `accountability.md` together.

---

## What was rejected

- **Inferring intent from behavior.** Session shape, dwell time, escalation patterns, and
  recidivism are all measurable and all proxies for *struggle*, never for *sin*. Even a
  perfect behavioral classifier would not earn the right to make the accusation, so the
  accusation is not made. Behavioral signals may inform *when to ask*; they may never
  inform *what is claimed*.
- **A universal sin taxonomy in the model.** Rejected for the reason in "Why the model
  never renders a verdict" — the label depends on the observer. Per-person heads over a
  shared frozen embedding, fit from that person's own labels.
- **Gamification of any kind.** See "No streaks, no scores."
- **Scoring the person rather than the content.** The model scores pixels and text. It has
  no person-level score, no risk rating, and no spiritual-state estimate. There is no field
  for it in any schema, which is the most reliable way to ensure no one ships one.
- **Generated reflection prompts, and cloud AI anywhere near user data.** See "The
  reflection surface is deliberately small." Both are rejected on safety grounds first and
  privacy grounds second; either one alone is sufficient.
- **Expanding the tool to meet the crisis case.** The correct response to "a person might
  be in a bad place" is a smaller tool that hands off faster, not a bigger one that tries
  to help. See [crisis-surface.md](crisis-surface.md).
