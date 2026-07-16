# Decision: Accountability Model

> **Status: reopened, pending [crisis-surface.md](crisis-surface.md).**
> Partner mode — including whether it should exist at all — is deferred. The counts-only
> weekly digest additionally conflicts with "show shape, never sum" in
> [formation-model.md](formation-model.md). Nothing below is retracted, but do not build
> against it without reading `crisis-surface.md` first. Solo mode, the voice gate, and the
> notification-on-attempt behaviour are unaffected by the deferral.

## Two modes: solo and partnered

The system supports two accountability modes. Neither is required. Neither is the
"incomplete" version of the other.

**Solo mode** (default) — the user has no configured partner. The voice gate still
runs on disable attempts. The weekly summary is kept locally in the event log. The
gate verse, the friction, and the event history all function exactly as in partnered
mode. The user is accountable to themselves and to God. This is a complete,
dignified configuration — not a stepping stone.

**Partnered mode** — the user designates one trusted person (a friend, spouse, pastor,
counsellor, or anyone they choose). In addition to everything solo mode provides:

1. **Voice gate notifications** — when the user attempts to transition to `off` mode,
   the partner receives a notification regardless of whether the attempt succeeds.
2. **Weekly summary** — a periodic digest of block/warn event counts (no URLs, no
   content) is sent to the partner so they have ambient awareness of activity.

The design goal is to make partnered mode the *easier path* — lower friction to set up,
more rewarding to maintain — without making solo mode feel like a failure state.
Someone who does not have a trusted person to ask, who lives somewhere a conversation
about this topic would be dangerous or impossible, or who simply is not ready to
involve another person should feel fully supported by the tool.

---

## Why solo mode is complete, not a fallback

Not everyone can find a suitable accountability partner. Reasons vary:

- No trusted relationship close enough for this conversation
- Cultural or religious context where naming this struggle publicly is unsafe
- Living somewhere a Christian accountability framework does not apply
- Early stages — the user wants to build trust in the tool before involving anyone else

Treating these users as having an incomplete setup would be a design failure. The gate,
the verse, the event log, and the internal friction are the core of what the tool
provides. The partner amplifies them; the partner does not enable them.

---

## Why partnered mode is worth offering

Going alone is harder. That is simply true, for most people, most of the time.

> "As iron sharpens iron, so one person sharpens another."
> — Proverbs 27:17 (NIV)

The notification to a partner at the moment of a gate attempt is not enforcement — the
partner cannot do anything to prevent the disable. It is presence. The knowledge that
someone will see the attempt changes the felt cost of the attempt, which is exactly
the moment when that change matters most.

The weekly summary creates a natural check-in cadence without requiring the user to
self-report. Self-reporting is hard. It is easy to rationalize silence on a bad week.
A count that arrives automatically removes the choice of whether to mention it.

The goal is not surveillance. It is making the easier path the one that includes
another person.

---

## Why the notification fires on attempt, not only on success

An attempt to disable protection is meaningful even when the user abandons it.

A person who opens the disable flow, reads the verse aloud, and then cancels has
experienced a significant temptation moment. That moment is worth being known by the
partner — not as evidence of failure, but as an opportunity for a supportive
conversation before a bad week becomes a worse one.

A system that only notified on successful disables would be gameable and would create a
false sense of clean weeks. Hidden near-misses accumulate into failures.

The notification content is minimal: timestamp, mode-change attempt, outcome (cancelled
or completed). No content about what was being blocked.

---

## What the partner receives

The weekly summary contains only aggregate counts:

```
This week:
  Blocked: 14
  Warned: 3
  Override attempts: 0
```

No URLs, no page titles, no matched keywords, no images. The summary is a signal, not
a log. It tells the partner whether the system is working and whether the user is
under unusual pressure. It does not expose browsing history.

The voice gate notification contains:

```
[Name] attempted to disable Holy Blocker protection at [time].
Outcome: [cancelled / protection disabled until [time] / protection disabled].
```

---

## Why the weekly summary is counts-only

