# 会话召唤伙伴（B 轨道，第二波）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在普通工作会话中"召唤伙伴"：技能物化 + 勾选记忆快照注入 + 只读 recall 工具 + 确认式记忆回写 + 伙伴侧反向分流（spec §设计 B 全部）。

**Architecture:** 召唤状态存会话行 `extra.summon`；记忆经 ContextContributor 每回合实时解析注入（预算 8000 字符）；技能复用 `materialize_skills_for_agent`；回写走 `companion_suggestions` 建议卡确认流；`nomi_create_conversation` 扩展 `workpath`/`summon` 参数作反向召唤入口。召唤/解除要求会话空闲，变更下一条消息生效（复用 knowledge signature 回收同款 runtime 重建路径）。

**Tech Stack:** Rust（nomifun-conversation / nomifun-companion / nomifun-ai-agent / nomifun-gateway）、React/Arco。

## Global Constraints

- **前置**：A 轨道（feat/companion-memory-upgrade）已合并 main——本计划消费其 `CompanionStore::search_memories(MemorySearchQuery)`、`MemorySearchHit`、MemoriesTab 的过滤/多选组件。开工第一步先 `git log --oneline -5` 确认并通读这些接口的实际落地形态。
- 构建前 PATH：`export PATH="/c/Users/MINISFORUM/.cargo/bin:/c/Program Files/CMake/bin:/c/tools/nasm-2.16.03:$PATH"`。
- persona 不接管；工作会话内**不注册** `save_memory`；伙伴记忆对召唤方只读。
- 首版完整支持 `type='nomi'` 会话；ACP 会话仅技能物化 + recall 工具（记忆快照区段留 TODO 注释与 spec 引用，不实现）。
- 新 i18n 键双语 + `bun run gen:i18n`；每任务一 commit。

---

## File Map

| 文件 | 动作 | 职责 |
|---|---|---|
| `crates/backend/nomifun-api-types/src/agent_build_extra.rs` | Modify | `extra.summon` 结构 `SummonConfig { companion_id, memory_ids, skill_exclusions, summoned_at }` |
| `crates/backend/nomifun-conversation/src/summon.rs` | Create | 召唤生命周期：set/clear 校验（会话空闲）+ runtime 签名失效触发重建 |
| `crates/backend/nomifun-conversation/src/routes.rs` + `service.rs` | Modify | `PUT/DELETE /api/conversations/{id}/summon` |
| `crates/backend/nomifun-companion/src/summon_support.rs` | Create | ①记忆快照解析器（ids→文本，8000 字符预算）②只读 recall sink 构造 ③`propose_companion_memory` 工具（写 companion_suggestions，source="summon"） |
| `crates/backend/nomifun-ai-agent/src/factory/nomi.rs`（+ `factory/mod.rs` 选项面） | Modify | 会话构建时读 `extra.summon`：注入 ContextContributor 区段、注册只读 recall + propose 工具、技能物化调用 |
| `crates/backend/nomifun-gateway/src/caps_conversation.rs` | Modify | `nomi_create_conversation` 加 `workpath`/`summon` 参数 |
| `crates/backend/nomifun-companion/src/companion.rs` | Modify | 本地模式提示词加"重型 coding 分流"规则段 |
| `ui/src/renderer/pages/conversation/.../SendBox 工具条` + 新 `SummonPanel/` | Create/Modify | 召唤按钮、三步面板（伙伴→技能→记忆多选，复用 MemoriesTab 组件）、头部/侧边栏徽标 |
| `ui/src/common/adapter/ipcBridge.ts` | Modify | summon set/clear API |

**Interfaces：**
- `SummonConfig`（serde：`companion_id: String, memory_ids: Vec<String>, skill_exclusions: Vec<String>, summoned_at: i64`），存 `extra.summon`。
- REST：`PUT /api/conversations/{id}/summon` body=SummonConfig（无 summoned_at，服务端盖章）；`DELETE .../summon`。会话非空闲返回 409。
- `summon_support::resolve_summon_context(store, &SummonConfig) -> String`（8000 字符预算，含"以下为召唤的伙伴记忆（只读参考）"前言与截断提示）。
- 工具：`recall_memories`（与伙伴侧同名同参，scope 锁定该 companion，只读）；`propose_companion_memory { kind, content, reason }` → 建议卡。

