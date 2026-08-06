# Changelog

NomiFun is pre-1.0. Until the first public release, this file records release
notes at a high level rather than a complete historical log.

## Unreleased

- **Fixed (appearance + accessibility).** Row separators, focus rings and keyboard
  focus outlines that never painted, and borders that painted 3px where the class
  said 1px. All of it is the same numeric-theme-key hijack as the entry below,
  reaching three more prefixes. Every one of the nine `divide-y` sites was broken —
  including the one that looked correct — by two stacked causes and a third nobody
  had seen: `divide-y` supplies only top/bottom *widths*, so the non-directional
  `divide-solid` put a style on all four sides and the unset left/right widths fell
  back to the CSS initial `medium`, growing a 3px vertical rule on every row after
  the first. There is no directional divide-style utility (`divide-y-solid` emits
  nothing), so `divide-x-0` is the fix. `ring-2` set only a ring *colour* custom
  property — no width, no box-shadow — so those focus rings did not exist; `ring-2px`
  is the width spelling. `outline-2` failed the same way — no width, and nothing in
  this project sets an `outline-style`, so the indicator did not render at all; it
  additionally overrode the colour with the `--bg-2` step, close enough to the panel
  behind it to camouflage what little was left, on a `border-0` button that had no
  other affordance.
  On the border side, `border-N`/`b-N` is a legitimate *colour* when another token
  supplies a real width (`border border-solid border-3`) and broken when it is the
  only border token; 34 of the latter now carry an explicit unit. Rule ordering
  mattered too: `.border-2` is emitted after `.border-[var(…)]`, so a selection ring
  was invisible in both states and six spinner tracks were silently repainted. 27
  dead colour names (`border-line`, `b-color-border-2`, `bg-border-2`, `color-text-3`,
  `text-error`, `text-t-error` …) are gone, along with the last `bg-fill-1/60`.

- The dead-CSS gate no longer plays whack-a-mole. Its seven regexes each target one
  *known* dead form — but every family in this effort was found by a mechanical
  sweep, and several had been live for months without any regex seeing them. So the
  gate gained a layer that does not enumerate mistakes at all: every token in the
  source that *looks like* a colour or decoration utility is compiled through the
  real UnoCSS generator, and anything emitting zero CSS fails. That closes the whole
  "compiles to nothing" class, including forms nobody has written yet. The
  discriminator is whether the leading segment is a colour/decoration prefix — not
  whether the class is defined in a stylesheet, which does not converge, because
  semantic hook names like `nomi-input` emit nothing either and would need an
  ever-growing allowlist. Two traps are pinned by its own self-test: numeric
  suffixes must NOT be excluded (bare integers are this project's colour rules;
  unit-suffixed ones are widths that compile anyway, so excluding them only buys a
  blind spot), and non-class strings must be — MIME types, CSS property names,
  `box-sizing` values, SVG data-URI fragments and prose beginning with `text-` all
  reach the token stream. `--self-test` now runs 117 cases in three parts, the last
  of which proves through the real generator that known-dead forms still compile to
  nothing and known-live ones still compile — so a broken detector fails loudly
  instead of passing silently. 186 tokens, one generator call, 24ms.

- `scripts/generate-i18n-types.mjs` fails with one actionable line instead of a
  loader stack when run under a runtime that cannot load TypeScript. It imports the
  shared plural-parity module (a `.ts` file), which made it bun-only; `package.json`
  only ever invoked it through bun, but a contributor's muscle memory would have hit
  `ERR_UNKNOWN_FILE_EXTENSION` with no mention of bun anywhere in the output. The
  import had to become dynamic for the guard to run at all — static specifiers are
  resolved before any statement executes — and the guard rethrows anything that is
  not a TypeScript-support error, so a genuine syntax error in the module is not
  disguised as "use bun".

