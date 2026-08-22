# Infinite Canvas 产品级重建与创意工坊替换计划

> 状态：授权闸门已通过；产品重建实施中。当前提交与剩余门禁见 `docs/handoffs/2026-08-21-creative-studio-rebuild-ledger.zh.md`。
> 目标仓基线：`aa307121`。
> 参考仓基线：`ef7303d`。
> 本计划不授权删除用户数据，也不承诺旧创意工坊数据迁移。

## 1. 目标与原则

本项目要把现有“创意工坊”整体替换为一个独立、完整、可长期维护的 AI 创作产品。新产品应覆盖参考项目 `infinite-canvas` 的用户侧产品能力和主要交互，而不是继续修补现有工坊。

实施遵循以下硬约束：

1. 旧工坊前端最终整体删除，不保留新旧切换、旧组件包装层或旧文档 schema 转换器。
2. 新创作产品使用独立的全屏 Focus Shell；从主侧栏进入，通过稳定可见的“返回工作台”回到 `/guid`。
3. 前端直接集成到现有 React/Vite 应用，不运行第二个 Next.js 服务，不用远程 Webview，也不以 iframe 作为主产品容器。
4. 后端继续使用 Rust，不随桌面端捆绑 Go/Gin/GORM sidecar。
5. 模型配置、凭证、能力发现和实际调用只使用 NomiFun 的统一模型体系；不移植参考项目的 Base URL/API Key 配置中心和浏览器直连逻辑。
6. 每个提交只完成一个可验证切面；提交前必须执行与改动匹配的静态检查和真实页面 UI 验收。
7. 不把 mock、静态 DOM、旧构建产物或宣传截图描述为真实功能运行成功。
8. 不自动删除旧工坊文件数据或数据库行。旧数据不进入新运行链；若以后需要清理或迁移，另行设计并取得明确授权。

## 2. 当前事实基线

### 2.1 目标项目

`nomifun-desktop` 已具备三块可以继续使用的 Rust 基础设施：

- `nomifun-workshop`：画布元数据、画布 JSON 正文、资产索引和二进制文件、缩略图、公开只读文件地址。
- `nomifun-creation`：异步生成任务、排队/运行/成功/失败/取消、并发控制、启动恢复和产物落盘。
- `nomifun-model-invoke`：按精确 `(provider, task, model)` 解析协议、鉴权和端点，覆盖 chat、图像生成/编辑、视频生成、TTS 等任务。

现有工坊前端位于 `ui/src/renderer/pages/workshop`，共 80 个文件、约 1.54 万行。它是普通 `Layout` 内容页，仍挂载主侧栏、会话快捷键和 PWA 下拉刷新，不是真正的全屏创作产品。平台 `/assets` 页面还反向依赖旧工坊组件，因此最终删除时必须一并重做资产入口。

当前开发环境可用：

- Bun `1.3.14`
- Node `24.18.0`
- Rust/Cargo `1.97.1`
- UI 依赖、`ui/dist`、Rust debug 构建产物均存在
- 审计时 `bun run typecheck` 通过

### 2.2 参考项目

参考项目用户侧主要产品面包括：

- 产品首页与创作目标入口
- 多画布项目管理、JSON/ZIP 导入导出
- 无限画布：节点、连线、平移缩放、框选、多选、小地图、背景、撤销重做、复制粘贴、分组
- 文本、图片、视频、音频参考、全景图、生成配置和导演台节点
- 左侧画布/资产/提示词面板和右侧画布助手
- 生图工作台、视频创作台、创作工作流
- 我的素材、团队素材、提示词中心、生成历史与分类
- 图片裁剪、遮罩、切分、局部编辑、多角度等工具
- 摄像机提示词、全景图和 3D 导演台/时间轴

参考项目的“AI 超分”当前仍是占位提示，不属于已完成能力，不能写入迁移成功清单。

