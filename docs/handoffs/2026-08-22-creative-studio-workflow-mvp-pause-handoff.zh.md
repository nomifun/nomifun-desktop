# Creative Studio Workflow MVP 优雅暂停交接

> 本文已被 `2026-08-22-creative-studio-workflow-mvp-second-pause-handoff.zh.md` 更新；继续实施时以后者为当前状态。

> 暂停时间：2026-08-22（Asia/Shanghai）
> 工作目录：`C:\Users\MINISFORUM\code\nomifun\nomifun-desktop`
> 分支：`codex/infinite-canvas-rebuild`
> 已提交功能锚点：`67f3d2f8`
> 已提交台账锚点：`f93e9c21`
> 参考项目：`C:\Users\MINISFORUM\code\nomifun\download\infinite-canvas`

## 1. 当前结论

Creative Studio 主体、严格 Canvas Agent 提案和最小首页 kickoff 已提交。当前唯一在开发中的切片是“最小 Workflow AI”：

1. 用户输入一个简单需求。
2. 用户选择 NomiFun 模型目录中的精确 Chat Provider/model。
3. 后端执行一次无会话、无工具、无持久化的 Chat completion。
4. 前端只接受严格 `nomifun.creative-studio.workflow-draft/v1` artifact。
5. 用户预览后应用到既有编辑器。
6. 只有用户再次点击编辑器“保存”才写入 Workflow；绝不自动运行或调用图片模型。

会话历史、分支、附件、公开模板、自动保存、自动运行和复杂创作编排全部后置。不要恢复已否决的普通 Conversation 轮询方案。

## 2. Git 与工作树

本交接文档提交的父提交与最后可交付功能锚点为：

```text
f93e9c21 docs(creative-studio): record minimal kickoff home gate
67f3d2f8 feat(creative-studio): add minimal agent kickoff home
97803724 docs(creative-studio): record durable canvas proposal gate
6ccc7a24 feat(creative-studio): apply durable canvas proposals
```

工作树包含未提交的 Workflow MVP 前端、严格合同、Skill 和固定配色改动。它们是当前任务进度，不要 reset、checkout 或覆盖：

```text
M  crates/backend/nomifun-app/assets/builtin-skills/creative-studio-workflow/SKILL.md
M  crates/backend/nomifun-app/tests/builtin_asset_contract.rs
M  ui/src/renderer/pages/creativeStudio/canvas/product/agent/artifacts.ts
M  ui/src/renderer/pages/creativeStudio/models/CreativeModelSelect.tsx
M  ui/src/renderer/pages/creativeStudio/workflows/page/CreativeWorkflowRoute.structure.test.ts
M  ui/src/renderer/pages/creativeStudio/workflows/page/CreativeWorkflowRoute.tsx
M  ui/src/renderer/pages/creativeStudio/workflows/page/CreativeWorkflowWorkspacePage.module.css
M  ui/src/renderer/pages/creativeStudio/workflows/page/CreativeWorkflowWorkspacePage.render.test.tsx
M  ui/src/renderer/pages/creativeStudio/workflows/page/CreativeWorkflowWorkspacePage.tsx
?? ui/src/renderer/pages/creativeStudio/workflows/agent/
?? ui/src/renderer/pages/creativeStudio/workflows/page/WorkflowAgentDraftModal.*
```

暂停时没有将半成品功能提交。下一位 Agent 应在同一工作树继续，不要从提交锚点重新实现这些文件。

## 3. 当前未提交实现

### 严格 Workflow artifact

`ui/src/renderer/pages/creativeStudio/workflows/agent/` 已包含：

- `artifacts.ts/.test.ts`：唯一且最终的 lowercase `json` fence、精确字段、decoded duplicate key、孤立 surrogate、256 KiB、字符串上限和两种 mode。
- `converter.ts/.test.ts`：从产品 `createBlankWorkflow` 生成 IDs/revision/time/变量；visibility 固定 private；tags 固定空；图片生成模型保持 `null`；多图 planning 只绑定本次精确 Chat 模型；至少要求一个允许的 placeholder。
- `draftPort.ts/.test.ts`：计划调用 `POST /api/creative-studio/workflow-drafts`，严格发送 `{prompt, model:{providerId,model}}`，通过共享 authenticated `httpRequest`，125 秒客户端上限，严格解析 `{text}`。

Canvas parser 只导出了通用 `assertStrictJsonWithoutDuplicateKeys` 供 Workflow 复用，既有 Canvas 行为未改变。

### Workflow UI

已新增 `WorkflowAgentDraftModal.tsx/.module.css/.test.tsx` 并接入 Workflow Route/Page：