- **Fixed (appearance).** Three tab underlines that did not exist, two spinner
  rings that painted nothing, 63 borders that were invisible, and 87 panel
  backgrounds that were transparent. All of it is one root cause: `uno.config.ts`
  merges the background ramp into the theme colour map under *numeric* keys, so a
  numeric suffix on a border-ish prefix is read as a COLOUR, not a length.
  `border-b-2 border-primary` on an active tab compiled to
  `border-bottom-color: var(--bg-2)` — no width, no style, and the author's colour
  overridden by a background variable. `border-3 border-fill-3` on the
  update-check spinner named a colour that does not exist. `bg-bg-1` looks up a
  colour literally named "bg-1" and emits nothing at all. Separately, this repo has
  no global border reset, so a border colour and width with no `border-style` also
  paints nothing — and the non-directional `border-solid` puts a style on all four
  sides, which makes the three sides that have no width class fall back to the CSS
  initial `medium` (~3px). Every site now uses same-direction classes
  (`border-b border-b-solid border-b-[…]`) or an explicit unit (`border-b-2px`).
  A global `*{border-width:0;border-style:solid}` reset was considered and
  rejected: it would strip default borders from native form controls across a
  thousand files, and it would silently turn the sites that pair `border-solid`
  with a colour-only `border-N` from 3px into nothing. The gate grew from four
  banned forms to seven and now analyses one class-list at a time — it had to learn
  which classes share an element — with a colour whitelist so an unknown token is
  not assumed to be a colour, and `border-*-0` classified as a width reset rather
  than a violation. Its self-test grew from 31 to 68 cases.

- **Fixed (accessibility).** The warning, error, success and info Alerts are
  legible in every theme. Light and dark are expressed here by *two* attributes —
  `html[data-theme]` and `body[arco-theme]` — and the Alert had a foot in each:
  Arco keys the surface off `--color-*-light-1`, whose dark values exist only under
  `body[arco-theme='dark']`, while the ambiance presets rewrite the *text* token
  under `[data-theme='dark'] body` with `!important`. Whenever the two attributes
  disagreed, the dark warning Alert rendered near-white text on Arco's light cream
  surface at **1.02–1.07:1** — effectively invisible. The info Alert was separately
  and unconditionally broken at 1.41:1, because the presets replace
  `--color-primary-light-1` with an opaque brand colour. All four types now derive
  from `data-theme`-keyed tokens via `color-mix()`, so the `arco-theme` dependency
  is gone and the desynchronised state measures identically to the normal one.
  Measured in a real browser across 72 theme × mode × type combinations: worst case
  11.25:1 for content text, 5.40:1 for body text under a title, 3.38:1 for icons —
  nothing below its floor, where before there were eleven. A new contract test
  resolves each theme's palette and recomputes the ratios, so a regression fails
  the suite rather than shipping.

- A skill's owner is named the same thing as a memory's. The physical column had
  already collapsed to `companion_skills.companion_id`, but the skill wire kept the
  retired spelling alive on a single `#[serde(rename = "scope_companion_id")]` —
  which is exactly how a wire and its column get to disagree without anyone
  noticing. The list route, the single-skill content route and `skills.jsonl` in
  every bundle now all say `companion_id`, and the UI/API contract version was
  bumped. Bundles written by an older build still import with their owner intact:
  the retired spelling is translated on the way in for both row types, which also
  let the per-caller `legacy_owner_key` parameter go away — one fewer thing for a
  future caller to forget. The skill adapter now REJECTS the retired name instead
  of ignoring it, matching the memory adapter, because a tolerated retired field is
  how a mismatched backend gets to serve rows whose owner every caller reads as
  `undefined`.

- The i18n gate is quiet again. `bun run check` had been printing two warnings on
  every single run: `browser.close.conversationContent_one` and
  `browser.diagnostics.hostCount_one` can never resolve in zh-CN, which has exactly
  one plural category. They could not simply be deleted, because
  `browserLocalization.test.ts` demanded literal key-for-key equality between the
  two locales — the dead keys were what kept the test green, and the warnings were
  what kept the gate noisy. The plural-aware rule the gate already had is now a
  single shared module imported by both, so the two cannot demand opposite things
  again, and the test additionally fails on any variant a locale can never resolve.
  A warning that prints on every build is not cosmetic: it teaches everyone to skim
  past the gate's output. Also corrected four comments in the desktop-companion
  window page that spelled its query parameter `?companionId=` when both ends of
  that URL have always used `?companion_id=` — verified by running the writer's
  format string through the reader's lookup rather than by reading them.

