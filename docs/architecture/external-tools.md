# External Tools and Office Attachment Previews

The current supply contract lives in
[`nomifun-runtime/components.json`](../../crates/backend/nomifun-runtime/components.json).
[`nomifun_runtime::resolve_tool`](../../crates/backend/nomifun-runtime/src/managed.rs)
is the admission entry point shared by build artifacts, consumers, diagnostics,
and the bundled Office skills.

## Component ownership

| Component | Approved version | Source | Consumer |
| --- | --- | --- | --- |
| Bun | 1.3.13 | Fixed `oven-sh/bun` release | Tasks explicitly requiring Bun |
| Node | 22.22.0 | Fixed `nodejs.org/dist` release | Genuine Node for Unified Plugin Core |
| OfficeCLI | 1.0.156 | Fixed `iOfficeAI/OfficeCLI` release; npm identity `@officecli/officecli` | Read-only Office previews and document skills |

Versions, platform/architecture assets, URLs, upstream checksum provenance,
archive SHA-256, delivery policy, development version ranges, and Docker build-image
digests are in the manifest. Archive hashes authorize component downloads; extracted executable
hashes identify runtime artifacts and `VerifiedTool.digest`. These hashes need
not be identical.

Release builds embed OfficeCLI only. Node and Bun use managed downloads from
the same manifest; they are not application payloads. Debug builds can embed
OfficeCLI with `NOMIFUN_EMBED_OFFICECLI=1`. An unsupported target fails the build.
Builds still verify approved Node/Bun archives and derive their executable digests,
embedding only small identity metadata. A matching executable cache from the former
bundled delivery is verified and reused offline without trusting its stamp or
requiring another archive download. Component updates belong to validated
application releases, with matching probes, tests, and notices; there is no
background global npm installation or latest-version upgrade.

[`notices/manifest.json`](../../crates/backend/nomifun-runtime/notices/manifest.json)
records component notice provenance and versions. Optional nfagent, ONNX native
libraries, and browsers retain their existing domain supply mechanisms; the
catalogue references those authorities rather than introducing another ledger.
User-configured MCP and terminal CLIs retain user ownership. Discovery failures
must remain distinct from successful empty configuration.

## Supply and admission

`resolve_tool(ToolId::Bun | Node | OfficeCli)` returns `VerifiedTool` with the
verified absolute executable path, observed version, and executable digest.
OfficeCLI extracts locally; Node/Bun first check the managed download cache,
then retrieve their approved platform artifact only if needed. Both paths use
`<data_dir>/runtime`, inter-process locks, bounded temporary files, hash
verification, and atomic publication. Downloaded executable identity is derived
from the approved archive rather than trusting a standalone cache stamp.
Library consumers without an initialized data root use platform cache storage.
Read returned paths or diagnostic observations instead of guessing cache paths.

Admission checks real identity and capabilities. Genuine Node must report Node
and no `process.versions.bun`; Bun must report its own engine. Renaming Bun to
`node` or relying on its Node compatibility version cannot satisfy Node
admission. OfficeCLI creates a private temporary DOCX and generates HTML.
File existence and version output alone are insufficient.

Successful admission is reused within the process; failures can be retried.
Later executable size or modification-time changes fail closed until repaired
and restarted. The application creates no Bun-as-Node aliases and does not
modify the system PATH. It retains managed Bun discovery in the application
process and its children only for an already prepared component. Synchronous
PATH discovery never starts or waits for a download. Real Bun consumers prepare
it asynchronously and use the returned absolute path. Plugin Core retains one immutable Node authority;
Node consumers use the real Node returned by its resolver. Process ownership,
timeouts, cancellation, and tree cleanup remain in `nomi-process-runtime`.

Debug-only `NOMIFUN_BUN_PATH`, `NOMIFUN_NODE_PATH`, and
`NOMIFUN_OFFICECLI_PATH` accept explicit absolute executable paths. Paths are
canonicalized and still pass engine, capability, and development version-range
checks. Release ignores overrides and never falls back to same-named PATH tools.

For declared OfficeCLI family Skills, `engine_skills::compile` resolves the
component once and attaches a separate immutable Host context resource with
status, absolute executable path, version, digest, source, and package identity.
The compiled body points to its exact `read_context_resource` ID. Frozen source
bytes, source digest, and capability requirements stay unchanged; executable
metadata grants no additional process/file permission. Skills read this resource
rather than guessing cache paths or performing global installations.

## Diagnostics

```bash
nomicore tool-doctor
nomicore --data-dir /absolute/data-dir tool-doctor --verify
```

This subcommand belongs to the `nomicore` CLI; the separate `nomifun-web` host
CLI does not accept it. Build it with `cargo build --release -p nomifun-app --bin nomicore`
and use the same data root as the installation being inspected.

The first command prints the catalogue, domain supply references, and notices
without extracting tools, probing processes, or opening a database. `--verify`
explicitly prepares all three tools, which may download Node/Bun, extract
approved binaries into that data root, and run private capability probes. JSON observations include
`ready` / `unavailable`, actual version/path/digest, or the error; any failed
component yields a nonzero exit code. Neither mode initializes Agent data,
database storage, logging, or startup database rebuild authority.

