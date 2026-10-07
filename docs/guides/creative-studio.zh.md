# 创作（Creation）

创作是 NomiFun Desktop 中专注、本地优先的创作产品，核心为：

- **Canvas**：持久化无限画布，包含媒体节点、可审计的生成操作、可复用素材和
  私有模板；
- **素材库资源**：素材、提示词库与私有模板，均可从主侧边栏直接进入。

创作没有 Project 产品对象。Canvas 就是 Canvas。生成使用 NomiFun 现有的
Provider 与模型目录，不维护第二套模型配置系统。原独立 Image/Video Workbench
页面已退役；生成任务通过 Canvas 节点与模板步骤完成。

> English: [creative-studio.md](creative-studio.md)

## 打开产品

主侧边栏直接暴露创作资源入口：**我的画布**（`/nomi/canvases`）、
**素材库**（`/asset-library/materials`）、**提示词库**
（`/asset-library/prompts`）和**模板工作台**（`/asset-library/templates`），
位于“数据空间”分组。我的画布入口会恢复当前应用会话中最后一个有效的创作
地址，包括完整查询参数和页内锚点；保存的地址如果非法、未知、外部或超长，会
fail-closed 回退到 `/nomi/canvases`。通过需求发起创作的能力由已打开 Canvas
内的**创作助手**提供。

规范路由面如下：

| 路由 | 用途 |
| --- | --- |
| `/nomi/canvases` | 创建、重命名、打开、导入、导出和删除 Canvas。 |
| `/nomi/canvases/:canvasId` | 编辑一个 Canvas 的 canonical 无限文档。 |
| `/asset-library/materials` | 在“我的素材”中管理可复用素材。 |
| `/asset-library/prompts` | 在提示词库中管理提示词。 |
| `/asset-library/templates` | 在模板工作台中管理私有模板。 |

已退役的 `/workshop/*` 路径不再作为路由挂载。保存的 `/workshop`、
`/workshop/canvases`、`/workshop/projects` 或 `/workshop/canvas/:canvasId`
恢复地址由
[`resourceRoutes.ts`](../../ui/src/renderer/pages/creativeStudio/app/resourceRoutes.ts)
中的 `migratedCreativeRoute()` 单向改写为对应的 `/nomi/canvases` 目标。
独立 Image/Video Workbench 页面已退役；生成任务通过 Canvas 节点与模板完成。

## 领域边界

任务 owner union 保持有意的最小形状 —— 创建任务的 wire 契约只接受两种
owner，service 层另支持会话轮 owner：

| Owner | 身份 | 使用场景 |
| --- | --- | --- |
| `CanvasNode` | `{ canvasId, nodeId }` | 从 Canvas 节点发起的任务。 |
| `TemplateStep` | `{ templateId, templateRunId, templateStepId }` | 模板执行。 |
| `ConversationTurn` | `{ conversationId, messageId }` | 绑定到会话轮次的创作任务。 |

只有从 Canvas 节点发起的任务才拥有 Canvas owner。已退役的
`standalone_workbench` owner kind 会被 wire 契约拒绝；旧行可以保留 legacy
`project_id` 作为 inert provenance，但它不参与 owner equality、历史分页、
退役、素材 origin 匹配或 Canvas 删除。

删除 Canvas 只受该 Canvas 的 live `CanvasNode` 任务限制。

## 画布模型

每个 Canvas 持久化一份带版本的 `nomifun.creative-studio/v1` 文档。图中恰好有七类
canonical 节点：

| 节点 | 当前职责 |
| --- | --- |
| `text` | 纯文本或 Markdown 内容。 |
| `image` | 真实图片素材、空图片承接节点及其持久 T2I/I2I Composer 草稿。 |
| `video` | 真实视频素材或带持久 Composer 草稿的空 T2V/I2V 承接节点。 |
| `audio` | 真实音频素材或带持久 Composer 草稿的空 TTS 承接节点。 |
| `timeline` | 持久化的图片/视频编辑时间线及其片段排列。 |
| `config` | exact 生成操作、参数、任务状态、输入与结果的可审计 owner。 |
| `group` | 对已有选区执行分组后产生的容器；它不是生成器。 |

