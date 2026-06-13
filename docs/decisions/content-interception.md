# ADR: Content Interception Strategy

| Field | Value |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-06-10 |
| **Owner** | Ivan Dutov |
| **Stakeholders** | Platform engineering, UX |
| **Supersedes** | — |
| **Superseded by** | — |

---

## Context

Holy Blocker needs to intercept content before the user perceives it — across browsers,
native desktop apps, media players, and third-party software — on whatever platform the
user is running. The challenge is that no single interception mechanism covers every
surface, and the right mechanism for each surface differs by platform and by the
permissions the daemon has been granted.

Three broad questions drive this decision:

1. **What surfaces exist?** Where can content reach the user, and what interception point
   is available for each?
2. **What mechanisms are available?** What are the real options for intercepting content at
   each surface, and what do they cost in terms of coverage, latency, fragility, and
   maintenance?
3. **Where are the component boundaries?** Which parts of the solution are inherently
   shared across platforms, and which are genuinely platform-specific?

The answer to question 3 is the load-bearing part of this decision. Getting the boundaries
wrong means either duplicating work that should be shared or coupling platform-specific
code that needs to diverge.

---

## Surfaces

Content reaches the user through two fundamentally different paths:

**Network path** — HTTP/S traffic flowing through the OS network stack. Covers browser
content and any app that fetches over the network. The interception point is below the
app; the app is unaware.

**Render path** — content already in memory, being painted to the screen. Covers native
apps, media players, sandboxed browsers where network interception is insufficient or
where TLS pinning bypasses the proxy, VM windows, and any process the network layer
cannot reach. The interception point is at or above the app, and the app may resist.

These two paths are not redundant — they cover different threats. A browser fetching a
resource over HTTPS is best handled at the network path (before it renders). A native app
displaying cached content is only reachable via the render path.

---

## Mechanism comparison

### Network-level MITM proxy

A local proxy intercepts all HTTP/S traffic, inspects request URLs and response bodies,
and can sever connections or inject content into HTML responses.

**Covers:** all browser traffic, any app that honours OS proxy settings and does not pin
its TLS certificate.

**Does not cover:** apps with certificate pinning, apps that bypass system proxy settings,
local or cached content, native app rendering.

**Cost:** requires the proxy CA to be trusted by the OS certificate store (user must
accept a one-time install-time prompt). Once trusted, inspection is transparent to the
browser.

**Shared vs platform-specific:** the proxy logic (URL scanning, body scanning, HTML
injection) is fully platform-neutral. The certificate trust installation is
platform-specific (Windows cert store, macOS Keychain, browser-specific stores on Linux).

**Verdict:** essential first layer for browser content. Cannot be the only layer.

---

### Browser extension

A companion extension runs in the browser, receives matched-term payloads injected by the
proxy, and applies DOM-level overlays via `TreeWalker` + `MutationObserver`. It also
handles same-origin content the proxy cannot see (e.g. content assembled entirely in
JavaScript with no network fetch).

**Covers:** browser content that has reached the DOM. Handles dynamic injection (infinite
scroll, SPAs) naturally since `MutationObserver` fires on every DOM mutation.

**Does not cover:** anything outside the browser.

**Cost:** must be installed in each browser the user uses. Cannot be force-installed on
all browsers by the daemon; user must install manually or via enterprise policy.

**Shared vs platform-specific:** the extension code is entirely cross-platform. The
mechanism for distributing or pre-installing it varies by OS and browser vendor policy.

**Verdict:** necessary complement to the proxy for browser surfaces. Degrades gracefully
to proxy-only if not installed (proxy injection still runs; DOM handling is just less
precise for dynamic content).

---

### OS-level process injection (native app, deep access) — evaluated, deferred

On platforms where the daemon has sufficient privilege, it can inject code into target
processes, subclass their windows, and draw cover regions directly into the process's own
paint cycle. This gives zero z-order overhead — the cover is painted inside the target
window, not layered above it — and resize/scroll are handled naturally because paint fires
on every frame.

**Covers:** unprotected native apps where the OS allows injection. On Windows this means
processes without AppContainer, ACG, or similar hardening; processes that use those
mitigations actively block injection.

