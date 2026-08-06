# 知识库回写三态化与待审移除 — 设计规格

日期:2026-08-05
状态:已获用户批准(设计逐节确认)
范围:知识库绑定、挂载、回写、待审(inbox)完整链路 — Rust 后端 + SQLite + WebUI/Tauri 前端

## 1. 背景与目标

现状的知识库回写由四个正交旋钮组成:`writeback`(布尔开关)、`writeback_mode`
(`staged` 暂存 / `direct` 直写,决定落点)、`writeback_eagerness`(`conservative`
保守 / `aggressive` 激进,决定意愿)、`channel_write_enabled`(外部 IM 渠道写入
特批,强制暂存)。暂存写入落到知识库根下 `_inbox/{scope}/`,由「待审」子系统
(HTTP 路由 + MCP 工具 + 审阅 UI + 红点计数)承接人工合并/丢弃。

本次重构把上述全部收并为**一个三态枚举**,并**彻底删除待审子系统**:

1. 移除暂存回写 —— 只要回写,就是直接回写。
2. 回写意识重命名并重定义:保守型 → **手动型**(仅当用户明确要求记录时才回写);
   激进型 → **自动型**(模型自主判断:与挂载库相关、确有价值、高置信才写,
   控制频率,跳过琐碎/临时/重复内容)。
3. 彻底移除「知识库-待审」:`_inbox` 目录、审阅 UI、待审 API/MCP 工具、
   计数徽标、暂存快照/元数据机器,全部删除,不留遗留隐患。

## 2. 已确认的决策(用户拍板)

| # | 决策点 | 结论 |
|---|--------|------|
| D1 | 存量 `_inbox/{会话id}/` 暂存文件 | **直接删除,不保留** |
| D2 | 自动型的频率/琐碎控制 | **只靠提示词约束**,不加代码限流/去重闸门 |
| D3 | 开关与意识的字段形状 | **收并为单一三态枚举** `off / manual / auto` |
| D4 | 外部 IM 渠道(无人值守 bot)回写 | **服从统一三态枚举**,删除 `channel_write_enabled` 特例 |
| D5 | 旧绑定数据映射 | **按意识直接映射(体验连续)**:`writeback=0→off`;`writeback=1 且 conservative→manual`;`writeback=1 且 aggressive→auto`;旧 staged/direct 不参与映射 |

知情确认的后果:D4 意味着渠道绑定选 `auto` 时,无人值守 bot 的写入直达知识库
正文;`knowledge_write` 在审批白名单中无条件放行(与旧行为一致),新设计下的
控制点是「该绑定选不选 auto」。

## 3. 语义模型

`writeback_mode: 'off' | 'manual' | 'auto'`,全链路唯一概念:

| 模式 | 语义 | 会话内工具写入 | 回合末自动沉淀(finalizer) |
|------|------|----------------|------------------------------|
| `off` | 只读 | 禁止(提示词 READ-ONLY + 服务端 `WriteMode::Disabled` 拒写) | 不触发 |
| `manual` | 用户明确要求时才回写 | 允许;提示词约束「仅当用户明确要求记录/保存时」 | **不触发(代码闸门)** |
| `auto` | 模型自主判断回写 | 允许;提示词约束「高置信、确有价值、控频、跳过琐碎/临时/重复」 | 触发 |

**manual 的执行点是代码闸门,不是文案**。回合末回写是后端自动发起的一次 LLM
提取(`build_turn_writeback_request` → `finalize_turn_writeback_with_progress`),
意识值只影响提取器提示词门槛;若只改文案,manual 就是运行时不守的空头承诺。
因此:

- `nomifun-conversation/src/service.rs` 的 `build_turn_writeback_request`(该函数
  同时服务回合末触发与用户手动重试两条入口)在 `mode != auto` 时返回 `None`
  —— finalizer 只在 `auto` 下运行,manual 下不消耗模型调用。