参考项目本机尚未安装 `web/node_modules`，本机也没有 Docker。其 Next 配置还设置了 `ignoreBuildErrors: true`，因此以后即使参考项目 `next build` 成功，也必须另跑严格 TypeScript 检查才能作为基线证据。

## 3. 许可证与来源闸门（L0）

在复制任何参考源代码、品牌资源、编译产物或 3D 模型前，必须完成 L0。

已发现的风险：

1. 参考仓初始提交 `4ff9532` 使用 AGPL-3.0。
2. 提交 `58c284f` 在一次普通 Grok2API 修复中把根许可证和商业说明直接改为 MIT。
3. 重许可之前已经存在外部贡献者；仓库还明确声明合并了 `basketikun/infinite-canvas` 和 `HuFakai/infinite-canvas`。
4. 当前仓库中的 MIT 文件不能单独证明维护者拥有对所有上游代码和外部贡献的重许可权。
5. `web/public/director` 只有压缩后的编译包；同时包含约 750 KB、标注 Sketchfab Standard 来源的 GLB 模型。它们不能在没有单独授权和再分发条件证明时直接搬入 NomiFun。

L0 的通过条件至少满足一项：

- 提供能够覆盖上游代码、外部贡献和 3D 资产的书面商业/再许可授权；或
- 法务确认当前许可证链足以让 NomiFun 在其既有许可和分发方式下直接移植；或
- 不复制受疑代码和资产，改为依据公开产品行为与重新编写的规格独立实现，并由法务确认该实施方式的衍生作品风险可接受。

**当前状态：已通过。** 2026-08-20，产品负责人确认已经取得覆盖本次迁移所需代码与资产的授权，并明确要求继续实施全部后续工作。实施仍需保留适用的版权、许可证和第三方归属材料；授权确认不等于可以删除这些声明，也不改变 NomiFun 发行前的依赖与资产清单检查。

L0 未通过时允许继续的工作：

- 路由/Focus Shell 重构；
- NomiFun 自有模型、任务、资产和持久化端口设计；
- 原创 UI 基础设施与自动化验收；
- 基于产品功能清单编写验收规格。

L0 未通过时禁止的工作：

- 整目录或逐文件复制参考仓 TypeScript/Go 代码；
- 复制压缩后的 Director JavaScript；
- 复制参考仓 Logo、宣传图、第三方模型和来源不明素材；
- 在 NomiFun 的 Apache-2.0 声明下把受疑代码当作已确认的 MIT 代码发布。

## 4. 架构决策

### 4.1 Go 嵌入与 Rust 重写的结论

选择：**Rust 原生集成，拒绝 Go sidecar**。

| 维度 | Go sidecar | Rust 原生集成 |
| --- | --- | --- |
| 模型配置 | 会形成第二套渠道、密钥和协议解析 | 直接使用 `nomifun-model-invoke` |
| 数据 | 需要第二个 DB、迁移和备份边界 | 进入现有 NomiFun 数据根、备份和锁语义 |
| 生命周期 | 需要端口、进程监管、崩溃恢复 | 与 Tauri/`nomifun-app` 同进程 |
| 打包 | 增加三平台 Go 二进制和签名 | 复用当前 Rust workspace 与发布链 |
| 安全 | 额外 localhost HTTP 边界和密钥暴露面 | 凭证留在现有后端边界 |
| 可观测性 | 两套日志、健康检查和错误模型 | 使用现有 tracing、AppError 和任务状态 |
| 长期维护 | 与 NomiFun 架构持续分叉 | 一个统一领域模型和调用链 |

Go 只作为参考实现的行为证据，不进入最终产品。

### 4.2 前端壳

目标路由结构：

```text
Root providers
└─ HashRouter
   ├─ /login
   ├─ /companion
   └─ ProtectedAppRuntime
      ├─ WorkbenchLayout
      │  └─ 现有普通产品页面
      └─ CreativeStudioFocusShell
         ├─ /workshop
         ├─ /workshop/canvas
         ├─ /workshop/canvas/:id
         ├─ /workshop/image
         ├─ /workshop/video
         ├─ /workshop/workflows
         ├─ /workshop/prompts
         └─ /workshop/assets
```

