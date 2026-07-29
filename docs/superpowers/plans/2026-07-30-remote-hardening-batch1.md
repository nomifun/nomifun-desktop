# 远控硬化第一批（D3 远程解锁 + D4 错误结构化）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给远程控制链路补上"重启后可远程解锁"与"结构化可重试错误"两块地基（spec §设计 D 的 D3/D4；D0 部署验证是用户在 Mac 上的动作，不在本计划内）。

**Architecture:** ① `conversation_delivery_receipts` 增两列（`result_error_code`/`result_error_retryable`），由 `RelayTerminal`→结构化映射函数在回合终结时写入，wire 类型透传；② gateway conversation 域新增 `nomi_stop_conversation`（Destructive 级），并让 `nomi_conversation_status` 对重启隔离态返回 `stuck: true`。**本计划不改 `nomifun-channel`**（渠道自动重试属第二批，避免与客服域轨道冲突）。

**Tech Stack:** Rust（sqlx/SQLite、axum）、既有 v3 迁移与 schema 契约测试体系。

## Global Constraints

- 构建前 PATH：`export PATH="/c/Users/developer/.cargo/bin:/c/Program Files/CMake/bin:/c/tools/nasm-2.16.03:$PATH"`（Git Bash）。
- 迁移号**固定为 `014_conversation_receipt_error_codes.sql`**（015 已预留给客服域轨道，不得占用）。
- 禁止物理 FOREIGN KEY/级联；receipt 终值不可改写语义不变（accepted→completed 单向吸收）。
- 旧 `result_error` 文本列原样保留并继续写入（兼容 Mac 端已有诊断工具）。
- 不修改 `crates/backend/nomifun-channel/**`（第二批的热区）。
- 每个任务完成即 `git commit`；提交信息用 conventional commits。

---

## File Map

| 文件 | 动作 | 职责 |
|---|---|---|
| `crates/backend/nomifun-db/migrations/014_conversation_receipt_error_codes.sql` | Create | 加两列 + 触发器契约更新 |
| `crates/backend/nomifun-db/src/models/conversation.rs` | Modify | `ConversationDeliveryReceiptRow` 加字段 |
| `crates/backend/nomifun-db/src/repository/conversation.rs` + `sqlite_conversation.rs` | Modify | `TurnReceiptCompletion` 加字段、完成写入 SQL 加列 |
| `crates/backend/nomifun-db/src/id_schema_contract.rs` | Modify | schema 契约测试收录新列 |
| `crates/backend/nomifun-conversation/src/relay_error_code.rs` | Create | `RelayTerminal`→(code, retryable) 纯映射 + 单测 |
| `crates/backend/nomifun-conversation/src/service.rs` | Modify | durable_completion 处调用映射（含 `empty_final_text` 不对称修复）；固定文案失败路径补 code |
| `crates/backend/nomifun-api-types/src/conversation.rs` | Modify | `SendMessageResponse`/`IdempotentMessageDelivery` 加可选字段 |
| `crates/backend/nomifun-gateway/src/caps_conversation.rs` | Modify | 新工具 `nomi_stop_conversation`；status 加 `stuck` 与 `result_error_code` 透出 |

**Interfaces（跨轨道契约，第二批 D1/D2 依赖）：**
- `relay_error_code::map_turn_failure(terminal: &RelayTerminal, final_text: Option<&str>) -> Option<(String, bool)>`——`None` 表示成功；`Some((code, retryable))`。
- code 取值（snake_case 字符串）：`AgentErrorCode` 各变体的 serde 名；另有固定值 `empty_final_text`(false)、`channel_closed`(true)、`turn_cancelled`(false)、`owner_task_exited`(true)、`admission_rejected`(false)、`preparation_failed`(true)。
- wire：`SendMessageResponse.result_error_code?: string`、`result_error_retryable?: boolean`（camelCase 按该文件既有 serde 规则）。
- gateway 工具 `nomi_stop_conversation{conversation_id, confirm:true}` → `{stopped: bool, previous_status: string}`。

---

### Task 1: 迁移 014 + 行模型/契约测试

**Files:** Create `migrations/014_conversation_receipt_error_codes.sql`；Modify `models/conversation.rs`、`id_schema_contract.rs`。

- [ ] **Step 1**：读 `012_conversation_receipt_lifecycle.sql` 的触发器全文，确认终值不可改写触发器是否按列名枚举（若是，触发器需在 014 中 DROP+重建以纳入新列——沿用 012 的写法）。
- [ ] **Step 2**：写迁移（基调）：

```sql
ALTER TABLE conversation_delivery_receipts ADD COLUMN result_error_code TEXT;
ALTER TABLE conversation_delivery_receipts ADD COLUMN result_error_retryable INTEGER;
-- 若 012 触发器按列枚举终值不可变，这里 DROP TRIGGER 并按 012 原文重建、把新列纳入
```

- [ ] **Step 3**：`ConversationDeliveryReceiptRow` 加 `pub result_error_code: Option<String>`、`pub result_error_retryable: Option<bool>`；所有构造/查询点随编译错误逐一补全。
- [ ] **Step 4**：在 `id_schema_contract.rs` 的 receipt 表契约测试中加两列断言（先跑测试看失败，再更新期望）。
- [ ] **Step 5**：`cargo test -p nomifun-db` 全绿 → commit `feat(db): add structured error code columns to delivery receipts`。

### Task 2: 映射函数（TDD）

**Files:** Create `crates/backend/nomifun-conversation/src/relay_error_code.rs`（在 `lib.rs`/`mod` 声明）。

- [ ] **Step 1**：写失败单测（先跑确认编译失败）：

