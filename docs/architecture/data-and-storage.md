# Data and Storage

NomiFun keeps its state in three places: a SQLite database (the source of
truth for everything structured), a per-installation **data directory**
(database file, logs, OS-cached runtimes), and per-conversation **work
directories** that hold the files agents read and write. This page explains
what lives where, how it's named, and how it's protected.

## The data directory

| Host | Default path | Override |
| --- | --- | --- |
| Desktop (`nomifun-desktop`) | Per-user app data: `%LOCALAPPDATA%\NomiFun\Nomi` on Windows, `~/Library/Application Support/NomiFun/Nomi` on macOS, `$XDG_DATA_HOME/NomiFun/Nomi` (usually `~/.local/share/NomiFun/Nomi`) on Linux. With `NOMIFUN_DATA_DIR` set, becomes `$NOMIFUN_DATA_DIR/Nomi`. Legacy installs under `<system temp>/nomifun-data/Nomi` are auto-relocated on launch (one-shot; the old dir is kept as a backup). | env `NOMIFUN_DATA_DIR` |
| Web (`nomifun-web`) and the `nomicore` bin | The **same** per-user directory as the desktop shell — `%LOCALAPPDATA%\NomiFun\Nomi` / `~/Library/Application Support/NomiFun/Nomi` / `$XDG_DATA_HOME/NomiFun/Nomi` (the old `./data`-relative default is gone). With `NOMIFUN_DATA_DIR` set, the value is taken **literally** (no `/Nomi` suffix), so Docker `/data` and systemd `/var/lib/nomifun` deployments are unaffected. | flag `--data-dir` or env `NOMIFUN_DATA_DIR` |

Inside the data directory:

```
<data_dir>/
├── nomifun-backend.db   SQLite database (sqlx)
├── server.lock          exclusive server-lock address file (the lock lives on
│                        the open OS handle; a leftover file is harmless)
├── logs/                tracing-appender file output (rotated daily)
├── conversations/       per-conversation workspaces (see below)
└── companion/                 companion file domain (shared memory hub + per-companion profiles, see below)
```

All three hosts resolve the unset default through one shared helper,
[`nomifun_app::cli::default_data_dir()`](../../crates/backend/nomifun-app/src/cli.rs):
`dirs::data_local_dir()/NomiFun/Nomi` (the per-user application-data
location), with the system temp dir (`<system temp>/nomifun-data/Nomi`)
only as an extreme fallback when the OS reports no user dir. Env semantics
stay host-specific: the desktop shell appends `"Nomi"` to `NOMIFUN_DATA_DIR`
(see [`apps/desktop/src/main.rs`](../../apps/desktop/src/main.rs)), while
`nomifun-web` and `nomicore` take the env value literally (a clap `env`
binding — new for `nomicore`, which previously ignored the variable).
A pre-existing legacy install under `<system temp>/nomifun-data/Nomi` is
relocated to the new location once at launch
([`apps/desktop/src/relocate.rs`](../../apps/desktop/src/relocate.rs)):
data is copied (regenerable caches/logs are left behind), the legacy dir is
kept as a backup, and the backend then rewrites absolute paths stored in the
database (knowledge-base roots, conversation workspaces, terminal cwds) to
the new root.

### One directory, one state

Sharing one default across every host is deliberate: the dev loops
(`bun run serve:web`, `dev:web`, `dev`) and the installed desktop app
read and write the same state, so a provider or companion configured once is
testable everywhere, and troubleshooting only ever has one directory to
look at. When you *do* want an isolated sandbox, `NOMIFUN_DATA_DIR` or
`--data-dir` is the escape hatch. (The dev scripts no longer pass a
repo-relative `--data-dir`; the old `data/` and `.dev-data/` directories
are not read by anything and their contents are **not** auto-migrated —
copy them into the new root or point `NOMIFUN_DATA_DIR` back at them if
you still need them.)