- 防御纵深:`nomifun-knowledge` 服务侧 finalizer 入口在非 `auto` 时短路返回
  `Disabled` 状态(finalizer 局部判断,不进共享的 `resolve_write_policy`,
  否则会把 manual 的会话内工具写入一并杀掉)。
- manual 的回写全部走会话内 `knowledge_write` 工具(nomi 引擎)或挂载目录文件
  写(终端/ACP),由提示词约束「仅在用户明确要求时」。这与 D2 的信任模型一致。

写入落点决策 `resolve_write_policy` 收敛为:`off → Disabled`;`manual/auto →
Direct`,四种 surface(RegularChat / TerminalAcp / Companion / ExternalChannel)
统一,`WriteMode::Staged { scope }` 变体删除,`WriteMode` 收敛为
`Disabled | Direct`。

解析回落语义变更:旧代码未知值回落 `staged`(旧世界的安全值);新枚举解析
失败/缺省一律回落 **`off`**(新世界的安全值 = 不写)。四处隐式 `"staged"`
默认(`default_writeback_mode`、factory/nomi.rs、conversation service、
caps_knowledge gateway 的 `..Default::default()`)全部改为显式 `off`。

## 4. 数据模型与迁移(migration 024)

两张表的 CHECK 约束把旧字面量钉死(`knowledge_bindings.writeback_mode CHECK IN
('staged','direct')`、`writeback_eagerness CHECK`、`preset_knowledge_policy.eagerness
CHECK`),SQLite 不能改 CHECK,也不能 DROP 被 CHECK 引用的列 —— **两表整表重建**,
遵循仓库唯一先例 `018_customer_service.sql:185-228` 的四步法(CREATE new →
INSERT SELECT → DROP → RENAME → 重建索引),`id` 原样拷贝。

`crates/backend/nomifun-db/migrations/024_knowledge_writeback_mode_collapse.sql`:

- **`knowledge_bindings`**:删 `writeback`、`writeback_eagerness`、
  `channel_write_enabled` 三列;`writeback_mode` 改为
  `TEXT NOT NULL DEFAULT 'off' CHECK (writeback_mode IN ('off','manual','auto'))`。
  数据映射按 D5(`CASE WHEN writeback = 0 THEN 'off' WHEN writeback_eagerness =
  'aggressive' THEN 'auto' ELSE 'manual' END`)。重建四个 partial unique 索引
  (`uq_knowledge_bindings_target_{workpath,conversation_id,terminal_id,companion_id}`)。
- **`preset_knowledge_policy`**:删 `mode`(纯落点概念,已实读确认:两个读者把
  `inherit` 折叠为 `staged`、`eagerness=NULL` 替换为 `conservative`,库选择语义
  在 `preset_knowledge_bases` 表,不受影响)、`writeback`、`eagerness` 三列,
  新增 `writeback_mode TEXT NOT NULL DEFAULT 'off' CHECK IN ('off','manual','auto')`,
  映射同 D5。重建索引 `idx_preset_knowledge_policy_preset_id`。
- **`messages.content` 清洗**:`json_remove` 剥离历史 `knowledge_writeback.written[]
  .staged` 字段(023 有 `json_remove` 先例)。历史 `_inbox/...` 路径字符串**不改写**
  (消息正文不做深度改写),由读侧归一化助手兜底(见 §6)。
- 禁改 `001_v3_baseline.sql`(checksum 冻结,三个测试 `include_str!` 引用)。
- schema 契约约束:`id INTEGER PRIMARY KEY AUTOINCREMENT` 原样保留;partial
  unique 索引谓词逐字保留(`id_schema_contract.rs` 的 `PARTIAL_UNIQUE_INDEXES`
  注册表校验);迁移结束不得残留 `_new` 表。

**Rust 侧同步**(编译器抓不住 SQL 列漂移 —— 全仓无 sqlx 编译期宏,绑定读取是
`SELECT *` + `try_get(列名)`,列删了字段没删就是运行时炸,终端路径还会被吞掉
静默卸载知识库,所以 DB 模型、repository、服务结构体必须与迁移同批落地):

