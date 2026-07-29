# 伙伴记忆系统升级（A 轨道）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 伙伴记忆检索从 SQL LIKE 升级为 FTS5（trigram）全文检索，归档层可检索可恢复，MemoriesTab 补批量操作与查重合并助手（spec §设计 A 全部）。

**Architecture:** memory.db（companion 域自管 schema）加外部内容 FTS5 表与预留 embedding 列；统一搜索接口进 `CompanionStore`；`recall_memories` 工具与 REST/前端换用新接口。衰减/归档/注入预算全部不动。

**Tech Stack:** Rust sqlx/SQLite（FTS5 bundled 已确认可用）、React/Arco（MemoriesTab 增强）。

## Global Constraints

- 构建前 PATH：`export PATH="/c/Users/developer/.cargo/bin:/c/Program Files/CMake/bin:/c/tools/nasm-2.16.03:$PATH"`。
- **只允许改**：`crates/backend/nomifun-companion/**`、`ui/src/renderer/pages/nomi/tabs/MemoriesTab*`、`ui/src/common/adapter/ipcBridge.ts`（companion 记忆 API 区段）、i18n `nomi`/相关 ns 的新增键。不得动 nomifun-db、nomifun-channel、nomifun-conversation。
- memory.db 演进走 companion store 既有机制（`store.rs` 的 SCHEMA 常量 + 启动逐表契约校验），对存量库做**幂等 ALTER/CREATE IF 升级**，禁止 hard reset 用户记忆数据。
- 半衰期参数、归档阈值 0.05、注入预算（pinned+每类 top5、6000 字符）不变。
- 新 i18n 键 zh-CN/en-US 双语言 + `bun run gen:i18n`；UI 测试用仓库既有 bun test 风格（纯函数 + 源码结构断言）。
- 每任务一 commit。

---

## File Map

| 文件 | 动作 | 职责 |
|---|---|---|
| `crates/backend/nomifun-companion/src/store.rs` | Modify | SCHEMA 加 FTS5 表 + embedding 两列；契约校验扩展；写路径同步 FTS |
| `crates/backend/nomifun-companion/src/memory_search.rs` | Create | `MemorySearchQuery`/`search_memories` 实现与排序融合（store 的子模块，由 store 调用） |
| `crates/backend/nomifun-companion/src/companion.rs` | Modify | `CompanionStoreSink::recall`（:1002-1031）换用 search；工具 schema 加 `queries[]`/`include_archived` |
| `crates/backend/nomifun-companion/src/service.rs` + `routes.rs` | Modify | listMemories 增强、batch、merge-suggestions/merge 四个 REST |
| `ui/src/common/adapter/ipcBridge.ts` | Modify | companion 记忆 API 类型与新端点封装 |
| `ui/src/renderer/pages/nomi/tabs/MemoriesTab.tsx`（+`.test.ts`） | Modify | 批量选择/操作、active/归档分段控件、命中高亮、排序切换、合并助手 |

**Interfaces（B 轨道第二波依赖，命名必须精确一致）：**
- `CompanionStore::search_memories(q: MemorySearchQuery) -> Result<Vec<MemorySearchHit>, AppError>`；
  `MemorySearchQuery { queries: Vec<String>, kind: Option<String>, scope: Option<MemoryScopeFilter>, status: MemoryStatusFilter /* Active|Archived|All */, companion_id: Option<CompanionId>, limit: usize }`；
  `MemorySearchHit { memory: CompanionMemoryRow, rank: f64, snippet: Option<String> }`。
- `recall_memories` 工具入参：`{ queries: string[], include_archived?: bool, limit?: int }`（旧单 `query` 字段保留为兼容别名：收到时视为 `queries:[query]`）。
- REST：`POST /api/companion/memories/batch { ids: string[], action: "archive"|"restore"|"delete"|"reclassify", kind?: string }`；`POST /api/companion/memories/merge-suggestions {}` → 分组数组；`POST /api/companion/memories/merge { group: string[], merged_content: string, kind: string }`。

---

### Task 1: schema 升级（FTS5 表 + embedding 列 + 契约校验）

