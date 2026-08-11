# 爬虫页「产出知识库」改为下拉框 · 改动记录与不确定点

日期：2026-08-07
关联：`docs/specs/2026-08-07-crawler-knowledge-entry-design.zh.md`（并行进行的
建库向导入口改造）

## 需求与动机

爬虫页新建作业时，「产出知识库」是一个裸 `<Input>`，要求手敲知识库 UUID。而
UI 任何位置都不展示知识库 id——知识库列表页只显示名称与路径。该字段实际上
无法在界面内闭环填写，只能开 DevTools 或直接打 API 才能拿到 id。

## 改动

| 文件 | 改动 |
| --- | --- |
| `hooks/knowledge/useKnowledgeBaseOptions.ts` | 新建。由 `pages/customerService/` 上移，option 增加 `rootPath` |
| `pages/customerService/useKnowledgeBaseOptions.ts` | 删除 |
| `pages/customerService/{CreateCsAgentModal,CsAgentDetailPage}.tsx` | import 改为共享路径 |
| `pages/crawl/CrawlPage/index.tsx` | `<Input>` → `<Select>`（名称 + 灰色根路径，`allowClear`，空态提示去建库） |
| `locales/{zh-CN,en-US}/crawl.json` | `knowledgeBase` 改名；新增 `knowledgeBaseHint` / `knowledgeBaseEmpty` |
| `pages/crawl/crawlPage.test.ts` | 新增结构测试，锁住「不得退回手敲 id」 |

## 验证

```
bun run typecheck / bun run check   → 全过
bun test --cwd ui                   → 1681 passed / 0 failed
运行时（dev:web）                    → 打开新建作业弹窗即请求 /api/knowledge/bases 200
```

## 不确定点

| # | 项 | 说明 | 状态 |
| --- | --- | --- | --- |
| 1 | 下拉未做搜索 | Arco 的 `filterOption` 拿不到 `option.props.children` 的类型（typecheck 报 TS2339），故去掉 `showSearch`。知识库通常个位数够用；库多了要改成 `filterOption={false}` + `onSearch` 受控过滤 | 待确认 |
| 2 | 选项副标题显示完整根路径 | 形如「测试craw知识库 D:\crawl-test-kb」。路径长时会撑宽下拉，是否只显示末级目录名 | 待确认 |
| 3 | 与建库向导的入口重叠 | 向导侧新增了「建库时选抓整站」，本改动是「爬虫作业选落到哪个已有库」。两者互补，但用户可能困惑该走哪条 | 待确认 |

## 顺带发现（不属于本次改动）

建库向导 site 模式把「浏览器渲染」开关映射为 `render_mode: 'browser'`
（`CreateStudio/index.tsx:261`），但阶段 A 的 `HttpCrawlFetcher::fetch` 对
`RenderMode::Browser` 直接返回 `BadRequest`，不是降级也不是只记 `wanted_render`。
实测（dev 环境，种子 `https://example.com/`）：

```
status=failed  attempt_count=3
error_code=fetch_failed
error_detail="Bad request: browser render mode is not available yet (stage B)"
job status=failed
```

即：用户在向导里勾上该开关建库，库会建成功，爬虫作业则重试 3 次后全灭。
爬虫页自己的渲染模式下拉已把 `browser` 设为 `disabled`，向导侧没有对应防护。
建议在阶段 B 落地前，向导 site 模式忽略该开关或同样禁用。
