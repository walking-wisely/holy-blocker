# ADR: Custom-app blocking

| Field | Value |
|---|---|
| **Status** | Accepted 2026-10-05 on the owner's answers; revised from the first draft, see "Owner answers" |
| **Date** | 2026-10-05 |
| **Owner** | Ivan Dutov |
| **Supersedes** | — |
| **Superseded by** | — |

Tier: product decision extending [mvp-scope.md](mvp-scope.md), which already records the owner's
answers on the panel and the cooldown. Its per-app modes are narrowed by this record to block only. It touches protection modes and the tamper model, so
the red-team is tier 2 (three lenses) under
[decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md); findings are at the end.

---

## Context

MVP requirement 2 is "a person names an application and it is blocked regardless of content". Today
neither platform has a per-app check. Measured 2026-10-05:

- `WindowSuppression` (macOS) hides an application only after a content block verdict, and holds a
  `defaultProtected` set (`com.holyblocker.daemon`, Finder, Dock, SystemUIServer, loginwindow) that
  it never hides. Its `protectedBundleIdentifiers` is a mutable `var` and its 5-second cooldown
  exists for content verdicts.
- `ScreenGuardService` (Android) reads `event.packageName` and passes it to `ScanGate` and
  `SettingsGuard`; nothing compares it to a deny list. It finds the foreground package from
  `rootInActiveWindow`, so it is event-driven and foreground-only; `watchedWindows` already walks all
  windows for the settings screens.
- `ProtectionSchedule.kt` implements request, cooldown, confirm and expiry on
  `SystemClock.elapsedRealtime()`, because `currentTimeMillis()` is user-settable. A confirmed disarm
  opens a window in which the guard is off.
- `apps/mobile` targets SDK 36, has `android:allowBackup="false"`, and its manifest has no
  `<queries>` and no `QUERY_ALL_PACKAGES`.
- `ScreenGuardService.kt` writes the foreground package to `Log.d` in three places: `scan pkg=`,
  `settings screen pkg=` and `empty harvest pkg=`. The macOS `MacDaemon` sources have no `os_log`,
  `NSLog` or `print` today.

## Decision

1. **A list entry is an application identity, and a listed application is blocked.** The identity is
   the Android package name or the macOS bundle identifier, never a display name. There are no
   per-app modes: an application is on the list or it is not.
2. **What blocking does.**

   | Android | macOS |
   |---|---|
   | Send the user home (`GLOBAL_ACTION_HOME`) and cover the app with the overlay on every visible window of that package | `terminate()` on launch and activation, and again on relaunch. If the app is still running after a grace period, hide it and cover its windows, repeating on every scan. Never `forceTerminate()` |

   Hiding and covering on macOS is the fallback for an app that refuses to quit, not a mode. The
   listed-app path does not use `WindowSuppression`'s 5-second cooldown: that cooldown exists so a
   content verdict does not hammer an app that refuses, and a listed app is not a verdict.

   Android blocking is not "terminate": plain Device Admin and an accessibility service have no
   documented way to stop a foreground app of another uid. That is unverified here and is observed by
   `mobile.custom-app-enforcement`. macOS never force-terminates, for the reason
   `WindowSuppression.swift` records: a blocker that loses unsaved work gets uninstalled.
3. **Honest labels.** Android blocking is shown as "send home and cover" and macOS blocking as
   "close". The panel states on its face that clones, work-profile copies and other copies of an
   app are not covered, and that an app's notifications are not suppressed. Release wording says an
   app is covered or closed, never that it is stopped.
4. **There is no per-app hide.** Hiding text or images inside an application is what the text policy
   and the image classifier do, for every application and not per app, and that path is unchanged.
   Scoping it per app is not needed and is not planned. The owner's earlier "hide" mode is dropped.
5. **Removal costs, in every protection phase.** Removing an app from the list is requested, waits
   out the disarm cooldown, and must be confirmed, as `ProtectionSchedule` does for disarming.
   Adding is immediate and idempotent. One pending removal per app; an unconfirmed request expires
   and is deleted, as is a confirmed one once applied. The store refuses a removal without a
   confirmed per-app request in every phase, including `DISARMED` and `OFF`, so the list cannot be
   edited while enforcement is off and found changed on re-arm.
