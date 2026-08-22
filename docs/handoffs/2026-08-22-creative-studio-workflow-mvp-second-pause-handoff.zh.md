# Creative Studio Workflow MVP 第二次优雅暂停交接

> 暂停时间：2026-08-22（Asia/Shanghai）
> 工作目录：`C:\Users\MINISFORUM\code\nomifun\nomifun-desktop`
> 分支：`codex/infinite-canvas-rebuild`
> 暂停前 HEAD：`1cfbb037`
> 参考项目：`C:\Users\MINISFORUM\code\nomifun\download\infinite-canvas`

## 1. 当前准确状态

最小 Workflow AI 的后端、严格 artifact、前端 Modal/Route 和固定创作配色已经实现并通过静态、单元、Rust 集成与真实拒绝路径检查，但整个功能切片仍未提交，也不能宣称已经交付。

最后一个真实 UI 检查发现：在 Workflow AI Modal 中点击 `big-pickle` option 后，下拉关闭，但受控 model value 没有出现在 Select 中，“生成工作流草稿”仍为 disabled。尚未确认这是浏览器自动化点击方式问题、`NomiSelect` 事件问题，还是本次 `CreativeModelSelect` 接线回归。必须先查清并修复/证明，再继续预览、Apply、手动 Save 的真实 UI 链路。

不要因为所有单元测试通过就跳过这个可见问题。

## 2. Git 与 WIP

当前 HEAD：

```text
1cfbb037 docs(creative-studio): archive workflow mvp handoff
f93e9c21 docs(creative-studio): record minimal kickoff home gate
67f3d2f8 feat(creative-studio): add minimal agent kickoff home
97803724 docs(creative-studio): record durable canvas proposal gate
```

工作树包含本轮全部有效进度，禁止 reset、checkout、清理、stash 覆盖或从头实现：

- `Cargo.lock`
- `crates/agent/nomi-providers/src/{lib,retry,openai,anthropic,gemini,bedrock,vertex}.rs`
- `crates/backend/nomifun-ai-agent/src/{lib.rs,factory/provider_config.rs}`
- `crates/backend/nomifun-workshop/src/workflow_draft.rs`（新增）及 routes/service/state/lib/Cargo
- `crates/backend/nomifun-app` services/router state/builtin Skill contract
- `ui/src/renderer/pages/creativeStudio/workflows/agent/`（新增）
- Workflow Route/Page/Modal/CSS/tests
- `CreativeModelSelect` popup container 扩展
- Focus Shell / Workflow 固定 stone palette
- Canvas strict JSON duplicate-key helper 导出

功能代码尚未 commit；只提交了上一次归档文档。

## 3. 已完成后端

新增 owner-only：

```http
POST /api/creative-studio/workflow-drafts
```

严格请求：

```json
{
  "prompt": "...",
  "model": {
    "providerId": "canonical UUIDv7",
    "model": "exact model name"
  }
}
```

严格响应：

```json
{"text":"raw assistant artifact text"}
```

合同：

- App owner middleware + Workshop installation owner 二次校验。
- 顶层和 model 均 `deny_unknown_fields + camelCase`。
- prompt trim、非空、最多 20,000 UTF-16。
- model 原样已 trim、非空、最多 512 UTF-16。
- 精确 enabled Provider/model/Chat capability，不做名称推断或相邻 task 回退。
- `resolve_provider_config + one_shot_completion_bounded`。
- 一个 user message、`tools=[]`、无 history、无 reasoning/tool loop。
- 无 Conversation、Skill、MCP、产品持久化、产品级 retry 或 model failover。
- Provider 可保留既有同模型/同协议的有界传输协商；注释已准确说明，不再伪称零底层 HTTP retry。
- 120 秒 hard timeout。
- Provider lifecycle read guard 只用于阻止破坏性 Provider/model delete；普通 update 不被 120 秒阻塞，本次配置由 resolver 冻结。

## 4. Timeout / Provider 取消链

复审发现 timeout 丢弃 receiver 后，旧 Provider detached SSE task 可能继续等待并重试。本轮已经从底层闭合：

