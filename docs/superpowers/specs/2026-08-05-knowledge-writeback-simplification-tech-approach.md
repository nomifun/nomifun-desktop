# 知识库回写简化 · 技术实施方案

> 上游设计：[`2026-08-05-knowledge-writeback-simplification-design.md`](./2026-08-05-knowledge-writeback-simplification-design.md)
>
> 本文只回答"代码级怎么改"：目标签名、逐字文案、完整迁移 SQL、关键路径重写后的代码、执行顺序与每步绿灯门。任务拆分见随后的 plan 文档。

## 1. 目标类型与签名（最终形态）

### 1.1 `nomifun-knowledge/src/context.rs`

```rust
// WritebackMode 整体删除（含 parse 与 doc 注释）

/// 回写意识（"回写意识"）。manual = 只有用户明确要求才回写；
/// auto = 模型自主判断，只写高置信、确有长期价值的知识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritebackEagerness {
    #[default]
    Manual,
    Auto,
}

impl WritebackEagerness {
    /// `None`/`"manual"` → Manual，`"auto"` → Auto。
    /// 未知值落到克制的一侧（Manual）并告警，绝不落到更主动的一侧。
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            None | Some("manual") => Self::Manual,
            Some("auto") => Self::Auto,
            Some(other) => {
                tracing::warn!(
                    writeback_eagerness = other,
                    "unknown writeback_eagerness; falling back to manual"
                );
                Self::Manual
            }
        }
    }
}

pub struct KnowledgeContextOptions<'a> {
    pub format: KnowledgeContextFormat,
    pub writeback: bool,
    pub writeback_eagerness: Option<&'a str>,   // 保留
    pub has_search_tool: bool,
    pub has_write_tool: bool,
    // writeback_mode: 删除
    // target_id:      删除（全仓库唯一用途是 _inbox/{}/ 拼接）
}

fn writeback_contract(options: &KnowledgeContextOptions<'_>, eagerness: WritebackEagerness) -> String
fn eagerness_clause(eagerness: WritebackEagerness) -> &'static str
```

**`target_id` 删除的连带**：三处跨 crate 结构体字面量必须同改 —— `nomifun-ai-agent/src/factory/nomi.rs`、`factory/acp_assembler.rs`、`nomifun-terminal/src/service.rs:1249-1266`。

### 1.2 `nomifun-knowledge/src/service.rs`

```rust
pub enum WriteMode { Disabled, Direct }              // 删除 Staged{scope}

pub struct WriteOutcome {
    pub kb_id: KnowledgeBaseId,
    pub final_rel_path: String,
    pub op: WriteOp,
    // staged: bool 删除
}

// scope 参数删除（它只服务 Staged{scope}）
pub fn resolve_write_policy(surface: WriteSurface, binding: &KnowledgeBinding) -> WritePolicy

pub struct KnowledgeBinding {
    pub enabled: bool,
    pub writeback: bool,
    #[serde(default = "default_writeback_eagerness")]
    pub writeback_eagerness: String,
    #[serde(default)]
    pub channel_write_enabled: bool,   // 注释改：不再是"forced staged"，而是"允许直写"
    #[serde(default)]
    pub kb_ids: Vec<KnowledgeBaseId>,
    // writeback_mode: 删除
}

fn default_writeback_eagerness() -> String { "manual".to_owned() }
// default_writeback_mode 删除

pub const WRITEBACK_EAGERNESS: [&str; 2] = ["manual", "auto"];
// WRITEBACK_MODES 删除

// StagedBaseSnapshot / StagedProposalMetadata / KB_INBOX_REL_DIR 删除
// KnowledgeBaseInfo.pending_inbox 删除
```

`resolve_write_policy` 的目标实现 —— **这是整个重构最容易改错的一处**（现状 `RegularChat|TerminalAcp` 的兜底是 `Staged` 而非 `Direct`，机械替换会顺手让 `ExternalChannel` 丢掉开关）：

