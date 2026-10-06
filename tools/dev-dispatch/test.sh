#!/bin/bash
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
failures=0

check() {
  local name="$1"
  shift
  if "$@"; then echo "ok   $name"; else echo "FAIL $name"; failures=$((failures + 1)); fi
}

not() { ! "$@"; }
in_dispatch() { ( source "$here/dispatch"; "$@" ) >/dev/null 2>&1; }
in_dispatch_out() { ( source "$here/dispatch"; "$@" ) 2>/dev/null; }
in_install() { ( source "$here/install.sh"; "$@" ) >/dev/null 2>&1; }

sandbox="$(realpath "$(mktemp -d)")"
trap 'rm -rf "$sandbox"' EXIT
mkdir -p "$sandbox/home" "$sandbox/elsewhere"
touch "$sandbox/home/file" "$sandbox/elsewhere/file"
mkdir -p "$sandbox/home/HolyBlockerDaemon.app" "$sandbox/home/Other.app"
ln -s "$sandbox/elsewhere/file" "$sandbox/home/link"
CALLER_HOME_FOR_TEST="$sandbox/home"

with_home() {
  ( source "$here/dispatch"; caller_home() { echo "$CALLER_HOME_FOR_TEST"; }; "$@" ) >/dev/null 2>&1
}

usage_ok()  { in_dispatch check_usage "$@"; }
usage_bad() { ! in_dispatch check_usage "$@"; }

check "usage: stage-app takes one path"      usage_ok stage-app /x
check "usage: stage-app without path"        usage_bad stage-app
check "usage: stage-app with extra"          usage_bad stage-app /x /y
check "usage: stage-proxy takes one path"    usage_ok stage-proxy /x
check "usage: ca-install takes none"         usage_ok ca-install
check "usage: ca-install with arg"           usage_bad ca-install /x
check "usage: ca-generate takes none"        usage_ok ca-generate
check "usage: ca-generate with arg"          usage_bad ca-generate /x
check "usage: ca-remove takes none"          usage_ok ca-remove
check "usage: ca-remove with arg"            usage_bad ca-remove /x
check "usage: daemon-bootstrap takes none"   usage_ok daemon-bootstrap
check "usage: daemon-bootout takes none"     usage_ok daemon-bootout
check "usage: status takes none"             usage_ok status
check "usage: revoke takes none"             usage_ok revoke
check "usage: unknown verb"                  usage_bad run-anything
check "usage: empty verb"                    usage_bad ""

die_message="$( ( source "$here/dispatch"; die "boom" 65 ) 2>&1 )"
check "die: message excludes the exit code" test "$die_message" = "dispatch: boom"
"$here/dispatch" bogus >/dev/null 2>&1
check "unknown verb exits 64" test $? -eq 64
"$here/dispatch" status >/dev/null 2>&1
check "valid verb without root exits 77" test $? -eq 77

check "path: file under home accepted"       with_home resolve_user_path "$sandbox/home/file"
check "path: relative rejected"              not with_home resolve_user_path "home/file"
check "path: outside home rejected"          not with_home resolve_user_path "$sandbox/elsewhere/file"
check "path: dotdot escape rejected"         not with_home resolve_user_path "$sandbox/home/../elsewhere/file"
check "path: symlink rejected"               not with_home resolve_user_path "$sandbox/home/link"
check "path: missing rejected"               not with_home resolve_user_path "$sandbox/home/nope"
check "path: newline rejected"               not with_home resolve_user_path "$sandbox/home/file"$'\n'"x"
empty_home() { ( source "$here/dispatch"; caller_home() { echo ""; }; resolve_user_path "$sandbox/elsewhere/file" ) >/dev/null 2>&1; }
root_home() { ( source "$here/dispatch"; caller_home() { echo "/"; }; resolve_user_path "$sandbox/elsewhere/file" ) >/dev/null 2>&1; }
check "path: empty home lookup rejected"     not empty_home
check "path: root as home rejected"          not root_home
check "path: empty rejected"                 not with_home resolve_user_path ""
check "app name: exact name accepted"        in_dispatch require_basename "$sandbox/home/HolyBlockerDaemon.app" HolyBlockerDaemon.app
check "app name: other name rejected"        not in_dispatch require_basename "$sandbox/home/Other.app" HolyBlockerDaemon.app

pin_file="$sandbox/pin"
good_pin="$(printf 'a%.0s' {1..64})"
read_pin_from() { in_dispatch read_pin "$pin_file"; }
echo "$good_pin" > "$pin_file";             check "pin: 64 hex accepted"       read_pin_from
echo "${good_pin:1}" > "$pin_file";         check "pin: short rejected"        not read_pin_from
echo "${good_pin}a" > "$pin_file";          check "pin: long rejected"         not read_pin_from
echo "$(echo "$good_pin" | tr a-f A-F)" > "$pin_file";         check "pin: uppercase rejected"    not read_pin_from
printf '%s\n%s\n' "$good_pin" "$good_pin" > "$pin_file"; check "pin: two lines rejected" not read_pin_from
: > "$pin_file";                            check "pin: empty rejected"        not read_pin_from
check "pin: missing file rejected"          not in_dispatch read_pin "$sandbox/absent"