- [ ] **Step 1**：通读 `store.rs` 的 SCHEMA 常量与契约校验（:441-676、:745-1344），确认它如何表达"表必须长这样"；找到升级钩子（若现状是"校验失败即拒启"，需为本次新增物新增幂等升级步骤：先 `ALTER TABLE ... ADD COLUMN` / `CREATE VIRTUAL TABLE IF NOT EXISTS`，再校验）。
- [ ] **Step 2**：写失败测试（store 测试模块既有风格）：在临时目录建"旧版"库（用当前 SCHEMA 去掉新增物），启动 store，断言升级后 `companion_memories` 有 `embedding`/`embedding_model` 列且 `companion_memories_fts` 存在、行数与主表 active+archived 总数一致。
- [ ] **Step 3**：实现。FTS 定义：

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS companion_memories_fts USING fts5(
  content, content='companion_memories', content_rowid='id', tokenize='trigram'
);
```

升级时全量重建填充：`INSERT INTO companion_memories_fts(rowid, content) SELECT id, content FROM companion_memories;`（先 `DELETE FROM companion_memories_fts` 保幂等）。
- [ ] **Step 4**：写路径同步——store 中所有 INSERT/UPDATE(content)/DELETE `companion_memories` 的方法逐一补 FTS 维护（external content 表用 `INSERT INTO companion_memories_fts(companion_memories_fts, rowid, content) VALUES('delete', old_id, old_content)` + 正常 insert 的标准配方）；归档/恢复只改 status 不动 content，无需动 FTS。加一致性测试：insert→update→archive→restore→delete 全程后 FTS 行数与内容匹配。
- [ ] **Step 5**：`cargo test -p nomifun-companion` 全绿 → commit `feat(companion): add FTS5 index and embedding columns to memory store`。

### Task 2: search_memories（TDD）

- [ ] **Step 1**：`memory_search.rs` 写失败测试：中文 trigram 命中（存"主人喜欢深烘焙咖啡豆"，查 `["咖啡"]` 命中）；多词 OR 去重；`status: Archived` 只回归档；kind/scope 过滤；rank 融合（同等 BM25 下 pinned > importance 高者 > strength 高者）；limit 生效；空 queries 返回 InvalidParam 错误。
- [ ] **Step 2**：实现：每个 query 词跑 `SELECT m.*, bm25(companion_memories_fts) AS r FROM companion_memories_fts f JOIN companion_memories m ON m.id=f.rowid WHERE companion_memories_fts MATCH ?`（词做 FTS 字符串转义，按仓库现有做法双引号包裹），合并去重后 `rank = -bm25 + pinned*2.0 + importance*0.5 + strength*0.5`，snippet 用 `snippet(companion_memories_fts, 0, '<b>','</b>','…',12)`。status/kind/scope/companion_id 作为 SQL WHERE 附加。全绿 → commit。

### Task 3: recall 工具升级

- [ ] **Step 1**：改 `CompanionStoreSink::recall`（companion.rs:1002-1031）调 `search_memories`；工具入参 schema 加 `queries[]`（兼容旧 `query`）与 `include_archived`；返回条目带 `memory_id/kind/created_at/archived`。伙伴提示词（:135-137 一带）把"recall_memories（搜你对主人的长期记忆）"补一句"可传多个查询词，找旧事可带 include_archived"。
- [ ] **Step 2**：更新/补 sink 单测（旧 LIKE 测试改为经 FTS 语义断言）→ 全绿 → commit `feat(companion): recall_memories multi-query FTS search with archive access`。

### Task 4: REST（batch / merge-suggestions / merge + list 增强）

- [ ] **Step 1**：`service.rs`+`routes.rs` 按 Interfaces 定义实现四个端点；batch 单事务（reclassify 校验 kind ∈ 六维）；merge-suggestions 复用 `find_similar_active`（store.rs:1879）按归一化相似分组（仅 active、同 scope），**不调 LLM**（建议文案生成放前端触发的既有 LLM 通道过重，首版返回分组+各条原文，由用户手工编辑合并文案——YAGNI）；merge：插入合并后新记忆（kind 取参数）+ 原组条目归档（audit 字段记 superseded_by）。
- [ ] **Step 2**：路由测试（routes 测试既有风格：batch 三动作、merge 后原条目归档新条目 active、非法 kind 400）→ 全绿 → commit。

### Task 5: 前端（ipcBridge + MemoriesTab）

- [ ] **Step 1**：ipcBridge companion 区段加 `batchMemories`/`memoryMergeSuggestions`/`mergeMemories` 封装与类型；`listMemories` 参数类型加 `sort?: 'relevance'|'time'|'importance'`（后端 list 同步支持，q 存在时默认 relevance 走 FTS）。
- [ ] **Step 2**：MemoriesTab：行 checkbox + 顶部批量工具条（归档/恢复/删除/改分类，Arco `Modal.confirm` 确认）；active/归档 Radio 分段控件替换状态下拉；q 命中时渲染 snippet 高亮（`<b>` 白名单渲染）；排序 Select；"查重合并"按钮 → 抽屉列分组 → 每组勾选保留项+编辑合并文案 → 提交 merge。所有新文案走 i18n（zh/en）+ `bun run gen:i18n`。
- [ ] **Step 3**：按 `MemoriesTab.test.ts` 既有风格补结构/纯函数测试；`bun test ui/src/renderer/pages/nomi/tabs` 与 `bun run check:i18n` 全绿 → commit `feat(ui): memory batch ops, archive browsing, merge assistant`。

### Task 6: 回归

- [ ] `cargo test -p nomifun-companion` + `bun test`（nomi 目录）+ `cargo check --workspace` 全绿 → commit。
