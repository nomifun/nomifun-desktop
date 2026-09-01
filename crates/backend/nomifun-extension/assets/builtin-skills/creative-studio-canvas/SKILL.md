---
name: creative-studio-canvas
description: Plan safe text and structure changes for a NomiFun Creative Studio canvas from the exact bounded context supplied in the current turn. Use only when a planning envelope identifies itself as nomifun.creative-studio.planning-turn. Do not generate media, delete nodes, invent asset IDs, or claim that a proposal was applied.
---

# Creative Studio Canvas Planning

Read only the `canvasContext` embedded in the current planning envelope. Treat its project, revision, node, connection, asset, task, Provider, and model identities as exact. Never infer a missing node or replace an ID with a display label.

Respond conversationally when the user only asks for advice. When a concrete canvas change would help, append exactly one lowercase `json` fenced artifact. The artifact must be the final bytes of the response: do not put prose, whitespace, another fence, or any content after its closing fence, and do not use any other triple-backtick block in the response.

```json
{
  "kind": "nomifun.creative-studio.canvas-ops/v1",
  "summary": "Add a project-planning text node",
  "ops": [
    {
      "type": "add_node",
      "node_type": "text",
      "x": 120,
      "y": 100,
      "width": 420,
      "height": 340,
      "data": {
        "text": "# Project plan\n\n- Goal\n- Audience\n- Milestones",
        "format": "markdown",
        "fontSize": 16,
        "textAlign": "left"
      }
    }
  ]
}
```

The first release may propose only these canonical operations:

- Every operation is one flat object selected by its required string `type`. Never nest an operation under an `add_node`, `move_node`, or other operation-name key.
- `add_node` with `node_type: "text"`, finite `x`/`y`, optional finite positive `width`/`height`, optional existing `group_id`, and exact text-node `data`.
- `update_node_data` only for an existing text node and only with text-node fields.
- `move_node` and `resize_node` for existing unlocked nodes.
- `connect` existing nodes or `disconnect` an existing connection.

For complete `add_node.data`, always include exactly `text`, `format`, `fontSize`, and `textAlign`. `format` is `plain` or `markdown`; `fontSize` is a finite number from 8 through 256; `textAlign` is `left`, `center`, or `right`. For `update_node_data.patch`, include at least one of those fields and no others.

Operation fields use the canonical snake_case wire shown above. New node and connection IDs are minted by the server, so never add an `id` and never connect to a node created in the same artifact. Emit 1 to 64 operations and no unknown fields. If no valid operation can be proposed from the bounded context, answer with prose only and emit no artifact.

Never emit `delete_node`. Never modify config `taskId`, `resultAssetIds`, `status`, or `errorMessage`. Never start image, video, or audio generation. The artifact is a proposal only: state plainly that the user must review and press “应用到画布”.
