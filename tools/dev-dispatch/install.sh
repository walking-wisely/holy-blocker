#!/bin/bash
set -euo pipefail
PATH=/usr/bin:/bin:/usr/sbin:/sbin

readonly LIBEXEC=/usr/local/libexec/holy-blocker
readonly DROPIN=/etc/sudoers.d/holy-blocker
readonly IDENTITY="Holy Blocker Dev"

valid_username() {
  [[ "$1" =~ ^[a-z_][a-z0-9_-]*$ && "$1" != ALL ]]
}

render_dropin() {
  local source_dir="${2:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
  sed "s/__USER__/$1/" "$source_dir/holy-blocker.sudoers.in"
}

identity_certificate_pem() {
  local user="$1" home="$2" keychain matches
  keychain="$home/Library/Keychains/login.keychain-db"
  matches="$(sudo -u "$user" security find-identity -v -p codesigning "$keychain" | grep -cF "\"$IDENTITY\"" || true)"
  [[ "$matches" -eq 1 ]] || return 1
  local pem subject
  pem="$(sudo -u "$user" security find-certificate -c "$IDENTITY" -p "$keychain")"
  [[ "$(grep -c 'BEGIN CERTIFICATE' <<<"$pem")" -eq 1 ]] || return 1
  subject="$(openssl x509 -noout -subject -nameopt RFC2253 <<<"$pem" | sed 's/^subject= *//')"
  [[ "$subject" == "CN=$IDENTITY" ]] || return 1
  echo "$pem"
}

scratch=""

install_all() {
  [[ "$EUID" -eq 0 && -n "${SUDO_USER:-}" ]] || { echo "run with sudo" >&2; exit 77; }
  valid_username "$SUDO_USER" || { echo "unsupported user name" >&2; exit 65; }
  local here home pin
  here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  unset TMPDIR
  scratch="$(mktemp -d)"
  trap 'rm -rf "${scratch:-}"' EXIT
  cp "$here/dispatch" "$here/holy-blocker.sudoers.in" "$scratch/"
  home="$(dscl . -read "/Users/$SUDO_USER" NFSHomeDirectory | sed 's/^NFSHomeDirectory: //')"
  local pem
  pem="$(identity_certificate_pem "$SUDO_USER" "$home")" \
    || { echo "expected exactly one \"$IDENTITY\" code-signing identity in the login keychain" >&2; exit 65; }
  pin="$(openssl x509 -outform DER <<<"$pem" | shasum -a 256 | cut -d' ' -f1)"
  echo "identity to pin:"
  openssl x509 -noout -subject -dates <<<"$pem"
  echo "sha256: $pin"
  echo "dispatch sha256:        $(shasum -a 256 "$scratch/dispatch" | cut -d' ' -f1)"
  echo "sudoers template sha256: $(shasum -a 256 "$scratch/holy-blocker.sudoers.in" | cut -d' ' -f1)"
  local answer
  read -r -p "Install and pin this identity? [y/N] " answer </dev/tty
  [[ "$answer" == y || "$answer" == Y ]] || { echo "aborted" >&2; exit 1; }

  render_dropin "$SUDO_USER" "$scratch" >"$scratch/dropin"
  visudo -cf "$scratch/dropin" >/dev/null

  install -d -m 755 -o root -g wheel "$LIBEXEC"
  install -m 755 -o root -g wheel "$scratch/dispatch" "$LIBEXEC/dispatch"
  printf '%s\n' "$pin" >"$LIBEXEC/trusted-cert.sha256"
  chown root:wheel "$LIBEXEC/trusted-cert.sha256"
  chmod 644 "$LIBEXEC/trusted-cert.sha256"
  install -m 440 -o root -g wheel "$scratch/dropin" "$DROPIN"

  echo "installed $LIBEXEC/dispatch"
  echo "pinned certificate sha256: $pin"
  echo "sudoers drop-in: $DROPIN"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  install_all
fi
