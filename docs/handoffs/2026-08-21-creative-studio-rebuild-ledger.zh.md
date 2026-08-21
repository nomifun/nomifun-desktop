# Creative Studio 重建实施台账

> 用途：在长任务发生上下文压缩或人员切换时，从可验证提交继续，而不是重新审计整仓。
> 分支：`codex/infinite-canvas-rebuild`
> 最后功能锚点：`1dd4b9e9`（`feat(creative-studio): add standalone task history API`）
> 参考产品锚点：`ef7303d`

## 1. 续接协议

每次继续实施先执行 `git status --short --branch` 和 `git log -1 --oneline`，确认工作树与本台账锚点。主线程一次只推进一个可提交切片；只有目录边界独立、能并行验证且能实质减少主线程负担时，才使用 1-2 个 Agent。Agent 不提交，主线程统一审阅、验证和 commit。

每个切片按以下顺序收口：

1. 明确产品状态、数据权威和文件边界。
2. 完成最小实现与定向测试。
3. 对 UI 使用相同视口和状态做源/目标并排比较，并检查动态交互、Console 与 Network。
4. 运行与风险匹配的 Rust/UI 检查。
5. 仅暂存该切片，执行 `git diff --cached --check` 后独立提交。
6. 只有阶段状态或门禁变化时更新本台账，避免把过程日志写成文档噪声。

## 2. 已完成的产品主体

| 阶段 | 当前实现 | 关键提交锚点 |
| --- | --- | --- |
| P1-P2 | 受保护运行时拆分、根主题、全屏 Focus Shell、侧栏入口与返回工作台 | `ed9c66df`…`954d6dcb` |
| P3 | `nomifun.creative-studio/v1`、UUIDv7、项目 CRUD/CAS、项目中心、自包含 ZIP 导入导出与完整引用重映射 | `d03a6a64`、`b4361084`、`44933dd3`…`2ad53b01`、`c12b8db` |
| P4 | 画布 reducer/history、视口、节点、连线、小地图、选择/分组/快捷键、Editor CAS、离开/reload 门禁；首节点居中、参考节点几何、固定创作配色、直接节点工具、图片节点生成/上传面板、图片/视频/音频持久 composer 草稿与双客户端冲突恢复；空视频节点 T2V/单图 I2V、空音频节点 TTS 与真实任务终态回填 | `b2e19806`…`dc6c3c34`、`dd18dc6f`、`451dc013`、`5352954c`、`b2e103c8`、`46c1ba1e`、`777caba7`、`ef26f4cb`、`60982e38`、`9fffd526` |
| P5 | Canonical 资产 API/库、文本/图像/视频/音频节点、素材选择与结果回填 | `57128727`、`e05b18a8`、`04f805a3`、`444db764` |
| P6 | NomiFun 精确任务模型目录、幂等任务、canonical owner、取消/恢复、pending 引用持久化；画布视频/音频 owner、提交响应不明状态确认与 authoritative 404 orphan 清理；真实 `standalone_workbench` owner、完整有序输入快照与 owner-scoped keyset 历史读取合同 | `46545c21`、`b27d70d5`、`d5179e77`、`9897cc44`、`60982e38`、`9fffd526`、`8e5f83cf`、`1dd4b9e9` |
| P7 | 生图/视频工作台、工作流定义/运行中心、提示词与素材中心 | `2283ee74`、`1414846e`、`ebd17f3a`…`aad21d9d`、`7e45f8ad` |
| P8 | 持久化画布 Agent、原子操作、裁剪/切分/遮罩编辑与真实任务回填 | `e5368474`…`00f407c6`、`cac4bca8`…`bc01849e` |
| P9 | Director v1 domain、Three.js runtime、CAS sidecar、时间轴、截图回填、sidecar 全资产闭包与归档重映射 | `1ccbd013`、`d3a609f6`、`7b7712c0`、`dfa3c0b3`、`f25825f1`、`c12b8db` |
| P10 | 旧 Workshop UI、翻译、后端路由、旧画布存储与旧任务归属退出运行链 | `63c99d4f`…`7867fe3f` |