6. **Clocks.** Android uses `elapsedRealtime`. macOS uses a monotonic clock that continues across
   sleep and that the user cannot set (`mach_continuous_time`), never `Date()`. A request whose
   stored start lies in the future, which is what a reboot looks like, is void on both platforms,
   never "ready". The vectors cover it.
7. **Strength.** Custom-app blocking is exactly as strong as protection itself: a confirmed disarm
   opens a window in which all enforcement stops, for every entry at once. The per-app cooldown stops
   removing an entry from being cheaper than disarming, and nothing makes it stronger than disarming.
   Release wording does not claim more. While protection is armed, including during a pending or
   ready disarm request, enforcement runs; it stops when protection is disarmed or off, and the
   development build's kill switch stops it with everything else.
8. **Some applications cannot be listed, decided by clues.** A short undo for everyone was
   considered and rejected (rejected alternatives): it is a loophole for every app to fix a mistake
   that only a few apps can make dangerous. Instead each candidate identity is rated on device:

   | Rating | Meaning | Panel behaviour |
   |---|---|---|
   | `never` | Life-sustaining, or the person cannot function without it: the platform shell, emergency and phone-dialer apps, banking and payment apps, medical-device apps, authenticator and 2FA apps, maps, and work tools including work messengers such as Slack | Refused at add, not offered, not accepted by hand |
   | `ordinary` | Everything else, including the common consumer messengers (Telegram, Instagram, Facebook) | Listed on the person's own decision; the removal cost is stated at add time |

   Messaging is not blanket `never`: the line is consumer messengers a person may be right to block
   against work messengers whose blocking would be a disaster.

   The clues are on-device and cheap: a bundled, reviewable list of identities; name tokens such as
   a bank or payment term in the identity; and on macOS the bundle's declared
   `LSApplicationCategoryType` (finance and health categories), whose exact values the core step
   verifies. Android has no local finance category, so there the clues are the list and the tokens;
   that is unverified here. The rating is conservative in one direction: it may refuse a harmless
   app and must not miss an obvious bank. Further applications are `never` only by a change to the bundled list, which the owner approves. Like the protected set, the rating is evaluated at
   enforcement time, so an entry that later turns `never` is not enforced.
9. **The protected set is evaluated at enforcement time.**
   - It holds the product's own id and the platform shell: on Android the system UI, the Settings
     package, the current default launcher, the default dialer and the default SMS app, each resolved
     through the role or intent at the moment of the check, not cached at add time; on macOS the
     existing `SuppressionPolicy.defaultProtected` set. The `never` rating of decision 8 joins it.
   - The panel and the store reject protected identities at add time as a convenience. The
     enforcement check is the one that holds: an entry that becomes protected later (a listed app
     made the default launcher) is ignored while it holds the role, never acted on, so there is no
     home-launches-the-app loop.
10. **Enumeration is not the boundary.** The panel enumerates in memory when it opens and discards
    the list when it closes. Android lists launchable apps through a `<queries>` entry for the
    `MAIN`/`LAUNCHER` intent, not `QUERY_ALL_PACKAGES`. macOS lists bundles under `/Applications`,
    `~/Applications` and `/System/Applications`. Both panels also accept an identity typed by hand,
    because an app the panel cannot list is not thereby unblockable.
11. **Android enforcement is not foreground-event-only.** Enforcement runs on every window of a
    listed package, as `watchedWindows` does for settings, and runs again when the list changes, so
    an app already in the foreground, in split-screen or in a floating window when it is added is
    covered without waiting for a new event.
