# 会话指标看板下线与指标链路瘦身设计（2026-08-06）

## 1. 背景与动机

nomi 会话右侧工具栏有一个「指标」tab（`NomiSessionMetricsPanel`），展示本次响应耗时、
会话跨度、上下文占用分段条、token 用量四宫格、缓存明细。用户调研结论：**没有人看这些
数字并据此改变操作**；面板顶部甚至挂着一条自我否定的告示（"因数据采集手段问题，数据
仅供参考，不可作为定论"）。唯一有行为价值的信号——上下文占用——已经由发送框旁的
`ContextUsageRing` 圆环承载（百分比、精确 token、70%/90% 变色、点击看详情）。

本仓库此前已做过一轮同类瘦身：`NomiSendBoxLayout.structure.test.ts:31-33` 断言发送框
**不允许**出现 per-turn 耗时与 total_tokens 文案。指标看板等于同一批数字换个位置复活。

调查同时确认了两条相邻的历史债务（详见 §3 G4/G5 与 §4）：

- `AgentExecution.total_tokens` 整条链「计算→落库→序列化」**零读端**，唯一读者是
  `serde::Serialize`；
- acp 侧的指标前端出口全部是死代码：hook 导出的 `tokenUsage`/`context_limit` 零消费者，
  回填分支读取的 `extra.last_token_usage`/`last_context_limit` 对 acp 会话**永远没有写入方**。

所有被删指标的获取成本本身为零（都来自 provider 响应或纯字段读取，无采样线程/聚合
查询），因此本次瘦身的收益全部在**维护面**：约 600 行 UI + 约 35 行 Rust + 一个 DB 列
+ 27×2 个 i18n key。

## 2. 决策

1. **整个「指标」tab 删除**，不保留任何精简版。上下文占用继续由 `ContextUsageRing` 承载。
2. 删除面板陪葬的所有孤立代码（G1）。
3. 收窄 `TokenUsageData` 到仍被消费的字段（G2）。
4. 删除 acp 前端的死指标出口（G3）；**acp 后端采集链保留**（真实实现、有单测，是唯一
   的采集能力；能否收到数据取决于外部 agent 是否实现 unstable `usage_update` 通知）。
5. 删除 `AgentExecution.total_tokens` 全链，**含 DB 列**（G4，migration 024）。
6. `TurnCompletedEventData` 删除 `cache_creation_tokens`/`cache_read_tokens`/`stop_reason`
   三个零读端字段（G5）。**不 bump** `ui-api-contract-version.txt`：它是 UI/后端同发的
   构建一致性闩（`ui_build_manifest.rs:78-88` 校验 manifest 与源文件一致），非运行时协商。

## 3. 删除范围

### G1 nomi 指标看板本体及陪葬代码（~520 行，纯 UI）

| 动作 | 位置 | 说明 |
|---|---|---|
| 删文件 | `ui/src/renderer/pages/conversation/platforms/nomi/NomiSessionMetricsPanel.tsx` | 285 行 |
| 删文件 | `.../nomi/nomiSessionMetricsPanel.test.ts` | 面板专属测试 |
| 改 | `ui/src/renderer/pages/conversation/components/ChatConversation.tsx` | 删 `:50` import、`:213-218` tab 条目；`:15` import 去掉 `ChartHistogram`（该文件仅此一用） |
| 改 | `ui/src/renderer/services/i18n/locales/zh-CN/conversation.json`、`en-US/conversation.json` | 各删 `sessionMetrics` 整块（27 key） |
| 重新生成 | `ui/src/renderer/services/i18n/i18n-keys.d.ts` | 跑 `bun run gen:i18n`，**不得手改** |
| 改 | `.../nomi/turnMetrics.ts` | 97→约 20 行：仅保留 `formatTokenCount`（`ContextUsageRing` 在用）；删 `formatTurnDuration`、`calculateContextUsagePercent`、`calculateCacheHitRatePercent`、`calculateContextUsageSegments`、`formatPercent`、`ContextUsageSegments` 类型；文件头注释改写（现在描述的 per-turn chip 早已删除） |
| 改 | `.../nomi/turnMetrics.test.ts` | 112→约 20 行，仅留 `formatTokenCount` 用例 |
| 改 | `ui/src/renderer/utils/emitter.ts:27` | 删 `nomi.usage.updated` 事件类型 |
| 改 | `.../nomi/useNomiMessage.ts:449` | 删对应 emit（面板删除后该事件零监听者） |

