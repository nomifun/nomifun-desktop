# UARC-000 工作树冻结清单

> 记录日期：2026-09-17
> 分支：`rf/agent-capability-platform-v2`
> source HEAD：`877b1a751536e40a3185c31790e6e63e86c6fa32`
> upstream：`origin/rf/agent-capability-platform-v2`，开始时 ahead/behind 为 `0/0`
> 冻结提交：`2147863da396835240296ec0a9b865200050b438`

## 1. 冻结边界

UARC-000 开始时暂存区为空。`git status --short` 显示 21 个路径条目；展开未跟踪目录后共有
26 个实际文件。下表逐文件记录来源、后续 owner 和 barrier 处理，不用目录级归类代替文件核对。

本次对这些继承改动的处理均为保留。没有删除、回滚或覆盖用户已授权内容。UARC-000 自己新增或
修改的清单、manifest 和状态台账仍由 Integration 独占。

## 2. 已确认设计与 Browser 文档修订

| 文件 | UARC-000 处理 | 后续 owner |
| --- | --- | --- |
| `docs/architecture/browser-platform.md` | 保留 Browser 产品模型纠正提示 | `UARC-040/041/061` 实现，`UARC-053/070` 最终残留审计 |
| `docs/architecture/browser-platform.zh.md` | 保留中文 Browser 产品模型纠正提示 | 同上 |
| `docs/specs/2026-09-13-browser-workspace-v2-progress.zh.md` | 保留历史证据/新模型 superseded 提示 | 同上 |
| `docs/specs/2026-09-13-browser-workspace-v2.zh.md` | 保留 WebView2/CEF 底层证据并废止专属 Session 产品结论 | 同上 |
| `docs/specs/2026-09-15-system-browser.zh.md` | 保留 attached Chrome 证据，归并为 Browser Provider | `UARC-040/053` |
| `docs/reviews/2026-09-16-coding-agent-runtime-compatibility.zh.md` | 保留 Coding 预设与 Runtime 不兼容审计 | `UARC-014/020/052` |
| `docs/specs/2026-09-16-agent-capability-product-redesign.zh.md` | 保留确认后的能力模型设计源 | 全部 Capability 波次 |
| `docs/specs/2026-09-16-agent-session-storage-redesign.zh.md` | 保留确认后的 Agent Store 设计源 | `UARC-011/012/051/054` |
| `docs/specs/2026-09-16-unified-nomi-runtime-convergence.zh.md` | 保留确认后的单 Runtime 设计源 | `UARC-013/014/020/052` |
| `docs/specs/2026-09-16-unified-agent-overhaul/README.zh.md` | 保留总计划 | Integration |
| `docs/specs/2026-09-16-unified-agent-overhaul/TASK-MANIFEST.json` | 保留并补齐设置页精确写集 | Integration |
| `docs/specs/2026-09-16-unified-agent-overhaul/STATUS.zh.md` | 保留并进入 active 状态 | Integration |
| `docs/specs/2026-09-16-unified-agent-overhaul/CONCURRENCY-MERGE-VALIDATION.zh.md` | 保留并发/合并/验证合同 | Integration |
| `docs/specs/2026-09-16-unified-agent-overhaul/MACOS-HANDOFF.zh.md` | 保留 macOS 真机证据合同 | `UARC-061/062/063`，Integration 管理状态 |

## 3. 已授权设置页检查点

这些文件完整进入 barrier，但它们描述的是重构前检查点，不是最终产品结论。尤其当前页面仍展示
Nomi/Coding 多 Runtime、Build/Profile 和 Agent Runtime 选择；`UARC-050` 必须将其改成单一 Nomi
Runtime 的只读 Build/健康/恢复诊断，并继续把 JavaScript Runtime 作为独立设置页。不得恢复 Runtime
selector 或把这一检查点当作最终 UI。

