# 爬虫接入知识库创建入口 · 技术设计与实施记录

状态：**已实施，待 review**（不确定点清单未过完）
日期：2026-08-07

---

## 1. 需求

「新建知识库 → 网页 / URL」这个已有入口只能抓固定的 URL 列表（上限 16 条，不
跟进链接）。把它和 `nomifun-crawl` 打通，让用户在建库时就能选「抓整个站」。

## 2. 现状

两边**已经共用底层**：

- `nomifun-crawl/src/fetcher.rs` 复用 `nomifun_knowledge::source_url::HttpFetcher`，
  SSRF 管线只有一份。
- `nomifun-crawl/src/sink.rs` 的 `KnowledgeSink` 已经通过
  `KnowledgeService::write_document` 落知识库。

缺的只是**建库向导侧的入口**，以及三处语义冲突。

| 维度 | 旧「网页/URL」 | 爬虫 |
| --- | --- | --- |
| 输入 | URL 列表（`MAX_URLS = 16`） | 种子 + scope |
| 跟进链接 | 无 | max_depth / max_urls |
| 礼貌抓取 | 无 | robots / 令牌桶 / 熔断 |
| 增量 | 全量重抓 | ETag / Last-Modified / 内容哈希 |
| 失败恢复 | 无 | claim / lease / 围栏令牌 |

旧模式本质是爬虫的 `max_depth = 0` 特例。

---

## 3. 三个冲突点的结论

### 3.1 落盘布局 → 两套目录并存，目录即所有权边界

`KnowledgeService::prepare_source_snapshots` 是**按 `entries` 全量重建**
`snapshots/` 下的文件的（slug 由配置 URL 推导，重名加数字后缀）。爬虫若也写
`snapshots/`，用户点一次「刷新源」就会冲掉爬虫产物或留下孤儿文件。

| 目录 | 归属 | 生命周期 |
| --- | --- | --- |
| `snapshots/{slug}.md` | KB 的 `extra.source.entries` | 随 entries 全量重建 |
| `crawl/{job-slug}-{id8}/{page}.md` | crawl job | 随 job 增量更新 |

### 3.2 inbox → 按「目标库是不是这次新建的」分流

`InboxReviewPanel` 是逐条 diff + 逐条接受。一次抓 500 页全进 inbox 等于 500 个
待办，功能不可用。

```
sink 是本次一同创建的新库  → via_inbox = false（直写）
sink 是已存在的库          → via_inbox = true（默认，可关）
```

新库是用户专为这次抓取而建的空库，不存在「污染已有资料」的风险；往已在用的库
里塞内容才是 inbox 的适用场景。

> 这修正了上游设计里「无条件走 inbox」的结论——那条的前提是「爬虫只有独立
> 入口」，加了建库入口后前提已变。

### 3.3 刷新归属 → 向导第三模式不写 `source.entries`

选「站点抓取」时创建的是**普通托管库（等同空白库）+ 一个 crawl job**，
`extra.source` 留空，不是 `kind: 'url'` 的源。

- 库里没有 entries ⇒ 没有 `refresh_source` 可点，歧义在数据模型层面消失。
- 关联是单向的：job 记 `knowledge_base_id`（已有字段），KB 侧反查
  `crawl_jobs where knowledge_base_id = ?`。**只有一个真相源，零 migration。**
- 同一 URL 被 entries 与爬虫各抓一份的重复问题从源头消失（两种模式互斥）。

---

## 4. 顺带修的三个 bug

### bug ① job 目录碰撞

`document_path` 用 `slugify(job.name)` 分组，注释声称「避免两次抓同一站互相
覆盖」，但**两个同名 job 会落进同一目录**并互相覆盖；`slugify` 还截断到 60
字符，长名字更易碰撞。

修：`crawl/{slug}-{job_id 尾 8 位}/`。

### bug ② inbox scope 与 rel_path 重复

`scope = "crawl"` 且 `rel_path = "crawl/{job}/…"`，而 staged 路径是
`_inbox/{scope}/{rel_path}`，拼出 `_inbox/crawl/crawl/my-site/page.md`。
且所有作业共用一个 scope，inbox 无法按作业分组。

修：`scope = "crawl-{job_id}"`。`validate_inbox_scope` 要求单段且过
`validate_portable_path_component`（禁 `:`），UUIDv7 的 hex+连字符可用。
副作用：inbox 天然按作业分组，后续「按作业批量接受」有了抓手。

### bug ③ UUIDv7 前缀不能用来区分 job（实施中由新测试抓出）

第一版把 bug ① 修成了取 job_id 的**前 8 位**。新增的
`same_named_jobs_do_not_share_a_directory` 直接挂掉：