- **Fixed (appearance).** Coloured text, backgrounds, borders, rings and outlines
  render in the colour they were written to be, in 293 places across 98 files —
  warning and error glyphs that were silently inheriting the surrounding text
  colour, tinted panels that painted nothing, focus rings that never appeared.
  `{text,bg,border,ring,outline}-[rgb(var(--ramp-N))]` looks like it names a
  colour, but UnoCSS treats an arbitrary value as opaque and appends its own
  slash-alpha, producing `rgb(var(--danger-6) / var(--un-text-opacity))` — and
  because every ramp variable is a comma-separated triplet (`245,63,63`), the
  result is unparseable and the browser drops the whole declaration. The project's
  own `text-danger-6` / `bg-primary-6` rules emit the alpha-aware form correctly
  and are what these all use now. Three tests had pinned the broken class strings
  as expected output; they now compile the class with the real generator and assert
  a parseable colour, so they fail if the bug returns. The gate that guards this
  (`scripts/check-dead-css-utilities.mjs`) was a ratchet whitelisting the 95
  pre-existing files; with the sweep done its baseline is deleted and it is a flat
  prohibition over four banned forms, so a single new occurrence anywhere fails.

- **Fixed (data safety).** A factory reset left half-finished by an older build no
  longer risks a hard startup error on a build whose managed-root registry has
  moved on. The persisted plan is compared element-by-element against a *frozen*
  registry chosen by the plan's own version, so v1 and the v2 shape every current
  release writes each validate against the bytes they were written with; a plan
  version this build does not know is quarantined into
  `retired/unrecognized-reset-plans/` with a warning and reported as "no reset
  pending" instead of failing the boot path outright. A test reproduces the frozen
  v2 list from the live registry, so moving the registry without minting a v3 fails
  at development time rather than on a user's data directory.

- A memory's owner is named the same thing everywhere. The column collapsed to a
  single nullable `companion_memories.companion_id`, but the wire kept spelling it
  `scope_companion_id` through a `#[serde(rename)]`, so the REST shape, the query
  parameters, the ipcBridge contract and every UI reader disagreed with the
  database. All of them now say `companion_id` (the response field and the
  parameter on list / add / update / delete / batch / merge / merge-suggestions),
  and the UI/API contract version was bumped accordingly. Memory bundles written by
  an older build still import: an owner arriving under the retired name is
  translated on the way in, never dropped, and a row that already carries a live
  owner keeps it. Skill rows still ship their owner as `scope_companion_id` — that
  wire has not been renamed yet. The memory adapter now also REJECTS both retired
  names (`scope_kind`, `scope_companion_id`) instead of ignoring them, the guard the
  skill adapter already had: a tolerated retired field is how a mismatched backend
  gets to serve rows whose owner every caller then reads as `undefined`.

- Translations are now checked in both directions. `bun run check:i18n` only ever
  read en-US and diffed it against the generated key union, so a key present in one
  language and missing in the other passed silently and fell back to English at
  runtime; three had already slipped through (`ssh.sessionsOnline_other` missing
  from zh-CN, and stray zh-CN-only `cron.actions.runNow` / `settings.addPreset`,
  whose live counterparts are `cron.detail.runNow` and `settings.createPreset`).
  The gate now requires every shipped locale to carry the same keys and names the
  missing ones per language. It is plural-aware rather than a naive set diff:
  i18next resolves `_one` / `_other` through `Intl.PluralRules`, and Chinese has
  exactly one plural category, so en-US's `_one` is not demanded of zh-CN while the
  one category zh-CN does have is. A `--self-test` mode proves both halves on
  fixtures — it catches a genuine one-sided key and tolerates a legitimate plural
  difference — and runs as part of the gate.

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
  destroys the memories filed under it, so export the companion first if you want
  to keep them (the companion bundle now carries its memories by default). A zero-companion install
  remains supported: rows stay unowned until a companion exists again and are
  re-homed at a later launch, and a learning run with an empty roster exits early
  instead of writing an ownerless memory. Importing a memory bundle re-homes
  every row onto the local owner, since companion ids are not stable across
  machines. The UI/API contract version was bumped accordingly
  (`PUT /api/companion/memories/{id}` no longer accepts `scope_kind`, and its
  owner parameter no longer names a target scope — see the ownership-check
  entry below, where it becomes the *asking* companion; the owner parameter on
  `POST /api/companion/memories` now names the owner instead of meaning
  "private", `scope_kind` is gone from the
  memory shape the UI consumes, and `memories_active` / `memories_archived` plus
  the digest's `memories_added` are per-companion counts rather than install-wide
  totals). Collection, learning and evolution **configuration** was still
  install-wide at that point; the entry below makes learning and evolution
  per-companion too.