主体采用 React/Vite + Rust 原生集成，没有 Next.js、Go sidecar、iframe 或第二套模型配置。模型调用只走 NomiFun 的 provider/model/capability 解析和任务体系。

## 3. 已完成的 UI 证据

- Focus Shell：WebUI 与真实 Tauri 已验证入口、返回、刷新、深链、主题和原生窗口控制。
- 素材中心：1440×900 浅色源/目标对照完成；真实分页、搜索提交、上传限制、编辑、删除与合集重命名已有定向测试。
- 提示词中心：1440×900 浅色源/目标并排对照完成；7 个固定上游共同步 1592 条。搜索 Enter、分类、详情、复制、加入素材、刷新恢复及 30→60 渐进加载均真实通过；干净重载 Console 无错误。
- 提示词对照图：`C:\Users\MINISFORUM\.codex\visualizations\2026\08\19\01a01aa7-ea42-76e3-aa34-9158a1382c97\32-prompts-accepted-source-target-comparison.png`。
- 项目中心：新建/打开/返回、重命名与重载持久化、选择态、真实 ZIP 导出/导入、删除确认与取消均已验证；隔离导入副本通过精确 ID 清理。1440×900 浅色/深色与 1024×768 深色源/目标同状态对照完成，干净重载 Console 无错误。
- 项目中心对照图：`C:\Users\MINISFORUM\.codex\visualizations\2026\08\19\01a01aa7-ea42-76e3-aa34-9158a1382c97\38-projects-accepted-source-target-comparison-light-1440x900.png`、`41-projects-source-target-comparison-dark-1440x900.png`、`44-projects-source-target-comparison-dark-1024x768.png`。
- 画布主链：隔离新项目真实完成文本/图片节点创建、拖拽、连线、框选、分组/取消分组、缩放、撤销/重做、背景、小地图、属性编辑与重载持久化。首节点现在以参考产品世界原点居中，重置视图不再把节点推向右下；8 类节点默认尺寸、节点外壳、连线和占位态已按参考实现收敛。
- 画布采用参考产品浅色 stone 中性配色作为固定创作表面，不跟随用户主题。1440×900 像素比较确认切换用户浅色/深色后仅顶部 65px 外层 NomiFun 导航变化，画布及创作控件区域逐像素一致。参考/目标对照图：`C:\Users\MINISFORUM\.codex\visualizations\2026\08\19\01a01aa7-ea42-76e3-aa34-9158a1382c97\68-canvas-reference-target-focused-comparison-light-1440x900.png`；主题隔离对照图：`65-target-canvas-fixed-palette-stable-theme-comparison.png`。
- 底部工具栏已按参考产品顺序直接暴露文本、图片、视频、音频、全景、导演台和生成配置；分组继续是选择动作，不伪装为新节点类型。真实点击“视频”后节点数从 2 变 3，撤销后回到 2，重载确认没有残留；1440×900 对照图：`71-canvas-direct-toolbar-source-target-comparison-light-1440x900.png`。
- 单选图片节点现在展开参考式 580px 生成面板：空图片使用精确 `image_generation` / `t2i`，已有真实素材使用 `image_edit` / `i2i`；提示词库、精确 NomiFun 模型、接口模式、质量、比例、尺寸、张数、幂等提交、取消、恢复和真实素材回填均走现有 canonical runtime。空图片首个结果原位填充，额外结果成为 config-linked 图片；已有图片永不被覆盖。无模型时保持明确禁用，没有伪造 provider、积分或 Camera Control。
- 图片生成面板沿用参考项目的固定浅 stone 配色，主面板、设置弹层和嵌套下拉均不受用户主题影响。面板会按剩余空间在节点上下翻转，1024×768 自动横向钳制，390×844 在画布列无法容纳时切为 16px 视口边距浮层；新浏览器会话完成 1440→1024→390→1440 动态门禁，Console 0 error / 0 warning。参考与目标截图：`C:\Users\MINISFORUM\.codex\visualizations\2026\08\19\01a01aa7-ea42-76e3-aa34-9158a1382c97\creative-studio-image-composer-qa\reference-image-composer-light-1440x900.png`、`target-image-composer-clean-final-light-1440x900.png`。
- 画布 CAS 已用两个真实浏览器客户端验证：A 保存新 panel revision，B 从旧 revision 写入时后端返回稳定 `REVISION_CONFLICT`；B 回到旧本地基线后仍保持冲突，返回项目被阻止，显式载入会取得 A 的权威状态，之后可按新 revision 继续保存。通用业务 `CONFLICT` 不会再被误判为 revision 冲突。产品只显示一份中文恢复条，不泄漏项目 ID 或后端诊断；隔离项目最终恢复为“画布”面板。冲突截图：`C:\Users\MINISFORUM\.codex\visualizations\2026\08\19\01a01aa7-ea42-76e3-aa34-9158a1382c97\creative-studio-image-composer-qa\target-canvas-cas-conflict-final-1440x900.png`。
- 单选空图片现在按参考产品显示固定深色“信息 / 删除 / 上传图片”工具条；信息打开真实属性面板，删除只删除节点/边，上传打开单文件图片选择器。上传使用 64 MB/类型前置校验、唯一 operation tag 与 response-loss 找回，重新读取最新 Editor 后原位更新同一 nodeId，并立即 CAS flush；Undo/Redo 保持同一节点，素材始终保留在素材库。pending T2I 同时保护 config owner 与 source image，用户删除/上传不能破坏终态回填。多选时工具条与 composer 均隐藏。1440×900 与 390×844 均完整可见且主题不影响工具条配色；文件选择器已真实打开，但未把本地文件写入测试后端。截图：`C:\Users\MINISFORUM\.codex\visualizations\2026\08\19\01a01aa7-ea42-76e3-aa34-9158a1382c97\creative-studio-image-composer-qa\target-image-node-toolbar-final-light-1440x900.png`、`target-image-node-toolbar-mobile-390x844.png`。
- 画布 reload 门禁已补强：动态 import/chunk 失败不再用必然复用 rejected `React.lazy` Promise 的原地重试，而显示“重新加载页面”；完整 reload 已真实恢复曾失败的 Canvas route。已 hydration 且仍有 pending document 时，`beforeunload` 会启动同一 CAS flush 并请求浏览器原生离开确认，不再静默穿过 600ms debounce。隔离项目的即时 panel 变更在 reload 后恢复，随后已还原“画布”基线。
- 图片节点 composer 的未提交提示词、精确 Provider/model、接口模式、质量、宽高、比例和张数现在由图片节点 canonical `data.composer` 持有，不再依赖 React 临时 map。恢复顺序是节点草稿 → 最近一次已提交 config → 产品默认值；显式空提示词不会错误回退旧 config。旧 v1 图片缺少该可选字段时，双端解析器规范化为 `null`；新节点与新序列化始终写出字段。
- 草稿更新走完整 `canvasCommands.updateNode`、统一 `image-composer:<nodeId>` history merge key 和现有 CAS 队列，因此 Undo/Redo、远端 reload 与 Provider 清理只有一个数据权威。空图通过上传或首个 T2I 结果变成有图时，只清除不再适配 I2I 的模型选择，提示词与其他设置保留；Provider 删除也只清目标 draft model，其他草稿及无关 Provider 保持不变。
- 真实浏览器在 1440×900 依次验证唯一 marker、Responses、高质量、16:9、3 张、Undo、Redo、完整 reload 和第二个干净客户端，刷新后所有字段精确恢复；随后隔离项目已恢复为空提示词与默认 Images/自动/1:1/1 张。新会话 Console 0 error / 0 warning。空图片的提示词库入口不再误插入文本节点；composer 覆盖到更高 z-index 的相邻节点时也不会再点穿，节点总数保持 3。
- 单项目 ZIP 现在从外层画布文档递归闭合 Director `sceneId` sidecar 及其中的 panorama、character、object、capture 资产，未发送到画布的截图也不会遗漏。导入先建立完整 asset/node identity map，再重写 sidecar `projectId`、所有嵌套 `assetId`、图片 composer/mask 的 `sourceNodeId`、`sourceAssetId` 与 `markedReferenceAssetId`，并重算 sidecar byteLength/SHA-256；内部 camera/entity/timeline 身份保持不变。
- 归档只允许当前四种已知引用型 operation：图片节点生成、图片遮罩编辑、视频节点生成和音频节点生成；未知 operation、缺依赖、悬空 source node、错误 Director envelope/project ownership、非法 asset ID、额外 entry 和 checksum/budget 漂移均失败关闭。合法 Director 可能拥有 5000 个 capture 与 2000 个 entity asset，因此资产上限已与共享 hardened ZIP 20000-entry 门禁对齐，仍受 256 MB 解压总预算和 manifest 大小约束，不会截断。
- 导出、单资产删除保护、项目删除清理和启动 managed-data audit 现在共享同一个异步 Director 资产闭包。三套全新 service/data root 已真实完成 A 导出 → B 导入/资产读取/再导出 → C 再导入，B 删除项目后 sidecar、全景和未发送截图均确认删除。隔离 Web 项目中心也从新后端真实导出 1 个 ZIP 并显示成功，Console 0 error / 0 warning。
- 单选空视频节点现在展开参考式固定浅 stone 视频 Composer。模式由直接连入的真实媒体自动推导：无引用为 T2V，恰好一张真实图片为 I2V；视频/音频引用、多图和非空视频目标明确禁用，当前后端不支持的 V2V 不会伪装成可用。模型只从 NomiFun 精确 `video_generation` Provider/model 目录选择；无兼容模型时生成按钮真实禁用，不显示参考项目中没有后端契约的 credits、Camera Control、首尾帧或混合引用。
- 视频节点 canonical `data.composer` 持有提示词、精确模型、720p/1080p、16:9/9:16/1:1 与 5/10 秒草稿；画布本地 owner 身份单独保存在强类型 `data.operation`。Provider 参数只包含真实 prompt、seconds、width、height，720p/1080p 会映射为确定尺寸；后端同时剥离旧本地元数据，避免 `canvasOperation/sourceNodeId/sourceAssetId` 等泄漏到 provider `extra`。旧 v1 config 可规范化迁移，归档会重映射 operation 的节点/资产引用。
- 每次视频生成固定一个 canonical config owner 和一个任务：pending 在 POST 前完成 CAS，queued/running 只做 runtime reconcile，刷新后按 exact owner 恢复；成功结果必须解析为真实 video asset，第一个结果原位填充空视频节点并保持 node ID，额外结果成为 config-linked 视频节点，重复 settlement 幂等且 pending 最后移除。失败、取消和 authoritative 404 均保留可审计 config；提交结果不明时可复用同一幂等键重试，也可显式“确认任务状态”，仅在后端确定 404 时清理恢复标记。
- 真实浏览器完成唯一 marker、720p、9:16、10 秒、Undo、Redo 与完整 reload；1440×900、1024×768、390×844 均可操作。移动端设置按钮只隐藏摘要、保留真实设置图标；切换应用浅/深主题前后 Composer 的背景与文字计算色完全一致。最终用 UI 删除 QA 视频节点并重载回原 3 节点；最后新页面会话 Console 0 error / 0 warning。没有调用付费 Provider，也没有上传本地文件。
- 单选空音频节点现在展开固定浅 stone 朗读 Composer，只走精确 `speech_synthesis` / `tts`、零输入和单任务。朗读正文、精确 Provider/model、Voice ID 与 MP3/WAV 草稿由 audio node canonical `data.composer` 持有；本地 source 身份只存在强类型 `audio-node-compose` operation，Provider 参数没有 canvas/node/asset 元数据。成功任务必须恰好返回一个真实 audio asset，并原位填充同一个节点 ID；失败、取消、响应丢失重试、恢复、authoritative 404 与 pending-last 均复用统一任务合同。
- 音频可选字段按八个现有 adapter 的 exact protocol profile 显示：未知协议降级为 prompt-only，Deepgram 正文上限 2000，要求 Voice ID 的协议会在本地阻止空音色提交；format 只允许 artifact gate 已支持的 MP3/WAV。Voice ID 只在同一 Provider 且同一协议内保留，跨 Provider/协议、失效模型或单模型自动接管都会清空并要求重新确认。首批明确不发送 speed、instructions、参考音频、VoiceClone、AAC 或 PCM。
- 真实浏览器完成音频唯一 marker、Undo、Redo、完整 reload 和提示词库入口；1440×900、1024×768、390×844 的 Composer 均完整可操作。空节点不再显示无意义的 `0:00 – ∞`；浅/深主题切换前后面板背景与文字计算色完全一致。最终用 UI 删除 QA 音频节点、恢复画布面板并重载回原 3 节点；最后新页面会话 Console 0 error / 0 warning。参考项目仍在 3000 打开并复核音频入口；没有调用付费 Provider、没有上传本地文件。
- migration 043 新增第三种 canonical task owner：`standalone_workbench { projectId, workbenchKind }`，其中 kind 仅允许 image/video/audio，并与 canvas node、workflow step 三分支严格互斥。Creation 会校验 workbench kind 与 capability 媒体族匹配；owner、Provider、模型、capability、params 和有序 inputs 全部参与同一幂等一致性比较，旧 key 不能换 owner、换输入或换顺序。
- 每个新任务显式持久并返回有序 `{ assetId, kind, role }` 输入快照；`kind` 只允许 image/video/audio/text，worker 会用真实 MIME 再校验声明。迁移旧任务只有在 fingerprint 含完整有序 inputs、每个 asset 仍存在且 kind 可证明时才恢复；否则响应为 `inputs:null`，前端将其视为“不可原参数重试”，绝不伪装成 `[]`。新零输入任务始终返回 `inputs:[]`，create 响应若不精确回显输入快照会失败关闭。
- 输入与结果资产进入 DB logical-reference registry；新 standalone 产物 origin 使用 `project_id + workbench_kind + creation_task_id`，并在写入时与真实任务 owner 逐字段比对。043 的旧输入解析、insert/update input trigger 与 asset-origin owner trigger 均使用 NULL-safe SQLite 判断，缺字段、显式 null、重复键、未知字段或混合 owner 都不能通过三值逻辑绕过。
- `GET /api/creative-studio/tasks` 已提供严格 owner-scoped 历史分页：必填 canonical project/kind，默认 30、上限 100，按 `submitted_at DESC, creation_task_id DESC` 使用二列 keyset，只有存在下一页时返回最后可见任务的 cursor。未知/重复 query、非法 UUID/kind/limit/cursor、owner 或 capability 逃逸均失败关闭；返回成功资产仍经过现有 artifact audit。
- 前端 history client 会验证精确字段、owner、排序、页长、请求 cursor 窗口与返回 cursor 锚点；history model 将同一任务的持久/live 状态合并但不拆分多结果，不允许陈旧 live 降级 durable 终态，也不允许把其他任务资产挂入输出。冲突终态失败关闭，只有 failed/canceled 且 `inputs` 可证明的任务允许精确重试。
- 当前独立图片/视频产品路由仍有意保留 config-node owner，直到下一切片同时切换 POST owner 并消费 owner-scoped list/recovery。底层历史 API、strict wire 和纯合并模型已经就绪，但不把尚未接到产品路由的持久历史描述成可用 UI 功能。

