# NomiFun Codex Runtime Vendor Boundary

This directory records the pinned Codex app-server fork inputs for the
NomiFun-managed sidecar. It intentionally does not copy the upstream source
tree.

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

`release-input.json` is the immutable release input fixture copied from the
canonical runtime contracts. Release tooling must regenerate real artifact,
license, notice, SBOM, patch-series, and package digests without writing
status or evidence back into that input.