`ProtectedAppRuntime` 只持有认证、托盘、桌宠和真正应用级副作用。`WorkbenchLayout` 继续持有普通标题栏、主侧栏、会话快捷键和 PWA 下拉刷新。`CreativeStudioFocusShell` 不挂载这些会与画布冲突的能力，只保留：

- Tauri 窗口拖动区域和原生窗口控制；
- NomiFun 主题、语言、反馈和认证上下文；
- 固定可见的“返回工作台”，目标为 `/guid`；
- 创作产品自己的导航、快捷键和弹层根节点。

自定义主题 CSS 的注入目前由 `Layout` 独占，必须先提升到根级 `AppThemeRuntime`，否则 Focus Shell 会在进入时丢失用户主题。

### 4.3 前端模块边界

新代码使用独立领域目录，例如：

```text
ui/src/renderer/pages/creativeStudio/
├─ app/                 # 产品路由、Focus Shell、顶栏
├─ canvas/              # 画布运行时、节点、连线、历史、快捷键
├─ workbenches/         # image/video/workflow
├─ assets/              # 我的素材/素材库
├─ prompts/             # 提示词中心
├─ assistant/           # 画布助手与操作协议
├─ director/            # 未来的原创/已授权导演台
├─ ports/               # 只描述 NomiFun 服务能力
├─ adapters/            # ports 到现有 hooks/REST 的实现
├─ stores/              # Zustand 产品状态
├─ theme/               # 局部 token 和 CSS 隔离
└─ testing/             # fixtures、stub ports、UI 状态构造器
```

新产品通过根类 `.creative-studio-root`、CSS Modules 和独立 portal 容器隔离样式。不得把参考项目的全局 reset 直接注入 NomiFun 根文档，也不得为追求“统一”把每个创作控件重画成普通 Arco 表单。

### 4.4 状态与持久化

前端状态分为五类：

1. `ProjectStore`：项目元数据、当前项目、视口和布局偏好。
2. `CanvasStore`：节点、连线、选择、分组和临时交互状态。
3. `HistoryStore`：结构化命令/快照、撤销重做和事务合并。
4. `GenerationStore`：任务状态、恢复、取消、批量结果和失败信息。
5. `LibraryStore`：素材、提示词、工作流、筛选和分页。

Rust 后端是项目、画布正文、资产和生成任务的权威来源。前端可使用 IndexedDB 保存尚未确认的编辑草稿和大文件临时引用，但不能再次建立一套带渠道配置和账号同步语义的独立数据库。

新画布正文使用新的 schema 标识，例如 `nomifun.creative-studio/v1`。旧 `WorkshopCanvasDoc` 不转换、不回退读取。新运行时只加载新 schema。旧文件保留在原位置，直到另行授权清理。

保存协议至少包含：

- 单调 revision 或 ETag；
- 800 ms 左右的批量保存；
- 保存中/已保存/保存失败的可见状态；
- 离开页面前 flush；
- 冲突显式失败，不能静默以最后写入覆盖；
- 重新打开后恢复视口、节点、连线、面板、助手会话和未完成任务引用。

### 4.5 NomiFun 服务端口

创作产品 UI 只依赖窄接口，不直接散落 import NomiFun 内部 hooks：

