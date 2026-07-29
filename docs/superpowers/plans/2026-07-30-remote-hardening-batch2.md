# 远控硬化第二批（D1 忙时排队 + D2 结果回推）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 渠道忙时消息持久排队 + `nomi_send_to_conversation` 结果回推（spec §设计 D 的 D1/D2），并补齐第一批移交的两个渠道侧缺口。

**第一批移交的缺口（本批必须处理）：**
1. **Channel surface 上 `nomi_stop_conversation` 被网关矩阵 Deny**（`(Channel, Destructive)=Deny` 无 allow-with-confirm 覆盖）：微信里无法远程解锁。本批在渠道层打通——利用渠道既有数字决策流程（message_loop 的 pending decision 机制）做确认：伙伴调 stop 被 Deny 时，渠道回"确认停止会话 X？1. 确认 2. 取消"，用户回 1 后由渠道以 owner 身份直接调 `ConversationService::cancel`（不经网关矩阵；渠道层本就持 conversation_svc）。
2. **`abandon_exact_turn_admission`（service.rs 原 :1233 一带 "Public turn request was dropped…" 路径）未写结构化 code**：本批补 `admission_abandoned`/retryable=true（其 repo SQL 需加两列写入，模式照抄第一批 `complete_delivery_receipt` 的扩参做法）。

**Architecture:** D1 在渠道层：新表 `channel_pending_prompts`，busy 时入队并回"已排队（第 N 位）"，turn 完成事件驱动同会话 FIFO 出队投递，retryable 失败有限重试。D2 在会话/网关层：`notify_back` 登记表 `conversation_delivery_notify`，receipt 终结钩子向发起伙伴会话投递 observed background 回执，经渠道既有 stream_relay 回传 IM；origin 标记防环。

**Tech Stack:** Rust（nomifun-db 迁移、nomifun-channel、nomifun-conversation、nomifun-gateway）。

## Global Constraints

- **前置**：D 第一批（feat/remote-hardening-batch1，提供 `result_error_retryable`）与 C1（feat/customer-service-c1，重构了 `message_service.rs` 路由头部并占用迁移 015）都已合并 main。开工先通读两者对这两个文件的实际改动。
- 迁移号**固定 `016_channel_pending_prompts_and_notify.sql`**。
- 构建前 PATH：`export PATH="/c/Users/MINISFORUM/.cargo/bin:/c/Program Files/CMake/bin:/c/tools/nasm-2.16.03:$PATH"`。
- 参数（spec 定死）：每 chat 队列上限 10；过期 30 分钟；渠道自动重试最多 2 次、退避 30s/120s；receipt 表不加新列（notify 登记独立成表，identity-immutable 触发器不动）。
- 客服绑定 bot 的消息**不进队列**（C1 接缝在队列判断之前短路）。
- 每任务一 commit（conventional commits + Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>）。

---

## File Map

| 文件 | 动作 | 职责 |
|---|---|---|
| `crates/backend/nomifun-db/migrations/016_channel_pending_prompts_and_notify.sql` | Create | 两张新表 |
| `crates/backend/nomifun-db/src/models/channel.rs` + `repository/{channel.rs,sqlite_channel.rs}` | Modify | pending prompts 行模型与仓储（入队/按会话取队头/结算/过期清理/计数） |
| `crates/backend/nomifun-db/src/models/conversation.rs` + conversation 仓储 | Modify | notify 登记行模型与仓储（登记/按 operation 取/结算） |
| `crates/backend/nomifun-db/src/id_schema_contract.rs` | Modify | 契约收录 |
| `crates/backend/nomifun-channel/src/message_loop.rs` | Modify | busy 分支改入队+第 N 位提示；「取消排队」指令 |
| `crates/backend/nomifun-channel/src/queue_drain.rs` | Create | 出队器：订阅 turn 完成 → FIFO 投递 + retryable 重试 + 过期通知 |
| `crates/backend/nomifun-conversation/src/service.rs` | Modify | receipt 终结处触发 notify 钩子（trait 注入，避免 conversation 依赖 gateway/companion） |
| `crates/backend/nomifun-gateway/src/caps_conversation.rs` | Modify | `nomi_send_to_conversation` 加 `notify_back?: bool`；登记 |
| `crates/backend/nomifun-app/src/services.rs` | Modify | 装配 notify 钩子实现与 queue_drain 启动 |

**Interfaces：**

```sql
CREATE TABLE channel_pending_prompts ( id INTEGER PRIMARY KEY AUTOINCREMENT,
  prompt_id TEXT NOT NULL UNIQUE CHECK (…v7 GLOB…),
  channel_plugin_id TEXT NOT NULL, chat_id TEXT NOT NULL, channel_session_id TEXT NOT NULL,
  conversation_id TEXT NOT NULL, text TEXT NOT NULL, idempotency_key TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'queued' CHECK (state IN ('queued','delivered','expired','cancelled','failed')),
  attempts INTEGER NOT NULL DEFAULT 0, queued_at INTEGER NOT NULL, settled_at INTEGER );
CREATE INDEX idx_cpp_conversation_state ON channel_pending_prompts(conversation_id, state, id);
CREATE TABLE conversation_delivery_notify ( id INTEGER PRIMARY KEY AUTOINCREMENT,
  operation_id TEXT NOT NULL UNIQUE, requester_conversation_id TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','notified','failed')),
  created_at INTEGER NOT NULL, settled_at INTEGER );
```

