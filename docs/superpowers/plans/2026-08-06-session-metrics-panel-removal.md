# 会话指标看板下线与指标链路瘦身 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 删除 nomi 会话「指标」tab 及其全部陪葬代码，并清除三条零读端指标链路（acp 前端死出口、`AgentExecution.total_tokens` 全链含 DB 列、`TurnCompletedEventData` 三个死字段）。

**Architecture:** 纯删除型重构，按同心圈从 UI 向后端推进：先删面板与入口（Task 1-2），再收窄共享类型（Task 3-4），最后动 Rust 协议与 DB（Task 5-6）。每个 Task 独立可测可提交。规格：`docs/specs/2026-08-06-session-metrics-panel-removal.zh.md`。

**Tech Stack:** React + TypeScript（bun test / typecheck）、Rust（cargo test、ts-rs 12、sqlx + SQLite migration）。

## Global Constraints

- **Git 署名（AGENTS.md，强制）**：作者/提交者必须是人类（本机 git user `muri <2206491416@qq.com>`）；**禁止**任何 AI 署名 trailer（`Co-Authored-By` 等）；**禁止** `--no-verify`。
- **禁止**在 `.github/workflows/` 创建任何 workflow 文件（AGENTS.md）。
- `ui/src/renderer/services/i18n/i18n-keys.d.ts` 是生成物：只能跑 `bun run gen:i18n`，**不得手改**。
- `ui/src/common/protocolBindings/*.ts` 是 ts-rs 生成物：由 `cargo test` 再生成，**不得手改**。
- **不** bump `ui-api-contract-version.txt`（构建一致性闩，UI 与后端同发）。
- Commit message 风格照仓库历史：`type(scope): subject`，英文。
- 所有命令在仓库根 `C:/Users/MINISFORUM/code/nomifun/multi/1` 执行。

---

### Task 1: 删除指标面板、tab 入口与 i18n（G1a）

**Files:**
- Delete: `ui/src/renderer/pages/conversation/platforms/nomi/NomiSessionMetricsPanel.tsx`
- Delete: `ui/src/renderer/pages/conversation/platforms/nomi/nomiSessionMetricsPanel.test.ts`
- Modify: `ui/src/renderer/pages/conversation/components/ChatConversation.tsx:15,50,205-221`
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/conversation.json:479-509`
- Modify: `ui/src/renderer/services/i18n/locales/en-US/conversation.json:479-509`
- Regenerate: `ui/src/renderer/services/i18n/i18n-keys.d.ts`

**Interfaces:**
- Consumes: 无
- Produces: `ChatConversation.tsx` 的 `workspaceExtraTabs` 只剩 `conversation-terminals` 一项；`conversation.sessionMetrics.*` key 全部消失。后续 Task 不依赖本 Task 的产物。

- [ ] **Step 1: 删两个文件**

```bash
git rm ui/src/renderer/pages/conversation/platforms/nomi/NomiSessionMetricsPanel.tsx \
       ui/src/renderer/pages/conversation/platforms/nomi/nomiSessionMetricsPanel.test.ts
```

- [ ] **Step 2: 改 `ChatConversation.tsx` 三处**

`:15` 的 import 去掉 `ChartHistogram`（本文件仅指标 tab 用它）：

```ts
// 旧
import { ChartHistogram, History, Terminal } from '@icon-park/react';
// 新
import { History, Terminal } from '@icon-park/react';
```

删除 `:50` 整行：

```ts
import NomiSessionMetricsPanel from '../platforms/nomi/NomiSessionMetricsPanel';
```

删除 `:205-221` useMemo 内数组的第二个元素（保留 terminal 项与 useMemo 本身，依赖数组 `[conversation, t]` 不变——terminal 项仍用 `conversation.id`）：

```ts
      {
        key: 'nomi-session-metrics',
        title: t('conversation.sessionMetrics.tab'),
        icon: <ChartHistogram size={18} />,
        content: <NomiSessionMetricsPanel conversation={conversation} />,
      },
