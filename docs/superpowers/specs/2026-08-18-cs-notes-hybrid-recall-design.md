# 客服笔记检索重构设计：从 LIKE 到分层混合召回

> 状态：**已实现**（2026-08-18）。所有关键结论均已在本仓库代码与真实 SQLite
> 上实测验证，验证方法与结果见文末「附录 A：实测记录」。
>
> 实际落地与本文的差异见文末「附录 B：交付记录」。

## 1. 根因：不是索引不够好，是查询没有被切分

现状 `crates/backend/nomifun-db/src/repository/sqlite_customer_service.rs:421-443`：

```rust
let escaped = query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
let pattern = format!("%{escaped}%");
// SELECT {NOTE_COLUMNS} FROM cs_notes
//  WHERE (cs_agent_id = ? OR cs_agent_id IS NULL) AND enabled = 1
//    AND content LIKE ? ESCAPE '\\'
//  ORDER BY created_at DESC, id DESC LIMIT ?
```

三个独立缺陷，缺一不可修：

1. **单一连续子串**。模型生成的整个 query 必须作为一个不间断的字面量出现在笔记里。
2. **零归一化**。`nomifun`/`NomiFun`/`ＮomiFun` 视作不同串（SQLite 的 `LIKE`
   仅对 ASCII 大小写不敏感，对 CJK 与全角字符无效）。
3. **按时间排序**。`ORDER BY created_at DESC` —— 即便命中，相关性也不参与排序，
   `LIMIT 10` 会裁掉最相关的笔记。

`tools.rs:119-147` 把模型的原始单串直接透传，输入 schema 是 `{query: string}`
（`tools.rs:22-28`）。空结果只回一句 `"没有找到匹配的客服笔记。"`，不给模型任何
下一步指引。

### 关键结论：换成 FTS5 本身并不能修好

这一点最反直觉，必须先说清楚。把模型的原串当成一个 FTS5 phrase 去 MATCH，
故障**原样复现**（实测，SQLite 3.50.4）：

| MATCH 表达式 | 结果 |
|---|---|
| `'"NomiFun是什么"'` | HIT |
| `'"NomiFun 是什么"'` | **MISS** ← 空格照样致命 |
| `'"介绍一下 NomiFun"'` | **MISS** |

真正的修复是 **查询侧切分 + 归一化 + OR 组合 + 相关性排序**。FTS5 只是让这件事
可以高效且带 BM25 地做。**因此本设计的重心在查询管道，索引是配套设施。**

## 2. 架构：一条归一化管道 + 四级召回阶梯

```
用户消息 "@xxx  NomiFun 是什么"
   │
   ├─ normalize()      NFKC + lowercase（索引侧与查询侧用同一个函数）
   ├─ tokenize()       去 @mention → 标点/空白切分 → CJK/ASCII 分段
   │                   → 去疑问停用词 → 剥句末语气助词 → 长 CJK 段切叠字 n-gram
   └─ 分级检索
        L1  FTS5 trigram MATCH（≥3 字词，OR 组合，BM25 排序）
        L2  LIKE 子串回退（2 字词/短 ASCII，trigram 索引不到）
        L3  CJK bigram 兜底（仅当 L1+L2 全空；低精度，必须限量并按重叠度排序）
        L4  真未命中 → 返回可用笔记主题清单，引导模型改写重查
```

### 2.1 归一化（索引侧与查询侧必须完全一致）

`unicode-normalization = "0.1"` 已是 workspace 依赖（`Cargo.toml:220`，
`nomifun-knowledge` 已在用）。

```rust
/// NFKC + 小写折叠。索引与查询共用，二者必须永远调用同一函数。
/// NFKC 负责全角→半角（Ｎ→N、？→?、：→:），lowercase 负责大小写。
pub fn normalize_for_search(input: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    input.nfkc().collect::<String>().to_lowercase()
}
```

### 2.2 切分（新模块 `crates/backend/nomifun-customer-service/src/note_query.rs`）

```rust
/// trigram 分词器无法 MATCH 少于 3 字符的词（与 memory_search.rs 同一约束）。
const TRIGRAM_MIN_CHARS: usize = 3;
/// 单次查询最多展开的词数，防止长句 n-gram 爆炸。
const MAX_TERMS: usize = 24;
/// L3 兜底最多返回的候选数。
const BIGRAM_FALLBACK_LIMIT: usize = 5;

/// 一次查询展开后的三条通道。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteQueryTerms {
    /// ≥3 字符：走 FTS5 MATCH。
    pub fts: Vec<String>,
    /// 2 字符或短 ASCII：走 LIKE 子串。
    pub like: Vec<String>,
    /// CJK bigram：仅 L3 兜底使用。
    pub bigrams: Vec<String>,
}

/// 把一段自然语言展开成检索词。输入可以是模型给的 query，也可以是访客原文。
pub fn expand_query(raw: &str) -> NoteQueryTerms;
```

