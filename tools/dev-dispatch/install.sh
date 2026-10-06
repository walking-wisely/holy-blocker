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
  local here
  here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  sed "s/__USER__/$1/" "$here/holy-blocker.sudoers.in"
}

pinned_certificate_hash() {
  sudo -u "$1" security find-certificate -c "$IDENTITY" -p "$2/Library/Keychains/login.keychain-db" \
    | openssl x509 -outform DER | shasum -a 256 | cut -d' ' -f1
}

install_all() {
  [[ "$EUID" -eq 0 && -n "${SUDO_USER:-}" ]] || { echo "run with sudo" >&2; exit 77; }
  valid_username "$SUDO_USER" || { echo "unsupported user name" >&2; exit 65; }
  local here home pin scratch
  here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  home="$(dscl . -read "/Users/$SUDO_USER" NFSHomeDirectory | sed 's/^NFSHomeDirectory: //')"
  pin="$(pinned_certificate_hash "$SUDO_USER" "$home")"
  [[ "$pin" =~ ^[0-9a-f]{64}$ ]] || { echo "no \"$IDENTITY\" certificate in the login keychain" >&2; exit 65; }

  scratch="$(mktemp -d)"
  render_dropin "$SUDO_USER" >"$scratch/dropin"
  visudo -cf "$scratch/dropin" >/dev/null

  install -d -m 755 -o root -g wheel "$LIBEXEC"
  install -m 755 -o root -g wheel "$here/dispatch" "$LIBEXEC/dispatch"
  printf '%s\n' "$pin" >"$LIBEXEC/trusted-cert.sha256"
  chown root:wheel "$LIBEXEC/trusted-cert.sha256"
  chmod 644 "$LIBEXEC/trusted-cert.sha256"
  install -m 440 -o root -g wheel "$scratch/dropin" "$DROPIN"
  rm -rf "$scratch"

  echo "installed $LIBEXEC/dispatch"
  echo "pinned certificate sha256: $pin"
  echo "sudoers drop-in: $DROPIN"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  install_all
fi