**Does not cover:** hardened processes (sandboxed browsers like Chrome and Electron apps,
Microsoft Store apps), processes running in VMs, fullscreen exclusive apps — i.e. exactly
the surfaces the capture path must cover anyway.

**Cost:** requires elevated privilege, is platform-specific, and is fragile against apps
that break subclassing or use non-standard message loops. It also needs a per-platform
known-hardened-process list to decide when *not* to attempt injection.

**Verdict — not part of the core model; deferred, and Windows-only if ever built.** Its one
real advantage over screen capture is per-frame paint performance on unprotected Win32 apps.
But the capture path (below) already covers those same surfaces *and* the hardened/VM/mobile
surfaces injection cannot reach, using the same shared classification models. Injection is
therefore a **later, optional performance optimization on Windows**, not a layer the
ecosystem is built around — it has no portable equivalent (see Linux/macOS below), so
designing the model around it would be designing around a single platform's edge case.
Building the capture path first means the reusable "brain" is exercised on day one.

---

### Screen capture + classification (two models, one capture)

The daemon captures window (or screen) regions and feeds each capture to **two models in
parallel**:

- **OCR → text-policy** — extracts on-screen text and runs it through the text-policy
  engine, producing flagged paragraph regions.
- **Image ML → image classification** — locates and classifies images in the capture,
  producing flagged image regions with a category.

These are not redundant. Text on screen and imagery on screen are different threats, and a
surface may carry either or both. A single capture per frame feeds both consumers; frame
differencing runs before either model so static frames don't trigger redundant inference.

This path works for any visible surface regardless of how the content was produced —
native app, sandboxed browser, VM window, PDF viewer — which is why it is the universal
fallback wherever the deeper paths cannot reach.

#### Response is mode-driven; geometry decides the mechanism

The models decide *whether* content is flagged. The protection mode decides *what happens*
when it is (see [protection-modes.md](protection-modes.md)); the geometry of the offending
content (windowed vs fullscreen) only decides the mechanism used to carry that out.

Regardless of mode, the daemon **pre-blurs first, refines second** — a cover is placed
over the region immediately on window creation, before any scan runs, then resolved to the
final response once the models return. This closes the gap between appearance and first
verdict; the alternative (wait, then respond) flashes unblurred content whenever pipeline
latency exceeds the window's own render latency.

What the cover resolves to depends on the mode:

- **`full` — block outright.** Flagged content is not made viewable. The daemon blocks the
  offending app surface (the cover stays opaque and is not dismissible; for whole-surface
  or repeated flags the app surface is blocked rather than the system covering region after
  region). There is no reveal affordance in this mode.

- **`warn` — cover with reveal.** Flagged regions are covered, but the cover is an
  interstitial the user can click to dismiss and read what is underneath. This mirrors the
  browser warn interstitial: the user sees that something was flagged, sees the verse, and
  makes a deliberate choice to uncover. Content stays reachable.

- **`off` — no response.** The capture loop is paused entirely; no cover is placed.

**Fullscreen is a geometry exception, not a mode exception.** When the image model flags
content occupying an exclusive or borderless fullscreen surface, neither a cover overlay
nor a clean surface block can be applied reliably to a fullscreen surface. The daemon first
**forces the app out of fullscreen** (restore to windowed), then applies the mode response
above to the now-windowed surface — block in `full`, cover-with-reveal in `warn`.

**Covers:** every visible surface the deeper paths cannot reach, plus fullscreen imagery
the overlay path alone could not handle. Also serves as the fallback for native apps when
the daemon lacks elevation.

**Does not cover:** content that never renders (pre-loaded but not yet on screen). True
exclusive-fullscreen apps that bypass the compositor are handled by forcing windowed mode
where the OS permits; where it does not (some games), the surface remains out of scope.

**Cost:** model latency introduces a brief full-window blur on each new window and after
scroll-settle. Scroll tracking (low-level mouse hook, scroll-delta extrapolation) gives
immediate visual response but requires re-scan on settle. The pipeline must be fast enough
that the blur period is not disruptive in normal use.

**Shared vs platform-specific:** the OCR pipeline, image ML model, classification logic,
verdict-to-response mapping, and frame differencing are shared. The capture mechanism,
overlay window construction, fullscreen-detection and exit-fullscreen mechanism, and the
OS event hooks for window lifecycle are platform-specific.