```rust
// conversation 侧钩子（nomifun-conversation 定义，nomifun-app 实现注入）
#[async_trait] pub trait TurnCompletionObserver: Send + Sync {
    async fn on_turn_completed(&self, conversation_id: &str, operation_id: &str,
        result_ok: bool, result_text: Option<&str>, result_error_code: Option<&str>);
}
```

- `nomi_send_to_conversation` 新入参 `notify_back?: bool`（默认 false）。
- 回执消息 origin 固定 `"delivery-notify"`；service 对 origin=="delivery-notify" 的 turn 禁用 notify 登记（防环硬约束）。
- 排队回复文案（i18n 不适用，渠道纯文本）：`⏳ 会话正忙，已排队（第 {n} 位），完成后自动处理。回复「取消排队」可清空。`

---

### Task 1: 迁移 016 + 两张表仓储（TDD）

- [ ] Step 1：写迁移（上方 SQL，GLOB 照抄基线）；契约测试收录。
- [ ] Step 2：channel 仓储：`enqueue_pending_prompt`（同 conversation queued 计数 ≥10 返回 QueueFull 错误）、`peek_next_queued(conversation_id)`、`settle_prompt(prompt_id, state)`、`expire_stale(before_ms) -> Vec<行>`、`cancel_chat_queue(plugin, chat)`。conversation 仓储：`register_notify`/`take_pending_notify(operation_id)`。内存库单测覆盖各态迁移与上限。commit。

### Task 2: 入队（message_loop）

- [ ] Step 1：busy 两分支（per-chat 守卫 :527-538 与 ConversationBusy :560-566，以合并后实际代码为准）改为：入队成功 → 回"已排队（第 N 位）"提示；QueueFull → 回"排队已满，请稍后再发"。客服绑定 bot 不受影响（接缝在其之前）。
- [ ] Step 2：文本命令「取消排队」→ `cancel_chat_queue` + 回执数量。集成测试：busy 时消息入队不丢、第 N 位计数正确、取消清空。commit。

### Task 3: 出队器（queue_drain）

- [ ] Step 1：`QueueDrain::run` 后台任务：订阅 turn 完成信号（探索现状：优先复用进程内事件总线/realtime 广播的后端订阅面；找不到则 5s 轮询 `peek_next_queued` 兜底，选型写进代码注释与 commit message）→ 对该 conversation 取队头 → `send_to_agent` 全链路投递（新 idempotency key 用存量的）→ 成功 settle delivered；失败且 `result_error_retryable`（D 批1 字段）→ attempts+1 重试（30s/120s，≤2 次）→ 仍败 settle failed 并回真实错误文案给该 chat。
- [ ] Step 2：启动时恢复：queued 且超 30 分钟 → expired 并给 chat 发放弃通知；其余照常等待。集成测试：turn 完成后队头自动投递、FIFO 顺序、retryable 重试封顶、过期通知。commit。

### Task 4: notify_back 登记与回推

- [ ] Step 1：conversation 定义 `TurnCompletionObserver`（Interfaces）并在 `release_and_complete_turn` 终结成功后（receipt completed 持久化之后）异步调用（spawn，绝不阻塞终结事务）；service 构造加 `Option<Arc<dyn TurnCompletionObserver>>`。
- [ ] Step 2：gateway `nomi_send_to_conversation` 加 `notify_back`：true 且 caller 有 operation_id 时 `register_notify(operation_id, caller 的 companion 会话 id)`；caller 非会话上下文（无 requester conversation）时忽略并在返回注明。origin=="delivery-notify" 的 turn 中强制忽略 notify_back。
- [ ] Step 3：nomifun-app 实现 observer：`take_pending_notify(operation_id)` 命中 → 组装回执文本（成功：结果摘要 result_text 截 1500 字符；失败：error code+text）→ `send_observed_background_message_with_idempotency_key`（幂等 key = `"delivery-notify:" + operation_id`，origin="delivery-notify"）投递给 requester 会话。伙伴会话若绑渠道，stream_relay 既有链路自动回传 IM——无需新码。
- [ ] Step 4：集成测试：notify_back 下发→目标完成→伙伴会话收到回执消息（幂等：重复终结不重复投递）；防环：回执 turn 内再调 send 带 notify_back 不登记。commit。

### Task 5: 回归

- [ ] `cargo test -p nomifun-db -p nomifun-channel -p nomifun-conversation -p nomifun-gateway -p nomifun-app` + `cargo check --workspace` 全绿 → commit。
