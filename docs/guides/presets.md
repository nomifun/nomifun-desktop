# Agent Workbench

> Updated to the current contract. The user-visible product name is **Agent
> Workbench**; the public UI route is `/agent`. `AgentPreset` is only the
> internal name of the backend authoring aggregate. Plugin Actions/Bindings are
> governed by Unified Plugin Core; this page does not define the Plugin author
> contract.

## Entry and shortest path

Open **Agent** (`/agent`) from the sidebar, then:

1. Create a new Agent;
2. Pick a creation seed: Lite, General, Full, or Custom;
3. Edit identity, instructions, model routing, capabilities, skills, and typed
   resources;
4. Review provenance, availability, and impact hints;
5. Save and try it out;
6. Continue in the resulting session at `/agent-sessions/:agentSessionId`.

Runtime, network, system, and provider management stay in global settings —
especially `/settings/execution-engines`. They are not part of what the Agent
Workbench edits.

## Domain boundaries

| Object | Owner | What the Agent Workbench can do |
| --- | --- | --- |
| Package / Agent Module | Agent platform internals | Read only officially materialized builtin sources and provenance |
| Unified Plugin Action | Plugin Core | Discover and invoke Agent Bindings of enabled Plugins by stable Action ID |
| Capability Catalog | Platform capability catalog domain | Query capabilities, sources, contracts, supported consumers, and availability |
| Skill Catalog | Platform skill catalog domain | Select instructions/workflows; a Skill itself is not an executor |
| AgentPreset | Agent authoring domain | Store user intent and the current Revision reference |
| AgentPresetRevision | Agent authoring domain | Store immutable payload, ContributionLocks, and the revision digest |
| Agent Session | Agent runtime domain | Consume the frozen Snapshot; never rewrites the Revision back |

Unified Plugin Core is not an Agent subsystem. Install, enable/disable,
configuration, Credentials, generation DataRoots, and Service lifecycle belong
to Plugin Core. The Agent only reads `agent.*` Bindings at the consumer
boundary and invokes Actions through the stable `plugin:<plugin_id>/<action_id>`
form.

## The four creation seeds

The four modes are creation-time seeds, not four persisted types:

| Seed | Initial content | What it does not auto-do |
| --- | --- | --- |
| Lite | Identity, instructions, model routing | Does not add tools, Workspace, MCP, or Plugins |
| General | Official common Capabilities/Skills | Does not absorb every extension the user installed |
| Full | Official coding baseline | Does not install or include every Plugin |
| Custom | Blank or optional seed | Does not allow entering internal IDs, digests, or runtime parameters |

Every creation flows into the same main chain:

```text
creation seed
  → AgentPreset Draft
  → canonical Compiler
  → AgentPresetRevision + ContributionLock[]
  → ResolvedSnapshot
  → AgentSession / AgentBinding
```

Template updates only affect Drafts created later; existing Revisions never
drift silently.

## Capabilities, skills, and provenance

The workbench queries the platform Capability Catalog. A catalog entry carries
at least: a stable capability ID, contract digest, owner, source Package or
Unified Plugin provenance, supported consumers, typed contributions, required
resources, and per-consumer availability.

Capabilities have no independent version number. Presets, dependencies, and
conflicts reference the stable capability ID only; builtin implementations are
frozen by Package version, contract digest, ContributionLock, and Snapshot
digest, so updating an implementation does not rewrite workbench references.

- Only builtin contributions that are enabled and materialized, plus Agent
  Bindings of enabled Plugins, enter the official discovery surface;
- Plugin Drafts, preview DataRoots, Credentials, and private generation
  DataRoots never enter an Agent Snapshot;
- Capabilities marked non-Agent-only do not appear in the Agent picker;
- Unavailable capabilities, contract mismatches, and missing resources surface
  explainable states and fail closed;
- Skills only supply instructions/workflows and resource descriptions; they do
  not auto-expand a Snapshot.

When multiple legitimate implementations exist for the same canonical
capability, the user must explicitly pick a source in the capability detail.
The server then produces the ContributionLock; the frontend never submits
internal Artifact digests to drive execution.

## Revisions and Snapshots

User edits live in a non-executable Draft first. A successful Save atomically
creates an immutable `AgentPresetRevision` and advances the current Revision
pointer. Preview, Save, and Test share the same canonical Compiler; Test
creates a normal AgentSession — there is no test-only session type.

The Revision digest covers the normalized:

```text
payload + contribution_locks
```

The Snapshot additionally freezes everything this run actually needs:
Capabilities, Provider/Model route, Tool schemas, typed resource bindings,
Runtime features, initial/on-demand grouping, and the Snapshot digest. Session
Open reads the saved Snapshot; it does not re-pick the latest source on every
Turn and never silently switches Providers or falls back.

Relevant migrations:

- `061_agent_snapshot_naming.sql` renamed the physical columns of
  `conversations`, `agent_execution_participants`,
  `agent_execution_template_participants`, and `cron_jobs` to `agent_snapshot`,
  with no alias or dual write;
- `062_agent_preset_contribution_locks.sql` added `contribution_locks_json` to
  `nomi_agent_preset_revisions` and requires it to be a valid JSON array;
- `preset_snapshot` in the old baseline is only a migration source, not a
  current runtime field;
- Where `preset_id`/`preset_revision` still appear, they only record AgentPreset
  provenance — they do not revive the old resolver or compatibility model.

## Canonical API (machine resource names)

The workbench uses these canonical APIs; the stable machine names do not mean
the UI must display "Preset":

| Purpose | Endpoint |
| --- | --- |
| Official creation seeds | `GET /api/agent-preset-templates?source=official` |
| Create an AgentPreset | `POST /api/agent-presets` |
| Create a Draft from an official seed | `POST /api/agent-presets/from-template/{template_id}` |
| Read the editor | `GET /api/agent-presets/{preset_id}/editor` |
| Preview / Save / Revision | `/api/agent-presets/{preset_id}/...` |
| Platform capability Catalog | `GET /api/capabilities` |
| Agent Session | `/api/agent-sessions/*` |
| Agent Binding | `/api/agent-bindings/*` |

Clients must not submit Snapshot digests, internal Revision IDs, complete
Bindings, or raw canonical JSON. The server owns owner checks, Catalog resolve,
ContributionLocks, Revision/Snapshot digests, and typed failures.

## Implementation boundaries

For the current implementation of Agent revisions, Snapshots, Sessions, and
Module authorization, see
[Agent Session architecture](../architecture/agent-session.md).

For Plugin creation, preview, install, configuration, and Backup, see
[Plugin Platform architecture](../architecture/plugin-platform.zh.md)
(Chinese only).
