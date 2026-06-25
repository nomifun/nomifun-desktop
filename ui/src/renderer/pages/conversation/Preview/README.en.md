# Conversation Preview Panel

`Preview/` is the reusable document/file preview module used by conversation
surfaces. It owns preview tabs, lightweight persistence, keyboard shortcuts,
viewer dispatch, and integration hooks for adding preview content back to the
send box.

## Current Structure

```text
Preview/
├── index.ts
├── context/
│   ├── PreviewContext.tsx
│   └── PreviewToolbarExtrasContext.tsx
├── components/
│   └── index.ts
├── hooks/
├── fileUtils.ts
├── previewUrls.ts
├── constants.ts
└── types.ts
```

Import from the module root:

```ts
import { PreviewProvider, usePreviewContext } from '@/renderer/pages/conversation/Preview';
import type { PreviewContentType } from '@/renderer/pages/conversation/Preview';
```

The directory is `Preview`, not `preview`.

## Data Model

Core IPC content types live in `@/common/types/office/preview` and are
re-exported from `Preview/types.ts`.

`PreviewContext.tsx` defines the renderer tab model:

```ts
interface PreviewMetadata {
  language?: string;
  title?: string;
  diff?: string;
  file_name?: string;
  file_path?: string;
  workspace?: string;
  editable?: boolean;
  truncated?: boolean;
}
```

Use snake_case metadata keys (`file_name`, `file_path`) because that is the
stream/IPC payload shape. Do not use old `fileName` / `filePath` snippets in new
docs or examples.

## Persistence

`PreviewProvider` persists only lightweight text tabs:

- namespace-aware keys: `nomifun_preview_tabs:<namespace>` and
  `nomifun_preview_active_tab_id:<namespace>`;
- default namespace: `conversation`;
- persistable content types: markdown, HTML, code, and diff;
- content-size cap to avoid localStorage jank.

Legacy single-key state is migrated on load and then cleaned up.

## Opening A Preview

```tsx
const { openPreview } = usePreviewContext();

openPreview(markdown, 'markdown', {
  title: 'notes.md',
  file_name: 'notes.md',
  file_path: '/workspace/notes.md',
  workspace: '/workspace',
  editable: true,
});
```

The provider can optionally subscribe to global `preview.open` events. Only the
primary conversation provider should do this; secondary surfaces should pass a
separate `persistNamespace` and disable global subscription if needed.

## Maintenance Notes

- Keep examples aligned with `PreviewContext.tsx`.
- Add new viewer exports through `components/index.ts`.
- Keep module-root imports stable; avoid deep imports from consumer code unless
  a hook or utility is intentionally internal.