What makes the sharing safe is an **exclusive server lock**: at boot
(`bootstrap::init_environment`, before the database is opened) the backend
takes an OS-level exclusive advisory lock on `{data_dir}/server.lock`
(`fs2`: `flock` on Unix, `LockFileEx` on Windows). The OS releases the lock
when the process exits *or crashes*, so a leftover `server.lock` file is
harmless and needs no staleness heuristics. A second backend on the same
directory fails fast with an error naming the holder (pid + exe) and the
two ways out: close the other instance, or point this one at its own
directory. The desktop shell now surfaces a backend-startup failure in a
native error dialog and exits (previously a silent white window).
`nomicore doctor` and the `mcp-*` stdio subcommands are unaffected by the
lock (`doctor` is designed to run alongside a live server).

## SQLite via `sqlx`

[`nomifun-db`](../../crates/backend/nomifun-db/) is the data layer. Highlights
from [`crates/backend/nomifun-db/src/lib.rs`](../../crates/backend/nomifun-db/src/lib.rs):

Persisted entity identity follows the canonical prefixed UUIDv7 contract in
[`id-system.md`](id-system.md). SQLite row order and auto-increment values are
not portable entity identities.

- `Database` — owns the `sqlx::SqlitePool` and the migrations. Exposed via
  `nomifun-db::SqlitePool` re-export.
- `init_database` — opens the file, runs embedded migrations.
- `init_database_memory` — in-memory variant used by tests.

The crate exposes ~20 repository **trait + Sqlite-impl** pairs. A non-exhaustive
list (see the `pub use repository::{...}` block in `lib.rs` for all of them):

| Trait | Sqlite implementation | Stores |
| --- | --- | --- |
| `IUserRepository` | `SqliteUserRepository` | Users and password hashes |
| `IConversationRepository` | `SqliteConversationRepository` | Conversations + messages, with filters and full-text search rows |
| `IAgentMetadataRepository` | `SqliteAgentMetadataRepository` | ACP handshake results, available models, agent-binary metadata |
| `IAcpSessionRepository` | `SqliteAcpSessionRepository` | Persistent ACP sessions for resume after restart |
| `IMcpServerRepository` | `SqliteMcpServerRepository` | Configured MCP servers (CRUD) |
| `IOAuthTokenRepository` | `SqliteOAuthTokenRepository` | Encrypted OAuth tokens for HTTP MCP servers |
| `IProviderRepository` | `SqliteProviderRepository` | LLM provider credentials (encrypted) |
| `IRemoteAgentRepository` | `SqliteRemoteAgentRepository` | Remote-agent endpoints |
| `IAgentExecutionRepository` | `SqliteAgentExecutionRepository` | AgentExecution, immutable Participants, revisioned Steps/Dependencies, Attempts, Conversation Links, and the Event outbox; see the [unified model](agent-execution.zh.md) |
| `IRequirementRepository` | `SqliteRequirementRepository` | AutoWork requirements (intentionally **no foreign key** to conversations — the loop survives conversation deletion) |
| `ICronRepository` | `SqliteCronRepository` | Scheduled tasks and their timezone-normalized expressions |
| `ITerminalRepository` | `SqliteTerminalRepository` | Terminal session metadata |
| `IPresetRepository` / `IPresetStateRepository` | `SqlitePresetRepository` / `SqlitePresetRepository` | Relational presets and per-user selection state |
| `IChannelRepository` | `SqliteChannelRepository` | External chat-channel plugin configs (Telegram / Lark / DingTalk / WeChat) |
| `IClientPreferenceRepository` | `SqliteClientPreferenceRepository` | Per-client preferences |
| `ITagSettingRepository` | `SqliteTagSettingRepository` | Tag-based grouping (used by AutoWork) |
| `ISettingsRepository` | `SqliteSettingsRepository` | Misc app settings |
| `IWebhookRepository` | `SqliteWebhookRepository` | Outbound webhook destinations (Lark) |