```ts
interface CreativeModelCatalogPort {
  list(task: 'chat' | 'image_generation' | 'image_edit' | 'video_generation' | 'speech_synthesis'):
    Promise<CreativeModelOption[]>;
  subscribe(listener: () => void): () => void;
}

interface CreativeGenerationPort {
  submit(request: CreativeGenerationRequest): Promise<CreativeGenerationTask>;
  get(taskId: string): Promise<CreativeGenerationTask>;
  cancel(taskId: string): Promise<void>;
}

interface CreativeProjectPort {
  list(): Promise<CreativeProjectSummary[]>;
  create(input: CreateCreativeProject): Promise<CreativeProjectSummary>;
  load(projectId: string): Promise<CreativeProjectDocument>;
  save(projectId: string, revision: string, document: CreativeProjectDocument): Promise<SaveResult>;
  rename(projectId: string, title: string): Promise<void>;
  remove(projectId: string): Promise<void>;
}

interface CreativeAssetPort {
  list(query: CreativeAssetQuery): Promise<CreativeAssetPage>;
  upload(file: File, metadata: CreativeAssetMetadata, signal?: AbortSignal): Promise<CreativeAsset>;
  update(assetId: string, patch: CreativeAssetPatch): Promise<CreativeAsset>;
  remove(assetId: string): Promise<void>;
  url(assetId: string, variant?: 'original' | 'thumbnail'): string;
}
```

模型选择必须按精确任务取能力，不能用模型名猜测、跨任务合并或静默 fallback：

| 用户能力 | NomiFun 任务 |
| --- | --- |
| 文本/画布助手 | `chat` |
| 文生图/全景图 | `image_generation` |
| 参考图、局部编辑、遮罩 | `image_edit` |
| 文生视频/图生视频 | `video_generation` |
| 语音生成 | `speech_synthesis` |

每次提交生成任务必须携带明确 `provider_id + model + task/capability`。前端永不接收原始 API Key，也不自行拼接 provider URL。任务状态统一为 `queued → running → succeeded | failed | canceled`；刷新和重开页面后从后端任务恢复。

## 5. 产品范围

### 5.1 必须迁入的用户侧能力

- 创作首页和从创作目标直接建立项目
- 我的画布和项目批量管理
- 完整无限画布交互
- 文本、图片、视频、音频、全景图、生成配置、导演台节点
- 左侧画布/素材/提示词面板
- 右侧画布助手
- 生图工作台与生成历史
- 视频创作台与多模态参考
- 创作工作流与模板
- 我的素材、素材库、提示词中心
- 图片编辑工具、摄像机提示词、全景工作流
- 明暗主题、导入导出、错误/空白/加载/取消/恢复状态

### 5.2 由 NomiFun 现有产品替代的能力

- 参考项目的登录、注册、JWT、Linux.do OAuth
- 管理员用户和积分管理
- 参考项目的模型渠道/Base URL/API Key 设置
- 参考项目的 AI 日志后台
- 参考项目的 S3/WebDAV 配置页面
- Go 服务、GORM 多数据库部署和 Docker 运行方式

这些不是“缺功能”，而是由 NomiFun 的认证、模型中心、数据根、日志和部署体系接管。创作产品只提供“前往模型中心”等明确入口。

### 5.3 需要单独决策的高级能力

- 3D Director：只有 L0 覆盖编译包和 3D 模型时才能直接迁入；否则按公开行为原创实现并使用来源清晰的模型资产。
- 团队素材库：桌面单用户形态先映射为 NomiFun 资产库；若未来需要远端团队协作，再设计账户/空间边界。
- 移动触控：参考项目自己也声明尚未系统完善。首个完成版本要求窄屏不白屏、关键返回可用，但不虚构完整移动端生产承诺。

## 6. 分阶段实施与提交计划

每个阶段可拆成多个更小提交；以下顺序是硬依赖顺序。

### P0：来源、基线和验收工装

1. `docs(creative-studio): record migration architecture and gates`
2. `test(ui): add creative-studio route and screenshot harness`

产物：本计划、功能验收矩阵、参考截图清单、隔离数据目录启动脚本、截图命名规则。

门禁：L0 状态被明确记录；源项目未被复制；两个仓库工作区边界清楚。

### P1：应用壳拆分（不改变现有页面）

1. `refactor(ui): split protected runtime from workbench layout`
2. `refactor(ui): lift theme runtime above routed layouts`

门禁：`/guid`、会话、设置、登录仍按原样工作；主题进入/离开路由不丢失；类型检查通过。

### P2：全屏创作入口

1. `feat(creative-studio): add authenticated focus shell`
2. `feat(creative-studio): add sidebar entry and return to workbench`