---

### Task 1: SummonConfig 类型 + summon 生命周期（TDD）

- [x] Step 1：agent_build_extra.rs 加 `SummonConfig`（serde 测试：roundtrip + 缺省字段拒绝）。
- [x] Step 2：`summon.rs`：`set_summon(conversation_id, cfg)` / `clear_summon(conversation_id)`——校验会话存在、属主、**无 active turn**（复用 runtime_state `has_active_turn` + 持久 running 检查，非空闲 409 Conflict）；写/删 `extra.summon`；调用与 knowledge 绑定变更同款的 runtime 失效路径（读 service.rs:12046-12084 的 `terminate_runtime_with_proof` 用法，空闲态直接终结注册 runtime 使下一条消息重建）。测试：空闲设置成功且 extra 落库；running 时 409；设置后 runtime 槽位被清。
- [x] Step 3：routes 挂 PUT/DELETE；commit。

### Task 2: companion 侧支撑件（TDD）

- [x] Step 1：`summon_support.rs`：`resolve_summon_context`（按 ids 查 store——含 archived 也解析，标注`[已归档]`；预算截断在条目边界；空 ids 返回空串）；只读 recall sink（复用 A 轨道 `search_memories`，scope 锁 companion_id，无写方法）；`propose_companion_memory` 处理器（插 `companion_suggestions`，kind 校验六维，source="summon"，payload 带来源 conversation_id）。
- [x] Step 2：单测：预算截断、归档标注、propose 落建议表且不触碰 companion_memories。commit。

### Task 3: 工厂接线

- [x] Step 1：读 factory/nomi.rs 现有 companion 工具注册与 ContextContributor 注入点（agent_build_extra.rs:285、factory/mod.rs:212 一带）；为带 `extra.summon` 的**非伙伴**会话：注册 recall/propose 两工具、加 summon ContextContributor（每回合调 resolve_summon_context）、系统提示追加一句"本会话已装载伙伴 {name} 的技能与所选记忆（只读）"。
- [x] Step 2：技能物化：会话准备阶段（工作区就绪后）对 summon 会话调 `materialize_skills_for_agent`（active 技能 − skill_exclusions，manifest 所有权沿用）；clear_summon 时按 manifest 卸载。
- [x] Step 3：集成测试（factory 测试基建）：summon 会话工具表含 recall/propose 不含 save_memory；上下文含记忆区段；非 summon 会话不受影响。commit。

### Task 4: gateway 扩展 + 伙伴分流提示词

- [x] Step 1：`nomi_create_conversation` 入参加 `workpath?: string`（设 `extra.custom_workspace=true`+`extra.workspace`）与 `summon?: { companion_id, memory_ids?, skill_exclusions? }`（服务端盖 summoned_at）；权限沿用现状（companion 身份可建顶层会话）。测试：带两参数创建后 extra 正确。
- [x] Step 2：companion.rs 本地模式提示词（"总管家"段后）加分流规则：重型 coding/工程任务 → 提议开工作会话并用 nomi_create_conversation 带 workpath（用户给了路径时）与 summon（自己 id + 按任务 recall 预选的记忆 ids）；征得主人同意再建。commit。

### Task 5: UI

- [x] Step 1：ipcBridge 加 `conversation.setSummon/clearSummon`；SendBox 工具条加"召唤伙伴"按钮（仅非伙伴、非客服会话显示）。
- [x] Step 2：`SummonPanel`（Drawer 三步：伙伴单选卡片 → 技能复选（默认全选 active）→ 记忆多选（复用 A 轨道 MemoriesTab 的搜索/过滤/多选件 + 预算字数条）→ 提交 PUT）；已召唤时按钮变徽标态，点开可查看/调整/解除（解除 DELETE）。会话头部与侧边栏条目徽标（侧边栏数据源 extra 已同步，加渲染即可）。
- [x] Step 3：i18n 双语 + gen:i18n；bun test 结构测试；commit。

### Task 6: 回归

- [x] `cargo test -p nomifun-conversation -p nomifun-companion -p nomifun-ai-agent -p nomifun-gateway` + `bun test` + `cargo check --workspace` 全绿；E2E 手册（记录在 commit message）：召唤→发消息→recall 命中→propose→建议卡出现→解除→技能目录被卸载。