A few row-update params types travel alongside (`UpdateAgentHandshakeParams`,
`ConversationFilters`, `ConversationRowUpdate`, `MessageRowUpdate`,
`MessageSearchRow`, `UpdateCronJobParams`, `UpsertOAuthTokenParams`,
`CreateProviderParams`, `UpdateRemoteAgentParams`,
`CreateAgentExecutionParams`, `ReconcileAgentExecutionPlanParams`,
`SettleAgentExecutionAttemptParams`, etc.). Repository traits are the feature
contract. Domain services use them rather than the pool; narrowly scoped
bootstrap/schema maintenance remains the documented exception.

### Migrations

Migrations are SQL files embedded with `sqlx::migrate!`. They run on every
boot inside `init_database`. Schemas evolve forward only; downgrades are not
supported.

### Scheduled-task ownership

`cron_jobs.user_id` is the non-null, immutable owner of the scheduled-task
aggregate, not a request-time hint inferred from a Conversation. A new task
receives the authenticated canonical user ID explicitly. Optional Conversation
bindings must already have the same owner; a missing target, multiple inverse
owners, or disagreement between the two directions is rejected rather than
guessed or silently repaired.

Public HTTP, Gateway, service, and repository operations all carry `user_id`;
cross-owner access is indistinguishable from a missing job. The scheduler is
the only internal global-id reader, and its timer captures the owner and
re-verifies that pair against the persisted row before execution, closing
delete/recreate races. Database triggers require both directions of an optional
Conversation binding, as well as Conversation Artifacts produced by the job, to
have the same owner; artifact status writes are owner-filtered before mutation.
Ownership cannot be moved in place. There is no runtime installation-owner
fallback. Scheduled work has one target—an Agent—so target type and legacy
terminal-only fields are not represented in the ID-v2 domain model, API, or
baseline schema.

### Installation execution authority

The canonical user referenced by `installation_identity.owner_user_id` is the
installation owner. The owner may
start host runtimes and use files, terminals, skills, presets, knowledge mounts,
Office preview and Platform Gateway capabilities. Every other authenticated
principal is limited to ordinary Nomi model calls in Conversations and
scheduled tasks; identity, role text or open-ended JSON cannot widen that
authority.

Migration 041 performs the hard cut: it canonicalizes retained secondary-user
Conversations and scheduled-task model selection, disables rows that have no
usable model, tombstones recoverable execution graphs, removes secondary
templates and terminals, and installs ownership/model-only triggers. Startup
reconciliation also deletes secondary or orphan scheduled-skill directories,
because SQLite migrations cannot remove filesystem state. Loopback capability
roots and renewable leases are process memory only and are never persisted.

### Per-conversation foreign-key note

`requirements` (the AutoWork queue) intentionally has **no foreign key** on
`conversation_id`. The persistent AutoWork runner (`nomifun-requirement`) is
backend-authoritative and survives conversation deletion — the FK would couple
its lifecycle to the conversation's, defeating the boot-resume design.

## Encryption at rest — AES-GCM

Sensitive strings (provider API keys, OAuth tokens, channel-bot tokens, ...)
are encrypted before insertion using AES-256-GCM via
`nomifun_common::crypto::{encrypt_string, decrypt_string}` and the
data-encryption key loaded by `nomifun_app::load_or_create_data_encryption_key`.

The master key is a per-installation file at `<data_dir>/encryption_key`.
Older installs did not have that file, so the first boot on the newer code
seeds it from the currently resolved JWT secret to keep existing ciphertext
readable. After that, password changes and JWT rotation do not alter the data
key. Losing `encryption_key` renders encrypted columns unreadable.

The `aes-gcm` crate version pinned in the workspace is `0.10`.

## Per-conversation workspaces

Each conversation owns a directory the agent can freely read and write:

```
{work_dir}/conversations/{label}-temp-{workspace_token}/
```

