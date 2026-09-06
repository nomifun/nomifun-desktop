# CAR 验证、发布与旧 Runtime 切换

## 1. 验证原则

验证只证明本阶段实际改变的行为，不复制上一阶段的超大证据矩阵。

优先级：

1. Runtime actor 和 Tool Loop 行为；
2. NomiFun Model/Capability/Session Port 边界；
3. Process/Patch/File/VCS 真实效果；
4. Context/Compaction/Resume/Cancellation；
5. 旧 Sidecar/Wrapper production reachability；
6. 三个平台真实 Artifact。

历史阶段文档、旧 Gate 结果和旧 Sidecar artifact 不得作为 CAR 通过证据。

## 2. 门禁分层

### CAR-G0：文档与源快照

检查：

- Codex commit 固定；
- 选定源路径存在；
- copy/adapt/rewrite/exclude 完整；
- LICENSE/NOTICE 可追溯；
- voice/realtime/TUI/CLI/Guardian/app-server 已排除；
- 无新增 `../codex` path dependency。

### CAR-G1：Runtime Core

检查：

- in-process Coding Engine actor；
- 多 Engine Build 可注册并独立解析；
- 新 Session 的 exact EngineBinding 可冻结；
- 同一 Session 不可切换 Engine，既有 Binding 不受 channel promotion 影响；
- fake model plain-text turn；
- bounded stream；
- one active turn；
- cancel/dispose/panic cleanup；
- actor crash 后无任务泄漏。

### CAR-G2：Model Stream

检查：

- ChatModelBroker 是唯一模型入口；
- exact route/causality/credential；
- text/reasoning/Tool Call/Tool Result/usage/terminal；
- semantic output 前后的 retry 边界；
- cancellation/backpressure。

### CAR-G3：Coding Tool Loop

检查：

- Tool Call delta 有界拼接；
- model-facing mapping；
- Snapshot/active-set/schema/resource admission；
- Tool result continuation；
- read-only parallel；
- effectful serial；
- Snapshot 外能力 fail closed。

### CAR-G4：Coding Owners

检查：

- File/Patch；
- Process/PTY/stdin；
- VCS；
- Workspace/AGENTS；
- timeout/cancel/process-tree；
- secret scan。

### CAR-G5：Session 主链

检查：

```text
AgentSession open
→ Runtime actor
→ model step
→ Tool call
→ owner effect
→ next model step
→ completed/cancelled/failed
```

同时验证：

- SessionEvent 唯一事实；
- UI/Remote/Automation 不产生第二 Session；
- Legacy Engine 与 Coding Engine 可在统一平台 Registry 中并存（由远程集成阶段验证）；
- channel 只影响新建/Fork Session，不影响既有 Session；
- no external Codex process；
- Snapshot 不漂移。

### CAR-G6：旧实现删除

生产 reachability 必须为零：

```text
nomifun-codex-runtime crate
codex-app-server executable
app-server --listen
runtime/hello
native_action/start
runtime/session/dispose
RuntimeStartTurnBrokerBridge
sidecar_artifact / runtime_sidecar production fields
```

审计脚本必须只扫描生产 Cargo、Rust、Schema、打包、路由和启动配置；不把历史
文档和 Git 对象当作生产残留。

### CAR-G7：发布

三个首发平台：

- Windows Desktop x64；
- macOS Desktop arm64；
- Linux Desktop x64。

每个平台必须验证同一 RC 的：

- build/package/install；
- fresh launch；
- Coding read/search/patch/exec/diff；
- cancel/crash/dispose；
- secret leakage；
- Session resume。

## 3. 测试矩阵

不做全组合矩阵，采用代表性闭环：

| 层 | 最小代表测试 |
|---|---|
| Model | text → tool call → tool result → continuation |
| Tool | Snapshot 内成功、Snapshot 外失败、schema mismatch |
| Process | 长输出、stdin、timeout、tree cleanup |
| Patch | invalid/context mismatch/atomic failure |
| VCS | status/diff/stage/commit |
| Context | AGENTS precedence、bounded loading、history rebuild |
| Compaction | summary replacement、retained facts、resume |
| Lifecycle | cancel、crash、dispose、late event |
| Ecosystem | MCP/Plugin/MiniApp contribution + non-Agent consumer |
| Release | 三平台同一 RC bytes |

## 4. 失败与回滚

### 4.1 CAR-00～CAR-06

如果新 Coding Engine 尚未接入产品路由：

- 保留旧代码；
- 普通 revert 新任务提交或修复；
- 不增加兼容 alias；
- 不修改旧阶段文档。

### 4.2 CAR-07

如果主链切换失败：

- 停止切换；
- 保留用户数据和 SessionEvent；
- 回到前一个未切换的开发提交；
- 不在同一 Session 中混用旧 Wrapper 和新 Runtime；
- 修复后建立新候选。

### 4.3 CAR-08 之后

旧 Wrapper 删除是 clean cut：

- 不恢复长期 Sidecar；
- 不恢复旧 `/api/presets`；
- 不用旧 rollout/history 恢复新 Session；
- 失败时从 NomiFun forward-fix commit 生成新候选。

## 5. 发布锁与证据

CAR 的 release lock 只记录：

```text
source commit
target platform
NomiFun Host artifact
NomiFun Agent Runtime artifact/build identity
Package artifact（如本阶段涉及）
actual suite
logs
```

禁止记录：

- Codex binary；
- Sidecar digest；
- provider credential；
- 本机绝对路径；
- 旧阶段 synthetic digest。

## 6. 完成定义

### 6.1 设计完成

- 00～07 文档和两个机器 Manifest 完整；
- 每个任务有唯一写集、依赖、验收和停止条件；
- 旧阶段文档无修改；
- Codex 选取和排除边界可审查。

### 6.2 Coding Runtime 内化完成

- Agent turn loop 在 NomiFun 进程内；
- 模型只经 ChatModelBroker；
- Tool 只经 Capability Kernel；
- File/Process/VCS 只经 NomiFun owner；
- SessionEvent 是唯一产品事实；
- reasoning/Tool Call/Tool Result 多轮闭环可用；
- compaction/resume/cancel/steer 可用；
- 不运行 app-server/Sidecar；
- 旧 Wrapper production reachability 为零。

### 6.3 Stable admission

- CAR-G0～CAR-G7 全部通过；
- Windows、macOS arm64、Linux Desktop x64 使用同一 RC；
- 无 P0 数据损坏、secret 泄漏、进程泄漏或核心 Coding 流程失败；
- 旧阶段收尾不被 CAR 任务打断；
- Plugin/MiniApp 仍可服务非 Agent consumer。

## 7. 本阶段明确不负责

- 重新设计 AgentPreset；
- 修改旧阶段 01～06；
- 重新设计 Plugin/MiniApp 产品；
- 迁移 Voice/Realtime；
- 构建 Codex app-server；
- 在本阶段把 Legacy Engine 与 Coding Engine 接入统一生产主链；
- 性能 benchmark、模型质量评分和长期在线 canary。
