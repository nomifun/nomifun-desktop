#!/bin/sh
# Run before deleting an AppImage: sh appimage-uninstall.sh /absolute/NomiFun.AppImage
set -eu
image=${1:?Pass the absolute installed NomiFun AppImage path}
case "$image" in /*.AppImage|/*.appimage) ;; *) echo 'An absolute AppImage path is required.' >&2; exit 2 ;; esac
[ -f "$image" ] && [ -x "$image" ] || { echo 'The AppImage is not executable.' >&2; exit 2; }
timeout 90s "$image" --keyboard-unregister
printf '%s\n' 'NomiFun keyboard service and login registration were removed. You may now delete the AppImage.'