- **Migration (schema + data cleanup).** The two vestigial ownership columns are
  physically gone. `companion_memories` and `companion_skills` encoded "whose row
  is this" in a PAIR — `scope_kind` (`'user'` / `'companion'`) plus a nullable
  `scope_companion_id`, welded by a table CHECK into exactly `('user', NULL)` =
  unowned and `('companion', id)` = owned — even though the discriminator was
  fully determined by whether an owner was present. Both tables now carry ONE
  nullable `companion_id`, where `NULL` says exactly what `('user', NULL)` said.
  That shape, rather than "drop `scope_kind` and make the owner NOT NULL", is what
  made this safe: a zero-companion install stays representable, so deleting down to
  zero companions is still supported and the rebuild needs no "is every row owned
  yet?" precondition. The paired CHECKs, the two `scope_kind`-keyed partial
  indexes and the two-legal-states validation on every row read went with it.
  The rebuild runs once, in one transaction, on the first launch after upgrading
  and preserves every row verbatim — including each row's `id`, because the
  full-text index is external-content FTS5 anchored on it. **Downgrading remains
  impossible** (see the 建议 entry below: 0.3.8 validates an exact table set), so
  the six install-wide `companion_state` rows that were kept "in case of a
  rollback" (`learn_cursor_ts`, `evolve_cursor_ts`, `last_learn_ts`,
  `last_evolve_ts`, `learn_parse_fail_streak`, `mood`) are now deleted once every
  companion has its own copy — an install with no companions keeps them, since
  they are still the only record of how far the owner's loops had read the event
  spool. Memory and skill export bundles are unaffected in both directions: a
  bundle written by 0.3.8 (which also carries `scope_kind`) still imports — the
  retired discriminator is accepted and discarded, and a retired owner spelling is
  translated (see the field-rename entry below). `scope_kind` no longer appears in any
  response, so the UI/API contract version was bumped.