12. **A listed app also blocks domains that carry its name, with a stated confidence.** An app
    and its website are one service, and a person who blocks the app means the service. The MVP
    does not map an app to a set of domains; it matches names.
    - From the entry the core derives tokens: the identity's distinctive segment (`instagram` from
      `com.instagram.android`) and the app's display name, lowercased and reduced to letters and
      digits.
    - A DNS name or proxy host matches when the registrable domain's label equals a token
      (`instagram.com`, high confidence) or a label of any depth contains it (`cdninstagram.com`,
      medium). Short tokens (under five characters) and tokens on a stoplist of common words (for
      example `line`, `meta`, `mail`, `maps`) never match, because they would block unrelated sites.
    - The panel shows the derived tokens and what each matches at add time and the person confirms
      them. A match is never silent, and the person can drop a token. Domains matched by a confirmed
      token are one unit with the app entry: they follow the entry, and removing the app removes them,
      under the same cooldown.
    - The matcher is a new kind of explicit rule in `net-shield` (keyword, evaluated after the
      device-local allowlist and with the explicit `DomainFilter` rules in the precedence of
      `blocklist.rs`), exposed to Android over the existing UniFFI wrapper and to the macOS proxy
      through its own step. This is a cross-platform contract, so it is an owner question
      (owner question 2).
    - A name under a private suffix (`github.io`, `blogspot.com`) never matches: its leading label
      belongs to a tenant, not to the service. Owner answer 2026-10-06.
    - Heuristic by design: it misses a service whose domains do not carry its name, and it can
      block an unrelated site that does. It is sufficient for the MVP and revisited afterwards.
13. **Data stays on the device and is not logged.**
    - The list lives in the app's private store: `filesDir` on Android (`allowBackup="false"`, and a
      test fails if it flips); the agent's Application Support directory on macOS with mode `0600`
      in a `0700` directory, excluded from Time Machine and iCloud backup. The macOS list is
      readable by the protected user's own account, and the claim that it stays on the device carries
      the standard-account scope of mvp-scope.md.
    - A pending lowering (app and request time) is stored in the same private store, one record per
      app, deleted on confirm or expiry.
    - Enforcement writes no per-block record anywhere: not the tamper log, not a log line, not a
      counter per app. The tamper-log vocabulary gains no event for a blocked app, and a test asserts
      no enforcement path passes a package or bundle identifier into `TamperEntry.detail`, which is
      free text.
    - No `Log.*` in `ScreenGuardService` prints a package name. `mobile.custom-app-enforcement`
      removes `pkg=` from all three lines and adds a gate that fails on a package name in any `Log.*`
      call in the file. On macOS no `os_log`, `NSLog` or `print` prints a bundle identifier, and a
      gate in `mac-daemon.custom-app-core` fails on one.
    - The privacy inventory row for these data classes (chosen-app list, installed-app enumeration,
      foreground package, pending lowering) is added by this record's PR, as mvp-scope.md requires
      before the first custom-app step merges.
14. **No tamper-log record of a list change in the MVP,** and no per-block signal to the person: there
    is no liveness indication that a block fired, and the release wording says so. A lowering
    request is reserved as the content-free event a partner flow will emit once partners exist; it
    will carry no identity.
15. **Two small pure modules, not a Rust crate.** The matcher and the lowering rule are about fifty
    lines each, written in Kotlin and Swift against the same test vectors, as
    `mac-daemon.protection-schedule` ports `ProtectionSchedule`. A UniFFI crate would add a
    cross-platform contract for logic that is neither a secret nor heavy. The shared contract is
    the decisions 5 and 6 above and the rating and token rules of decisions 8 and 12.
16. **macOS is advisory until its agent is observed.** The agent runs as the protected user, so a
    standard user can edit the list file or stop the agent. Decisions 5 and 6 on macOS are advisory
    until `mac-daemon.standard-user-tamper` observes whether the agent can be stopped, and release
    wording claims no parity with Android for requirement 2 on macOS until then. Hardening the store
    (root-owned, written only through the daemon) is a trust-boundary change that passes through the
    owner after that result.

### Keyword stoplist

`net-shield::keyword::STOPLIST` is the list decision 12 refers to. Tokens under five characters
never match, so the short words in decision 12's examples (`line`, `meta`, `mail`, `maps`) are
listed only to record them. The words that carry weight are the longer common ones:

`browser`, `calculator`, `calendar`, `camera`, `chat`, `clock`, `cloud`, `drive`, `email`, `files`,
`games`, `line`, `mail`, `maps`, `media`, `meta`, `mobile`, `music`, `news`, `notes`, `phone`,
`photo`, `photos`, `search`, `settings`, `shop`, `store`, `translate`, `video`, `videos`,
`weather`, `world`.

It changes only through this record. Brand names are deliberately absent: a token such as `google`
is the person's own confirmed choice.