**Verdict:** universal fallback and the only path that handles on-screen imagery and
fullscreen content. Required wherever the deeper paths are unavailable or blocked. The
pre-blur-first sequence, the mode-driven resolution (block in `full`, reveal in `warn`),
and the force-windowed handling of fullscreen imagery are all non-negotiable for this path.

---

### Kernel / driver-level interception

A kernel driver (Windows minifilter, macOS kext / System Extension, Linux netfilter) can
intercept at a layer below all user processes, potentially closing gaps that both the
proxy and the render path leave.

**Covers:** in principle, anything. In practice: kernel network inspection offers little
over the proxy for HTTP/S traffic; kernel-level render interception does not exist as a
standard OS mechanism.

**Cost:** EV code signing (Windows), Apple notarization and System Extension approval
(macOS), root installation (Linux). A kernel bug is a BSOD/kernel panic — blast radius
is catastrophic. The remaining coverage gap between our current model and a kernel driver
is limited to surfaces already classified as out of scope (fullscreen exclusive games,
boot from external media).

**Verdict:** rejected for v1 and for the foreseeable future. The cost-to-coverage ratio
is deeply unfavourable given the gaps it would close are out of scope.

---

## Decision: two-layer interception with shared engines

We use a **two-layer interception model**, one per surface path. Each layer is independent,
degrades gracefully, and pushes as much logic as possible into shared, platform-neutral
engines.

```
Layer 1 — Network path (proxy + extension)
    Handles browser/network HTTP/S. Inspects URL + response body; the only
    path to true content (not just hostname) inspection. Shared logic;
    platform-specific cert install only. Desktop-only — Android/iOS cannot
    terminate TLS and degrade to DNS/SNI/IP filtering.

Layer 2 — Render path (screen capture → text + image models)
    The universal native-app path. One capture per frame feeds OCR→text-policy
    and image ML→classification. Pre-blur first, then resolve by mode:
    full = block outright, warn = cover with click-to-reveal, off = paused.
    Fullscreen imagery is forced to windowed first, then the mode response
    applies. Covers every visible surface and is the only path for on-screen
    imagery. Platform-specific capture/overlay/fullscreen-exit; shared OCR,
    image ML, and mode-to-response logic.
```

Process injection + in-process paint (the former "deep native" path) is **not a layer in
this model**. It is a deferred, Windows-only performance optimization over Layer 2 for
unprotected Win32 apps; it covers a strict subset of Layer 2's surfaces, has no portable
equivalent, and is built — if ever — only after the capture path ships. See the mechanism
comparison above.

The **component boundary** follows directly from this layering:

- **Shared (platform-neutral):** MITM proxy, text-policy engine, OCR pipeline, image ML
  model and classification, verdict-to-response mapping, frame differencing, browser
  extension, ProtectionMode propagation.
- **Platform-specific:** certificate store installation, screen-capture mechanism, overlay
  construction, fullscreen detection and exit-fullscreen mechanism, OS event hooks (window
  lifecycle, scroll, resize). *(Plus, deferred and Windows-only: the injection + paint
  mechanism, if that optimization is ever pursued.)*

The platform-specific parts are thin adapters. They feed into and consume from shared
pipelines. This boundary should be enforced structurally — platform-specific modules
must not contain policy logic, and shared modules must not import platform APIs.

### Highest-leverage components for the whole ecosystem

The per-platform analysis below makes one investment conclusion unavoidable: the adapters
get rewritten on every platform, but the two shared "brains" are reused everywhere they can
run and are what the product's value actually depends on. They deserve disproportionate
design effort, the most testing, and the most stable internal APIs:

1. **The Layer 2 classification models — text-policy, OCR, and image ML.** The most
   universal component in the system. They run on Windows, Linux, macOS, *and* Android —
   every platform with a render path — and they are what survives when the network layer
   collapses to DNS/SNI (Android). Only iOS cannot run them. If any one thing must be
   excellent and platform-clean, it is these.

2. **The Layer 1 MITM proxy.** Desktop-only (Windows/Linux/macOS, since Android and iOS
   cannot terminate TLS), but where it runs it is the highest-value, fully-shared network
   component and the *only* path to true content inspection — full URL and response body —
   anywhere in the ecosystem. Everywhere else, network filtering degrades to hostname/SNI.

