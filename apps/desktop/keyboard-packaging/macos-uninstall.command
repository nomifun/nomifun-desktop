#!/bin/sh
# Installed in NomiFun.app/Contents/Resources/keyboard-packaging.
set -eu
resources=$(CDPATH= cd "$(dirname "$0")/.." && pwd -P)
binary=$resources/../MacOS/nomifun-desktop
[ -x "$binary" ] || { echo 'Installed NomiFun executable is missing.' >&2; exit 1; }
"$binary" --keyboard-unregister
printf '%s\n' 'NomiFun keyboard service and LaunchAgent were removed. You may now move NomiFun.app to Trash.'
