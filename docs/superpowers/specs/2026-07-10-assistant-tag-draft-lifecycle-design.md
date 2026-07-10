# Assistant Tag Draft Lifecycle Fix

## Problem

`AssistantEditDrawer` stays mounted when it is hidden. Each `AssistantTagPicker`
therefore retains its internal `adding` and `draft` state after the drawer is
closed. Reopening the assistant creator can show the previous unfinished input
instead of the normal add affordance.

## Scope

This change applies only to the audience and scenario tag pickers inside the
assistant create/edit drawer. The shared skill-tag modal keeps its current
save and blur behavior.

## Required Behavior

- Pressing Enter validates and creates a tag, selects it, and clears the input.
- Moving focus elsewhere keeps the unfinished text in the current open drawer
  and does not create a tag.
- While the input is visible, a small localized hint says “Press Enter to finish
  adding” (`按回车完成添加`).
- Saving, creating, cancelling, using the header close control, or dismissing
  the drawer clears both unfinished tag drafts and returns both pickers to their
  normal add state.
- An unfinished draft is never included in the assistant payload and never
  creates a tag implicitly.

## Design

`AssistantEditDrawer` will hold refs to both `AssistantTagPicker` instances and
use their existing `resetPendingTag()` imperative API. A single drawer-level
reset/close helper will cover every dismissal path. The save button will clear
both drafts before delegating to the existing assistant save handler.

The picker will render a localized hint only while its inline input is active.
No blur handler will be enabled for the assistant drawer, preserving its current
draft-on-blur behavior.

## Error Handling

Clearing local draft state is synchronous and cannot fail. Existing Enter-key
tag creation errors continue to follow the current error path and leave the
drawer open.

## Testing

Add a focused regression test that proves:

- both drawer pickers expose refs;
- every assistant drawer exit path clears pending drafts;
- save clears drafts rather than flushing them;
- the localized hint is rendered from the picker and translated in English and
  Simplified Chinese;
- assistant drawer pickers do not opt into `commitOnBlur`.

Run the focused test, the relevant existing tag-draft test, and UI type checking.