cat > "$sandbox/ca.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = v3_ca
prompt = no
[dn]
CN = Holy Blocker Local CA
[v3_ca]
basicConstraints = critical,CA:TRUE
CNF
sed 's/Holy Blocker Local CA/Some Other CA/' "$sandbox/ca.cnf" > "$sandbox/other.cnf"
sed 's/CA:TRUE/CA:FALSE/' "$sandbox/ca.cnf" > "$sandbox/leaf.cnf"
make_cert() {
  openssl req -x509 -newkey rsa:2048 -nodes -days 1 -config "$1" \
    -keyout "$sandbox/key.pem" -out "$2" >/dev/null 2>&1
}
make_cert "$sandbox/ca.cnf" "$sandbox/ca.pem"
make_cert "$sandbox/other.cnf" "$sandbox/other.pem"
make_cert "$sandbox/leaf.cnf" "$sandbox/leaf.pem"
cat "$sandbox/ca.pem" "$sandbox/other.pem" > "$sandbox/two.pem"
sed 's/^basicConstraints.*/basicConstraints = critical,CA:FALSE\nnsComment = "CA:TRUE"/' "$sandbox/ca.cnf" > "$sandbox/trick.cnf"
make_cert "$sandbox/trick.cnf" "$sandbox/trick.pem"
echo "not a certificate" > "$sandbox/junk.pem"

check "ca: expected subject and CA flag accepted" in_dispatch ca_certificate_ok "$sandbox/ca.pem"
check "ca: other subject rejected"                not in_dispatch ca_certificate_ok "$sandbox/other.pem"
check "ca: non-CA certificate rejected"           not in_dispatch ca_certificate_ok "$sandbox/leaf.pem"
check "ca: two-certificate bundle rejected"       not in_dispatch ca_certificate_ok "$sandbox/two.pem"
check "ca: CA:TRUE only in a comment rejected"    not in_dispatch ca_certificate_ok "$sandbox/trick.pem"
check "ca: junk rejected"                         not in_dispatch ca_certificate_ok "$sandbox/junk.pem"

check "user: plain name accepted"     in_install valid_username dev_user1
check "user: empty rejected"          not in_install valid_username ""
check "user: space rejected"          not in_install valid_username "a b"
check "user: comma rejected"          not in_install valid_username "a,b"
check "user: percent rejected"        not in_install valid_username "%admin"
check "user: leading dash rejected"   not in_install valid_username "-x"
check "user: ALL rejected"            not in_install valid_username ALL

rendered="$sandbox/dropin"
( source "$here/install.sh"; render_dropin devuser ) > "$rendered" 2>/dev/null
check "dropin: parses under visudo"   visudo -cf "$rendered"
check "dropin: exactly one rule line" test "$(grep -c '^devuser ' "$rendered")" -eq 1
check "dropin: grants the dispatcher path only" \
  grep -qE '^devuser ALL=\(root\) NOPASSWD: NOSETENV: /usr/local/libexec/holy-blocker/dispatch$' "$rendered"
check "dropin: no wildcard or ALL command" not grep -qE '(\*|NOPASSWD: ALL|: ALL$)' "$rendered"

verb_boom() { false; echo continued; }
verb_fine() { true; }
boom_output="$( ( source "$here/dispatch"; run_verb boom ) 2>/dev/null )"
check "run_verb: failing command stops the verb" test -z "$boom_output"
verb_status_of() { ( source "$here/dispatch"; set +e; run_verb "$1" >/dev/null 2>&1; echo $? ); }
check "run_verb: failure is reported"  test "$(verb_status_of boom)" -ne 0
check "run_verb: success is reported"  test "$(verb_status_of fine)" -eq 0

swap_dir="$sandbox/swap"
mkdir -p "$swap_dir" && echo old > "$swap_dir/dest" && echo new > "$swap_dir/staged"
in_dispatch swap_into_place "$swap_dir/staged" "$swap_dir/dest"
check "swap: new content in place"     test "$(cat "$swap_dir/dest")" = new
check "swap: previous removed"         test ! -e "$swap_dir/dest.previous"
rm -f "$swap_dir/staged"
in_dispatch swap_into_place "$swap_dir/missing" "$swap_dir/dest"
check "swap: failure restores previous" test "$(cat "$swap_dir/dest")" = new

log_probe="$sandbox/log"
head -c 300000 /dev/zero > "$log_probe"
in_dispatch rotate_log_if_large "$log_probe"
check "log: oversize file rotated"     test -f "$log_probe.1" -a ! -f "$log_probe"
echo small > "$log_probe"
in_dispatch rotate_log_if_large "$log_probe"
check "log: small file kept"           test -f "$log_probe"

acl_tree="$sandbox/acl"
mkdir -p "$acl_tree/tree" "$acl_tree/target"
touch "$acl_tree/tree/inner"
chmod +a "everyone deny delete" "$acl_tree/target"
chmod +a "$(id -un) allow write" "$acl_tree/tree/inner"
ln -s "$acl_tree/target" "$acl_tree/tree/link"
in_dispatch strip_acls "$acl_tree/tree"
check "acl: copied file cleared"          test "$(ls -le "$acl_tree/tree/inner" | wc -l)" -eq 1
check "acl: symlink target untouched"     test "$(ls -lde "$acl_tree/target" | wc -l)" -eq 2

chmod -N "$acl_tree/target"
chmod +a "everyone deny delete" "$acl_tree/target"
call_site="$sandbox/call"
mkdir -p "$call_site/src" "$call_site/dest"
ln -s "$acl_tree/target" "$call_site/src/link"
( source "$here/dispatch"; chown() { :; }; copy_to_root_owned "$call_site/src" "$call_site/dest" copy ) >/dev/null 2>&1
check "copy: symlink target ACL untouched via the real call site" test "$(ls -lde "$acl_tree/target" | wc -l)" -eq 2
check "copy: no symlink-following chmod remains" not grep -q 'chmod -RN' "$here/dispatch"
chmod -N "$acl_tree/target"
long_args="$(in_dispatch_out bounded_args "$(head -c 2000 /dev/zero | tr '\0' a)" b)"
check "log: arguments truncated"          test "${#long_args}" -le 300

echo
if [[ $failures -eq 0 ]]; then echo "all passed"; else echo "$failures failed"; exit 1; fi