This is also the build-order argument for dropping injection from the core model: the
capture path (Layer 2) exercises the most reusable component — the classification models —
on day one, on the platform being built first, in a form that transfers to every other
platform. Injection would invest early effort in a Windows-only mechanism that transfers
nowhere. Everything outside the two pillars — capture, overlay, fullscreen control, event
hooks, certificate installation, and the deferred injection optimization — is a platform
adapter: necessary, but rewritten per platform and kept thin so it never absorbs policy
logic.

### ProtectionMode propagation

`ProtectionMode` is held in the proxy and mirrored to the daemon via local IPC. Mode
changes take effect on the next request/capture cycle with no restart. Both layers read the
same mode.

| Mode | Layer 1 — Network (proxy) | Layer 2 — Render (capture + models) |
|---|---|---|
| Full | Block, sever connection | Block outright — opaque cover / app block, no reveal |
| WarnOnly | Allow + inject overlay script into HTML | Cover with click-to-reveal interstitial, content reachable |
| Off | Allow, no script injection | No response, capture loop paused |

(Fullscreen imagery is forced to windowed first in either active mode, then the row above
applies.)

---

## What is out of scope

These surfaces are explicitly not covered and are not planned:

- **True exclusive-fullscreen apps that resist windowing (some games):** where image ML
  flags fullscreen content, the daemon first attempts to force the app into windowed mode
  and then applies the standard response. Only the residual case — apps that hold an
  exclusive fullscreen surface the OS will not let us restore — remains out of scope.
- **Boot from external media:** the daemon is not running.
- **Content that never renders** (background tabs, pre-fetched resources): Layer 2 only
  captures visible windows. Layer 1 handles network-fetched content before it renders,
  which covers the browser case; native app pre-fetch is not a priority surface.

---

## Per-platform instantiation

The layered model is platform-neutral; each platform supplies its own thin adapters for
capture, overlay, and event hooks. The shared engines (text-policy, OCR, image ML,
mode-to-response logic) do not change between platforms.

### Windows — current state

Windows is the first platform being built. Layer 1 (proxy) is in progress; **Layer 2
(capture + text/image models) is the priority build target** for `win-daemon`, because it
exercises the reusable classification core that every other platform depends on.

- **Layer 2 mechanism:** window/screen capture via `BitBlt` / DXGI Desktop Duplication,
  feeding OCR→text-policy and image ML→classification in parallel.
  `WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | HWND_TOPMOST` overlay window for
  cover regions. Fullscreen detected via window style/extent checks; force-windowed via
  `ShowWindow(SW_RESTORE)` or synthesised restore where the app cooperates. Scroll tracking
  via `WH_MOUSE_LL`. Window lifecycle via `WH_SHELL` + `SetWinEventHook`.
- **Deferred optimization — in-process injection paint:** `CreateRemoteThread` +
  `LoadLibrary` for DLL injection, `SetWindowSubclass` + `WM_PAINT` for in-process paint,
  UIA `IUIAutomationTextRange` for text. This is the only place in the ecosystem injection
  applies, and it buys only paint-performance on unprotected Win32 apps that Layer 2 already
  covers. Not built until the capture path is solid; may never be worth it.
- **Elevation model:** elevation no longer gates *coverage* (Layer 2 capture runs at user
  IL). It gates **tamper resistance**: a SYSTEM service the standard user cannot stop is the
  Windows analogue of the macOS admin lock and the Android device-PIN gate. The optional
  injection optimization, if built, would also require the elevated service.

### Linux — what carries over and what changes

Linux is not yet built. This section records how the model maps so the boundary stays
honest as the codebase grows.

The Linux desktop is not one target. There is no portable Layer 2 mechanism — capture,
overlay, and window control all differ by display server (X11 vs Wayland) and, on Wayland,
by compositor. Rather than chase every combination, **we target the two environments that
cover the large majority of real Linux desktop users first**, and treat the rest as
follow-on work to be considered only once those two ship.