```rust
pub fn resolve_write_policy(surface: WriteSurface, binding: &KnowledgeBinding) -> WritePolicy {
    let writeback = binding.enabled && binding.writeback;
    let mode = if !writeback {
        WriteMode::Disabled
    } else {
        match surface {
            WriteSurface::Companion => WriteMode::Direct,
            // 外部 IM 渠道：默认关闭，靠 channel_write_enabled 显式开启。
            // 开启后落点是知识库正文（暂存已移除），安全性由工具写入路径的
            // 追加式合并 + CAS 承担。
            WriteSurface::ExternalChannel => {
                if binding.channel_write_enabled {
                    WriteMode::Direct
                } else {
                    WriteMode::Disabled
                }
            }
            WriteSurface::RegularChat | WriteSurface::TerminalAcp => WriteMode::Direct,
        }
    };
    WritePolicy { mode, allow_create: true, surface }
}
```

### 1.3 `nomi-agent/src/knowledge_tools.rs`

镜像枚举 `WriteMode{Staged{scope},Direct}` 删剩一个变体没有意义 —— **整体删除**，工具不再需要知道落点（落点由服务层强制）：

```rust
// pub enum WriteMode 删除

pub struct WriteRequest {
    pub target: WriteTarget,
    pub content: String,
    pub bound_kb_ids: Vec<KnowledgeBaseId>,
    // mode: WriteMode 删除
}

pub struct WriteReceipt {
    pub final_rel_path: String,
    pub updated: bool,
    // staged: bool 删除
}

pub struct KnowledgeWriteTool {
    sink: Arc<dyn KnowledgeWritebackSink>,
    bases: Vec<(KnowledgeBaseId, String)>,
    bound_kb_ids: Vec<KnowledgeBaseId>,
    // mode: WriteMode 删除
}

impl KnowledgeWriteTool {
    // 入参 4 → 3
    pub fn new(
        sink: Arc<dyn KnowledgeWritebackSink>,
        bases: Vec<(KnowledgeBaseId, String)>,
        bound_kb_ids: Vec<KnowledgeBaseId>,
    ) -> Self
}
```

### 1.4 `nomifun-ai-agent/src/knowledge_writeback.rs`

`TMode` 导入与整段 mode 映射消失，sink 退化为纯目标映射：

```rust
use nomi_agent::knowledge_tools::{
    KnowledgeWritebackSink, WriteReceipt, WriteRequest as TReq, WriteTarget,
};
use nomifun_knowledge::{
    KnowledgeService, WriteMode, WriteOp, WritePolicy, WriteRequest, WriteSurface, WriteTargetSpec,
};

#[async_trait]
impl KnowledgeWritebackSink for LiveKnowledgeWritebackSink {
    async fn write(&self, req: TReq) -> Result<WriteReceipt, String> {
        let spec = match req.target {
            WriteTarget::Handle(h) => WriteTargetSpec::Handle(h),
            WriteTarget::Path { kb_id, rel_path } => WriteTargetSpec::Path { kb_id, rel_path },
        };
        // 落点只有直写一种；surface 在本层是信息性标签。
        let policy = WritePolicy {
            mode: WriteMode::Direct,
            allow_create: true,
            surface: WriteSurface::RegularChat,
        };
        let svc_req = WriteRequest {
            spec,
            content: req.content,
            policy,
            bound_kb_ids: req.bound_kb_ids,
        };
        let out = self.service.write_document(svc_req).await.map_err(|e| e.to_string())?;
        Ok(WriteReceipt {
            final_rel_path: out.final_rel_path,
            updated: matches!(out.op, WriteOp::Update),
        })
    }
}
```

### 1.5 `nomifun-db`

```rust
// models/knowledge.rs — KnowledgeBindingRow
pub writeback_eagerness: String,     // 保留，注释改写为 manual|auto
// pub writeback_mode: String,       删除（含 from_row 的 try_get）

// models/preset.rs:161 — 5 元组降为 4 元组
/// enabled, writeback, eagerness, grounded
pub knowledge_policy: (bool, bool, Option<String>, bool),
```

`repository/sqlite_knowledge.rs` 三处同改：入参 `:277-278` 去掉 `writeback_mode`、UPDATE `:314-321`、INSERT `:335-345`；测试断言 `:712-713`、`:743-744` 改为新值域。
`repository/sqlite_preset.rs:349-352` 的解构与 INSERT 去掉 `mode`。

## 2. 模型面向文案（逐字）

这三段是产品语义本体，必须逐字落地，不可意译。全部英文，遵循 `context.rs:13` 的项目约定。

### 2.1 `context.rs::eagerness_clause`

