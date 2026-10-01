# 6. Firefox NSS trust — `FirefoxTrust.swift`

Part of the [macOS Daemon plan](../plan.md).

Firefox keeps its own NSS trust store and ignores the System keychain, so `CATrust` alone leaves
every Firefox user staring at certificate errors. The `Certificates` → `ImportEnterpriseRoots`
enterprise policy sets `security.enterprise_roots.enabled`, which makes Firefox read the platform
store. It is supported on Windows and macOS only.

An earlier draft of this section named
`/Library/Application Support/Mozilla/Certificates / policies.json` as the policy location. **No
such path exists.** Mozilla documents exactly two delivery mechanisms on macOS:

1. **The managed-preferences domain `org.mozilla.firefox`** — what this module uses. Policy keys
   sit at the top level of the domain: `EnterprisePoliciesEnabled` as a boolean, and `Certificates`
   as a nested dictionary containing `ImportEnterpriseRoots`.
2. **`Firefox.app/Contents/Resources/distribution/policies.json`** — rejected. Writing inside the
   bundle breaks the notarized app's code-signature seal, and every Firefox update replaces the
   bundle and silently drops the policy.

Reads and writes go through `CFPreferences`, not the plist file, because that is the API Firefox
itself reads with; writing the file directly races `cfprefsd`, which caches the domain and can
serve or write back a stale copy. `kCFPreferencesAnyUser` + `kCFPreferencesAnyHost` is the pair
that maps to `/Library/Preferences/org.mozilla.firefox.plist` — `kCFPreferencesCurrentHost` would
land in `/Library/Preferences/ByHost/` under a hardware UUID instead. Writing requires root, which
suits the tamper model: the protected user cannot revoke it unprivileged.

The merge behaviour is the part worth testing, and it mirrors the third-party-proxy concern
recorded below. An MDM-managed Mac may already carry a Firefox policy payload, so `install()`
merges into whatever is there and `uninstall()` removes only `ImportEnterpriseRoots` — dropping the
`Certificates` dictionary only if it empties, and `EnterprisePoliciesEnabled` only if no other
policy still depends on it.

Direct NSS import into each profile with `certutil` remains the fallback for versions that ignore
the policy. It is fragile — it requires locating every profile and a tool Firefox does not install
— and is not built.

## This module is not needed for coverage, and cannot work the way it is written

Two findings, both from reading the shipped Firefox 152.0.5 build rather than reasoning about it.
Together they retire the premise this step was written on.

**1. Firefox already trusts System-keychain roots.** `security.enterprise_roots.enabled` is a
`StaticPref` with a default of `true`. There is nothing to turn on. The original claim at the top of
module 2 — that Firefox ignores the System keychain and therefore needs a separate step — has been
obsolete for several releases.

**2. Firefox deliberately ignores this exact policy when it stands alone.** From
`modules/EnterprisePoliciesParent.sys.mjs` inside `Firefox.app/Contents/Resources/omni.ja`:

```js
// Because security.enterprise_roots.enabled is true by default, we can
// ignore attempts by Antivirus to try to set it via policy.
if (
  Object.keys(provider.policies).length === 1 &&
  provider.policies.Certificates &&
  Object.keys(provider.policies.Certificates).length === 1 &&
  (provider.policies.Certificates.ImportEnterpriseRoots === true ||
    provider.policies.Certificates.ImportEnterpriseRoots === 1)
) {
  this.status = Ci.nsIEnterprisePolicies.INACTIVE;
  return;
}
```

A policy set consisting of *only* `Certificates.ImportEnterpriseRoots` short-circuits to INACTIVE
and returns before `_activatePolicies` — so the policy is genuinely not applied, not merely
reported oddly. **To make `ImportEnterpriseRoots` take effect via policy you must ship at least one
other policy alongside it.** There is no warning; `about:policies` just says the service is
inactive.

Note what this does *not* mean. `forced`-ness was a dead end: hand-creating
`/Library/Managed Preferences/org.mozilla.firefox.plist` does flip
`CFPreferencesAppValueIsForced` to true (measured), and Firefox still reported inactive, because the
short-circuit above is the real gate. Mozilla's KB advertising
`sudo defaults write /Library/Preferences/org.mozilla.firefox ...` is fine as far as it goes.

### What the module is actually for

Keep it, but on a different justification. It is useless as "make Firefox trust the CA" and
potentially useful as **"re-assert the pref if something turns it off"** — a user or an antivirus
setting `security.enterprise_roots.enabled` to false is a plausible evasion of the whole render-path
model. That is a tamper-model concern, not a coverage one, and any implementation of it has to
carry a companion policy or hit the short-circuit above.

### Verifying this without a human

`about:policies` is a GUI page, but Firefox renders it headlessly, which makes this checkable from
CI or an agent:

```
/Applications/Firefox.app/Contents/MacOS/firefox --headless --new-instance \
  -profile <tmp-profile> --window-size 1200,900 \
  --screenshot <out.png> about:policies
```

Use a throwaway `-profile` so a running Firefox is undisturbed. The shipped policy logic itself is
readable without launching anything — `omni.ja` is a plain zip:

```
unzip -p /Applications/Firefox.app/Contents/Resources/omni.ja \
  modules/EnterprisePoliciesParent.sys.mjs
```

Prefer that to reading `mozilla-central`, which is several versions ahead of any installed build and
had already refactored this code path.

## Reference documents

- [Firefox enterprise policy reference — Certificates](https://firefox-admin-docs.mozilla.org/reference/policies/certificates/)
  — `ImportEnterpriseRoots` semantics, platform support, and the preference it maps to.
- [Mozilla policy templates — `mac/org.mozilla.firefox.plist`](https://github.com/mozilla/policy-templates/blob/master/mac/org.mozilla.firefox.plist)
  — the authoritative plist shape; confirms the keys are top-level with no wrapping container.
- [Apple — `CFPreferences`](https://developer.apple.com/documentation/corefoundation/preferences_utilities)
  — the user/host domain pairs and which file each resolves to.