- `KnowledgeBindingRow` / `FromRow` / `set_binding`(9 参 → 6 参)/ 测试夹具;
- `KnowledgeBinding`(服务/wire 结构)收敛为 `{enabled, writeback_mode, kb_ids}`,
  `serde` 缺省 `off`,不加 `deny_unknown_fields`(陈旧 UI 发来的已删字段静默忽略);
- `MountOutcome` 收敛为单 `writeback_mode` 字段;
- `binding_from_row` 校验新值域;`WRITEBACK_MODES = ["off","manual","auto"]`,
  `WRITEBACK_EAGERNESS` 常量删除;
- `binding_signature` 版本前缀 `kb-binding-v1:` → **`kb-binding-v2:`**,哈希输入
  改为 `(enabled, writeback_mode, targets)` —— 否则升级后首次运行把活租约误判
  为冲突对等体;
- `PresetKnowledgePolicy`(api-types)收敛为 `{enabled, writeback_mode, grounded}`;
  `UpsertPresetParams.knowledge_policy` 5 元组 → 3 元组(位置元组,靠元数变化
  强制编译期暴露所有触点,含 sqlite_preset.rs 的 drift 比较 —— 漏改会导致内置
  预设每次启动重播种)。

**契约版本**:`ui-api-contract-version.txt` 4 → 5,与 `bun run build:ui` 产物
同 commit(后端启动对 dist 的 `api_contract_version` 做硬断言)。

## 5. 存量 `_inbox` 清理(三道防线 + README 重刷)

1. **打开知识库时一次性清扫**:删除库根下整个 `_inbox/` 目录树 —— 含
   `.nomifun-base-{sha256}.json` 暂存 sidecar(点前缀非 md 文件,所有遍历器都
   看不见它,只删 `*.md` 会留下删不掉的空壳目录)。挂靠在现有的库打开/挂载
   同步路径上,幂等。
2. **导入剥离**:旧导出 zip 特意携带 `_inbox/**`(export.rs 注释言明),导入时
   静默跳过该顶层目录,防止旧包在新版本复活它。导出侧不再打包(随代码删除
   自然消失)。
3. **保留名过滤**:`_inbox` 留在共享保留目录名单 `is_excluded_tree_dir_name`
   中 —— 永不显示(文件树/TOC/搜索/文档列表/autogen 采样)、永不可写(写目标
   校验拒绝)。成本一个字符串,收益是任何漏网旧备份不会把暂存残骸暴露给
   agent 与 UI,也不会被当成合法写入目标。这是**唯一保留的 inbox 痕迹**,
   性质是保留字,不是功能。

**终端 README 重刷**:`{cwd}/.nomi/knowledge/README.md` 已落盘旧 STAGED 契约
文案(指名 `_inbox/{target_id}/`),挂载清扫特意豁免它(MANAGED_KEEP),且只有
活终端会刷新。升级后做一次 workpath 绑定 README 重刷:遍历 `knowledge_bindings`
中 `target_kind='workpath'` 的绑定,对仍存在的工作区目录用新契约重写 README
(绑定为空则删除)。挂靠应用启动的一次性任务,幂等,目录不存在则跳过。

## 6. 历史消息兼容(读侧归一化)

持久化的助手消息 JSON 存有 `written[].staged: true` 与 `_inbox/{scope}/...`
相对路径;重试排除与去重键通过 inbox 路径算术读回。删掉算术后,历史消息上的
重试会与新提取的逻辑目标对不上 → 同一知识写两遍。处置:

- 保留 `logical_writeback_target_from_storage_path` 作为**纯函数遗留归一化助手**
  (剥 `_inbox/{scope}/` 前缀),conversation 侧重试排除(service.rs 的
  `staged_prefix` 剥离处)改用该共享助手;`KB_INBOX_REL_DIR` 常量随之保留
  (crate 内私有化),注释标注「仅服务于历史消息归一化与保留名过滤」。
