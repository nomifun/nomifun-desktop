# Assistants

An **assistant** is a reusable persona package for an agent: display metadata,
default agent backend, optional model preferences, system prompt, and skill
selection.

Open the current page at **`/assistants`**. The legacy
`/settings/assistants` route redirects to `/assistants?tab=assistants`.

![Assistants list and drawer](../images/assistants-01-list.png)

## Sources

Assistants are merged from three sources:

| Source | Where it comes from | Editable? |
| --- | --- | --- |
| Builtin | Embedded manifest under `crates/backend/nomifun-app/assets/builtin-assistants/`, loaded by `BuiltinAssistantRegistry`. | Content is read-only; enable/sort/last-used state and builtin `preset_agent_type` override are stored separately. |
| Custom | User-created rows in the `assistants` table plus files in the data dir. | Fully editable and deletable. |
| Extension | Installed extensions via `resolvers::assistant`. | Read-only from this page; manage the extension lifecycle instead. |

The merged list is returned by `GET /api/assistants`.

## What an Assistant Owns

Key fields:

- `id`, `source`, `name`, `description`, `avatar`
- `preset_agent_type`: default backend such as `nomi`, `claude`, `codex`, `gemini`
- `models`: optional preferred model ids
- `prompts` / `prompts_i18n`: assistant instructions
- `enabled_skills`: skills attached when starting a session
- `enabled`, `sort_order`, `last_used_at`
- tag metadata used by the picker and filters

Custom assistant rule files live under the data dir:

- `assistant-rules/`
- `assistant-skills/`
- `assistant-avatars/`

Deleting a custom assistant removes its associated files.

## Editing Rules

| Field / action | Builtin | Extension | Custom |
| --- | --- | --- | --- |
| Enable / disable | yes | yes | yes |
| Sort / last-used state | yes | yes | yes |
| Change default agent backend | builtin override only | no | yes |
| Edit name / description / avatar | no | no | yes |
| Edit prompt / skill text | no | no | yes |
| Delete | no | no | yes |

Builtin mutations are stored in `assistant_overrides`. Extension assistants are
owned by their extension and intentionally read-only here.

![Assistant editor drawer](../images/assistants-02-editor.png)

## Skills

The Skills tab is also under `/assistants`:

- `/assistants?tab=assistants`
- `/assistants?tab=skills`

Assistant `enabled_skills` are merged with auto-injected builtin skills when a
session starts. The materialization rules are implemented by the skill routes
and backend-specific agent adapters; users do not need to copy skill folders
manually for normal use.

For MCP servers, use `/mcp`; skills and MCP servers are related but managed on
separate pages now.

## API

| Operation | Endpoint |
| --- | --- |
| List / create | `GET`, `POST /api/assistants` |
| Update / delete | `PUT`, `DELETE /api/assistants/{id}` |
| State override | `PATCH /api/assistants/{id}/state` |
| Avatar | `GET /api/assistants/{id}/avatar` |
| Bulk import | `POST /api/assistants/import` |
| Tags | `GET`, `POST /api/assistant-tags`; `PUT`, `DELETE /api/assistant-tags/{key}` |

Rule and assistant-skill file reads/writes go through `/api/skills/assistant-*`
routes so builtin, extension, and user sources can be dispatched correctly.

## Notes

- Creating an assistant without `preset_agent_type` requires at least one
  configured provider; the service defaults to `nomi` when possible.
- CLI-backed agents still require their CLI to be installed on the host. Picking
  `claude`, `codex`, or `gemini` as an assistant backend does not install those
  tools.
- Import from legacy JSON is insert-only and idempotent: existing ids are
  skipped, and invalid rows are reported per assistant.

## Related

- [MCP & Skills](./mcp-and-skills.md)
- [Model Failover Queue](./model-routing.md)