**G1 保留**：`useNomiMessage.ts:450-456` 的 `last_token_usage` 持久化——它是
`ContextUsageRing` 重进会话后的回填来源（`:713-716`）。
`TurnProcessDisclosure.tsx:73` 有同名本地 `formatTurnDuration`，独立实现，不受影响。

### G2 `TokenUsageData` 收窄（~12 行）

`ui/src/common/config/storage.ts:97`：删 `input_tokens`、`output_tokens`、
`cache_creation_tokens`、`cache_read_tokens`、`elapsed_ms` 五个字段；保留
`total_tokens`（回填真值门）、`context_tokens`/`context_window`（圆环）。
`useNomiMessage.ts:437-446` 的构造处同步收窄（事件里的 `input/output_tokens` 仍要读，
用于算 `total_tokens`）。

已持久化的旧 `last_token_usage` JSON 可能含被删键：`extra` 是不透明 JSON，多余键运行时
被忽略，无需数据迁移。

### G3 acp 前端死指标（~33 行，纯 UI）

全部在 `ui/src/renderer/pages/conversation/platforms/acp/useAcpMessage.ts` 与
`ui/src/common/config/storage.ts`：

| 位置 | 内容 | 死因 |
|---|---|---|
| `useAcpMessage.ts:19` | `TokenUsageData` import | 删完即孤立 |
| `:166-167` | return 类型里的 `tokenUsage`/`context_limit` | 零消费者（已逐组件核到 `AcpSendBox.tsx:115-128` 解构的字段清单） |
| `:186-187` | 两个 useState | 同上 |
| `:668-677` | `acp_context_usage` 的 setState 分支 | state 无人读 |
| `:873-874` | 切会话 reset | 同上 |
| `:928-937` | `extra.last_token_usage`/`last_context_limit` 回填 | `last_context_limit` 全仓库零写入方；`last_token_usage` 仅 nomi 前端写。此 if 对 acp 永不进入 |
| `:1086-1087` | return 导出 | 同上 |
| `storage.ts:145-148` | acp extra 的两个键声明 | 同上（`storage.ts:265` 的 nomi 变体**保留**） |

**G3 顺带修正**：`chatLib.ts:1472` 注释 "handled by AcpSendBox" 与事实不符，case 本身
保留（防落入 default 分支的 `console.warn`），注释改为「已知事件，前端无消费者，静默」。

### G4 `AgentExecution.total_tokens` 全链（~20 行 Rust + 1 列）

证据：唯一语义读端是 `serde::Serialize`；前端类型有字段零消费者；DAG 画布展示的是
per-attempt 值，此 execution 级汇总从设计上冗余。

| 动作 | 位置 |
|---|---|
| 删聚合计算与写参 | `crates/backend/nomifun-agent-execution/src/scheduler.rs:2019-2024, 2036` |
| 删字段 | `crates/backend/nomifun-api-types/src/agent_execution.rs:275-276, 293`（及 `:783` 测试 fixture） |
| 删字段 | `crates/backend/nomifun-db/src/models/agent_execution.rs:18` |
| 删参数 | `crates/backend/nomifun-db/src/repository/agent_execution.rs:188`（`UpdateAgentExecutionParams.total_tokens`） |
| 删 SQL 写入 | `crates/backend/nomifun-db/src/repository/sqlite_agent_execution.rs:1919, 1936-1937` |
| 删映射 | `crates/backend/nomifun-agent-execution/src/domain_mapper.rs:102` |
| 删 TS 字段 | `ui/src/common/types/agentExecution/agentExecutionTypes.ts:97` |
| 新增 migration | `crates/backend/nomifun-db/migrations/024_drop_agent_execution_total_tokens.sql` |

Migration 内容（照抄 016/023 的注释+一行式风格）：

```sql
-- agent_executions.total_tokens has been write-only since the v3 baseline:
-- the scheduler summed per-attempt tokens into it at terminal states, but no
-- Rust logic, SQL clause, or UI code ever read the value back (the DAG canvas
-- renders per-attempt tokens instead). Drop the storage. The column-level
-- CHECK travels with the column; no index or table-level constraint
-- references it, so SQLite DROP COLUMN is legal here.
ALTER TABLE agent_executions DROP COLUMN total_tokens;
```

可行性已实测：对 `001_v3_baseline.sql:243-285` 的真实 DDL（含三个既有索引、表级
CHECK 均不涉及该列）执行 `DROP COLUMN` 成功，剩 21 列。**无需建表重写。**

