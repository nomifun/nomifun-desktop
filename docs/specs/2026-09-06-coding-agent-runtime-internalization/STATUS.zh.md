# CAR 阶段状态

> 更新时间：2026-09-13
>
> 当前阶段：**隔离代码已合入本地主重构分支；开放 Runtime/目录/Coding 适配器已实现，生产接线未完成**
>
> 唯一状态源：本文与 `TASK-MANIFEST.json`。上一阶段
> `2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md` 不记录 CAR 状态。

## 阶段状态

| 项目 | 状态 |
|---|---|
| 文档系列 | in_progress |
| 源码/许可证基线 | completed |
| 通用 Engine Catalog/exact Binding 合同（未持久接线） | completed |
| 开放 Registered Runtime / Coding adapter | implemented |
| 隔离 Coding Engine Core | completed |
| Model Broker 原生取消/中央适配 | in_progress |
| Kernel Tool admission/owner adapter | pending_validation |
| Process owner adapter | pending_validation |
| Patch/File/VCS/Workspace owner 接线 | planned |
| Context/Compaction/Resume contracts | pending_validation |
| AgentSession 主链与异构 Registry | in_progress（按 CAR-D-019 保留现有 owner） |
| 旧 Wrapper 删除 | planned |
| 生态消费者接入 | planned |
| 三平台发布 | planned |

## 任务快照

| 任务 | 状态 | 依赖 | 备注 |
|---|---|---|---|
| `CAR-00` | completed | 无 | Codex commit、选取/排除、LICENSE/NOTICE 和 owner 边界已核对 |
| `CAR-00A` | completed | `CAR-00` | Coding family Catalog、immutable Build、Stable/Canary alias 和 exact Binding |
| `CAR-01` | completed | `CAR-00A` | 独立 `nomifun-coding-engine`，未接生产组合根 |
| `CAR-02` | in_progress | `CAR-01` | Broker 原生取消已实现；当前生产 Session/真实 Provider 的端到端验收待接线 |
| `CAR-03` | pending_validation | `CAR-02` | Kernel adapter、Snapshot/active-set admission、标准 Tool surface 本地完成；等待主链验证 |
| `CAR-04` | pending_validation | `CAR-03` | `nomi-process-runtime` adapter 已完成；Windows start/wait/stdin/timeout/cancel/output 已验证，跨平台待远程 |
| `CAR-05` | planned | `CAR-03` | File/Patch/VCS/Workspace 通过 Wave2 owner 接线，未接 AgentSession 主链 |
| `CAR-06` | pending_validation | `CAR-03` | AGENTS/context/compaction/checkpoint contracts 已完成；SessionEvent/resume 主链未接 |
| `CAR-07` | in_progress | `CAR-04`～`CAR-06` | 开放目录/Registered/Coding adapter 已实现；默认生产 owner ports 与持久绑定仍待接线 |
| `CAR-08` | planned | `CAR-07` | 删除旧 Wrapper/Sidecar |
| `CAR-09` | planned | `CAR-08` | 平台生态与非 Agent consumer |
| `CAR-10` | planned | `CAR-08`、`CAR-09` | 三平台发布和 Stable admission |

## 2026-09-13 主重构分支本地合入

- 目标分支 `rf/agent-capability-platform-v2`，起点 `08caa20d7`。
- 已按顺序取入 CAR 的 8 个独立提交；对应本地 tip `924b6dccb`。
- 本地实现提交：`6d87dc232`（开放目录/Registered/Coding adapter、取消与兼容修复）。
- Kernel 测试更新为当前 contribution lock、Revision digest 和目标资源绑定合同。
- CAR-02：Broker 原生取消和 Coding adapter 已接通；端到端 Session/真实 Provider
  验收仍未执行，不将该任务标记 completed。
- CAR-07：采用当前 Nomi-core owner，通过开放接口接入任意用户 Runtime。
  Nomi factory 已返回 Registered；通用目录拒绝 Build 覆盖、摘要漂移和未知 profile。
  Coding adapter 在现有 registry 中验证构造、取消、事件投影和清理失败隔离。
  尚未安装默认生产 host；没有切换默认路由或引入第二套持久 Session。
- 完整证据与建议见 `LOCAL-INTEGRATION-2026-09-13.zh.md`。
- 已验证 `cargo test -p nomifun-coding-engine -p nomifun-chat-model-broker`：
  Coding **40 passed**；Broker unit **10 passed**、conformance **21 passed**。