```rust
fn eagerness_clause(eagerness: WritebackEagerness) -> &'static str {
    match eagerness {
        WritebackEagerness::Manual => {
            "Disposition — MANUAL: the owner drives write-back. Do NOT write to a knowledge \
             base on your own initiative, however useful the material seems. Write only when the \
             user in this conversation explicitly asks you to record, save, or remember something \
             into a knowledge base — then persist exactly what they asked for and nothing more."
        }
        WritebackEagerness::Auto => {
            "Disposition — AUTO: you decide, and the bar is high. Write back only knowledge that \
             is durable, reusable in future sessions, clearly relevant to a mounted base, and that \
             you are confident is correct. Skip anything trivial, session-specific, transient, \
             speculative, or already recorded. Writing nothing is the correct outcome for most \
             turns — a knowledge base earns its value by what it leaves out."
        }
    }
}
```

### 2.2 `context.rs::writeback_contract`（直写唯一落点）

工具分支（`has_write_tool == true`）—— **措辞随 §4 的追加式合并同步改变，这是硬约束**：

```rust
"Write-back is ENABLED: when you produce reusable knowledge (conclusions, domain facts, \
 lessons learned), persist it by CALLING the `knowledge_write` tool — it writes straight into \
 the matching knowledge base. To ADD to an existing document, pass the `handle` from a \
 `knowledge_search` result and put ONLY THE NEW MATERIAL in `content`: the system appends it to \
 the document and silently skips it when it is already there. Never resend a document's existing \
 text, and never try to rewrite, reorder, or shorten a document — you cannot see all of it, so \
 any rewrite risks destroying content. To CREATE a new document, pass `base` plus a descriptive \
 `.md` `rel_path`. Never rebuild paths by hand. Do NOT use the generic Write/Edit file tools for \
 knowledge; never delete files."
```

文件分支（`has_write_tool == false`，裸终端 PTY 与 pre-MCP ACP 会话）：

```rust
"Write-back is ENABLED: when you produce reusable knowledge (conclusions, domain facts, \
 lessons learned), distill it into well-structured markdown inside the matching knowledge base \
 directory — create new files, or append focused additions to existing ones. Never rewrite \
 documents wholesale and never delete files; other sessions may be using the same base \
 concurrently. Keep entries concise, organized, and free of session-specific noise."
```

Disabled 分支不变。

### 2.3 `turn_writeback.rs`

`TURN_WRITEBACK_SYSTEM` 第 28 行 `- rel_path must be ... never under _inbox/.` 改为：

```
- rel_path must be a relative markdown path inside that base, never absolute.
```

并在规则段补一条（配合追加式合并语义）：

```
- content must be ONLY the new material to record, never a rewrite of an existing document.
```

`build_turn_writeback_prompt` 的两个 match（即使 manual 在正常流程下不会走到这里，仍按纵深防御保留两臂）：

```rust
let eagerness_label = match eagerness {
    WritebackEagerness::Manual => "manual",
    WritebackEagerness::Auto => "auto",
};
let eagerness_rule = match eagerness {
    WritebackEagerness::Manual => {
        "Manual: the owner drives write-back. Return an EMPTY candidates array unless the user \
         in THIS turn explicitly asked to record or save something; then extract only that."
    }
    WritebackEagerness::Auto => {
        "Auto: you decide, and the bar is high. Extract only durable, reusable, clearly-correct \
         knowledge. Prefer an empty array over a marginal candidate, and never extract trivia, \
         transient state, or anything the known_paths listing shows is already recorded."
    }
};
```

## 3. 迁移 `025_knowledge_writeback_simplification.sql`

配方已在 SQLite 3.46.1（= `libsqlite3-sys 0.30.1` 内置版本）端到端实测：最终 schema 正确、CHECK 在 `RENAME COLUMN` 后仍生效、部分唯一索引存活、值映射正确、不点名该列的 INSERT 拿到新默认值、旧值被拒。

