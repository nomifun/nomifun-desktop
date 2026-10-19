#!/bin/sh
# Invoked by deb/rpm before replacing or removing installed executables.
set -eu
mode=${1:?stop or unregister required}
case "$mode" in stop) flag=--keyboard-stop ;; unregister) flag=--keyboard-unregister ;; *) exit 2 ;; esac
binary=/usr/bin/nomifun-desktop
getent passwd | while IFS=: read -r account unused uid gid comment directory login_shell; do
  case "$uid" in ''|*[!0-9]*) continue ;; esac
  [ "$uid" -ge 1000 ] && [ "$uid" -lt 65534 ] || continue
  case "$directory" in /*) ;; *) continue ;; esac
  runtime=/run/user/$uid
  owner=$runtime/NomiFun/keyboard-service/owner.json
  desktop=$runtime/NomiFun/keyboard-service/desktop.json
  fallback=$directory/.local/state/NomiFun/keyboard-service/owner.json
  fallback_desktop=$directory/.local/state/NomiFun/keyboard-service/desktop.json
  if [ ! -x "$binary" ]; then
    if [ -f "$owner" ] || [ -f "$desktop" ] || [ -f "$fallback" ] || [ -f "$fallback_desktop" ]; then
      echo 'NomiFun endpoint exists but its installed executable is missing; refusing package replacement.' >&2
      exit 1
    fi
    continue
  fi
  # A GUI-only microphone test has no serial owner, and a live process can be
  # between lock acquisition and descriptor publication. The CLI checks both
  # authenticated endpoints and process locks; never gate it on owner.json.
  if [ -d "$runtime" ]; then
    timeout 90s runuser -u "$account" -- env HOME="$directory" XDG_RUNTIME_DIR="$runtime" "$binary" "$flag"
  fi
  # Check the fallback independently even when a graphical runtime exists.
  timeout 90s runuser -u "$account" -- env -u XDG_RUNTIME_DIR HOME="$directory" "$binary" "$flag"
done
