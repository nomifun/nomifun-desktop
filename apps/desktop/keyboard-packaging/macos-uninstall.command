#!/bin/sh
# Installed in NomiFun.app/Contents/Resources/keyboard-packaging.
set -eu
resources=$(CDPATH= cd "$(dirname "$0")/.." && pwd -P)
binary=$resources/../MacOS/nomifun-desktop
[ -x "$binary" ] || { echo 'Installed NomiFun executable is missing.' >&2; exit 1; }
# This command requires current native-capture stop proof and the application's
# maintenance lifetime fence. It never queries or cleans historical keyboard
# registrations; optional retirement is separate from the uninstall path.
if ! "$binary" --app-maintenance-unregister; then
  printf '%s\n' 'NomiFun could not confirm that the application stopped. Quit NomiFun and retry before moving the app to Trash.' >&2
  exit 1
fi
printf '%s\n' 'The current NomiFun application was stopped. You may now move NomiFun.app to Trash. Your settings and user data are preserved. Retired keyboard registrations are handled separately.'