## Step requirements

| Step | Must do, beyond its title |
|---|---|
| `mobile.custom-app-core` | Matcher, idempotent add, removal request/confirm/expiry, store with the backup test, protected-set resolution and the `never` rating at check time, token derivation, the vectors in decisions 5, 6, 8 and 12, a test that nothing writes an identity into `TamperEntry.detail` |
| `mac-daemon.custom-app-core` | The same, with the monotonic sleep-surviving clock, the file mode and backup exclusion, and the no-logging gate. Depends on `mac-daemon.protection-schedule` |
| `mobile.custom-app-enforcement` | Window-wide enforcement and a rescan on list change, removal of every package name from `Log.*` with its gate, observation of whether `block` can stop an app and how long content shows before cover |
| `mac-daemon.custom-app-block` | Launch and activation observers, repeat-hide after the grace period, no use of `WindowSuppression`'s cooldown |
| `mobile.custom-app-panel`, `mac-daemon.custom-app-panel` | The honest labels and the coverage note of decision 3, manual identity entry, the removal cost stated at add time, refusal of `never` identities, and the derived tokens shown for confirmation |
| `net-shield.keyword-rule` | The keyword rule kind of decision 12: token minimum, stoplist, label matching, precedence after the allowlist; the stoplist changes only through this record |
| `mitm-proxy.keyword-rule` | Refuse a CONNECT authority or plain HTTP host that a keyword rule matches; depends on `mitm-proxy.domain-blocklist` |
| `mobile.custom-app-domains`, `mac-daemon.custom-app-domains` | Feed a confirmed app's tokens to the DNS path (Android) or the proxy (macOS), observed: a name carrying the token is refused, an unrelated name is not, and the tokens follow the entry and its removal. Needs `net-shield.keyword-rule` (and `mitm-proxy.keyword-rule` on macOS) |

## Rejected alternatives

| Alternative | Why not |
|---|---|
| Match on display name | Names are localized, renamed and spoofable; the identity is the only stable key |
| `forceTerminate()` for macOS `block` | Discards unsaved work; `WindowSuppression.swift` chose hide over close for this reason |
| `QUERY_ALL_PACKAGES` on Android | Broader than needed to list launchable apps and a wider package-visibility grant to hold |
| A Rust crate with UniFFI for the matcher and the schedule | A new cross-platform contract for a trivial pure function |
| Record each block in the tamper log | A usage history of the person's chosen apps; the log is for the guard, not for activity |
| Immediate removal | Defeats requirement 3 as it would defeat it for disarm |
| Check the protected set only when adding | The Android launcher role and a mutable macOS set change after add; a check at add time either exempts a listed launcher or loops home into the app |
| The wall clock for the cooldown | User-settable on both platforms |
| A short undo window for every newly added app | Owner review: a loophole for every app to fix a mistake only some apps can make dangerous. Replaced by the `never` rating of decision 8 |
| A per-app `hide` mode | Owner review: hiding text and images is app-wide and decided by the text policy and the image classifier; a per-app cover is not that, and is not needed |

## What this does not cover

- **Another route to the same service beyond its name.** Decision 12 blocks domains that carry the
  app's name; a mirror, a second client with a different identity, or a service whose domains do not
  carry its name is not covered, and the match can block an unrelated site.
- **Clones, work profiles and parallel-space apps on Android,** and a macOS app copied and re-signed
  with a new bundle identifier. Neither is detected.
- **A short flash before cover.** The overlay is drawn after an accessibility or `NSWorkspace` event,
  so content can show for a moment. How long is observed in the enforcement steps, not argued here.
- **Notifications, widgets, share sheets, audio and background work** of a covered or closed app.
- **Android `block` does not terminate the app.** It is "home and cover" until an observation says
  otherwise.
- **The disarm window.** All enforcement stops for its length (decision 7).
- **A false positive on an app that is essential to one person.** The `never` rating covers the
  obvious cases only; every other app is the person's own decision, and a wrongly listed ordinary
  app stays blocked for the cooldown.
- **A user-writable list on macOS** until `mac-daemon.standard-user-tamper` (decision 16).
- **Windows and iOS.**

## Revisit after the MVP

Deliberately deferred, to be reopened once the MVP is observed on the owner's devices:

