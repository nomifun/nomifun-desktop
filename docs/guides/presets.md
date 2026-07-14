# Presets

A **preset** is a reusable launch configuration. It captures how an Agent,
execution step, companion, or scheduled task should start without turning
that configuration into another identity or executor.

Open the preset library at **`/presets`**. Skills remain an independent
capability library at **`/skills`**.

## Presets versus other concepts

| Concept | Owns | Does not own |
| --- | --- | --- |
| Preset | Instructions, target scope, preferred agents and models, skill scope, knowledge scope, examples, and selection metadata | Runtime process, conversation history, or companion identity |
| Agent | An executable backend such as Nomi, Codex, Claude, Gemini, or a remote agent | A reusable user configuration |
| Companion | Persistent identity, persona, figure, memory, and relationship state | The reusable launch template itself |
| Skill | One focused capability that can be discovered and loaded | Agent/model selection or a complete persona |

This separation lets the same preset launch a normal conversation, materialize
an Agent Execution step, configure a companion, or seed a scheduled task. It also
lets a companion profile or a successful collaboration role be copied into a reusable
preset without conflating the resulting template with its source.

## What a preset contains

The preset model can store:

- display name, avatar, description, and an agent-facing routing description;
- localized names, descriptions, instructions, and example prompts;
- one or more targets: conversation, execution step, companion, public
  companion, or scheduled task;
- ordered agent preferences and an optional per-user preferred agent;
- provider-qualified model preferences;
- included skills and builtin skills that must not be auto-injected;
- bound knowledge bases and an inherit/append/replace knowledge policy;
- fallback and automatic-selection behavior;
- audience/scenario tags, enabled state, ordering, and last-used state.

## Sources and editing

The catalog merges three sources:

| Source | Origin | Editing behavior |
| --- | --- | --- |
| Builtin | Embedded catalog under `crates/backend/nomifun-app/assets/builtin-presets/` | Content is read-only. Duplicate it to customize; user state such as enabled/order/preferred agent is stored separately. |
| User | Relational preset records in SQLite | Fully editable and deletable. |
| Extension | Installed extension `presets` contributions | Read-only in the preset library; manage the owning extension instead. |

User instruction and avatar assets use `preset-instructions/` and
`preset-avatars/` under the NomiFun data directory. Deleting a user preset also
removes its associated assets.

## Resolution and immutable snapshots

Choosing a preset is not just a UI filter. Before execution, the backend calls
the preset resolver for a concrete target. Resolution applies values in this
order:

1. explicit launch-time overrides;
2. the user's preferred agent, then the preset's ordered preferences;
3. an enabled fallback only when the preset allows fallback.

The resolver validates agent/model availability, applies localized
instructions, combines skill overrides, and materializes knowledge scope. It
returns a `ResolvedPresetSnapshot` containing the preset id and revision plus
the exact resolved agent, model, instructions, skills, and knowledge policy.

Conversations, scheduled tasks, and Agent Execution steps persist this snapshot.
Later edits to the catalog therefore do not silently change an already-created
target. Agent collaboration can reuse presets marked auto-selectable, while explicit
selection always remains available for supported targets.

## API

| Operation | Endpoint |
| --- | --- |
| List / create | `GET`, `POST /api/presets` |
| Read / update / delete | `GET`, `PUT`, `DELETE /api/presets/{id}` |
| User state | `PATCH /api/presets/{id}/state` |
| Resolve for a target | `POST /api/presets/{id}/resolve` |
| Avatar | `GET /api/presets/{id}/avatar` |
| Bulk import | `POST /api/presets/import` |
| Tags | `GET`, `POST /api/preset-tags`; `PUT`, `DELETE /api/preset-tags/{key}` |

Builtin and extension presets must be duplicated before their catalog-owned
content is edited. CLI-backed agents still require the corresponding CLI on the
host; selecting one as a preference does not install it.

## Related

- [MCP & Skills](./mcp-and-skills.md)
- [Model Failover Queue](./model-routing.md)
