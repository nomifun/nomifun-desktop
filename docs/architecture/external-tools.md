# External Tools and Office Attachment Previews

The current supply contract lives in
[`nomifun-runtime/components.json`](../../crates/backend/nomifun-runtime/components.json).
The runtime resolver and explicit preparation APIs live in
[`nomifun-runtime/src/managed.rs`](../../crates/backend/nomifun-runtime/src/managed.rs);
per-launch environment and command preparation live in
[`environment.rs`](../../crates/backend/nomifun-runtime/src/environment.rs).
Build artifacts, managed cache admission, diagnostics, and the bundled Office
skills share the approved component identities.

## Component ownership

| Component | Approved version | Source | Consumer |
| --- | --- | --- | --- |
| Bun | 1.3.13 | Fixed `oven-sh/bun` release | Tasks explicitly requiring Bun |
| Node | 22.22.0 | Fixed `nodejs.org/dist` release | Agent, Terminal, MCP and managed component launches; complete Node/npm/npx distribution |
| OfficeCLI | 1.0.156 | Fixed `iOfficeAI/OfficeCLI` release; npm identity `@officecli/officecli` | Read-only Office previews and document skills |

Versions, platform/architecture assets, URLs, upstream checksum provenance,
archive SHA-256, delivery policy, development version ranges, and Docker build-image
digests are in the manifest. Archive hashes authorize component downloads; extracted executable
hashes identify runtime artifacts and `VerifiedTool.digest`. These hashes need
not be identical.

Release builds embed OfficeCLI only. Optional Node and Bun downloads use
the same manifest; they are not application payloads or startup prerequisites. Debug builds can embed
OfficeCLI with `NOMIFUN_EMBED_OFFICECLI=1`. An unsupported target fails the build.
Builds still verify approved Node/Bun archives and derive their executable digests,
embedding only small identity metadata. A matching executable cache from the former
bundled delivery is verified and reused offline without trusting its stamp or
requiring another archive download. Component updates belong to validated
application releases, with matching probes, tests, and notices; there is no
background global npm installation or latest-version upgrade.

[`notices/manifest.json`](../../crates/backend/nomifun-runtime/notices/manifest.json)
records component notice provenance and versions. Optional nfagent, ONNX native
libraries, and browsers retain their domain supply mechanisms; the catalogue
references those authorities rather than introducing another ledger. NomiRelay connection, restart and restore automatically prepare its approved nfagent artifact when needed. Download progress, cancellation, checksum verification and atomic publication remain owned by the relay lifecycle.
The DIY matting model has an explicit product exception: after first app use, it
may install silently in the background under the authenticated app runtime. This
does not block application startup; unavailable or failed model installation keeps
the quick matting fallback. The model retains its own supply mechanism and is not
added to the managed tool catalogue.
User-configured MCP and terminal CLIs retain user ownership. Discovery failures
must remain distinct from successful empty configuration.

## Supply and admission

The existing-artifact resolver returns `VerifiedTool` with the
verified absolute executable path, observed version, and executable digest.
Read-only availability and diagnostic resolution admit only local artifacts. Actual launches automatically prepare a missing required approved runtime; OfficeCLI extracts its embedded payload locally. Managed supply uses
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
Later executable size or modification-time changes fail closed until repaired.
Node uses its complete official distribution, including npm, npx, modules and launchers.
The application creates no Bun-as-Node aliases and does not modify the system PATH.

`nomifun-runtime::prepare_process_launch` supplies the shared command and per-launch
user tool environment for Agent native processes, Terminal and MCP stdio, and may
prepare a missing approved runtime for the requested launch.
Explicit shell text remains shell text and receives the same composed environment;
native executables retain literal argv. Each launch combines the caller's environment,
current durable user CLI directories and verified runtime fallbacks without mutating
the multi-threaded host environment. Explicit PATH and available user toolchains take
precedence. The real HOME/USERPROFILE, user npm configuration, proxy and certificate
settings remain available. Managed npm defaults to durable user tools outside the
replaceable runtime cache, while an existing user prefix remains authoritative.

`nomifun-runtime::prepare_existing_process_launch` accepts only an absolute,
existing program file. It composes the same child environment and preserves literal
argv, but never invokes managed supply or creates directories. Office Hub
diagnostics, business calls and queries through an existing npm executable use this
entry point. Only the workbench's explicit **Prepare component** operation may use
the supplier-enabled entry point for a relative `npm` command. An existing native
Feishu CLI needs no Node simply because it resides inside an npm prefix; an unknown
script with a missing interpreter is unavailable rather than installing one during
diagnosis. Existing Agent, Terminal and MCP preparation behavior is unchanged.