`dd18dc6f` 的提交前检查：

- 画布目录完整测试：193 passed / 1051 assertions。
- `bun run typecheck`、`bun run check:theme`、`bun run check:dead-css`、`git diff --cached --check`：通过。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。

`451dc013` 的提交前检查：

- 画布目录完整测试：194 passed / 1066 assertions。
- `bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --cached --check`：通过。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。

`5352954c` 的提交前检查：

- Canvas product + imageTools：113 passed / 624 assertions。
- T2I/I2I composer 精确契约：空图片无伪输入素材、首结果原位填充、多结果幂等与 confirmed-404 单项清理均有测试。
- `bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --cached --check`：通过。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。
- 真实浏览器：1440×900、1024×768、390×844，浅色/深色外层主题、设置弹层、下拉、Enter/Shift+Enter、无模型禁用与提示词库入口通过；未触发付费生成。

`b2e103c8` 的提交前检查：

- Canvas editor/product + project repository：98 passed / 642 assertions。
- `nomifun-workshop`：70 passed；`nomifun-common` error 相关 unit/integration：56 passed。
- `cargo check -p nomifun-app --lib`、Rust 定向 fmt、`bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --cached --check`：通过。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。
- 真实双客户端完成 stale 409、回旧基线仍锁定、离开阻止、显式远端 reload、新 revision 续存与单一恢复 UI；Console 仅有预期的 stale PUT 409 记录，无其他 warning/error。

