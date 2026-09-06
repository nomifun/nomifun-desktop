# Coding Engine 远程主工作进程交接

> 交接日期：2026-09-06
>
> 交接类型：本地隔离实现 → 远程主工作进程合并、中央接线与验收

## 1. 交接基线

```text
source_branch: car/coding-engine
base_sha: 6a2a94bd192ef67eda5dd67331f6c047b1c1b315
isolated_code_commits:
  - b652fa29ce02c91f600d54abaecd98dfb967f9c4
  - c8b0193892ad7f1b73586b7570b7a2f0172c8d1b
isolated_code_tip: c8b0193892ad7f1b73586b7570b7a2f0172c8d1b
```

代码提交只包含：

```text
Cargo.lock
crates/backend/nomifun-coding-engine/
```

文档提交位于同一分支后续 commit。合并时优先合并整个分支；如果必须 cherry-pick，
按以上代码 commit 顺序取入，再取文档提交。

## 2. 当前已经完成

独立 `nomifun-coding-engine` crate 已实现：

- immutable Engine Family/Build descriptor；
- Stable/Canary channel alias 和 exact selector；
- 同一已验证 Build 的 Canary → Stable promotion；
- immutable `EngineBinding`，固定 Session、runtime binding、Build digest、profile、
  Snapshot；
- 一个 Session 一个 active turn；
- provider-neutral `CodingModelPort`；
- `ChatBrokerPort` 隔离 adapter；
- `CodingToolPlan`、canonical capability/action/resource/effect mapping；
- Tool schema canonical digest；
- text/reasoning/Tool Call/Tool Result 多 model-step continuation；
- Tool argument/result 大小边界；
- read-only parallel、effectful serial 调度；
- cancel、dispose、panic cleanup；
- Native Responses Item 和音频输出的显式 unsupported。

当前实现没有接入：

- `nomifun-app`；
- `nomifun-agent-platform`；
- `nomifun-agent-session` durable event 主链；
- `nomifun-agent-kernel`；
- Process/File/VCS/Workspace owner；
- UI、Remote 或 Automation 路由；
- 旧 `nomifun-codex-runtime` 替换或删除。

因此合并本分支不会自动切换生产 Runtime。

## 3. 已完成验证

```text
cargo fmt --package nomifun-coding-engine
cargo check -p nomifun-coding-engine
cargo test -p nomifun-coding-engine
git diff --check
```

最后一次单测结果：

```text
13 passed
0 failed
```

覆盖：

- Stable/Canary 分别解析；
- 同一 immutable Build channel promotion；
- duplicate Build 拒绝；
- exact digest mismatch fail closed；
- 不同 Session 固定不同 Build；
- plain-text turn；
- Tool Call → Tool Result → 第二次 model step；
- Session/Binding mismatch 拒绝；
- Tool schema digest mismatch 拒绝；
- bounded UTF-8 Tool Result；
- active Tool 取消；
- one-active-turn；
- dispose 幂等并拒绝新 turn。

未运行：

- Clippy：当前 stable toolchain 未安装 `cargo-clippy`；
- 全仓测试：隔离 crate 未接生产主链，按仓库规则只跑定向检查；
- live Provider/Kernel/Process/File/VCS/Session E2E：当前没有中央 owner 接线；
- macOS/Linux：当前电脑只完成 Windows 开发切片。

## 4. 远程合并顺序

1. 在远程主分支保存并核对现有 WIP。
2. 获取 `car/coding-engine`，先审查与一期收尾分支的 base 差异。
3. 合并或 cherry-pick 隔离代码提交和随后文档提交。
4. 只按当前远程 HEAD 重新生成/解决 `Cargo.lock`；不要覆盖远程其他依赖更新。
5. 运行 `cargo check/test -p nomifun-coding-engine`。
6. 建立平台级异构 Engine Registry/Factory，再接 AgentSession create/fork。
7. 依次完成 `CAR-02`～`CAR-07`；旧 Wrapper 在灰度验收前继续保留，但不能成为新
   Coding Engine 的隐式 fallback。
8. Stable 验收通过后再执行 `CAR-08` clean cut。

## 5. 远程中央接线责任

### 5.1 平台 Engine Registry

平台层新增能够同时注册以下实现的统一 Registry/Factory：

```text
Legacy Nomi Engine
NomiFun Coding Engine Build A
NomiFun Coding Engine Build B
```

