#!/bin/sh
set -eu
helper=/usr/lib/nomifun-keyboard/package-lifecycle.sh
if [ -f "$helper" ]; then /bin/sh "$helper" stop; fi
