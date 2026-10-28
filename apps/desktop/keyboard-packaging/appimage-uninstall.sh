#!/bin/sh
# Run before deleting an AppImage: sh appimage-uninstall.sh /absolute/NomiFun.AppImage
set -eu
image=${1:?Pass the absolute installed NomiFun AppImage path}
case "$image" in /*.AppImage|/*.appimage) ;; *) echo 'An absolute AppImage path is required.' >&2; exit 2 ;; esac
[ -f "$image" ] && [ -x "$image" ] || { echo 'The AppImage is not executable.' >&2; exit 2; }
# The caller may supply an older image. Keep its recognized maintenance-only
# stop/unregister alias; current releases dispatch it to application shutdown.
timeout 90s "$image" --keyboard-unregister
printf '%s\n' 'NomiFun was stopped and any retired keyboard login item was removed. You may now delete the AppImage.'