门禁：真实点击“入口 → Focus Shell → 返回工作台”；主侧栏、会话快捷键和 PWA 下拉刷新在 Focus Shell 中未挂载；保留窗口控制；深链接和刷新可恢复。

### P3：原创视觉骨架与项目中心

1. `feat(creative-studio): add product navigation and project hub`
2. `feat(creative-studio): add new project document schema`
3. `feat(creative-studio): connect project persistence port`

门禁：空白、加载、错误、多项目四态；创建、重命名、删除、导入、导出；浅色/深色和中英文。

### P4：画布核心

1. `feat(creative-studio): add viewport and background runtime`
2. `feat(creative-studio): add nodes connections and selection`
3. `feat(creative-studio): add history clipboard and grouping`
4. `feat(creative-studio): add minimap shortcuts and context menus`

门禁：平移、缩放、框选、多选、拖拽、缩放节点、连线、高亮、复制粘贴、删除、撤销重做、小地图和三种背景均做真实交互检查。

### P5：资产和基础节点

1. `feat(creative-studio): connect creative asset port`
2. `feat(creative-studio): add text image video and audio nodes`
3. `feat(creative-studio): add side library and prompt panels`

门禁：拖放/上传、缩略图、下载、素材新增/编辑/删除、刷新恢复、大文件失败和缺失文件状态。

### P6：统一模型与生成任务

1. `feat(creative-studio): add exact-task model catalog adapter`
2. `feat(creative-studio): add generation task adapter`
3. `feat(creative-studio): add image edit video and tts runs`
4. `feat(creative-studio): restore pending runs after reopen`

门禁：无 provider、无对应任务模型、禁用模型、排队、运行、成功、失败、取消、重试和重开恢复。先用可控 stub 证明 UI 状态；真实付费 provider 只在单独授权后冒烟。

### P7：独立工作台与历史

1. `feat(creative-studio): add image workbench and history`
2. `feat(creative-studio): add video workbench and references`
3. `feat(creative-studio): add workflow templates and variables`
4. `feat(creative-studio): add asset and prompt centers`

门禁：侧边/底部布局、批量任务、分类、失败详情、按原参数重试、结果回填画布、工作流单图/系列模式。

### P8：画布助手和高级图片工具

1. `feat(creative-studio): add assistant sessions and canvas ops`
2. `feat(creative-studio): add crop mask split and local transforms`
3. `feat(creative-studio): add camera prompt controls`
4. `feat(creative-studio): add panorama workflow`

门禁：助手引用选中和上游节点；结果插入与自动连线；编辑工具不破坏原文件；摄像机配置随节点持久化；2:1 全景判定。

### P9：3D Director

1. L0 已授权路径：移植并保留完整第三方 notice；或
2. 独立实现路径：原创 Director 前端、可审计源码、来源清晰的模型资产。

门禁：场景树、角色/模型/机位、全景背景、截图回填、镜头管理、时间轴、关键帧、播放和视频导出逐项验证。没有源码和资产许可时不得用参考仓编译包冒充完成。

### P10：删除旧产品面

1. `refactor(creative-studio): replace platform asset entry`
2. `refactor(creative-studio): remove legacy workshop ui`
3. `refactor(creative-studio): remove legacy locales types and routes`

门禁：旧 80 文件、旧 Beta 入口、旧六组 i18n、旧 schema 客户端和失去使用者的 ID 全部删除；`@xyflow/react` 仍保留给执行 DAG；仓库不存在新旧切换或旧数据转换代码。

### P11：最终回归与发布准备

1. `test(creative-studio): complete interaction regression matrix`
2. `docs(creative-studio): publish verified capabilities and limits`

门禁：`bun run check`、相关 Rust crate 测试、WebUI 快环、Tauri 桌面慢环、打包静态资源检查；文档只声明实际跑通的能力。

## 7. UI 验收协议

### 7.1 每个 UI 提交

使用独立数据根，例如：