`46c1ba1e` 的提交前检查：

- Canvas 全目录：216 passed / 1168 assertions。
- `bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --cached --check`：通过。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。
- 真实浏览器：单选/多选、属性面板、文件选择器、浅/深主题、1440×900 与 390×844 自适应通过；干净桌面/手机页面 Console 0 error / 0 warning。

`777caba7` 的提交前检查：

- Route boundary + Canvas editor：33 passed / 176 assertions。
- `bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --cached --check`：通过。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。
- 真实浏览器复现 Canvas 动态模块加载失败；原地“重试”确认无效，完整页面 reload 恢复。新 matcher 覆盖 Chromium/Safari/Vite/Webpack 常见 chunk 文案。

`ef26f4cb` 的提交前检查：

- Canvas 全目录 + Creative Studio domain contract：229 passed / 1259 assertions；composer 定向复跑：32 passed / 233 assertions。
- `cargo test -p nomifun-workshop --lib`：72 passed；Provider cleanup 与 image composer exact pair 定向复跑通过。
- `cargo check -p nomifun-app --lib`、Rust 定向 fmt、`bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --cached --check`：通过。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。
- 真实浏览器：参考项目仍运行在 3000，目标运行在隔离 5174/8788；marker 与全部设置跨 reload/第二客户端恢复，Undo/Redo 和空图片提示词入口通过，未触发付费生成或写入本地文件。

