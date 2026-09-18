---
name: privacy-review
description: >
  Review Holy Blocker changes against the project's privacy disposition — what sensitive data a
  diff creates, stores, or transmits, for how long, and whether it ever leaves the device — and
  audit an existing package's data handling against the inventory in references/data-classes.md.
  Use this skill on every full-loop step (wired unconditionally in step-loop), and on any change
  touching screen capture, OCR or AX text, URL or page-title handling, classification scores or
  verdicts, blocklist hits, tamper logs, or persistence. Also triggers on "privacy review",
  "what data does this collect", "audit privacy", "is this GDPR ok", or "does this phone home".
---

# Holy Blocker Privacy Review Skill

This skill runs in one of two modes. Decide which before doing anything else.

**Authoring mode** — a change is being written or reviewed. Scope is the diff. Apply the checks for
whichever data classes the diff touches, and nothing else. Output is inline feedback.

**Audit mode** — a package is being assessed as a whole. Scope is the package. Output is backlog
entries in `docs/engineering/privacy-backlog.md`, *not* fixes. Do not fix findings during an audit;
an audit that turns into a refactor stops being an audit.

If the user hasn't said which, infer from scope: a diff means authoring, "audit `packages/x`" means
audit. Ask only if genuinely ambiguous.

## Scope discipline

This skill is mechanical and internal. It is **not legal advice** and it does **not** certify GDPR
(or any other) compliance. It answers one narrower question, which a diff set can always be judged
against:

> What personal or sensitive data does this change create, store, or transmit? For how long, and
> is that retention bounded and stated? Does any path leave the device — cross-checked against
> `CLAUDE.md`'s no-cloud-calls rule ("do not add cloud calls, telemetry, remote content analysis,
> or external dataset dependencies unless the user explicitly asks for them")?

If the answer to the first question is "none of the enumerated classes and no new one", no further
question is answered — there is nothing to govern. Whether a bound is *legally adequate* is a
question for a lawyer; whether a bound *exists and is stated* is a question this skill can settle,
and doing that much is its whole job.

## Step 1: route

Read `references/data-classes.md` and match the touched paths to the enumerated classes. If nothing
matches, say so and stop — a change that adds a settings toggle to `apps/desktop/src/renderer/`
touches no data class, and inventing one wastes the reviewer's attention and erodes trust in the
gate. The same rule applies to a diff that introduces a genuinely new kind of stored or logged data:
then the new class is the match, and it is governed as one (see Step 2, third check).

Load only the sections that matched. Most changes touch one.

## Step 2: apply the checks per touched class

For each data class the diff touches, the data-classes entry names the files that own it, its
GDPR special-category standing, its current retention, and whether any path transmits it
off-device. Check the diff against the class's *current* state:

- **Retention is finite and stated.** Any new file, log line, buffer, or ring that holds a data
  class must have a cap written next to it (a line count, a byte bound, a `MAX_ENTRIES`-style
  constant), in the same change that introduces it. "Unmeasured" in `data-classes.md` is a
  finding, and a diff that creates a store while an existing one stays unmeasured is two findings,
  not one.
- **No new transmission path.** No socket, pipe, or framework connection that a data class can
  reach may be added without an explicit statement of where it goes, and if it leaves the device
  it is blocked by this skill unless the user asked for it — the `CLAUDE.md` rule. A loopback or
  Unix-socket leg that stays on the device is fine and should be named as such.
- **New stored or logged data classes extend the inventory in the same change.** If the diff
  introduces a genuinely new kind of stored or logged data — not a new file in an enumerated
  class, but a *new class* — it must add a row to `references/data-classes.md` in that same PR,
  with a path citation, and never be left implicit in a comment.

Audit mode adds a fourth check: every entry in the package's current inventory is *re-observed*,
not assumed — a class whose owning file no longer exists, or whose retention constant has moved,
is a finding (see the report format).

## Step 3: report

**Authoring mode** — name the class, the concrete data (screen pixels, extracted text, a score, a
domain), and what the change does with it. One finding per real issue; if the diff is clean, say
it's clean; do not pad.

If a finding is a genuine question of product taste — "should scan events be kept at all, and at
what cap?" — flag it as a judgment call and do not adjudicate it here; it goes up the same
escalation channel as every other call the loop is not allowed to make for the user.

**Audit mode** — append non-fixed findings to `docs/engineering/privacy-backlog.md` using this
shape:

```markdown
### <package>: <one-line claim>

- **Data class:** <from references/data-classes.md>
- **Retention:** <finite and stated, or unmeasured>
- **Off-device:** <none | path, named>
- **Evidence:** `path/to/file:line`
- **Status:** open | accepted-baseline | fixed in <commit>
```

A finding you cannot attach to a real file and a real retention state is not a finding — drop it.
Recording a gap as `accepted-baseline` is a valid outcome and the point of the audit: it converts
unknown debt into scheduled work. The gate is green on *new* findings, never on a clean package.
An audit finding must never be fixed inline; it is filed so it can be scheduled.

## Scope limits

- Ordinary bugs and code quality → `/code-review`, not this skill.
- Attack modelling, trust boundaries, and attacker-with-a-code-path questions → the
  `holy-blocker-security` skill; the two overlap at content capture, and privacy-review flags the
  leak while security-review names the attacker.
- Legal and regulatory questions → out of scope entirely. This skill does not give legal advice
  and does not certify GDPR compliance; it answers the mechanical data-inventory question the
  frontmatter and the scope-discipline paragraph state, and nothing more.

## Where this skill's claims come from

The inventory in `references/data-classes.md` is derived from the repository at the commit it is
reviewed at, not from `CLAUDE.md` or a plan: every row cites a file, and a citation that no longer
resolves is a finding, not an edit to make silently. Retention is settled by a named constant in a
named file, never by a comment that says "we don't keep it". Off-device is settled by a named path
or by the checked absence of one across the runtime trees. GDPR Art. 9 special-category standing is
recorded per class where the class is about a person; a score about the content of a specific
person's screen is special-category data, and that is recorded as the product's own finding, not as
a compliance claim about it.