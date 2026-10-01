# 12. `AccessibilityText` — AX text extraction. **Done.**

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/AccessibilityText.swift
```

Reads on-screen text without injection by walking the AX tree of the focused window
(`kAXValueAttribute`, `kAXTitleAttribute`, `kAXDescriptionAttribute` over the descendant elements),
and feeds it to `packages/text-policy`. Behind **Accessibility**.

A **supplement to OCR, not a replacement.** Coverage varies wildly, and anything drawing its own
text — canvas-based UIs, games, most Electron content — returns nothing useful. The scan loop must
never treat an empty AX result as "no text on screen".

Two traps:

1. **AX calls are synchronous cross-process IPC and can block for seconds** against a hung or busy
   application. Called on the scan loop's thread, one unresponsive app stalls the entire daemon.
   Call `AXUIElementSetMessagingTimeout` with a short timeout (a few hundred ms) and do the walk off
   the scheduler's thread. Also bound the tree walk by depth and node count — some apps expose
   pathologically large trees.
2. **Chromium-based apps expose nothing by default.** Chrome, Electron and anything else on
   Chromium keep their accessibility tree switched off for performance and only build it when a
   client sets `AXManualAccessibility` to true on the application element. Without that, the walk
   returns a nearly empty tree and reads as "this app has no text" — which, given how much of the
   relevant surface is a browser, would quietly gut the module's value.
   **This is measured wrong — see the table below.** The symptom is real; the stated cause and fix
   are not. Chrome 151 refuses the attribute outright.

## What was built, and what the live measurements overturned

**Done.** `AccessibilityText.swift` ships `AXNodeID` (an opaque handle, so the walk needs no AX
types), `AXWalkLimits` (40 deep / 2000 nodes), `AXTextWalk` (text plus which bound stopped it), the
`AXElementProbing` edge with `FakeAXElementProbe`, the pure iterative walk, and `SystemAXProbe`.
22 tests (`AccessibilityTextTests.swift`), 272 in the suite. An `ax-text [delay] [no-manual]` verb
exercises the real edge; it walks **twice** from one process, because a single walk cannot tell a
tree that is still building apart from an app that exposes nothing.

**Trap 2 above is wrong, and it was wrong in the direction that matters.** Measured on macOS 26.5:

| Application | `AXManualAccessibility` write | Web content exposed |
|---|---|---|
| Chrome 151 | **rejected** — `kAXErrorAttributeUnsupported` (`-25205`) | yes, ~3s after any client starts reading |
| Chrome 151, `AXEnhancedUserInterface` | **rejected** — `kAXErrorNotImplemented` (`-25208`) | — |
| Obsidian, Todoist (Electron) | **accepted** — `kAXErrorSuccess` | **no** `AXWebArea`, still absent after 4s |

So the opt-in the plan called the fix is refused by the one application it was written for, and
accepted by the ones where it changes nothing. What actually built Chrome's tree was **being an AX
client at all** — the first walk returned 65 nodes of toolbar chrome, and a walk about three
seconds later returned the page's heading, body text, button label and image `alt` text. The write
is kept: it is one ignored message, and it is the only lever there is on older Chromium builds.

The *consequence* the plan drew is unchanged and is what to design around — **the first walk against
a browser is legitimately thin, and only a repeated walk sees content**. `AccessibilityScanner`
(session 4) walks on the OCR cadence and so gets this for free; a one-shot caller does not.

Four smaller findings:

- **A `static let` of type `CFString` does not compile under Swift 6** — non-`Sendable` global. Held
  as a `String` and bridged at the call site, the same shape as `PermissionGate`'s
  `trustedCheckOptionPrompt`.
- **Cycle detection cannot be skipped in favour of the depth bound.** A cycle still costs a full
  40-level walk on every branch that enters it. `AXUIElement` is a CoreFoundation type with no
  useful Swift identity, so the visited set is keyed on `CFEqual`/`CFHash`.
- **A real window can nearly saturate the 2000-node bound** — a Telegram window measured 1524 nodes
  and 61 KB of text. The default is the right order of magnitude, but not by much.
- **The element separator is not a boundary, and no character could be.** Elements are joined with a
  newline, but `text-policy`'s `collapse_whitespace` rewrites it to a space and its `compact`
  pipeline strips it entirely — so a lexicon phrase can match across two unrelated elements (a
  sidebar label ending in one word, a heading starting with the next). The only real fix is to
  evaluate elements separately rather than as one blob, which is a **decision for session 4's
  `AccessibilityScanner`**, not something this module can make by choosing a different character.
- **An app can report zero windows while running.** Safari returned an empty `AXWindows` with its
  windows on another Space, and `open` reused them there rather than moving one across. Harmless
  for the daemon, which only ever reads the frontmost app on the active Space, but it makes any
  "measure app X without focusing it" diagnostic unreliable.

**Outstanding:** Safari's page *body* has not been confirmed extracted — Safari read fine (45 nodes
of real window content) but every attempt to put the test page in front of it lost focus back to the
terminal. Chrome proves the web-content path works; this is a 20-second manual re-run of
`ax-text`, not a code question.

## Reference documents

- [Apple — `AXUIElement` attributes](https://developer.apple.com/documentation/applicationservices/axuielement_h) — attribute reads and `AXUIElementSetMessagingTimeout`.
- [Apple — `AXError`](https://developer.apple.com/documentation/applicationservices/axerror) — the `-25205`/`-25208` codes in the table above.
- [Chromium — accessibility on macOS](https://www.chromium.org/developers/design-documents/accessibility/) — the on-demand tree and the `AXManualAccessibility` opt-in behind trap 2, which Chrome 151 no longer implements.

---