- 需求 textarea、精确 Chat model select、生成、严格预览、应用到编辑器。
- catalog 必须为 `ready`；选择或目录变化会清草稿；应用前再次验证草稿与当前精确模型一致。
- 应用只执行 `setEditing(workflow)` 与 `setEditingIsNew(true)`；Repository 写仍只在原编辑器保存函数中。
- Modal 已加 viewport `maxWidth`；页面与 Workflow Modal 使用固定浅色 stone tokens，不跟随用户主题。
- `CreativeModelSelect` 新增可选 `getPopupContainer`，Workflow 下拉挂到 `creative-studio-portal-root`。Portal root 自身的固定 tokens 尚未补，需在视觉门禁前完成或用等价方案保证 popup 不继承用户主题。

### Builtin Skill

`creative-studio-workflow/SKILL.md` 已从 planning prose 改为严格 Workflow artifact 指令：

- 只允许 `single-image` / `multi-image-series`。
- 单图 placeholders：`product_name`、`selling_points`。
- 多图 placeholders：`topic`、`style`、`platform`。
- 不允许模型输出 IDs、model binding、variables、visibility、tags、attachments。
- 用户必须预览、应用并手动保存；不保存、不运行、不生成媒体。

## 4. 已否决的 Conversation 方案

不要恢复 `conversationPort.ts`。只读复审确认它有以下产品级问题：

1. send receipt 的 `msg_id` 是用户消息 ID，不是另铸的 root turn ID；按它过滤 assistant `turn_id` 会正常请求也轮询超时。
2. 普通 Conversation 会继承全局 model failover，可能切到备用模型并自动重发，违反精确模型和单次调用。
3. Conversation 会冻结 auto-inject Skills、默认 MCP/内建工具，并可能进入隐藏 confirmation；为清理这些状态再增加 stop/confirm 体系与本轮简化目标冲突。

因此当前前端已改为无会话 `draftPort`，但后端端点还没有实现。

## 5. 后端下一步（最高优先级）

在 `nomifun-workshop` 实现：

```http
POST /api/creative-studio/workflow-drafts
Content-Type: application/json

{
  "prompt": "...",
  "model": {
    "providerId": "canonical UUIDv7",
    "model": "exact model name"
  }
}
```

成功响应只允许：

```json
{"text":"raw assistant response"}
```

硬门：

- installation owner only，并在 Workshop 域内保留 owner 二次校验。
- request 使用 strict camelCase + `deny_unknown_fields`。
- prompt trim 后非空，最多 20,000 UTF-16 code units；model 原样已 trim、非空、最多 512；Provider ID canonical UUIDv7。
- 精确当前 enabled Provider/model/Chat capability；不做相邻 capability 猜测。
- 无 Conversation、无 Skill 注入、无 MCP、无 workspace/session 持久化、无自动 retry/failover。
- 固定系统提示词要求严格 `nomifun.creative-studio.workflow-draft/v1`；前端仍是最终 fail-closed parser。
- 固定 120 秒左右服务端 timeout；超时后没有后台任务。
- 一个 HTTP 请求最多开始一次 Provider stream。

重要：优先复用 `resolve_provider_config + one_shot_completion(... tools=[])` 的真正单轮路径。不要直接用当前 `run_one_shot_turn` 默认 tool loop：即使 whitelist 为空，异常 Provider 返回 `ToolUse` 时它仍可能追加 unavailable tool result 并再次请求模型，最多 8 轮。若必须复用 `run_one_shot_turn`，先修改其 empty-tools 分支为遇到任何 tool-use 立即失败，并加入“stream 调用次数始终为 1”的回归测试。

建议增加极小 `WorkflowDraftRunner` trait seam，使 route/service 测试注入 fake 并断言精确 Provider/model、固定 system、空 history、单轮和 timeout，而不调用真实模型。

接线位置：

- `crates/backend/nomifun-workshop/Cargo.toml`
- `crates/backend/nomifun-workshop/src/state.rs`
- `crates/backend/nomifun-workshop/src/routes.rs`
- `crates/backend/nomifun-app/src/router/state.rs` 的 `build_workshop_state`
- 生产依赖已有 `services.model_invoke_service` 与 `services.data_dir`

## 6. 必须补的测试与 UI 门禁

后端：

- strict request / unknown fields / UTF-16 bounds / UUIDv7 / exact Chat capability。
- 非 owner、disabled Provider、disabled model、非 Chat、缺凭据在 Provider 调用前失败。
- fake runner 证明无持久写，且一次请求最多一次 Provider stream。
- timeout、空响应、Provider error 的稳定错误映射。

