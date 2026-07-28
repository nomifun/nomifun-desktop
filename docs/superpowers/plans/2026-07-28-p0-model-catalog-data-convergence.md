# P0 数据收敛（provider_connections + provider_models）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 per-model 元数据从 providers 行上的 6 个平行 JSON map + model_profiles 表收敛为一张权威 `provider_models` 实体表，并新增 `provider_connections` 连接档案表（承载非默认角色的"域名+鉴权+凭证"），同时修复克隆丢标签、孤儿 profile、health 客户端写三个数据 bug。

**Architecture:** 扩展-收缩迁移：迁移 014 建两张新表并从旧数据回填；repository 层对旧列做同事务双写（单一 choke point，防止托管模型服务等直接写入方造成漂移）；服务层读路径切到新行后，迁移 015 drop `model_profiles`。`providers` 行本身继续充当 `default` 连接（现有 chat/creation/STT/探针消费者零改动），`provider_connections` 只存附加角色（如火山 `voice`）。wire 兼容：`ProviderResponse` 旧字段全部由行投影生成，新增 `models_detail`/`connections` 字段。

**Tech Stack:** Rust (axum + sqlx/SQLite), 迁移 `sqlx::migrate!`, 测试 `cargo test -p <crate>`（无特殊 feature），AES-GCM 加密 `nomifun_common::{encrypt_string,decrypt_string}`。

## Global Constraints

- v3 数据契约：每张产品表 `id INTEGER PRIMARY KEY AUTOINCREMENT`；无物理 FOREIGN KEY/trigger/`*_row_id`；业务 ID 是裸小写 UUIDv7 并带四片段 CHECK（`length(x)=36`、`lower(x)=x`、GLOB `'????????-????-7???-[89ab]???-????????????'`、`replace(x,'-','') NOT GLOB '*[^0-9a-f]*'`）。
- 新表必须登记：`crates/backend/nomifun-db/src/id_schema_contract.rs` 的 `PRODUCT_TABLES`（字母序）+ `LOGICAL_REFERENCES`（`text_ref!` 宏，索引名必须真实存在）；有自有业务 ID 列的表还要登记 `UUIDV7_BUSINESS_COLUMNS` + `NON_REFERENCE_ID_COLUMNS`。否则 `init_database_memory()` 直接失败。
- 已发布迁移（001-013）不可修改；本分支新增 014/015 在同一 PR 内允许彼此配套。
- GitHub Actions 全面禁止（仓库规则）——不创建任何 workflow。
- 提交信息结尾：`Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。
- 每个任务结束运行其 crate 的测试：`cargo test -p nomifun-db`、`cargo test -p nomifun-api-types`、`cargo test -p nomifun-system`；跨 crate 任务追加 `cargo check --workspace --exclude nomifun-desktop`。

## File Structure

- Create: `crates/backend/nomifun-db/migrations/014_provider_models_and_connections.sql`（T1）
- Create: `crates/backend/nomifun-db/migrations/015_drop_model_profiles.sql`（T5）
- Create: `crates/backend/nomifun-db/src/models/provider_model.rs`、`src/models/provider_connection.rs`（T2）
- Create: `crates/backend/nomifun-db/src/repository/provider_model.rs`、`sqlite_provider_model.rs`、`provider_connection.rs`、`sqlite_provider_connection.rs`（T2）
- Delete (T5): `crates/backend/nomifun-db/src/models/model_profile.rs`、`src/repository/model_profile.rs`、`sqlite_model_profile.rs`
- Modify: `nomifun-db/src/id_schema_contract.rs`（T1、T5）、`src/lib.rs`、`models/mod.rs`、`repository/mod.rs`（T2、T5）、`repository/sqlite_provider.rs`（T3）
- Create: `crates/backend/nomifun-api-types/src/provider_model.rs`、`src/provider_connection.rs`（T4、T6）
- Modify: `nomifun-api-types/src/provider.rs`（`ProviderResponse` 加字段）、`src/lib.rs`（T4、T6）
- Modify: `nomifun-system/src/provider.rs`（投影+种子）、`src/model_profile.rs`（改挂新 repo）、`src/routes.rs`（新路由）；Create: `nomifun-system/src/provider_model.rs`、`src/provider_connection.rs`（T4-T9）
- Modify: `nomifun-ai-agent/src/services/provider_health.rs`（repo 换型 + health 回写）（T5）
- Modify: `nomifun-app/src/services.rs`（reconcile 与 repo 字段）、`src/router/state.rs`（state 装配）（T5、T6）

---

### Task 1: 迁移 014 —— 建表 + 回填 + 契约登记

**Files:**
- Create: `crates/backend/nomifun-db/migrations/014_provider_models_and_connections.sql`
- Modify: `crates/backend/nomifun-db/src/id_schema_contract.rs`
- Test: `crates/backend/nomifun-db/tests/provider_models_migration.rs`

**Interfaces:**
- Produces: 表 `provider_models`（UNIQUE(provider_id, model)，列见 DDL）、表 `provider_connections`（UNIQUE(provider_id, role)，业务 ID `connection_id`）。本任务后 `model_profiles` 仍存在（015 才 drop）。

- [ ] **Step 1: 写迁移 SQL**

`crates/backend/nomifun-db/migrations/014_provider_models_and_connections.sql`：

```sql
-- Converge per-model metadata (providers.models + 5 parallel JSON maps +
-- model_profiles) into one authoritative provider_models entity table, and add
-- provider_connections for non-default per-task connection profiles (e.g. a
-- separate voice domain + credential set). The providers row itself remains
-- the 'default' connection in P0; model_profiles is dropped by migration 015
-- after the Rust read path switches.

