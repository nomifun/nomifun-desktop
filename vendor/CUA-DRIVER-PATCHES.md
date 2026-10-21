# Cua Driver source and patch register

NomiFun imports the MIT-licensed Cua Driver from
<https://github.com/trycua/cua>, fixed at commit
`4d959bbfa57f0c89f310330e849500b222084194` (workspace version `0.37.0`).
The requested release label is `cua-driver-rs-v0.37.0`. On 2026-10-20 its Git
ref and peeled commit both resolved to
`2d2f99228d4d2508dc6fcc84c176b296e146714a`; the explicit immutable commit above
is authoritative. Sync always fetches and checks that commit independently.

The D03 extraction includes only the nine approved Rust crates, the upstream
lockfile, license, third-party notices, version and contract manifest. Crate
`tests/` and `examples/` directories are excluded except individual source files
required to resolve explicit Cargo target declarations; no test fixtures are
imported. Embedded source assets,
Wayland protocol XML and the `Skills/cua-driver` text used by `include_str!`
are retained unchanged. The root Cargo manifest is a mechanical workspace
projection: its member list contains only these nine crates, while all package
and dependency declarations remain upstream. This extraction is not a driver
behavior patch. No upstream CLI, perception model, language binding or test
fixture is imported.

| Patch | Reason | Changed files |
| --- | --- | --- |
| [0001-platform-core-without-default-features.patch](patches/cua-driver/0001-platform-core-without-default-features.patch) | D04: prevent platform dependency feature unification from re-enabling `rego`, `yaml` and `script`; remove the Windows Spectre-library build dependency. | The `cua-driver-core` dependency in `platform-linux`, `platform-macos` and `platform-windows` Cargo manifests only. |
| [0002-x11-owned-input-crash-cleanup.patch](patches/cua-driver/0002-x11-owned-input-crash-cleanup.patch) | User-authorized exception on 2026-10-20 for the reproduced X11 input cleanup defect: give foreground XTEST an exact owned device pair and pre-runtime cleanup guardian, and conditionally restore only native-owner-authored temporary keymap rows. Preserve upstream background delivery and keyboard-layout planning. | `platform-linux/src/input/{interactive.rs,mod.rs,targeted.rs}` and the new `owned_xtest.rs` source module. |
| [0003-windows-user-app-lifetime.patch](patches/cua-driver/0003-windows-user-app-lifetime.patch) | User-authorized exception on 2026-10-20 for the reproduced Windows launch lifetime defect: user applications must survive normal worker retirement. Launch through a capability-bound one-shot helper whose exact native parent is the existing same-user interactive shell. Preserve the driver's Job and precise launch PID; never fall back to launching inside that Job. | `platform-windows/Cargo.toml` (`Win32_System_Environment` API feature) and `src/{lib.rs,user_app_launch.rs,tools/impl_.rs}`. |
| [0004-macos-direct-capture-consent-home.patch](patches/cua-driver/0004-macos-direct-capture-consent-home.patch) | User-authorized exception on 2026-10-21 for the reproduced macOS consent lookup defect: private worker `HOME` is writable driver state, not the native account home that owns replayd metadata. Resolve the read-only approval path through `getpwuid_r`, while keeping private write roots and the existing Missing/Unknown policy. Direct embedded-host consent guidance to the host's explicit Screen Recording / Verify capture setup. | `platform-macos/src/capture_consent.rs` only. |

The X11 cleanup patch also drains the original core pointer's hover client and
keyboard-focus client when a device pair was created but input binding had not
completed. This extends the same native WM-ping drain before retiring the owned
pair; it does not move or bind the original core devices or change the API.
After releasing and parking held input and restoring borrowed core rows, the
same live guardian retains responsibility for a PING-capable client until a real
acknowledgement or native XRes connection disappearance is observed. A missing
window or an unanswered PING alone does not release that responsibility; no
input is replayed while waiting.
The guardian searches the same native client's surviving PING windows before
classifying a classic client as unsupported; unused process-start-time state is
not retained.
Each retirement pass attempts core-row restoration independently of input
parking, so a parking error cannot skip restoration or release the native
owner's remaining responsibility.
Creation and retirement recover an existing pair through its retained exact
native name before draining it. An empty cached pair does not prove that no pair
exists; partial discovery or inspection errors retain the same owner for cleanup.

There are exactly four registered patches. The latter three are the narrowly
authorized input-cleanup, user-app lifetime and macOS consent-lookup exceptions recorded in the sole implementation plan's
[execution progress](../docs/handoffs/computer-use-rebuild.zh.md#12-单份执行进展);
They do not authorize any other driver behavior changes. The Windows helper has
no persistent endpoint, accepts one nonce-bound typed file request, and inherits no
worker handles at creation. Before any user-app launch it duplicates only the
existing host stderr pipe as a non-inheritable native lifetime handle. Host EOF
therefore continues to cover a launch-capable helper after its worker exits;
no user application inherits that handle. One native wait watches its exact
worker owner and the bounded launch lifetime.
Its environment is rebuilt with `CreateEnvironmentBlock(false)` from that same
shell token, and its working directory is that token's user profile. Private
driver HOME, XDG, CUA and temporary-directory overrides never reach the user app.
It changes neither process-runtime Job limits nor system permissions. Both direct
Win32 and ShellExecute paths, including secondary URLs, use the same launch boundary.
Do not edit imported
sources. Change the registered patch and resync if an approved source change is
needed. Sync verifies the existing owned files against their recorded hashes
before replacing the snapshot; local vendor edits are never overwritten.
Runtime write locations are controlled by the private worker's environment,
not by a vendor patch.

The macOS consent patch reads only the native account's existing replayd record;
it never writes or fabricates approvals and never falls back to an overridden
`HOME`. Missing and unreadable/unknown records keep their upstream meanings.
The existing capture guard and `CUA_DRIVER_ALLOW_CAPTURE_CONSENT_ALERT` branch
are unchanged; the embedding-host hint does not suggest that bypass. A matching
OS record is not proof that a real capture succeeded. The host's explicit
permission flow remains responsible for any native prompt and live verification.

`cua-driver/SOURCE.json` records every upstream path/blob, final SHA-256, the
workspace extraction and registered patch hashes. Run:

```sh
node scripts/vendor/sync-cua-driver.mjs --check
node scripts/vendor/sync-cua-driver.mjs --verify-upstream
```

The first command checks the local snapshot without network access. The second
uses a fresh filtered sparse clone to reconstruct the snapshot and compare every
byte. `--source /absolute/upstream/checkout` may supply a read-only checkout at
the exact pinned commit instead. Default sync also builds the private worker,
regenerates the frozen tool surface through `scripts/computer/freeze-tools.mjs`
and runs the worker's protocol tests. Use `--vendor-only` while bootstrapping the
standalone worker.

Registered patch hashes cover their exact original bytes. Sync normalizes CRLF
to LF only when passing textual hunks to `git apply`, matching this repository's
`eol=lf` contract and keeping generated source hashes stable across checkout.
`SOURCE.json` records this mechanical step as `patch_application: git-apply-text-lf`.
Each apply uses command-local `core.autocrlf=false` and `core.eol=lf`; developer
Git configuration is never changed or allowed to rewrite the generated bytes.

The sync pipeline discovers Cargo's effective target directory and its actual
worker executable from Cargo metadata/artifact messages. Unless explicitly set,
`CARGO_BUILD_BUILD_DIR` is placed under that target directory as
`build-<platform>-<architecture>.noindex`, keeping Windows and WSL intermediate
artifacts separate in a shared checkout. Explicit build-directory environment
configuration is preserved.
