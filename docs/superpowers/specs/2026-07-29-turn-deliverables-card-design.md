# 会话任务产物可视化交付卡(Turn Deliverables Card)设计

日期:2026-07-29
状态:已定稿(自主执行模式下按用户"深度分析 → 设计 → 实施"指令推进)

## 1. 目标

Agent turn 成功结束后,把该轮所有**最终确认成功**的文件产物聚合为一张"本次交付"卡片,渲染在该轮最终 Assistant 回复下方。适用于所有由 Agent 执行任务的会话形态(nomi / acp / openclaw-gateway / nanobot / remote、桌面伙伴发起的会话、AutoWork 执行只读转写视图)。

非目标:不为本需求做后端改动、不迁移数据库、不 bump `ui-api-contract-version.txt`、不触碰 `.github/workflows/`。

## 2. 现状结论(深度分析摘要)

经 5 路并行子系统探查 + 交叉验证(详见会话记录),关键事实:

1. **所有会话形态共用一个 `MessageList`**(`ui/src/renderer/pages/conversation/Messages/MessageList.tsx`):5 个平台 Chat、桌面伙伴投递产生的会话、AutoWork `ReadOnlyConversationView` 全部经由它渲染 —— 在此层做聚合天然覆盖全部形态,无需两套实现。
2. **已有强验证的文件收据通道**:`PersistedToolArtifact {id, kind, mime_type, path, relative_path, size_bytes, sha256}` 由后端两阶段提交(stream_relay 在 turn 成功终止时逐收据 reverify 后原子落库并在 Finish 前重播);历史读取时后端再次复验并降级失效收据(`project_historical_artifact_integrity`)。UI 侧 `normalizeToolCallContent`/`normalizeAcpToolCallContent` 只在 terminal success + `artifact_delivery_committed` 时保留 artifacts,失败吸收、成功单调。**流式阶段 relay 从不下发未提交收据**(先发 provisional 投影)。
3. **关键缺口**:普通 Write/Edit/ApplyPatch(nomi 原生)与 ACP 编辑(Claude Code/Codex)**不产生收据** —— 收据只覆盖生成器类工具(image_gen、export_pdf、MCP 媒体等)。截图动机场景(生成 snake-game.html)正是无收据路径。文件路径存在于:nomi `tool_call.args.file_path`、ACP `acp_tool_call` 的 diff content 项与 `locations[]`、legacy `tool_group` WriteFile 的 `result_display.file_diff`。
4. **turn 语义已贯通**:每条持久化行/WS 帧带 `turn_id`;UI `turnDisclosureModel.ts` 做 turn 归属回填与生命周期状态;`turn.completed` 事件永远是 `finished`(释放信号,非成功信号),成功须由消息级状态判定。
5. **可复用件**:`FileChangesPanel`(卡片骨架)、`parseDiff`(±行数)、`getFileTypeInfo`、`usePreviewLauncher`(预览,含 Provider 缺失降级)、`ipcBridge.fs.getFileMetadata`(带 workspace 边界校验的 stat)、`shell.openFile/showItemInFolder`(桌面端)、`diff@8`(依赖已有,可精确计算 ACP diff 行数)、i18n codegen。

## 3. 方案选型

| 方案 | 说明 | 结论 |
| --- | --- | --- |
| A. 纯前端派生(选定) | 在 MessageList 的 turn 分组层按 turn 聚合消息中已有的产物证据,合成卡片 VO 渲染 | 零后端改动、零迁移、零契约 bump;历史重载天然可重建;全形态覆盖 |
| B. 后端聚合(新消息种类/扩展 conversation_artifacts) | commit 窗口追加汇总行或新 WS 事件 | 触碰 2PC 提交事务、消息类型白名单、契约版本,与"不要大规模重构"冲突;收益仅是省去前端折叠 |
| C. 给 Write/Edit 补收据 | 在 BackendOutputSink 为编辑类工具注册收据义务 | 会把编辑纳入"义务失败即整 turn 失败"语义、向模型可见输出注入 locator 文本,风险大,明确不做 |

**分层信任模型(混合)**:
- **receipt 层(强验证)**:committed `PersistedToolArtifact`(nomi `tool_call.content.artifacts` + ACP `acp_tool_call` content 的 `artifact` 项)。完整性 = 收据存活(后端提交与历史读取均已校验 SHA-256/size/边界),卡片展示"已校验"徽标。
- **reported 层(工具成功 + 渲染期存在性校验)**:
  - ACP diff content 项(`{path, old_text, new_text}`,仅 update.status==='completed'):±行数用 `diff` 包精确计算;
  - legacy `tool_group` WriteFile 成功项(`isSuccessfulWriteFileResult`):`parseDiff(file_diff)` 得路径与 ±行数;
  - nomi 原生编辑类 `tool_call`(status==='completed' 且按 `classifyToolForReceipt` 归为 edit_files):从 `args.file_path`/`args.files[].file_path|path` 提取路径(跳过 `delete:true` 项),无行数。
  reported 项在渲染前经 `ipcBridge.fs.getFileMetadata({path, workspace})` 校验:成功 → 展示并补文件大小;NotFound/Forbidden → **不展示**(绝不把未验证项当可用交付物)。后端 stat 端点自带 allowed-roots∪workspace 路径围栏,模型伪造的越界路径会被 Forbidden 拒绝。

