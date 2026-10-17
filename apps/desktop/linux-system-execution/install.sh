#!/bin/sh
# Explicit development/AppImage installation. Never called by GUI startup.
set -eu
RUNNER=/usr/libexec/nomifun-system-runner
POLICY=/usr/share/polkit-1/actions/com.nomifun.system-execution.policy
SOURCE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)

fail() { printf '%s\n' "$*" >&2; exit 1; }
secure_directory() {
  directory=$(realpath -e -- "$1")
  while :; do
    [ "$(stat -Lc '%u' -- "$directory")" = 0 ] || fail "System directory is not root-owned: $directory"
    mode=$(stat -Lc '%a' -- "$directory")
    [ "$((0$mode & 0022))" -eq 0 ] || fail "System directory is group/world writable: $directory"
    [ "$directory" != / ] || break
    directory=$(dirname -- "$directory")
  done
}
check_file() {
  [ -f "$1" ] && [ ! -L "$1" ] || fail "Missing regular system component: $1"
  [ "$(stat -c '%u:%g' -- "$1")" = 0:0 ] || fail "System component is not root:root: $1"
  [ "$(stat -c '%a' -- "$1")" = "$2" ] || fail "Unexpected system component mode: $1 (expected $2)"
}
check_installation() {
  [ -x /usr/bin/pkexec ] || fail 'Missing /usr/bin/pkexec; install the distribution polkit/pkexec package.'
  secure_directory /usr/libexec
  secure_directory /usr/share/polkit-1/actions
  check_file "$RUNNER" 755
  check_file "$POLICY" 644
  if [ -x /usr/bin/pkaction ]; then
    /usr/bin/pkaction --action-id com.nomifun.system-execution.run --verbose
  fi
  printf '%s\n' 'Installed system components have the expected paths, ownership and modes. Actual operations still require native authentication.'
}
package_owned() {
  if command -v dpkg-query >/dev/null 2>&1 && dpkg-query -S "$1" >/dev/null 2>&1; then return 0; fi
  if command -v rpm >/dev/null 2>&1 && rpm -qf "$1" >/dev/null 2>&1; then return 0; fi
  return 1
}

case "${1:-}" in
  --check) [ "$#" = 1 ] || fail 'Usage: install.sh [--check|--uninstall]'; check_installation; exit 0 ;;
  --uninstall|'') [ "$#" -le 1 ] || fail 'Usage: install.sh [--check|--uninstall]' ;;
  *) fail 'Usage: install.sh [--check|--uninstall]' ;;
esac
[ "$(id -u)" = 0 ] || fail 'Run this explicit installation with the system administrator, for example: sudo ./install.sh'
for path in "$RUNNER" "$POLICY"; do
  if package_owned "$path"; then fail "A native package owns $path; update/remove it with the package manager instead."; fi
done
secure_directory /usr
secure_directory /usr/share
for directory in /usr/libexec /usr/share/polkit-1 /usr/share/polkit-1/actions; do
  if [ ! -e "$directory" ]; then mkdir -m 0755 -- "$directory"; fi
  secure_directory "$directory"
done

if [ "${1:-}" = --uninstall ]; then
  # Removing the policy prevents new starts. Existing supervised tasks retain
  # their own lifetime and cleanup; there is no daemon or credential store.
  rm -f -- "$POLICY" "$RUNNER"
  printf '%s\n' 'Removed the manually installed NomiFun system execution components.'
  exit 0
fi

[ -x /usr/bin/pkexec ] || fail 'Missing /usr/bin/pkexec; install the distribution polkit/pkexec package first.'
for name in nomifun-system-runner com.nomifun.system-execution.policy; do
  [ -f "$SOURCE_DIR/$name" ] && [ ! -L "$SOURCE_DIR/$name" ] || fail "Missing staged regular file: $SOURCE_DIR/$name"
done
runner_stage=$(mktemp /usr/libexec/.nomifun-system-runner.XXXXXXXX)
policy_stage=$(mktemp /usr/share/polkit-1/actions/.nomifun-system-execution.XXXXXXXX)
trap 'rm -f -- "$runner_stage" "$policy_stage"' EXIT HUP INT TERM
install -o 0 -g 0 -m 0755 -- "$SOURCE_DIR/nomifun-system-runner" "$runner_stage"
install -o 0 -g 0 -m 0644 -- "$SOURCE_DIR/com.nomifun.system-execution.policy" "$policy_stage"
mv -fT -- "$runner_stage" "$RUNNER"
mv -fT -- "$policy_stage" "$POLICY"
# Apply the distribution's own SELinux path labels when its native tool exists.
# Authentication and execution still obey that policy; never disable enforcement.
if command -v restorecon >/dev/null 2>&1; then restorecon "$RUNNER" "$POLICY"; fi
check_installation
