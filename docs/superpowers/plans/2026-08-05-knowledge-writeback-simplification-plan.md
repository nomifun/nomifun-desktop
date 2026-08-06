# 知识库回写简化 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 移除暂存回写、把回写意识从 conservative/aggressive 改为具备真实行为差异的 manual/auto、彻底删除待审功能，并把工具写入路径统一到既有的追加式合并 + CAS 安全路径。

**Architecture:** 回写从"两个正交维度 + 人工审阅"退化为"一个开关 + 一个意识"。落点只剩直写；意识的差异落在**是否触发回合末自动抽取**上，而非仅提示词措辞。数据库侧走追加式迁移 025（baseline 被校验和钉死，不可编辑），两张表各自的 CHECK 值域用 ADD→UPDATE→DROP→RENAME 配方替换。

**Tech Stack:** Rust（axum / sqlx / SQLite）、React 19 + TypeScript（Arco Design + UnoCSS）、bun、i18next。

## Global Constraints

- **禁止 AI 署名**：Claude 及任何 AI 模型不得出现在 Git author / committer / co-author / 致谢中，不得添加 AI 归属 trailer。钩子 `commit-msg` 与 `pre-push` 会拒绝。不得使用 `--no-verify`。
- **不得编辑 `crates/backend/nomifun-db/migrations/001_v3_baseline.sql`**：sqlx 校验和钉死，改动即令每个现存安装启动失败。
- **新迁移文件版本号必须唯一**：当前最高 `024`，本次用 `025`；不得移除 `migration_file_versions_are_unique` 守卫测试。
- **模型面向文案全部英文**（`context.rs:13` 项目约定），逐字采用技术方案 §2 给出的文本，不可意译。
- **两个 locale 必须同步改**：`ui/src/renderer/services/i18n/locales/{zh-CN,en-US}`；`localeKeyParity.ts:122-130` 把单侧存在视为硬错误。
- **`i18n-keys.d.ts` 是生成物**：只能由 `bun run gen:i18n` 生成，不得手改。
- **禁用词**：`orchestrat*`、`sub-agent`、`agent-cluster`、`fleet*` 不得出现在 `crates/`、`apps/`、`ui/src/`、`scripts/`、`README(.zh-CN).md`、`CONTRIBUTING.md`、`docs/{architecture,guides,reference,skills}` —— `check:agent-vocabulary` 会拒。
- **不得改写**：两份 `docs/handoffs/*`、已发布的 `CHANGELOG.md` 段落（`STATUS.md:73-78` 保护的历史记录）。
- **假阳性禁改清单**（一次全局替换就会毁掉的无关功能）：`nomifun-knowledge/workspace_binding.rs:58,224,548`（锁保守性）、`mount.rs:195,204,260-266,697`（原子替换 staging）、`locales/*/nomi.json:223-226`（`EvolvePreference` 技能生成偏好）、`locales/*/idmm.json:42-46`（备用模型倾向）、`i18n-keys.d.ts:2064-2077` 与 `:3973`（存活的回写键）、`docs/guides/companions.md:106` 与 `.zh.md:44`。
- **每个任务结束提交一次**，提交信息用 conventional commit 前缀，正文说明"为什么"。

---

### Task 1: 迁移 025 + `nomifun-db` 层

**Files:**
- Create: `crates/backend/nomifun-db/migrations/025_knowledge_writeback_simplification.sql`
- Modify: `crates/backend/nomifun-db/src/models/knowledge.rs:66-69,129`（删 `writeback_mode` 字段与 `try_get`）、`:201`（测试字面量）
- Modify: `crates/backend/nomifun-db/src/models/preset.rs:161`（5 元组 → 4 元组）
- Modify: `crates/backend/nomifun-db/src/repository/sqlite_knowledge.rs:277-278,314-321,335-345,712-713,743-744`
- Modify: `crates/backend/nomifun-db/src/repository/sqlite_preset.rs:349-352`
- Modify: `crates/backend/nomifun-db/src/repository/sqlite_conversation.rs:6304-6314`（确认 INSERT 不点名这两列，依赖 DDL 默认值 —— 只需确认，通常无需改）
- Modify: `crates/backend/nomifun-db/tests/id_schema_contract.rs:1277-1299`
- Test: `crates/backend/nomifun-db/tests/`（迁移测试，落在既有测试文件或新建 `knowledge_writeback_migration.rs`）

**Interfaces:**
- Produces：`KnowledgeBindingRow` 不再有 `writeback_mode` 字段；`writeback_eagerness: String` 值域为 `"manual" | "auto"`。`CreatePresetParams::knowledge_policy` 变为 `(bool, bool, Option<String>, bool)` = (enabled, writeback, eagerness, grounded)。

- [ ] **Step 1：写失败测试 —— 迁移映射**

在 `crates/backend/nomifun-db/tests/knowledge_writeback_migration.rs` 新建。测试要点：用内存库跑到迁移 024 的状态插入旧值行不现实（sqlx 一次跑全链），所以改为跑完整链后断言**新 schema 的行为**，并单独用裸 SQLite 复现旧值映射：

```rust
use nomifun_db::init_database_memory;
use sqlx::Row;

#[tokio::test]
async fn migration_025_leaves_only_manual_auto_and_drops_mode() {
    let db = init_database_memory().await.expect("db");
    let pool = db.pool();

    // writeback_mode 列必须不存在
    let cols: Vec<String> = sqlx::query("PRAGMA table_info(knowledge_bindings)")
        .fetch_all(pool)
        .await
        .expect("pragma")
        .into_iter()
        .map(|r| r.get::<String, _>("name"))
        .collect();
    assert!(!cols.iter().any(|c| c == "writeback_mode"), "writeback_mode must be dropped");
    assert!(cols.iter().any(|c| c == "writeback_eagerness"));

    // 不点名该列的 INSERT 必须拿到新默认值 manual
    let binding_id = nomifun_common::KnowledgeBindingId::new();
    sqlx::query(
        "INSERT INTO knowledge_bindings (knowledge_binding_id, target_kind, target_workpath, updated_at) \
         VALUES (?, 'workpath', '/tmp/x', 1)",
    )
    .bind(binding_id.as_str())
    .execute(pool)
    .await
    .expect("insert relying on DDL default");
    let eagerness: String = sqlx::query_scalar(
        "SELECT writeback_eagerness FROM knowledge_bindings WHERE knowledge_binding_id = ?",
    )
    .bind(binding_id.as_str())
    .fetch_one(pool)
    .await
    .expect("select");
    assert_eq!(eagerness, "manual");

    // 旧值必须被 CHECK 拒绝
    let stale = nomifun_common::KnowledgeBindingId::new();
    let err = sqlx::query(
        "INSERT INTO knowledge_bindings (knowledge_binding_id, target_kind, target_workpath, \
         writeback_eagerness, updated_at) VALUES (?, 'workpath', '/tmp/y', 'conservative', 1)",
    )
    .bind(stale.as_str())
    .execute(pool)
    .await;
    assert!(err.is_err(), "legacy 'conservative' must be rejected by the new CHECK");

    // preset_knowledge_policy 同样处理
    let pcols: Vec<String> = sqlx::query("PRAGMA table_info(preset_knowledge_policy)")
        .fetch_all(pool)
        .await
        .expect("pragma")
        .into_iter()
        .map(|r| r.get::<String, _>("name"))
        .collect();
    assert!(!pcols.iter().any(|c| c == "mode"), "preset policy mode must be dropped");
}
```

