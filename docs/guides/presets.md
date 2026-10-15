# Agent Workbench

> The user-visible product name is **Agent Workbench**; the public UI route is `/agent`. `AgentPreset` is only the
> internal name of the backend authoring aggregate. Application MCP tools reuse
> current session selection; process ownership belongs to application hosting.

Open **Agent** (`/agent`) from the sidebar to edit an official Agent directly or create a personal Agent. The official list includes Chat, General, Coding, Companion, Customer Service, and Creation. The builtin `app.developer` template belongs to application workspaces and is not a general conversation entry.

## Edit an official Agent

1. Select an Agent from the official list, such as **Companion**.
2. Enable or disable capabilities and individual actions. Choose compatible implementations on the components tab.
3. Edit the name, description, persona, instructions, and runtime defaults.
4. Configure an available Chat model, then select **Save**.

Saving updates the current user's official Agent configuration and keeps the same Agent identity. Creating a copy is optional. Other users have separate configurations. Future sessions launched from that official product entry use the saved configuration. Official template updates preserve capabilities, instructions, and component choices saved in the current dataset.

The official Companion keeps its two core capabilities, Companion and Companion memory. Other capabilities and actions remain editable.

Existing sessions keep their saved configuration until you explicitly select another Agent or apply a saved update. Companion controls provide **Apply saved updates** and apply the change to the same idle Session, keeping history, the current model and applicable resources. Active or paused turns and unsettled effects block the update. The UI explains unavailable capabilities and missing resources. Failed saves are not treated as successful changes.

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

MCP and Skills use the global extension catalog and session selections. Neither official nor personal Agent configurations contain an extension master switch. Installation, credentials, enablement, and service lifecycle remain with their respective settings and domain owners.

## Saving and session boundaries

Save validates the draft and creates a new immutable Agent revision. New Sessions use that saved configuration. Saving never rewrites an existing Session or an active turn.

To change an existing ordinary conversation, explicitly select an Agent or apply its saved update. The Session must be idle, with no paused turn or unsettled effect. The update keeps the same Session and history; missing resources or incompatible capabilities are explained in the UI. Companion entry points provide their own **Apply saved update** action. Application development workspaces keep their fixed builtin Agent.

MCP and Skill selections are managed per Session from the input controls. Global installation and server enablement remain on [MCP & Skills](mcp-and-skills.md), and do not change merely because an Agent is saved.

For revisions, snapshots and transition contracts, see [Agent Session architecture](../architecture/agent-session.md). For application workflows, see [NomiFun applications](app-hosting.md).
