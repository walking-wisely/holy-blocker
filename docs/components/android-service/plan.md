# Android Service — superseded

**This plan is not current. The Android work lives in [`apps/mobile/`](../mobile/plan.md).**

`native-modules/android-service/` was never created and will not be. When the Android MVP was
built it landed as a standalone Gradle project under `apps/mobile/` instead, and
[components/mobile/plan.md](../mobile/plan.md) is the maintained plan for it. This file is kept
only so the two decisions that reversed are on the record rather than rediscovered.

## What changed

### The package moved

Planned as a `native-modules/` sibling to `win-daemon`; shipped as `apps/mobile/`. The Android
build is a self-contained Gradle project that the pnpm workspace does not manage, which makes it
an app rather than a native module. Nothing else in `native-modules/` is a full application.

### Device Owner was ruled out

The superseded plan was built around Device Owner provisioning and called
`setAlwaysOnVpnPackage` "the highest-value call in the package". **This product targets plain
Device Admin and must not be designed around owner-only capability.**

Device Owner requires a factory-reset device and grants a level of control that a user
installing a self-imposed accountability tool should not reasonably be asked for — the request
itself reads as hostile. That rules out everything the old plan's `policy-admin` module was
built on: `addUserRestriction` and every `DISALLOW_*` constant, `setUninstallBlocked`,
`setUserControlDisabledPackages`, `setPermittedAccessibilityServices`, and
`setAlwaysOnVpnPackage`. Each is documented as callable by a device owner, profile owner, or
delegate; a legacy device admin is none of those.

The replacement is not weaker in practice. Removal paths are blocked by the accessibility
service itself — watching for the settings screens that would disable it and backing out before
they are reachable — which is what shipping blockers do without any owner privilege. See
§7 of the [mobile plan](../mobile/plan.md) for the current design, its real limits, and the
ethical boundary it holds.

## What carried over

The rationale for why Android inverts the desktop layer order, the Restricted Settings analysis,
and the MediaProjection start-order requirement all survive and now live in the mobile plan. The
design rationale itself is unchanged and remains in
[content-interception.md](../../decisions/content-interception.md#android--the-layer-order-inverts).

## What was dropped as speculative

The two-process architecture (`:ui` process, AIDL `IProtectionService`, a `ProtectionStatus`
parcelable, React Native dashboard) was designed before any Android code existed. No dashboard
has been built, the services are a single process today, and none of that structure has been
validated against a real requirement. It is not carried forward. If a dashboard is built later,
the process-boundary question should be reopened on its own merits then.