再补一个纯 SQL 的值映射测试，直接建旧表跑迁移语句，证明 `conservative→manual` / `aggressive→auto`：

```rust
#[tokio::test]
async fn migration_025_maps_legacy_eagerness_values() {
    let pool = sqlx::SqlitePool::connect(":memory:").await.expect("pool");
    sqlx::query(
        "CREATE TABLE knowledge_bindings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            writeback_eagerness TEXT NOT NULL DEFAULT 'conservative'
                CHECK (writeback_eagerness IN ('conservative','aggressive')),
            writeback_mode TEXT NOT NULL DEFAULT 'staged'
                CHECK (writeback_mode IN ('staged','direct')))",
    )
    .execute(&pool)
    .await
    .expect("legacy ddl");
    sqlx::query("INSERT INTO knowledge_bindings (writeback_eagerness, writeback_mode) VALUES ('conservative','staged'), ('aggressive','direct')")
        .execute(&pool)
        .await
        .expect("seed");

    for stmt in include_str!("../migrations/025_knowledge_writeback_simplification.sql")
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with("--"))
    {
        if !stmt.contains("knowledge_bindings") {
            continue; // 本测试只覆盖 knowledge_bindings 段
        }
        sqlx::query(stmt).execute(&pool).await.unwrap_or_else(|e| panic!("{stmt}: {e}"));
    }

    let values: Vec<String> = sqlx::query_scalar("SELECT writeback_eagerness FROM knowledge_bindings ORDER BY id")
        .fetch_all(&pool)
        .await
        .expect("select");
    assert_eq!(values, vec!["manual".to_owned(), "auto".to_owned()]);
}
```

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p nomifun-db --test knowledge_writeback_migration
```
预期：编译失败或断言失败（迁移文件尚不存在 / 列仍在）。

- [ ] **Step 3：写迁移文件**

把技术方案 §3 的完整 SQL（含注释）写入 `crates/backend/nomifun-db/migrations/025_knowledge_writeback_simplification.sql`。

- [ ] **Step 4：改 `nomifun-db` 类型与 SQL**

- `models/knowledge.rs`：删 `KnowledgeBindingRow.writeback_mode` 字段（`:69`）、`from_row` 的 `try_get`（`:129`）、测试字面量 `:201`；`:70-74` 的 `writeback_eagerness` 文档注释改写为 `manual`/`auto` 语义；`:75-79` 的 `channel_write_enabled` 注释去掉 "forced to STAGED placement"。
- `models/preset.rs:161`：`knowledge_policy: (bool, String, bool, Option<String>, bool)` → `(bool, bool, Option<String>, bool)`，doc 注释同改为 `enabled, writeback, eagerness, grounded`。
- `repository/sqlite_knowledge.rs`：`:277-278` 入参去掉 `writeback_mode: &str`；`:314-321` UPDATE 去掉该列与 `.bind`；`:335-345` INSERT 同理；`:712-713`、`:743-744` 测试断言改新值域。
- `repository/sqlite_preset.rs:349-352`：解构去掉 `mode`，INSERT 列表与 `.bind` 同步。
- `tests/id_schema_contract.rs:1277-1299`：两处点名 `writeback_mode, writeback_eagerness` 的 INSERT —— 前者删列，后者字面量改 `'manual'`；`:1295-1299` 那条断言 `is_err()` 的用例要确认它仍因**原本的原因**失败（非法 companion id），而不是因为新的 CHECK 巧合失败。

- [ ] **Step 5：跑测试确认通过**

```bash
cargo test -p nomifun-db
```
预期：全绿，含 `migration_file_versions_are_unique`。

- [ ] **Step 6：提交**

```bash
git add crates/backend/nomifun-db
git commit -F - <<'MSG'
refactor(db): drop writeback_mode and move eagerness to manual/auto

Staged write-back is going away, so writeback_mode has a single legal value
left and the on/off switch is already knowledge_bindings.writeback. Both
affected columns carry column-level CHECK constraints and the v3 baseline is
checksum-pinned, so migration 025 replaces the value domains with the
ADD/UPDATE/DROP/RENAME recipe instead of editing the baseline.

preset_knowledge_policy carries a second, independent eagerness CHECK; a
knowledge_bindings-only migration would leave it rejecting the new values
forever. Its mode column loses all meaning with the placement dimension gone.

CHECK and DEFAULT change together because sqlite_conversation inserts
knowledge_bindings rows without naming these columns.
MSG
```

---

### Task 2: `nomifun-knowledge` 全量改造

本 crate 内部是原子的 —— 删除 `WriteMode::Staged` 会同时打断本 crate 全部 match，所以枚举、策略、写路径、路由、清理必须在一个任务里落地。

**Files:**
- Modify: `crates/backend/nomifun-knowledge/src/context.rs`（`:25-53` 删 `WritebackMode`；`:55-90` 改 `WritebackEagerness`；`:107-131` 删两字段；`:281-346` 改 `writeback_contract`；`:348-366` 改 `eagerness_clause`；`:869-876` 测试）
- Modify: `crates/backend/nomifun-knowledge/src/service.rs`（见下）
- Modify: `crates/backend/nomifun-knowledge/src/routes.rs:14-17,70-88,579-684`
- Modify: `crates/backend/nomifun-knowledge/src/lib.rs:47`
- Modify: `crates/backend/nomifun-knowledge/src/turn_writeback.rs:10,17-30,53-64,187-205`
- Modify: `crates/backend/nomifun-knowledge/src/autogen.rs:18,222-226,244,312-332`
- Modify: `crates/backend/nomifun-knowledge/src/broker.rs:750`
- Modify: `crates/backend/nomifun-knowledge/src/mcp_server.rs:46,400-406,546,1007,1070,1206,1280,1328`
- Modify: `crates/backend/nomifun-knowledge/src/export.rs:8-10`
- Modify: `crates/backend/nomifun-knowledge/src/testutil.rs`（暂存相关固定值）
- Modify: `crates/backend/nomifun-knowledge/Cargo.toml:26`、`Cargo.toml:132`（摘 `similar`）

**Interfaces:**
- Consumes：Task 1 的 `KnowledgeBindingRow`（无 `writeback_mode`）。
- Produces：`WriteMode { Disabled, Direct }`；`WriteOutcome` 无 `staged`；`resolve_write_policy(surface, binding)`（无 `scope` 参数）；`KnowledgeBinding` 无 `writeback_mode`；`WRITEBACK_EAGERNESS = ["manual","auto"]`；`WritebackEagerness { Manual, Auto }`；`KnowledgeContextOptions` 无 `writeback_mode`/`target_id`；`KnowledgeBaseInfo` 无 `pending_inbox`。`lib.rs` 不再导出 `InboxDiff`/`InboxEntry`/`InboxMergeResult`/`KB_INBOX_REL_DIR`。

- [ ] **Step 1：写失败测试 —— 意识解析 + 四表面写策略**

替换 `context.rs:869-876` 的既有解析测试，并在 `service.rs` 测试模块新增策略测试：

```rust
// context.rs 测试模块
#[test]
fn eagerness_parse_falls_back_to_manual() {
    assert_eq!(WritebackEagerness::parse(None), WritebackEagerness::Manual);
    assert_eq!(WritebackEagerness::parse(Some("manual")), WritebackEagerness::Manual);
    assert_eq!(WritebackEagerness::parse(Some("auto")), WritebackEagerness::Auto);
    // 大小写不宽容、旧值不再识别、空串与未知值一律落到克制的一侧
    assert_eq!(WritebackEagerness::parse(Some("AUTO")), WritebackEagerness::Manual);
    assert_eq!(WritebackEagerness::parse(Some("conservative")), WritebackEagerness::Manual);
    assert_eq!(WritebackEagerness::parse(Some("aggressive")), WritebackEagerness::Manual);
    assert_eq!(WritebackEagerness::parse(Some("")), WritebackEagerness::Manual);
    assert_eq!(WritebackEagerness::default(), WritebackEagerness::Manual);
}
```

```rust
// service.rs 测试模块
fn binding(writeback: bool, channel: bool) -> KnowledgeBinding {
    KnowledgeBinding {
        enabled: true,
        writeback,
        writeback_eagerness: "manual".into(),
        channel_write_enabled: channel,
        kb_ids: Vec::new(),
    }
}

