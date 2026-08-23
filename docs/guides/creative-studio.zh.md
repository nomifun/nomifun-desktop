# 创意工坊（Creative Studio）

创意工坊是 NomiFun Desktop 中专注、本地优先的创作产品，包含三个彼此独立的
创作面：

- **Canvas**：持久化无限画布，包含媒体节点、可审计的生成操作、可复用素材、
  私有模板和受限 Director。
- **Image Workbench**：独立的图片生成工作台。
- **Video Workbench**：独立的视频生成工作台。

创意工坊没有 Project 产品对象。Canvas 就是 Canvas。Image Workbench 和
Video Workbench 不要求、不推断、不选择、也不创建 Canvas；它们直接使用
NomiFun 现有的 Provider 与模型目录，不维护第二套模型配置系统。

> English: [creative-studio.md](creative-studio.md)

## 打开产品

从应用侧边栏打开**创意工坊**。页面复用 NomiFun 默认标题栏里的侧栏、历史、系统
窗口等控制；左侧主侧栏会像进入“设置”时一样切换为创意工坊内部导航，并且可以折叠
以释放工作空间。入口会恢复当前应用会话中最后一个有效的创意工坊地址，包括完整查询
参数和页内锚点。保存的地址如果非法、未知、外部或超长，会 fail-closed 回退到
`/workshop/canvases`。创意工坊不再提供独立首页项；通过需求发起创作的能力由已打开
Canvas 内的**创作助手**提供。待 Canvas 或 Director 的保存结果处理完毕后，点击侧栏
底部的**返回工作台**会回到 `/guid`。

规范路由面如下：

| 路由 | 用途 |
| --- | --- |
| `/workshop` | 兼容入口，重定向到 `/workshop/canvases`。 |
| `/workshop/canvases` | 创建、重命名、打开、导入、导出和删除 Canvas。 |
| `/workshop/canvas/:canvasId` | 编辑一个 Canvas 的 canonical 无限文档。 |
| `/workshop/director/:canvasId` | 编辑附属于该 Canvas 的受限 Director 状态。 |
| `/workshop/image` | 使用独立 Image Workbench；零 Canvas 时也完整可用。 |
| `/workshop/video` | 使用独立 Video Workbench；零 Canvas 时也完整可用。 |
| `/workshop/prompts`、`/workshop/assets`、`/workshop/templates` | 管理提示词、可复用素材和模板工作台中的私有模板。 |

`/workshop/projects` 是 deprecated 兼容路由，会重定向到
`/workshop/canvases`，不是产品页面，也不是 Canvas 的第二个名称。
`/workshop/audio` 已退役；音频创作仍可通过 Canvas 音频节点完成。

## 领域边界

任务 owner union 保持有意的最小形状：

| Owner | 身份 | 使用场景 |
| --- | --- | --- |
| `CanvasNode` | `{ canvasId, nodeId }` | 从 Canvas 节点发起的任务。 |
| `StandaloneWorkbench` | `{ workbenchKind }` | Image、Video 或其他独立工作台任务。 |
| `WorkflowStep` | 保持现有 workflow/run/step 身份 | Workflow 执行。 |

只有从 Canvas 节点发起的任务才拥有 Canvas owner。独立任务不会获得隐藏、临时、
默认或自动选择的 Canvas。旧 standalone 行可以保留 legacy `project_id` 作为 inert
provenance，但它不参与 owner equality、历史分页、退役、素材 origin 匹配或 Canvas
删除。新 standalone 任务不写入 legacy 项目绑定，新 standalone 素材 origin 只携带
`workbench_kind`。

删除 Canvas 只受该 Canvas 的 live `CanvasNode` 任务限制。live standalone 任务不阻止
任何 Canvas 删除。

## 画布模型

每个 Canvas 持久化一份带版本的 `nomifun.creative-studio/v1` 文档。图中恰好有八类
canonical 节点：

