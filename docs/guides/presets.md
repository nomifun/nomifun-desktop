# Agent Workbench

> Updated to the current contract. The user-visible product name is **Agent
> Workbench**; the public UI route is `/agent`. `AgentPreset` is only the
> internal name of the backend authoring aggregate. Application MCP tools reuse
> current session selection; process ownership belongs to application hosting.

Open **Agent** (`/agent`) from the sidebar to edit an official Agent directly or create a personal Agent. The official list includes Chat, General, Coding, Companion, Customer Service, and Creation.

## Edit an official Agent

1. Select an Agent from the official list, such as **Companion**.
2. Enable or disable capabilities and individual actions. Choose compatible implementations on the components tab.
3. Edit the name, description, persona, instructions, and runtime defaults.
4. Configure an available Chat model, then select **Save**.

Saving updates the current user's official Agent configuration and keeps the same Agent identity. Creating a copy is optional. Other users have separate configurations. Future sessions launched from that official product entry use the saved configuration. Official template updates preserve capabilities, instructions, and component choices saved in the current dataset.

The official Companion keeps its two core capabilities, Companion and Companion memory. Other capabilities and actions remain editable.

Existing sessions keep their frozen configuration. For a desktop companion, select **Apply saved updates** in the companion overview or chat controls, review the capability changes, and confirm. This updates the same idle session. Active or paused turns and unsettled effects block the update. The companion's model and still-required resource selections and parameters are retained; resources removed with disabled capabilities appear in the preview. Configure missing resources required by newly enabled capabilities before applying. If the saved configuration changes after preview, preview and confirm again.

| Object | Owner | What the Agent Workbench can do |
| --- | --- | --- |
| Package / Agent Module | Agent platform internals | Read only officially materialized builtin sources and provenance |
| Application MCP tools | Application hosting and current MCP owner | Discover application sources and explicitly select tools for Agent/session use without launching programs |
| Capability Catalog | Platform capability catalog domain | Query capabilities, sources, contracts, supported consumers, and availability |
| Skill Catalog | Platform skill catalog domain | Select instructions/workflows; a Skill itself is not an executor |
| AgentPreset | Agent authoring domain | Store user intent and the current Revision reference |
| AgentPresetRevision | Agent authoring domain | Store immutable payload, ContributionLocks, and the revision digest |
| Agent Session | Agent runtime domain | Consume the frozen Snapshot; never rewrites the Revision back |

Current application files, commands, processes and MCP addresses belong to
the hosting owner. Agents discover and call explicitly selected tools through
the current MCP chain without a second writable source registry or process.

Select **Create a copy** when you need an independent personal Agent. This creates a new identity and leaves the saved official configuration intact. Personal Agents can be deleted; official product entries are retained.

## Create a personal Agent

Select **Create** to start a blank Agent. Choose capabilities, implementations, identity, instructions, and runtime defaults, then save. Use **Use Agent** to start a conversation or choose it in product selectors that support personal Agents. Concrete resources such as knowledge bases, workspaces, and companions are selected by the session or product entry, not stored in a second binding inside the workbench.

## Capabilities and extensions

| Seed | Initial content | What it does not auto-do |
| --- | --- | --- |
| Lite | Identity, instructions, model routing | Does not add tools, Workspace, MCP, or application tools |
| General | Official common Capabilities/Skills | Does not absorb every extension the user installed |
| Full | Official coding baseline | Does not install or include every application tool |
| Custom | Blank or optional seed | Does not allow entering internal IDs, digests, or runtime parameters |

The workbench reads enabled, materialized capabilities that support the target consumer. Editing cannot grant access to unavailable or uninstalled capabilities. The server validates action grants, dependencies, conflicts, and implementation compatibility. Choose a source explicitly when multiple implementations exist.

MCP and Skills use the global extension catalog and session selections. Neither official nor personal Agent configurations contain an extension master switch. Installation, credentials, enablement, and service lifecycle remain with their respective settings and domain owners owners.

## Saving and session boundaries

Edits first live in a draft. The canonical compiler validates a save, atomically creates an immutable `AgentPresetRevision`, and advances its current revision pointer. Official and personal configurations use this same save path.

Session creation freezes capabilities, implementations, models, tool contracts, and resources in a Snapshot. Saving an Agent never rewrites previous revisions, Snapshots, session events, or an executing turn. Existing sessions change configuration through an explicit canonical binding transition after admission checks, runtime cleanup, and effect settlement.

The workbench queries the platform Capability Catalog. A catalog entry carries
at least: a stable capability ID, contract digest, owner, source Package or
application MCP provenance, supported consumers, typed contributions, required
resources, and per-consumer availability.

Capabilities have no independent version number. Presets, dependencies, and
conflicts reference the stable capability ID only; builtin implementations are
frozen by Package version, contract digest, ContributionLock, and Snapshot
digest, so updating an implementation does not rewrite workbench references.

- Only builtin contributions that are enabled and materialized, plus explicitly selected application
  MCP tools, enter the official discovery surface;
- Application connection credentials and private business data never enter
  an Agent Snapshot;
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

Storage and transitions follow the [Agent Session architecture](../architecture/agent-session.md) and current canonical baseline; retired Conversation or preset fields are never a recovery source.

## Canonical API (machine resource names)

The workbench uses these canonical APIs; the stable machine names do not mean
the UI must display "Preset":

Official templates remain release baselines for initialization and explicit copies. Template updates affect unsaved defaults; saved official configurations never drift silently.

## APIs and implementation

| Purpose | Endpoint |
| --- | --- |
| Templates and the user's official and personal configurations | `GET /api/agent-preset-templates` |
| Create a personal Agent | `POST /api/agent-presets` |
| Explicit personal copy of an official baseline | `POST /api/agent-presets/from-template/{template_id}`, `reuse_existing=false` |
| Prepare saved configuration for an official product entry | Same endpoint, `reuse_existing=true` |
| Read the shared editor | `GET /api/agent-presets/{preset_id}/editor` |
| Save an immutable revision | `POST /api/agent-presets/{preset_id}/revisions` |
| Capability and implementation catalog | `GET /api/agent-catalog` |
| Sessions and explicit Agent updates | `/api/agent-sessions/*` |

`official_templates` contains initialization baselines. `official_presets` maps template keys to the current user's stable editable identities. `user_presets` contains personal Agents only. Templates, source labels, and internal IDs are never client authorization; the server validates ownership, capabilities, and Snapshots.

## Implementation boundaries

For the current implementation of Agent revisions, Snapshots, Sessions, and
Module authorization, see
[Agent Session architecture](../architecture/agent-session.md).

For application imports, preview, settings and commands, see
[application hosting architecture](../architecture/app-hosting.md).

See [Agent Session architecture](../architecture/agent-session.md) for runtime boundaries and [Plugin Platform architecture](../architecture/plugin-platform.zh.md) for Plugin management (Chinese).