Generator、Loop、Compare 与 Output 不是 canonical 节点类型。生成由媒体节点与
`config` 共同表达；分组是明确的选区动作。

Canvas 在受支持的桌面外壳与桌面浏览器 WebUI 视口（880x600 及以上）中支持选择、移动、
缩放节点、连线、分组、复制/粘贴、撤销/重做、画布缩放、重置/适配视图、小地图导航与重载。

画布编辑使用短延迟 debounce 的 compare-and-swap（CAS）保存，每次写入都带上最后
一版权威 revision。发生冲突后自动保存会停止，不会强写，也不会覆盖新版本后静默
重试。请通过界面载入权威远端版本，再重新应用想保留的改动。离开创作页面前会
flush 待处理的 Canvas 写入；结果不安全时会阻止离开。

Canvas Agent 产生的是提案，不是后台改图。支持的提案 artifact 会 fail-closed 解析，
只有用户点击**应用到 Canvas**才会执行 Canvas CAS 写入。删除和媒体生成不属于这套
提案子集。

## Canvas API 与 Gateway

规范 HTTP 资源是：

- `GET/POST /api/creative-studio/canvases`
- `GET/PATCH/DELETE /api/creative-studio/canvases/:canvasId`
- `PUT /api/creative-studio/canvases/:canvasId/document`
- Canvas Agent 操作和归档操作也挂在同一个 Canvas 资源下。

旧 `/api/creative-studio/projects` 路由仅作为 deprecated 兼容 alias 保留。旧
`project/projectId` 名称只表示历史 wire 兼容，不代表当前创作仍有 Project
领域对象。

进程内 Gateway 暴露 Canvas-first capability：
`nomi_creative_studio_list_canvases` 与
`nomi_creative_studio_get_canvas`，以及素材、apply-ops、生成和任务 capability。
旧 `nomi_creative_studio_list_projects` 与
`nomi_creative_studio_get_project` 是 deprecated legacy alias。它们都属于
instance-owner capability，只对策展的 `desktop` 与 `admin` Gateway profile 可见；
`work`/`lite` profile、普通会话、伙伴与非 owner 调用方无法发现或执行。

这次 wire 变更的 UI/API contract version 是 **21**。

## 精确模型与任务路由

一次模型选择是 exact `{ providerId, model }`。创作按所需任务查询 NomiFun
托管模型目录，并排除已禁用的 Provider、已禁用模型，以及只声明了相邻任务的模型。
系统不会通过模型名称猜能力，也不会静默替换成另一个任务。

| 操作 | 要求的 NomiFun task | 创作 capability |
| --- | --- | --- |
| Canvas 创作助手 | `chat` | Canvas-scoped Assistant turn；严格图提案仍需人工批准。 |
| 模板 AI 草稿/规划 | `chat` | 一次不带工具的有界 completion。 |
| 空图片承接节点 | `image_generation` | `t2i`。 |
| 带真实参考的图片（包括蒙版编辑路径） | `image_edit` | `i2i`。 |
| 空视频承接节点 | `video_generation` | `t2v`。 |
| 带恰好一张直接真实图片参考的视频 | `video_generation` | `i2v`。 |
| Canvas 空音频承接节点 | `speech_synthesis` | `tts`。 |

持久 operation 会把 Provider、模型、task、capability、有序输入素材绑定和类型化参数
放在一起。复用同一个幂等身份重试时，不能悄悄替换这些事实。删除 Provider 或单个
模型也会经过协调门禁，不能静默留下活跃任务或其他硬绑定孤儿。

## 已退役的独立工作台

独立 Image/Video Workbench 页面已退役，不再挂载路由。创建任务的 wire 契约
只接受 `canvas_node` 与 `template_step` owner；`standalone_workbench` owner
会被拒绝。旧 standalone 行仍可作为 provenance 读取，但不能精确重试。图片、
视频与音频生成现在通过 Canvas 节点 Composer 与模板步骤完成。

