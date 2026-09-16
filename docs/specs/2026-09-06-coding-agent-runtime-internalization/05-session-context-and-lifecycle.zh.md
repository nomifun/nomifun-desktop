# CAR Session、Context 与生命周期方案

## 1. 唯一事实边界

产品事实只由 NomiFun 保存：

```text
AgentPresetRevision
ResolvedSnapshot
AgentBinding
AgentSession
SessionEvent
UI Projection
```

Runtime 只保存进程内、可丢弃的执行状态：

```text
exact EngineBinding copy
active turn
model stream
partial tool arguments
pending tool tasks
token counters
compaction working state
disposable checkpoint
```

不引入 Codex rollout DB、thread store、第二个 Conversation aggregate 或私有
历史文件格式。

## 2. Engine Binding、切换与 Fork

`EngineBinding` 是 Session 级不可变执行事实，至少固定：

```text
agent_session_id
runtime_binding_id
engine_family_id
engine_build_id
engine_build_digest
runtime_profile
resolved_snapshot_ref
```

规则：

- 新建 Session 时可按 Stable/Canary 或 exact Build 选择，Resolver 随即冻结 exact
  Build；
- Resume 必须重新获得同一 exact Build；Build 不可用或 digest 不匹配时 fail closed；
- 同一个 Session 或正在运行的 turn 不允许切换 Engine；
- 用户希望换 Engine 时只能显式 Fork 为新的 AgentSession，并创建新的 Binding；
- Fork 可以继承允许继承的语义 history，但不能继承旧 actor、pending Tool、Provider
  round、process handle 或 disposable checkpoint；
- Engine failure 不自动 Fork、不自动 fallback，也不由后台重写 Binding；
- Stable/Canary channel 后续指向新 Build，只影响之后创建或 Fork 的 Session。

## 3. ContextAssembler

输入固定为：

- immutable Snapshot/RuntimeProfile；
- SessionEvent history reader；
- 当前 user input、steer、follow-up；
- typed Workspace；
- AGENTS.md；
- selected Skill instructions/resources；
- selected Capability/tool summaries；
- bounded Tool Result；
- 当前 token budget 和 retained facts。

输出为当前 model step 的 `ChatModelInput`。ContextAssembler 不读取 root SQLite、
Plugin/MiniApp private dataDir、Credential store、全局未选 Catalog 或 Codex rollout。

当前隔离实现已提供 bounded `CodingContextAssembler`、AGENTS.md reader、
`CodingCompactionSummary` 和 `CodingCheckpoint`。它们只处理内存中的 bounded
projection；SessionEvent history reader、durable compaction event 和 resume application
service 仍由远程 `CAR-06`/`CAR-07` 接入。

## 4. Workspace 与 AGENTS.md

可以借鉴 Codex 的层级发现算法，但规则归 NomiFun 冻结：

1. Workspace root 来自 typed resource binding；
2. 只在 root 到当前工作目录范围内查找；
3. root/parent/child precedence 明确且稳定；
4. 单文件大小、总字节、总 token、层级深度和读取次数有界；
5. containment 失败或读取失败返回 context warning，不伪造成功；
6. `AGENTS.md` 不能写入 SessionEvent、credential 或隐私历史；
7. context 变化只影响下一次 model step，不修改 Snapshot；
8. 未绑定 Workspace 时不自动使用当前进程 cwd。

## 5. History 重建

模型 history 由 NomiFun SessionEvent 和已确认的 bounded projection 重建：

```text
SessionEvent
  → semantic history mapper
  → user/assistant/reasoning/tool-call/tool-result messages
  → ContextAssembler
  → ChatModelRequest
```

Transient delta 不必逐条持久化。正常完成至少保存：

- 最终 assistant message；
- Tool invocation/result 摘要；
- usage（按既有 Session 规则）；
- terminal turn event。

中断最多保存一份 bounded partial；不能让每个 delta 形成长期 event log。

## 6. Compaction

### 6.1 触发

Runtime 根据 input/output/reasoning token 和模型 context window，在超出安全阈值
前触发 compaction。触发点属于 Runtime turn loop，不由 UI 或 Provider 自己决定。

### 6.2 摘要内容

摘要至少保留：

- 当前任务和用户约束；
- Workspace/代码状态摘要；
- 已完成 Tool 和重要结果；
- 未完成工作；
- 当前 Snapshot/route/context 的必要身份；
- 后续行动所需的 retained facts。

### 6.3 事实处理

Compaction 只替换模型窗口，不删除原 SessionEvent。写入一个 bounded、typed
的 `session/context-compacted` 语义事件（最终名称以 Registry 为准），包括：

- source cursor；
- summary；
- route identity；
- snapshot digest；
- compaction operation identity。

不得调用 Codex 私有 compact endpoint，也不得把原始 Codex checkpoint 当作产品历史。

## 7. Resume 与 Checkpoint

Resume 的权威输入：

```text
AgentSession
  + SessionEvent
  + immutable Snapshot
  + current Runtime availability
```

可选 checkpoint key：

```text
agent_session_id
  + snapshot_digest
  + engine_family_id
  + engine_build_id
  + engine_build_digest
  + last_event_cursor
```

checkpoint 只能是加速恢复的 disposable cache：

- 缺失、损坏、过期或 digest 不匹配时丢弃；
- 不做旧 rollout converter；
- 不按 thread ID、本地文件名或 provider response ID 恢复；
- 不改变 Snapshot、Session、EngineBinding 或 route identity；
- 从 NomiFun facts 重新构造下一次模型输入。

## 8. Steer、Follow-up、Cancel

### 8.1 Steer/Follow-up

每次 steer/follow-up 必须：

- 经 AgentSession application service 生成 operation/causation identity；
- 写入 SessionEvent；
- 使用同一 EngineBinding、Snapshot 和 route；
- 在 turn boundary 或安全插入点处理；
- 不绕过正在运行的 state-changing Tool；
- 不重新扫描 Catalog；
- 不自动扩展 active capability set。

### 8.2 Cancel

```text
cancel request
  → Session admission
  → cancel model stream
  → cancel pending Tool tasks
  → ask Process owner to stop command
  → timeout 后 hard-kill descendant tree
  → append one terminal cancelled/failed event
  → release actor resources
```

取消必须幂等。取消后的迟到 provider/tool event 不得重新打开 turn，也不得重放
uncertain effect。

## 9. Dispose 与 Crash

Runtime actor dispose：

- 停止接受新的 turn；
- 取消 model stream、Tool task、timer 和 transient channel；
- 请求 Process owner 清理命令子进程；
- 等待有限时间；
- 由 owner 返回真实 cleanup report；
- 重复 dispose 返回稳定结果。

App crash 或 actor panic 后，启动恢复只根据 AgentSession 状态、SessionEvent 和
disposable cache 判断；不恢复 Codex 私有状态。无法确认的外部 Effect 保留原
idempotency identity，交给 owning domain reconcile，不自动重试。

## 10. 事件建议

最终事件名称以 NomiFun SessionEvent Registry 为准，建议覆盖：

```text
runtime/bound
session/ready
turn/started
model/step-started
message/content-part
message/reasoning-bounded
tool/started
tool/completed
tool/failed
session/context-compacted
turn/completed
turn/cancelled
turn/failed
runtime/disposed
```

Runtime event 只能通过 `SessionEventSink` 进入产品主链。Runtime private event ID、
stdout、provider stream 和 checkpoint 不得直接驱动 UI 或领域操作。