| 节点 | 当前职责 |
| --- | --- |
| `text` | 纯文本或 Markdown 内容。 |
| `image` | 真实图片素材、空图片承接节点及其持久 T2I/I2I Composer 草稿。 |
| `video` | 真实视频素材或带持久 Composer 草稿的空 T2V/I2V 承接节点。 |
| `audio` | 真实音频素材或带持久 Composer 草稿的空 TTS 承接节点。 |
| `panorama` | 真实等距柱状全景素材及其查看状态。 |
| `config` | exact 生成操作、参数、任务状态、输入与结果的可审计 owner。 |
| `director` | 从 Canvas 指向其 Director 场景、机位和时间轴状态的引用。 |
| `group` | 对已有选区执行分组后产生的容器；它不是生成器。 |

Generator、Loop、Compare 与 Output 不是 canonical 节点类型。生成由媒体节点与
`config` 共同表达；分组是明确的选区动作。

Canvas 支持选择、移动、缩放节点、连线、分组、复制/粘贴、撤销/重做、画布缩放、
重置/适配视图、小地图导航与重载。窄屏布局已经适配，但这不等于已经实现完整的
移动端触控与手势等价能力。

画布编辑使用短延迟 debounce 的 compare-and-swap（CAS）保存，每次写入都带上最后
一版权威 revision。发生冲突后自动保存会停止，不会强写，也不会覆盖新版本后静默
重试。请通过界面载入权威远端版本，再重新应用想保留的改动。离开创意工坊页面前会
flush 待处理的 Canvas 或 Director 写入；结果不安全时会阻止离开。

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
`project/projectId` 名称只表示历史 wire 兼容，不代表当前创意工坊仍有 Project
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

一次模型选择是 exact `{ providerId, model }`。创意工坊按所需任务查询 NomiFun
托管模型目录，并排除已禁用的 Provider、已禁用模型，以及只声明了相邻任务的模型。
系统不会通过模型名称猜能力，也不会静默替换成另一个任务。

| 操作 | 要求的 NomiFun task | 创意工坊 capability |
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

## 独立 Image/Video Workbench

Image 和 Video 是独立工作台。它们的路由没有 Canvas query、选择器、父级加载门槛或
scope bar；即使 Canvas 列表为空，也必须完整可用。它们不会创建或选择隐藏 Canvas。

独立任务历史只按 `workbench_kind` 分桶：

- `GET /api/creative-studio/tasks?workbench_kind=image|video`
- `POST /api/creative-studio/tasks/retire`，body 为
  `{ workbench_kind, task_ids }`

历史分页、活跃任务恢复、重试、退役和素材 origin 匹配都会忽略旧 standalone 的
`project_id` provenance。历史模型按任务 identity 合并旧 provenance 分桶，并保持严格
keyset 分页。无法证明有序输入的旧行仍可见，但不能执行精确重试。

### 工作台 session 草稿连续性

Image 和 Video 按 `workbenchKind` 在 `sessionStorage` 中各保存一份版本化草稿。
storage key 不包含 `projectId` 或 `canvasId`。草稿只保存：

- prompt；
- exact `{ providerId, model }` 身份；
- 受控生成参数；
- 有序 reference asset IDs；
- 工作台布局。

busy 状态、错误、打开的 modal、当前选择、任务状态和完整素材对象都会明确排除。
损坏、超长、未知版本、跨工作台或 storage 不可用时，值会 fail-closed 丢弃，不阻止
页面加载。

进入路由时，reference ID 会通过 canonical asset `get` API 逐个 hydrate。缺失、不可读、
类型不匹配、重复、超量或错误类型的引用会被移除，不会恢复成过期浏览器对象。初始
hydrate 完成前不会允许生成。模型目录准备好后，只有同一个 exact Provider/model
仍支持所需 task 时才恢复；不会用另一个 Provider 的同名模型替换。

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

## Canvas 归档

规范 Canvas 导出是版本 2 的 `*.nomifun-canvas.zip` 归档。manifest 使用 Canvas
身份，包含已校验的 Canvas 文档与完整引用素材闭包，其中包括 Director sidecar 及
其引用素材。导入会校验归档，并重映射 Canvas、节点、连接、素材、operation、session
和 Director 引用，避免导入副本与源对象共用身份。

