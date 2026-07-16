# iOS — Platform Feasibility Notes

**Status: shelved.** No iOS package exists and none is planned near-term. Development is blocked on hardware — see [Development is device-only](#development-is-device-only).

This is not an implementation plan. It records what iOS does and does not permit, so the constraints do not have to be re-derived when the work is picked up. Nothing here has been verified against a physical device; items flagged **Unverified** need confirming before anything is designed around them.

## The one-line summary

iOS is not a scaled-down port of the desktop architecture — it is a different product. Desktop inspects traffic and decides. iOS configures Apple's blocker and locks the door. `mitm-proxy` never ships, and `text-policy` only runs as a live scorer inside a browser we render ourselves.

## No traffic decryption, at any tier

There is no path to plaintext on iOS. Not via VPN, not via the content filter API, not via full MDM supervision. This is the constraint everything else follows from.

A `NEPacketTunnelProvider` sees raw IP packets:

- destination IP
- the TLS ClientHello, and therefore the **SNI** — hostname only
- DNS queries, *if* the app uses the system resolver
- packet sizes and timing

It never sees URLs, paths, headers, cookies, or bodies. `https://example.com/a` and `https://example.com/b` are indistinguishable.

MITM is not a workaround. It requires the user to install a `.mobileconfig`, then *separately* enable the CA under Settings → General → About → Certificate Trust Settings. Certificate-pinned apps (most large apps, all of Apple's) then fail to connect rather than being decrypted — the phone breaks instead of being filtered. App Review is very unlikely to pass an app shipping a root CA for interception. **Treat MITM as off the table.**

Two further erosions of the SNI path, worth accounting for as it is a shrinking surface rather than a stable one:

- **Encrypted Client Hello** encrypts the SNI, and is on by default in Safari when the connection uses DoH.
- Apps running their own DoH resolver (Chrome, Firefox) remove DNS visibility too.

### Why request mirroring does not work

The idea of duplicating the device's GETs out-of-band to scan page text fails before the privacy question arises: **we never have the URL** — only the SNI. There is no request to replay. It also has no access to Safari's cookie jar (sandboxed; only `WKWebView` stores we own are reachable), and replaying authenticated GETs is unsafe in practice — logout links, one-time tokens, read receipts, doubled traffic, anti-abuse tripwires. Rejected.

## Family Controls is the load-bearing dependency

Family Controls buys **lock, not sight**. It grants no decryption. What it does unlock:

| Capability | Without Family Controls | With it |
|---|---|---|
| App shielding (`ManagedSettings`) | Impossible | Yes |
| `NEFilterDataProvider` | Needs supervision | Yes |
| Per-app flow attribution | No | Via `sourceAppIdentifier` |
| Tamper resistance | None | `.child` authorization |
| `SensitiveContentAnalysis` | Policy likely `.disabled` | Likely active (**Unverified**) |

- **`NEFilterDataProvider`** is not a decrypting filter. `handleOutboundData` hands back the TLS record stream — as encrypted as what the tunnel saw. Its advantages over a packet tunnel are position (below the VPN slot, so another VPN cannot dodge it) and per-app attribution. Same visibility ceiling.
- **`ManagedSettings` shields** invert the model: we do not inspect, Apple blocks. `WebContentSettings.filterAutomatically` enables Apple's own adult-content filter; `.specific(blocked:)` takes our domain list. Apple's filter reaches into WebKit in ways we never will, so delegating beats anything buildable in a tunnel for Safari and WebKit apps.
- **`.child` vs `.individual`** are different products on the same entitlement. `.individual` is self-control — the user authorizes themselves and can revoke (Opal's model, usually hardened with a Screen Time passcode). `.child` requires a parent's Apple ID to revoke; this is the accountability model and the only real tamper resistance iOS offers.

`ApplicationToken`s come only from `FamilyActivityPicker`, which requires authorization — so the dependency chain for app blocking is airtight by design. iOS also exposes **no foreground-app API**; `canOpenURL` reports installation, not use, and is capped at 50 declared schemes. Without the entitlement we cannot even observe an app, let alone shield it.

Nothing on iOS is "indestructible" in the Android Device Admin sense. The honest framing is *high friction, requires a second person*. A user holding the Screen Time passcode always wins.

## Content access requires being the renderer

There is no interception path to content — only a rendering path.

**The architecture that works:** ship our own `WKWebView` browser, then use `ManagedSettings` to shield Safari, Chrome, Firefox, and the browser category. Ours is the only one left standing. (This is what Canopy and Bark do.) Inside it we have `decidePolicyFor`, `WKUserScript` injection at `document_start`, `WKContentRuleList`, native message handlers, and full DOM text — so `text-policy` becomes a real runtime scorer again.

**Hide-scan-reveal eliminates the exposure window.** Owning the renderer means gating the paint rather than reacting to it: inject CSS at `document_start` hiding images, collect the `src` list, classify natively, reveal what passes. Same for text — hide `body`, extract, score, reveal. Cost is latency, not exposure. This is strictly better than downloading and scanning after display.

**`SensitiveContentAnalysis`** (iOS 17+, `SCSensitivityAnalyzer`) is Apple's on-device nudity classifier — no model to ship or train. `analysisPolicy` returns `.disabled` unless Sensitive Content Warnings or Communication Safety is enabled; under `.child` these should be on. If that holds, it removes a large part of the `image-sandbox` port. **Unverified** — check the entitlement requirements and the policy behavior on-device.

**Fallback if shipping a browser is too much:** a Safari Web Extension (iOS 15+) gets content scripts with real DOM access inside actual Safari, same hide-scan-reveal trick. But it needs per-site permission, the user can disable it, and Family Controls cannot force it on — weaker on both coverage and tamper resistance.

## What ships without the entitlement

Only two things, and they degrade honestly:

- **Safari Content Blocker** (`WKContentRuleList`). No entitlement, no review friction. It sees *more* than the VPN — Safari terminates TLS itself and applies rules to the real URL. Limits: Safari/WKWebView only, declarative JSON compiled ahead of time (~150k rule cap), no callback into our code, so `text-policy` acts as a **rule compiler** emitting blocker JSON at build time rather than scoring at runtime.
- **Local VPN** — `NEPacketTunnelProvider`, or the lighter `NEDNSSettingsManager` for a DoH resolver. Domain/IP blocking across all apps, blind above the hostname. The Network Extension capability is self-serve in the developer portal; App Review will ask what it is for, but that is a review conversation, not an entitlement request.

Pseudo-app-blocking by starving an app of its domains works, but is fragile: shared CDNs cause collateral damage, IPs churn, the per-app endpoint list needs perpetual maintenance, and without `sourceAppIdentifier` we are blocking domains we *believe* belong to that app, blind.

It is all moot for accountability regardless — without Family Controls the user disables the VPN in two taps. This tier is a demo, not a blocker.

## Shipping sequence

The distribution entitlement is the gate, and it is **separate from the development one**:

- `com.apple.developer.family-controls` can be added in Xcode and built against **today**, no approval required.
- *Distributing* it requires a manual entitlement request. Apple is conservative and expects a genuine parental-control or accountability product with a plausible operator model.

So the sequence is: ship v1 with blocker + VPN only (no entitlement, no review risk), request the entitlement in parallel, light up the shield tier in v2. Two-mode support falls out of this naturally — authorization is requested at runtime via `AuthorizationCenter.shared.requestAuthorization(for:)`, and when it is absent we simply do not touch `ManagedSettings`. Holding the entitlement forces nothing at launch.

**The real first question is not architectural.** It is whether Apple grants the entitlement. Everything above hangs off it. Worth finding out before designing much.

## Development is device-only

**Neither half of this works on the Simulator.** Family Controls authorization requests fail, shields do not apply, `FamilyActivityPicker` tokens are meaningless, `DeviceActivity` does not run — and `NEPacketTunnelProvider` does not work there either. A physical device is a hard prerequisite. *This is why the work is shelved.*

Testing `.child` additionally needs a real Family Sharing group with a child Apple ID under an organizer, which requires the organizer's payment method on file. There is no sandbox for this. Set up a spare device and a test family **before** writing much code — it is an afternoon's work, better done early than discovered mid-sprint.

## Impact on existing packages

| Package | iOS story |
|---|---|
| `net-shield` | The reusable piece. Radix domain/IP filter and SNI parser are exactly the iOS ceiling. |
| `text-policy` | Rule compiler at the shield tier; real runtime scorer only inside our own browser. |
| `image-sandbox` | Possibly replaced by `SensitiveContentAnalysis` under `.child`. |
| `mitm-proxy` | No story. Never ships on iOS. |
| `video-watchdog` | No story — depends on the MITM byte stream. |

Open question for whenever this resumes: how much of `text-policy` survives as a shared Rust core over FFI versus being rewritten in Swift.

## Unverified claims

Confirm on a physical device before relying on any of these:

1. Whether revoking an app's Screen Time access under `.individual` is itself gated by the Screen Time passcode. Load-bearing for how strong self-control mode actually is, and behavior has shifted across iOS versions.
2. Whether `SCSensitivityAnalyzer`'s `analysisPolicy` is reliably active under `.child`, and what entitlement it requires.
3. `NEFilterFlow.url` — it appears to hand over full URLs, but is believed to be populated only for some flows and unreliable in practice. If it were dependable it would partly undercut the "no content without rendering" conclusion, so it is worth checking properly rather than assuming either way.

## Reference documents

- [Family Controls](https://developer.apple.com/documentation/familycontrols) — `AuthorizationCenter`, `FamilyActivityPicker`, `.child` vs `.individual`
- [ManagedSettings](https://developer.apple.com/documentation/managedsettings) — shields, `WebContentSettings`
- [DeviceActivity](https://developer.apple.com/documentation/deviceactivity)
- [Network Extension](https://developer.apple.com/documentation/networkextension) — `NEPacketTunnelProvider`, `NEFilterDataProvider`, `NEDNSSettingsManager`
- [NEFilterFlow](https://developer.apple.com/documentation/networkextension/nefilterflow) — flow metadata surface, `url` property
- [SensitiveContentAnalysis](https://developer.apple.com/documentation/sensitivecontentanalysis)
- [Creating a content blocker](https://developer.apple.com/documentation/safariservices/creating-a-content-blocker) — `WKContentRuleList` rule format and limits
- [Safari web extensions](https://developer.apple.com/documentation/safariservices/safari-web-extensions)
- [WKWebView](https://developer.apple.com/documentation/webkit/wkwebview) — `WKUserScript`, `decidePolicyFor`
- [TLS 1.3 — RFC 8446 §4.1.2](https://www.rfc-editor.org/rfc/rfc8446#section-4.1.2) — ClientHello, SNI placement
- [Encrypted Client Hello draft](https://datatracker.ietf.org/doc/draft-ietf-tls-esni/) — why SNI visibility is eroding
