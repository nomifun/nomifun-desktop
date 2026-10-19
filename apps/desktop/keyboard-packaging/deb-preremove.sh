#!/bin/sh
set -eu
mode=stop
case "${1:-}" in remove|deconfigure) mode=unregister ;; esac
helper=/usr/lib/nomifun-keyboard/package-lifecycle.sh
if [ -f "$helper" ]; then /bin/sh "$helper" "$mode"; fi