**Carries over unchanged (every environment).** Layer 1 is almost entirely portable: the
proxy and browser extension are identical. Only certificate trust installation differs —
there is no single system trust store; the CA must be added to the NSS databases used by
Firefox and Chromium-family browsers (`~/.pki/nssdb`) and to the system bundle
(`/usr/local/share/ca-certificates` + `update-ca-certificates`). The text-policy, OCR, and
image ML engines and the mode-to-response logic are untouched everywhere.

**The deferred injection optimization does not apply here at all.** Windows-style in-process
injection has no portable equivalent — `LD_PRELOAD` and `ptrace` are fragile, restricted by
`yama` (`ptrace_scope`), and blocked under Flatpak/Snap, with no single rendering API to
subclass across X11/Wayland and GTK/Qt. This only reinforces keeping it out of the core
model. All native-app coverage on Linux is Layer 2 (capture). Where deep text extraction is
wanted, the accessibility bus (AT-SPI over D-Bus) is the right substitute for OCR — it reads
on-screen text without injection — but it is opt-in per app and inconsistently populated, so
it supplements rather than replaces capture.

#### Primary path 1 — X11 sessions

X11 is the permissive case and the cheapest to cover; it is also still a large share of the
installed base. A normal client has every Layer 2 power it needs:

- **Capture:** `XShmGetImage` / XComposite for per-window pixmaps.
- **Overlay:** an override-redirect, input-transparent, top-most window — the direct
  analogue of the Windows layered click-through overlay.
- **Fullscreen exit:** EWMH `_NET_WM_STATE_FULLSCREEN` toggle via the root window.
- **Event hooks:** X11 event selection / XInput2 for window lifecycle, scroll, and move.

X11 sessions are close to fully coverable with Layer 1 + Layer 2, mechanism-for-mechanism
with Windows. This is the first Linux target.

#### Primary path 2 — GNOME on Wayland

GNOME is the most-used desktop, and most distributions now default it to Wayland — so this
is the most important Wayland environment, and also the most restrictive. GNOME does **not**
implement `wlr-layer-shell` and has no plans to; its stated position is that this class of
functionality belongs in a **GNOME Shell extension**, which runs inside the compositor and
therefore has the window access a normal client is denied. The Layer 2 mechanism on GNOME
is consequently split between a normal daemon process and a Shell extension:

- **Capture:** the `xdg-desktop-portal` `ScreenCast` interface over PipeWire. A persistent
  `restore_token` (`persist_mode = 2`) stores consent in the permission store, so capture
  costs a **one-time** prompt rather than per-session consent.
- **Overlay:** a GNOME Shell extension draws the cover regions inside the compositor — the
  only reliable way to place a cover over arbitrary windows on GNOME Wayland.
- **Fullscreen:** on Wayland everything is composited, so a compositor-drawn cover sits over
  fullscreen content directly; the Windows "force-windowed" step is not needed here.
- **Event hooks:** window lifecycle and geometry come from the Shell extension's view of the
  window stack; input via the compositor.

The cost is that this path requires shipping and maintaining a Shell extension alongside the
daemon, versioned against GNOME Shell releases. That is the price of covering the largest
Wayland desktop, and it is accepted for this target.

#### Deferred — everything else (consider only after the two paths above)

These are real but secondary; none should be started before X11 and GNOME-on-Wayland ship:

- **wlroots compositors (Sway, Hyprland, …) and KDE Plasma on Wayland.** These *do*
  implement `wlr-layer-shell`, which allows a much cleaner mechanism than the GNOME
  extension: a single full-screen, input-transparent surface on the **overlay layer**
  (specified to render above fullscreen surfaces), onto which the daemon composites blur
  regions using coordinates from the portal capture stream — no foreign-window access
  required. Worth doing because it is *less* work than the GNOME path once the capture and
  overlay-compositing code exists, but it serves fewer users, so it waits.
- **Weston and other non-layer-shell, non-GNOME compositors.** No general client mechanism;
  would each need bespoke compositor integration. Low priority.
- **Flatpak/Snap-confined target apps.** Sandboxing further constrains capture/overlay for
  confined apps; not a priority surface.

### macOS — what carries over and what changes

macOS is not yet built. Every render-path capability is gated behind permissions the OS
deliberately keeps under user control. Unlike Linux, those permissions can be *locked*
against the protected user through the account model, which is what makes a tamper-resistant
configuration possible.