- `work_dir` — the runtime work directory; falls back to the data dir when
  not set explicitly. Sources, in order: `--work-dir` flag → env
  `NOMIFUN_WORK_DIR` → `<data_dir>`.
- `label` — a short slug derived from the conversation title.
- `temp` — literal string; signals these directories are mutable scratch
  space the user can also drop files into.
- `workspace_token` — a backend-minted `ws_…` token stored as
  `extra.temp_workspace_id`. It identifies this managed workspace without
  overloading the canonical conversation entity ID.

For a conversation without a user-selected workspace, the directory is
provisioned immediately after the conversation row is created.
On conversation deletion the directory is removed (the
`OnConversationDelete` hook in `nomifun_common::hooks`). File operations
inside it are sandboxed and watched:

- [`nomifun-file::path_safety`](../../crates/backend/nomifun-file/src/path_safety.rs)
  rejects paths that escape the workspace (e.g. via `..` or absolute roots).
- [`nomifun-file::watch_service`](../../crates/backend/nomifun-file/src/watch_service.rs)
  uses `notify` to surface filesystem changes back to the SPA over WS.
- [`nomifun-file::snapshot_service`](../../crates/backend/nomifun-file/src/snapshot_service/)
  records before/after snapshots for tool-edit auditability.

The repo enforces an extra constraint via
`nomifun_common::error::workspace_path_has_edge_whitespace_segment`: no
directory name in a workspace path may begin or end with whitespace (or
consist entirely of whitespace). Such names break Win32 path round-tripping
and are visually indistinguishable in any UI. Interior whitespace is fully
supported — the default per-user data dir on macOS
(`~/Library/Application Support/NomiFun/Nomi`) contains a space, and every
process-spawn pipeline passes the workspace as a discrete argument
(`Command::current_dir`, PTY cwd, ACP session JSON), which is
whitespace-safe.

### Knowledge-base mounts (`.nomi/knowledge/`)

When a conversation, terminal session, or companion binding brings knowledge
bases into a workspace, they are mounted under
`{workspace}/.nomi/knowledge/` — the same `.nomi/` domain as project
skills — as junctions/symlinks with a copy fallback, plus a built-in
`.gitignore` so mounts never enter version control. A platform-managed
`README.md` (retrieval protocol, per-base digests + TOC, write-back
rules) is rewritten there on every launch. Legacy mounts under the old
`{workspace}/.nomifun/knowledge/` location are cleaned up automatically
on the next sync.

## Companion data (the `companion/` file domain)

The virtual companion's data deliberately stays **out of the main database's
migration system** — it is a file domain that can be exported or wiped
as a whole (see the [Companions guide](../guides/companions.md)). The multi-companion
layout:

```
<data_dir>/companion/
├── shared/                      shared memory hub (one copy for all companions)
│   ├── config.json              SharedCompanionConfig: collect switches, learn interval & model, default_companion_id
│   ├── events/YYYYMMDD.jsonl    raw events from the collection pipeline (privacy-sensitive; export is opt-in)
│   └── memory.db                standalone SQLite (PRAGMA user_version ladder):
│                                shared memories/suggestions/learn history + per-companion runtime
│                                state (companion_runtime_state: XP, …)
└── companions/
    └── {companion_id}/                companion_{uuid_v7}; the directory is the source of truth
        └── config.json          CompanionProfileConfig: name/character/persona/per-companion model/desktop-companion toggle & position
```

The legacy single-companion layout `companion/nomi/` is migrated automatically on
first boot into `shared/` plus a first companion named "Nomi"; the old
directory gets a `.migrated` marker and is kept around (cleanup after
one release cycle).

Knowledge bases bound to companions do not live in the `companion/` domain: the
bindings are stored in the main database as
`knowledge_bindings('companion', companion_id)`, and the base content lives in the
knowledge bases' own managed directories (URL-sourced bases keep their
fetched markdown snapshots in a `snapshots/` subdirectory there).

