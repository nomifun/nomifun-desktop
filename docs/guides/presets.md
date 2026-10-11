# Agent Workbench

Open **Agent** (`/agent`) from the sidebar to edit an official Agent directly or create a personal Agent. The official list includes Chat, General, Coding, Companion, Customer Service, and Creation.

## Edit an official Agent

1. Select an Agent from the official list, such as **Companion**.
2. Enable or disable capabilities and individual actions. Choose compatible implementations on the components tab.
3. Edit the name, description, persona, instructions, and runtime defaults.
4. Configure an available Chat model, then select **Save**.

Saving updates the current user's official Agent configuration and keeps the same Agent identity. Creating a copy is optional. Other users have separate configurations. Future sessions launched from that official product entry use the saved configuration. Official template updates preserve capabilities, instructions, and component choices saved in the current dataset.

The official Companion keeps its two core capabilities, Companion and Companion memory. Other capabilities and actions remain editable.

Existing sessions keep their frozen configuration. For a desktop companion, select **Apply saved updates** in the companion overview or chat controls, review the capability changes, and confirm. This updates the same idle session. Active or paused turns and unsettled effects block the update. The companion's model and still-required resource selections and parameters are retained; resources removed with disabled capabilities appear in the preview. Configure missing resources required by newly enabled capabilities before applying. If the saved configuration changes after preview, preview and confirm again.

Select **Create a copy** when you need an independent personal Agent. This creates a new identity and leaves the saved official configuration intact. Personal Agents can be deleted; official product entries are retained.

## Create a personal Agent

Select **Create** to start a blank Agent. Choose capabilities, implementations, identity, instructions, and runtime defaults, then save. Use **Use Agent** to start a conversation or choose it in product selectors that support personal Agents. Concrete resources such as knowledge bases, workspaces, and companions are selected by the session or product entry, not stored in a second binding inside the workbench.

## Capabilities and extensions

The workbench reads enabled, materialized capabilities that support the target consumer. Editing cannot grant access to unavailable or uninstalled capabilities. The server validates action grants, dependencies, conflicts, and implementation compatibility. Choose a source explicitly when multiple implementations exist.

MCP and Skills use the global extension catalog and session selections. Neither official nor personal Agent configurations contain an extension master switch. Installation, credentials, enablement, and service lifecycle remain with their settings and Plugin Core owners.

## Saving and session boundaries

Edits first live in a draft. The canonical compiler validates a save, atomically creates an immutable `AgentPresetRevision`, and advances its current revision pointer. Official and personal configurations use this same save path.

Session creation freezes capabilities, implementations, models, tool contracts, and resources in a Snapshot. Saving an Agent never rewrites previous revisions, Snapshots, session events, or an executing turn. Existing sessions change configuration through an explicit canonical binding transition after admission checks, runtime cleanup, and effect settlement.

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

See [Agent Session architecture](../architecture/agent-session.md) for runtime boundaries and [Plugin Platform architecture](../architecture/plugin-platform.zh.md) for Plugin management (Chinese).