The partner relationship is a support relationship, not a surveillance relationship.
Sending URLs or content details to the partner would:

1. Create a chilling effect on the user's normal browsing — they would censor
   legitimate searches to avoid awkward conversations.
2. Expose content to the partner that neither of them needs to dwell on.
3. Shift the dynamic from mutual support to parental monitoring, which is the wrong
   frame for a voluntary adult covenant.

The partner's role is restoration and encouragement. Counts give them enough to know
when a conversation is needed. Details are not required for that and would likely make
the relationship harder to sustain.

---

## Making partner mode the easier path

The risk with an opt-in accountability feature is that setup friction causes most users
to skip it indefinitely — they intend to add a partner "later" and never do. The design
should make partnered mode genuinely easier than staying solo, not just available.

### Setup: invite link, not a form

The partner should not need to install anything or create an account. The user
generates a shareable invite link from the desktop app and sends it however they
naturally communicate with that person — a message, an email, a shared note. The
partner opens the link in a browser, reads a plain-English explanation of what they
will and won't receive, and confirms with their name and email. That is the entire
setup on their side.

Requiring the partner to install software or create an account would cut the completion
rate dramatically. A link they can open on their phone in thirty seconds will not.

### Setup: surfaced in onboarding, not buried in settings

Partner setup should appear as a named step during initial onboarding — not as a
settings panel the user might find later. The onboarding step should be skippable
without friction (solo mode is complete), but it should be the natural next thing after
protection mode is configured, not an afterthought.

### Rewarding: shared streak

Both the user and the partner see a streak — days without a successful gate override.
The streak is the primary positive signal. The weekly block counts tell the partner
whether the tool is working; the streak tells them whether the user is holding.

The streak resets to zero when protection is disabled (gate cleared). It does not reset
on warn events or cancelled gate attempts — those are the system doing its job, not
failures.

Milestones (7 days, 30 days, 90 days, 1 year) generate a notification to both parties.
This gives the partner something to respond to positively, not just something to watch
for when things go wrong.

### Rewarding: partner can send one encouragement

The partner's dashboard (a simple webpage, no app required) has a single action
available: send a short encouragement to the user. Either a tap on a pre-written option
("Proud of you", "Praying for you", "Keep going") or a short free-text field. This
arrives as a notification in the desktop app.

This closes the loop. Without it, the accountability relationship is one-directional —
the partner receives information and has no obvious way to respond inside the system.
A low-friction response mechanism turns the weekly summary from a report the partner
reads into a conversation that both parties participate in.

### Partner configuration

The partner is configured during onboarding or from the desktop control panel. Setup
requires only: the partner's name (for display) and a shareable invite link sent to
them via any channel. The partner must confirm before notifications begin — they opt
in, not the user on their behalf.

The partner can revoke participation at any time via an unsubscribe link in any
notification. If the partner unsubscribes, the user is notified so they can reconfigure
or transition to solo mode without losing event history.

There is no way for the partner to remotely change settings, force a mode change, or
receive any content beyond the counts and gate events described above.

---

## Alternatives considered

**Solo mode as a degraded fallback.** Rejected. Users who cannot or choose not to
involve a partner deserve a tool that works fully for them. Framing their situation as
incomplete would exclude a large portion of the people this project is for.

**Real-time per-event notifications to the partner.** Rejected. High-frequency
notifications would create anxiety for both parties and would expose more content
context than is appropriate. Weekly summaries are sufficient for the check-in model
this feature targets.

**Partner can view the full event log.** Rejected — counts-only is the right boundary
between accountability and surveillance.

**Partner must approve mode changes.** Rejected. The user is an adult making a
voluntary covenant. The partner is a support, not a gatekeeper. Requiring partner
approval before the user can disable their own tool inverts the relationship in a way
that would likely cause the partner to disengage.

**Anonymous community accountability (shared streak board, etc.).** Possible future
feature. For v1, the one-to-one partner model is simpler and safer to implement without
creating a social platform the project cannot moderate.