#[test]
fn write_policy_is_direct_or_disabled_on_every_surface() {
    for surface in [
        WriteSurface::RegularChat,
        WriteSurface::Companion,
        WriteSurface::TerminalAcp,
        WriteSurface::ExternalChannel,
    ] {
        assert!(matches!(
            resolve_write_policy(surface, &binding(false, true)).mode,
            WriteMode::Disabled
        ), "writeback off must disable {surface:?}");
    }
    assert!(matches!(resolve_write_policy(WriteSurface::RegularChat, &binding(true, false)).mode, WriteMode::Direct));
    assert!(matches!(resolve_write_policy(WriteSurface::Companion, &binding(true, false)).mode, WriteMode::Direct));
    assert!(matches!(resolve_write_policy(WriteSurface::TerminalAcp, &binding(true, false)).mode, WriteMode::Direct));
    // 渠道回写仍然默认关闭 —— 这道开关不能随暂存一起消失
    assert!(matches!(resolve_write_policy(WriteSurface::ExternalChannel, &binding(true, false)).mode, WriteMode::Disabled));
    assert!(matches!(resolve_write_policy(WriteSurface::ExternalChannel, &binding(true, true)).mode, WriteMode::Direct));
}
```

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p nomifun-knowledge write_policy_is_direct_or_disabled_on_every_surface
```
预期：编译失败（`resolve_write_policy` 签名不匹配、`KnowledgeBinding` 仍有 `writeback_mode`）。

- [ ] **Step 3：改 `context.rs`**

按技术方案 §1.1 改类型与 §2.1/§2.2 逐字改文案。`writeback_contract` 从五段（{工具,文件}×{暂存,直写} + 禁用）压成三段（{工具,文件}×直写 + 禁用），`mode` 参数删除。

- [ ] **Step 4：改 `service.rs` 类型与策略**

- `:383-390` `WriteMode` 删 `Staged{scope}`；`:408-419` 删 `StagedBaseSnapshot`/`StagedProposalMetadata`；`:422-427` 删 `WriteOutcome.staged`。
- `:231-245` `KnowledgeBinding` 删 `writeback_mode`，`channel_write_enabled` 注释改写；`:247-253` 删 `default_writeback_mode`，`default_writeback_eagerness` 返回 `"manual"`；`Default` impl 同改。
- `:46-55` 删 `WRITEBACK_MODES`，`WRITEBACK_EAGERNESS` 改 `["manual","auto"]`。
- `:507-536` `resolve_write_policy` 按技术方案 §1.2 全量替换。
- `:3520-3530` `set_binding` 删 mode 校验分支。
- `:4414-4426` `binding_from_row` 删 mode 校验分支。
- `:142-144` 删 `KnowledgeBaseInfo.pending_inbox`；`:4298-4305` 与 `:1919-1929` 两处计算删除。
- 删 `KB_INBOX_REL_DIR` 常量。

- [ ] **Step 5：改写工具写入路径（追加式合并 + CAS）**

按技术方案 §4 替换 `write_resolved_document_under_target_lock`（`:2039-2164`），并去掉 `:2035` 调用点的第三个 `None` 实参。同时删除只服务暂存的方法族：`capture_staged_base_snapshot`、`ensure_staged_base_snapshot_is_current`、`write_staged_proposal_metadata`、`remove_staged_proposal_metadata`、`require_valid_staged_proposal_metadata`、`list_inbox`、`inbox_diff`、`merge_inbox`、`discard_inbox`、`merge_all_inbox`、`discard_all_inbox`、`count_pending_inbox` 及其私有辅助（技术方案 §6 给出规模）。

- [ ] **Step 6：写失败测试 —— §4 追加式合并**

```rust
#[tokio::test]
async fn tool_direct_update_appends_and_never_truncates() {
    let (svc, kb_id) = crate::testutil::service_with_base().await;
    let original = format!("# 术语表\n\n{}\n", "旧内容行\n".repeat(2000));
    svc.write_file_for_test(kb_id.as_str(), "terms.md", &original).await.expect("seed");

    let req = WriteRequest {
        spec: WriteTargetSpec::Path { kb_id: kb_id.clone(), rel_path: "terms.md".into() },
        content: "市盈率 = PER".into(),
        policy: WritePolicy {
            mode: WriteMode::Direct,
            allow_create: true,
            surface: WriteSurface::RegularChat,
        },
        bound_kb_ids: vec![kb_id.clone()],
    };
    let out = svc.write_document(req.clone()).await.expect("write");
    assert_eq!(out.op, WriteOp::Update);

    let after = svc.read_file(kb_id.as_str(), "terms.md").await.expect("read").content;
    assert!(after.starts_with("# 术语表"), "original must be preserved verbatim");
    assert!(after.contains("旧内容行"), "original body must survive");
    assert!(after.contains("市盈率 = PER"), "new material must be appended");
    assert!(after.len() > original.len(), "append must grow the document, not replace it");

    // 幂等：同一材料再提交一次不得重复追加
    svc.write_document(req).await.expect("idempotent write");
    let again = svc.read_file(kb_id.as_str(), "terms.md").await.expect("read").content;
    assert_eq!(again, after, "resubmitting the same material must be a no-op");
}
```

