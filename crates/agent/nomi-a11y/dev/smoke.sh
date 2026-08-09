#!/usr/bin/env bash
# Headless AT-SPI behavioral harness (runs INSIDE the dev container):
# Xvfb virtual display + private D-Bus session + at-spi2 a11y bus + a real
# accessible GTK app, then run the nomi-a11y `linux_smoke` example against it.
set -uo pipefail

export DISPLAY=:99
Xvfb :99 -screen 0 1280x900x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
XVFB_PID=$!
sleep 1

dbus-run-session -- bash -u -c '
  export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1
  export GTK_MODULES="${GTK_MODULES:-}:atk-bridge"
  export NO_AT_BRIDGE=0
  # Launch the AT-SPI registry/bus (path differs across Debian versions; try both).
  for d in /usr/libexec /usr/lib/at-spi2-core /usr/lib/at-spi2; do
    [ -x "$d/at-spi-bus-launcher" ] && ( "$d/at-spi-bus-launcher" --launch-immediately >/tmp/atspi-bus.log 2>&1 & )
    [ -x "$d/at-spi2-registryd" ]  && ( "$d/at-spi2-registryd" >/tmp/atspi-reg.log 2>&1 & )
  done
  sleep 1
  gtk3-widget-factory >/tmp/app.log 2>&1 &
  APP_PID=$!
  sleep 3
  echo "=== AT-SPI bus address: ${AT_SPI_BUS_ADDRESS:-<unset>} ==="
  echo "=== running nomi-a11y linux_smoke example ==="
  CARGO_TARGET_DIR=/target cargo run -p nomi-a11y --example linux_smoke
  rc=$?
  kill "$APP_PID" 2>/dev/null || true
  exit $rc
'
rc=$?
kill "$XVFB_PID" 2>/dev/null || true
exit $rc
