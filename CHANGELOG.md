# Changelog

NomiFun is pre-1.0. Until the first public release, this file records release
notes at a high level rather than a complete historical log.

## Unreleased

- Desktop companion management page (`/nomi`) rebuilt as a three-pane
  workspace: a companion sidebar (create, drag-reorder, 形象库), a seven-tab
  centre workspace (总览 / 记忆&知识库 / 远程控制 / 进化 / 技能 / 聊天历史 /
  其他), and an on-demand right detail pane. The previous page stacked two
  identical `Radio.Group` controls — an outer 伙伴/共享/形象库 "domain" switch
  above an inner tab switch — which existed only because half the settings were
  install-wide rather than per-companion. New in the rebuild: a per-companion
  chat-history reader grouped by day, and a persisted sidebar order
  (`order_index` on the companion profile). Removed from 总览: the counter row
  that summed memories/suggestions/skills across **all** companions while
  rendering inside a single companion's card.

- **Breaking / irreversible data change.** Companion memory is now strictly
  per-companion: every memory row belongs to exactly one companion, only its
  owner's rows are injected into that companion's prompts, and 记忆&知识库 lists
  the selected companion's memories only. 共享记忆 is gone as a product concept —
  there is no shared/all-companions scope left to write, and no way, in the UI or
  the API, to re-home a memory from one companion to another. **On the first
  launch after upgrading, memories that used to be shared by the whole family are
  re-homed onto a single companion**: the explicit default companion if it is
  still in the roster, otherwise the oldest one. Nothing is deleted or duplicated
  (ids, content, timestamps, strength and pinned/archived state are preserved),
  but every *other* companion permanently loses sight of them and this cannot be
  undone — if you want a particular companion to inherit the history, make it the
  default companion **before** launching this build. Deleting a companion now
  destroys the memories filed under it, and no bundle can bring them back (the
  companion bundle carries settings and growth only). A zero-companion install
  remains supported: rows stay unowned until a companion exists again and are
  re-homed at a later launch, and a learning run with an empty roster exits early
  instead of writing an ownerless memory. Importing a memory bundle re-homes
  every row onto the local owner, since companion ids are not stable across
  machines. The UI/API contract version was bumped accordingly
  (`PUT /api/companion/memories/{id}` no longer accepts `scope_kind` /
  `scope_companion_id`, `scope_companion_id` on `POST /api/companion/memories`
  now names the owner instead of meaning "private", `scope_kind` is gone from the
  memory shape the UI consumes, and `memories_active` / `memories_archived` plus
  the digest's `memories_added` are per-companion counts rather than install-wide
  totals). Collection and learning **configuration** is unchanged and still
  install-wide: one schedule, one learn model, one set of event sources for the
  whole machine.

- **Breaking / no downgrade.** The 建议 (Suggestions) feature is removed
  entirely — the `/nomi` tab, the desktop-pet unread badge, the detached
  suggestion popup window (`/nomi-memory-panel`), the learner's suggestion
  distillation, both `/api/companion/suggestions*` endpoints, the
  `companion.suggestion-created` / `-decided` WebSocket events, the
  `nomi_companion_list_suggestions` / `nomi_companion_decide_suggestion` agent
  tools, and the `companion_suggestions` table. The table is dropped on first
  launch after upgrading, which destroys any pending suggestion cards.
  **Downgrading to 0.3.8 after this upgrade fails at boot**: 0.3.8 validates an
  exact table set and refuses to start when a table is missing. Export anything
  you need before upgrading. The UI/API contract version was bumped
  accordingly (`suggestions_new` is gone from the companion status shape and
  `suggestions_added` from the learn result).
  The summoned-session `propose_companion_memory` capability went with it:
  suggestion cards were its only storage and its only review surface, so it has
  no confirm-before-write channel left. Restoring it needs a new design.