`testutil.rs` 若无 `service_with_base` / `write_file_for_test`，按该文件既有固定值风格补上（它已有建库与写文件的辅助，复用而非新造）。

- [ ] **Step 7：清理路由、再导出与外围模块**

- `routes.rs`：删 `:70-88` 的 7 条待审路由注册、`:581-684` 的 7 个 handler 与 4 个 DTO（`InboxItemQuery`/`InboxActionRequest`/`InboxBatchRequest`/`InboxBatchResult`）、`:14-17` 导入；`:579` 段标题从 `// ── P4 inbox review + consumers ───` 改为只描述 consumers。
- `lib.rs:47`：删四个再导出。
- `turn_writeback.rs`：`:17-30` 按技术方案 §2.3 改 `TURN_WRITEBACK_SYSTEM`；`:53-64` 改两个 match；`:187-205` 测试改为断言 `eagerness: auto` 与新规则句，删除已失去意义的 `!prompt.contains("_inbox/{")` 断言。
- `autogen.rs`：删 `:18` 的 `KB_INBOX_REL_DIR` 导入与 `:244` 过滤器、`:222-226` 注释中的 `_inbox` 描述、`:312-332` 那个种 `_inbox/` 草稿的测试。
- `broker.rs:750`、`mcp_server.rs:1007,1070,1206,1280,1328`：删测试字面量 `writeback_mode: "staged"`。
- `mcp_server.rs:546`：删 wire JSON 键 `"staged"`；`:46,400-406`：删 `opaque_workpath_write_scope` 并清掉随之失用的 `sha2`/`hex` 导入。
- `export.rs:8-10`：注释改写（打包本就无路径过滤，行为不变）。
- `Cargo.toml`：摘 `similar`（`nomifun-knowledge/Cargo.toml:26` 与工作区 `Cargo.toml:132`）。

- [ ] **Step 8：跑测试确认通过**

```bash
cargo test -p nomifun-knowledge
```
预期：全绿。若 clippy 报未用导入，一并清理：`cargo clippy -p nomifun-knowledge -- -D warnings`。

- [ ] **Step 9：提交**

```bash
git add crates/backend/nomifun-knowledge Cargo.toml
git commit -F - <<'MSG'
refactor(knowledge): make write-back direct-only and manual/auto

Placement collapses to one option, so WritebackMode and the _inbox path
builder go away along with the review inbox's seven routes, its service
methods, and the pending_inbox projection. The eagerness enum becomes
Manual/Auto and its prose now states a real policy rather than a tone.

The tool write path stops overwriting. It now reads the document, appends
through merge_direct_turn_writeback, and publishes under
write_file_if_unchanged — the same path the turn-final finalizer already
used. Staging held the only compare-and-swap in the write surface, so
unifying the two paths is what keeps a single bad model call from replacing
a curated document with a summary of itself.

Because the merge appends, the tool schema and the contract prose had to stop
telling the model to resend the whole document: a proposal that contains the
document would otherwise fail the already-present check and be appended to it.

similar loses its last workspace user with the inbox diff.
MSG
```

---

### Task 3: `nomi-agent` 工具层

**Files:**
- Modify: `crates/agent/nomi-agent/src/knowledge_tools.rs:251-259,264-279,302-322,418-424,476-480,740-752`
- Modify: `crates/agent/nomi-agent/src/lib.rs`（若再导出 `WriteMode`）
- Modify: `crates/agent/nomi-agent/tests/engine_test.rs`（知识相关固定值）

**Interfaces:**
- Produces：`WriteRequest { target, content, bound_kb_ids }`（无 `mode`）；`WriteReceipt { final_rel_path, updated }`（无 `staged`）；`KnowledgeWriteTool::new(sink, bases, bound_kb_ids)`（3 参）；`WriteMode` 枚举**已删除**。

- [ ] **Step 1：改写既有测试 `write_by_handle_builds_handle_target`**

`:743-752` 是全仓库唯一验证"模型给的 `handle` 变成 `WriteTarget::Handle`"的覆盖，**不能删**，改为直写构造：

```rust
#[tokio::test]
async fn write_by_handle_builds_handle_target() {
    let sink = Arc::new(RecordingSink::default());
    let kb_id = KnowledgeBaseId::new();
    let tool = KnowledgeWriteTool::new(
        sink.clone(),
        vec![(kb_id.clone(), "Ops".to_owned())],
        vec![kb_id],
    );
    let res = tool
        .call(serde_json::json!({ "handle": "h-1", "content": "# note" }))
        .await;
    assert!(!res.content.is_empty());
    let req = sink.last().expect("sink received a request");
    assert!(matches!(req.target, WriteTarget::Handle(ref h) if h == "h-1"));
}
```

`RecordingSink` 按该文件既有测试替身风格实现 `KnowledgeWritebackSink`，其 `write` 返回 `WriteReceipt { final_rel_path: "ops/note.md".into(), updated: true }`。

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p nomi-agent write_by_handle_builds_handle_target
```
预期：编译失败（`KnowledgeWriteTool::new` 仍是 4 参、`WriteReceipt` 仍有 `staged`）。

- [ ] **Step 3：改类型与工具**

按技术方案 §1.3 删 `WriteMode` 枚举、`WriteRequest.mode`、`WriteReceipt.staged`、`KnowledgeWriteTool.mode` 字段与 `new` 的第三参；`:418-424` 按技术方案 §4.1 改 `content` 字段描述；`:476-480` 删 `" (STAGED to the review inbox; ...)"` 整段，回执改为只区分创建与追加；`:251-259`、`:291-301`、`:307` 的 doc 注释去掉 staged/direct 措辞。

- [ ] **Step 4：跑测试确认通过**

```bash
cargo test -p nomi-agent
```
预期：全绿。`:740` 那条 `assert!(!res.content.contains(...))` 的负向断言若断言的是 STAGED 文案，随文案删除一并处理。

- [ ] **Step 5：提交**

```bash
git add crates/agent/nomi-agent
git commit -F - <<'MSG'
refactor(nomi-agent): drop the placement mirror from the knowledge write tool

With staging gone the mirror WriteMode enum would have a single variant, and
the tool never needed to know the placement anyway — the service enforces it.
Removing it takes the mode field off WriteRequest, the staged flag off
WriteReceipt, and one argument off KnowledgeWriteTool::new.