Each child receives the host-derived, read-only `NOMIFUN_USER_CLI_ROOT` location
hint for `dirs::data_local_dir()/nomifun-user-tools/npm`. It is shared across
Agents and working directories. Skills that need a fixed installation location
pass it explicitly as npm's `--prefix`; the hint does not change the user's npm
prefix. Its command directory (the root on Windows, `bin` on macOS/Linux) is
included after user command directories. Through `prepare_process_launch`, CLIs
in either this root or a user npm prefix remain eligible for missing-Node preparation;
the existing-file entry point never grants that preparation authority.
Runtime upgrades and Agent data reset do not own the installed user tools. Hosted applications prepare required Node/Bun from recognized project metadata or their built-in static preset, with custom Node taking priority and cancellation attached to the original operation. Arbitrary shell text is preserved.

Terminal retains its existing Windows PowerShell shim adapter. The Windows native
Agent/MCP batch-launcher integration still requires implementation and native verification;
this macOS change does not claim their arbitrary `.cmd`/`.bat` argv contract is complete.
Process ownership, timeouts, cancellation, and tree cleanup remain in
`nomi-process-runtime`.

Debug-only `NOMIFUN_BUN_PATH`, `NOMIFUN_NODE_PATH`, and
`NOMIFUN_OFFICECLI_PATH` accept explicit absolute executable paths. Paths are
canonicalized and still pass engine, capability, and development version-range
checks. Release ignores these development overrides for managed component
admission. User command discovery and App Hosting retain their separate system
and custom-path selection.

Agent `workspace.process/start` accepts `component`, ordinary `command`
or explicit shell `cmd`. A component is prepared inside the original invocation and
verified again before start; OfficeCLI fixes `OFFICECLI_NO_AUTO_RESIDENT=1`.
Skill compilation preserves its frozen source and points to the callable component
contract; it does not cache readiness or executable paths. Only a new operation prepares
a component. Existing Effect receipts remain authoritative and unknown effects are not replayed.

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
verifies available local components, may extract embedded OfficeCLI into that
data root, and runs private capability probes. It never downloads missing Node/Bun. JSON observations include
`ready` / `unavailable`, actual version/path/digest, or the error; any failed
component yields a nonzero exit code. Neither mode initializes Agent data,
database storage, logging, or startup database rebuild authority.

Source developers can use `cargo run -p nomifun-app --bin nomicore -- tool-doctor --verify`.
A debug build without OfficeCLI embedded needs its explicit development
override; missing managed Node/Bun are reported as unavailable. The
separate `doctor` command discovers user CLIs through PATH; it does not replace
product component admission.

## First preparation and package size

Desktop and Web backend startup do not prepare or download Node. A fresh
installation can start without managed Node or Bun. Terminal and MCP UIs submit the execution request once. The owner resolves its
working directory and child PATH, preferring user tools before verified managed supply.
A missing necessary runtime is prepared automatically before the requested program starts.
The UI observes read-only progress; it neither asks for another download approval nor
replays a launch after preparation. Real preparation failures remain retryable failures.
Office conversion uses embedded OfficeCLI and needs no Node/Bun download or network.

Owner-authenticated `GET /api/runtime-tools/{tool}/availability` diagnoses local
components without downloading. `GET /api/runtime-tools/{tool}` observes preparation
state. `POST /api/runtime-tools/{tool}/prepare` explicitly starts or joins the same
preparation task without introducing a separate permission decision. Only `bun`, `node`
and `officecli` are accepted; callers cannot provide URLs, paths, versions or global
cancellation. Shared preparation retains integrity checks, temporary staging and atomic
publication. Cancelling an observer does not revoke another consumer's preparation.

Only the target architecture is delivered. Raising compression or deleting
source files does not substitute for excluding Node/Bun blobs from the release
payload. Compare actual App and DMG artifacts from the same release profile and
compression format; compressed component sizes alone are not installer measurements.

## Feishu Office CLI

The cloud [Office Hub](../guides/office-hub.md) uses the official
`@larksuite/cli`, with 1.0.98 as its verified compatibility baseline. It is separate
from the embedded OfficeCLI used for local file rendering. The Feishu CLI is not
embedded or exposed as a managed `component` target.

The connection dialog guides the user through environment preparation, connection
and read permissions. Missing components use one Host supplier after explicit
opt-in, reusing an existing compatible installation first. The fixed official npm
package goes into `user_cli_root`, under the shared-prefix preparation lock.
Existing external installations keep their original location and maintenance.
The full upstream wizard is only a command for the user to run explicitly; it may
also install global skills. Read-only update checks do not install or sync skills.

All business queries and saves use this CLI. Documents and Base resources open
in the browser; NomiFun offers document search, title creation and confirmed
deletion. Save submits one stable request, and a lost response is recovered by
that request identity. Pending document and Base targets retain their SQL write
fences across identity changes. The same process owner keeps retrying tree cleanup
until completion is proved; cancelling execution cannot end that cleanup early.
Office retains four product tables for connections and submitted request
receipts, separately from canonical Agent facts. The page opens directly on
Documents; see the [active plan](../handoffs/office-feishu-workbench.zh.md).

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