`c12b8db` 的提交前检查：

- `cargo test -p nomifun-workshop --lib`：75 passed；包含真实 Director v1 sidecar、两类 canvas config 参数、缺失/非法依赖，以及三套全新 data root 的导入/再导出/删除集成门禁。
- `cargo check -p nomifun-app --lib`、Rust 定向 fmt、`bun run check:dead-css`、`git diff --cached --check`：通过；输出仅有其他 crate 的既存未使用代码警告。
- 真实浏览器：8788 后端以原隔离数据目录重启，现有画布正常重载；项目中心真实导出“无限画布 2”并提示“已导出 1 个画布”，回归时间窗内 Console 0 error / 0 warning。
- 参考项目保持运行在 3000，目标保持运行在 5174/8788；没有付费模型调用、没有上传本地文件、没有把隔离归档重新写入用户正式数据根。

`60982e38` 的提交前检查：

- Canvas 全目录 + Creative Studio domain contract：257 passed / 1448 assertions；最终视频路由/Composer 定向复跑：15 passed / 177 assertions。
- `cargo test -p nomifun-creation --lib`：61 passed；`cargo test -p nomifun-workshop --lib`：78 passed；覆盖 provider extra 隔离、视频草稿/operation 双端严格解析、Provider cleanup、归档重映射与终态运行时。
- `cargo check -p nomifun-app --lib`、Rust 定向 fmt、`bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --cached --check`：通过；Rust 输出仅有其他 crate 的既存未使用代码警告。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。
- 真实浏览器：参考项目运行在 3000，目标运行在隔离 5174/8788；桌面/中窄屏/390px、固定主题、草稿保存/撤销/重做/reload、无模型禁用、移动设置图标与 QA 清理通过，最终干净会话 Console 0 error / 0 warning；未触发付费生成。