The content field description now says the value is the new material to
append, not the full document: the service merges append-only, so a model
that resends the document would have it appended to itself.
MSG
```

---

### Task 4: `nomifun-ai-agent`

**Files:**
- Modify: `crates/backend/nomifun-ai-agent/src/knowledge_writeback.rs:1-49`（整文件按技术方案 §1.4）
- Modify: `crates/backend/nomifun-ai-agent/src/factory/nomi.rs:44,303-311,316-322,702-706,778,976,1581,1601,2472-2513`
- Modify: `crates/backend/nomifun-ai-agent/src/factory/acp_assembler.rs:323,828-856,871-903`
- Modify: `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs:378,393,414,573-577,752-761`
- Modify: `crates/backend/nomifun-ai-agent/src/manager/openclaw/agent/mod.rs`、`manager/remote/agent.rs`、`runtime_registry.rs:245`
- Modify: `crates/backend/nomifun-api-types/src/agent_build_extra.rs:395`（删 `knowledge_writeback_mode`，留 `knowledge_channel_write_enabled`）
- Modify: `crates/backend/nomifun-ai-agent/tests/knowledge_writeback_e2e.rs:117-121`
- Modify: `crates/backend/nomifun-ai-agent/tests/prompt_pipeline_integration.rs:50-78`
- Modify: `crates/backend/nomifun-ai-agent/tests/factory_provider_integration.rs`、`acp_agent_integration.rs`（若含相关固定值）

**Interfaces:**
- Consumes：Task 2 的 `WriteMode{Disabled,Direct}`、`WriteOutcome`（无 `staged`）、`KnowledgeContextOptions`（无 `writeback_mode`/`target_id`）；Task 3 的 3 参 `KnowledgeWriteTool::new`、无 `mode` 的 `WriteRequest`、无 `staged` 的 `WriteReceipt`。
- Produces：`AcpBuildExtra`/`AgentBuildExtra` 无 `knowledge_writeback_mode`；`NomiAgentManager::new` / `new_with_host_wiring` 少一个位置参数。

- [ ] **Step 1：改写 e2e 断言**

`tests/knowledge_writeback_e2e.rs:117-121` 现断言"直写 Update 覆盖原文"，改为断言追加式合并：

```rust
// 3. Update by handle in DIRECT mode → appends to the original, never replaces it.
let after = read_doc(&svc, &kb_id, "ops/runbook.md").await;
assert!(after.contains(ORIGINAL_BODY), "the original body must survive an update");
assert!(after.contains(NEW_MATERIAL), "the new material must be appended");
```

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p nomifun-ai-agent --test knowledge_writeback_e2e
```
预期：编译失败（sink/类型不匹配）。

- [ ] **Step 3：改 sink 与 factory**

`knowledge_writeback.rs` 整文件按技术方案 §1.4 替换。`factory/nomi.rs`：`:303-306` 删 `.unwrap_or_else(|| "staged".to_owned())` 兜底；`:311` 删 `knowledge_writeback_mode` 传递；`:316-322` 的 sink 门与 `resolve_write_policy` 调用去掉 `scope` 实参；`:976` 的 `KnowledgeContextOptions` 字面量删两字段；`:44` 的 overrides 清理；`:2499-2513` 的测试断言从 `WriteMode::Staged` 改为直写并与 `:2472-2492` 的 `channel_write_enabled` 存活性回归解耦。

- [ ] **Step 4：改位置参数 —— 逐调用点核对**

`manager/nomi/agent.rs:378,393,414` 的 `knowledge_writeback_staged: bool` 是 `new` / `new_with_host_wiring` 的**第 12 个位置参数**。删除后立即到 `factory/nomi.rs:778` 核对全部实参顺序 —— 相邻同类型参数错位不会报编译错误。核对方式：删除前后各跑一次 `cargo expand` 不现实，改为人工逐参对照参数名与实参表达式。

- [ ] **Step 5：改 ACP 装配与 api-types**

`acp_assembler.rs:323` 的 `KnowledgeContextOptions` 字面量、`:828-856` 与 `:871-903` 三个**无 `..Default::default()`** 的穷尽 `AcpBuildExtra` 字面量、`api-types/agent_build_extra.rs:395` 附近的字段、`runtime_registry.rs:245` 的键名列表、`tests/prompt_pipeline_integration.rs:50-78` 的第四个穷尽字面量。

- [ ] **Step 6：跑测试确认通过**

```bash
cargo test -p nomifun-ai-agent -p nomifun-api-types
```

- [ ] **Step 7：提交**

```bash
git add crates/backend/nomifun-ai-agent crates/backend/nomifun-api-types
git commit -F - <<'MSG'
refactor(ai-agent): stop threading a placement through the write-back sink

The sink becomes a pure target mapping now that placement has one value, and
the staged flag drops out of the receipt. knowledge_writeback_mode leaves
AcpBuildExtra and the runtime registry, and the knowledge_writeback_staged
positional argument leaves NomiAgentManager's two constructors — that one was
argument twelve among fourteen same-typed neighbours, so every call site was
re-checked by hand rather than trusted to the compiler.

The e2e test asserted the old overwrite contract; it now asserts that an
update preserves the original body and appends the new material.
MSG
```

---

### Task 5: `nomifun-conversation`（含手动型早退门）

**Files:**
- Modify: `crates/backend/nomifun-conversation/src/service.rs:4643-4666,6321-6344,9146-9160,9210-9260,12448,12475-12476,12481-12561,13324`
- Modify: `crates/backend/nomifun-conversation/src/stream_relay.rs:313-322,476`
- Modify: `crates/backend/nomifun-conversation/src/routes.rs:107`
- Modify: `crates/backend/nomifun-conversation/src/convert.rs`
- Modify: `crates/backend/nomifun-conversation/src/service_test.rs`

**Interfaces:**
- Consumes：Task 2 的 `KnowledgeBinding`（无 `writeback_mode`）、`WriteOutcome`（无 `staged`）、`TurnWritebackStatus`。
- Produces：wire 事件 `written[]` 不再含 `staged`；`build_turn_writeback_request` 在 `manual` 下返回 `None`。

- [ ] **Step 1：写失败测试 —— 手动型不触发回合末抽取**

在 `service_test.rs` 按该文件既有构造风格补：

```rust
#[tokio::test]
async fn manual_eagerness_skips_the_turn_final_extractor() {
    let ctx = TestCtx::with_knowledge_mount("manual").await;
    let req = ctx.build_turn_writeback_request_for_last_turn().await;
    assert!(req.is_none(), "manual must not schedule a turn-final extraction");
    assert_eq!(ctx.completer_calls(), 0, "manual must not reach the provider at all");
}

#[tokio::test]
async fn auto_eagerness_still_schedules_the_turn_final_extractor() {
    let ctx = TestCtx::with_knowledge_mount("auto").await;
    let req = ctx.build_turn_writeback_request_for_last_turn().await;
    assert!(req.is_some(), "auto must still schedule a turn-final extraction");
}
```