```
left:  "crawl/docs-019fda3a/example-com-a.md"
right: "crawl/docs-019fda3a/example-com-a.md"
```

UUIDv7 的前 12 个 hex 是 48 位毫秒时间戳，**前 8 位是它的高 32 位，约 65 秒才
变一次**——同一分钟内建的两个 job 前缀完全相同，等于没修。改取**尾 8 位**
（随机段）后通过。

---

## 5. 实际改动

| 文件 | 改动 |
| --- | --- |
| `nomifun-crawl/src/sink.rs` | `document_path` 加 `job_id` 参数并用尾 8 位；新增 `inbox_scope()` / `id_suffix()`；`CRAWL_INBOX_SCOPE` → `CRAWL_INBOX_SCOPE_PREFIX` + `CRAWL_REL_DIR`；新增 4 个单测 |
| `nomifun-crawl/tests/knowledge_sink_e2e.rs` | `doc_dir()` helper + 路径断言跟随新布局 |
| `CreateStudio/SourceConfig.tsx` | `UrlMode` 增 `'site'`；三段式 segment 改为数据驱动；site 面板（种子 / 深度 / 页数上限 / 同站开关 / 提示）；导出 `SITE_DEFAULT_MAX_DEPTH=2`、`SITE_DEFAULT_MAX_URLS=200` |
| `CreateStudio/index.tsx` | site 分支：建库（不带 source）→ `crawl.createJob`（`via_inbox: false`）→ `startJob`；抽出 `isHttpUrl` 并复用到原有 entries 校验 |
| `locales/{zh-CN,en-US}/knowledge.json` | 新增 9 个 key，改写 `webModeHint` |
| `knowledge/createStudioFormVisual.test.ts` | 导入行断言跟随 `InputNumber` |

**不需要**新增 REST 路由、migration，也不需要递增 `ui-api-contract-version.txt`
（只复用既有端点与既有 DTO）。

## 6. 验证

```
cargo test -p nomifun-crawl   → 85 passed / 0 failed（lib）+ 3 passed / 1 ignored（e2e）
bun run check                 → typecheck / i18n / theme / icons / 三个 boundary / help 全过
bun test --cwd ui             → 1680 tests，全过
```

实跑落盘证据（`--ignored` 手工 e2e）：

```
_inbox/crawl-019fda40-3fa3-7433-9871-ab60eebb6ff6/crawl/e2e-crawl-eebb6ff6/127-0-0-1-page-2.md
_inbox/crawl-019fda40-3fa3-7433-9871-ab60eebb6ff6/crawl/e2e-crawl-eebb6ff6/127-0-0-1.md
```

改前同一场景是 `_inbox/crawl/crawl/e2e-crawl/…`（scope 与目录重复、无 job 区分）。

Windows 路径长度：staged 路径最坏约 root+209 字符。已确认
`nomifun-knowledge` 的 Windows 写入走 `\\?\` 扩展路径（`windows_api_path`），
Rust std 的 `File`/`create_dir` 也自动转 verbatim，MAX_PATH 不构成阻塞。

---

## 7. 不确定点清单

逐条过，确认后更新状态。

| # | 项 | 风险 / 备选 | 状态 |
| --- | --- | --- | --- |
| 1 | `via_inbox = false` 让向导建的爬虫直写库体 | 与上游「一律走 inbox」结论相反。若你更看重「一律可审」，翻回 true 即可（一行） | 待确认 |
| 2 | site 模式建的是空白库，不写 `extra.source` | 库详情页的「源」区块会是空的，用户可能疑惑「我明明选了网页」 | 待确认 |
| 3 | 建库后自动 `startJob` | 点「创建知识库」即刻产生外网流量，无二次确认 | 待确认 |
| 4 | 向导默认 `max_depth=2` / `max_urls=200`，低于爬虫页默认（3 / 10000） | 因为向导会立即开跑，误配代价要小；但用户可能觉得抓不全 | 待确认 |
| 5 | job 目录后缀取 8 位（32 bit） | 同名 job 且尾 8 位碰撞才会重现 bug ①，1000 个作业约 0.01%。要更稳可加到 12 位 | 待确认 |
| 6 | 已跑过的旧 job 产物留在 `crawl/{slug}/` | 不做数据迁移，旧目录成为孤儿 | 待确认 |
| 7 | 知识库详情页未展示关联 job | 本次不做，用户得去「爬虫」页看进度 | 待确认 |
| 8 | inbox 仍是逐条接受，没做「按作业批量接受」 | 已有的独立入口（写已存在的库）依然会堆待办 | 待确认 |
| 9 | `browserRender` 开关在 site 模式映射为 `render_mode: 'browser'` | 但爬虫阶段 A 还没有浏览器后端，只会记 `wanted_render` 不会真渲染 | 待确认 |
