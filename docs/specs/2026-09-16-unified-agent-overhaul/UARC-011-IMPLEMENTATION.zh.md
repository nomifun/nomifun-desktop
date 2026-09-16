# UARC-011 Canonical Agent Store baseline 实施记录

> 开始 barrier：`dd2c098dab8bc13bc8693719953f9864498360d4`
> 实现提交：`fa164520f72a053e8e244721cb9682bc58b1269b`
> 数据代际：Agent Store generation 5
> 平台：共享 SQLite/schema；Windows 执行工程验证

## 交付

- 将 `0001_fresh_v4.sql` 物理重命名为 `0001_agent_store.sql`，并建立 generation 5 schema manifest。
- canonical Agent facts 为：
  `agent_sessions`、`agent_turns`、`agent_events`、`agent_payloads`、`agent_effects`、
  `agent_session_resources`；`agent_messages` 和 `agent_session_heads` 是可重建 projection。
- `agent_payloads` 只允许 inline body 或 `objects/<sha256>` content-addressed object ref 二选一；
  checkpoint locator/digest 继续作为可丢弃派生数据保存在 Session head。
- `AgentSessionStore` 继续使用一个 `SqlitePool` 和同一事务边界；migration 109 将相同 Agent Store
  DDL 安装进当前 main SQLite，定向测试逐表比较 clean baseline 与 main schema。
- Turn lifecycle 在 append `turn/*` event 的同一事务写入 `agent_turns`；`read_turn_receipt` 只读取该
  canonical row 并按 frozen event IDs 取得证据，不从 message 文本推断终态。
- Effect lifecycle 在 append `effect/*` event 的同一事务写入统一 `agent_effects` ledger，冻结
  owner domain、Module、Action、Turn、resource binding/key、input digest 和 bounded observation。
  `pending/unknown` 对同一 domain/resource key 保持唯一，未知外部效果不能自动重放。
- Session 创建/分叉在同一事务把 `AgentBindingValue.typed_resource_bindings` 写入
  `agent_session_resources`；foreign owner、重复 binding 和空 operation fail closed。
- 删除旧 projection 中嵌入 `events[]` 的兼容读取/归约器；generation 5 不导入旧 transcript。
- 新增 `reset_agent_data`：按 schema `reset_scope` 在一个事务中清空 Session/Turn/Event/Effect/
  Resource/Preset/Snapshot/Remote binding 数据，同时验证 Provider、Plugin、MCP、Skill、设置等已知
  非 Agent 表行数不变；未知非 Agent domain 表也不被触碰。测试额外证明 users 与 Knowledge 保留。

## 当前主库接线

`109_uarc_agent_store.sql` 只安装新 generation，不迁移旧 Agent 行，也不建立双写。当前旧
Conversation/Message/Effect 表仍由生产旧链使用，owner 是 `UARC-051/054`；`UARC-012` 从同一个
main `SqlitePool` 接入新的唯一 Session API。Wave 6 切换后才删除旧 writer/reader 和历史 migrations。

`nomifun-v4-root` 仍通过临时常量/type alias 编译，但它消费的已经是 generation 5 schema；该 root
coordinator 与 Fresh-v4 名称明确归 `UARC-054` 删除。新 Store、迁移和 schema manifest 不使用
Fresh-v4 身份。

## 删除与 reachability

- 新 baseline 与 `nomifun-agent-session` 生产源码中不存在
  `session_events`、`session_payloads`、`session_heads`、`message_projection`、旧 Conversation receipt/
  runtime/effect table 名或 `0001_fresh_v4.sql`。
- `nomifun-agent-session` 生产源码中不存在 legacy reader/normalizer；带旧 `events[]` 的 projection
  在测试中被明确拒绝。
- Nomi filesystem transcript 不属于新 schema，也没有进入新 Store。
- Fresh-v4 root coordinator 残留只在 `nomifun-v4-root` 和兼容 aliases，已有唯一删除 owner
  `UARC-054`，不是无 owner wrapper。

## 验证

```text
cargo test -p nomifun-agent-contracts
  105 passed

cargo test -p nomifun-agent-session --lib -- --test-threads=1
  27 passed

cargo test -p nomifun-db --test agent_store_reset -- --test-threads=1
  3 passed

cargo test -p nomifun-db --test id_schema_contract -- --test-threads=1
  20 passed

cargo test -p nomifun-db --test published_main_migration_upgrade
  5 passed

cargo test -p nomifun-v4-root --lib
  14 passed

cargo test -p nomifun-db --lib -- --test-threads=1
  411 passed（在 migration 109 落盘前的同任务 DB 基线；其后 migration/schema 定向门禁均通过）
```

默认并行 `nomifun-db --lib` 与 ID schema integration tests 会因共享 SQLite/migration 资源长时间排队；
完整 411 项在显式单线程下 496.83 秒通过。该行为已登记到 UARC-001 inventory，并将 DB crate 的
Wave gate 固定为 `--test-threads=1`，不把重复运行当作 flaky 修复。