- 启动对账(stream_relay.rs:1235-1275)是第二个读历史 `staged`/`_inbox` 的
  durable 读者,同样走归一化助手。
- 新消息不再携带 `staged` 字段;UI 渲染只看 `rel_path`(现状已如此),
  `normalizeKnowledgeWritebackFiles` 白名单删掉 `staged` 行,历史消息的该
  字段被静默丢弃。

## 7. 提示词契约(context.rs)

`WritebackMode { Staged, Direct }` + `WritebackEagerness { Conservative, Aggressive }`
两枚举合并为一个 `WritebackMode { Off, Manual, Auto }`(wire 字面量
`"off" | "manual" | "auto"`,解析失败回落 `Off` 并告警)。
`KnowledgeContextOptions` 删 `writeback` 布尔与 `writeback_eagerness`;
`target_id` 字段随暂存 scope 消亡(实施时确认无其余用途后删除)。

契约文案(英文,项目约定;要点,实施时按现有文风润色):

- **Off**:沿用现有 DISABLED 文案(READ-ONLY,禁止增改删)。
- **Manual**(工具面):"Write-back is MANUAL: treat the mounted bases as
  read-only by default. Write ONLY when the user explicitly asks you to record,
  save, or update something in a knowledge base — then CALL the `knowledge_write`
  tool. To UPDATE an existing document, pass the `handle` from a
  `knowledge_search` result (read first with `knowledge_read`, merge, then write
  the full `content`); to CREATE, pass `base` plus a descriptive `.md`
  `rel_path`. Never write on your own initiative. Do NOT use the generic
  Write/Edit file tools for knowledge; never delete files."
  文件面(终端/ACP)同义改写为文件写措辞。
- **Auto**(工具面):"Write-back is AUTO: autonomously persist knowledge
  related to the mounted bases when it is genuinely valuable, durable, and
  high-confidence. Be selective and control frequency: skip trivial, transient,
  session-specific, uncertain, or duplicate material; prefer updating an
  existing document over creating near-duplicates." + 同样的工具用法段。
  文件面同义改写。
- 回合末提取器(turn_writeback.rs)只保留 auto 一档规则文本(manual 不进
  提取器);系统提示删除「never under `_inbox/`」行。
- 终端 README(TerminalReadme 格式)同步换新契约;交付通道是挂载同步 +
  §5 的一次性重刷。

## 8. API / 工具 / 事件面收缩

**HTTP 路由删除(7 条)**:
`GET /api/knowledge/bases/{id}/inbox`、`GET /api/knowledge/inbox/pending-count`、
`GET /api/knowledge/bases/{id}/inbox/diff`、`POST /api/knowledge/bases/{id}/inbox/merge`、
`POST /api/knowledge/bases/{id}/inbox/discard`、`POST /api/knowledge/inbox/merge-all`、
`POST /api/knowledge/inbox/discard-all`。
`POST /api/knowledge/binding/{kind}/{target_id}` 路径不变,请求/响应体变为
`{enabled, writeback_mode, kb_ids}`;所有 base 响应删 `pending_inbox` 字段
(该字段无 `skip_serializing_if`,属 wire break,由契约版本 bump 兜底)。

**MCP/网关工具**:删除 `nomi_knowledge_list_inbox` / `nomi_knowledge_merge_inbox`
/ `nomi_knowledge_discard_inbox`(caps_knowledge_ext.rs 保留文件与
`list_consumers`);`nomi_knowledge_set_binding` 参数收并为 `writeback_mode`
(值域校验补上 —— 现状盲赋值);`nomi_knowledge_write_file` 与终端 MCP
`knowledge_write` 结果删 `staged` 字段;`/context` 的 `write_enabled` 改为
`mode != off` 推导。gateway 能力数 148 → 145,仍高于 `>= 135` 注册表下限。
终端 MCP 的 `write_scope` 推导与 `opaque_workpath_write_scope`(及其唯一的
sha2 import)删除。