```sql
-- 暂存回写已移除：回写只有直写一种落点，开关由 knowledge_bindings.writeback 承担，
-- 所以 writeback_mode 整列失意。回写意识的值域从 conservative|aggressive 改为
-- manual|auto，且语义升级为真实的行为差异（manual 不触发回合末自动抽取）。
--
-- SQLite 无法 ALTER 或 DROP 一条 CHECK 约束，因此值域变更走 ADD → UPDATE → DROP →
-- RENAME（模板见 020_channel_owner_domain.sql:14-27）。CHECK 与 DEFAULT 必须同时改：
-- sqlite_conversation.rs 的 INSERT 不点名这些列，依赖 DDL 默认值。
-- 不重建 knowledge_bindings 表：重建必须逐字复现三处 UUIDv7 GLOB CHECK 与四个部分
-- 唯一索引，而 id_schema_contract 在每次打开库时校验索引的规范化 WHERE 谓词文本。

ALTER TABLE knowledge_bindings ADD COLUMN writeback_eagerness_v2 TEXT NOT NULL
    DEFAULT 'manual' CHECK (writeback_eagerness_v2 IN ('manual', 'auto'));

UPDATE knowledge_bindings SET writeback_eagerness_v2 =
    CASE writeback_eagerness WHEN 'aggressive' THEN 'auto' ELSE 'manual' END;

ALTER TABLE knowledge_bindings DROP COLUMN writeback_eagerness;
ALTER TABLE knowledge_bindings RENAME COLUMN writeback_eagerness_v2 TO writeback_eagerness;
ALTER TABLE knowledge_bindings DROP COLUMN writeback_mode;

-- preset_knowledge_policy 带第二条、完全独立的 eagerness CHECK（baseline:1546）；
-- 漏掉它，新值会被永久拒绝。它的 mode 列（baseline:1544，无 CHECK，值域
-- 'inherit'|'staged'|'direct'）在没有"模式"维度后整体失意，一并删除。

ALTER TABLE preset_knowledge_policy ADD COLUMN eagerness_v2 TEXT
    CHECK (eagerness_v2 IS NULL OR eagerness_v2 IN ('manual', 'auto'));

UPDATE preset_knowledge_policy SET eagerness_v2 =
    CASE eagerness
        WHEN 'aggressive' THEN 'auto'
        WHEN 'conservative' THEN 'manual'
        ELSE NULL
    END;

ALTER TABLE preset_knowledge_policy DROP COLUMN eagerness;
ALTER TABLE preset_knowledge_policy RENAME COLUMN eagerness_v2 TO eagerness;
ALTER TABLE preset_knowledge_policy DROP COLUMN mode;
```

`preset_knowledge_policy.eagerness` 可空（`NULL` = 继承），所以 `ELSE NULL` 保留"未指定"语义，不硬塞默认值。

## 4. 工具写入路径统一为追加式合并 + CAS

`write_resolved_document_under_target_lock`（`service.rs:2039-2164`）去掉 `staged_base_snapshot` 参数与整个 staged 分支后：

```rust
async fn write_resolved_document_under_target_lock(
    &self,
    req: WriteRequest,
    res: WriteResolution,
) -> Result<WriteOutcome, AppError> {
    validate_write_request(&req)?;
    validate_canonical_write_target(&res.canonical_rel_path)?;
    let row = self.require_base(res.kb_id.as_str()).await?;
    let _base_guard = self.acquire_base_lifecycle_lock(&row).await?;
    let current = self.require_base(res.kb_id.as_str()).await?;
    if current.root_path != row.root_path {
        return Err(AppError::Conflict(
            "knowledge base root changed while write-back was starting; retry".into(),
        ));
    }
    validate_source_owned_write_target(&current, &res.canonical_rel_path)?;
    drop(_base_guard);
    if res.op == WriteOp::Create && !req.policy.allow_create {
        return Err(AppError::Forbidden(
            "creating new knowledge documents is not allowed for this session".into(),
        ));
    }

    let final_rel_path = res.canonical_rel_path.clone();
    if res.op == WriteOp::Create {
        self.write_file_if_absent(
            res.kb_id.as_str(),
            &final_rel_path,
            &res.canonical_rel_path,
            &req.content,
        )
        .await?;
    } else {
        // 与回合末回写同一条安全路径（原 service.rs:2585-2637）：读取现有内容，
        // 追加式合并，再在 CAS 下发布。绝不无条件覆盖 —— 模型看不到整篇文档，
        // 一次全文重写就可能静默销毁用户精修的内容。
        let existing = self.read_file(res.kb_id.as_str(), &final_rel_path).await?.content;
        let merged = merge_direct_turn_writeback(&existing, &req.content);
        if markdown_identity(&existing) == markdown_identity(&merged) {
            // 该材料已在文档中：幂等无操作，不算失败。
            return Ok(WriteOutcome { kb_id: res.kb_id, final_rel_path, op: res.op });
        }
        self.write_file_if_unchanged(
            res.kb_id.as_str(),
            &final_rel_path,
            &res.canonical_rel_path,
            &existing,
            &merged,
        )
        .await?;
    }
    Ok(WriteOutcome { kb_id: res.kb_id, final_rel_path, op: res.op })
}
```