`9fffd526` 的提交前检查：

- Canvas 全目录 + Creative Studio domain contract：286 passed / 1654 assertions；音频 domain/runtime/bridge/Composer、协议 profile、Voice ID 作用域、pending guard、旧 v1 草稿和路由接线均有定向门禁。
- `cargo test -p nomifun-workshop --lib`：81 passed；`cargo test -p nomifun-creation --lib`：62 passed；覆盖 audio draft/operation、Provider cleanup、归档 remap、TTS typed 字段与 provider extra 隔离、真实音频 artifact。
- `cargo check -p nomifun-app --lib`、Rust 定向 fmt、`bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --cached --check`：通过；Rust 输出仅有其他 crate 的既存未使用代码警告。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。
- 真实浏览器：参考项目运行在 3000，目标运行在隔离 5174/8788；草稿保存/撤销/重做/reload、提示词库、无模型禁用、1440/1024/390、固定主题和 QA 清理通过，最终干净会话 Console 0 error / 0 warning；未触发付费生成。

`8e5f83cf` 的提交前检查：

- `cargo test -p nomifun-db --lib`：387 passed；`id_schema_contract`：22 passed；043 owner/input migration：5 passed。覆盖三 owner 分支、旧输入可证明恢复/不可证明 NULL、input trigger 缺失/null/重复键、asset origin insert/update 与幂等输入顺序。
- `cargo test -p nomifun-creation --lib`：65 passed；`cargo test -p nomifun-gateway --lib`：142 passed；standalone DTO、kind/capability、input kind/MIME、origin 与 Gateway kind 证明通过。
- `cargo check -p nomifun-app --lib`、Rust 定向 fmt、`bun run typecheck`、图标/主题/dead-css、UI production build、`git diff --cached --check`：通过；仅有仓库既存未使用代码与 Vite chunk 提示。
- Creative Studio 广域 UI 测试运行得到 604 passed / 3 failed；本切片的 WIRING 断言已修正并定向复跑通过，剩余 2 项是未改动的 Director capture 几何旧期望（期望 x=900/y=2160，当前 canonical 结果 x=920/y=2320），不作为本数据合同提交的伪阻断，也未篡改测试掩盖。
- 隔离 8788 在原 QA 数据根真实应用 043 后正常启动；提交 `image` owner + `t2v` 的错误媒体族请求返回 400，随后同 task ID GET 为 404，证明门禁发生在 Provider/任务落库前。现有画布重载仍为原 3 节点，Console 0 error / 0 warning；未调用付费 Provider。

