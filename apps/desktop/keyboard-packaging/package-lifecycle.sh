#!/bin/sh
# Invoked by deb/rpm before replacing or removing installed executables.
set -eu
mode=${1:?stop or unregister required}
# This executable may predate the incoming package. These maintenance-only
# aliases are understood by old releases and map to application shutdown in
# current releases; never send an unknown new flag that could open an old GUI.
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
  # The GUI owns the keyboard transport and can be between lock acquisition
  # and descriptor publication. Maintenance also retires any authenticated old
  # service owner; never gate application shutdown on a legacy owner.json.
  if [ -d "$runtime" ]; then
    timeout 90s runuser -u "$account" -- env HOME="$directory" XDG_RUNTIME_DIR="$runtime" "$binary" "$flag"
  fi
  # Check the fallback independently even when a graphical runtime exists.
  timeout 90s runuser -u "$account" -- env -u XDG_RUNTIME_DIR HOME="$directory" "$binary" "$flag"
done
