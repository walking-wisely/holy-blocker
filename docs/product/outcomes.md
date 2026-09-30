# Product outcomes

This file states what Holy Blocker is meant to do for the person using it, and where each outcome
stands today. It speaks in user-facing capabilities and names only the packages that carry them.

It sits above, and is separate from, the two technical layers below it: the per-package build plans
in [`../components/`](../components/) describe how each package is built and in what order, and the
[coverage ledger](../engineering/coverage.md) is the only place that computes what actually gets
blocked, on which platform, by which layer, and what does not. This file is not generated from
either. The rationale for the mission lives in [`../mission.md`](../mission.md).

## Block adult content on Android without a cloud service

**Status:** in progress, on an unmerged branch for part of it. The on-screen text path is observed
working on `master`; DNS domain blocking is observed only on an unmerged branch, and imagery is not
covered at all.

**Packages:** `apps/mobile`, `packages/text-policy`, `packages/text-policy-ffi`,
`packages/net-shield`, `packages/net-shield-ffi`, `packages/domain-blocklist`,
`packages/domain-normalize`

What actually gets blocked: [coverage ledger — Android](../engineering/coverage.md#android).
Why local-only matters: [mission — local-first and private](../mission.md#local-first-and-private).

## Block adult content on macOS

**Status:** in progress, on unmerged branches. The frontmost-window text path and the screen image
path both live on branches that are not on `master`, and the image path has never run against a
real captured frame.

**Packages:** `native-modules/mac-daemon`, `packages/mitm-proxy`, `packages/text-policy`,
`packages/text-policy-ffi`, `packages/image-sandbox`

What actually gets blocked: [coverage ledger — macOS](../engineering/coverage.md#macos).

The network half is the furthest along: a real HTTPS page can be rendered through the supervised
proxy with the local certificate installed. Screen text and screen imagery are not yet real.

## Block adult content before it reaches the browser

**Status:** in progress. The signed blocklist artifact and its lookup path are implemented, but the
producer's output has not been fed through the device lookup path in one run, and no device has
been observed blocking a domain from it.

**Packages:** `packages/net-shield`, `packages/domain-blocklist`, `packages/domain-normalize`,
`packages/net-shield-ffi`, `packages/mitm-proxy`, `native-modules/win-network`

What actually gets blocked: [coverage ledger — Cross-cutting](../engineering/coverage.md#cross-cutting).

This is the layer meant to drop known-unholy domains at the packet level and inspect the rest.

## Block explicit imagery, in transit and on screen

**Status:** in progress. Intercepted-image classification is present on `master`; on-screen imagery
is unverified on macOS and not built on Android, and no ledger row records either as observed end
to end.

**Packages:** `packages/image-sandbox`, `machine-learning`, `native-modules/mac-daemon`,
`apps/mobile`

What actually gets blocked: [coverage ledger — macOS](../engineering/coverage.md#macos) and
[Android](../engineering/coverage.md#android).

A domain block cannot see an image served from a clean-looking CDN, which is what this outcome
addresses; it is the least proven of the blocking outcomes.

## Ship a desktop control panel

**Status:** not started. An internal shell and daemon-status plumbing exist, but there is no usable
control panel and no packaged, user-installable build.

**Packages:** `apps/desktop`, `native-modules/mac-daemon`, `native-modules/win-daemon`

What actually gets blocked: [coverage ledger — Cross-cutting](../engineering/coverage.md#cross-cutting).

## Make disabling or removing protection a deliberate act

**Status:** in progress. Android device administration and settings guarding are observed working
on `master`; the macOS permission-revocation path has never been observed after a real revocation,
and no package asserts at runtime that it is still running.

**Packages:** `apps/mobile`, `native-modules/mac-daemon`, `native-modules/win-daemon`

Where the gaps are: [coverage ledger — Android](../engineering/coverage.md#android).
Why the friction is intentional: [mission](../mission.md).

## Know that protection is actually running

**Status:** not started. The ledger records that no layer proves it is still alive rather than
merely not complaining, so a silent failure is currently indistinguishable from working protection.

**Packages:** `apps/desktop`, `apps/mobile`, `native-modules/mac-daemon`, `native-modules/win-daemon`

What actually gets blocked: [coverage ledger — Cross-cutting](../engineering/coverage.md#cross-cutting).

## Protect on Windows

**Status:** not started. The Windows daemon has event hooks and a message loop only; nothing is
captured, classified, or blocked.

**Packages:** `native-modules/win-daemon`, `native-modules/win-network`, `packages/net-shield`,
`apps/desktop`

What actually gets blocked: [coverage ledger — Windows](../engineering/coverage.md#windows).

## Protect on iPhone and iPad

**Status:** not started. Investigated only.

**Packages:** None yet.

What actually gets blocked: [coverage ledger — iOS](../engineering/coverage.md#ios).

## What's next for the product

1. Ship the on-screen text guard that already works on Android as an installable build a person
   can run.
2. Turn the macOS screen text and image paths into an observed, end-to-end block.
3. Feed the produced domain blocklist through the device lookup path and observe a real domain
   being blocked.
4. Validate image classification against real captured frames on both macOS and Android.
5. Make the desktop a usable control panel: local history, mode control, and a visible connection
   to the background protection.
6. Add the deliberate-pause path — the accountability notification and the voice gate — to every
   way protection can be switched off.
7. Build Windows protection outward from the daemon and the network service.
8. Revisit iOS once the hardware needed to verify a local VPN path is available.

## A one-way relationship

Product outcomes reference packages; the build plans and the coverage ledger never reference
outcomes. The plans say how a package is built, and the ledger says what the union of packages
actually covers. This file says what the user is meant to get and how much of it is real. When a
capability changes, the ledger is updated first, and this file follows only if the user-facing
promise changed.