`write_document`（`service.rs:2035`）的调用点去掉第三个 `None` 实参。

### 4.1 必须同步的工具 schema 与回执

`merge_direct_turn_writeback` 是**追加式**：`contains_markdown_block(existing, proposal)` 判断的是 proposal 是否已在 existing 内。若模型仍按旧提示词"读取→合并→写全文"，proposal ⊇ existing，判否，于是整篇新文档被追加到旧文档后面，**文档翻倍**。因此以下三处必须与 §4 同一提交落地：

1. `knowledge_tools.rs:418-424` 的 `content` 字段描述：`"The FULL markdown content to store (overwrite semantics for updates)"` → `"The new material to record. For an update this is appended to the existing document (already-present material is skipped) — never resend the document's existing text."`
2. `context.rs` 工具分支契约文案（§2.2 已给逐字文本）。
3. `knowledge_tools.rs:476-480` 回执文案中的 `" (STAGED to the review inbox; the user merges it into the base later)"` 整段删除；`WriteReceipt.staged` 消失后回执只区分创建/追加。

### 4.2 有意的行为收窄

工具路径**不再能真正原地改写**已有文档，只能追加。这正是 `merge_direct_turn_writeback` 的既有注释所主张的立场（"a prompt excerpt cannot represent an arbitrarily large file and would make silent truncation/data loss possible"），原先只应用于回合末路径，现在统一。需写入 CHANGELOG 的用户可见变更。

## 5. 手动型早退门

唯一落点：`nomifun-conversation/src/service.rs` 的 `build_turn_writeback_request`（`:12481-12561`），紧接现有的意识读取（`:12522-12526`）之后，与该函数已有的四个早退门（origin 非空 / 用户文本空 / 无挂载 / 回写关闭）并列：

```rust
// 手动型：回写由用户驱动。不做回合末自动抽取，也就不发生 provider 调用。
// 用户在回合内说"记一下"时，仍由已注册的 knowledge_write 工具承担。
if writeback_eagerness == "manual" {
    return None;
}
```

放在 provider 调用**之前**是关键：放在抽取提示词里靠模型自觉，等于每轮白烧一次 LLM 调用，且"手动"不可保证。

`knowledge_write` 工具的注册**不随意识改变**（`manager/nomi/agent.rs:201-206` 的 `should_register_knowledge_write(has_sink, bases)` 不动），这正是手动型下"用户明确要求即可写入"的实现基础。

## 6. 机械删除清单（指针，不重复正文）

| 范围 | 指针 |
|---|---|
| 待审 HTTP 路由 7 条 + 路由内 DTO 4 个 | `nomifun-knowledge/routes.rs:70-88`、`:581-684`；`:579` 段标题改写（`list_consumers` 存活）；`:14-17` 导入 |
| `lib.rs` 再导出 | `:47` 的 `InboxDiff`/`InboxEntry`/`InboxMergeResult`/`KB_INBOX_REL_DIR` |
| service 暂存/待审专属实现 | 15 方法 + 6 自由函数 + 5 类型，约 604 行；`pending_inbox` 两处计算 `:4298-4305`、`:1919-1929` |
| `similar` 依赖 | 删除 `service.rs:7310` 唯一使用者后，摘除 `nomifun-knowledge/Cargo.toml:26` 与工作区 `Cargo.toml:132` |
| `_inbox` 排除过滤器 | `autogen.rs:18,222-226,244` 等 4 处，按设计 §0.2 一并删除（文件留在磁盘，从此当普通文档） |
| 位置参数陷阱 | `manager/nomi/agent.rs:378,393,414` 的 `knowledge_writeback_staged` 是第 12 个位置参数，调用点 `factory/nomi.rs:778` 约 14 个位置参数 —— 删除必须同步，否则同类型相邻参数静默错位 |
| UI | 见设计 §3.2 表 |
| i18n | zh-CN/en-US 各 24 个键；`control.eagerness*` 改写、`control.modeStaged*`/`control.writebackMode`/`detail.inbox*`/`detail.tabInbox`/`inbox.*`/`detail.use.writebackStaged*` 删除；顺带清理三个本就无调用点的孤儿键 |

