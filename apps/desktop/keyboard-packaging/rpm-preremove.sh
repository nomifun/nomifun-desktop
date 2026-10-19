#!/bin/sh
set -eu
mode=stop
if [ "${1:-0}" = 0 ]; then mode=unregister; fi
helper=/usr/lib/nomifun-keyboard/package-lifecycle.sh
if [ -f "$helper" ]; then /bin/sh "$helper" "$mode"; fi