`TestCtx` 的两个辅助按 `service_test.rs` 既有的会话搭建辅助复用；若不存在等价物，以最小实现补上（建会话 → 建挂载 → 落一轮消息）。

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p nomifun-conversation manual_eagerness_skips_the_turn_final_extractor
```
预期：失败（当前 `manual` 仍返回 `Some`）。

- [ ] **Step 3：加早退门**

按技术方案 §5，在 `build_turn_writeback_request` 的意识读取（`:12522-12526`）之后插入早退。

- [ ] **Step 4：清理 staged 连带**

- `:4643-4666`：删 preset mode → binding mode 映射（`match snapshot.knowledge_policy.mode.as_str()`），eagerness 兜底改 `"manual"`；`knowledge_policy` 已是 4 元组，解构同改。
- `:6321-6344`：删基于持久化 `staged` 标记与 `_inbox/{scope}/` 前缀的重试去重逻辑。**注意**：历史消息 JSON 里仍存着 `staged: true` 与 `_inbox/` 前缀的 `rel_path`，重试路径不得因此 panic —— 改为忽略未知字段而非断言其形状。
- `:12448`、`:12475-12476`、`:13324`：`knowledge_channel_write_enabled` 保留，只清理 `knowledge_writeback_mode` 相关键。
- `stream_relay.rs:476`：删 `written[].staged`；`:313-322` 的 `turn_writeback_status_label` 穷尽 match 随枚举同步。

- [ ] **Step 5：跑测试确认通过**

```bash
cargo test -p nomifun-conversation
```

- [ ] **Step 6：提交**

```bash
git add crates/backend/nomifun-conversation
git commit -F - <<'MSG'
feat(conversation): make the manual disposition actually manual

Eagerness used to change only the extractor prompt's wording while the
turn-final trigger fired on every finished turn, so "restrained" was left to
the model's discretion and still cost a provider call each turn. The gate now
sits in build_turn_writeback_request alongside the four early returns already
there, before the provider call: manual schedules nothing.

knowledge_write stays registered regardless of disposition, which is what
lets "save this to the knowledge base" still work inside a manual turn.

Retry de-duplication no longer keys on the persisted staged flag and the
_inbox prefix. Messages written before this change still carry both, so the
retry path ignores them rather than asserting their shape.
MSG
```

---

### Task 6: `companion` / `terminal` / `preset` / `gateway`

**Files:**
- Modify: `crates/backend/nomifun-companion/src/routes.rs:699-718,709-713`
- Modify: `crates/backend/nomifun-terminal/src/service.rs:1249-1266,4224,4258,4497,4514-4521,4553`
- Modify: `crates/backend/nomifun-preset/src/service.rs:566-570`
- Modify: `crates/backend/nomifun-gateway/src/caps_knowledge.rs:4,105,129-137,370,453-454`
- Modify: `crates/backend/nomifun-gateway/src/caps_knowledge_ext.rs:393-419`

**Interfaces:**
- Consumes：Task 2 的 `KnowledgeBinding`、`resolve_write_policy`；Task 1 的 4 元组 `knowledge_policy`。

- [ ] **Step 1：改写 terminal 的读-改-写保留断言**

`terminal/service.rs:4514-4521` 经由 `writeback_mode` 证明"更新挂载时保留其它字段"，**不能删**，换存活字段（`writeback_eagerness` 或 `channel_write_enabled`）重写：

```rust
// 读-改-写必须保留调用方未指定的字段
assert_eq!(after.writeback_eagerness, "auto", "an unrelated update must not reset the disposition");
assert!(after.channel_write_enabled, "an unrelated update must not reset the channel opt-in");
```

- [ ] **Step 2：跑测试确认失败**

```bash
cargo test -p nomifun-terminal
```
预期：编译失败（`writeback_mode` 已不存在）。

- [ ] **Step 3：改四个 crate**

- `companion/routes.rs:699-718`：删与 `conversation/service.rs:4643-4666` 重复的 mode 映射，eagerness 兜底改 `"manual"`。
- `terminal/service.rs:1249-1266`：`KnowledgeContextOptions` 字面量删 `writeback_mode`/`target_id`；`:4224,4258,4497,4553` 构造清理。
- `preset/service.rs:566-570`：校验值域改 `manual|auto`，错误文案改 `"knowledge eagerness must be manual or auto"`。
- `gateway/caps_knowledge.rs`：`:105,129-137` `SetBindingParams` 删 `writeback_mode` 字段（该结构体是 `#[serde(deny_unknown_fields)]`，旧调用方发该键会被硬拒 —— 这是有意的 wire 破坏）；`:370` 的 `KnowledgeBinding{..Default::default()}` 确认新 `Default` 给出直写符合预期并更新注释；`:4` 的模块注释去掉 staged 描述；`:453-454` 保留 `channel_write_enabled` 赋值。
- `gateway/caps_knowledge_ext.rs:393-419`：删 3 个待审 MCP 能力。

- [ ] **Step 4：跑测试确认通过**

```bash
cargo test -p nomifun-companion -p nomifun-terminal -p nomifun-preset -p nomifun-gateway
```

- [ ] **Step 5：工作区收口**

```bash
cargo check --workspace
```
预期：零错误。有遗漏则在本任务内补齐。

- [ ] **Step 6：提交**

```bash
git add crates/backend/nomifun-companion crates/backend/nomifun-terminal crates/backend/nomifun-preset crates/backend/nomifun-gateway
git commit -F - <<'MSG'
refactor: retire the write-back placement across the remaining surfaces

The preset boundary validated conservative|aggressive independently of the
knowledge service, so it had to move in lockstep or every preset save would
reject the new vocabulary. SetBindingParams denies unknown fields, so
dropping writeback_mode there is a deliberate wire break for external MCP
callers, covered by the contract version bump.

The terminal test that proved read-modify-write preserves untouched binding
fields did so through writeback_mode; it now proves the same invariant
through the disposition and the channel opt-in instead of being deleted.
MSG
```

---

### Task 7: UI wire 层

**Files:**
- Modify: `ui/src/common/adapter/ipcBridge.ts:2856-2866,5427-5428,5476,5483,5508-5522,5779-5782,5929-5993`
- Modify: `ui/src/common/chat/chatLib.ts:112-122,452-463`
- Modify: `ui/src/common/types/agent/presetTypes.ts`
- Modify: `ui/src/renderer/pages/conversation/Messages/components/MessageText.tsx:62-65,203-217`
- Modify: `ui/src/renderer/pages/conversation/Messages/hooks.ts`
- Modify: `ui/src/common/adapter/ipcBridge.preset-wire.test.ts`、`ipcBridge.wire-contract.test.ts`、`apiModelMapper.test.ts`、`chatLib.test.ts`

**Interfaces:**
- Produces：`IKnowledgeBinding` 无 `writeback_mode`；`KnowledgeWritebackEagerness = 'manual' | 'auto'`；`KnowledgeWritebackMode` 类型删除；`IKnowledgeInboxEntry`/`IKnowledgeInboxDiff` 删除；7 个 inbox 方法删除；`pending_inbox` 删除。

- [ ] **Step 1：改类型与 wire**

删 `KnowledgeWritebackMode` 类型与 `IKnowledgeBinding.writeback_mode`；`KnowledgeWritebackEagerness` 改 `'manual' | 'auto'`；删 `pending_inbox`（`:5427-5428`）、两个 inbox 接口（`:5508-5522`）、7 个方法（`:5929-5993`）；`:5476,5483` 注释改写；`:5779-5782` `fromApiKnowledgeBinding` 同步（它不提供默认值，字段增删必须与后端同一版本落地）。