**知识服务删除面**(service.rs ~1500 行 inbox 机器):`list_inbox` / `inbox_diff`
/ `merge_inbox` / `discard_inbox` / `merge_all_inbox` / `discard_all_inbox` /
`count_pending_inbox` / `list_inbox_entries*` / `validate_inbox_scope` /
`unified_md_diff` / `prune_empty_inbox_dirs`;`StagedBaseSnapshot` /
`StagedProposalMetadata` 及其快照/校验/sidecar 读写函数;
`write_resolved_document_under_target_lock` 的暂存发布分支(签名 3 参 → 2 参);
`staged_turn_writeback_candidate_already_present` 去重门;`WriteOutcome.staged`;
`InboxEntry` / `InboxDiff` / `InboxMergeResult` 类型;`KnowledgeBaseInfo.pending_inbox`
及其有界遍历计数。`list_consumers` 幸存。锁粒度说明:暂存曾按外层 scope 加锁
防「显式工具写 vs 回合末写」竞争,直写按 `(root, rel_path)` 加锁 —— 收敛后
统一后者,原有的 direct 并发测试语义不变。

**事件**:无 inbox 专属事件。`knowledge.binding-changed` 载荷键收缩(删三键、
`writeback_mode` 换值域);`knowledge.base-*` 载荷随 `pending_inbox` 消失;
回合回写流事件载荷删 `staged`。

**agent 侧**:`nomi-agent/knowledge_tools.rs` 的独立 `WriteMode { Staged{scope},
Direct }` 枚举、`WriteRequest.mode`、`WriteReceipt.staged`、工具描述的
「(STAGED to the review inbox…)」尾注删除,`KnowledgeWriteTool::new` 4 参 → 3 参;
`normalize_write_rel_path` 显式拒绝首段 `_inbox`(与保留名过滤一致,防模型
按旧惯性写出该路径);`nomifun-ai-agent` 的 `knowledge_writeback_staged` 推导、
staged placement 构造、`TMode::Staged` 映射臂删除(约 20 个构造调用点为编译期
断裂,好事);`AcpBuildExtra` / `NomiBuildExtra` 的 `knowledge_writeback` 布尔与
`knowledge_writeback_eagerness` 删除、`knowledge_writeback_mode` 换新值域、
`knowledge_channel_write_enabled` 删除;`runtime_registry.rs` 的原始 JSON 键
探测名单同步(三处键名单 —— conversation service 两处 + runtime_registry 一处
—— 必须锁步,漂移会导致租约签名反复搅动、agent 反复回收);
`apply_model_only_ceiling` 补上强制 `writeback_mode = off`(单字段承载全部
语义后,ceiling 不清它就等于没清);conversation routes 的
`strip_server_owned_runtime_fields` 名单补入新键。
`factory/nomi.rs` 的 `append_knowledge_context` 现存 raw-vs-resolved 不对称
(963-977 用原始值,`has_write_tool` 用已解析策略)一并修正 —— 单一来源:
已解析的三态。

**孤儿依赖**:`similar` crate(唯一消费者 `unified_md_diff`)从
nomifun-knowledge 的 Cargo.toml 移除;`sha2`/`hex` 保留(service.rs 等仍用)。

## 9. UI 与文案

- **挂载控件 KnowledgeControl**:Switch + 模式 Segment + 意识 Segment →
  **单个三段选择器**(复用现有 `renderSegment`),始终可见:
  关闭 / 手动 / 自动,配一行 hint。`defaultKnowledgeBinding()` →
  `{enabled: false, writeback_mode: 'off', kb_ids: []}`。