**Carries over (Layer 1).** The proxy and the Chrome/Firefox extension are portable; the CA
installs into the System keychain (`security add-trusted-cert`, admin-gated). The one gap is
**Safari**: Safari Web Extensions must ship inside a notarized app distributed through the
App Store, so Safari coverage may have to lean on the proxy alone rather than an extension.
The text-policy, OCR, image ML, and mode-to-response engines are untouched.

**The deferred injection optimization is impossible here — which is fine, since it is not in
the core model.** Injection is harder on macOS than anywhere: SIP ignores
`DYLD_INSERT_LIBRARIES` for Apple binaries, and the hardened runtime (every notarized
third-party app) blocks all `DYLD_*` injection and validates loaded libraries; Mach/
`task_for_pid` needs a non-hardened target plus SIP disabled. All native-app coverage is
Layer 2 (capture). The Accessibility (AX) API is the right substitute for OCR where deep text
extraction is wanted — it reads on-screen text without injection, behind the Accessibility
permission, but is inconsistent across apps so it supplements rather than replaces capture.

**Layer 2 changes mechanically.** Conceptually identical (capture → two models → mode-driven
response); every mechanism is an Apple API and every one is gated by **TCC** permissions the
user must grant explicitly:

- **Capture:** `ScreenCaptureKit` (`SCStream`). The older `CGWindowListCreateImage` is
  obsoleted in macOS 15, and the modern API requires the **Screen Recording** permission.
- **Overlay:** a borderless, transparent, high-level `NSWindow` with `ignoresMouseEvents`,
  which *can* float over other apps. Covering a native-fullscreen Space needs
  `collectionBehavior` (`.canJoinAllSpaces` / `.fullScreenAuxiliary`).
- **Fullscreen:** macOS fullscreen is a Space; force-exit is possible via the AX
  `AXFullScreen` attribute (Accessibility permission), but as on Wayland a compositor-level
  cover over the Space is usually enough — the Windows force-windowed step is rarely needed.
- **Event hooks:** `AXObserver` for window lifecycle/geometry, `CGEventTap` for scroll —
  both behind Accessibility / Input Monitoring permissions.

#### Tamper resistance — the standard-user + partner-held-admin model

This is the part that distinguishes macOS from Linux: the permissions can be locked against
the protected user. Since Big Sur, Screen Recording is a **system-wide service only an
administrator may modify** —
a standard user hitting the padlock must enter admin credentials to enable *or* disable it.
The model is therefore:

> The protected person runs as a **standard user**; the accountability partner (or parent)
> holds the **admin password**. The daemon's Screen Recording and Accessibility grants are
> made once with the admin credential, after which the standard user cannot revoke them.

This is the literal parental-controls pattern and it needs **no MDM**. MDM is only a partial
help here, with an asymmetry that matters: a PPPC profile *can* pre-grant and fully lock
**Accessibility** (it moves to the `MDMOverrides` store, invisible to the user), but **no MDM
can force-grant Screen Recording** — Apple reserves that for user consent and always permits
revocation. Since the capture pipeline depends on Screen Recording, the account model — not
MDM — is what actually locks the critical permission. MDM, if used, only hardens Accessibility
on top.

| Permission | Lock mechanism |
|---|---|
| Screen Recording (capture) | Standard-user account; admin password held by partner. MDM cannot force this. |
| Accessibility (text / events / fullscreen) | Same admin-auth lock, or MDM PPPC (stronger — fully hidden from the user). |

Caveats, stated plainly:

1. The protected user must **give up local admin** to a partner — most Mac users are their
   own sole admin, so this is a deliberate setup with the same friction as any Screen Time /
   parental-controls arrangement.
2. It resists the *standard user*, not an admin — anyone with the admin password can undo it,
   exactly as an admin can stop the Windows service.
3. Physical-access resets (Recovery mode, password reset) remain an escape hatch unless
   FileVault plus a recovery lock are also configured — the same class of out-of-scope risk
   as Windows boot-from-external-media.

