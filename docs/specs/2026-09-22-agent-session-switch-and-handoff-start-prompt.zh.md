# 会话内 Agent 切换与任务交接修复启动 Prompt

将下面代码块完整复制到一个新的 coding agent 会话中。它授权实施和验证，但不授权 commit 或 push。

```text
你正在 `/Users/muri/code/nomifun/nomifun-desktop` 仓库中工作。

目标：完整实施“已有会话内切换 Agent，并从下一 Turn 生效，同时实现安全、高质量的任务交接”。不要只修改前端跳转，也不要恢复已经退役的 legacy Conversation switchPreset/replace_agent_preset_snapshot 路径。请持续实施到目标行为可用并完成与风险相称的验证；若真实外部凭据不可用，必须明确区分本地/协议验证与 live-provider 未验证。

第一步必须只读完成：

1. 阅读仓库根目录 `AGENTS.md`。
2. 完整阅读 `docs/specs/2026-09-22-agent-session-switch-and-handoff.zh.md`。
3. 运行 `pwd`、`git status --short --branch`、`git remote -v`，确认当前分支和工作树。
4. 当前已知 `scripts/run-dev.mjs`、`scripts/run-dev.test.mjs` 可能存在用户未提交修改；它们不属于本任务。不得覆盖、恢复、暂存或顺手修改，除非实际检查证明任务不可避免且先向用户说明。
5. 重新核对当前代码，不把实施文档中的文件列表、HEAD、接口位置或测试状态当成无需验证的事实。

产品合同：

- 普通本地 Nomi 会话可以在当前页面切换 Agent，不跳回 Guid。
- 保持同一 AgentSession/Conversation ID、标题、消息和用户草稿。
- 新 Agent 从下一次新 Turn admission 生效。
- 首期只允许 idle Turn；生成中不做 pending switch。
- 服务端解析 exact Preset revision、ResolvedSnapshot、模型和 typed resources；客户端不得提交自造 binding/Snapshot。
- 当前模型兼容时保留；不兼容时提供 machine-readable 恢复路径，禁止静默换模型。
- 跨 Agent 只携带 data-only 消息和 deterministic handoff facts；禁止重放旧系统提示、tool calls、effects、prior task、checkpoint、process/browser/MCP 私有句柄或旧权限。
- Remote Session、AgentExecution Attempt 审计会话、产品身份强绑定会话、active AgentExecution、unsettled effects 和 pending Patch recovery 必须 fail closed。
- 不放宽 `historical_model_binding_compatible`；合法 Agent transition 前的历史需要显式分段。
- Renderer 仍只支持 desktop/Tauri 和最小 880x600；不要增加移动端布局或断点。

实施要求：

1. 先建立或调整回归测试，证明当前选择 Agent 会导航 Guid，以及目标行为缺失。
2. 按实施方案的 ASH-01 至 ASH-08 推进：
   - preview/apply DTO、typed errors 和 canonical event；
   - bounded `AgentHandoffEnvelopeV1`；
   - Store 中 binding/resource/handoff/checkpoint/active-set 的原子 CAS transition；
   - Control Plane 的 target Agent/model/resource 解析与 diff；
   - Runtime history 的 legal transition 分段；
   - canonical active capability generation 初始化；
   - Router 生命周期、teardown/effect/recovery guards；
   - 当前会话 UI、handoff mode、transition marker 和中英文文案；
   - AgentExecution downstream brief 携带 verified `output_files` quick win。
3. 不削弱 `AgentPriorTask` 的 exact-binding 验证。跨 Agent 设计独立 handoff contract。
4. 不伪造用户聊天消息来绕过 requirement citation。若 typed handoff provenance 不能在本轮安全闭合，必须把 `continue_task` 明确限制为 data-only handoff，并如实报告 completion gate 尚未继承旧 requirements；不要用模糊实现宣称完整交接。
5. Store transition 必须全原子；任何失败不得留下 binding、resource projection、active set 或 handoff payload 的部分更新。
6. Runtime teardown 成功而 DB transition 失败时，旧 binding 必须仍可在下一 Turn 重建。
7. Snapshot mismatch 只有在 canonical `session/agent-binding-changed` event 证明合法边界时才允许降级为 data-only 历史；其他 mismatch 继续失败。
8. pending Patch recovery 首期阻止切换，不得静默丢弃或跨 Snapshot 恢复。
9. 复用现有组件与 resolver，避免建立第二套 Session Store、Conversation 双写、平行 transcript 或新的 Runtime supervisor。

高质量交付重点：

- handoff 由 latest exact closed Turn 的 Plan、requirements、completion account 和 verified artifacts 确定性导出，不额外调用模型生成权威摘要。
- historical completion/evidence 只是来源信息，新 Agent 必须重新读取当前 workspace 并重新验证关键事实。
- 如果没有结构化任务状态，不得臆造 requirements；退化为 bounded message context。
- AgentExecution synthesis/downstream 必须接收 exact settled Attempt 的 verified artifact paths，不能从 assistant prose 猜路径。

验证最低要求：

- Store：CAS、rollback、Remote、active Turn、resource FK、event/idempotency、active generation。
- Runtime：model-only replay 保持；Agent boundary 分段；旧 tool/effect/system prompt/prior task 不泄漏；非法 mismatch fail closed；pending recovery 阻断。
- API/UI：同一 Session ID、旧消息不丢、selector 不跳 Guid、下一 Turn target Snapshot 生效、错误可恢复、draft 不丢、transition marker 正确。
- AgentExecution：verified output files 进入 downstream brief，旧 Attempt/prose 路径不能冒充产物。
- 按实际写集运行最小测试，并至少考虑：

  cargo test -p nomifun-agent-session --lib
  cargo test -p nomifun-agent-control-plane --lib
  cargo test -p nomifun-agent-runtime --lib
  cargo test -p nomifun-agent-execution --lib
  cargo test -p nomifun-app --test nomi_core_route_gap
  cargo fmt --all -- --check
  (cd ui && bun test src/renderer/pages/conversation)
  bun run typecheck
  bun run build:ui
  bun run check:desktop-ui-boundary
  bun run check:agent-vocabulary
  bun run check:nomi-core-live-provider
  git diff --check

跨 Store、Runtime、API 和 UI 的 targeted checks 通过后运行 `bun run check`。若失败属于既有或用户修改，给出精确证据，不得篡改无关文件来制造全绿。

真实验收：

- 若隔离凭据可用，扩展并运行 `bun run test:nomi-core-live-provider`：Agent A 建立多约束任务并产生 verified artifact/unverified item，切换 Agent B，保持同一 Session，B 下一 Turn 接收 handoff、重新检查 workspace、无旧权限泄漏并继续工作。
- mock、compile-only、fixture、截图和普通 build 不能表述为真实 provider 成功。

工作方式：

- 深度追踪真实请求、Store、Runtime、投影和 UI 链路，不能以 API 返回 200 或测试桩通过作为全部完成证据。
- 使用 `apply_patch` 编辑文件；保留用户无关修改。
- 只暂存任务文件；但本次未经用户明确要求，不要 commit，不要 push。
- 完成后报告：实现的产品语义、关键不变量、修改文件、运行命令及结果、live-provider 是否真实验证、剩余风险和工作树状态。
```