### G5 `TurnCompletedEventData` 删三字段（~15 行 Rust + binding 再生成）

| 字段 | 死因 |
|---|---|
| `cache_creation_tokens`、`cache_read_tokens` | G1 删除面板后 UI 零读端；后端 relay 只读 `input+output` |
| `stop_reason` | 从未有读端——所有 `stop_reason` 消费者（`stream_relay.rs:4284,4302,4506`、`message_service.rs:648,659`）读的都是 `Finish` 事件的；此字段自注释为 "mirrors Finish" 的冗余副本 |

改动点：`crates/backend/nomifun-ai-agent/src/protocol/events/mod.rs:131-138,147-149`
（struct）、`:2512-2513,2523-2524`（序列化断言）、`manager/nomi/agent.rs:1251-1255`
（构造处）、`stream_relay.rs:6951,6957,6986` 等以显式字段构造该 struct 的测试点。
ts-rs binding（`ui/src/common/protocolBindings/TurnCompletedEventData.ts`）由
`cargo test` 再生成。前端 `useNomiMessage.ts:423-433` 的手写解析类型同步收窄。

**G5 保留**：`map_engine_stop_reason`（`Finish` 事件仍消费其结果）；
`agent.rs` 的 `turn_completed_mapping_tests` 相应调整而非删除。

## 4. 明确保留清单（承重证据）

| 保留项 | 证据 |
|---|---|
| `ExecutionAttempt.tokens` 全链 | 真实用户可见读端：DAG step 节点快览卡「⚡ N tokens」。链：`attempt_runner.rs:217` → `scheduler.rs:1818` → `sqlite_agent_execution.rs:3614` → `domain_mapper.rs:252` → `routes.rs:131` → `DagCanvas.tsx:275` → `StepNode.tsx:329-331`；结构测试 `executionCanvasIntegration.structure.test.ts:48` 锁定 |
| `TurnCompleted.input_tokens`/`output_tokens` | 喂上面那条链（`stream_relay.rs:3118-3123` 累加进 `runtime_state.add_turn_tokens`） |
| `TurnCompleted.elapsed_ms` | relay 与 `agent.rs` 结构化日志在用 |
| `TurnCompleted.context_tokens`/`context_window` | `ContextUsageRing` 数据源 |
| acp 后端采集链 | `translate.rs:1246` → `agent_event_tracker.rs:119` → `session.rs:509` → `acp_session_sync.rs:262` → `sqlite_acp_session.rs:269`；有单测覆盖；是唯一真实采集能力 |
| `GET /api/conversations/{id}/usage` | 公开 HTTP API，且是唯一能读出已持久化 acp usage 的出口 |
| `useAcpMessage.ts:72` 的 `'acp_context_usage'` 分类 | 事件仍会到达；移出集合会错误终结 thinking 段落 |
| `chatLib.ts:1472` 的 case | 防 console.warn 噪音（仅改注释） |
| `useNomiMessage.ts:450-456` 持久化 | 圆环回填来源 |
| `runtime_state.rs` 的 `add/take/clear_turn_tokens` | attempt.tokens 链的内存段 |

## 5. 验证方案

1. `bun run gen:i18n`——再生成 key 类型；`bun run check`（typecheck + check:i18n 等）。
2. `bun test --cwd ui`——含既有结构测试：`NomiSendBoxLayout.structure.test.ts`（圆环
   仍在）、`executionCanvasIntegration.structure.test.ts`（⚡ tokens 仍在）。
3. `bun run test`（= ensure-ui-dist + `cargo test`）——覆盖 migration 024、ts-rs binding
   再生成、`stream_relay`/`runtime_state`/`attempt_runner` 的既有 token 链测试。
4. 手工冒烟：新建 nomi 会话完成一轮——右侧工具栏只剩 文件/变更/终端；圆环随
   `turn_completed` 更新；重进会话圆环回填；DAG 画布 step 节点仍显示 token。
5. 确认 `.github/workflows/` 下无工作流文件（仓库规则）。

## 6. 明确不做

- **不**给 acp 接 `ContextUsageRing`（调查中发现的"最后一米"方案）——那是加功能，与
  瘦身目标相反；acp usage 数据依赖 unstable 协议能力，仓库内无一条真实数据通路的证据。
- **不** bump `ui-api-contract-version.txt`（理由见 §2.6）。
- **不**删 `/usage` REST 链路（公开 API + 唯一读出口）。
- **不**动 `Finish.stop_reason` 及其消费者。
