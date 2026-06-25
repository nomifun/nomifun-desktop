# Conversation Workspace Rail

The workspace rail is the file-tree and changes panel shown beside a
conversation. It used to be one large conversation-specific component; it is now
split into a thin conversation binding plus a source-agnostic body.

## Current Structure

```text
Workspace/
├── index.tsx                 # ChatWorkspace: conversation -> WorkspaceSource
├── WorkspaceRailBody.tsx     # reusable source-agnostic rail body
├── types.ts                  # WorkspaceSource, SelectedFile, tab/source types
├── components/               # toolbar, tab bar, context menu, dialogs
├── hooks/                    # tree, file ops, paste/drag, search, changes
├── utils/                    # preview and tree helpers
└── workspace.css
```

`index.tsx` adapts a conversation into `WorkspaceSource`:

- loads tree data through `ipcBridge.conversation.getWorkspace`,
- maps selected files back to the SendBox emitter payload,
- subscribes to agent stream events and manual refresh events,
- handles SendBox selection sync,
- enables paste/drag/upload with a conversation tracking key.

`WorkspaceRailBody.tsx` renders the actual UI. It receives a `WorkspaceSource`
and intentionally has no conversation-specific imports or `if (terminal)`
branches. A future terminal or non-conversation surface can provide another
source and reuse the same body.

## Important Types

- `WorkspaceSource`: pluggable data/capability provider for the rail.
- `WorkspaceTreeSource`: lazy root/child loader used by `useWorkspaceTree`.
- `SelectedFile`: source-agnostic file/folder selection shape.
- `WorkspaceUploadConfig`: presence enables upload, drag, and paste UI.
- `eventPrefix`: one of `'acp' | 'codex' | 'nomi' | 'openclaw-gateway' | 'nanobot' | 'remote'`.

## Persistence And Settings

Do not use legacy `ConfigStorage` patterns here. Cross-surface settings should
flow through the current shared services such as `configService`, layout
context, or feature-specific hooks.

## Imports

Use the actual case-sensitive paths:

```ts
import WorkspaceRailBody from './WorkspaceRailBody';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
```

The directory is `Preview`, not `preview`.