- **Audio, notifications, widgets and share sheets** of a covered or closed app. These are the
  leaks the MVP leaves open and the first to revisit.
- Background work of a blocked app, and the short flash before cover.
- Clones, work profiles and re-signed copies.
- The domain match of decision 12: a real app-to-domain mapping instead of name matching, and the
  stoplist.
- The `never` rating's clues and which further applications belong in it.
- Hardening the macOS list store, and the disarm window's effect on enforcement.
- A partner event for a lowering request, and any liveness signal that a block fired.

The same list is in the mobile and mac-daemon backlogs.

## Owner answers

Given 2026-10-05 on the first draft:

1. **`never` additions:** medical devices, authenticators and 2FA, maps and work tools. Messaging is
   not blanket `never`: consumer messengers (Telegram, Instagram, Facebook) stay listable and work
   messengers (Slack) are `never`.
2. **Name matching:** confidence tiers and a stoplist, as decision 12 has them.
3. **Per-app `hide` is dropped.** Hiding text and images is app-wide through the text policy and
   the image classifier. An application has only the blocked state (decision 4).
4. **No record of list changes** in the MVP is accepted (decision 14).

## Red-team findings

Three fresh-context runs, 2026-10-05, all by reading; none executed code, followed by the owner's
review comments. Folded findings are listed; the rest are in "What this does not cover".

| # | Lens | Finding | Resolved by |
|---|---|---|---|
| 1 | Bypass | The add path can lower a mode with no cooldown | moot after modes were dropped; decision 5 |
| 2 | Bypass | Per-app cooldown adds nothing beyond disarm; a disarm window stops all enforcement, and the list may be edited while off (**changed the design**) | decisions 5 and 7 |
| 3 | Bypass | The protected set is checked at add time; a listed launcher either loops or is exempt (**changed the design**) | decision 9 |
| 4 | Bypass | macOS relaunch gets a 5-second window per cycle and a refused terminate falls to a user-undoable hide | decision 2 |
| 5 | Bypass | The macOS list is user-writable, which makes the cooldown advisory | decision 16 |
| 6 | Bypass | Android enforcement is foreground-event-only | decision 11 |
| 7 | Bypass | Enumeration gaps leave apps unlistable | decision 10 |
| 8 | Bypass | No clock named for macOS | decision 6 |
| 9 | Intent | Android `block` and `hide` promise more than they do | decisions 3 and 4 |
| 10 | Intent | A mistaken block of an essential app has no emergency path (**changed the design**) | decision 8 (the undo was rejected on owner review) |
| 11 | Intent | The cooldown conflates a mistaken add with an impulse to lower | decision 8 |
| 12 | Intent | A lowering is an attempt the accountability model would want, and nothing signals it | decision 14 |
| 13 | Intent | No liveness signal that a block fired | decision 14 |
| 14 | Legal and privacy | `pkg=` is logged in three places, not one (**changed the design**) | decision 13 |
| 15 | Legal and privacy | The macOS no-logging rule is unenforced | decision 13 |
| 16 | Legal and privacy | The macOS store has no stated permissions or backup exclusion | decision 13 |
| 17 | Legal and privacy | The macOS confidentiality claim is overstated | decision 13, standard-account scope |
| 18 | Legal and privacy | `allowBackup` is relied on without a test | decision 13 |
| 19 | Legal and privacy | The inventory row was not committed to | decision 13 and this PR |
| 20 | Legal and privacy | `TamperEntry.detail` is free text, so content-freeness rests on the vocabulary alone | decision 13 |
| 21 | Legal and privacy | A pending lowering stores timing metadata | decision 13 |
| 22 | Owner review | Name-matching domains for a listed app is wanted for the MVP (**changed the design**) | decision 12 |
| 23 | Owner review | List what to revisit, audio and notifications first | "Revisit after the MVP" |
| 24 | Owner review | A short undo for every app is a bad idea; rate apps by clues and refuse the life-sustaining (**changed the design**) | decision 8 |
| 25 | Owner review | Per-app hide is not what was meant; hiding is app-wide (**changed the design**) | decision 4 |
| 26 | Owner review | Consumer messengers are listable, work messengers and the named categories are not | decision 8 |