```rust
#[test]
fn finish_with_text_is_success() {
    assert_eq!(map_turn_failure(&RelayTerminal::Finish, Some("ok")), None);
}
#[test]
fn finish_with_empty_text_is_empty_final_text_not_retryable() {
    assert_eq!(
        map_turn_failure(&RelayTerminal::Finish, Some("  ")),
        Some(("empty_final_text".into(), false))
    );
}
#[test]
fn channel_closed_is_retryable() {
    assert_eq!(
        map_turn_failure(&RelayTerminal::ChannelClosed, None),
        Some(("channel_closed".into(), true))
    );
}
#[test]
fn error_uses_agent_error_code_serde_name_and_retryable() {
    let t = RelayTerminal::Error { code: Some(AgentErrorCode::UserLlmProviderRateLimited), retryable: Some(true) };
    assert_eq!(map_turn_failure(&t, None), Some(("user_llm_provider_rate_limited".into(), true)));
}
#[test]
fn error_without_code_defaults_unknown_upstream_and_uses_retryable_flag() {
    let t = RelayTerminal::Error { code: None, retryable: None };
    assert_eq!(map_turn_failure(&t, None), Some(("unknown_upstream_error".into(), false)));
}
```

（serde 名以 `agent_error.rs` 实际 `#[serde(rename_all=…)]` 为准，测试先照抄枚举定义处的规则。）

- [ ] **Step 2**：实现 `map_turn_failure`，只做纯映射不带副作用。`cargo test -p nomifun-conversation relay_error_code` 全绿 → commit。

### Task 3: 终结路径写入 + 固定文案补 code

**Files:** Modify `service.rs`（durable_completion 生成点 :8711-8721 一带；固定文案失败点 :8682/:8740/:8775/:8802/:8973/:7207/:7270/:1233/:7878-7887 等）、repository 完成写入链（`TurnReceiptCompletion` 加字段 → `finalize_exact_turn_operation`/`complete_delivery_receipt_before_release` SQL 加列）。

- [ ] **Step 1**：`TurnReceiptCompletion` 加 `result_error_code: Option<String>`、`result_error_retryable: Option<bool>`；随编译错误把所有构造点补上（固定文案处按 Interfaces 表填对应 code；`empty_final_text` 场景同时保持 `result_ok=false`）。
- [ ] **Step 2**：找到既有的回合终结集成测试（`service_test.rs` 中含 receipt 终值断言者，如 :6329 一带），复制一个用例改造成"空文本 Finish"场景，断言 `result_ok=false && result_error_code=="empty_final_text"`；先跑确认失败，再接通写入使其通过。
- [ ] **Step 3**：`cargo test -p nomifun-conversation` 全绿 → commit `feat(conversation): persist structured error codes on turn receipts`。

### Task 4: wire 透传

**Files:** Modify `nomifun-api-types/src/conversation.rs`（`SendMessageResponse`）、`nomifun-conversation/src/service.rs`（`IdempotentMessageDelivery` :137-148 及其到 response 的组装点）、`routes.rs` 若有独立组装。

- [ ] **Step 1**：两类型各加 `#[serde(skip_serializing_if="Option::is_none")] pub result_error_code: Option<String>` 与 `pub result_error_retryable: Option<bool>`；重放读取路径（`idempotent_delivery_result_*` :6389）同样回填。
- [ ] **Step 2**：跑 `cargo test -p nomifun-api-types -p nomifun-conversation`；若仓库有 wire 契约快照测试则更新。`ui-api-contract-version.txt` 递增一位（单行文件，若与其他轨道冲突以 rebase 取大者）。commit。

### Task 5: gateway——status 透出 + stuck 标记（TDD）

**Files:** Modify `caps_conversation.rs`（`nomi_conversation_status` :228 起）。

- [ ] **Step 1**：status 输出加 `last_result_error_code`（读最近 completed receipt 的新列）与 `stuck: bool`。stuck 判定：会话持久 `running` 且 runtime registry 无该会话 runtime（即 orphan 隔离态）。在该 crate 既有测试风格下写用例：构造持久 running 无 runtime 的会话 → status 返回 `stuck:true` 且附建议文案字段 `stuck_hint`（文案：`该会话因后端重启被保护性挂起，可用 nomi_stop_conversation 解除后重试`）。
- [ ] **Step 2**：实现并全绿 → commit。

### Task 6: gateway——`nomi_stop_conversation`（TDD）

**Files:** Modify `caps_conversation.rs`（注册表 :516-581 一带加第 8 个能力）。

- [ ] **Step 1**：先读桌面 UI"手动停止"的服务入口（从 `routes.rs` 的 `/cancel` 与 service 的 stop/reset 路径追）；工具实现必须复用**同一个** service 方法，不得绕过 tombstone 栅栏自行改库。
- [ ] **Step 2**：注册 `nomi_stop_conversation`：入参 `{conversation_id: string}`，`DangerLevel::Destructive`（Remote/Channel surface 需 `confirm:true`，网关既有门禁自动处理）；出参 `{stopped: bool, previous_status: string}`；对不存在/非本人会话返回既有 NotFound/PermissionDenied 语义。
- [ ] **Step 3**：测试：① 对 running 会话调用 → stopped=true 且会话归 finished；② 对 idle 会话调用 → stopped=false、previous_status="finished"；③ 无 confirm 从 Remote surface 调用被网关拒绝（若既有测试基建可表达）。全绿 → commit `feat(gateway): add nomi_stop_conversation remote unstick tool`。

### Task 7: 全量回归

- [ ] `cargo test -p nomifun-db -p nomifun-conversation -p nomifun-gateway -p nomifun-api-types` 全绿；`cargo check --workspace` 通过（PATH 见全局约束）→ 最终 commit。
