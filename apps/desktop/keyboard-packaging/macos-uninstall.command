#!/bin/sh
# Installed in NomiFun.app/Contents/Resources/keyboard-packaging.
set -eu
resources=$(CDPATH= cd "$(dirname "$0")/.." && pwd -P)
binary=$resources/../MacOS/nomifun-desktop
[ -x "$binary" ] || { echo 'Installed NomiFun executable is missing.' >&2; exit 1; }
"$binary" --app-maintenance-unregister
printf '%s\n' 'NomiFun was stopped and any retired keyboard login item was removed. You may now move NomiFun.app to Trash.'
