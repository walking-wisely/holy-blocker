# Failure catalogue

Ordered by how often each has occurred in this repository. The first three account for most of the
real defects found so far. Every entry names an instance, because a checklist item with no instance
behind it is a guess about ourselves.

---

## 1. A constant that was guessed rather than measured

**Shape.** A threshold, size floor, timeout, cadence, buffer size, or retry count appears in the
code with no measurement behind it, often justified by a plausible sentence.

**Instances.** Both of `image-sandbox`'s v0 constants were wrong at once — the size floor
(remeasured to 96px) and the classification threshold (which now has no shippable default at all,
because a threshold belongs to a model *and* a geometry).

**How to review it.** For each numeric literal introduced or changed: where did this number come
from, and what would falsify it? Accept "measured, here is the command"; accept "arbitrary, and
nothing depends on the exact value, here is why"; reject a number with a rationale but no
observation. Check whether the constant is annotated at its definition with the measurement — an
unannotated constant is a claim that has lost its evidence.

**Escalate when** the constant crosses a boundary. A threshold that ships inside an FFI surface,
a config struct, or a bundled artifact is a contract, and the wrong pairing has shipped here more
than once with nothing failing to indicate it.

---

## 2. Failing open, silently

**Shape.** A guard that stops guarding produces output indistinguishable from a guard that is
working. This is the highest-severity class in this repository and every component has produced at
least one.

**Instances.**

- A missing `INTERNET` permission on Android: blocked names refused, permitted names silently
  unanswered — *the failure looks exactly like the filter working*.
- A `420v` pixel format on macOS: every frame correctly refused, symptom identical to a missing
  Screen Recording grant.
- A failure to load the previous manifest in `domain-blocklist`'s gates: indistinguishable from a
  first build, and it silently disables two gates.
- `ImageOutcome.Allow` carrying no score versus a score of 0.0 — collapsing them makes a broken
  image path read as a clean screen.
- An empty or missing `blocklist.txt`: an open guard with no signal.

**How to review it.** For each failure path the change introduces — permission denied, artifact
missing, parse failed, upstream unreachable, work still in flight — ask: *what does an observer see,
and can they distinguish it from success?* Then ask the stronger question: **does anything assert
the positive direction?** Refusing bad input is not evidence of filtering; permitting good input
must be asserted too, in the same test or the same runtime check.

Fail-open is often correct — `image-sandbox` fails open deliberately on every path. The defect is
never the fail-open; it is the silence. Require a distinguishable signal, and require that
"nothing was classified" stays representable separately from "classified as fine".

---

## 3. Built to the plan's model of the world rather than the world

**Shape.** The implementation is a faithful realisation of a plan whose premise is false. Reviews
that check code against plan cannot see this; only measurement can.

**Instances.** `mac-daemon` module 12 was written around `AXManualAccessibility`, which Chrome
rejects outright. The bundle module was written around usage-description `Info.plist` keys that do
not exist on the shipped OS. Both findings are recorded in `CLAUDE.md` with the phrase
"contradicts the plan it was written from" — after the code existed.

**How to review it.** Identify the two or three external facts the change depends on and check
whether any were observed rather than assumed. Where a comment says "the plan says X" or cites
documentation for runtime behaviour, treat it as unverified. Run `assumption-audit`'s recipes
against them.

**When a load-bearing premise is false, say so in those words** and propose the redesign. Do not
report it as a note to fold into the plan; that is how these findings have historically been
absorbed without changing anything.

---

## 4. Scope narrowed to what is reachable, with the remainder recorded as a note

**Shape.** A defensible engineering narrowing — "only the focused window", "only CONNECT-tunnelled
traffic", "only plaintext port 53" — is documented honestly in its own component's row, and the
union of all such narrowings is never computed anywhere.

**Instances.** `mitm-proxy`'s `forward_http` takes no `ScanHooks` at all, so URL, body and image
scanning apply only to tunnelled traffic. The mac daemon's text path scans only the frontmost
window. The Android VPN filters plaintext DNS on port 53 and claims a single `/32` route.

**How to review it.** State the narrowing in user terms — not "the scanner reads the focused
window" but "content in a window you are not clicking on is not read". Then name what covers that
case, and if nothing does, report it as a finding rather than a limitation. Update
`docs/engineering/coverage.md`.

---

## 5. Done at the module boundary rather than the capability boundary

**Shape.** A module is complete, tested, and merged while the ends it connects have never met.
The status ledger records sessions completed rather than capability shipped.

**Instances.** `ScreenCapture` and `ImageGuard` were each fully built and separately verified while
"the classifier has never seen a real captured frame" — the two ends met only through a synthetic
buffer.

**How to review it.** Ask what single user-visible sentence this work makes true, and what
demonstrated it. If the answer is a unit test with a fake at the seam, the finding is that the
seam is unverified. Check the merge state too: work described as done in a status table but sitting
on an unmerged branch behind `master` has not shipped, and the table should not say otherwise.

---

## 6. Documentation drift standing in for integration

**Shape.** A discovered gap is written into the plan, the backlog, or the status row — which is
correct and valuable — and that write-up becomes the whole response. Nothing merges, nothing
integrates, and the finding reads as handled.

**How to review it.** For each "recorded in the plan" / "filed in the backlog" in a change, ask
whether recording was the appropriate response or the available one. Also check that the status
table being updated is the one on `master`; per-branch status files fork, and a branch's copy can
say "done" about work `master` has never seen.

---

## 7. Deference under pressure

**Shape.** Challenged on a finding, the reviewer reframes rather than either holding the position
with evidence or conceding it plainly. Both correct outcomes are cheap; the reframe is the failure.

**How to review it.** When re-reviewing after pushback, restate the original claim exactly, then
say whether it stands and on what evidence. If it does not stand, say it does not stand. Do not
produce a third position that avoids the question.

---

## 8. Fabricated or unverifiable citation

**Shape.** A quotation attributed to a file that does not contain it; an RFC section number that
does not say what is claimed; a vendor behaviour sourced from memory.

**Instance.** A review here quoted `docs/decisions/classifier-operating-point.md` for a sentence
that is not in it. A separate fix round corrected an RFC citation from RFC 1035 §4.1.1 to
RFC 2308 §2.2 — the first defines the RCODE field, not NODATA semantics.

**How to review it.** Open the file and copy the line. Open the RFC section and read it. Where a
vendor domain, API name, or version number is asserted from memory, mark it `UNVERIFIED` — inventing
plausible ones is worse than omitting them.