- **Behaviour change + migration.** 定时学习 and 技能进化 are now **per
  companion**, not install-wide. Each companion carries its own `learn`
  (`enabled` / `interval_minutes` / `model`) and `evolve` (`enabled`, a
  保守/激进 preference, `min_distinct_sessions`) block on its profile, runs its
  own loop on its own schedule from its **own cursor** into the shared raw-event
  spool, and owns everything the run produces — memories, mined skills, XP and
  mood alike. The 进化 tab therefore writes through `PATCH
  /api/companion/companions/{id}` instead of the shared config, and every "these
  settings currently apply to every companion" / "the output lands on the default
  companion" disclosure is gone because it is no longer true. Two consequences
  worth naming: learning-run XP is credited only to the companion that ran (it
  used to be granted to the whole roster), and **mood is per companion** (it used
  to be one global row, so whichever run finished last set everyone's mood).
  休眠时段 now gates the two background loops as well as the desktop bubbles —
  inside the window a companion neither interrupts you nor spends tokens; IM
  auto-replies are deliberately still answered.

  **On the first launch after upgrading, every existing companion is seeded from
  the current install-wide values**, so nobody's behaviour changes: an install
  learning every 25 minutes in 激进 mode keeps doing exactly that, on every
  companion. Each companion's event cursors and its mood are seeded from the
  retired global ones rather than from zero — seeding to zero would make every
  companion re-distill the entire retained event history on its first run
  (duplicate memories and a large unexpected LLM bill). The migration is additive
  and idempotent: a companion that already has settings or a cursor of its own is
  never overwritten, and `shared/config.json` is only rewritten without `learn` /
  `evolve` once the seeding has durably succeeded, so an interrupted upgrade
  replays cleanly. No table is rebuilt and no column is dropped. A companion
  created after the upgrade starts reading the spool from its creation time, for
  the same token-burn reason.

  Raw-event retention follows: an event day-file is deleted only once **every**
  companion with a consumer enabled has read past it, and a companion whose
  consumer is on but has no cursor yet protects everything. `SharedCompanionConfig`
  keeps `collect`, `archive`, `smart_collaboration`, `default_companion_id` and
  `bridge_to_memory_dir`; `PATCH /api/companion/config` no longer accepts `learn`
  or `evolve`, `POST /api/companion/learn/run` became `POST
  /api/companion/companions/{id}/learn/run` (companion-scoped, so one companion's
  run cannot serialise the others), and the MCP `learn`/`evolve` patch shape moved
  from `nomi_companion_update_config` to `nomi_companion_update`. Skill-pattern
  statistics and accept/reject feedback stay install-wide on purpose: a repeated
  tool sequence is a fact about how the owner works, and a rejection is the
  owner's judgement about that pattern rather than about a companion.

- **Memory ownership is now enforced where memory is stored, not merely respected
  by its callers.** `PUT /api/companion/memories/{id}`, `DELETE
  /api/companion/memories/{id}`, `POST /api/companion/memories/batch` and `POST
  /api/companion/memories/merge` used to address rows by memory id alone, so "a
  memory can only be changed by its owner" held only because a companion's
  workspace happens to know just its own ids. All four now require the asking
  companion (`companion_id` in the body; the query string for `DELETE`) and
  refuse anything that is not its memory with a `404` — never a silent no-op
  reported as success. It is the caller's identity, not a new owner: no wire can
  re-home a memory. The one cross-companion surface is the machine owner's MCP
  tools (`nomi_memory_update` / `nomi_memory_delete`), which pass an explicitly
  named "any owner" actor, because the owner agent has no companion identity of its
  own. `POST /api/companion/memories/merge-suggestions` is scoped the same way and
  requires `companion_id` too: it used to scan every companion's memories and
  let the client filter, which put other companions' memory **text** on a wire
  belonging to a single companion. The UI/API contract version was bumped
  accordingly.

- 导出伙伴 now also carries the companion's **mood**. `state.json` in a companion
  bundle grew a `mood` field beside `xp`, and import restores it, so a companion
  moved between machines keeps the mood it was in. Bundles written before this
  change have no such field and still import (the importing machine's default
  stands). The memory bundle's own `mood` field stays deliberately null and
  ignored — mood belongs to one companion, not to a whole memory hub.

- 聊天历史 (`/nomi` → 聊天历史) now reads a **server-side** day index:
  `GET /api/companion/companions/{id}/history/days` returns every local calendar
  day the companion's conversation holds visible messages on (plus every day with
  an archive digest), and `GET /api/conversations/{id}/messages` takes a new
  `day=YYYYMMDD` parameter that returns exactly that day, oldest-first. The day
  boundary is now computed once, on the server, in the same timezone that
  partitions archive digests — so the digest marker can no longer land on the
  wrong day near midnight, and a browser in a different timezone from the backend
  no longer mislabels days. The day rail is therefore complete: 「加载更早」 and
  the "the index only reaches {day}" footnote are gone. The reader still never
  mints a session, so it works for a companion with no model configured.