| 文件 | UARC-000 处理 | 后续 owner |
| --- | --- | --- |
| `ui/src/renderer/components/layout/Router.tsx` | 保留路由拆分 | Integration 为 `UARC-050` 提供配套路由修改 |
| `ui/src/renderer/pages/agentSession/navigation.structure.test.ts` | 保留结构测试 | `UARC-050` |
| `ui/src/renderer/pages/settings/AgentSettings/ExecutionEnginesSettingsContent.tsx` | 保留已授权删除 | `UARC-050` 不得恢复旧复合入口 |
| `ui/src/renderer/pages/settings/AgentSettings/index.tsx` | 保留已授权删除 | `UARC-050` 不得恢复旧复合入口 |
| `ui/src/renderer/pages/settings/components/SettingsSider.tsx` | 保留 Execution/JavaScript 分栏入口 | `UARC-050` |
| `ui/src/renderer/pages/settings/components/settingsNavigation.test.ts` | 保留导航测试 | `UARC-050` |
| `ui/src/renderer/services/i18n/i18n-keys.d.ts` | 保留已同步生成物 | Integration 统一生成 |
| `ui/src/renderer/services/i18n/locales/en-US/settings.json` | 保留英文文案检查点 | `UARC-050` |
| `ui/src/renderer/services/i18n/locales/zh-CN/settings.json` | 保留中文文案检查点 | `UARC-050` |
| `ui/src/renderer/pages/settings/ExecutionEngines/index.tsx` | 保留页面与状态处理检查点 | `UARC-050` 重做为单 Runtime 诊断 |
| `ui/src/renderer/pages/settings/ExecutionEngines/ExecutionEngines.test.tsx` | 保留 loading/error/stale/refresh 测试基线 | `UARC-050` 随最终交互更新 |
| `ui/src/renderer/pages/settings/JavaScriptRuntimeSettings.tsx` | 保留独立 JavaScript Runtime 页面 | `UARC-050` |

## 4. 拉取后架构核对

source HEAD 是合并提交。其第一父分支增量包含现有 macOS CEF child NSView、原生 fixture 和共享
Browser 语义；第二父分支增量包含 Companion 入口/资源选择与相关 Agent 合同生成物。UARC-000
没有改写两组已合并生产代码，并确认：

- Windows Browser 继续使用 WebView2；Windows target 不接入 CEF；
- macOS CEF 仍是底层开发检查点，后续由 `UARC-061` 接入 Browser Resource/Provider 和生产生命周期；
- 合并后的 i18n 生成物与当前全部 locale 一致；
- 设置页路由、测试和 IPC 类型在合并后的源码上仍可编译；
- 当前多 Runtime 设置页只作为保存的中间检查点，目标架构仍是唯一 `nomifun.nomi` factory。

## 5. UARC-000 基线验证

开始阶段结果：

```text
git diff --check
  passed

bun run typecheck
  passed

bun test --cwd ui \
  src/renderer/pages/settings/ExecutionEngines/ExecutionEngines.test.tsx \
  src/renderer/pages/settings/components/settingsNavigation.test.ts \
  src/renderer/pages/agentSession/navigation.structure.test.ts
  9 passed, 0 failed

bun run check:i18n
  passed; 7,787 keys / 35 modules

bun run check:desktop-ui-boundary
  passed; 1,931 renderer sources, minimum 880x600

bun run check
  passed; typecheck, desktop boundary, i18n, theme, icons, dead CSS,
  Windows installer, Creative Studio retirement, process/browser/automation
  boundaries, Agent vocabulary and help contract all green
```

逐文件清单与 dirty file set 的机器比较结果为 `26/26` 精确覆盖。最终 barrier 前重新运行
`git diff --check`，审查 staged diff，并在状态台账记录提交。

UARC-000 已由提交 `2147863da396835240296ec0a9b865200050b438` 冻结；该提交是后续 inventory
和 Wave 1 基础工作的可复现内容 barrier。