- [ ] **Step 2：三处状态字面量同步收缩**

`ipcBridge.ts:2856-2866`（wire 联合）、`chatLib.ts:112-122`（renderer 联合）+ `:452-463`（运行时 Set 白名单）、`MessageText.tsx:62-65`（渲染分支）—— 四处必须一致。暂存相关状态若有专属成员，一并删除；`disabled`/`no_candidate` 等存活成员保留（手动型下 UI 不渲染回写芯片，因为不产生 `knowledge_writeback` 对象）。

- [ ] **Step 3：跑 typecheck 确认通过**

```bash
bun run typecheck
```
预期：零错误。wire 契约测试若钉住了被删的名字，同步更新。

- [ ] **Step 4：提交**

```bash
git add ui/src/common ui/src/renderer/pages/conversation
git commit -F - <<'MSG'
refactor(ui): take the placement and the inbox off the wire types

fromApiKnowledgeBinding supplies no defaults, so the binding shape has to
change in the same release as the backend or reads silently become undefined.
The write-back status vocabulary is duplicated across the wire union, the
renderer union, its runtime allowlist, and MessageText's render branches; all
four move together.
MSG
```

---

### Task 9: UI 页面

> **依赖：必须在 Task 8（i18n）之后执行。** 本任务引用 `control.eagerness{Manual,Auto}{,Hint}`，这些键由 Task 8 创建；顺序颠倒会让本任务的 typecheck 必然失败。

**Files:**
- Delete: `ui/src/renderer/pages/knowledge/InboxReviewPanel.tsx`
- Modify: `ui/src/renderer/pages/knowledge/useKnowledge.ts`（删 `useKnowledgeInbox`、`useKnowledgeInboxPending`）
- Modify: `ui/src/renderer/pages/knowledge/KnowledgeDetailPage/index.tsx:24,73,77,1391-1408` + 「使用规则」第 3 步
- Modify: `ui/src/renderer/pages/knowledge/KnowledgeCard.tsx`
- Modify: `ui/src/renderer/components/layout/Sider/index.tsx:15,67,235`
- Modify: `ui/src/renderer/pages/conversation/components/KnowledgeControl.tsx:39-40,398-399,600,618,622`
- Modify: `ui/src/renderer/pages/settings/PresetSettings/PresetEditDrawer.tsx:657`
- Modify: `ui/src/renderer/pages/guid/hooks/useGuidAdvancedConfig.ts:70-71`
- Modify: `ui/src/renderer/pages/nomi/workspace/tabs/OtherTab/bundleIo.ts:99-108`
- Modify: `ui/src/renderer/pages/knowledge/knowledgeConsumersUnmount.test.ts:13-14,25-26,36-37`
- **保留**：`ui/src/renderer/components/media/Diff2Html.tsx`（与 `Workspace/components/FileChangeList.tsx:417` 共用）

- [ ] **Step 1：改测试固定值**

`knowledgeConsumersUnmount.test.ts` 的三处 `toEqual` 硬断言含 `writeback_mode: 'direct'` 与 `writeback_eagerness: 'aggressive'`，改为 `writeback_eagerness: 'auto'` 并删 `writeback_mode` 键。

- [ ] **Step 2：跑测试确认失败**

```bash
bun run test:ui
```
预期：类型或断言失败。

- [ ] **Step 3：删待审 UI**

删 `InboxReviewPanel.tsx` 整文件与两个 hook；删详情页 TabPane + Badge（`:1391-1408`）并清理 `:24` `Badge`、`:73` `useKnowledgeInbox`、`:77` `InboxReviewPanel` 三个失用导入（`noUnusedLocals` 会报错）；删 `KnowledgeCard` 角标；删侧边栏红点三处（`Sider/index.tsx:15,67,235`）。

- [ ] **Step 4：改选择器与调用方**

`KnowledgeControl.tsx` 删模式选择整块（`:600` 的 `{ value: 'staged', ... }` 与相邻 direct 项）、意识两项改 `manual`/`auto` 与新 i18n 键；`PresetEditDrawer.tsx:657` 删 staged 选项；`useGuidAdvancedConfig.ts:70-71` 的判定条件改为只比对意识（`kb.writeback_eagerness !== 'manual'`）；`bundleIo.ts:99-108` 的 `setBinding` 载荷删 `writeback_mode`、`writeback_eagerness` 改 `'manual'`；详情页「使用规则」第 3 步从三态（关闭/暂存审阅/直接写入）改为二态说明。

- [ ] **Step 5：跑门确认通过**

```bash
bun run typecheck && bun run test:ui
```

- [ ] **Step 6：提交**

```bash
git add ui/src/renderer
git commit -F - <<'MSG'
refactor(ui): delete the review surface and the placement selector

The inbox panel, its two hooks, the detail-page tab, the card badge, and the
sidebar dot all go. Diff2Html stays — FileChangeList shares it.

Two live callers hardcoded the retired literals and would have started
failing with a 400 about eagerness on unrelated screens: the companion bundle
import and the onboarding wizard's decision about whether to persist a
binding at all.
MSG
```

---

### Task 8: i18n

> **依赖：必须在 Task 9（UI 页面）之前执行**，因为 Task 9 会引用本任务新建的键。本任务自身的 typecheck 门允许"旧键已删、新键尚无引用"的中间态。

**Files:**
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/knowledge.json`
- Modify: `ui/src/renderer/services/i18n/locales/en-US/knowledge.json`
- Regenerate: `ui/src/renderer/services/i18n/i18n-keys.d.ts`（只能由 `bun run gen:i18n` 生成）

- [ ] **Step 1：删键（两个 locale 同步）**

删除：`control.modeStaged`、`control.modeStagedHint`、`control.writebackMode` 及其余模式选项键、`detail.inbox.*`（9 键）、`detail.inboxEmpty`（**注意它是 `detail.inbox` 的兄弟，不是子节点**）、`detail.tabInbox`、`detail.use.writebackStaged`、`detail.use.writebackStagedDesc`、`inbox.*`（11 键）。顺带清理三个本就无调用点的孤儿键：`inbox.empty`、`inbox.scopeLabel`、`inbox.selectProposal`（已含在上面的 `inbox.*` 内）。

- [ ] **Step 2：改键（两个 locale 同步）**

`control.eagernessConservative` → `control.eagernessManual`：zh-CN `'手动型（推荐）'`；`control.eagernessConservativeHint` → `control.eagernessManualHint`：zh-CN `'只有你在对话里明确要求时才回写，模型不会自作主张。'`
`control.eagernessAggressive` → `control.eagernessAuto`：zh-CN `'自动型'`；`control.eagernessAggressiveHint` → `control.eagernessAutoHint`：zh-CN `'模型自主判断：只沉淀确有长期价值、且它有把握的知识，过滤掉琐碎、临时、重复的内容。'`
en-US 给出对应英文；`mount.eagernessLabel`（`'积极程度'`）改为与 `control.writebackEagerness`（`'回写意识'`）一致的措辞。

- [ ] **Step 3：重新生成并校验**

```bash
bun run gen:i18n && bun run check:i18n
```
预期：通过。`localeKeyParity` 对单侧存在硬报错，所以两个 locale 的键集必须完全一致。

- [ ] **Step 4：跑 typecheck**

```bash
bun run typecheck
```
预期：零错误 —— `I18nKey` 联合收窄后，任何仍引用旧键的 `t()` 调用都会在这里暴露。

- [ ] **Step 5：提交**

```bash
git add ui/src/renderer/services/i18n
git commit -F - <<'MSG'
i18n: retire the inbox and placement copy, reword the disposition

