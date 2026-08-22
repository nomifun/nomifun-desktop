# 创意工坊（Creative Studio）

创意工坊是 NomiFun Desktop 中专注、本地优先的创作产品。它把持久化无限画布、
项目内生成、独立图片/视频工作台、可复用素材与提示词、最小 Workflow，以及刻意
受限的 3D 导演台放在同一个产品边界内。它直接使用 NomiFun 现有的 Provider 与
模型目录，不维护第二套模型配置系统。

> English: [creative-studio.md](creative-studio.md)

## 打开产品

从应用侧边栏打开**创意工坊**。全屏聚焦外壳会隐藏普通工作台侧栏；待画布或导演台
的保存结果处理完毕后，点击**返回工作台**会回到 `/guid`。

当前路由面如下：

| 路由 | 用途 |
| --- | --- |
| `/workshop` | 新建项目，也可附带一次简单的 exact Chat kickoff。 |
| `/workshop/projects` | 新建、重命名、打开、导入、导出和删除项目。 |
| `/workshop/canvas/:projectId` | 编辑一个项目的 canonical 无限画布。 |
| `/workshop/director/:projectId` | 编辑同一项目内受限的 3D 导演场景。 |
| `/workshop/image`、`/workshop/video` | 运行归属于项目的独立图片或视频任务。 |
| `/workshop/prompts`、`/workshop/assets`、`/workshop/workflows` | 管理提示词、可复用素材和私有 Workflow。 |

`/workshop/audio` 已退役，不是现行路由。音频创作仍可通过项目画布里的音频节点完成。

## 画布模型

每个项目持久化一份带版本的 `nomifun.creative-studio/v1` 文档。图中恰好有八类
canonical 节点：

| 节点 | 当前职责 |
| --- | --- |
| `text` | 纯文本或 Markdown 内容。 |
| `image` | 真实图片素材、空图片承接节点及其持久 T2I/I2I Composer 草稿。 |
| `video` | 真实视频素材或带持久 Composer 草稿的空 T2V/I2V 承接节点。 |
| `audio` | 真实音频素材或带持久 Composer 草稿的空 TTS 承接节点。 |
| `panorama` | 真实等距柱状全景素材及其查看状态。 |
| `config` | exact 生成操作、参数、任务状态、输入与结果的可审计 owner。 |
| `director` | 从画布指向项目导演场景/机位/时间轴状态的引用。 |
| `group` | 对已有选区执行分组后产生的容器；它不是生成器。 |

Generator、Loop、Compare 与 Output 不是 canonical 节点类型。生成由媒体节点与
`config` 共同表达；分组是明确的选区动作。

画布支持选择、移动、缩放节点、连线、分组、复制/粘贴、撤销/重做、画布缩放、
重置/适配视图、小地图导航与项目重载。窄屏布局已经适配，但这不等于已经实现完整
的移动端触控与手势等价能力。

## 精确模型与任务路由

一次模型选择是 exact `{ providerId, model }`。创意工坊按所需任务查询 NomiFun
托管模型目录，并排除已禁用的 Provider、已禁用模型，以及只声明了相邻任务的模型。
系统不会通过模型名称猜能力，也不会静默替换成另一个任务。

| 操作 | 要求的 NomiFun task | 创意工坊 capability |
| --- | --- | --- |
| 简单 kickoff 与 Canvas Assistant | `chat` | 项目内 Assistant turn；严格图提案仍需人工批准 |
| Workflow AI 草稿/规划 | `chat` | 一次不带工具的有界 completion |
| 空图片承接节点 | `image_generation` | `t2i` |
| 带真实参考的图片（包括当前蒙版编辑路径） | `image_edit` | `i2i` |
| 空视频承接节点 | `video_generation` | `t2v` |
| 带恰好一张直接真实图片参考的视频 | `video_generation` | `i2v` |
| 空音频承接节点 | `speech_synthesis` | `tts` |

持久 operation 会把 Provider、模型、task、capability、有序输入素材绑定和类型化参数
放在一起。复用同一个幂等身份重试时，不能悄悄替换这些事实。删除 Provider 或单个
模型也会经过协调门禁，不能静默留下活跃任务或其他硬绑定孤儿。

### 受治理的 Gateway 访问

进程内 Gateway 通过六个 instance-owner capability 暴露同一领域：
`nomi_creative_studio_list_projects`、`nomi_creative_studio_get_project`、
`nomi_creative_studio_list_assets`、`nomi_creative_studio_apply_ops`、
`nomi_creative_studio_generate` 与 `nomi_creative_studio_get_task`。它们读取同一份
canonical 项目与素材，使用同一项目 revision CAS，并向与 UI 相同的幂等
任务队列提交。只有策展的 `desktop` 与 `admin` Gateway profile 可见；
`work`/`lite` profile、普通会话、伙伴与非 owner 调用方无法发现或执行。

## 持久化、冲突与恢复

项目文档保存在 SQLite；素材 metadata 同样在 SQLite，二进制原件与缩略图则位于
后端数据目录的 `workshop/assets/` 树下。