切分规则（逐条都有实测依据）：

- **去前缀 @mention**：`^@\S+\s*`。渠道消息形如 `"@客服 NomiFun是什么"`，
  mention 是噪声词。
- **按标点与空白切分**，含全角 `，。？！：；、（）【】`。
- **CJK / ASCII 分段**：`[a-z0-9_]+` 与 `[^\x00-\x7f]+` 分别成词。这一步让
  `"NomiFun是什么"` 产出 `["nomifun", "是什么"]` —— **这是四个报告用例的核心修复点**。
- **去疑问停用词**：`是什么 / 介绍一下 / 怎么 / 如何 / 请问 / 哪些 / what / how / is`…
  它们在任何 FAQ 里都出现，是纯噪声。
- **剥句末语气助词**：`(吗|呢|吧|啊|的|了|么)+$`。`"免费吗"` → 同时保留
  `免费吗` 与词干 `免费`，后者才能命中 `"开源免费"`。
- **长 CJK 段切叠字 trigram**：`"能干什么"` → `["能干什", "干什么"]`。
- **总量截断到 `MAX_TERMS`**，按词长降序保留（长词更具区分度）。

### 2.3 排序

复用 `memory_search.rs` 的既有做法：`bm25()` 越小越好，取负转成越大越好；
多词 OR 结果按 note 去重，取各词中的最优分；tie-break 链以 id 结尾保证确定性。

```rust
/// 融合排序：BM25 相关性为主，kind 与新鲜度为轻量加权。
/// L2/L3 命中无 BM25，记 0.0（与 memory_search.rs 对 LIKE 回退的处理一致）。
fn fused_rank(bm25: f64, kind: &str, matched_terms: usize) -> f64 {
    -bm25
        + if kind == "faq" { 0.5 } else { 0.0 }
        + (matched_terms as f64) * 0.25
}
```

## 3. 数据模型：`search_text` + 运营可维护的别名

同义改写是纯词汇手段**证明性地**做不到的一类（实测：`"这个软件是干什么的"`
与笔记零词汇重叠）。本仓库**没有**任何离线向量能力（见 §6），所以答案是
**运营编写的别名**，而不是学出来的向量。

迁移 `035_cs_notes_search.sql`：

```sql
-- 归一化后的检索文本：content + aliases 的 NFKC+lowercase 结果。
-- 由写路径维护（v3 禁止触发器），是 FTS5 的 external-content 来源列。
ALTER TABLE cs_notes ADD COLUMN search_text TEXT NOT NULL DEFAULT '';
-- 运营手写的同义问法，换行分隔。这是同义召回唯一可审计、可纠正的入口。
ALTER TABLE cs_notes ADD COLUMN aliases TEXT NOT NULL DEFAULT '';

CREATE VIRTUAL TABLE cs_notes_fts USING fts5(
  search_text, content='cs_notes', content_rowid='id', tokenize='trigram'
);
```

`search_text` 存在的理由：FTS5 的 trigram 分词器对索引内容只做有限折叠，
而我们需要 NFKC 全角折叠。把归一化结果**物化**成一列，索引它，是唯一能保证
「索引侧与查询侧归一化完全一致」的方式，同时也让 L2 的 `LIKE` 直接查这一列
（而不是查原始 `content`，否则 L2 会漏掉全角/大小写变体）。

**回填只需一条语句**（实测有效）：

```sql
UPDATE cs_notes SET search_text = lower(content);  -- Rust 侧随后做一次 NFKC 重写
INSERT INTO cs_notes_fts(cs_notes_fts) VALUES('rebuild');
```

`'rebuild'` 会从内容表整体重建索引，无需逐行 SQL。SQL 的 `lower()` 不处理
CJK 全角，所以迁移后需由启动期一次性任务用 `normalize_for_search` 重写
`search_text` 再 `rebuild`；或者更简单：迁移只建表，`search_text` 的首次填充
与 `rebuild` 都放在 Rust 侧的一次性 backfill 中，避免 SQL/Rust 归一化语义分叉。
**推荐后者**，理由是单一归一化实现。