- **删除**:`InboxReviewPanel.tsx` 整文件(唯一引用者是详情页;`Diff2Html.tsx`
  另有 Workspace 消费者,**不删**);详情页 inbox 标签 + `TabKey` 收缩(旧
  `?tab=inbox` 深链靠现有 `ALL_TABS.includes()` 守卫回落 docs);
  `useKnowledgeInbox` / `useKnowledgeInboxPending`;Sider 红点(`SiderKnowledgeEntry`
  的 `dot` 机制整体删除);KnowledgeCard 待审计数 pill;ipcBridge 7 个 inbox
  方法 + `IKnowledgeInboxEntry` / `IKnowledgeInboxDiff` / `pending_inbox` /
  `KnowledgeWritebackEagerness` 类型;chatLib 两处 `staged` 声明(裸 spread 桥接,
  必须同 commit 删,否则静默腐烂)。
- **预设抽屉**:mode 三选一(inherit/staged/direct)整个删除 + 「允许回写」
  勾选 + 意识下拉 → 单个三态 Select(`knowledgePolicy.writeback_mode`);
  `presetTypes.ts` 同步收敛。
- **GUID 向导**:`kbTouched` 判定改为
  `kb.enabled || kb.kb_ids.length > 0 || kb.writeback_mode !== 'off'`(半改状态
  会让每个新会话误判为「有高级配置」并多发一次绑定 POST —— 与
  `defaultKnowledgeBinding()` 同 commit 改)。
- **MigrateTab**:绑定字面量改 `{enabled: true, writeback_mode: 'off', kb_ids}`。
- **i18n**(两语种同步;`bun run gen:i18n` 重新生成 `i18n-keys.d.ts`,CI 的
  `check:i18n` 只看 en-US 键集,zh-CN 残键要人工清;**所有内联 `defaultValue:`
  必须与键同步改**,因为 `t()` 不做键类型检查):
  - 删:`knowledge.card.pending`;`knowledge.control.{modeDirect,modeDirectHint,
    modeStaged,modeStagedHint,writebackHint,writebackMode,writebackEagerness,
    eagernessAggressive,eagernessAggressiveHint,eagernessConservative,
    eagernessConservativeHint}`;`knowledge.detail.inbox.*`(9 键)、`inboxEmpty`、
    `tabInbox`、`use.writebackStaged(Desc)`、`use.writebackDirect(Desc)`;
    `knowledge.inbox.*`(10 键);`knowledge.tabs.*`(已死);
    `knowledge.mount.eagernessLabel`;`settings.presetKnowledgeModeInherit/
    Staged/Direct`、`settings.presetKnowledgeWriteback`。
  - 增:`knowledge.control.modeOff` 关闭 / Off;`modeOffHint` 本会话只读知识库,
    不写回任何内容。/ This session only reads the bases — nothing is written
    back.;`modeManual` 手动 / Manual;`modeManualHint` 只在你明确要求时才写回
    知识库。/ Writes back only when you explicitly ask.;`modeAuto` 自动 / Auto;
    `modeAutoHint` 模型自行判断,把高价值、高置信的知识自动写回。/ The model
    decides on its own and writes back high-value, high-confidence knowledge.;
    `knowledge.detail.use.writebackManual(Desc)` / `writebackAuto(Desc)`;
    `settings.presetKnowledgeWritebackMode`(选项标签复用 `knowledge.control.mode*`)。
  - 改:详情页 step3 文案、三点式回写说明(关/手动/自动)、删除警告去掉
    「待审内容」。
- **文档**:README.md:167 与 README.zh-CN.md:166 的「staged into a review
  inbox」承诺改写为三态描述;docs/guides/companions.md/.zh.md 的 staged/direct
  两模式段落改写;docs/guides/channels*.md 的渠道回写描述同步;
  `knowledge_stdio.rs` 工具描述删「or is staged for review」;
  CHANGELOG「Unreleased」记 breaking change(平铺 `-` 条目、因果式散文、注明
  契约版本 bump —— 参照 capabilities 移除先例的写法);历史 handoffs 与
  CHANGELOG 旧条目**不改**(历史记录)。

## 10. 兼容性与风险清单