**Net macOS assessment.** Layer 1 carries over (minus Safari's extension). The injection
optimization is impossible, but it is not in the core model anyway. Layer 2 is fully
buildable on Apple APIs but always permission-gated; the standard-user + partner-held-admin
configuration locks those permissions and yields a tamper-resistant setup without MDM, while
MDM optionally hardens Accessibility further.

### Android — the layer order inverts

Android is not yet built, and it departs from the desktop model more than any other target.
The change is structural: the network layer (Layer 1) largely collapses, leaving the render
path (Layer 2) to carry essentially everything — which is exactly why building Layer 2 first
on desktop pays off here. We do **not** plan to distribute through the Play Store, which
removes the policy constraint that otherwise dominates Android content-control apps and makes
sideloading the assumed channel.

**Layer 1 collapses to packet filtering.** A normal app can capture all traffic via
`VpnService` (TUN interface, no root, one-time consent) — but it cannot terminate TLS. Since
Android 7, apps targeting API 24+ trust only the **system** certificate store; a user-installed
CA lands in the user store, which apps (including Chrome) ignore unless they opt in via
network-security-config, and almost none do. Installing into the system store needs root or a
custom ROM. So **there is no general MITM on stock Android** — the `scan_url` / `scan_body`
content proxy does not work. What survives at the VPN layer is only what is visible without
decryption: **DNS filtering, SNI inspection** (net-shield already parses SNI; eroded over time
by Encrypted Client Hello), and **IP/port blocking** — i.e. net-shield's Phase 1, not the
content proxy. The browser extension survives only on **Firefox for Android**; Chrome on
Android has no extensions.

**Layer 2 carries everything, and maps cleanly.** All content analysis moves onto the render
path, built from three no-root Android primitives:

- **Capture:** `MediaProjection` (per-session user consent) → feeds OCR + image ML.
- **Text, events, actions:** `AccessibilityService` — the UIA/AX/AT-SPI analogue. Reads
  on-screen text from other apps (a non-OCR shortcut for the text path), receives
  window-change events, and can perform global actions (back/home) to navigate away from an
  offending app. This is the workhorse layer on Android.
- **Overlay:** `SYSTEM_ALERT_WINDOW` ("display over other apps") — the cover mechanism.

**Sideload friction is a feature here, not a bug.** On Android 13+, sensitive permissions —
**Accessibility** and **Device Admin**, the two this design relies on — are placed behind
**Restricted Settings** for any app not installed from the Play Store. The first attempt to
enable the service is blocked; the user must open App Info → ⋮ → "Allow restricted settings"
and authenticate with the device PIN/biometric before the toggle works. This is a one-time,
per-app, authenticated detour, and it aligns with the accountability model: if the partner
holds the device PIN, the protected user cannot clear Restricted Settings or later disable the
service without them — the Android analogue of the macOS admin-held-credential lock.

**Tamper resistance.** Device Admin / Device Owner can prevent uninstall and lock settings;
the daemon can also detect the VPN or Accessibility service being switched off. Device Owner
gives the strongest hold but requires fresh-device provisioning. Combined with a partner-held
device PIN (gating Restricted Settings), this reaches a practical "the user cannot quietly
disable it" posture.

**Net Android assessment.** Layer 1 degrades to DNS/SNI/IP filtering (no MITM); Layer 2
(`MediaProjection` + `AccessibilityService` + overlay) carries all content analysis. Sideloading
is assumed and its Restricted-Settings friction reinforces rather than blocks the accountability
model. The remaining unknowns are OEM variation in the enable-Accessibility flow and the
durability of Device Owner provisioning.

### iOS — the project's core cannot run; only the sanctioned path remains

iOS is the one target where the central premise of this project — our **own** on-device
classifier inspecting arbitrary rendered content — is not implementable at all. The wall is
the sandbox, and there is no sideload escape hatch as on Android: a third-party app on iOS
**cannot read another app's screen content, cannot draw over another app, and has no
cross-app accessibility-read API.** There is no `MediaProjection`, no `SYSTEM_ALERT_WINDOW`,
no `AccessibilityService` equivalent. **The entire render path — and with it OCR, the image
ML model, and the overlay UX — has no analogue on iOS.** This is not a degraded tier; the
content-classification engine simply has nothing it is permitted to see.

What remains is the network path and Apple's own sanctioned content-control surface:

- **Network, consumer:** an `NEPacketTunnelProvider` VPN captures traffic with no root —
  but, as on Android, it cannot terminate TLS for arbitrary apps, so it is limited to
  **DNS / SNI / IP** filtering (net-shield Phase 1). No MITM, no `scan_body`.
- **Network, FamilyControls-gated content filter:** with `FamilyControls` authorization, the
  app may bundle an `NEFilterDataProvider` **content filter** that installs automatically and
  **cannot be removed** — on a *consumer* device, no supervision required. This makes per-flow
  allow/deny decisions (hostname/SNI visible; payload still encrypted), so net-shield-style
  domain and SNI logic can live here, but there is still no decrypted body to classify.
  (Outside FamilyControls, `NEFilterDataProvider` is **supervised/MDM-only** — not a consumer
  path.)
- **Content blocking via Apple's enforcement:** `ManagedSettings` can **shield apps** and
  apply a **web content filter** (Apple's adult-content category, or an allow/deny domain
  list) in Safari and WebKit apps. The classification here is **Apple's**, category-based —
  we configure policy, the OS enforces. Our text-policy and image ML do not participate.

**Tamper resistance is, paradoxically, the strongest of any platform.** `FamilyControls`
requires guardian approval (a parent Apple ID / Screen Time passcode) to authorize, and once
authorized the app **cannot be uninstalled without that approval**, and the bundled content
filter cannot be removed. This is a first-class OS guarantee, not a bolted-on account trick
like the macOS admin split or the Android device-PIN gate — but it only protects the
Apple-enforced policies, since those are the only thing running.

**Net iOS assessment.** The render path is impossible and the custom classification pipeline
cannot run; an iOS build is necessarily a **different, lesser product** — a policy front-end
over Apple's Screen Time / web-content-filter enforcement, plus net-shield-style DNS/SNI/flow
filtering via a FamilyControls content filter. It has the best tamper resistance and the
cleanest non-removable install of any platform, and the least content intelligence. Whether
that reduced product is worth shipping is a product decision, deferred — but the technical
ceiling is fixed and recorded here so it is not rediscovered later.

---

## Open questions

- Scroll-delta OCR debounce threshold (~150 ms) — validate against real apps on slow
  hardware.
- Whether the deferred Windows injection optimization is ever worth building, given Layer 2
  capture already covers its surfaces — and only revisit the known-hardened-process list
  (static vs dynamic AppContainer/ACG detection) if it is.
- GNOME Shell extension maintenance — versioning the extension against GNOME Shell
  releases, and how to degrade if it is disabled or incompatible (Layer 1 still runs).
- Whether the wlroots/KDE layer-shell path is worth building immediately after the two
  primary Linux paths, given it reuses the portal-capture code and is less work than the
  GNOME extension but serves fewer users.
- macOS tamper resistance — whether to require the standard-user + partner-held-admin setup
  during onboarding (and how to detect/warn when the protected user is themselves a local
  admin, which defeats the lock). Whether to additionally ship an MDM PPPC profile to harden
  Accessibility, or leave that to enterprise deployments only.
- Android onboarding — the enable-Accessibility / "Allow restricted settings" flow varies by
  OEM (Samsung One UI, Xiaomi HyperOS, etc.); onboarding needs per-OEM guidance, and testing
  must use the real tap-to-install path (ADB-installed apps are exempt from Restricted
  Settings and would hide the friction real users hit).
- Android tamper resistance — whether to require Device Owner provisioning (strong but needs
  a fresh-device setup) or rely on Device Admin + partner-held device PIN.
- iOS product decision — whether a content-intelligence-free build (Apple Screen Time / web
  content filter policy front-end + FamilyControls DNS/SNI flow filtering) is worth shipping
  at all, given it cannot run the text-policy or image ML engines that define the product
  elsewhere. The technical ceiling is fixed; only the product call is open.
- FamilyControls distribution — **decided:** iOS is the one platform that must ship through
  the App Store. The `FamilyControls` / `NEFilterDataProvider` entitlements require Apple
  approval and store distribution, and there is no sideload alternative — so iOS is an
  explicit exception to the not-launching-in-stores stance taken for desktop and Android.
  This is the only way iOS ships at all; the alternative is no iOS build.