## 4. 索引维护：v3 禁止触发器，必须在写路径手工维护

`nomifun-db` 的 v3 契约禁止未注册触发器（`tests/id_schema_contract.rs:367-432`）。
FTS5 本身**不创建任何触发器**（实测：`type='trigger'` 计数为 0），所以合法，
但索引同步必须由 `create_note` / `update_note` / `delete_note` 亲自完成，
照抄 `nomifun-companion/src/store.rs:709-760` 的既有范式：

```rust
/// external-content FTS5 维护：索引一行。
async fn fts_index_insert<'e, E>(executor: E, rowid: i64, search_text: &str) -> Result<(), DbError>
where E: sqlx::Executor<'e, Database = sqlx::Sqlite>;
// INSERT INTO cs_notes_fts(rowid, search_text) VALUES(?, ?)

/// external-content FTS5 维护：删除一行。
/// `old_search_text` 必须是**当初被索引的那个值**（fts5 'delete' 命令契约）。
async fn fts_index_delete<'e, E>(executor: E, rowid: i64, old_search_text: &str) -> Result<(), DbError>
where E: sqlx::Executor<'e, Database = sqlx::Sqlite>;
// INSERT INTO cs_notes_fts(cs_notes_fts, rowid, search_text) VALUES('delete', ?, ?)
```

### ⚠️ 最危险的坑（实测确认，务必写进注释）

`update_note` 必须**先读旧 `search_text`，再 delete(旧)，再 UPDATE 行，再 insert(新)**，
全程同一事务。若给 `'delete'` 传了新值：

- SQLite 抛 `database disk image is malformed`；
- **但随后的 `'integrity-check'` 仍然报 PASSED** —— 损坏静默存在；
- 后果是该笔记从索引里消失，即「笔记明明存在却搜不到」，**与当前这个 bug 一模一样**。

逃生舱：`INSERT INTO cs_notes_fts(cs_notes_fts) VALUES('rebuild')` 可整体修复
（实测有效），应作为一个运维命令暴露出来。

## 5. Schema 契约必须同步改动（本设计最大的落地风险）

`cs_notes` 位于 `nomifun-db`，而该库有**集合相等**的表注册表；
`companion_memories` 在**另一个数据库**（`companion_dir.join("memory.db")`，
`store.rs:1864`），有自己独立的校验器 —— 所以 companion 的 FTS 先例
**不会自动**在 `nomifun-db` 合法。

实测：加入 `cs_notes_fts` 后契约查询返回 6 张表，其中 4 张新影子表
**违反** `id INTEGER PRIMARY KEY AUTOINCREMENT` 断言：

```
pass  cs_notes
FAIL  cs_notes_fts         pk count=0
FAIL  cs_notes_fts_config  pk name='k'（且 WITHOUT ROWID）
FAIL  cs_notes_fts_data    缺 AUTOINCREMENT 声明
FAIL  cs_notes_fts_docsize 缺 AUTOINCREMENT 声明
FAIL  cs_notes_fts_idx     pk count=2（复合 segid,term）
```

好消息：外键与 `FOREIGN KEY`/`ON DELETE`/`ON UPDATE` token 扫描、
触发器白名单三项**均不受影响**（实测 pass）。

必须精确修改的四处：

| 文件 | 位置 | 改动 |
|---|---|---|
| `nomifun-db/src/id_schema_contract.rs` | `PRODUCT_TABLES`（:25-50 区段） | 新增 `cs_notes_fts` 常量 + `CS_NOTES_FTS_SHADOW_TABLES` 数组 |
| 同上 | `validate_id_schema_contract` :891-905 | 期望集合 `.chain(once(FTS_TABLE)).chain(SHADOW.iter().copied())`，照抄 companion `store.rs:1386-1390` |
| 同上 | :908 `for table in PRODUCT_TABLES` 循环 | 结构断言只遍历真实产品表，**跳过** FTS 及影子表 |
| `nomifun-db/tests/id_schema_contract.rs` | `EXPECTED_PRODUCT_TABLES` :114；断言 :201/:228/:327/:367 | 同样加入并豁免；影子表形状由 SQLite 拥有，不承担本仓库的行键契约 |

建议同时补一个 `validate_cs_notes_fts_contract`，仿
`store.rs:1318-1344`，断言建表 SQL 含
`content='cs_notes'` / `content_rowid='id'` / `tokenize='trigram'`，
防止后人误改分词器而静默降级召回。

## 6. 语义通道：诚实结论 —— 现在不做，但留好接缝