- WebUI realtime delivery behind reverse proxies: the `/ws` WebSocket
  handshake no longer rejects browsers whose proxy rewrites the `Host` header
  (e.g. nginx's default `proxy_set_header Host $proxy_host`). The
  browser-origin check now also accepts the first `X-Forwarded-Host` entry
  and an explicit `NOMIFUN_ALLOWED_ORIGINS` allowlist (comma-separated full
  origins). Previously every handshake in such deployments failed with a
  silent 403, so streaming replies, tool-call progress, and task/queue status
  only appeared after a manual page refresh. Rejected handshakes are now
  logged at WARN with the offending `origin`/`host`/`forwarded_host` values
  (once per distinct combination).

- AutoWork/IDMM: bypass-model (sidecar) decisions with confidence below 0.4
  now fall back to the conservative rule action instead of being applied
  verbatim, restoring the Phase-1 safety posture. Previously the floor was
  0.0, so the low-confidence fallback never triggered.
- Provider API: the retired `capabilities` field was removed from the
  provider wire shape (`GET/POST/PUT /api/providers*`). The backing column
  was dropped in an earlier migration and the field had been an
  accepted-and-ignored `[]` ever since. The UI/API contract version was
  bumped accordingly.

- Knowledge-base imports and companion (memory/companion bundle) imports now
  enforce zip-bomb limits: at most 256 MB of cumulative decompressed data and
  20,000 entries per archive. Oversized import bundles fail instead of
  exhausting disk/memory.

- `NOMIFUN_DATA_DIR` is now taken literally as the final data root on **every**
  host — the desktop shell no longer appends `/Nomi` to the env value, matching
  `nomifun-web` and `nomicore`. This fixes the 0.3.2 → 0.3.3 Windows
  auto-update failure "Conflict: work directory ... is already a NomiFun data
  root".
- New default data roots: the unset data directory is now the per-user
  `NomiFun` directory itself (`%LOCALAPPDATA%\NomiFun`,
  `~/Library/Application Support/NomiFun`, `$XDG_DATA_HOME/NomiFun`) instead of
  the nested `NomiFun/Nomi`. Non-stable channels use sibling directories
  (`NomiFun-dev`, `NomiFun-beta`) that are never nested inside the stable root;
  the extreme temp fallback is `<system temp>/nomifun-data`
  (dev: `nomifun-data-dev`).
- One-shot automatic migration: on the first boot after upgrading, an existing
  legacy dataset at `NomiFun/Nomi<suffix>` is moved into `NomiFun<suffix>`.
  The migration is crash-safe and resumes on the next boot if interrupted; if
  the old app instance is still running it is deferred to the next launch.
  Absolute paths persisted in the database (knowledge-base roots, terminal
  cwds, custom workspaces) are rewritten once after the move.
- Windows paths shown or persisted no longer carry the `\\?\` extended-length
  prefix.
- Work-dir resolution hardening: an inherited `NOMIFUN_WORK_DIR` that names a
  default data-root location or a directory that no longer exists is ignored,
  protecting against stale self-exported values across auto-update restarts.
- **BREAKING**: The 对外伙伴 (public companion / public agent) domain has been
  removed and replaced by the new 客服 (customer service) domain. Existing
  public-companion configurations are NOT migrated — recreate the agent under
  「服务 → 客服」, rebind its knowledge bases, service policy and channel bots
  there. `/api/public-agents` and `channel_plugins.public_agent_id` are gone;
  bot ↔ agent bindings now live in the customer-service domain
  (`PUT /api/customer-service/agents/{id}/bindings`).
- **BREAKING**: Presets targeting the old `public_companion` surface lose that
  target: `parse_target("public_companion")` now resolves to none (the existing
  unknown-target degrade semantic), so such presets simply stop offering the
  retired surface.
- New customer-service domain: stateless concurrent visitor dialogues over IM
  channels (per-agent concurrency ceiling, same-visitor serial merge), replies
  generated by disposable one-shot engine sessions whose tool registry is fixed
  at construction to three read-only tools (knowledge search / knowledge read /
  service notes) — dangerous capabilities are never registered, replacing the
  retired runtime `ExposureMode::PublicService` clamp.

## v0.3.3 - 2026-07-30

- Hardened the managed browser platform across lane/host lifecycle, startup,
  restart, cleanup, ownership lineage, and same-site resource handling.
- Improved turn execution UX with retry actions for failed turns, a live current
  step strip, stop-moment status copy, and verified deliverables surfaced in the
  conversation UI.
- Tightened agent/channel reliability around forwarded browser tool-call
  delivery, queued knowledge fetches, cleanup cancellation lineage, busy-turn
  handling, skill resolution, and AutoWork knowledge-service wiring.
- Fixed WeChat inbound pairing and configuration-status handling.
- Added focused regression coverage for browser platform, gateway, app, UI, and
  scripted browser/UI test lanes.
- Packaging note: this Windows-first GitHub release publishes the Windows x64
  installer and signed Tauri updater assets. macOS and Linux packages are not
  part of this first publication and should be appended from native build
  machines when available.
- The Windows installer is updater-signed but not Authenticode-signed, so manual
  downloads may show a SmartScreen or unknown-publisher warning.

## v0.3.0 - 2026-07-24

- Rebuilt the persistence and identifier architecture around a v3 data
  contract: local technical rows use integer identities, stable business
  references use canonical UUIDv7 values, and cross-domain relationships use
  explicit logical-reference policies.
- Added a guarded whole-dataset reset lifecycle for pre-v3 installations,
  including managed-root inventory, quarantine/retired-dataset receipts,
  crash-safe recovery, generation isolation, and stricter v3-only
  backup/restore validation.
- Improved conversation and agent reliability with idempotent message delivery,
  durable execution state, safer retries, stronger terminal/process cleanup,
  bounded knowledge writeback, and more consistent provider/model routing.
- Hardened AutoWork, requirement execution, scheduled-task delivery, channel
  routing, and notification synchronization across reconnects and retries.
- Added a Skill Market tab to the independent Skills capability, with bounded
  ClawHub and SkillHub ranking sync, tag/search filtering, localized skill
  descriptions, and a reviewed installation draft handoff to Nomi.
- Reduced bundled built-in skills and made OfficeCLI opt-in to keep default
  installations smaller and avoid injecting unused capabilities.
- **Breaking upgrade:** upgrading from an earlier data contract does not migrate
  local product data into v3. On first launch, the previous managed dataset is
  retired/quarantined and a clean v3 dataset is initialized. Dataset-owned
  credentials and integrations must be configured again; arbitrary external
  user workspaces are not deleted.
- Packaging note: this Windows-first release publishes the Windows x64
  installer and signed Tauri updater assets. macOS and Linux packages can be
  appended later from their native build machines.
- The Windows installer is updater-signed but not Authenticode-signed, so manual
  downloads may show a SmartScreen or unknown-publisher warning.

## v0.1.13 - 2026-07-01

- Improved orchestration reliability and control: DAG node pre-configuration,
  per-node model selection, explicit in-conversation approval before execution,
  and fixes for broken DAG lines, orphaned running nodes, one-node planning, and
  blank pending states.
- Added graceful handling for providers/models that do not support image input:
  image capability tracking, proactive image removal, retry without interrupting
  the conversation, and a visible in-conversation notice.
- Expanded browser-use controls with silent mode defaults, managed/system
  browser source selection, persistent encrypted browser login, a one-click
  browser login action, and screenshot context for silent approvals.
- Fixed WebUI credential persistence across restarts and added per-model context
  window configuration.
- Polished updater error handling, local update test clients, README screenshots,
  provider quick links, and contact assets.
- Packaging note: this Mac-side release publishes macOS installer and updater
  assets. Windows and Linux packages must be added later from their native build
  machines.

## v0.1.12 - 2026-07-01

- Documentation overhaul for public website and open-source preparation.
- Clarified desktop, web, remote access, AutoWork, scheduled tasks, and
  packaging documentation.
- Removed proprietary PDF skill assets from the bundled built-in skills.

## Release Note Policy

Every public release should include:

- User-facing changes.
- Breaking configuration or data migration notes.
- Security-relevant changes.
- Packaging and updater notes.
- Known limitations.

Use calendar dates or semantic versions consistently once public releases
begin.