## 7. 执行顺序与绿灯门

每步结束都有一个可独立运行的门。`nomifun-knowledge` 内部是原子的（删枚举变体会同时打断本 crate 全部 match），所以它是一个不可再分的步骤。

| # | 范围 | 绿灯门 |
|---|---|---|
| 1 | 迁移 025 + `nomifun-db` | `cargo test -p nomifun-db` |
| 2 | `nomifun-knowledge` 全量（枚举/策略/写路径/§4/回合末/routes/lib/autogen/broker/mcp/export/similar） | `cargo test -p nomifun-knowledge` |
| 3 | `nomi-agent` 工具层 | `cargo test -p nomi-agent` |
| 4 | `nomifun-ai-agent` | `cargo test -p nomifun-ai-agent` |
| 5 | `nomifun-conversation`（含 §5 早退门） | `cargo test -p nomifun-conversation` |
| 6 | `nomifun-companion` / `nomifun-terminal` / `nomifun-preset` / `nomifun-gateway` | `cargo test -p` 四个 |
| 7 | 工作区收口 | `cargo check --workspace` |
| 8 | UI wire 层 | `bun run typecheck` |
| 9 | UI 页面 | `bun run typecheck && bun run test:ui` |
| 10 | i18n | `bun run gen:i18n && bun run check:i18n` |
| 11 | 文档 + CHANGELOG + `ui-api-contract-version.txt` 14→15 | `bun run check && bun run build:ui` |
| 12 | 全量 | `cargo test` 相关 crate + `bun run check` + `bun run test:ui` |

`bun run check` 展开为：`typecheck`、`check:i18n`、`check:theme`、`check:icons`、`check:dead-css`、`check:process-runtime-boundary`、`check:browser-platform-boundary`、`check:agent-vocabulary`、`help --check` —— **不含任何 Rust 步骤，也不含 `test:ui`**，两者必须单独跑。

## 8. 新增测试（最小充分集）

1. **迁移**：造含 `('staged','conservative')` 与 `('direct','aggressive')` 的库 → 跑迁移 → 断言映射结果、新 CHECK 生效、旧值被拒、不点名列的 INSERT 得到 `manual`。
2. **`resolve_write_policy` 四表面**：`Companion`→Direct；`ExternalChannel` + `channel_write_enabled=false`→Disabled，`=true`→Direct；`RegularChat`/`TerminalAcp`→Direct；`writeback=false`→Disabled。
3. **手动型早退**：`build_turn_writeback_request` 在 `manual` 下返回 `None`，且**无 provider 调用**发生。
4. **自动型仍触发**：提示词含 `eagerness: auto` 与新规则句，且不含 `_inbox`。
5. **§4 追加式合并**：工具直写 Update 不截断原文（40KB 原文 + 60 字节新材料 → 原文完整保留）；重复提交同一材料为幂等无操作；并发写入命中 CAS 冲突。
6. **`WritebackEagerness::parse`**：`None`/`"manual"`→Manual，`"auto"`→Auto，`"conservative"`/`"AUTO"`/`""`/未知→Manual。

改写而非删除的既有测试（它们保护的不变量与暂存无关）：

- `nomi-agent/knowledge_tools.rs:743-752` `write_by_handle_builds_handle_target` —— 全仓库唯一验证"模型给的 handle 变成 `WriteTarget::Handle`"，改直写版本。
- `nomifun-terminal/service.rs:4514-4521` —— 经由 `writeback_mode` 证明"读-改-写保留其它字段"，换存活字段重写。
- `nomifun-ai-agent/factory/nomi.rs:2472-2492` —— `channel_write_enabled` opt-in 存活性回归，与 mode 断言解耦后保留。
- `nomifun-ai-agent/tests/knowledge_writeback_e2e.rs:117-121` —— 现断言"直写 Update 覆盖原文"，改断言追加式合并。