不要把当前 `CodingEngineCatalog` 直接扩成平台 God Registry。它只管理 Coding
family Build；平台 Registry 应持有通用 descriptor/factory seam，并把选定结果写成
exact Session `EngineBinding`。

### 5.2 Session 选择与持久事实

- 用户只在新建 AgentSession 或显式 Fork 时选择 Engine family/channel/exact Build；
- channel resolver 只在创建时运行；
- SessionEvent/Binding 保存 exact family/build/digest，不保存一个会漂移的 channel；
- Resume 必须重新获得 exact Build；
- Build unavailable/digest mismatch 返回 typed failure；
- 同一 Session/Turn 禁止切换 Engine；
- 禁止 Engine failure 自动 fallback；
- Fork 到另一 Engine 是显式用户动作，并创建新 Session。

### 5.3 ChatModelBroker

当前 `ChatBrokerPort::open_chat_stream` 没有 cancellation 参数。远程 `CAR-02` 必须：

- 为 Broker/adapter 增加不泄漏到 JSON 的进程内取消传播；
- 确保取消终止 Provider attempt，而不只是停止转发；
- 保持 Broker 是 route retry/failover 的唯一 owner；
- semantic output 或 Tool Effect 后不 failover；
- 保持 `ChatModelRequest`/`ChatModelEvent` 为唯一模型合同。

### 5.4 Capability Kernel

远程 `CAR-03` 必须将 `CodingToolBinding` 对齐到 Snapshot 编译结果：

```text
agent_session_id
resolved_snapshot_ref
active_set_generation
capability_id / action_id
schema_digest
resource_binding_ids
principal / owner
effect_class
operation / idempotency identity
```

Capability handler/owner Port 需要接收取消或等价 lifecycle signal。不得让 Coding
Engine 直接调用具体 Browser、Computer、MCP、Plugin 或文件 handler。

### 5.5 Process/File/VCS/Context

- Process/PTY/stdin/tree cleanup 接 `nomi-process-runtime`；
- Patch/File 接 `nomifun-file` owner；
- VCS 进入 NomiFun canonical owner；
- Workspace 必须是 typed resource，不使用宿主进程 cwd fallback；
- Context 从 SessionEvent 重建；
- AGENTS.md、compaction、checkpoint 和 resume 按 `CAR-06` 接入。

### 5.6 SessionEvent 与 UI

- `CodingEventSink` 适配到 canonical SessionEvent/transient stream；
- durable event 只保存语义结果，不逐 delta 持久化；
- UI 显示 exact Engine Build、Stable/Canary 选择和 unavailable 原因；
- Existing Session 不显示可直接“热切换”的操作；
- Remote/Automation 使用同一 AgentSession/EngineBinding 主链。

## 6. 远程验收最小闭环

```text
新建 Session 并选择 Coding Canary
→ 冻结 exact EngineBinding
→ 模型 text/tool-call stream
→ Kernel admission
→ File 或 Process owner 真实执行
→ Tool Result continuation
→ SessionEvent completed
→ cancel/dispose
→ Resume 同一 exact Build
```

并行验证：

```text
Legacy Session 继续使用 Legacy Engine
Coding Stable Session 使用 Build A
Coding Canary Session 使用 Build B
```

更新 Canary/Stable channel 后，以上既有 Session 的 Binding 必须保持不变。

## 7. 禁止事项

- 不启动或打包 `codex-app-server`；
- 不把 `../codex` 加为 path dependency；
- 不把新 Engine 接到旧 Sidecar；
- 不复用一期 `CodexRuntimeReleaseManifest`、Sidecar digest 或旧 frozen source SHA
  作为新 Engine Build/provenance 合同；
- 不用旧 Wrapper 作为 Coding Engine fallback；
- 不修改 `docs/specs/2026-08-28-agent-capability-platform-v2/`；
- 不恢复旧 `/api/presets`；
- 不引入第二套 Session、Model、Tool、File、Process、Catalog 或权限事实；
- 不允许同一 Session 双 Engine 执行或中途切换；
- 不在灰度验收前删除 Legacy Engine；
- 不宣称停止消费 Broker stream 等于 Provider 已完成取消。

## 8. 远程完成回传

```text
merged_source_commit:
remote_base_sha:
integration_commit:
engine_registry_path:
session_binding_path:
broker_cancellation_result:
kernel_cancellation_result:
owner_integrations:
checks:
not_run_and_reason:
legacy_engine_gray_result:
coding_stable_canary_result:
same_session_switch_rejection:
blockers:
follow_up:
```
