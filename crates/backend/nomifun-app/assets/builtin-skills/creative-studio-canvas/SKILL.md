---
name: creative-studio-canvas
description: Plan safe text and structure changes for a NomiFun Creative Studio canvas from the exact bounded context supplied in the current turn. Use only when a planning envelope identifies itself as nomifun.creative-studio.planning-turn. Do not generate media, delete nodes, invent asset IDs, or claim that a proposal was applied.
---

# Creative Studio Canvas Planning

Read only the `canvasContext` embedded in the current planning envelope. Treat its project, revision, node, connection, asset, task, Provider, and model identities as exact. Never infer a missing node or replace an ID with a display label.

Respond conversationally when the user only asks for advice. When a concrete canvas change would help, append exactly one JSON artifact with this closed shape:

```json
{
  "kind": "nomifun.creative-studio.canvas-ops/v1",
  "summary": "Short user-facing explanation",
  "ops": []
}
```

The first release may propose only these canonical operations:

- `add_node` with `node_type: "text"`, finite `x`/`y`, optional finite positive `width`/`height`, optional existing `group_id`, and exact text-node `data`.
- `update_node_data` only for an existing text node and only with text-node fields.
- `move_node` and `resize_node` for existing unlocked nodes.
- `connect` existing nodes or `disconnect` an existing connection.

Nested operation fields use the canonical snake_case wire. New node and connection IDs are minted by the server, so never add an `id` and never connect to a node created in the same artifact. Keep the batch at 64 operations or fewer.

Never emit `delete_node`. Never modify config `taskId`, `resultAssetIds`, `status`, or `errorMessage`. Never start image, video, or audio generation. The artifact is a proposal only: state plainly that the user must review and press “应用到画布”.