调查结果（实测）：

- `KnowledgeEmbeddingConfig::Local {}` **不是**本地向量模型，它走
  `local_keyword_candidates`（`service.rs:3389-3390`, `:3873`），是纯关键词打分。
  「local」在本仓库语义里 = 无向量。
- **没有**任何离线 embedding / 中文分词 crate（`fastembed`/`candle`/`jieba`/
  `charabia`/`tantivy` 均不在 `Cargo.lock`）。
- 远程通道每次查询都要重算文档向量（`remote_embedding_candidates`
  `service.rs:3417+`，带 `REMOTE_RETRIEVAL_MAX_DOCUMENTS` 等上限正是为此），
  且需运营配置 provider。

因此：**本期不引入向量通道。** 理由不是懒，而是它违反「离线零配置必须可用」
这条硬约束，且会把网络往返放到客服实时回复的关键路径上。同义召回由 §3 的
`aliases` 列承担 —— 可审计、可纠正、零延迟、不漂移，而且运营本来就最清楚
自家产品的用户叫法。

留下的接缝：`NoteSearchHit` 带 `rank: f64`，检索层是单一入口函数，
未来加 RRF 融合的第二通道只需在该函数内多一个 channel，不改调用方。

## 7. 工具契约：调用方是模型，契约本身就是设计面

`one_shot.rs:33` 有 `MAX_TOOL_ROUNDS: usize = 8` —— **模型本来就能重查，
只是没人告诉它要重查**。这让空结果文案成为一个真实的召回杠杆。

```rust
fn query_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "queries": {
                "type": "array",
                "items": { "type": "string" },
                "description": "1-5 个关键词或问法；不要传整句，多个改写会自动 OR 合并"
            }
        },
        "required": ["queries"]
    })
}
```

数组入参在本仓库有先例（`nomifun-gateway/src/registry/capability.rs:919`），
且 `MemorySearchQuery.queries` 早已是 `Vec<String>`。工具描述改为明确指导：
「传关键词而非整句；命中为空时换更短的关键词重试」。

空结果不再是一句死话，而是回退到 L4：列出该客服可用笔记的主题清单（每条取
首行 Q 或前 N 字），让模型据此改写重查。**兼容性**：保留 `query: string`
作为 deprecated 别名并在内部归一到 `queries`，避免旧 prompt 直接失效。

### 另加一道保险：turn 级预检索

`dialogue.rs:206` 处 `user_text` 已就绪，`:219` 紧接着 `build_cs_tools` ——
访客原文与工具构造在同一作用域。建议在此对**访客原文**做一次 `expand_query`
预检索，把 top-N 命中注入 system prompt。这样即使模型生成了糟糕的 query，
已存在的 FAQ 也不会丢。同时 `build_system_prompt`（`dialogue.rs:278-298`）
目前对检索策略**零指导**，应补充「先用短关键词搜，空了就换词再搜」。

## 8. 需要删除的遗留（不保留隐患）

- `sqlite_customer_service.rs:421-443` 原 `search_notes` 的 `LIKE '%q%'` 实现
  与 `ORDER BY created_at DESC` —— **整体删除**，不做并存。
- `ICustomerServiceRepository::search_notes` 的 `query: &str` 签名
  （`customer_service.rs:140-146`）→ 换成 `&NoteQueryTerms` 或 `&[String]`，
  并返回带 `rank` 的 `Vec<NoteSearchHit>` 而非裸 `Vec<CsNoteRow>`。
  旧签名不保留，避免双入口。
- `tools.rs:118` 的注释 `MVP uses LIKE, no FTS` —— 连同实现一起失效，删。
- 顺带修正一处同类缺陷：`nomifun-knowledge` 的 `query_terms`
  （`service.rs:5767-5775`）**只按空白切分**，对中文等价于整句匹配 ——
  `knowledge_search` 存在同一 bug 类。本设计的 `expand_query` 应提升为共享
  工具（放 `nomifun-common` 或新 `nomifun-text-search`），两个检索面共用，
  否则就是把同一 bug 留了一份。

## 9. 测试计划（确定性、无网络）

评测表就是回归护栏 —— 当前实现会在**前四行**直接失败。
实测 19/20 通过（附录 A）。