- `SecretRedactingProvider` 的外层 receiver 关闭会立即 drop 内层 receiver。
- OpenAI、Anthropic、Gemini、Bedrock、Vertex active stream drain 与 `tx.closed()` 竞争。
- retry backoff、in-flight send、response processor、error emit 全部响应 receiver close。
- receiver 关闭后不再继续等待、send、process 或 retry。
- 多 delta 输出在 append 前按 UTF-8 bytes 检查；越界立即 drop receiver。

输出上限：

- fence 内 JSON：262,144 bytes。
- canonical opening/closing fence：12 bytes。
- whole response：262,156 bytes。

## 5. Runtime Prompt / Skill / 前端严格合同

- kind：`nomifun.creative-studio.workflow-draft/v1`。
- exact top-level：`kind / summary / draft`。
- exact draft：`mode / name / description / category / promptTemplate`。
- mode：`single-image` 或 `multi-image-series`。
- 单图 placeholders：`product_name / selling_points`。
- 多图 placeholders：`topic / style / platform`。
- 至少一个允许的 placeholder；拒绝未知、未闭合、嵌套或 unmatched placeholder。
- 模型不能拥有 ID、revision、timestamps、variables、visibility、tags、attachments、assets 或 model binding。
- 产品生成 IDs/variables/defaults；visibility 固定 private；图片生成模型保持 null。
- 多图 planning 只绑定本次精确 Chat model。
- runtime system 与 packaged Skill 的 canonical example 均是合法单图模板，包含 `product_name` 和 `selling_points`。
- 前端仍把模型文本视为不可信：唯一 final lowercase json fence、duplicate decoded key、孤立 surrogate、精确字段和大小全部 fail closed。

## 6. 前端完成部分

- Workflow 页接入“AI 创建”。
- Modal 仅保留需求、精确 Chat 模型、草稿预览、应用编辑器。
- catalog 必须 ready；目录/selection 变化清草稿；Apply 前再次校验 exact row。
- Apply 只调用 `setEditing(workflow)` + `setEditingIsNew(true)`。
- 真正 repository create/save 仍只在原编辑器“保存”；不会自动运行或调用图片模型。
- `draftPort` 使用共享 authenticated `httpRequest`，125 秒客户端 timeout，strict `{text}`，whole-response 262,156 UTF-8 bytes。
- 页面、Workflow Modal、输入、Select、popup、disabled button 均使用固定浅 stone palette。
- 真实浏览器计算色证明用户浅/深主题下 page/modal/textarea/select/option/disabled primary 全部一致。
- 参考项目 Modal 已实际打开对照；目标有意删除 visibility、参考图和复杂会话功能，符合最新 MVP 范围。

## 7. 暂停前验证证据

通过：

- UI Workflow/Model/Focus/Canvas 定向：42 passed / 184 assertions。
- `bun run typecheck`。
- `bun run check:theme`、`check:icons`、`check:dead-css`。
- UI production build：7696 modules；仅既存 dynamic/static import 与大 chunk warning。
- `nomi-providers --lib`：177/177。
- receiver cancellation：6/6；retry 子集：17/17。
- bounded drain：2/2。
- Workflow route/domain：4/4。
- Workshop 全量：111/111（实施 Agent 最终复跑）。
- Skill/runtime contract：1/1。
- `cargo check -p nomifun-app`。
- Rust fmt 与 `git diff --check`。
- 最终只读审查：未发现剩余后端 P0-P2。

真实 8788 拒绝路径（没有 Provider 调用）：

- unknown `history`：400。
- exact Provider + missing model：409。
- blank prompt：400。
- 后端日志只出现对应 400/409/400，无模型调用日志。

没有真实 Provider 调用、没有媒体生成、没有新增 Workflow、没有留下页面 fetch mock。此前安装的页面内 mock 已恢复并完整 reload。

## 8. 当前本地服务

暂停时监听：

- 参考 Next：3000。
- 参考 Go：8080。
- 目标 Vite：5174。
- 当前代码的隔离 Rust backend：8788，进程 PID 在交接时需重新查询。

8788 使用：

- data dir：`C:\Users\MINISFORUM\AppData\Local\Temp\nomifun-creative-projects-qa-20260821-1748`
- cargo target：`C:\Users\MINISFORUM\AppData\Local\Temp\nomifun-target-qa-noindex-20260821`
- 当前 unified exec session：`55583`（新账号不应假设仍可访问，先查端口）。