```

- [ ] **Step 3: 删两个 locale 的 `sessionMetrics` 整块**

zh-CN 与 en-US 的 `conversation.json` 各删 `:479-509`——从 `"sessionMetrics": {` 到配对的 `},`（含尾逗号）。块的前一项是 `contextUsage`（保留），后一项是 `skill_generator`（保留）。删完确认 JSON 合法：

```bash
bun -e "JSON.parse(require('fs').readFileSync('ui/src/renderer/services/i18n/locales/zh-CN/conversation.json','utf8')); JSON.parse(require('fs').readFileSync('ui/src/renderer/services/i18n/locales/en-US/conversation.json','utf8')); console.log('json ok')"
```

- [ ] **Step 4: 再生成 i18n key 类型并校验**

```bash
bun run gen:i18n && bun run check:i18n
```

Expected: PASS；`i18n-keys.d.ts` 中 27 个 `conversation.sessionMetrics.*` 条目消失（原 `:927-953`）。

- [ ] **Step 5: typecheck + UI 测试**

```bash
bun run typecheck && bun test --cwd ui
```

Expected: 全部 PASS。若 typecheck 报出其他 `sessionMetrics` 或 `NomiSessionMetricsPanel` 残留引用，回到 Step 2/3 处理该处（spec 调查结论：仅上述位置引用）。

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "refactor(conversation): remove nomi session metrics tab"
```

---

### Task 2: 瘦身 `turnMetrics` 并删除零监听者事件（G1b）

**Files:**
- Modify: `ui/src/renderer/pages/conversation/platforms/nomi/turnMetrics.ts`（97→约 18 行）
- Modify: `ui/src/renderer/pages/conversation/platforms/nomi/turnMetrics.test.ts`（112→约 29 行）
- Modify: `ui/src/renderer/utils/emitter.ts:13,27`
- Modify: `ui/src/renderer/pages/conversation/platforms/nomi/useNomiMessage.ts:22,449`

**Interfaces:**
- Consumes: 无
- Produces: `turnMetrics.ts` 仅导出 `formatTokenCount(tokens: number): string`（`ContextUsageRing.tsx:9` 与 `NomiSendBoxLayout.structure.test.ts` 依赖此导出名，不得改名）。`EventTypes` 中不再有 `'nomi.usage.updated'`。

- [ ] **Step 1: 先收窄测试（删除型 TDD——测试先行定义存活面）**

`turnMetrics.test.ts` 整文件替换为：

```ts
import { describe, expect, test } from 'bun:test';

import { formatTokenCount } from './turnMetrics';

describe('formatTokenCount', () => {
  test('renders small counts verbatim', () => {
    expect(formatTokenCount(0)).toBe('0');
    expect(formatTokenCount(42)).toBe('42');
    expect(formatTokenCount(999)).toBe('999');
  });

  test('renders thousands with a k suffix and one decimal', () => {
    expect(formatTokenCount(1000)).toBe('1.0k');
    expect(formatTokenCount(1234)).toBe('1.2k');
    expect(formatTokenCount(12_500)).toBe('12.5k');
  });

  test('renders millions with an m suffix', () => {
    expect(formatTokenCount(1_000_000)).toBe('1.0m');
    expect(formatTokenCount(2_300_000)).toBe('2.3m');
  });
});
```

- [ ] **Step 2: 整文件替换 `turnMetrics.ts`**（头注释同步改写——旧注释描述的 per-turn chip 已在上一轮瘦身中删除）：

```ts
/**
 * Pure formatter for the context-usage ring beside the model selector.
 * Kept separate from React so the formatting rule is unit-testable.
 */

/**
 * Compact token count: `942`, `1.2k`, `2.3m`. One decimal place at each
 * magnitude so the ring stays narrow while still conveying scale.
 */
export function formatTokenCount(tokens: number): string {
  if (tokens < 1000) {
    return String(tokens);
  }
  if (tokens < 1_000_000) {
    return `${(tokens / 1000).toFixed(1)}k`;
  }
  return `${(tokens / 1_000_000).toFixed(1)}m`;
}
```

- [ ] **Step 3: 删 emitter 事件类型**

`emitter.ts:27` 删除：

```ts
  'nomi.usage.updated': [{ conversation_id: ConversationId; tokenUsage: TokenUsageData }];
```

`emitter.ts:13` 的 `import type { TokenUsageData } from '@/common/config/storage';` 仅服务此行，一并删除。（`ConversationId` 仍被其他事件用，保留。）

- [ ] **Step 4: 删 emit 调用**

`useNomiMessage.ts:449` 删除：

```ts
                emitter.emit('nomi.usage.updated', { conversation_id, tokenUsage: newTokenUsage });
```

`useNomiMessage.ts:22` 的 `import { emitter } from '@/renderer/utils/emitter';` 在本文件仅此一用（已核实），一并删除。**保留** `:450-456` 的 `ipcBridge.conversation.update.invoke`（`last_token_usage` 持久化是圆环回填来源）。