前端：

- `draftPort` 请求/响应合同。
- Modal catalog loading/error/stale exact row 时生成与应用均 fail closed。
- 应用只进入编辑器，绝不调用 repository create/save/run。
- `CreativeModelSelect.getPopupContainer` 结构测试，以及下拉实际固定 stone 配色。
- 1280×720、1024×768、390×844：打开 Modal、选择器、空预览、错误态；不要点击真实生成，除非另有成本授权。
- fresh reload Console 0 error / 0 warning。

## 7. 暂停前验证

当前未提交前端状态已完成：

- Workflow agent + Modal + Route/Page 定向测试：`21 passed / 99 assertions`。
- `bun run typecheck`：通过。
- 早先 builtin Skill 合同定向 Rust 测试：`1 passed`；之后只修正了合法 mode 示例、删除 canvas artifact 冲突句，建议继续时快速复跑。
- 没有调用真实 Provider、没有生成媒体、没有付费模型调用。

尚未执行：后端端点（不存在）、当前切片 UI production build、图标/主题/dead-css、最终浏览器交互和任何提交。

## 8. 当前本地服务

暂停时监听状态：

- 参考 Next：`3000`
- 参考 Go：`8080`
- 目标 Vite：`5174`
- 目标隔离 Rust backend：`8788`

继续时必须重新检查，不要假设进程仍存活。目标 8788 仍是提交 `67f3d2f8` 的后端，不包含尚未实现的 workflow-drafts endpoint。

## 9. 建议提交顺序

1. 完成后端 one-shot、复审当前前端 WIP、运行目标化测试与真实 UI 门禁。
2. 只在完整可用后提交：`feat(creative-studio): add minimal workflow draft assistant`。
3. 单独更新主台账并提交：`docs(creative-studio): record minimal workflow draft gate`。
4. 再进入 P11：三视口、Tauri 慢环、UI build/桌面打包、最终能力文档。
5. 高级媒体、附件、公开模板、复杂会话/创作编排继续后置。

## 10. 可直接交给下一位 Coding Agent 的启动 Prompt

```text
请继续 C:\Users\MINISFORUM\code\nomifun\nomifun-desktop 的 Creative Studio 整体迁移，当前分支 codex/infinite-canvas-rebuild。

开始前必须完整阅读：
1. docs/handoffs/2026-08-21-creative-studio-rebuild-ledger.zh.md
2. docs/handoffs/2026-08-22-creative-studio-workflow-mvp-pause-handoff.zh.md
3. 仓库 AGENTS.md

先执行 git status --short --branch 与 git log -6 --oneline。当前 HEAD 应是本交接文档提交，其父提交为 f93e9c21；工作树中有未提交的 Workflow MVP 前端/Skill/合同进度，这些都是有效 WIP，禁止 reset、checkout、覆盖或重新实现。参考项目必须打开并对照 C:\Users\MINISFORUM\code\nomifun\download\infinite-canvas。

用户最新范围：会话与创作功能全部先简化，以最低成本、高质量、可上线为目标。Workflow AI 只做“简单需求 + 精确 Chat 模型 -> strict draft 预览 -> 应用既有编辑器 -> 用户手动保存”。不做 Conversation 历史/分支、附件、公开模板、自动保存、自动运行或复杂编排；不要恢复已删除的 conversationPort。

最高优先级是实现 owner-only POST /api/creative-studio/workflow-drafts：strict {prompt,model:{providerId,model}} -> {text}，精确 enabled Chat capability，真正无会话、无工具、无持久化、无自动 retry/failover且单个 HTTP 请求最多一次 Provider stream。优先使用 resolve_provider_config + one_shot_completion(... tools=[])；不要直接使用会在异常 ToolUse 后进入多轮的 run_one_shot_turn，除非先把空工具路径改为单轮 fail-closed并测试 stream count=1。

后端完成后复审现有 frontend draftPort/artifact/converter/Modal/Route WIP，补 stale catalog、固定 popup stone 配色、strict response 和“仅应用到编辑器、不自动写/跑”的门禁。按需最多用 1-2 个独立 Agent，不追求高并发；不得强行中断 Agent。

验证按风险最小充分执行：Workflow 定向 UI tests + typecheck + icons/theme/dead-css + UI build；Workshop/one-shot 定向 Rust tests + cargo check nomifun-app；真实 UI 检查 1280x720、1024x768、390x844、fresh reload Console。没有额外成本授权时绝不点击真实生成或调用付费 Provider。

完成后先提交功能 commit，再单独更新实施台账并提交。然后继续 P11 上线门，直至可替换原“创意工坊”的可交付状态。
```