| 风险 | 处置 |
|------|------|
| 迁移前旧行 + 新代码 → 每次绑定读取 500(会话路径)/静默卸载(终端路径) | 迁移与 Rust 值域校验同批落地;`binding_from_row` 只认新值域,兜底由 024 保证 |
| 陈旧 UI dist 对新后端 | 契约版本 4→5 硬断言,启动即失败并提示重建 |
| 陈旧渲染端 POST 旧字段 | 结构体无 `deny_unknown_fields`,旧字段静默忽略;`writeback_mode` 旧值(staged/direct)被 400 拒绝(显式失败,优于静默改义) |
| 历史消息重试重复写入 | §6 读侧归一化助手 |
| 旧导出包复活 `_inbox` | 导入剥离 + 保留名过滤 |
| 未再启动的终端工作区旧 README 引导写 `_inbox` | §5 README 重刷 + 保留名过滤兜底(写目标校验拒绝) |
| `IdmmTendency` 共享 `conservative/aggressive` 字面量、Workspace git 的 `staged`、engine 的 `steering_inbox`/`system_resource_inbox`、companion 的 `_drafts` 审阅暂存区 | **禁止机械全局替换**;实施计划逐文件点名,这些子系统不触碰 |
| 渠道 auto 直写正文 | D4 知情决策;文档写明 |

## 11. 非目标

- 不保留任何暂存/待审逃生舱(无 feature flag、无隐藏配置)。
- 不做代码级限流、内容指纹去重、写入配额(D2)。
- 不为 manual 做「用户意图检测」LLM 调用 —— manual 的守门是提示词 + finalizer
  代码闸门。
- 不改多轮消息正文中的历史路径字符串(只做读侧归一化)。
- 不动 URL 快照保护(`validate_source_owned_write_target` 与落点无关,原样保留)。

## 12. 验收标准

1. `cargo check --workspace` 无警告级错误;`bun run test`(串行 cargo test)、
   `bun run test:ui`、`bun run check`(typecheck + i18n + theme + icons + 边界
   + 词汇检查)全绿。
2. 新增测试:024 迁移新旧行四种映射(含 NULL eagerness)断言;manual 模式下
   finalizer 不调用 completer(零模型调用)断言;auto 模式回合末直写断言;
   `_inbox` 打开清扫 + 导入剥离断言;历史 `_inbox` 路径重试去重断言。
3. 全仓 grep 验证:除「保留名过滤、读侧归一化助手、历史 handoffs/CHANGELOG、
   与知识库无关的同名词」四类白名单外,`_inbox` / `staged`(知识域)/
   `eagerness` / `conservative|aggressive`(知识域)/ 待审 / 暂存(知识域)
   零残留。
4. 手工链路验证:三态选择器持久化往返;off 拒写;auto 会话回合末直写正文;
   知识库详情页无待审痕迹;Sider 无红点;导入旧 zip 不出现 `_inbox`。

## 13. 实施顺序(供实施计划展开)

1. **DB 批**:migration 024 + models/repository/backup 同步 + schema 契约测试
   + 迁移映射测试。
2. **知识服务批**:类型收敛(KnowledgeBinding/MountOutcome/WriteMode/
   WritePolicy)、inbox 机器删除、清扫/导入剥离、契约文案重写、finalizer 闸门、
   binding_signature v2、服务测试重写。
3. **消费者批**:conversation / terminal / companion / preset / channel /
   ai-agent factory / nomi-agent tools / gateway / app,含三处键名单锁步与
   README 重刷任务。
4. **前端批**:ipcBridge 类型与方法、KnowledgeControl 三段选择器、详情页/
   Sider/卡片删除、预设抽屉、GUID/Migrate、i18n 全套 + 代码生成、UI 测试。
5. **收尾批**:契约版本 bump + `bun run build:ui`、文档/CHANGELOG、全仓 grep
   验收、全量测试。

批间依赖:1→2 同 PR 强耦合(列删除是运行时炸点);3、4 可在 2 完成后并行;
5 收口。
