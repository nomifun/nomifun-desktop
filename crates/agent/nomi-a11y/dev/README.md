# Linux Accessibility Backend Validation

This directory contains helper assets for validating the Linux AT-SPI backend
from `nomi-a11y`.

Use the lightest path that proves the behavior you are changing.

## Path A: Type And API Check

This catches most portability issues without linking or running Linux binaries.

```bash
rustup target add x86_64-unknown-linux-gnu
cargo check --target x86_64-unknown-linux-gnu \
  -p nomi-a11y --examples --tests
```

## Path B: Docker

The Dockerfile installs Rust, AT-SPI, Xvfb, D-Bus, and GTK example widgets so
the smoke test can run headlessly.

```bash
docker build -t nomi-a11y-linux:dev \
  -f crates/agent/nomi-a11y/dev/Dockerfile.linux-a11y .

crates/agent/nomi-a11y/dev/run-linux-a11y.sh test
crates/agent/nomi-a11y/dev/run-linux-a11y.sh smoke
```

If the environment cannot pull base images, run the same commands inside any
Linux VM with the dependencies below installed.

## Path C: Native Linux VM

Install the runtime dependencies:

```bash
sudo apt-get update
sudo apt-get install -y \
  at-spi2-core gtk-3-examples xvfb dbus-x11 \
  build-essential pkg-config
```

Build the smoke example with a VM-local target directory so host builds do not
share artifacts:

```bash
CARGO_TARGET_DIR="$HOME/nomi-a11y-target" \
CARGO_BUILD_BUILD_DIR="$HOME/nomi-a11y-build" \
  cargo build -p nomi-a11y --example linux_smoke
```

Run the smoke test under Xvfb and a private D-Bus session:

```bash
export DISPLAY=:99
Xvfb :99 -screen 0 1280x900x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &

dbus-run-session -- bash -uc '
  export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1
  export GTK_MODULES=atk-bridge
  export NO_AT_BRIDGE=0
  export DISPLAY=:99
  gtk3-widget-factory >/tmp/app.log 2>&1 &
  sleep 4
  ./target/debug/examples/linux_smoke
'
```

Set `NOMI_A11Y_CLICK=<substring>` to ask `linux_smoke` to invoke the first
matching element action.

## Coverage Notes

- The smoke test validates an X11 session. Wayland support can legitimately
  degrade for synthetic pixel input while semantic actions remain available;
  check the reported `capabilities`.
- KDE/Qt apps may require `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`.
- Electron apps often require `--force-renderer-accessibility`.
- Sandboxed apps such as Flatpak may not expose a complete accessibility tree.

`atspi` / `zbus` are pure Rust dependencies, so cross-checking the Linux backend
from a non-Linux host is practical. Behavior validation still needs a Linux
runtime.