## 9. 下一位 Agent 的第一任务

先处理模型 Select 的真实 UI 未闭合点：

1. 完整 reload `/workshop/workflows`。
2. 打开 AI 创建，输入需求，打开“对话模型”。
3. 点击 `big-pickle openai.chat_text`。
4. 检查受控 value、`data-selection-state`、selection meta 和生成按钮。
5. 比较同一 `CreativeModelSelect` 在 `/workshop` 首页的真实选择行为。
6. 检查 `NomiSelect` onChange 事件形态、popup container 变化是否影响 click，以及是否只是 Browser locator 事件差异。
7. 增加能真实触发 onChange 的交互测试；不要仅靠源码 contains。

确认选择可用后，用页面内临时 fetch interception（只拦截 `/api/creative-studio/workflow-drafts`，返回 strict artifact）验证完整 UI，但不要调用真实模型：

- Generate 显示预览。
- Apply 打开既有编辑器。
- Apply 前 GET workflows 仍为 0。
- 用户点击 Save 后才新增 1 个 Workflow。
- reload 后存在。
- 按精确 ID 删除 QA Workflow，并确认回到 0。
- 恢复原 fetch 并完整 reload。

随后复跑定向测试、typecheck、主题/图标/dead-css、UI build、Rust关键门、fresh reload Console 0 error / 0 warning，再提交功能。

## 10. 提交与后续

只有真实 UI 主链闭合后才提交：

```text
feat(creative-studio): add minimal workflow draft assistant
```

再单独更新实施台账：

```text
docs(creative-studio): record minimal workflow draft gate
```

然后继续 P11：1280×720、1024×768、390×844、Tauri 慢环、最终 Web/UI build、桌面打包、退役门禁和能力文档。复杂会话、附件、公开模板与高级媒体继续后置。

## 11. 可直接使用的启动 Prompt

```text
继续 C:\Users\MINISFORUM\code\nomifun\nomifun-desktop 的 Creative Studio 迁移，分支 codex/infinite-canvas-rebuild。

开始前完整阅读：
1. docs/handoffs/2026-08-21-creative-studio-rebuild-ledger.zh.md
2. docs/handoffs/2026-08-22-creative-studio-workflow-mvp-pause-handoff.zh.md
3. docs/handoffs/2026-08-22-creative-studio-workflow-mvp-second-pause-handoff.zh.md
4. AGENTS.md

先执行 git status --short --branch、git log -6 --oneline 和端口检查。当前 HEAD 的父功能锚点仍是 f93e9c21；工作树中有大量未提交但已验证的 Workflow one-shot、Provider cancellation、严格 artifact、Modal 和固定配色 WIP。禁止 reset、checkout、清理、覆盖或重新实现。

务必打开参考项目 C:\Users\MINISFORUM\code\nomifun\download\infinite-canvas。用户最新范围是低成本、高质量、先上线：Workflow AI 只做“简单需求 + exact Chat -> strict preview -> Apply 既有编辑器 -> 用户手动 Save”，不做复杂 Conversation、附件、公开模板、自动保存或自动运行。

后端 POST /api/creative-studio/workflow-drafts、单轮 bounded completion、timeout cancellation、runtime/Skill合同已经实现并通过审查。不要重写后端；先解决/证明真实 UI 模型选择未生效问题：目标页点击 big-pickle 后下拉关闭但 value 未显示、生成按钮仍 disabled。比较首页同组件，检查 NomiSelect onChange 与 popup container，补真实交互测试。

选择闭合后，不调用真实模型；用仅页面内临时 fetch mock 跑 Generate -> Preview -> Apply -> Editor -> 手动 Save -> reload -> 精确删除 QA Workflow，全程确认 Apply 前零持久化、Save 后才持久化，并恢复 fetch。再做 fresh console/network、三视口和全部风险匹配门禁。

完成后先提交功能，再单独更新台账并提交，然后继续 P11 直至可替换原“创意工坊”的可交付状态。按需最多 1-2 个独立 Agent，不追求高并发，不强行中断 Agent；没有成本授权不得调用真实 Provider。
```