Source developers can use `cargo run -p nomifun-app --bin nomicore -- tool-doctor --verify`.
A debug build without OfficeCLI embedded needs its explicit development
override; Node/Bun use the managed download supplier by default. The
separate `doctor` command discovers user CLIs through PATH; it does not replace
product component admission.

## First preparation and package size

Node is required before the unified backend can start. The desktop startup
window observes the shared supplier and displays checking, download byte progress,
extraction, and capability verification. A retryable preparation failure offers
retry before any backend is created. Web hosts report the same preparation through
startup diagnostics. One successful Node admission establishes the existing
immutable runtime authority; there is no second runtime selector or hot switch.

Bun is prepared only when a feature explicitly requests it. Concurrent requests
share preparation; cancellation of one observer does not revoke another user.
Prepared approved artifacts are reused offline across releases while their identity
is unchanged. A fresh installation requires network for initial Node preparation.
Office conversion itself needs neither a Node/Bun download nor network access.

Owner-authenticated `GET /api/runtime-tools/{tool}` observes preparation;
`POST /api/runtime-tools/{tool}/prepare` immediately begins or joins it. Only
`bun`, `node`, and `officecli` are accepted; callers cannot supply URLs, paths,
versions, or global cancellation. MCP tests and terminal launches prepare Bun
before their business RPC. Unprepared direct requests return
`COMPONENT_PREPARATION_REQUIRED` before creating a terminal row or process.

Only the target architecture is delivered. Raising compression or deleting
source files does not substitute for excluding Node/Bun blobs from the release
payload. Compare actual App and DMG artifacts from the same release profile and
compression format; compressed component sizes alone are not installer measurements.

## Office artifacts

PPT, Word, and Excel share the following API. Built-in preview accepts `.pptx`,
`.docx`, and `.xlsx`; unsupported formats offer system opening and downloading.

| Operation | Endpoint | Contract |
| --- | --- | --- |
| Start | `POST /api/office-preview/start` | `{request_id,file_path,workspace?,doc_type}`; UUIDv7 request ID and `ppt` / `word` / `excel` type |
| Status | `GET /api/office-preview/status/{request_id}` | Owner-scoped request |
| Stop | `POST /api/office-preview/stop` | `{request_id}`; cancellation may precede start |
| Artifact | `GET /api/office-preview-artifacts/{capability}/?request_id=...&nonce=...` | Active capability, matching request, UUIDv7 nonce |

Start immediately creates a cancellable owner-scoped lease. States are `queued`,
`rendering`, `ready`, and `error`; ready returns a root capability URL, while
errors carry `{code,message,retryable}`. Only pending requests are polled.
Bounded cancellation tombstones cover stop-before-start races.

The backend snapshots a stable, bounded source and validates its Office ZIP
structure and resource limits. Cache keys combine content digest, engine
version/digest, and renderer parameters. Leases can share a bounded render job;
one cancellation leaves other consumers intact, while no consumers cancels the
job. Lease count/lifetime, cache bytes/entries, and process time are bounded.
Published active results stay pinned during eviction. Source-path authorization
runs for every new request, including cache hits.

Rendering uses the approved engine's `view <file> html` with automatic resident
processes disabled. Every exit, timeout, cancellation, and output limit uses
managed process cleanup and bounded stderr diagnostics. Validated HTML publishes
atomically. Artifact HTTP responses use bounded streaming reads so slow clients
do not each buffer an entire large HTML document.

## Display and refresh

Artifacts are read-only capability resources. Their routes use a random lease
capability rather than requiring iframe SPA credentials; start/status/stop are
authenticated. Reverse proxies preserve the exact path and request/nonce query.
Do not turn this grant into arbitrary file or API access. Reading rechecks source
access and the active lease.

Frames use `sandbox="allow-scripts"` and CSP prohibiting network connections,
external resources, forms, and nested frames. Readiness waits for fonts, images,
and two animation frames. The renderer verifies iframe source, opaque origin,
request ID, capability, and nonce. HTTP ready or iframe load alone is not display
success. Pending candidate frames remain inert.

Existing file events and active-tab metadata polling advance source revision.
Updates retain the painted frame and lease until the next first screen confirms,
then replace it and release the old lease. Failure retains the previous version.
PPT/Excel use their actual inner scroll panes; Word uses document scrolling.
Position messages share the identity checks, and Excel sheet indices are bounded
integers. Auto-open coalesces creation/write events; the backend establishes
stable document completeness. Only recoverable errors expose retry. System-open
and download actions remain available.

See [`nomifun-office`](../../crates/backend/nomifun-office/) and
[`OfficeArtifactViewer`](../../ui/src/renderer/pages/conversation/Preview/components/viewers/OfficeArtifactViewer.tsx)
with their adjacent behavior tests. The replaced watch servers, port probes,
proxies, and per-format start endpoints are retired.