```text
NOMIFUN_DATA_DIR=%TEMP%\nomifun-creative-studio-<commit>
```

日常快环使用 `bun run dev:web`，里程碑慢环使用真实 Tauri 桌面窗口。运行前先确认 `5173` 和 `8787` 没有无关监听者，因为仓库脚本会主动释放固定端口。

每个提交至少记录：

- 对应路由能真实打开；
- Console 没有新增未处理错误；
- Network 没有意外 404/500、循环请求和失控轮询；
- 入口、返回、刷新和深链接可用；
- `1440×900` 的浅色与深色截图；
- 与本提交有关的动态交互操作记录；
- Git staged diff 只包含本阶段文件。

截图放在仓库外的临时证据目录，命名：

```text
<commit>-<route>-<state>-<theme>-<viewport>.png
```

### 7.2 里程碑尺寸

- `1440×900`：主设计基准
- `1280×720`：常见小型桌面窗口
- `1024×768`：紧凑桌面布局
- `390×844`：不白屏、不隐藏关键返回入口

### 7.3 不可用状态必须与成功状态同等验收

- 首次空白
- 后端加载
- 后端不可用
- 无模型平台
- 有平台但无对应任务模型
- 上传失败
- 任务排队/运行/取消/失败
- 资产缺失
- 保存失败/冲突
- 项目不存在

## 8. 静态和后端验证策略

按改动选择最小直接检查：

- 前端单文件/组件：目标测试 + `bun run typecheck`
- 路由、i18n、主题和依赖调整：`bun run check`
- `nomifun-workshop`：对应 crate 测试
- `nomifun-creation`：队列、恢复、取消和产物事务测试
- `nomifun-model-invoke`：任务路由、协议、鉴权、错误清洗和 secret redaction 测试
- 跨 crate 数据迁移：数据库迁移测试 + 备份/恢复覆盖检查
- 打包资源：`bun run build:ui`，里程碑再做桌面 fast build

文档提交不要求全量构建。平台特定能力无法在 Windows 验证时，必须明确列出未运行项，不能写成成功。

## 9. 完成定义

只有同时满足以下条件，才能称为“整体替换完成”：

1. 用户从 NomiFun 主侧栏进入的是新的全屏创作产品，并能稳定返回 `/guid`。
2. 参考产品的用户侧核心页面和交互在验收矩阵中都有对应实现和真实运行证据。
3. 所有模型选择和调用只走 NomiFun 精确任务能力与 Rust 任务队列。
4. 页面刷新、应用重开和任务恢复不会丢失已确认数据。
5. 旧工坊 UI、路由实现、i18n 和旧 schema 客户端已删除，没有双栈。
6. 没有未经确认的 AGPL/第三方源代码、压缩包或 3D 资产进入发行物。
7. WebUI 与真实 Tauri 桌面都完成关键路径验证。
8. 文档准确区分：已设计、已实现、静态检查通过、真实 UI 运行通过、真实模型调用通过。

## 10. 当前状态、阻塞与下一步

截至 2026-08-21，P1-P10 的主体代码已经按独立契约完成，旧 Workshop UI、路由、画布存储和旧任务归属已经退出运行链。项目中心、CAS 画布、资产、提示词目录、精确模型能力、幂等生成任务、工作流、持久化 Agent、图片工具和原创 Three.js Director 均已有真实产品接线；不能由现有后端诚实支持的能力保持显式不可用。

当前不把“主体已接线”等同于“产品全部验收完成”。P11 仍需逐页完成多尺寸、双主题、动态交互、WebUI/Tauri、任务恢复和打包回归；真实付费模型调用还需要对应凭证与单独的成本授权。准确的提交锚点、验证证据、已知限制和下一步顺序统一维护在：

- `docs/handoffs/2026-08-21-creative-studio-rebuild-ledger.zh.md`

后续若发现不在既有授权范围内的新第三方素材或依赖，仍须对该单项重新执行来源检查，不能把本次确认泛化为未知资产的授权。