CREATE TABLE provider_models (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id       TEXT NOT NULL,
    model             TEXT NOT NULL,
    enabled           INTEGER NOT NULL DEFAULT 1,
    sort_order        INTEGER NOT NULL DEFAULT 0,
    tasks             TEXT NOT NULL DEFAULT '[]',
    traits            TEXT NOT NULL DEFAULT '[]',
    protocol          TEXT,
    connection_role   TEXT,
    params            TEXT NOT NULL DEFAULT '{}',
    context_limit     INTEGER,
    description       TEXT,
    source            TEXT NOT NULL DEFAULT 'inferred',
    health            TEXT,
    health_checked_at INTEGER,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    UNIQUE (provider_id, model),
    CHECK (length(provider_id) = 36 AND lower(provider_id) = provider_id AND provider_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE INDEX idx_provider_models_provider_id ON provider_models(provider_id);

CREATE TABLE provider_connections (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_id         TEXT NOT NULL UNIQUE
                          CHECK (length(connection_id) = 36 AND lower(connection_id) = connection_id AND connection_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(connection_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    provider_id           TEXT NOT NULL,
    role                  TEXT NOT NULL,
    label                 TEXT,
    base_url              TEXT NOT NULL,
    auth_scheme           TEXT NOT NULL DEFAULT 'bearer',
    credentials_encrypted TEXT NOT NULL,
    is_full_url           INTEGER NOT NULL DEFAULT 0,
    extra                 TEXT NOT NULL DEFAULT '{}',
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    UNIQUE (provider_id, role),
    CHECK (length(provider_id) = 36 AND lower(provider_id) = provider_id AND provider_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE INDEX idx_provider_connections_provider_id ON provider_connections(provider_id);

-- Backfill: one provider_models row per (provider, catalog model). Profile
-- fields merge from model_profiles when present; per-model map values merge
-- from the providers row's JSON map columns. Orphan model_profiles rows (their
-- model no longer in providers.models) are intentionally NOT migrated — that
-- is the orphan cleanup. Idempotency guard keeps this statement re-runnable.
INSERT INTO provider_models (
    provider_id, model, enabled, sort_order, tasks, traits, protocol, params,
    context_limit, description, source, health, created_at, updated_at
)
SELECT
    p.provider_id,
    je.value,
    COALESCE((SELECT e.value FROM json_each(COALESCE(p.model_enabled, '{}')) e WHERE e.key = je.value), 1),
    je.key,
    COALESCE(mp.tasks, '[]'),
    COALESCE(mp.traits, '[]'),
    (SELECT e.value FROM json_each(COALESCE(p.model_protocols, '{}')) e WHERE e.key = je.value),
    COALESCE(mp.params, '{}'),
    (SELECT e.value FROM json_each(COALESCE(p.model_context_limits, '{}')) e WHERE e.key = je.value),
    (SELECT e.value FROM json_each(COALESCE(p.model_descriptions, '{}')) e WHERE e.key = je.value),
    COALESCE(mp.source, 'inferred'),
    (SELECT json(e.value) FROM json_each(COALESCE(p.model_health, '{}')) e WHERE e.key = je.value),
    CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    COALESCE(mp.updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
FROM providers p
JOIN json_each(p.models) je
LEFT JOIN model_profiles mp
    ON mp.provider_id = p.provider_id AND mp.model = je.value
WHERE NOT EXISTS (
    SELECT 1 FROM provider_models pm
    WHERE pm.provider_id = p.provider_id AND pm.model = je.value
);
```

- [ ] **Step 2: 登记 schema 契约**

`crates/backend/nomifun-db/src/id_schema_contract.rs` 四处修改：

1. `PRODUCT_TABLES`（字母序插入两行，在 `"presets"` 相邻位置按字母序）：

```rust
    "provider_connections",
    "provider_models",
```

2. `LOGICAL_REFERENCES` 中现有 `model_profiles` 条目（L677 附近）**保持不动**（015 才删除），紧邻新增：

```rust
    text_ref!("provider_connections", "provider_id" => "providers", "provider_id", false, "idx_provider_connections_provider_id", Cascade),
    text_ref!("provider_models", "provider_id" => "providers", "provider_id", false, "idx_provider_models_provider_id", Cascade),
```

3. `UUIDV7_BUSINESS_COLUMNS` 新增：

```rust
    ("provider_connections", "connection_id"),
```

4. `NON_REFERENCE_ID_COLUMNS` 新增：

```rust
    ("provider_connections", "connection_id"),
```

- [ ] **Step 3: 写迁移测试（含回填断言）**

`crates/backend/nomifun-db/tests/provider_models_migration.rs`（新文件，用独立 Migrator 手动逐版本 apply 以构造 pre-014 数据）：

```rust
use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const PROVIDER: &str = "0190f5fe-7c00-7a00-8abc-012345678901";

/// Apply migrations up to (and including) `max_version` on a fresh pool.
async fn migrate_to(pool: &sqlx::SqlitePool, max_version: i64) {
    let mut conn = pool.acquire().await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    for m in MIGRATOR.iter() {
        if m.version <= max_version {
            conn.apply(m).await.unwrap();
        }
    }
}

#[tokio::test]
async fn backfill_merges_maps_and_profiles_and_drops_orphans() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate_to(&pool, 13).await;

    // Legacy-shaped provider row: 2 catalog models + per-model maps.
    sqlx::query(
        "INSERT INTO providers (provider_id, platform, name, base_url, api_key_encrypted, models, enabled, capabilities, model_context_limits, model_protocols, model_descriptions, model_enabled, model_health, is_full_url, sort_order, created_at, updated_at) \
         VALUES (?, 'openai', 'P', 'https://x.test/v1', 'enc', ?, 1, '[]', ?, ?, ?, ?, ?, 0, 0, 1, 1)",
    )
    .bind(PROVIDER)
    .bind(r#"["gpt-4o","flux-pro"]"#)
    .bind(r#"{"gpt-4o":128000}"#)
    .bind(r#"{"flux-pro":"anthropic"}"#)
    .bind(r#"{"gpt-4o":"desc"}"#)
    .bind(r#"{"flux-pro":false}"#)
    .bind(r#"{"gpt-4o":{"status":"healthy"}}"#)
    .execute(&pool)
    .await
    .unwrap();
    // Profile for gpt-4o + an ORPHAN profile (model not in catalog).
    sqlx::query(
        "INSERT INTO model_profiles (provider_id, model, tasks, traits, params, source, updated_at) VALUES \
         (?, 'gpt-4o', '[\"chat\"]', '[\"vision_input\"]', '{\"endpoint\":\"/x\"}', 'user', 42), \
         (?, 'ghost-model', '[\"chat\"]', '[]', '{}', 'user', 42)",
    )
    .bind(PROVIDER)
    .bind(PROVIDER)
    .execute(&pool)
    .await
    .unwrap();

    migrate_to(&pool, 14).await;

    let rows: Vec<(String, i64, i64, String, String, Option<String>, String, Option<i64>, Option<String>, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT model, enabled, sort_order, tasks, traits, protocol, params, context_limit, description, source, health, updated_at \
         FROM provider_models WHERE provider_id = ? ORDER BY sort_order",
    )
    .bind(PROVIDER)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2, "orphan profile must not migrate");
    let gpt = &rows[0];
    assert_eq!(gpt.0, "gpt-4o");
    assert_eq!(gpt.1, 1);
    assert_eq!(gpt.3, r#"["chat"]"#);
    assert_eq!(gpt.4, r#"["vision_input"]"#);
    assert_eq!(gpt.6, r#"{"endpoint":"/x"}"#);
    assert_eq!(gpt.7, Some(128000));
    assert_eq!(gpt.8.as_deref(), Some("desc"));
    assert_eq!(gpt.9, "user");
    assert!(gpt.10.as_deref().unwrap_or("").contains("healthy"));
    assert_eq!(gpt.11, 42);
    let flux = &rows[1];
    assert_eq!(flux.0, "flux-pro");
    assert_eq!(flux.1, 0, "model_enabled=false must carry over");
    assert_eq!(flux.5.as_deref(), Some("anthropic"));
    assert_eq!(flux.9, "inferred");
}

#[tokio::test]
async fn fresh_database_passes_schema_contract_with_new_tables() {
    // init_database_memory runs ALL migrations + the id schema contract.
    let db = nomifun_db::init_database_memory().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_connections")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(n, 0);
}
```

- [ ] **Step 4: 运行验证失败→实现→通过**

先只写测试文件、跑 `cargo test -p nomifun-db --test provider_models_migration`（期望：找不到迁移/表而失败），再落 SQL 与契约登记，重跑至绿。随后全量 `cargo test -p nomifun-db`（现有 schema-contract/backup 测试必须仍绿——registry 与 DDL 匹配即绿）。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-db
git commit -m "feat(db): add provider_models and provider_connections tables with backfill (migration 014)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: 行模型 + 仓储（provider_models / provider_connections）

**Files:**
- Create: `crates/backend/nomifun-db/src/models/provider_model.rs`、`src/models/provider_connection.rs`
- Create: `crates/backend/nomifun-db/src/repository/provider_model.rs`、`sqlite_provider_model.rs`、`provider_connection.rs`、`sqlite_provider_connection.rs`
- Modify: `models/mod.rs`、`repository/mod.rs`、`lib.rs`（导出）
- Test: 各 sqlite_* 文件内嵌 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces（后续任务按此签名消费）:

```rust
// models/provider_model.rs
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderModelRow {
    pub id: i64,
    pub provider_id: String,
    pub model: String,
    pub enabled: bool,
    pub sort_order: i64,
    pub tasks: String,           // JSON Vec<ModelTask>
    pub traits: String,          // JSON Vec<ModelTrait>
    pub protocol: Option<String>,
    pub connection_role: Option<String>,
    pub params: String,          // JSON object
    pub context_limit: Option<i64>,
    pub description: Option<String>,
    pub source: String,          // "inferred" | "user"
    pub health: Option<String>,  // JSON ModelHealthStatus
    pub health_checked_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone, Default)]
pub struct NewProviderModel<'a> {
    pub model: &'a str,
    pub enabled: bool,
    pub sort_order: i64,
    pub tasks: &'a str,
    pub traits: &'a str,
    pub protocol: Option<&'a str>,
    pub params: &'a str,
    pub context_limit: Option<i64>,
    pub description: Option<&'a str>,
    pub source: &'a str,
    pub health: Option<&'a str>,
}

/// Partial update; `None` = keep, `Some(None)` = clear (for nullable columns).
#[derive(Debug, Clone, Default)]
pub struct ProviderModelUpdate<'a> {
    pub enabled: Option<bool>,
    pub sort_order: Option<i64>,
    pub tasks: Option<&'a str>,
    pub traits: Option<&'a str>,
    pub protocol: Option<Option<&'a str>>,
    pub connection_role: Option<Option<&'a str>>,
    pub params: Option<&'a str>,
    pub context_limit: Option<Option<i64>>,
    pub description: Option<Option<&'a str>>,
    pub source: Option<&'a str>,
}

// repository/provider_model.rs
#[async_trait::async_trait]
pub trait IProviderModelRepository: Send + Sync {
    async fn list(&self) -> Result<Vec<ProviderModelRow>, DbError>;
    async fn list_for_provider(&self, provider_id: &str) -> Result<Vec<ProviderModelRow>, DbError>;
    async fn get(&self, provider_id: &str, model: &str) -> Result<Option<ProviderModelRow>, DbError>;
    async fn create(&self, provider_id: &str, row: &NewProviderModel<'_>) -> Result<ProviderModelRow, DbError>;
    /// Insert only when (provider_id, model) absent; returns whether inserted.
    async fn insert_if_absent(&self, provider_id: &str, row: &NewProviderModel<'_>) -> Result<bool, DbError>;
    async fn update(&self, provider_id: &str, model: &str, update: &ProviderModelUpdate<'_>) -> Result<ProviderModelRow, DbError>;
    /// Server-side health write (probe outcome). No-op returning false when the row is absent.
    async fn set_health(&self, provider_id: &str, model: &str, health_json: Option<&str>) -> Result<bool, DbError>;
    async fn delete(&self, provider_id: &str, model: &str) -> Result<bool, DbError>;
}

// models/provider_connection.rs
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderConnectionRow {
    pub id: i64,
    pub connection_id: String,
    pub provider_id: String,
    pub role: String,
    pub label: Option<String>,
    pub base_url: String,
    pub auth_scheme: String,
    pub credentials_encrypted: String,
    pub is_full_url: bool,
    pub extra: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone)]
pub struct UpsertProviderConnectionParams<'a> {
    pub role: &'a str,
    pub label: Option<&'a str>,
    pub base_url: &'a str,
    pub auth_scheme: &'a str,
    pub credentials_encrypted: &'a str,
    pub is_full_url: bool,
    pub extra: &'a str,
}

// repository/provider_connection.rs
#[async_trait::async_trait]
pub trait IProviderConnectionRepository: Send + Sync {
    async fn list_for_provider(&self, provider_id: &str) -> Result<Vec<ProviderConnectionRow>, DbError>;
    async fn get(&self, provider_id: &str, role: &str) -> Result<Option<ProviderConnectionRow>, DbError>;
    async fn upsert(&self, provider_id: &str, params: &UpsertProviderConnectionParams<'_>) -> Result<ProviderConnectionRow, DbError>;
    async fn delete(&self, provider_id: &str, role: &str) -> Result<bool, DbError>;
}
```

实现要点（镜像 `sqlite_model_profile.rs` 的既有模式，见其全文）：

- 两个 sqlite 实现都先 `ProviderId::parse` + 父行存在性写锁（`UPDATE providers SET updated_at = updated_at WHERE provider_id = ?`，rows_affected==0 → `DbError::Conflict("... does not exist")`），与 `sqlite_model_profile.rs` upsert 完全同型；
- `create`/`insert_if_absent` 用 `ON CONFLICT(provider_id, model) DO NOTHING`（create 冲突时返回 `DbError::Conflict`）；`update` 用动态 `SET` 拼接（模式参照 `sqlite_provider.rs::update` 的现有写法），`updated_at = now_ms()`；
- `upsert`（connection）生成 `connection_id`：`nomifun_common::ProviderId::new().into_string()` 不对——connection_id 是通用 UUIDv7；查 `nomifun_common` 里裸 UUIDv7 生成函数（`nomifun_common::generate_uuid_v7()` 或同名——以 `rg "pub fn.*uuid" crates/backend/nomifun-common/src` 实际结果为准）；`ON CONFLICT(provider_id, role) DO UPDATE SET`（connection_id 保持不变）。

- [ ] **Step 1: 写失败测试**（每个 sqlite 实现文件内嵌；provider 种子函数照抄 `sqlite_model_profile.rs` tests 的 `seed_provider`）：

```rust
#[tokio::test]
async fn create_get_update_delete_roundtrip() {
    let db = init_database_memory().await.unwrap();
    seed_provider(db.pool(), PROVIDER_1).await;
    let r = SqliteProviderModelRepository::new(db.pool().clone());
    r.create(PROVIDER_1, &NewProviderModel {
        model: "gpt-image-1", enabled: true, sort_order: 0,
        tasks: r#"["image_generation"]"#, traits: "[]", protocol: None,
        params: "{}", context_limit: None, description: None,
        source: "user", health: None,
    }).await.unwrap();
    assert!(!r.insert_if_absent(PROVIDER_1, &NewProviderModel { model: "gpt-image-1", tasks: "[]", traits: "[]", params: "{}", source: "inferred", enabled: true, ..Default::default() }).await.unwrap());
    let row = r.update(PROVIDER_1, "gpt-image-1", &ProviderModelUpdate {
        context_limit: Some(Some(4096)),
        description: Some(Some("img")),
        ..Default::default()
    }).await.unwrap();
    assert_eq!(row.context_limit, Some(4096));
    assert_eq!(row.tasks, r#"["image_generation"]"#, "partial update keeps profile");
    assert!(r.set_health(PROVIDER_1, "gpt-image-1", Some(r#"{"status":"healthy"}"#)).await.unwrap());
    assert!(!r.set_health(PROVIDER_1, "missing", None).await.unwrap());
    assert!(r.delete(PROVIDER_1, "gpt-image-1").await.unwrap());
    assert!(r.get(PROVIDER_1, "gpt-image-1").await.unwrap().is_none());
}

#[tokio::test]
async fn unknown_provider_is_conflict() {
    let db = init_database_memory().await.unwrap();
    let r = SqliteProviderModelRepository::new(db.pool().clone());
    let err = r.create(PROVIDER_2, &NewProviderModel { model: "m", tasks: "[]", traits: "[]", params: "{}", source: "inferred", enabled: true, ..Default::default() }).await.unwrap_err();
    assert!(matches!(err, DbError::Conflict(_)));
}
```

connection 侧同型测试：upsert 两次同 role 不换 connection_id、`list_for_provider` 排序、delete 幂等。

- [ ] **Step 2: 跑测试确认编译失败** → **Step 3: 实现** → **Step 4: `cargo test -p nomifun-db` 全绿**
- [ ] **Step 5: Commit** `feat(db): provider_models and provider_connections repositories`

---

### Task 3: repository 双写 —— providers 写路径同事务同步行

**Files:**
- Modify: `crates/backend/nomifun-db/src/repository/sqlite_provider.rs`
- Test: 同文件测试模块 + `tests/provider_binding_invariants.rs` 不回归

**Interfaces:**
- Consumes: Task 2 的表结构（直接 SQL，不经 trait——同事务）。
- Produces: 语义保证——任何经 `IProviderRepository::create/update` 的写入（包括托管模型服务等直接调用方），`provider_models` 行与 `models`/5 map 列保持一致；`delete` 级联清理两张新表。

行为规范（写在实现注释里）：

1. `create`：插入 providers 行后，在**同一事务**内按 `params.models` JSON 数组为每个模型插入 `provider_models` 行（enabled/protocol/context_limit/description/health 从对应 map 参数取值；tasks/traits='[]'、source='inferred'、sort_order=数组下标）。
2. `update`：若 `params.models` 为 `Some`：数组内新模型插行（同上），不在数组内的现有行删除，保留行的 sort_order 更新为新下标；若某 map 参数为 `Some(...)`：对该 provider 全部行做整-map 替换语义（map 中无该 model 键 → 该列置 NULL/默认，恰与旧 wire 语义一致）。**profile 列（tasks/traits/params/source）永不被双写触碰**。
3. `delete`：在现有 `DELETE FROM model_profiles` 旁边追加：

```rust
        sqlx::query("DELETE FROM provider_models WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM provider_connections WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
```

实现函数：`sync_provider_models_tx(tx, provider_id, models_json, enabled_map, protocols_map, limits_map, descriptions_map, health_map, replace_maps: bool)`——create 与 update 复用；map 解析用 `serde_json::from_str::<HashMap<String, serde_json::Value>>`，容忍 None。

- [ ] **Step 1: 失败测试**（`sqlite_provider.rs` tests 模块新增）：

```rust
#[tokio::test]
async fn create_syncs_provider_model_rows() {
    let (repo, db) = setup().await;
    let p = repo.create(CreateProviderParams {
        provider_id: None, platform: "openai", name: "P",
        base_url: "https://x.test/v1", api_key_encrypted: "enc",
        models: r#"["a","b"]"#, enabled: true, capabilities: "[]",
        model_context_limits: Some(r#"{"a":100}"#),
        model_protocols: None, model_descriptions: None,
        model_enabled: Some(r#"{"b":false}"#), model_health: None,
        bedrock_config: None, is_full_url: false, sort_order: None,
    }).await.unwrap();
    let rows: Vec<(String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT model, enabled, context_limit FROM provider_models WHERE provider_id = ? ORDER BY sort_order")
        .bind(&p.provider_id).fetch_all(db.pool()).await.unwrap();
    assert_eq!(rows, vec![("a".into(), 1, Some(100)), ("b".into(), 0, None)]);
}

#[tokio::test]
async fn update_membership_adds_and_removes_rows_preserving_profiles() {
    let (repo, db) = setup().await;
    let p = repo.create(/* models ["a","b"] 同上，maps None */).await.unwrap();
    // 手工给 a 打上 user profile
    sqlx::query("UPDATE provider_models SET tasks='[\"chat\"]', source='user' WHERE provider_id=? AND model='a'")
        .bind(&p.provider_id).execute(db.pool()).await.unwrap();
    repo.update(&p.provider_id, UpdateProviderParams {
        models: Some(r#"["a","c"]"#), ..Default::default()
    }).await.unwrap();
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT model, tasks, source FROM provider_models WHERE provider_id = ? ORDER BY sort_order")
        .bind(&p.provider_id).fetch_all(db.pool()).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], ("a".into(), r#"["chat"]"#.into(), "user".into()), "existing row's profile untouched");
    assert_eq!(rows[1].0, "c");
}
```

- [ ] **Step 2-4: 红→实现→`cargo test -p nomifun-db` 绿**（注意既有 `deleting_provider_explicitly_cleans_profiles` 等测试必须仍绿）
- [ ] **Step 5: Commit** `feat(db): dual-write provider model rows inside provider create/update/delete transactions`

---

### Task 4: 读路径投影 —— ProviderResponse 由行生成 + models_detail

**Files:**
- Create: `crates/backend/nomifun-api-types/src/provider_model.rs`（`ProviderModelResponse` + `row_to_*` 不放这——api-types 无 db 依赖，转换放 system）
- Modify: `nomifun-api-types/src/provider.rs`（`ProviderResponse` 加 `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub models_detail: Vec<ProviderModelResponse>`）+ `lib.rs` 导出
- Modify: `crates/backend/nomifun-system/src/provider.rs`（`ProviderService` 增加 `provider_model_repo: Arc<dyn IProviderModelRepository>` 字段与构造参数；`list`/`row_to_response` 改为行投影）
- Modify: `crates/backend/nomifun-app/src/router/state.rs`（`build_system_state`/`build_shell_state` 传入新 repo）
- Test: api-types wire 测试 + nomifun-system 服务测试

**Interfaces:**
- Produces:

```rust
// nomifun-api-types/src/provider_model.rs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderModelResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    pub model: String,
    pub enabled: bool,
    pub sort_order: i64,
    pub tasks: Vec<ModelTask>,
    pub traits: Vec<ModelTrait>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_role: Option<String>,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub source: ProfileSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<ModelHealthStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_checked_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}
```

- 投影规则（`ProviderService::row_to_response` 改造，转换函数 `nomifun-system/src/provider_model.rs::row_to_model_response(ProviderModelRow) -> Result<ProviderModelResponse, AppError>`，JSON 解析失败降级空值并 `tracing::warn!`，不让一行坏数据毁掉整个列表——沿用 `row_to_profile` 的既有宽容策略）：
  - `models` = 行按 `(sort_order, id)` 排序后的 `model` 列表；
  - `model_enabled`/`model_protocols`/`model_context_limits`/`model_descriptions`/`model_health` = 行字段非默认值时收进 map（空 map → None，保持旧 wire 的 `skip_serializing_if` 行为）；
  - `models_detail` = 全部行。
  - **providers 表旧列自此不再被读**（仍被 Task 3 双写，P2 收缩期删除）。

- [ ] **Step 1: 失败测试**（nomifun-system `provider.rs` tests：建 provider（走 repo create 带 maps）→ `service.list()` → 断言 models 顺序、maps 内容与 models_detail 与行一致；再断言直接 UPDATE provider_models 行（改 enabled）后 list 反映行的值而非旧列的值——证明读已切换）
- [ ] **Step 2-4: 红→实现→`cargo test -p nomifun-api-types -p nomifun-system` 绿 + `cargo check --workspace --exclude nomifun-desktop`**（所有 `ProviderResponse` 字面量构造点会因新字段编译失败——`models_detail: Vec::new()` 补齐；用 `rg "ProviderResponse \{" crates ui -l` 找全 Rust 侧）
- [ ] **Step 5: Commit** `feat(system): project ProviderResponse from provider_models rows and expose models_detail`

---

### Task 5: model_profiles 退役 —— 消费者切换 + 015 drop + health 服务端回写

**Files:**
- Modify: `crates/backend/nomifun-system/src/model_profile.rs`（`ModelProfileService` 改持 `Arc<dyn IProviderModelRepository>`；`upsert` → 行存在则 `update` profile 字段、不存在则 `create`（enabled=true, sort_order=当前最大+1）；`list` → 行投影为 `ModelProfile`；`seed_missing_inferred` → 对 `tasks == "[]" && source == "inferred"` 的行回填 `derive_tasks_and_traits`，以及 `insert_if_absent` 补缺行）
- Modify: `crates/backend/nomifun-app/src/services.rs`（`model_profile_repo` 字段类型换为 `Arc<dyn IProviderModelRepository>`；`reconcile_model_profiles` 改为遍历 `list_for_provider` 后按上述规则回填）
- Modify: `crates/backend/nomifun-ai-agent/src/services/provider_health.rs`（`model_profile_repo` 换型；`profile` 读取改为 `get` 行 + 解析 tasks/params；`health_check` 成功/失败后调用 `set_health(provider_id, model, Some(json))`，序列化现有 `ModelHealthStatus` 类型 + `health_checked_at` 由 repo 写 now）
- Create: `crates/backend/nomifun-db/migrations/015_drop_model_profiles.sql`：

```sql
-- provider_models (migration 014) is now the authoritative per-model store;
-- every Rust consumer has switched. Remove the superseded table.
DROP INDEX idx_model_profiles_provider_id;
DROP TABLE model_profiles;
```

- Modify: `nomifun-db/src/id_schema_contract.rs`（`PRODUCT_TABLES` 删 `"model_profiles"`；`LOGICAL_REFERENCES` 删其 `text_ref!` 行）
- Delete: `nomifun-db/src/{models/model_profile.rs, repository/model_profile.rs, repository/sqlite_model_profile.rs}` + `mod.rs`/`lib.rs` 导出行；`sqlite_provider.rs` delete 事务里的 `DELETE FROM model_profiles` 语句删除
- Test: 受影响的既有测试全部迁移到新 repo（`sqlite_model_profile` 的测试逻辑移入 `sqlite_provider_model.rs`；`model_profile.rs` 服务测试改用行断言；Task 1 的迁移测试**保持引用 model_profiles**——它验证的是 013→014 的历史行为，014 时点该表存在，测试仍然成立；migrate_to(14) 后表仍在，migrate_to(15) 后才消失，可补一条 `migrate_to(&pool, 15)` 后 `sqlite_schema` 无 model_profiles 的断言）

**Interfaces:**
- Consumes: Task 2 `IProviderModelRepository` 全部方法。
- Produces: `/api/model-profiles*` 端点 wire 行为不变（`ModelProfile` 继续作为投影返回）；`provider_models.health` 只由探针写。

- [ ] **Step 1**: 先改 nomifun-db（删旧文件、015、契约），跑 `cargo check --workspace --exclude nomifun-desktop` 列出全部编译错误清单 → 逐个切换消费者（system → app → ai-agent）
- [ ] **Step 2**: 补 health 回写测试（nomifun-ai-agent 或 system 层：探针路径难以单测网络——对 `set_health` 的调用封装成 `persist_probe_outcome(repo, &response)` 纯函数 + 单测：healthy/unhealthy 两分支序列化正确）
- [ ] **Step 3**: `cargo test -p nomifun-db -p nomifun-system -p nomifun-ai-agent` + `cargo check --workspace --exclude nomifun-desktop` 全绿
- [ ] **Step 4: Commit** `refactor(db,system,ai-agent): retire model_profiles in favor of provider_models (migration 015)`

---

### Task 6: 连接档案服务 + API

**Files:**
- Create: `crates/backend/nomifun-api-types/src/provider_connection.rs` + `lib.rs` 导出：

```rust
/// Response never echoes credentials back; `has_credentials` signals presence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderConnectionResponse {
    pub connection_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub base_url: String,
    pub auth_scheme: String,
    pub has_credentials: bool,
    #[serde(default)]
    pub is_full_url: bool,
    #[serde(default)]
    pub extra: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertProviderConnectionRequest {
    pub role: String,
    #[serde(default)]
    pub label: Option<String>,
    pub base_url: String,
    #[serde(default = "default_bearer")]
    pub auth_scheme: String,
    /// Write-only structured credentials (shape depends on auth_scheme),
    /// encrypted at rest. `None` on update keeps the stored credentials.
    #[serde(default)]
    pub credentials: Option<serde_json::Value>,
    #[serde(default)]
    pub is_full_url: bool,
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
}
fn default_bearer() -> String { "bearer".into() }
```

- Create: `crates/backend/nomifun-system/src/provider_connection.rs`（`ProviderConnectionService { repo, provider_repo, encryption_key }`：`list(provider_id)`（校验 provider 存在）、`upsert(provider_id, req)`、`delete(provider_id, role)`。校验：role 匹配 `^[a-z][a-z0-9_-]{0,31}$` 且 ≠ `"default"`（错误信息："role 'default' is reserved: the provider's own base_url/api_key is the default connection"）；base_url 复用 `validate_base_url` 逻辑（copy 私有函数或提为 pub(crate)）；auth_scheme 非空小写；create 时 credentials 必填非空对象，update 时 None 保留旧值；`serde_json::to_string(credentials)` 后 `encrypt_string`）
- Modify: `crates/backend/nomifun-system/src/routes.rs`：state 加 `provider_connection_service`，路由（注意在 `/api/providers/{provider_id}` 通配之前注册无关；这些带字面后缀，axum 可正确匹配）：

```rust
        .route(
            "/api/providers/{provider_id}/connections",
            get(list_provider_connections).post(upsert_provider_connection),
        )
        .route(
            "/api/providers/{provider_id}/connections/{role}",
            delete(delete_provider_connection),
        )
```

- Modify: `nomifun-app/src/router/state.rs::build_system_state` 装配（`SqliteProviderConnectionRepository::new(pool)` + `services.encryption_key`）
- Test: service 层单测（内存库）：upsert→list 不回读明文且 `has_credentials=true`；同 role 二次 upsert 无 credentials 保留旧密文；role="default" 拒绝；provider 删除后（repo.delete）连接消失（Task 3 已级联，此处断言）

- [ ] **Step 1-4: 红→实现→绿**（`cargo test -p nomifun-api-types -p nomifun-system` + workspace check）
- [ ] **Step 5: Commit** `feat(system): provider connection profiles CRUD with encrypted credentials`

---

### Task 7: 行级模型 API（/api/provider-models）

**Files:**
- Modify: `crates/backend/nomifun-api-types/src/provider_model.rs`（加请求类型）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProviderModelRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    #[serde(default = "crate::provider_model::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub tasks: Vec<ModelTask>,
    #[serde(default)]
    pub traits: Vec<ModelTrait>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub connection_role: Option<String>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    #[serde(default)]
    pub context_limit: Option<i64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: Option<i64>,
}
pub(crate) fn default_true() -> bool { true }

/// Partial update. Nullable fields use double-Option: absent = keep,
/// null = clear, value = set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateProviderModelRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub sort_order: Option<i64>,
    #[serde(default)]
    pub tasks: Option<Vec<ModelTask>>,
    #[serde(default)]
    pub traits: Option<Vec<ModelTrait>>,
    #[serde(default, with = "::serde_with::rust::double_option", skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option", skip_serializing_if = "Option::is_none")]
    pub connection_role: Option<Option<String>>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    #[serde(default, with = "::serde_with::rust::double_option", skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<Option<i64>>,
    #[serde(default, with = "::serde_with::rust::double_option", skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderModelKeyRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
}
```

（若 workspace 无 `serde_with` 依赖，手写 double_option 反序列化函数替代——`rg serde_with Cargo.toml` 确认；无则用 `#[serde(default, deserialize_with = "crate::serde_util::deserialize_double_option")]` 自写通用函数放 serde_util。）

- Create: `crates/backend/nomifun-system/src/provider_model.rs`（`ProviderModelService { repo: Arc<dyn IProviderModelRepository>, provider_repo }`：`list(provider_id: Option<&str>)`、`create(req)`（tasks 为空时用 `derive_tasks_and_traits(platform, model)` 播种、source=user 当 tasks 显式给出否则 inferred；provider 校验存在；同时**把模型名追加进 providers.models 旧列**——经 `IProviderRepository::update` 走 Task 3 双写？不行，那会整表替换。改为：repo 层 create 已插行，旧列由投影生成、legacy 列允许滞后（P0 内旧列仅剩托管服务读——托管服务在 Task 5 已切行读。结论：**不回写旧列**，投影已是唯一读方，双写只为防直接写方漂移，方向是旧→新，不需要新→旧）、`update(req)`（映射 double-Option 到 `ProviderModelUpdate`；tasks/traits 显式更新时 source 置 "user"）、`delete(req)`）
- Modify: `nomifun-system/src/routes.rs`：

```rust
        .route("/api/provider-models", get(list_provider_models).post(create_provider_model))
        .route("/api/provider-models/update", post(update_provider_model))
        .route("/api/provider-models/delete", post(delete_provider_model))
```

`list` 用 `Query<ListProviderModelsQuery>`（`provider_id: Option<String>`）。

- Test: service 单测：create 播种 inferred tasks；update 局部改 description 不动 tasks；`context_limit: Some(None)` 清空；delete 后 `ProviderService.list()` 的 models 数组同步消失（投影一致性）

- [ ] **Step 1-4: 红→实现→绿** → **Step 5: Commit** `feat(system): row-level provider model API (/api/provider-models)`

---

### Task 8: 服务端克隆（修复克隆丢标签）

**Files:**
- Modify: `crates/backend/nomifun-api-types/src/provider.rs`（`CloneProviderResponse` 可直接复用 `ProviderResponse`；无需新类型）
- Modify: `crates/backend/nomifun-system/src/provider.rs`（`ProviderService::clone_provider(&self, id: &str, model_repo, connection_repo) -> Result<ProviderResponse, AppError>`：读源 provider 行 + 全部模型行 + 全部连接行；生成新 provider_id；name 加后缀 `" (副本)"`?——沿用前端现状文案，查 `providerClone.ts` 的命名规则并保持一致（执行时读该文件确认，找不到明确后缀就用 `format!("{name} copy")` 并在 PR 描述注明）；托管平台拒绝（`reject_persisted_managed_provider`）；连接行复制新 `connection_id`）
- Modify: `nomifun-system/src/routes.rs`：`.route("/api/providers/{provider_id}/clone", post(clone_provider))`
- Test: 克隆后新 provider 的 `models_detail` 与源完全一致（除 provider_id/时间戳）、连接行数一致且 connection_id 不同、加密凭证密文原样复制（同一把钥匙，无需重加密）

- [ ] **Step 1-4: 红→实现→绿** → **Step 5: Commit** `feat(system): server-side provider clone preserving model profiles and connections`

---

### Task 9: 播种闭环 —— create/update 后行级 inferred 回填

**Files:**
- Modify: `crates/backend/nomifun-system/src/provider.rs`（`ProviderService::create`/`update` 成功后调用 `seed_inferred_rows(&self.provider_model_repo, &row)`：对该 provider `tasks=="[]" && source=="inferred"` 的行执行 `derive_tasks_and_traits(platform, model)` 回填——复用 Task 5 里 `ModelProfileService::seed_missing_inferred` 的新实现，提取为 `nomifun-system` 内 pub 自由函数 `seed_inferred_provider_models(repo, provider_id, platform) -> Result<usize, AppError>`；boot reconcile 与此复用同一函数）
- Test: create 带 `models: ["step-asr", "gpt-4o"]` → list 后 `models_detail` 中 step-asr 的 tasks 含 `speech_recognition`、gpt-4o 含 `chat`（依赖 `derive_tasks_and_traits` 现有词表——step-asr 命中 ASR 子串表）

- [ ] **Step 1-4: 红→实现→绿** → **Step 5: Commit** `feat(system): seed inferred model profiles on provider create/update`

---

### Task 10: 收尾验证 + 文档 + 交接

- [ ] **Step 1**: `bun run test:crate nomifun-db && bun run test:crate nomifun-system && bun run test:crate nomifun-api-types && bun run test:crate nomifun-ai-agent`（或直接 `cargo test -p ...` 四连）；`cargo test --workspace --exclude nomifun-desktop`（core 全量，接受与本改动无关的既有红灯——记录并核对基线）
- [ ] **Step 2**: `cargo fmt --check` + `cargo clippy -p nomifun-db -p nomifun-system -p nomifun-api-types -- -D warnings`（若仓库无此约定则跳过 clippy 严格模式，`rg clippy package.json scripts/` 确认约定）
- [ ] **Step 3**: 更新 `docs/specs/2026-07-28-multimodal-model-provider-redesign.zh.md` 状态行（P0 已实施，注明"P0 采用增量式连接档案：providers 行 = default 连接"这一与原文的偏差及理由）；新增 `docs/handoffs/2026-07-28-model-catalog-p0.md`（一页：做了什么、两张表、API 变化、旧列冻结状态、P1 入口）
- [ ] **Step 4: Commit** `docs: record P0 model catalog convergence outcome`

## Self-Review 结论

- 覆盖对照：设计文档 P0 四项——建表迁移 ✓(T1)、连接/模型行级 API ✓(T6/T7)、兼容投影 ✓(T4)、三 bug（克隆 ✓T8、孤儿 ✓T1 回填即清理+T3 级联、health 服务端写 ✓T5）、变更时播种 ✓(T9)。
- 有意偏差（已在 T10 文档化）：① default 连接不迁入 provider_connections（增量式，P2 收缩）；② providers 旧 map 列物理保留且继续被双写（防直接写方漂移），P2 删除；③ health 旧 map 列仍接受客户端写（wire 兼容），行上的 health 为服务端权威，P2 切 UI 后关闭旧写路径。
- 类型一致性：`IProviderModelRepository`/`NewProviderModel`/`ProviderModelUpdate` 在 T2 定义、T3(SQL 同步不经 trait)、T4/T5/T7/T9 消费，签名一致；`ProviderModelResponse` T4 定义 T7/T8 复用。
- 执行时待核实点（已内联标注）：UUIDv7 生成函数名（T2）、`serde_with` 是否在 workspace（T7）、克隆命名文案（T8）、clippy 约定（T10）。