detail.inboxEmpty is a sibling of the detail.inbox object rather than a child,
so deleting the object alone would have left it behind. The disposition hints
now describe what the setting does — manual means the model does not write
unless asked — instead of describing a tone.

Regenerated with gen:i18n; the key union is generated from en-US only and
locale parity is a hard error.
MSG
```

---

### Task 10: 文档 + CHANGELOG + 契约版本

**Files:**
- Modify: `README.md:167`、`README.zh-CN.md:166`（EN/ZH 必须同一提交）
- Modify: `docs/guides/companions.md:151-155`、`docs/guides/companions.zh.md:64-66`（EN/ZH 必须同一提交）
- Modify: `CHANGELOG.md`（`## Unreleased` 之后插入）
- Modify: `ui-api-contract-version.txt`（`14` → `15`）
- **不改**：两份 handoff、已发布 CHANGELOG 段落、`STATUS.md`（零命中）、`docs/architecture/data-and-storage*`、`docs/guides/terminal*`、`docs/architecture/external-knowledge-mcp.zh.md`（措辞与落点无关，仍成立）

- [ ] **Step 1：改 README 卖点（EN/ZH）**

现文案宣称"默认暂存到审阅收件箱 + unified-diff 预览 + 合并/丢弃"，全部不再成立。改为描述"手动/自动两种回写意识 + 追加式合并永不覆盖"这一实际保障。

- [ ] **Step 2：改伙伴指南（EN/ZH）**

删两种模式与 `_inbox/` 描述，改为说明回写意识。

- [ ] **Step 3：写 CHANGELOG 条目**

`## Unreleased` 之后，按该文件既有条目风格。必须写明三项用户可见变更：待审移除且残留提案从此当普通文档、存量"保守型"变为"手动型"后不再自动回写、工具路径改为追加式合并因而不再能原地改写整篇文档。

- [ ] **Step 4：升契约版本并重建 UI**

```bash
printf '15\n' > ui-api-contract-version.txt
bun run build:ui
```
预期：构建成功。跳过 `build:ui` 会让 `apps/build-support/ui_build_manifest.rs:80-87` 在后续 `cargo build` 时 panic。

- [ ] **Step 5：跑全部静态门**

```bash
bun run check
```
预期：通过（含 `check:agent-vocabulary`、`check:dead-css`）。

- [ ] **Step 6：提交**

```bash
git add README.md README.zh-CN.md docs/guides/companions.md docs/guides/companions.zh.md CHANGELOG.md ui-api-contract-version.txt ui/dist
git commit -F - <<'MSG'
docs: retire the staged write-back story and bump the UI contract

The README advertised staging into a review inbox with a diff preview as the
default safety story; that story is now append-only merge under
compare-and-swap, so the bullet describes what actually protects a document.

The companions guide held the only user-facing description of the two modes
and of _inbox/, and both language editions move together.

The contract version gates a cached old UI bundle from calling the deleted
inbox routes and posting rejected enum values.
MSG
```

---

### Task 11: 全量验收

- [ ] **Step 1：Rust 全量**

```bash
cargo check --workspace
cargo test -p nomifun-db -p nomifun-knowledge -p nomifun-conversation \
           -p nomifun-ai-agent -p nomi-agent -p nomifun-preset \
           -p nomifun-gateway -p nomifun-terminal -p nomifun-companion
```

- [ ] **Step 2：前端全量**

```bash
bun run check
bun run test:ui
```

- [ ] **Step 3：残留扫描**

```bash
grep -rniE "writeback_mode|_inbox|KB_INBOX|pending_inbox|StagedProposal|StagedBaseSnapshot|writebackMode|IKnowledgeInbox" \
  --include="*.rs" --include="*.ts" --include="*.tsx" --include="*.json" --include="*.sql" \
  crates apps ui/src scripts 2>/dev/null | grep -v node_modules
```
预期：只剩迁移 025 内的历史列名引用。任何其它命中都是遗漏。

```bash
grep -rniE "\bconservative\b|\baggressive\b" --include="*.rs" crates/backend/nomifun-knowledge crates/agent/nomi-agent
```
预期：只剩 `workspace_binding.rs` 的锁保守性（假阳性禁改清单）。

- [ ] **Step 4：行为验证**

调用 `/verify` skill 或按其指引真实驱动一次：建知识库 → 挂到会话并开回写 → 手动型下跑一轮确认**无**回写发生 → 切自动型跑一轮确认回写落入库正文 → 对同一文档再回写一次确认原文完整保留且新材料追加。

- [ ] **Step 5：最终提交（若有修补）**

```bash
git add -A && git commit -m "test: close the residual scan on the write-back simplification"
```

## Self-Review

**Spec coverage**：设计 §1（模式移除）→ Task 2；§2 后端清单 → Task 2/3/4；§3.1 wire → Task 7；§3.2 UI → Task 8；§3.3 i18n → Task 9；§3.4 文档 → Task 10；§4 合并+CAS → Task 2 Step 5-6 + Task 3 Step 3 + Task 4 Step 1；§5 假阳性 → Global Constraints + Task 11 Step 3；§6 迁移 → Task 1；§7 破坏性变更 → Task 10 Step 3；§8 测试 → 各任务 Step 1 + Task 11；§9 验收门 → 各任务末 + Task 11；§10 YAGNI → 无对应任务（有意）。

**类型一致性**：`WriteMode{Disabled,Direct}`（Task 2 产出 → Task 4/6 消费）；`WriteReceipt{final_rel_path,updated}`（Task 3 产出 → Task 4 消费）；`KnowledgeWriteTool::new` 3 参（Task 3 产出 → Task 4 Step 4 消费）；`knowledge_policy` 4 元组（Task 1 产出 → Task 5/6 消费）；`KnowledgeWritebackEagerness = 'manual'|'auto'`（Task 7 产出 → Task 9 消费）；i18n 键 `control.eagerness{Manual,Auto}{,Hint}`（Task 8 产出 → Task 9 消费）。

**顺序修正**：初稿把 i18n 排在 UI 页面之后，会让 UI 任务引用尚不存在的键、typecheck 必然失败。已改为 **Task 8 = i18n，Task 9 = UI 页面**（文档中 Task 9 的章节位置在 Task 8 之前，执行一律按编号）。
