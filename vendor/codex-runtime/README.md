# NomiFun Codex Runtime Vendor Boundary

This directory records the pinned Codex app-server fork inputs for the
NomiFun-managed sidecar. It intentionally does not copy the upstream source
tree.

These protocol notes describe the current adapter only. They are not a final
Phase 1 Sidecar contract: 05 requires an upstream app-server spike before the
fork, custom RPC set, and dispose handshake can be retained.

- Frozen source: `../codex` at
  `dc2ccc6843abb09c9d297862dc10b6bd12a3935d`
- Transport: local managed stdio JSONL only
- Public runtime RPCs: the exact eight-method allowlist in
  `protocol/adapter-map.json`
- Runtime policy: `approvalPolicy=never` and
  `sandboxPolicy=dangerFullAccess`
- Credential transport: one-shot inherited anonymous pipe or duplicated OS
  handle; credential bytes never enter argv, environment, disk, or protocol
  JSON
- Lifecycle: stable `runtime/session/dispose`, followed by managed
  descendant-process-tree proof

The former `release-input.json` belongs only to the schema/contract fixture
domain. Packaging, native validation, and release tooling must not read
fixture digests as real artifact identities.

The macOS package flow creates an external `*.release-lock.json` only after
the app and DMG exist. The lock has one small cross-platform shape:

```json
{
  "schema_version": "1.0.0",
  "source_commit": "<clean candidate Git SHA>",
  "platform": "universal-apple-darwin",
  "host": { "path": "<artifact-root-relative path>", "sha256": "<real file SHA-256>" },
  "sidecars": {
    "macos_desktop_arm64": {
      "path": "<artifact-root-relative path>",
      "sha256": "<real file SHA-256>"
    }
  },
  "helpers": [],
  "package": { "path": "<artifact-root-relative path>", "sha256": "<real file SHA-256>" },
  "legal": []
}
```

`scripts/release/release-lock.mjs` creates and verifies this file. Creation
requires a clean tracked worktree plus real regular, non-symlink host,
sidecar, and package files; it never writes placeholder digests. Paths are
relative to the supplied artifact root so packaging and native validation can
resolve the same shape without build-machine absolute paths.

`scripts/validation/check-macos-arm64-native.mjs` requires that release lock
and emits a minimal platform result containing the source commit, platform,
target, actual check list, status, release-lock reference, and log
references. Missing locks or artifacts remain explicitly blocked; digest
mismatches fail.