| 输入 | 期望 | 命中机制 |
|---|---|---|
| `@xxx  NomiFun是什么` | note 1 | CJK/ASCII 分段 → `nomifun` |
| `@xxx  NomiFun 是什么` | note 1 | 同上；空格不再致命 |
| `@xxx  nomifun是什么` | note 1 | lowercase 归一化 |
| `@xxx 介绍一下 NomiFun` | note 1 | 去停用词 `介绍一下` → `nomifun` |
| `@xxx ＮomiFun是什么？` | note 1 | NFKC 全角折叠 |
| `@xxx NOMIFUN 能干什么？` | note 1 | lowercase + trigram `能干什`/`干什么` |
| `@xxx 这个软件是干什么的` | note 1 | **aliases** 列（同义，词汇零重叠） |
| `@xxx AI 工作空间` | note 1 | `工作空间` trigram |
| `@xxx what is nomifun` | note 1 | 英文停用词 `is`/`what` 剔除 |
| `@xxx 怎么安装` / `安装包在哪下载` | note 2 | trigram `怎么安`/`安装包` |
| `@xxx 免费吗` | note 3 | 语气助词剥离 → `免费` |
| `@xxx 要钱吗` / `多少钱？` | note 3 | **aliases** 列 |
| `@xxx 访客很生气怎么办` | note 4 | **L3 bigram 兜底**（`访客`，2 字低于 trigram 底线） |
| `@xxx 完全无关的问题zzz` | 空 | 正确的未命中 |
| `@xxx `（空）/ `@xxx AND` / `@xxx " OR *` | 空且不 panic | FTS5 语法注入防护 |

最后三行不是凑数：实测原始输入直送 MATCH 会抛
`fts5: syntax error near "AND"`、`unterminated string`、`unknown special query`。
**每个词都必须过 `fts_phrase()` 式转义**（`"` → `""` 并整体加引号），
照抄 `memory_search.rs:78-80`。

其他必测项：

- **FTS 同步不变量**：create/update/delete 后，`cs_notes_fts_docsize` 行数
  与 `cs_notes` 一致；改内容后旧词搜不到、新词搜得到。
- **`update_note` 的 delete 契约**：断言走的是「读旧值→delete(旧)→UPDATE→insert(新)」，
  这是静默索引损坏的唯一防线。
- **可见性**：`cs_agent_id IS NULL`（共享）与 `enabled = 1` 过滤在新检索路径
  上仍然成立 —— 换索引最容易顺手丢掉的就是这两条。
- **schema 契约**：`validate_id_schema_contract` 在含 FTS 的库上通过。

## 10. 实施顺序

1. `note_query.rs`：`normalize_for_search` + `expand_query` + 纯函数单测。
   无 DB 依赖，评测表可先跑起来。
2. 迁移 `035` + schema 契约四处豁免 + `validate_cs_notes_fts_contract`。
   先让 `cargo test -p nomifun-db` 绿。
3. 仓储层：FTS 维护函数、三个写路径接线、`search_notes` 换签名、
   L1–L3 检索与融合排序。
4. 一次性 backfill（`search_text` 重写 + `'rebuild'`）。
5. 工具契约：`queries` 数组、描述改写、L4 主题清单、prompt 检索指导。
6. 预检索注入 `dialogue.rs`。
7. 把 `expand_query` 提升为共享工具，修 `knowledge_search` 的同类缺陷。
8. api-types / TS 导出 / `ui-api-contract-version.txt` / i18n：仅当第 5、6 步
   改动了对外类型或用户可见文案时才涉及；`aliases` 字段进 UI 需要编辑器支持。

## 11. 已决策（原为待定，实现时一并落地）

1. **`aliases` 的 UI**：已做。`CsAgentDetailPage` 笔记编辑弹窗新增「其他问法」
   多行输入框 + 说明文案，中英文案齐备。
2. **预检索**：已做。见 §7 末与附录 B。
3. **`knowledge_search` 的同类修复**：**未做，故意留下**。见附录 B「未做的部分」。

---

## 附录 A：实测记录

环境：Python `sqlite3` 3.50.4（与仓库 `libsqlite3-sys 0.30.1` bundled 同代）。
仓库侧 FTS5 trigram 已在 `nomifun-companion` 生产使用，故能力可用性无疑。

1. **朴素 FTS5 复现原 bug**：整串 phrase MATCH 时
   `"NomiFun 是什么"`、`"介绍一下 NomiFun"` 均 MISS（§1 表格）。
2. **完整管道 19/20 通过**：唯一失败项 `"访客很生气怎么办"` 促成了 L3 bigram
   兜底的设计；并同时实测确认 bigram 单独使用**精度很低**（`怎么` 这类高频
   二字词会命中大量无关笔记），故必须限定为「仅在高层级全空时启用、限量、
   按重叠度排序」，而非与 L1/L2 平级合并。
