# NomiFun Computer worker

This is an independent Cargo workspace. It links only the allowlisted vendored
Cua Driver crates and exposes the inherited private worker protocol v1 over
stdin/stdout. The host starts it directly:

```text
nomifun-computer-worker __private-worker --generation <id>
nomifun-computer-worker --self-check
```

Build and test from this directory with the repository's stable toolchain:

```text
cargo build --locked
cargo test --locked
cargo run --locked -- --self-check
```

The root `scripts/vendor/sync-cua-driver.mjs` pipeline also builds and tests
this workspace. It keeps Windows and WSL intermediate directories separate and
uses Cargo's reported executable path. When invoking Cargo directly from both
systems in one checkout, set a separate `CARGO_BUILD_BUILD_DIR` for each system.

The self-check and real-worker protocol tests isolate all runtime/configuration
directories and perform no desktop actions. Self-check sends only initialize,
list and shutdown. Protocol tests additionally exercise metadata, session
binding/closing, rejection and stdin EOF.

Linux includes `portal-input`. `portal-capture` remains disabled pending the S1
GNOME Wayland POC; a Windows build does not establish Linux or macOS support.
On macOS the private channel runs off-thread and the AppKit cursor overlay runs
on the main thread. Windows embeds an asInvoker Per-Monitor V2 DPI manifest.
Linux prepares the registered native XTEST cleanup owner on the initial thread
before creating Tokio/SDK threads; normal exit checks its actual cleanup result.

`src/private_worker.rs`, the startup portion of `src/main.rs`, the manifest/
resource script and the platform linking portion of `build.rs` are adapted from
trycua/cua at `4d959bbfa57f0c89f310330e849500b222084194`. Their MIT attribution is
retained in [LICENSE](LICENSE). The dependency lock started from that snapshot's
Rust workspace lock and is resolved for this smaller workspace. No upstream
binary crate, host extension/update tool, telemetry sender or perception model
is linked.