reader 必须继续支持已发布的版本 1 `.nomifun-canvas.zip` 格式。v1 manifest 可能包含
历史 `project/projectId` 字段；这些只是兼容 wire 数据，不会把 Project 重新引入产品。
Conversation 消息与活跃 pending turn 位于归档之外，导入不会克隆 Conversation。

归档不包含 Provider 凭据，也不会安装缺失的 Provider 或模型。全局模板与 Canvas
没有引用的素材不会被隐式塞进 Canvas 归档。

## 最小模板 AI

`/workshop/templates` 的 **AI 创建**首发范围刻意保持简单：

1. 输入简单需求并选择一个 exact、已启用的 `chat` 模型。
2. NomiFun 执行一次不带工具的 completion：墙钟上限 120 秒、输出上限 4,096 token、
   本地响应上限 262,156 bytes。
3. 客户端只接受一个位于最终位置、结构严格的
   `nomifun.creative-studio.workflow-draft/v1` JSON artifact。草稿模式仅有
   `single-image` 与 `multi-image-series`。
4. 先审阅预览。**应用**只会把一个私有的内存草稿打开到现有模板编辑器。
5. 需要时继续编辑，然后点击**保存**。只有这次显式 Save 才创建模板；Apply
   不会持久化，也不会运行。

这次 one-shot 不创建 Conversation、附件、公开模板、Skill/MCP 工具会话、已保存模板
或模板运行记录；也不会自动重试、模型故障切换、保存或执行。模型不能决定 ID、
revision、时间戳、可见性、标签、媒体生成模型或素材。公开模板发布/发现与复杂
模板会话不在首发范围内。首发 UI 是 private-only：新建、编辑、复制与 AI Apply
都会把底层 Workflow 定义规范化为 `private`，界面不提供公开可见性开关。

## Director v1 子集

Director 是 Canvas 内的 Three.js 场景编辑器，不是完整 DCC 或视频编辑器。当前产品
支持场景和机位 transform、机位画幅与三分线、真实 2:1 全景环境、时间轴时长/播放/循环、
机位位置轨道与关键帧、当前机位 PNG/JPEG 截图、把截图上传为真实 NomiFun 图片素材，
以及幂等发送截图回 Canvas。Director 状态是一份由 Canvas 文档引用、通过 Canvas CAS
推进的版本化文本 sidecar。

当前素材后端不接收 GLB/glTF 模型导入，因此角色与模型库动作不会创建假占位模型。
四方位/十二方位批量截图、时间轴/视频导出，以及完整全景/视频生产仍不可用。

## 当前限制

- 视频目前只支持 T2V 与单图 I2V；V2V、首尾帧、多图引用、视频/音频混合参考与未
  类型化的隐藏 Provider 参数都会被拒绝。
- Canvas 音频生成目前只支持零输入 TTS，并要求一个 MP3 或 WAV 结果。参考音频、声音
  克隆、音频到音频、speed/instructions、AAC 与 PCM 没有在本合同中开放。
- Provider 协议存在差异。只有 exact 类型化协议 profile 支持时才显示对应控制项；
  未知协议使用更小的安全子集。
- 默认标题栏和创意工坊侧栏会跟随应用语言，但首发 Canvas 与编辑器的大部分正文仍以
  简体中文为主。
- 配置了模型不等于远端 Provider 可达，也不等于已经执行付费请求。生成前请留意
  Provider 的计费和数据政策。
- 浏览器窄屏布局验证不能证明完整触控设备支持。

## 如何理解验证结论

创意工坊按层报告验证结果，避免把一个层级的成功误当成另一个层级：

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

- 产品路由：[`app/routes.ts`](../../ui/src/renderer/pages/creativeStudio/app/routes.ts)
- Canvas 文档：[`creative_studio.rs`](../../crates/backend/nomifun-workshop/src/creative_studio.rs)
- Canvas、素材与模板路由：[`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs)
- 生成任务路由：[`nomifun-creation/src/routes.rs`](../../crates/backend/nomifun-creation/src/routes.rs)
- 模型选择：[`models/catalog.ts`](../../ui/src/renderer/pages/creativeStudio/models/catalog.ts)