画布编辑使用短延迟 debounce 的 compare-and-swap（CAS）保存，每次写入都带上最后
一版权威 revision。发生冲突后自动保存会停止，不会强写，也不会覆盖新版本后静默
重试。请通过界面载入权威远端版本，再重新应用想保留的改动。离开聚焦产品前会 flush
待处理的画布或导演台写入；结果不安全时会阻止离开。

图片、视频与音频 Composer 草稿保存在所属节点上；已提交工作还拥有一个持久
`config` owner 和一个 canonical creation task。重载后，界面只会按这个 exact owner
与权威任务状态对账。终态结算是幂等的；响应不确定时不会虚构成功，也不会丢掉审计
轨迹。权威 `404` 与暂时网络失败会被区别处理。

Canvas Agent 产生的是提案，不是后台改图。支持的提案 artifact 会 fail-closed 解析，
只有用户点击**应用到画布**才会执行项目 CAS 写入。删除和媒体生成不属于这套提案子集。

## 项目、ZIP 归档与素材

项目中心会把每个选中项目分别导出为 `*.nomifun-canvas.zip`。归档包含已验证的项目
文档与完整引用素材闭包，其中包括 Director sidecar 及其引用素材。导入时会校验归档、
创建新项目，并重映射项目、节点、连接、素材、operation、聊天/session 与 Director
引用，避免导入副本和源项目共用身份。

Conversation 消息与活跃 pending turn 位于项目归档之外。导入会清除这些外部引用，
只保留安全的项目自有状态；它不会克隆 Conversation。

归档不包含 Provider 凭据，也不会安装缺失的 Provider 或模型。全局 Workflow 与项目
没有引用的素材不会被隐式塞进项目归档。

素材库支持真实 `text`、`image`、`video`、`audio` 素材，包含搜索、类型筛选、集合、
标签、metadata 修改与复用选择器。二进制上传上限为 64 MiB。所有列表和写 API 都只
允许实例 owner。`GET /api/creative-studio/files/{assetId}` 是一个窄的只读例外：浏览器
媒体元素无法附带桌面 trust header，因此 opaque UUIDv7 作为 capability URL；它不是
列表或写入接口。

## 最小 Workflow AI

`/workshop/workflows` 的 **AI 创建**首发范围刻意保持简单：

1. 输入简单需求并选择一个 exact、已启用的 `chat` 模型。
2. NomiFun 执行一次不带工具的 completion：墙钟上限 120 秒、输出上限 4,096 token、
   本地响应上限 262,156 bytes。
3. 客户端只接受一个位于最终位置、结构严格的
   `nomifun.creative-studio.workflow-draft/v1` JSON artifact。草稿模式仅有
   `single-image` 与 `multi-image-series`。
4. 先审阅预览。**应用**只会把一个私有的内存草稿打开到现有 Workflow 编辑器。
5. 需要时继续编辑，然后点击**保存**。只有这次显式 Save 才创建 Workflow；Apply
   不会持久化，也不会运行。

这次 one-shot 不创建 Conversation、附件、公开模板、Skill/MCP 工具会话、Workflow
或 Workflow run；也不会自动重试、模型故障切换、保存或执行。模型不能决定 ID、
revision、时间戳、可见性、标签、媒体生成模型或素材。公开模板发布/发现与复杂
Workflow 会话不在首发范围内。首发 UI 是 private-only：新建、编辑、复制与 AI
Apply 都会把 Workflow 规范化为 `private`，界面不提供公开可见性开关。

## Director v1 子集

Director 是项目内的 Three.js 场景编辑器，不是完整 DCC 或视频编辑器。当前产品支持
场景和机位 transform、机位画幅与三分线、真实 2:1 全景环境、时间轴时长/播放/循环、
机位位置轨道与关键帧、当前机位 PNG/JPEG 截图、把截图上传为真实 NomiFun 图片素材，
以及幂等发送截图回画布。Director 状态是一份由项目文档引用、通过项目 CAS 推进的
版本化文本 sidecar。

当前素材后端不接收 GLB/glTF 模型导入，因此角色与模型库动作不会创建假占位模型。
四方位/十二方位批量截图、时间轴/视频导出，以及完整全景/视频生产仍不可用。

## 当前限制

- 视频目前只支持 T2V 与单图 I2V；V2V、首尾帧、多图引用、视频/音频混合参考与未
  类型化的隐藏 Provider 参数都会被拒绝。
- 画布音频生成目前只支持零输入 TTS，并要求一个 MP3 或 WAV 结果。参考音频、声音
  克隆、音频到音频、speed/instructions、AAC 与 PCM 没有在本合同中开放。
- Provider 协议存在差异。只有 exact 类型化协议 profile 支持时才显示对应控制项；
  未知协议使用更小的安全子集。
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
- Canonical 文档：[`creative_studio.rs`](../../crates/backend/nomifun-workshop/src/creative_studio.rs)
- 项目/素材/Workflow 路由：[`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs)
- 生成任务路由：[`nomifun-creation/src/routes.rs`](../../crates/backend/nomifun-creation/src/routes.rs)
- 模型选择：[`models/catalog.ts`](../../ui/src/renderer/pages/creativeStudio/models/catalog.ts)
