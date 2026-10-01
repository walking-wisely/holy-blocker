# What Layer 1 does *not* cover on macOS — honest limits

Part of the [macOS Daemon plan](plan.md).

These are coverage gaps inherent to the system-proxy approach and must be recorded, not discovered
later:

- **QUIC / HTTP3 over UDP 443** bypasses an HTTP proxy entirely. Chrome disables QUIC when a system
  proxy is configured, but this is browser policy, not a guarantee. A complete answer needs UDP 443
  blocking, which belongs to the transparent-proxy path below, not here.
- **Apps that ignore system proxy settings** — anything using raw `BSD sockets`, hardcoded
  networking, or its own proxy configuration. CFNetwork/`URLSession` clients and the major browsers
  do honour them; command-line tools generally do not.
- **Certificate pinning** — pinned apps will fail to connect rather than be inspected. The bypass
  list is the mitigation.
- **The protected user can change proxy settings back** unless they are a standard user. This is
  exactly why the tamper model is an account-model question, not a code question — and it is
  sharper than it first appears: **`networksetup` proxy changes require no root at all for an
  admin user** (verified on macOS 26.5; `-setwebproxy` succeeds and mutates from an unprivileged
  shell). Only the CA install and the default state directory need privilege. So on a machine
  where the protected user is a local admin, Layer 1 is trivially reversible by them, and the
  standard-user split is not a hardening option but a prerequisite.

## Snapshot/restore assumes exclusive ownership of the proxy setting

`ProxyConfiguration` reads the prior state once, then writes it back on stop. That is only correct
if nothing else touches proxy settings in between — and on a machine running this software, that
assumption is weaker than usual, because the people who install a content blocker frequently
already run another one.

Three failure modes, none currently handled:

1. **Another tool sets a proxy while ours is applied.** Our `restore()` writes back the value
   captured at start, silently reverting their change.
2. **Another tool owned the proxy when we started.** `apply()` overrides it, disabling *their*
   filtering for as long as we run. For a blocker, quietly turning off a competing blocker is the
   worst possible bug — it is a bypass.
3. **Our snapshot is stale after a crash.** The on-disk snapshot is replayed on next start, which
   may now be older than the user's own settings.

Mitigation when this is built: re-read the current value immediately before restoring and only
write back if it still matches what we applied (compare-and-swap rather than blind restore), and
refuse to `apply()` over a pre-existing third-party proxy without explicit user consent.

Verified on the development machine (macOS 26.5): **Cold Turkey Blocker** is installed and running
there but does *not* use this surface — it blocks through browser extensions via a native
messaging host, leaves `/etc/hosts` untouched, and installs no root CA. So it composes with
Layer 1 rather than conflicting. Tools that *do* take the system proxy (Charles, Proxyman, mitm
tooling, some VPN clients, corporate MDM proxy profiles) will conflict on a last-writer-wins basis.

**A second peer product is present and was missed by that survey.** `systemextensionsctl list` on
the same machine shows `com.Cvnt.ce.FirewallExtension` — **Covenant Eyes** — alongside Tailscale's
network extension. It is a `NetworkExtension` content filter, not a system-proxy consumer, so it
also composes with Layer 1; but it occupies the surface the transparent-proxy path below would want,
and two filters on that surface is a genuine conflict to plan for rather than discover. Two things
follow beyond the conflict question: the leading product in precisely this category chose the
**network** boundary over the exec boundary, which is a strong signal about what is practically
approvable; and it runs there with the user as a local admin, which is the configuration the tamper
model treats as weakened.

## The transparent-proxy alternative, and why it is deferred

`NETransparentProxyProvider` / `NEAppProxyProvider` (a Network Extension system extension) would
capture traffic regardless of app proxy support and is the true macOS analogue of net-shield's
Wintun path. It is deferred because it requires a **gated Apple entitlement**, a signed and
notarized app bundle, and full Xcode — weeks of lead time — whereas the `networksetup` path works
today with zero entitlements and covers the browsers that matter most.

There is a second, strategic reason to build it eventually: `NEFilterDataProvider` on macOS is the
**same API as the iOS content filter**, so the macOS extension is the development and debugging
environment for the iOS build. See the iOS section of
[content-interception.md](../../decisions/content-interception.md).

Note that its entitlement is the **more routinely granted** of the two gated options — it is what
Covenant Eyes ships — so if an entitlement request is going to be made at all, this is the one with
the shorter path and the iOS payoff.

## Endpoint Security, and why it is deferred rather than rejected

`ES_EVENT_TYPE_AUTH_EXEC` is the only sound place to authorize command execution: the client is
called before `execve` completes with the resolved path, full argv and code-signing identity. It sits
below the shell, so quoting, `base64`, variable assembly and `eval` are all already resolved, and it
covers binaries — which no text-inspection scheme can. Confirmed present on macOS 26.5.2 as
`libEndpointSecurity.tbd` with headers under `$SDK/usr/include/EndpointSecurity/`. It also supplies
the self-defence events Android gets from DeviceAdmin: `AUTH_SIGNAL` (deny `SIGKILL`),
`AUTH_UNLINK` / `AUTH_RENAME` (deny removal), `AUTH_GET_TASK` (deny debugger attach).

**Two constraints to record before anyone starts.** First, the entitlement
`com.apple.developer.endpoint-security.client` is Apple-gated and needs a notarized System Extension.
Second, and less obvious: AUTH messages carry a per-message `deadline`, and the header states that a
client failing to answer before it **will be killed** — so a decision cache keyed on the binary hash
(`es_respond_auth_result`'s cache flag, `es_clear_cache`) is mandatory rather than an optimisation.

It is **deferred, not rejected**, because its marginal value over root `LaunchDaemon` + standard user
is narrow: against a standard user those already block the routes ES would block, and against an
admin the extension is itself removable. See
[content-interception.md](../../decisions/content-interception.md) for the full argument, including
why `systemextensionsctl developer on` cannot ship (it needs SIP off, and SIP off makes `TCC.db`
directly writable — a larger hole than the one being closed) and why a kernel extension is strictly
dominated by this path.