Canvas 节点 Composer 的草稿随 canonical Canvas 文档持久化，包含 prompt、
exact `{ providerId, model }` 身份、受控生成参数和有序 reference asset IDs。
hydrate 时缺失、不可读、类型不匹配、重复、超量或错误类型的引用会被移除，
不会恢复成过期浏览器对象；初始 hydrate 完成前不允许生成。已保存模型只有在
同一个 exact Provider/model 仍支持所需 task 时才恢复，不会用另一个 Provider
的同名模型替换。

## 提示词库与可复用输入

`/asset-library/prompts` 是独立的提示词管理页面，汇总三类来源：

- 从固定 allow-list 上游提示词仓库同步的、带来源与许可证信息的 offline-first 目录；
- 包含会话指令、且当前启用的 NomiFun 设定；
- 已经保存在创作素材库中的用户自有文字素材。

提示词库支持文字搜索、精确分类筛选、标签交集筛选、详情查看和复制。目录或设定中的
提示词可以显式保存为「我的素材」中的文字素材；目录来源会继续保留仓库与许可证
归属。由提示词库保存出的文字素材带有稳定的来源身份：它不会作为新提示词回流到
提示词库，同一来源也只会保存一次；详情页可以将它移出「我的素材」，该操作仅改变
素材库可见性，不删除素材记录或画布引用，并可随时再次加入。用户独立创建的文字素材
仍可作为提示词来源。成功同步一次后，有效缓存可以在离线时继续使用。

独立提示词页面有意不持有隐藏 Canvas 插入目标。复制或保存提示词不会创建 Canvas，
也不会自动开始生成；需要进入某条具体创作链路时，再从 Canvas 中选择对应的
文字素材。

## 素材、持久化与恢复

素材 metadata 保存在 SQLite；二进制原件与缩略图位于后端数据目录的
`workshop/assets/` 树下。素材库支持真实 `text`、`image`、`video`、`audio` 素材，
包含搜索、类型筛选、集合、标签、metadata 修改与复用选择器。二进制上传上限为
64 MiB。所有列表和写 API 都只允许实例 owner。
`GET /api/creative-studio/files/{assetId}` 是一个窄的只读例外：浏览器媒体元素无法
附带桌面 trust header，因此 opaque UUIDv7 作为 capability URL；它不是列表或写入接口。

Canvas 已提交工作拥有一个持久 `config` owner 和一个 canonical creation task。重载后，
界面只会按这个 exact owner 与权威任务状态对账。终态结算是幂等的；响应不确定时不会
虚构成功，也不会丢掉审计轨迹。权威 `404` 与暂时网络失败会被区别处理。

在「我的素材」中删除素材会永久删除原件及缩略图。画布节点、已完成的生成任务和
模板历史保留明确的「素材已删除」标记；重新载入、重放任务或复用旧素材 ID 都不会
恢复原始内容。已删除素材不会出现在可复用列表中。若仍有运行中的生成任务或模板
运行使用它，需要先等待任务结束或取消任务。

永久删除支持幂等重试。所有文件清理完成前，数据库会保留待清理路径；请求失败可
重试，中断的清理会在下次启动时继续。没有用户删除标记的意外文件缺失仍会触发
完整性检查。已下载到外部的文件及既有备份不属于此次删除范围。

## Canvas 归档

规范 Canvas 导出是版本 3 的 `*.nomifun-canvas.zip` 归档。manifest 使用 Canvas
身份，包含已校验的 Canvas 文档与完整引用素材闭包。导入会校验归档，并重映射 Canvas、
节点、连接、素材、operation 和 session 引用，避免导入副本与源对象共用身份。版本 3 以空内容项携带已删除素材的
标记，导入不会重新生成已删除的媒体文件。

reader 继续支持版本 2 Canvas 归档和已发布的版本 1 `.nomifun-canvas.zip` 格式。v1 manifest 可能包含
历史 `project/projectId` 字段；这些只是兼容 wire 数据，不会把 Project 重新引入产品。
Conversation 消息与活跃 pending turn 位于归档之外，导入不会克隆 Conversation。