## Bundled bun runtime

NomiFun ships its own `bun` runtime (1.3.13) so MCP servers and tool
subprocesses do not require a system Node.js install:

| Step | What happens |
| --- | --- |
| Build time | The bun binary for the target OS/arch is **zstd-compressed** and embedded into `nomifun-runtime` via `include_dir!`. |
| First run | `nomifun_runtime::init(&data_dir)` extracts the binary into a **`<data_dir>/runtime/`** subtree (see the runtime-cache details below). |
| Boot | `enhance_process_path()` prepends the bun bin dir to the process `PATH` **before any tokio thread is built** (the order is enforced in both host `main.rs` files). |
| Spawn | `nomi_process_runtime::ChildProcessBuilder` inherits the boot-time merged `PATH`, so `npx`, `bun`, and other JS tools resolve correctly. |
| Cleanup | `nomi_process_runtime::ProcessSupervisor` or `kill_process_tree` owns and reaps Agent / MCP child-process trees. |

The runtime cache is anchored to the backend's `data_dir`:
[`nomifun_runtime::init(&data_dir)`](../../crates/backend/nomifun-runtime/src/cache.rs)
records `<data_dir>/runtime` as the cache root, so on the desktop the bun
binary extracts under `<data_dir>/runtime/bun-<version>-<sha12>/` —
i.e. `%LOCALAPPDATA%\NomiFun\Nomi\runtime\bun-…\` by default on Windows
(the per-user app-data equivalents on macOS/Linux), or
`$NOMIFUN_DATA_DIR/Nomi/runtime/bun-…/` when the env var is set. When
`init` has not been called (the `mcp-*` subcommands, unit tests, `build.rs`)
the cache falls back to the platform cache dir via `dirs::cache_dir()`:
`%LOCALAPPDATA%\nomifun\runtime\` on Windows, `~/Library/Caches/nomifun/runtime/`
on macOS, `$XDG_CACHE_HOME/nomifun/runtime/` (or `~/.cache/nomifun/runtime/`)
on Linux.

## Logs

Logs go to `<data_dir>/logs/` via `tracing-appender`. The default level is
`info`; override with `--log-level` (e.g. `--log-level info,nomifun_mcp=trace`)
or env `RUST_LOG`. The desktop shell additionally keeps a console attached
in debug builds (the release build sets `windows_subsystem = "windows"`).

The logging configuration types — `ResolvedLogging`, `create_file_layer` —
live in `nomi_config::logging` (the agent layer's config crate). The
backend reaches them through the seam: `nomifun_ai_agent::nomi_config::logging::*`.

## First-run state

On a brand-new install the boot sequence is:

```
1. nomifun-runtime::init           extract bun into OS cache
2. enhance_process_path             prepend cache bin dir to PATH
3. bootstrap::init_environment      resolve work_dir / log_dir, init tracing,
                                    take the exclusive {data_dir}/server.lock
