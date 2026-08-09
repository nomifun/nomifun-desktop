#!/usr/bin/env bash
# Build / test / smoke the nomi-a11y Linux backend inside the dev container.
# Native Linux build (no cross-linker). Named volumes cache target + registry
# so incremental builds are fast across runs.
#
#   ./run-linux-a11y.sh build        # cargo build -p nomi-a11y
#   ./run-linux-a11y.sh test         # cargo test  -p nomi-a11y
#   ./run-linux-a11y.sh check-chain  # cargo check -p nomi-agent --features computer-use
#   ./run-linux-a11y.sh smoke        # headless AT-SPI behavioral run (Xvfb+dbus+gtk app)
#   ./run-linux-a11y.sh shell        # interactive shell in the container
set -euo pipefail
REPO="$(cd "$(dirname "$0")/../../../.." && pwd)"
IMG=nomi-a11y-linux:dev
COMMON=(--rm -v "$REPO":/work -w /work
  -v nomi-a11y-target:/target -e CARGO_TARGET_DIR=/target
  -v nomi-a11y-cargo-registry:/usr/local/cargo/registry)

case "${1:-test}" in
  build)       docker run "${COMMON[@]}" "$IMG" cargo build -p nomi-a11y ;;
  test)        docker run "${COMMON[@]}" "$IMG" cargo test  -p nomi-a11y ;;
  check-chain) docker run "${COMMON[@]}" "$IMG" cargo check -p nomi-agent --features computer-use ;;
  smoke)       docker run "${COMMON[@]}" "$IMG" bash crates/agent/nomi-a11y/dev/smoke.sh ;;
  shell)       docker run -it "${COMMON[@]}" "$IMG" bash ;;
  *) echo "usage: $0 {build|test|check-chain|smoke|shell}" >&2; exit 1 ;;
esac
