# 会话预览面板

`Preview/` 是会话表面使用的可复用文档 / 文件预览模块。它负责 preview tabs、
轻量持久化、快捷键、viewer 分发，以及把预览内容重新加入发送框的集成 hook。

## 当前结构

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

从模块根导入：

```ts
import { PreviewProvider, usePreviewContext } from '@/renderer/pages/conversation/Preview';
import type { PreviewContentType } from '@/renderer/pages/conversation/Preview';
```

目录名是 `Preview`，不是 `preview`。

## 数据模型

跨进程的核心 preview content type 位于 `@/common/types/office/preview`，并由
`Preview/types.ts` 重新导出。

`PreviewContext.tsx` 定义 renderer tab metadata：

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

metadata 使用 snake_case（`file_name`、`file_path`），因为 stream / IPC payload
就是这个形状。不要在新文档或示例中继续使用旧的 `fileName` / `filePath`。

## 持久化

`PreviewProvider` 只持久化小体积文本 tab：

- 带 namespace 的 key：`nomifun_preview_tabs:<namespace>` 与
  `nomifun_preview_active_tab_id:<namespace>`；
- 默认 namespace：`conversation`；
- 可持久化类型：markdown、HTML、code、diff；
- 有内容长度上限，避免 localStorage 卡顿。

旧的单 key 状态会在加载时迁移并清理。

## 打开预览

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

Provider 可以选择订阅全局 `preview.open` 事件。只有主会话 provider 应该订阅；
次级表面如果需要预览，应使用自己的 `persistNamespace`，并按需关闭全局订阅。

## 维护说明

- 示例必须对齐 `PreviewContext.tsx`。
- 新 viewer 通过 `components/index.ts` 对外导出。
- 消费方优先从模块根导入；除非 hook / utility 明确是内部实现，否则避免深导入。