- [ ] **Step 5: 运行测试**

```bash
bun test --cwd ui src/renderer/pages/conversation/platforms/nomi/turnMetrics.test.ts && bun run typecheck && bun test --cwd ui
```

Expected: 全部 PASS（`NomiSendBoxLayout.structure.test.ts:41-42` 断言 `ContextUsageRing` 仍用 `formatTokenCount` —— 未动，应过）。

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "refactor(nomi): drop dead turn-metric helpers and usage event"
```

---

### Task 3: 收窄 `TokenUsageData`（G2）

**Files:**
- Modify: `ui/src/common/config/storage.ts:97-114`
- Modify: `ui/src/renderer/pages/conversation/platforms/nomi/useNomiMessage.ts:423-446`

**Interfaces:**
- Consumes: 无
- Produces: `TokenUsageData = { total_tokens: number; context_tokens?: number; context_window?: number }`。Task 4 删除 acp 侧对旧字段的最后引用前，本 Task 已保证 acp 现存代码只用 `total_tokens`（`useAcpMessage.ts:671,931`），不会 break。

- [ ] **Step 1: 收窄类型**

`storage.ts:97-114` 替换为：

```ts
// Token 使用统计数据类型
export interface TokenUsageData {
  total_tokens: number;
  /** Current context occupancy (gauge numerator). */
  context_tokens?: number;
  /** Effective context budget (gauge denominator). */
  context_window?: number;
}
```

（删除 `input_tokens`、`output_tokens`、`cache_creation_tokens`、`cache_read_tokens`、`elapsed_ms` 五个字段。已持久化的旧 `last_token_usage` JSON 里多余键运行时被忽略，无需迁移。）

- [ ] **Step 2: 同步收窄 `useNomiMessage.ts` 的事件解析与构造**

`:423-446` 的解析类型与构造替换为（`input/output_tokens` 仍从事件读——用于算 `total_tokens`；`elapsed_ms`/`cache_*` 不再读）：

```ts
            const metrics = message.data as
              | {
                  input_tokens?: number;
                  output_tokens?: number;
                  context_tokens?: number;
                  context_window?: number;
                }
              | undefined;
            if (metrics && typeof metrics === 'object') {
              const inputTokens = metrics.input_tokens || 0;
              const outputTokens = metrics.output_tokens || 0;
              const newTokenUsage: TokenUsageData = {
                total_tokens: inputTokens + outputTokens,
                context_tokens: metrics.context_tokens,
                context_window: metrics.context_window,
              };
```

（其后的 `setTokenUsage(newTokenUsage);` 与持久化块不变。）

- [ ] **Step 3: 验证**

```bash
bun run typecheck && bun test --cwd ui
```

Expected: PASS。若 typecheck 报出其他文件引用被删字段：`useAcpMessage.ts` 只该用 `total_tokens`（`:671,931`），其余报错处按报错逐一确认是死引用后删除。

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "refactor(chat): narrow TokenUsageData to consumed fields"
```

---

### Task 4: 删除 acp 前端死指标出口（G3）

**Files:**
- Modify: `ui/src/renderer/pages/conversation/platforms/acp/useAcpMessage.ts:19,166-167,186-187,668-677,872-874,928-937,1086-1087`
- Modify: `ui/src/common/config/storage.ts:145-148`
- Modify: `ui/src/common/chat/chatLib.ts:1472`（仅注释）

**Interfaces:**
- Consumes: Task 3 产出的收窄版 `TokenUsageData`（本 Task 删掉 acp 对它的 import）。
- Produces: `UseAcpMessageReturn` 不再含 `tokenUsage`/`context_limit`（已核实零消费者：`AcpChat.tsx:75-93` 与 `AcpSendBox.tsx:115-128` 的解构清单均不含）。

- [ ] **Step 1: 删 `useAcpMessage.ts` 七处**

1. `:19` 删 `import type { TokenUsageData } from '@/common/config/storage';`
2. `:166-167` 删 return 类型两行：

```ts
  tokenUsage: TokenUsageData | null;
  context_limit: number;
```

3. `:186-187` 删两个 state：

```ts
  const [tokenUsage, setTokenUsage] = useState<TokenUsageData | null>(null);
  const [context_limit, setContextLimit] = useState<number>(0);
```

4. `:668-677` 的 `case 'acp_context_usage'` 整块替换为透传注释（事件仍会到达，必须保留 case 防止落入 default 的 console.warn；`:72` 的 `ACP_THINKING_NON_BOUNDARY_TYPES` 成员**不动**）：

```ts
        case 'acp_context_usage':
          // Known engine event with no UI consumer; swallowed so it neither
          // breaks thinking segmentation (see ACP_THINKING_NON_BOUNDARY_TYPES)
          // nor hits the unsupported-type warning below.
          break;
```

5. `:872-874`（reset effect 内）删两行：

```ts
    setTokenUsage(null);
    setContextLimit(0);
```

6. `:928-937` 删整个回填块（`last_context_limit` 全仓库零写入方，`last_token_usage` 仅 nomi 前端写，此 if 对 acp 永不为真）：

```ts
        // Restore persisted context usage data
        if (res.type === 'acp' && res.extra?.last_token_usage) {
          const { last_token_usage, last_context_limit } = res.extra;
          if (last_token_usage.total_tokens > 0) {
            setTokenUsage(last_token_usage);
          }
          if (last_context_limit && last_context_limit > 0) {
            setContextLimit(last_context_limit);
          }
        }
```

7. `:1086-1087` 删 return 对象两行：

```ts
    tokenUsage,
    context_limit,
```

- [ ] **Step 2: 删 `storage.ts:145-148` acp extra 的两个键**

```ts
          /** Cumulative token usage reported by the ACP `usage_update` notification. */
          last_token_usage?: TokenUsageData;
          /** Context window size reported by the ACP `usage_update` notification. */
          last_context_limit?: number;
```

（以实际 `:145-148` 内容为准整块删除。**`storage.ts:265` 附近 nomi 变体的 `last_token_usage` 是另一处独立声明，保留。**）

- [ ] **Step 3: 修正 `chatLib.ts:1472` 的过时注释**（case 保留）

```ts
// 旧
    case 'acp_context_usage': // Context usage updates, handled by AcpSendBox
// 新
    case 'acp_context_usage': // Known engine event; no UI consumer, swallowed
```

- [ ] **Step 4: 验证**

```bash
bun run typecheck && bun test --cwd ui
```

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "refactor(acp): remove dead token-usage plumbing"
```

---

### Task 5: `TurnCompletedEventData` 删三个死字段（G5，Rust）

**Files:**
- Modify: `crates/backend/nomifun-ai-agent/src/protocol/events/mod.rs:120-150`（struct）、`:2506-2541`（roundtrip 测试）
- Modify: `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs:1247-1256`
- Regenerate: `ui/src/common/protocolBindings/TurnCompletedEventData.ts`

**Interfaces:**
- Consumes: 无（前端解析类型已在 Task 3 收窄，不再读被删字段）。
- Produces: `TurnCompletedEventData { elapsed_ms: i64, input_tokens: u64, output_tokens: u64, context_tokens: u64, context_window: u64 }`。`stream_relay.rs:6951,6957,6986` 的测试构造用 `..Default::default()`，无需改。`map_engine_stop_reason` 与 `agent.rs:1206,1317` 的 `stop_reason` 变量保留（`Finish` 事件消费）；`turn_completed_mapping_tests`（`agent.rs:4614`）不动。

- [ ] **Step 1: 先改 roundtrip 测试（红）**

`events/mod.rs:2506-2541` 的 `turn_completed_event_roundtrip_and_backcompat` 替换为：

```rust
    #[test]
    fn turn_completed_event_roundtrip_and_backcompat() {
        // Serializes under the snake_case wire tag with all metric fields.
        let event = AgentStreamEvent::TurnCompleted(TurnCompletedEventData {
            elapsed_ms: 1234,
            input_tokens: 500,
            output_tokens: 250,
            context_tokens: 8000,
            context_window: 100_000,
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "turn_completed");
        assert_eq!(json["data"]["elapsed_ms"], 1234);
        assert_eq!(json["data"]["input_tokens"], 500);
        assert_eq!(json["data"]["output_tokens"], 250);
        assert_eq!(json["data"]["context_tokens"], 8000);
        assert_eq!(json["data"]["context_window"], 100_000);

        // Back-compat: an old payload with extra retired fields and no context
        // fields deserializes cleanly (unknown keys ignored, defaults applied).
        let old = serde_json::json!({
            "type": "turn_completed",
            "data": {
                "elapsed_ms": 1, "input_tokens": 2, "output_tokens": 3,
                "cache_read_tokens": 4, "stop_reason": "end_turn"
            }
        });
        let back: AgentStreamEvent = serde_json::from_value(old).unwrap();
        assert!(matches!(
            back,
            AgentStreamEvent::TurnCompleted(d)
                if d.context_tokens == 0 && d.context_window == 0
        ));
    }
```

- [ ] **Step 2: 跑该测试确认失败**

```bash
cargo test -p nomifun-ai-agent turn_completed_event_roundtrip
```

Expected: FAIL（编译错误——构造缺字段）。

- [ ] **Step 3: 删 struct 字段**

`events/mod.rs:123-150` 的 struct 改为：

```rust
pub struct TurnCompletedEventData {
    /// Wall-clock duration of the turn in milliseconds.
    #[ts(type = "number")]
    pub elapsed_ms: i64,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    /// Current context occupancy (last request's prompt tokens). Gauge numerator.
    #[serde(default)]
    #[ts(type = "number")]
    pub context_tokens: u64,
    /// Effective context budget (engine compaction window). Gauge denominator.
    #[serde(default)]
    #[ts(type = "number")]
    pub context_window: u64,
}
```

（derive 行与 `#[ts(export_to = ...)]` 不动。`TurnStopReason` 类型仍被 `FinishEventData` 使用，import 保留。）

- [ ] **Step 4: 改构造处**

`agent.rs:1247-1256` 改为（`stop_reason` 局部变量保留——`:1317` 的 `emit_finish_for_turn` 仍消费）：

```rust
                self.runtime.emit(AgentStreamEvent::TurnCompleted(TurnCompletedEventData {
                    elapsed_ms,
                    input_tokens: agent_result.usage.input_tokens,
                    output_tokens: agent_result.usage.output_tokens,
                    context_tokens,
                    context_window,
                }));
```

- [ ] **Step 5: 跑测试（绿）并再生成 binding**

```bash
cargo test -p nomifun-ai-agent && cargo test -p nomifun-conversation turn_completed
```

Expected: 全部 PASS（`nomifun-ai-agent` 的 export_bindings 测试重写 `TurnCompletedEventData.ts`）。然后确认：

```bash
git diff --stat ui/src/common/protocolBindings/ && bun run typecheck
```

Expected: `TurnCompletedEventData.ts` 有 diff（三字段与 `TurnStopReason` import 消失）；typecheck PASS。若 binding 未变化，运行 `cargo test -p nomifun-ai-agent export_bindings`。

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "refactor(protocol): drop unread TurnCompleted fields"
```

---

### Task 6: 删 `AgentExecution.total_tokens` 全链 + migration 024（G4）

**Files:**
- Create: `crates/backend/nomifun-db/migrations/024_drop_agent_execution_total_tokens.sql`
- Modify: `crates/backend/nomifun-agent-execution/src/scheduler.rs:2019-2036`
- Modify: `crates/backend/nomifun-agent-execution/src/domain_mapper.rs:102`
- Modify: `crates/backend/nomifun-api-types/src/agent_execution.rs:293,783`
- Modify: `crates/backend/nomifun-db/src/models/agent_execution.rs:18`
- Modify: `crates/backend/nomifun-db/src/repository/agent_execution.rs:188`
- Modify: `crates/backend/nomifun-db/src/repository/sqlite_agent_execution.rs:1919,1936-1937`
- Modify: `ui/src/common/types/agentExecution/agentExecutionTypes.ts:97`

**Interfaces:**
- Consumes: 无
- Produces: `AgentExecution`/`AgentExecutionRow`/`UpdateAgentExecutionParams`/`TAgentExecution` 均无 `total_tokens`。**`ExecutionAttempt.tokens` 及其整条链（`attempt_runner.rs`→`scheduler.rs:1720,1776`→`StepNode.tsx` ⚡ tokens）不动。**

- [ ] **Step 1: 新建 migration**

`crates/backend/nomifun-db/migrations/024_drop_agent_execution_total_tokens.sql`：

```sql
-- agent_executions.total_tokens has been write-only since the v3 baseline:
-- the scheduler summed per-attempt tokens into it at terminal states, but no
-- Rust logic, SQL clause, or UI code ever read the value back (the DAG canvas
-- renders per-attempt tokens instead). Drop the storage. The column-level
-- CHECK travels with the column; no index or table-level constraint
-- references it, so SQLite DROP COLUMN is legal here.
ALTER TABLE agent_executions DROP COLUMN total_tokens;
```

- [ ] **Step 2: 删 Rust 链（六处）**

1. `scheduler.rs:2019-2024` 删聚合计算：

```rust
        let token_values: Vec<i64> = detail
            .attempts
            .iter()
            .filter_map(|attempt| attempt.tokens)
            .collect();
        let total_tokens = (!token_values.is_empty()).then(|| token_values.into_iter().sum());
```

2. `scheduler.rs:2036` 删参数行 `total_tokens: Some(total_tokens),`（`UpdateAgentExecutionParams` 构造内，`..Default::default()` 已在，无需补）。
3. `domain_mapper.rs:102` 删 `total_tokens: row.total_tokens,`。
4. `api-types/agent_execution.rs:293` 删 `pub total_tokens: Option<i64>,`；`:783` fixture 删 `"total_tokens": null,`。再全 crate 扫残留：

```bash
grep -rn "total_tokens" crates/backend/nomifun-api-types/
```

Expected: 零命中；有则逐处删除。

5. `db/models/agent_execution.rs:18` 删 `pub total_tokens: Option<i64>,`（`AgentExecutionRow`；migration 删列后 `SELECT *` 行结构与 struct 继续匹配）。
6. `repository/agent_execution.rs:188` 删 `pub total_tokens: Option<Option<i64>>,`；`sqlite_agent_execution.rs` 删 UPDATE 中 `:1919` 的 `total_tokens = CASE WHEN ? THEN ? ELSE total_tokens END, \` 一行，及 `:1936-1937` 两个对应 bind（**bind 顺序与 SQL 占位符一一对应，必须同时删这三行**）：

```rust
        .bind(params.total_tokens.is_some())
        .bind(params.total_tokens.as_ref().and_then(|value| *value))
```

- [ ] **Step 3: 编译并全扫残留**

```bash
cargo check -p nomifun-db -p nomifun-agent-execution -p nomifun-api-types && grep -rn "total_tokens" crates/backend/nomifun-db/src crates/backend/nomifun-agent-execution crates/backend/nomifun-api-types
```

Expected: check PASS；grep 除 migration 文件与 `001_v3_baseline.sql`（历史 DDL，不改）外零命中。

- [ ] **Step 4: 跑三个 crate 测试（覆盖 migration 024 对全部既有 DB 测试的兼容）**

```bash
cargo test -p nomifun-db -p nomifun-agent-execution -p nomifun-api-types
```

Expected: 全部 PASS。

- [ ] **Step 5: 删 TS 字段并验证**

`agentExecutionTypes.ts:97` 删 `total_tokens: number | null;`，然后：

```bash
bun run typecheck && bun test --cwd ui
```

Expected: PASS（`executionCanvasIntegration.structure.test.ts:48` 的 per-attempt tokens 断言不受影响）。

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "refactor(agent-execution): drop write-only execution total_tokens"
```

---

### Task 7: 全量验证与手工冒烟

**Files:** 无新改动（只验证）。

- [ ] **Step 1: 全量静态检查与 UI 测试**

```bash
bun run check && bun test --cwd ui
```

Expected: 全部 PASS。

- [ ] **Step 2: 全量 Rust 测试（含 ts-rs binding 一致性与全部 migration）**

```bash
bun run test
```

Expected: 全部 PASS。跑完确认 `git status` 无未预期的 binding diff。

- [ ] **Step 3: 仓库规则复查**

```bash
ls .github/workflows/ 2>/dev/null; git log --format='%an <%ae> %s' origin/main..HEAD
```

Expected: workflows 目录不存在或为空；所有提交作者为 `muri <2206491416@qq.com>`，subject 无 AI 署名 trailer。

- [ ] **Step 4: 手工冒烟（需人工确认）**

1. `bun run dev:web` 启动。
2. 新建 nomi 会话完成一轮 → 右侧工具栏只剩 文件/变更/终端 三个 tab。
3. 发送框圆环随 turn 完成更新百分比；点击弹出上下文详情。
4. 离开再重进该会话 → 圆环仍显示（`last_token_usage` 回填）。
5. 跑一个带执行计划的会话 → DAG 画布 step 节点 hover 仍显示「⚡ N tokens」。

- [ ] **Step 5: 汇报结果**

冒烟通过后，向用户汇报删除统计（`git diff --stat main@{起点}..HEAD`）与任何偏差。