`1dd4b9e9` 的提交前检查：

- `cargo test -p nomifun-creation --lib`：67 passed；SQLite standalone owner/keyset 定向门禁通过。覆盖严格 query/cursor、limit + 1、两列倒序分页、owner/capability 二次审计与结果资产审计。
- standalone history client/model/workbench planner：24 passed / 105 assertions；覆盖 page/cursor 精确绑定、三类 owner 媒体族、durable/live 单调合并、终态冲突、结果资产同序绑定、legacy retry 禁用与多结果不拆分。
- `bun run typecheck`、Rust 定向 fmt、`git diff --cached --check`：通过。UI production build 通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。
- 隔离 8788 真实 GET 空历史返回 200/`next_cursor:null`，非 canonical cursor 返回 400；同一项目 API 经 8788 与 5174 代理均返回 200。完整重载后画布为原 3 节点/0 连接，结构树恢复且新鲜 Console 0 error / 0 warning；未触发付费生成。

`7e45f8ad` 的提交前检查：

- `cargo test -p nomifun-workshop`：70 passed。
- `cargo check -p nomifun-app --lib`：通过；输出仅有其他 crate 的既存未使用代码警告。
- 提示词 + 资产 client + canvas node factory：40 passed。
- `bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`cargo fmt --check`、`git diff --check`：通过。

`03b8d591` 的提交前检查：

- 项目中心定向测试：16 passed / 113 assertions。
- `bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --check`：通过。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。

## 4. 仍需明确保留的限制

- Standalone 工作台必须显式选择真实项目；不会借用或偷偷创建最近项目。
- `standalone_workbench` owner、完整 inputs 快照、owner-scoped list client 和 durable/live 合并模型已经落地，但图片/视频产品尚未消费它们，仍使用旧 config-node 持久化。下一切片必须同时切换 POST owner、启动恢复和终态历史，不能只改其中一段制造刷新后失联任务。
- 迁移旧任务的 `inputs:null` 表示输入快照无法证明，只允许审计展示，必须禁用精确重试；不能把它归一为空数组或猜测素材 kind/role。
- 当前 standalone 路由仍由只拥有一个 `taskId` 的 v1 config node 持久化，因此视频批量数固定为 1。要支持 1-6 并行任务，必须先切换到已落地的 standalone owner/history，并让 owner-scoped recovery 与多任务 UI 同步接通。
- 画布视频当前只开放空节点 T2V 与一张直接连接真实图片的 I2V。Creation engine 对 V2V 返回 `unsupported_capability`；首尾帧、多图、视频/音频混合参考和 Provider 特有高级参数必须等协议能力矩阵完成后再开放。
- 视频任务“取消”是 NomiFun 本地权威取消：会阻止晚到结果覆盖和入库，但现有 adapter 没有远端 cancel 契约，不能承诺 Provider 作业或计费必然停止。
- 画布音频当前只开放空节点 TTS，零输入且每次一个真实音频结果。参考音频、VoiceClone、声音设计、speed/instructions、AAC/PCM 与音频到音频必须先扩展 typed TTS 输入、协议 profile 和 artifact gate，不能只添加 UI 控件。
- 音频任务取消同样是本地权威取消；现有 TTS adapter 没有远端 cancel 方法，不能承诺 Provider 同步请求或计费已经停止。
- 手工素材上传当前只支持图片与视频；音频可由生成任务入库，但不伪装成已支持的拖放上传。
- Director 的 GLB/glTF 模型导入仍不可用；四/十二方位批量捕获和视频导出保持显式不可用。当前不使用参考项目的压缩 Director bundle 或来源不清模型。
- 390px 只承诺关键入口和状态不白屏；尚未宣称完整移动触控生产体验。
- 浏览器刷新/关闭时会在 pending revision 上启动 flush 并请求原生确认；浏览器仍不能等待异步持久化，用户若明确接受离开，最后一段编辑仍可能尚未得到服务端确认。Tauri 普通关闭只隐藏窗口，renderer 会继续保存。
- 真实付费 provider 端到端冒烟尚未执行，需要可用凭证和单独的成本授权。
- `data.composer` 是协调发布的 v1 可选扩展：当前前后端均向后读取缺字段文档，但 `ef26f4cb` 之前带 `deny_unknown_fields` 的旧二进制不能读取新字段，不能把新数据库回退给旧程序。
- 整个 Provider 删除会清理 config 与图片 composer 引用；单独删除一个 Provider model 仍未经过 Creative Studio 清理协调器，这是 config 节点既存但必须在最终模型管理门禁消除的系统性缺口。

## 5. 下一步单线程优先级

1. Standalone：基础 owner、完整任务快照、可分页 list client 与纯历史合并合同已经完成；下一提交把图片/视频产品路由成套切到真实 owner/recovery/终态历史，随后实现引用安全软删除。不要继续用一个 config node 或 React hidden IDs 冒充历史。
2. 模型与 Agent：消除单模型删除的 Creative Studio 引用缺口，再补首页 Agent 启动画布、选择上下文/创作技能和 Workflow AI 创建。
3. 高级媒体：视频多任务/首尾帧/高级引用、音频上传/时长/VoiceClone、Director/全景与完整视频输出均须先补 typed 后端能力。
4. P11：1280×720、1024×768、390×844、Tauri 慢环、真实 provider 冒烟、UI build/桌面打包和最终能力文档。

不要因为主体代码已存在就跳过这些门禁；画布 image/video/audio 基础生成闭环和 standalone 历史数据合同已完成，下一功能提交从第 1 项图片/视频产品路由的 owner/history/recovery 协调切换开始。
