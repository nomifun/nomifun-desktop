#!/usr/bin/env bash
# Native X11 primitives through the product ComputerTool, using only this
# invocation's disposable display and windows. This is not login-desktop,
# Wayland authorization, or default-Agent end-to-end acceptance.
set -euo pipefail
for executable in Xvfb openbox xauth mcookie xdpyinfo cargo; do
  command -v "$executable" >/dev/null || { echo "Missing native test dependency: $executable" >&2; exit 1; }
done
repo_root="$(cd "$(dirname "$0")/../.." && pwd -P)"
fixture_root="$(mktemp -d /tmp/nomifun-computer-x11.XXXXXXXX)"
export XAUTHORITY="$fixture_root/authority"
touch "$XAUTHORITY"
chmod 600 "$XAUTHORITY"
cookie="$(mcookie)"
# WSLg owns /tmp/.X11-unix. Explicit numbered servers can still use abstract
# Unix transport, while -displayfd rejects the unavailable filesystem listener.
# Select an unused high display and retain our actual server PID throughout.
display_number=
for candidate in $(seq 90 149); do
  if [ ! -e "/tmp/.X$candidate-lock" ] && ! grep -Fq "@/tmp/.X11-unix/X$candidate" /proc/net/unix; then
    display_number="$candidate"
    break
  fi
done
[[ "$display_number" =~ ^[0-9]+$ ]] || { echo "No private X11 display is available" >&2; exit 1; }
export DISPLAY=":$display_number"
xauth -f "$XAUTHORITY" add "$DISPLAY" . "$cookie"
Xvfb "$DISPLAY" -screen 0 1024x768x24 -nolisten tcp -noreset -auth "$XAUTHORITY" \
  >"$fixture_root/xvfb.log" 2>&1 &
xvfb_pid=$!
openbox_pid=
cleanup() {
  if [ -n "$openbox_pid" ]; then kill "$openbox_pid" 2>/dev/null || true; wait "$openbox_pid" 2>/dev/null || true; fi
  kill "$xvfb_pid" 2>/dev/null || true
  wait "$xvfb_pid" 2>/dev/null || true
  rm -f -- "$XAUTHORITY"
}
trap cleanup EXIT
for attempt in $(seq 1 50); do
  if xdpyinfo >/dev/null 2>&1; then break; fi
  kill -0 "$xvfb_pid" 2>/dev/null || { cat "$fixture_root/xvfb.log" >&2; exit 1; }
  sleep 0.1
done
export XDG_SESSION_TYPE=x11
export GDK_BACKEND=x11
unset WAYLAND_DISPLAY
xdpyinfo >"$fixture_root/display-info.log"
openbox --sm-disable >"$fixture_root/openbox.log" 2>&1 &
openbox_pid=$!
for attempt in $(seq 1 50); do
  if xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null | grep -q 'window id'; then break; fi
  sleep 0.1
done
echo "Native fixture: $fixture_root (display $DISPLAY; Unix transport; TCP disabled; private Xauthority)"
cd "$repo_root"
if [ "$#" -gt 0 ]; then
  # A native App acceptance driver may run for this display's whole lifetime.
  # Its executable and literal argv are passed through without shell parsing.
  "$@"
else
  cargo test --locked -p nomi-computer --test linux_x11_native -- --ignored --nocapture
fi