- 已验证 `cargo test -p nomifun-ai-agent --lib runtime_`：**70 passed**，
  覆盖开放目录、registry、runtime state 和相关 option 合同。
- 已验证 `cargo test -p nomifun-app --lib router::chat_broker_host::tests`：
  **9 passed**，并编译经过 Conversation/Remote/Automation 等下游依赖。
- `cargo test -p nomifun-ai-agent --lib coding_runtime::tests`：**7 passed**；
  `cargo test -p nomifun-ai-agent --lib factory::tests`：**3 passed**。
- `cargo test -p nomifun-app --test nomi_core_route_gap`：**36 passed，2 failed**。
  `installation_token_is_limited_to_headless_product_control_planes` 的 `/api/plugins`
  期望 403，实际 200；`agent_session_model_selection_is_exact_persistent_and_keeps_the_agent_unchanged`
  未提供 `fs.read` 所需 workspace selection，收到 `RESOURCE_SELECTION_REQUIRED` 422。
  路由、资源解析器和测试文件相对目标起点 `08caa20d7` 无改动，失败发生在 Runtime
  构造前；这是静态差异核对，**未在旧 checkout 上重跑基线**，不宣称全绿或已修复。
  同一已构建测试二进制单独运行插件认证用例（`--exact`）仍失败，排除了仅由
  本次并行运行导致的偶发现象；没有改动认证逻辑或放宽断言。
- `git diff --check` 与 manifest JSON 解析通过。新模块单独执行 rustfmt；
  未把仓库默认 `disable_all_formatting = true` 下的空操作当作格式验证。
- 未运行真实付费 Provider、桌面 UI E2E、macOS/Linux、发布构建或全仓测试；
  本次没有生产接线，不宣称 UI/多引擎灰度闭环完成。

## 源分支隔离交付（2026-09-06 历史基线）

基线：

```text
branch: car/coding-engine
base_sha: 6a2a94bd192ef67eda5dd67331f6c047b1c1b315
code_commits:
  - b652fa29ce02c91f600d54abaecd98dfb967f9c4
  - c8b0193892ad7f1b73586b7570b7a2f0172c8d1b
  - 0e33dbed53e248ef5c926b548bfde378120bb400
  - f3ff31b4b6168c2cc6b4b3de867b5f922a3eb8a4
  - 449e06110c34902e070a1b7b506bdda2db9f147a4
code_tip: 449e06110c34902e070a1b7b506bdda2db9f147a4
```

已验证：

```text
cargo fmt --package nomifun-coding-engine
cargo check -p nomifun-coding-engine
cargo test -p nomifun-coding-engine
git diff --check
```

最后一次定向测试结果：`36 passed; 0 failed`。

未运行：

- `cargo clippy -p nomifun-coding-engine --all-targets -- -D warnings`：当前 stable
  toolchain 未安装 `cargo-clippy`；
- 全仓构建/测试：本次只新增未接生产主链的独立 crate，按仓库规则使用定向检查；
- live Provider、Broker 原生取消、AgentSession/SessionEvent E2E：这些属于远程中央接线；
- File/Patch/VCS 的真实产品路由：当前通过 Kernel/Wave2 owner adapter 留出合同，
  尚未接入生产 AgentSession。

## 已知集成缺口

- CAR-02 已新增 `open_chat_stream_cancellable`；取消覆盖本机 attempt future。
  当前生产 Session 的端到端验收仍待接线，不承诺 Provider 服务端停止计算；
- Capability handler 合同尚无 `CancellationToken`；当前 Kernel adapter 只能在调用
  future 层 fail-fast；
- 原 `CodingEngineCatalog` 仍只管理 Coding family；新增通用 `RuntimeEngineCatalog`
  可注册任意实现，但尚未与生产 Session 创建/Fork 事务和 API 发现入口连接；
- `NativeResponsesItem` 与音频输出在 Coding P0 明确 unsupported；
- SessionEvent durable projection、UI、Remote/Automation、Workspace/File/VCS owner
  的生产接线尚未完成。

远程交接与禁止事项见 `HANDOFF-REMOTE-INTEGRATION.zh.md`。

## 状态更新规则

任务状态变更必须同时写明：

- `task_id`；
- 变更原因；
- commit SHA（如果已有代码提交）；
- 实际验证命令；
- 未运行项和准确原因；
- blocker 或 follow-up。

不在本文写入 API key、credential、主机地址、完整模型响应或秘密日志。