**turn 成功门控**(全部满足才出卡):
1. turn 已关闭(非 active running/waiting;`tailClosed` 语义与 disclosure 一致);
2. disclosure 状态非 `canceled`;
3. turn 内最后一个携带过程状态的项(含 assistant 角色的 error tips)状态非 `failed`/`canceled`(终止性失败不出卡;中途失败但恢复并正常完成的 turn 可出卡 —— 失败工具项本身不产出任何条目,且后端已对失败 turn 撤回收据)。

**去重**:按规范化的工作区相对路径(`\`→`/`,workspace 前缀剥离)为键,同一 turn 内后写覆盖先写("最终有效版本");receipt 与 reported 同路径时合并,取 receipt 的 size/sha 与较高信任层级,行数取 reported 的最新值。每个条目保留 `sources[]`(call_id / sourceMessageIds / 载体类型)供追溯扩展。

**空卡**:聚合结果为空,或 reported 项全部校验失败且无 receipt 项 → 不渲染任何 DOM。

## 4. 组件与数据流

```
useMessageList() ──► processedList(现有:tool_summary/file_summary/artifact VO)
                        │
displayList useMemo ────┼► modelInput(现有:turnId 回填 assignTurnIdsFromUserRequests)
                        ├► buildTurnDisclosureItems(现有:turn 折叠条)
                        └► collectTurnDeliverables(新:turnDeliverablesModel.ts,纯函数)
                              → Map<turnId, TurnDeliverablesVO>
                              → 插入到该 turn 在 displayList 中最后一项之后
renderItem ──► case 'turn_deliverables' ──► <TurnDeliverablesCard/>
                                              ├─ useDeliverableAvailability(stat 校验 reported 项,模块级缓存)
                                              ├─ usePreviewLauncher(预览/diff 查看)
                                              └─ shell.openFile / showItemInFolder(仅 isDesktopShell)
```

新增文件:
- `Messages/turnDeliverablesModel.ts` + `turnDeliverablesModel.test.ts`(bun:test)—— 纯聚合逻辑,单一共享实现;
- `Messages/components/TurnDeliverablesCard.tsx` —— 卡片 UI(标题"已生成 {{count}} 个文件"、默认前 3 条 + "再显示 N 个文件/收起"、行内:类型图标/文件名/相对路径/大小/±行数/完整性徽标/预览);
- i18n:`locales/{en-US,zh-CN}/messages.json` 增 `turnDeliverables.*`,跑 `scripts/generate-i18n-types.mjs`。

修改文件:仅 `MessageList.tsx`(VO 联合、构建、renderItem 分支、辅助函数补分支)。

## 5. 边界与错误处理

- **运行中的 turn**:不出卡;turn 完成后 `hooks.ts` 已有 turnCompleted → 重拉消息窗口,committed 行进入 store 自动触发重算。
- **失败/取消 turn**:门控排除;后端撤回帧(error + artifacts:[])经 merge 吸收,自愈。
- **历史重载**:持久化行带 turn_id + 2PC 标记,收据经后端读侧复验;reported 项渲染期 stat,被删文件自动消失(诚实语义)。
- **compact 截断**:>4096 字符的 file_diff 可能被截断 → parseDiff 行数不准;行数为可选展示,容忍。
- **分页窗口(60 条)**:旧 turn 的卡随滚动加载出现;VO id 锚定 `turn-deliverables-${turnId}`,coalesce 后不会重复。
- **Web 模式**:打开/定位按钮仅桌面端渲染(`isDesktopShell` 门控,遵循 WorkspaceOpenButton 先例);预览走 HTTP 正常可用。
- **只读视图/无 PreviewProvider**:`canPreview===false` 时隐藏预览按钮而非留死按钮。
- **性能**:聚合 O(n) 在既有 displayList useMemo 内;stat 一次性、模块级缓存(键 workspace|relativePath),与 PreviewContext 的 1s 轮询相比开销可忽略。

## 6. 测试策略

- 模型单测(bun:test):三载体聚合、跨 call 去重后写覆盖、receipt/reported 合并、失败/取消/运行中 turn 不出卡、error tips 终止失败不出卡、空聚合不出卡、路径规范化(Windows 反斜杠、绝对/相对)、ApplyPatch delete:true 排除、ACP diff 行数计算、来源追溯字段。
- 结构测试:保持现有 MessageList 结构断言通过;新增卡片组件轻量结构断言(隐藏死按钮、锚点 id 约定)。
- 全量:`bun run typecheck`、`bun scripts/generate-i18n-types.mjs --check`、`bun scripts/check-icon-imports.mjs`、Messages 目录 bun test。