4. bootstrap::init_data_layer       open database, run migrations
5. AppServices::from_config         instantiate every service
6. ensure_admin_credentials (web)   pre-seed admin if NOMIFUN_ADMIN_PASSWORD is set
7. create_router → axum::serve      bind and start serving
```

Step 3 is where a second backend on an already-claimed data dir fails fast
(see "One directory, one state" above).

In the desktop shell step 6 is skipped, but the desktop is not the old blanket
`--local` story: it uses `TrustLocalToken` and trusts only its own WebView's
per-boot secret. In the web host, if no admin exists and no
`NOMIFUN_ADMIN_PASSWORD` is set, the install enters **interactive first-run
setup**: the next browser visitor chooses a username and password through
`POST /api/auth/setup`. A warning is logged if first-run setup is exposed on a
non-loopback bind address.

## Backups and reinstall

- **Database** — create a consistent SQLite snapshot with the SQLite Backup API
  or `VACUUM INTO` while the database is open. Do **not** copy
  `nomifun-backend.db` directly: WAL data may still be in
  `nomifun-backend.db-wal`, and a raw file copy can be incomplete.
- **Bundle manifest** — record schema version, storage-generation/dataset ID,
  creation time, and checksums for every included file. Restore preserves
  entity IDs; merge import rejects same-ID/different-content conflicts.
- **Encryption key** — the offline bundle includes
  `<data_dir>/encryption_key` when present. Without this file, provider API
  keys, OAuth tokens, channel bot tokens, and other encrypted columns cannot
  be decrypted.
- **Workspaces** — the bundle recursively includes only the backend-managed
  `<work_dir>/conversations/` tree. User-selected/custom workspaces elsewhere
  on disk are external user projects and are never copied implicitly.
- **Companion data** — the bundle recursively includes
  `<data_dir>/companion/` (shared memory hub + per-companion profiles; see the
  [Companions guide](../guides/companions.md)).
- **Bun runtime cache** — disposable; will be re-extracted on next boot.

Offline CLI commands are provided by `nomicore`:

```text
nomicore --data-dir <source> backup --output <bundle-dir>
nomicore restore --bundle <bundle-dir> --destination-data-dir <new-data-dir>
```

`backup` acquires the per-data-directory `server.lock` before opening SQLite,
so it fails instead of racing a live backend. It resolves `work_dir` with the
same CLI/persisted/environment rules as server boot. The output directory must
not already exist and must be outside both source roots. Backup opens the
existing ID-v2 database without running migrations, recovery, or quarantine;
an invalid source fails closed. A complete bundle contains the WAL-safe database snapshot,
the persistent encryption key when present, the companion file domain, and
managed conversation workspaces. Logs, `server.lock`, database WAL/SHM
sidecars, runtime/Bun caches, browser profiles, process/session scratch data,
and custom external workspaces are excluded.

Every payload file has a portable relative path, byte size, and SHA-256 digest
in the manifest; directory entries preserve empty companion/workspace
directories. Backup and restore reject symlinks, Windows junctions/reparse
points, path traversal, special files, undeclared payload files/directories,
and bundles above 8 GiB per file, 64 GiB total, 200,000 files, or 200,000
directories; the JSON manifest itself is capped at 64 MiB. `restore` verifies the complete bundle
before writing, accepts only an absent or empty destination, stages and
validates all files beside the destination, and installs the data directory
with one rename. A failure leaves no partial destination. All entity IDs are
preserved, while a new `storage-generation` is written so browser caches from
the source dataset cannot be mistaken for restored state. Managed workspaces
from a source custom work directory are intentionally rebased to
`<destination-data-dir>/conversations`; custom external workspaces must be
backed up separately by their owner.

The bundle contains the database encryption key and encrypted credentials.
Treat the entire bundle as sensitive data; store and transfer it with the same
access controls as the live data directory. If encrypted rows exist while the
persistent key is missing, or if the key file is invalid, backup refuses to
create an unrestorable bundle.

The restore command has no destination `--work-dir` option: it intentionally
creates the managed workspace tree below the new data directory. To use a
separate work root, move that restored managed tree and set the normal
work-directory override before the first server boot; never point restore at
an existing external project.

These commands implement full offline backup/restore. The logical Merge/Clone
rules described by the ID contract are not yet exposed as a SQLite CLI
operation; the CLI must not be described as providing merge or clone.

A clean uninstall therefore deletes the data dir, the work dir (if set
separately), and the OS cache dir.

## Cross-references

- The repository traits and their consumers are catalogued in
  [`backend-crates.md`](backend-crates.md).
- The HTTP routes that hit each repository, and the WS topics that mirror
  state changes, are summarized in [`communication.md`](communication.md).
- The agent-side data (TOML config, skills, file cache) is described in
  [`agent-engine.md`](agent-engine.md).
