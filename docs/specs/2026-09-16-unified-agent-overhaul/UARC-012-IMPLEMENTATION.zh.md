# UARC-012 单一 AgentSession owner 与 API 实施记录

> 开始 barrier：`9b62887aee46130a144bca7369c01823718e79c8`
> 实现提交：`82954016810ed5fabe48248adc4952d2bd5f199e`
> 数据源：main SQLite 中的 Agent Store generation 5
> 平台：共享 Rust/API；Windows 执行工程验证

## 交付

- 在 `nomifun-conversation` 建立 `CanonicalAgentSessionOwner`，以同一个 main `SqlitePool` 上的
  `AgentSessionStore` 作为 Session、Turn、Event、Message projection、Resource、Fork 和 Delete 的
  唯一事实 owner。
- `/api/agent-sessions` 直接创建一个 `agent_session_id`，不再先创建 Conversation 再补 AgentSession；
  create replay 会核对 owner、冻结 binding、标题和初始 active capability set。
- Turn admission 在单个 Store 事务中提交 `message/user-accepted` 与 `turn/started`，并在写锁内选择
  exact predecessor。一个请求只产生一个 `agent_turns` receipt；第二个 active Turn 被拒绝。
- `turns/steer` 和 `turns/cancel` 都在同一 active-turn fence 下选择 exact operation；重复 key 返回原
  cursor，改变 steer input 会得到 idempotency conflict。
- Turn replay 不依赖当前 head。即使原 Turn 已完成或取消，相同 key 和输入仍返回原 receipt；改变输入
  不会创建新 Turn，也不会把旧 receipt 解释成新操作。
- Fork 原子创建 self-contained child、继承父 Session 的冻结 binding 和 active capability set，并提交
  child `session/ready`；因此 child 可立即接纳 Turn。改变同一 fork key 的 cursor、标题或 binding 会
  fail closed。
- Delete 只留下 generation 5 精确 tombstone；重复请求返回原 `deleted_at`，不伪造新的删除时间。
- Event 与 Message API 直接读取 canonical events/projection；测试删除 projection 后从 events 重建，
  用户输入内容与 cursor 保持一致。
- App composition 只创建一个 canonical owner 并注入 `NomiCoreSessionOwner`。本地 AgentSession REST 与
  model-facing `SessionControlSink` 都通过该 owner；能力查询读取 Store 中真实 active set。

## 物理删除与 authority 边界

- 从 canonical AgentSession handler 区段删除 Conversation create/get/send/fork/transcript-copy bridge、
  compatibility Session observation/message projection 和“不支持 events”的占位路径。
- 新 API 不再从 Conversation `extra` 读取 Runtime、Agent binding、MCP 或 capability authority；
  Preset、capability 和 MCP 的 session 内变更明确返回 immutable-binding conflict。
- create/turn/steer/cancel/fork/delete handler 的静态边界测试禁止重新引入
  `create_session_idempotent`、`load_owned_nomi_core_session`、`service()`、`session_metadata` 或
  request `extra` authority。

## 串行边界与保留项

- `ConversationService` 仍暂时服务 Cron、Channel、AutoWork、Companion、IDMM、AgentExecution 和旧
  Remote 路径；这些不是 `/api/agent-sessions` 的双写/兼容 reader，迁移与删除 owner 分别是
  `UARC-033/034/051`。用户界面的“对话”术语仍可作为 canonical AgentSession projection 保留。
- UARC-012 负责 durable admission 和唯一 receipt，不在此任务接回旧 Nomi Manager。accepted Turn
  到唯一官方 Driver 的 dispatch、cancel/cleanup 与 terminal journal 由紧随其后的串行 `UARC-013`
  完成；期间没有 legacy fallback 或第二份 Session 写入。
- `runtime_engine_binding` 响应字段当前只返回 `null`，字段的合同物理删除归 `UARC-014`，没有作为
  Session authority 保存。

## 验证

```text
cargo test -p nomifun-agent-session --lib -- --test-threads=1
  27 passed

cargo test -p nomifun-conversation --lib -- --test-threads=1
  335 passed

cargo test -p nomifun-api-types --lib -- --test-threads=1
  534 passed

cargo test -p nomifun-app router::nomi_core_session::session_boundary_tests --lib -- --test-threads=1
  6 passed

cargo check -p nomifun-app --lib
  passed（既有 warning 保留）

cargo fmt -p nomifun-agent-session -p nomifun-conversation -p nomifun-api-types -p nomifun-app -- --check
  passed

bun scripts/check-uarc-boundary.mjs --self-test
  passed；3,032 个 production files，13 组旧路径均未增长

git diff --check
  passed
```

canonical owner 的生命周期测试覆盖 open/turn/steer/cancel/fork/delete、终态后的 exact replay、输入或
配置改变时的冲突、one-active-turn、Fork child ready、active set 继承、projection rebuild、原时间戳
删除重放和父子隔离。UARC-012 completion gate 因而收敛为一个 Session ID 与一个 canonical Turn
receipt；Runtime 只允许在 UARC-013 从该 receipt 接管执行。