归档不包含 Provider 凭据，也不会安装缺失的 Provider 或模型。全局模板与 Canvas
没有引用的素材不会被隐式塞进 Canvas 归档。

## 最小模板 AI

`/asset-library/templates` 的 **AI 创建**首发范围刻意保持简单：

1. 输入简单需求并选择一个 exact、已启用的 `chat` 模型。
2. NomiFun 执行一次不带工具的 completion：墙钟上限 120 秒、输出上限 4,096 token、
   本地响应上限 262,156 bytes。
3. 客户端只接受一个位于最终位置、结构严格的
   `nomifun.creative-studio.template-draft/v1` JSON artifact。草稿模式仅有
   `single-image` 与 `multi-image-series`。
4. 先审阅预览。**应用**只会把一个私有的内存草稿打开到现有模板编辑器。
5. 需要时继续编辑，然后点击**保存**。只有这次显式 Save 才创建模板；Apply
   不会持久化，也不会运行。

这次 one-shot 不创建 Conversation、附件、公开模板、Skill/MCP 工具会话、已保存模板
或模板运行记录；也不会自动重试、模型故障切换、保存或执行。模型不能决定 ID、
revision、时间戳、可见性、标签、媒体生成模型或素材。公开模板发布/发现与复杂
模板会话不在首发范围内。首发 UI 是 private-only：新建、编辑、复制与 AI Apply
都会把底层模板定义规范化为 `private`，界面不提供公开可见性开关。

## 当前限制

- 视频目前只支持 T2V 与单图 I2V；V2V、首尾帧、多图引用、视频/音频混合参考与未
  类型化的隐藏 Provider 参数都会被拒绝。
- Canvas 音频生成目前只支持零输入 TTS，并要求一个 MP3 或 WAV 结果。参考音频、声音
  克隆、音频到音频、speed/instructions、AAC 与 PCM 没有在本合同中开放。
- Provider 协议存在差异。只有 exact 类型化协议 profile 支持时才显示对应控制项；
  未知协议使用更小的安全子集。
- 默认标题栏和创作侧栏会跟随应用语言，但首发 Canvas 与编辑器的大部分正文仍以
  简体中文为主。
- 配置了模型不等于远端 Provider 可达，也不等于已经执行付费请求。生成前请留意
  Provider 的计费和数据政策。
- 浏览器窄屏布局验证不能证明完整触控设备支持。

## 如何理解验证结论

创作按层报告验证结果，避免把一个层级的成功误当成另一个层级：

1. **合同检查**：TypeScript/Rust 测试、schema 检查、typecheck、主题/图标/dead-CSS
   与编译，用于证明代码层合同。
2. **浏览器产品检查**：真实点击、重载、持久化计数、Console 与目标视口，用于证明
   被实际走过的 Web UI 链路。可以使用本地 mock Provider 在不消耗额度时闭环。
3. **宿主与产物检查**：Web/Tauri 慢环、UI production build 与平台打包，只证明
   对应宿主或产物，不能外推到另一操作系统。
4. **真实 Provider 检查**：只有经过明确授权、真正请求所选 Provider，才能证明实时
   凭据、厂商兼容性、延迟、计费与生成质量。
5. **发布检查**：成功生成安装包与代码签名、公证、Updater 校验、发布到 release
   channel 是不同门禁。

除非发布记录明确写明，否则不能从源码、单测、浏览器 mock、构建或打包成功推断已经
完成付费 Provider 冒烟，或已经签名/公开发布桌面版本。

## 实现索引

- 产品路由：[`app/resourceRoutes.ts`](../../ui/src/renderer/pages/creativeStudio/app/resourceRoutes.ts)
- Canvas 文档：[`creative_studio.rs`](../../crates/backend/nomifun-workshop/src/creative_studio.rs)
- Canvas、素材与模板路由：[`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs)
- 生成任务路由：[`nomifun-creation/src/routes.rs`](../../crates/backend/nomifun-creation/src/routes.rs)
- 模型选择：[`models/catalog.ts`](../../ui/src/renderer/pages/creativeStudio/models/catalog.ts)
