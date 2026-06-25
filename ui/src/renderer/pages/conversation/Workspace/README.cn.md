# 会话工作区右栏

工作区右栏是会话旁边的文件树与变更面板。旧实现是一个很大的会话专用组件；
当前结构已经拆成「会话绑定层」和「source-agnostic 展示层」。

## 当前结构

```text
Workspace/
├── index.tsx                 # ChatWorkspace：conversation -> WorkspaceSource
├── WorkspaceRailBody.tsx     # 可复用、与来源无关的右栏主体
├── types.ts                  # WorkspaceSource、SelectedFile、tab/source 类型
├── components/               # toolbar、tab bar、context menu、dialogs
├── hooks/                    # tree、file ops、paste/drag、search、changes
├── utils/                    # preview 与 tree helper
└── workspace.css
```

`index.tsx` 把会话适配为 `WorkspaceSource`：

- 通过 `ipcBridge.conversation.getWorkspace` 加载文件树；
- 把选择的文件映射回 SendBox emitter payload；
- 订阅 agent stream 与手动刷新事件；
- 处理 SendBox selection sync；
- 用 conversation tracking key 启用 paste / drag / upload。

`WorkspaceRailBody.tsx` 负责实际 UI。它只接收 `WorkspaceSource`，不导入会话专用
代码，也不写 `if (terminal)` 分支。未来 terminal 或其他表面可以提供另一种
source 并复用同一个 body。

## 关键类型

- `WorkspaceSource`：右栏的数据与能力提供者。
- `WorkspaceTreeSource`：`useWorkspaceTree` 使用的 lazy root/child loader。
- `SelectedFile`：与来源无关的文件 / 文件夹选择结构。
- `WorkspaceUploadConfig`：存在即启用 upload、drag、paste UI。
- `eventPrefix`：只能是 `'acp' | 'codex' | 'nomi' | 'openclaw-gateway' | 'nanobot' | 'remote'`。

## 持久化与设置

不要再使用旧的 `ConfigStorage` 模式。跨表面设置应走当前共享服务，例如
`configService`、layout context，或具体功能自己的 hook。

## Import

使用真实大小写路径：

```ts
import WorkspaceRailBody from './WorkspaceRailBody';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
```

目录名是 `Preview`，不是 `preview`。
