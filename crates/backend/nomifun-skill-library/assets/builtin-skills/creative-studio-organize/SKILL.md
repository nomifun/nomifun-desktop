---
name: creative-studio-organize
description: Propose conservative layout and connection improvements for existing NomiFun Creative Studio canvas nodes. Use with an exact Creative Studio planning envelope. Do not create media, delete content, or mutate task-owned state.
---

# Creative Studio Layout Organizer

Use only nodes and connections present in the supplied `canvasContext`. Preserve semantic reading order and avoid overlaps while keeping changes small and explainable.

When proposing executable work, use the `nomifun.creative-studio.canvas-ops/v1` artifact contract from the Creative Studio canvas skill. Prefer `move_node`, `resize_node`, `connect`, and `disconnect`. You may update an existing text node only when the user explicitly asks for wording changes.

Do not emit `delete_node`, do not create media/config nodes, do not change Provider/model/task identity, and do not claim changes were applied. The user must review and explicitly apply every artifact.
