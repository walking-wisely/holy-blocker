#!/usr/bin/env bash
# Create (or rotate) the self-signed code-signing identity used to sign
# HolyBlockerDaemon.app for local development.
#
# Why this exists: TCC keys permission grants to the app's code-signing identity,
# not its path or contents. An ad-hoc signature (the default for a plain SwiftPM
# build) is derived from the binary's cdhash and changes on every rebuild, so
# every grant dies with the next `swift build`. A real certificate's identity is
# stable across rebuilds as long as the *same* certificate keeps signing the
# bundle — see docs/components/mac-daemon/signing-identity.md for the full
# runbook and docs/components/mac-daemon/plan.md module 0 for why this had to
# exist before any other Layer 2 module could be verified.
#
# This is a development stand-in, not a distribution identity. It is
# self-signed, trusted only on this machine, and will not survive Gatekeeper or
# notarization. See the runbook for the Developer ID path required before
# shipping to anyone else.
#
# Usage:
#   scripts/create-dev-signing-identity.sh            # create if absent, else no-op
#   scripts/create-dev-signing-identity.sh --rotate    # delete and recreate
#
# After running, export HOLY_BLOCKER_SIGNING_IDENTITY to the printed name and
# re-run scripts/bundle.sh. Rotating invalidates every existing TCC grant made
# to the old identity — the new certificate is a new client as far as TCC is
# concerned, so Screen Recording / Accessibility must be re-granted.
set -euo pipefail

IDENTITY_NAME="Holy Blocker Dev"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
ROTATE=0

for arg in "$@"; do
  case "$arg" in
    --rotate) ROTATE=1 ;;
    *) echo "create-dev-signing-identity.sh: unknown argument $arg" >&2; exit 1 ;;
  esac
done

existing="$(security find-identity -v -p codesigning "$KEYCHAIN" 2>/dev/null | grep -F "\"$IDENTITY_NAME\"" || true)"

if [[ -n "$existing" && "$ROTATE" -eq 0 ]]; then
  echo "already present: $existing"
  echo "set HOLY_BLOCKER_SIGNING_IDENTITY=\"$IDENTITY_NAME\" and run scripts/bundle.sh"
  exit 0
fi

if [[ -n "$existing" && "$ROTATE" -eq 1 ]]; then
  echo "rotating: removing existing \"$IDENTITY_NAME\" from $KEYCHAIN"
  echo "this invalidates every TCC grant made to the old certificate."
  sha1="$(echo "$existing" | sed -n 's/^ *[0-9]*) \([0-9A-F]*\).*/\1/p')"
  security delete-identity -Z "$sha1" "$KEYCHAIN"
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

openssl req -x509 -newkey rsa:2048 \
  -keyout "$workdir/dev.key" -out "$workdir/dev.crt" \
  -days 3650 -nodes \
  -subj "/CN=$IDENTITY_NAME" \
  -addext "extendedKeyUsage=codeSigning" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "basicConstraints=critical,CA:true"

p12_pass="$(openssl rand -base64 24)"
openssl pkcs12 -export \
  -inkey "$workdir/dev.key" -in "$workdir/dev.crt" \
  -out "$workdir/dev.p12" -passout "pass:$p12_pass"

security import "$workdir/dev.p12" -k "$KEYCHAIN" -P "$p12_pass" -T /usr/bin/codesign -A
security add-trusted-cert -p codeSign -k "$KEYCHAIN" "$workdir/dev.crt"

echo
security find-identity -v -p codesigning "$KEYCHAIN" | grep -F "\"$IDENTITY_NAME\""
echo
echo "created. set HOLY_BLOCKER_SIGNING_IDENTITY=\"$IDENTITY_NAME\" and run scripts/bundle.sh"
[[ "$ROTATE" -eq 1 ]] && echo "remember: re-grant Screen Recording / Accessibility — the old grants are gone."
exit 0