3. **`'delete'` 契约**：传错内容 → `database disk image is malformed`，
   而 `'integrity-check'` 仍报 PASSED（静默损坏）。
4. **`'rebuild'` 有效**：可从内容表整体重建，既是迁移回填手段也是修复手段。
5. **契约断言逐条模拟**：4 张影子表违反行键断言，外键/token/触发器三项 pass。
6. **FTS5 语法注入**：`AND`、`a OR`、`"`、`*`、`(`、`收费吗?`、`a-b`
   作为原始 MATCH 输入全部抛错 —— 转义为强制项。

---

## 附录 B：交付记录（2026-08-18）

### 落地文件

| 文件 | 内容 |
|---|---|
| `crates/backend/nomifun-common/src/text_search.rs` | 新增。`normalize_for_search`（NFKC+小写）、`expand_query`（切分成 fts/like/bigrams 三通道）、`fts_phrase`/`like_pattern` 转义。13 个单测 |
| `crates/backend/nomifun-db/migrations/035_cs_notes_hybrid_recall.sql` | 新增。`search_text` + `aliases` 两列 + `cs_notes_fts` 虚表 |
| `crates/backend/nomifun-db/src/repository/customer_service_search.rs` | 新增。三级召回阶梯、BM25 融合排序、FTS 维护helper、`backfill_note_search_text`、`list_note_topics` |
| `id_schema_contract.rs`（src + tests） | 影子表登记进表集合 + 豁免行键断言；新增 `validate_cs_notes_fts_contract` 防止分词器被误改 |
| `sqlite_customer_service.rs` | 三个写路径接线索引（单事务）；`search_notes` 换签名；删除旧 LIKE 实现 |
| `tools.rs` | `queries` 数组契约、工具描述改写、空结果返回主题清单 |
| `dialogue.rs` | 预检索注入 + 系统提示补检索策略 |
| UI / i18n | `aliases` 编辑框、`ipcBridge` 类型与请求体、中英文案、`i18n-keys.d.ts` 重新生成 |

### 与设计的差异

- **`expand_query` 放在 `nomifun-common` 而非 CS crate**：`nomifun-db` 也需要它
  （仓储层做检索），而 db 不能依赖 CS crate。放 common 同时为后续复用留口。
- **bigram 通道的实现比设计更严**：设计说「限量并按重叠度排序」，实现是
  `acc.is_empty()` 才启用 + `BIGRAM_FALLBACK_LIMIT = 5` + 按重叠 bigram 数排序，
  且命中会带「弱相关」标签交给模型判断。
- **`update_note` / `search_notes` 的签名都变了**（多一个 `aliases` 参数、
  改收 `&NoteQueryTerms` 并返回 `Vec<CsNoteSearchHit>`）。旧签名未保留，
  符合「不留遗留隐患」；调用点已全部改完。
- **工具入参对裸字符串与旧 `query` 字段保持宽容**：schema 声明数组，但模型若仍
  发单串也能正常检索。硬报错会比原 bug 更糟。

### 未做的部分（故意）

- **`knowledge_search` 的同类缺陷仍在**：`nomifun-knowledge/src/service.rs:5767`
  的 `query_terms` 只按空白切分，对中文等价于整句匹配。修它需要改
  `nomifun-knowledge` 的检索链，与本次改动无耦合，独立成任务更安全。
  `expand_query` 已放在 `nomifun-common`，届时直接复用即可。
- **语义向量通道未引入**：理由见 §6（无离线 embedding 能力，且会把网络往返
  放进实时回复路径）。接缝已留：`CsNoteSearchHit.rank: f64` + 单一检索入口。

### 验证

- `cargo test -p nomifun-common text_search` — 13 passed
- `cargo test -p nomifun-db --lib customer_service` — 24 passed（含
  `visitor_phrasings_reach_the_expected_note`：16 条真实问法评测表，
  旧实现会在前四条失败）
- `cargo test -p nomifun-db --test id_schema_contract` — 22 passed
- `cargo test -p nomifun-customer-service` — 15 passed
- `cargo check --workspace --all-targets` — 无错误
- `bun run typecheck` / `bun run check:i18n` — 通过
- **已知既有失败（与本次改动无关）**：
  `nomifun-common` 的 `zip_safe::tests::colon_policy_diverges_only_on_non_prefix_colons`
  在 stash 掉全部改动的干净树上同样失败，属改动前既有问题。