- 导出伙伴 now actually exports the companion. The bundle carries its
  `memories.jsonl` (on by default), optionally its skills (rows plus their
  `SKILL.md` bodies), and the custom figure image whenever the companion wears
  one; settings and growth progress are always included. `POST
  /api/companion/export/companions/{id}` gained `include_memories` (default
  `true`) and `include_skills` (default `false`), the response's `file_count` and
  `memories` are computed rather than hardcoded, and it reports a new `skills`
  count. Import re-homes every carried memory and skill onto the freshly minted
  companion id with fresh row ids, so a bundle from another machine can no longer
  leave a foreign owner behind (which would hard-fail the next boot's reference
  audit). Two latent import bugs are fixed on the way: a companion exported
  before its chat model was configured no longer fails to import, and a bundle
  claiming a custom figure it has no image for now drops the figure instead of
  producing an install that refuses to boot. Bundles written by the previous
  build (four entries, no memories) still import. The UI/API contract version was
  bumped accordingly.

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

- **Fixed (data safety, regression in 0.3.8).** Backups made by 0.3.1–0.3.7 can
  be restored again, and factory reset once more sweeps the leftover
  `browser-secrets/` directory. Removing the companion-credential subsystem in
  0.3.8 also deleted `browser-secrets` from the managed dataset-root registry,
  but that registry is a compatibility surface, not an implementation detail:
  backup coverage is validated as an *exact* match against it, so every bundle
  those releases wrote — all of which list `browser-secrets` — began failing
  verification with `InvalidManifest`. The same removal changed the persisted
  factory-reset plan shape, so a reset interrupted before the upgrade could fail
  plan validation and turn into a hard error at startup, and a "factory reset" on
  an upgraded install silently left the old credential directory behind. The root
  is restored as cleanup-only (no live code writes there) and a new test
  reproduces the frozen released registry from the live one, so a future removal
  fails loudly at development time instead of on a user's data directory.

- SSH remote sessions: save a remote Linux host's SSH credentials (password,
  private key with passphrase, certificate, or the local ssh-agent — all
  AES-256-GCM encrypted at rest and never returned in plaintext) under a new
  owner-scoped host book, then open a chat session bound to that host. The
  agent operates the remote host through its ordinary tools — `Bash`, `Read`,
  `Edit`, `Write`, `Grep`, `Glob` are transparently backed by a persistent
  remote shell (cwd/env persist across commands) and SFTP; the local machine
  is not involved. An optional per-host sudo password is injected at the
  transport layer on the remote sudo prompt, so the model never sees it. Host
  keys are learned into the operator's own `~/.ssh/known_hosts` on first
  connect (TOFU); a changed key blocks the connection. New `/api/ssh-hosts`
  CRUD + test-connection routes (instance-owner only) and a new `ssh_hosts`
  table (migration 024). Transport is `russh`/`russh-sftp` in a dependency-
  isolated `nomi-ssh` crate; the backend host book and connection pool live in
  `nomifun-ssh`. Security posture matches local execution by design — no extra
  approval gates or command interception. The UI/API contract version was
  bumped accordingly.

- SSH remote sessions, live link state: a host-bound session's connection is
  now held by a process-level pool keyed by (conversation, host), so it
  survives model switches and idle stretches instead of being rebuilt per
  turn — the remote cwd, exported environment and open files are still there
  on the next turn. A link that dies is noticed by a keepalive round trip and
  walked back up a 1s→60s backoff ladder (10 attempts) that replays the last
  cwd the remote shell actually proved, while a rejected credential or a
  changed host key stops the ladder instead of hammering the host towards an
  account lockout. Closing a session now reports what could be *proven* about
  the remote shell's exit rather than assuming it died, and deleting a
  conversation (or a host) takes its links with it. Every transition is pushed
  to the owner over `/ws` as `ssh.status` and the same shape is readable as a
  snapshot from the new `GET /api/ssh-hosts/statuses` route (instance-owner
  only), so a client that missed an event cannot be told a different story.
  The conversation header carries a host pill — which machine this session
  drives, whether the link is up, the retry countdown, and, when a retry
  cannot help, what has to be fixed by hand. Host-bound sessions also get
  their own top-level group in the session sidebar, so one is no longer
  unreachable after navigating away. The UI/API contract version was bumped
  accordingly.

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
