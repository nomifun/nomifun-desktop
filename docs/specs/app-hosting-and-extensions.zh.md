# NomiFun 应用：产品与实现边界

更新：2026 年 10 月 12 日。应用交互以用户最新手绘原型及项目会话要求为准：
**应用首页 → 创建 / 导入 → 普通会话开发 + 右侧应用详情与程序预览。**
本文描述当前产品与执行边界，不代替实施报告或真实验收证据。

## 1. 产品职责

应用是用户自己的普通程序，可以是网页、后台服务，也可以通过现役接口连接 NomiFun。
核心操作是创建或导入、配置、预览、使用、启停、修改当前文件和删除。
不要求 SDK、专有 Manifest 或 activate，不增加 token 配置、额外权限审批、版本、部署、发布、
维护、业务数据库管理或专属创作验收流程。

| 产品部分 | 明确行为 |
| --- | --- |
| 应用首页 | 创建、导入、搜索、标签分类、应用卡片网格与分页 |
| 创建 / 编辑 | 左侧完整普通会话；右侧当前应用详情、设置、操作、输出和预览 |
| 导入 | 相同布局；支持 Agent 协助，也可完全手工完成，不要求模型 |
| 开发 Agent | 固定官方「内置应用」`app.developer`；模型与推理强度可选 |
| 工作区 | 一个应用对应一个独立项目目录，不同应用不共用项目；直接复用普通 canonical Session 工作区 |
| 项目会话 | 同一目录支持多个独立普通会话、分页历史、归档显示和新会话入口 |
| 运行时 Agent | 应用需要请求平台 Agent 时另行选择，与开发 Agent 区分 |
| 程序预览 | 按需加载普通 HTTP(S) 页面，可放大、收起、关闭或在浏览器打开 |
| 开发过程 | 保留普通会话的消息、思考、工具调用、取消和完整历史 |
| 应用身份 | 修改、工作区导入、覆盖、构建始终操作同一当前应用，不新建版本 |
| 删除 | 删除自有文件与登记，保留原来源、外部数据和普通开发会话历史 |

这是桌面产品：Tauri 与桌面级 WebUI 最低 880×600。窄栏用容器约束适配；
不增加手机/平板布局、触摸专用交互或低于 880px 的页面断点。

## 2. 首页与工作区

首页从唯一应用 owner 读取名称、说明、标签、来源类型、运行状态和当前操作。
搜索匹配名称、说明和标签；分类来自实际标签，分页作用于筛选后的结果。
卡片进入应用工作区，并可呈现当前可用的启停操作。空状态同时提供创建与导入。

创建入口准备普通空 Source 应用记录，直接使用它的自有目录开发。
导入入口接收文件夹、ZIP、源码或编译产物。桌面文件选择针对 Host，本机浏览器上传 ZIP 针对当前电脑；
远程 WebUI 的 Host 路径不能冒充客户端文件夹。

工作区左侧复用普通会话组件与 canonical Session，而不是隐藏中间过程的专用生成器。
输入框固定内置应用 Agent，保留模型、推理强度和既有会话扩展选择。
右侧用户可独立保存名称、说明、标签与运行设置，完成导入和运行，不必先启动 Agent。

“创建/修改应用”的普通请求包括维护右侧实际名称、说明和运行配置并预览当前成果，不以写出
文件为完成。内置应用使用其正式冻结职责引导同 owner 操作；不存在另一套创作流程。
Agent 实际启动预览后直接在右栏显示网页，进程换代加载新程序；同一网页的普通轮询和缩放
不重置应用状态。手工草稿按字段保留，未编辑字段及时跟随 Agent 的真实配置。

Agent 导入助手检查真实输入，说明源码/产物判断，填写配置并引导完成工作。
应用已有当前记录时，外部导入写入该记录，保持 ID、目录及开发会话。
Agent 通过 `current_app` 找到当前应用后直接改文件，用 `update_app` 改设置，
不能把当前托管目录再次导入自身，或为每次修改新建应用。

## 3. 普通 Session 与能力

`app.developer` 复用现有编程模板的文件、进程和项目记忆能力，并选入系统能力「NomiFun 应用开发」。
普通个人 Agent 也可在工作台明确选入相同的 `app.hosting` 模块和所需动作。
只有内置应用模板默认选入全部十个动作，其他官方模板默认不启用；已有个人配置和冻结会话不自动扩权。

| canonical 动作 | 模型调用名 | 用途 |
| --- | --- | --- |
| `app.hosting/guide` | `read_app_guide` | 按需读取开发与无感连接指南 |
| `app.hosting/current` | `current_app` | 从确切 Session 工作区定位当前应用 |
| `app.hosting/import` | `import_app` | 外部源码/产物导入；指定 app ID 时导入当前应用 |
| `app.hosting/list` | `list_apps` | 读取应用登记 |
| `app.hosting/update` | `update_app` | 原子修改同一应用元数据和普通设置 |
| `app.hosting/preview` | `preview_app` | 执行预览命令链 |
| `app.hosting/rebuild` | `rebuild_app` | 停止后重新构建并启动 |
| `app.hosting/start` | `start_app` | 首次准备 / 日常启动 |
| `app.hosting/status` | `app_status` | 读取真实状态和有限输出 |
| `app.hosting/stop` | `stop_app` | 停止应用进程树 |

详细指南只在工具被调用时读取，不占用每个 Agent 的常驻工作上下文。
模块和动作授权继续由现役 Revision/Snapshot 冻结。工具不能作者 Agent binding、开发 Session 引用或独占所有权。

AppRecord 的 `development_session_id` 仅为最近打开的非独占导航引用，不是项目成员或 binding ledger。
历史从确切 owner、普通用途和当前 canonical workspace 分页派生，包含已归档会话。
用户选择历史会话或新建独立会话；切换保留应用预览与未保存设置。重开验证普通用途、固定模板
与确切工作区，其他 Agent 的同项目会话通过普通会话打开；GET 不改 binding。
新建使用原 Session owner 与稳定创建 key，重试复用同一个 canonical opening。
同项目的新会话共享文件及当前 Host 生命周期内按 owner/目录隔离的原有项目记忆；
不将现役内存记忆视为跨重启存储，源码与 canonical 历史仍持久。旧冻结默认记忆保持不变，
不向新会话拼接全项目 transcript，不新增项目数据库、会话存储副本或 retired PluginAuthoring。

## 4. 当前文件与设置

文件和命令由唯一 `nomifun-app-hosting` owner 管理。HTTP、手工 UI 与 Agent 工具调用同一 owner。
创建直接准备 `{data_dir}/app-hosting/apps/{app_id}`；外部来源导入复制到此目录，原来源保持不变。
必要元数据、普通设置和精确进程观察按应用原子写入 `registry/{app_id}.json`。

| 设置 | 行为 |
| --- | --- |
| 名称、说明、标签 | 用于详情、搜索与分类 |
| 源码 / 产物 | 用户或 Agent 根据实际输入选择，可明确修正 |
| 项目目录 | 应用目录内的相对子目录，作为命令 cwd |
| 依赖安装命令 | 可空；源码准备时执行，产物通常跳过 |
| 预览命令 | 有网页时可配置；缺失则使用构建与启动链 |
| 构建命令 | 可空；执行真实项目构建 |
| 产物目录 | 普通静态 HTTP 预设的自有输出路径 |
| 生产启动命令 | 实际前台常驻命令，不叠加另一套守护进程 |
| 网页地址 | 应用真实 HTTP(S) 页面；无网页则为空 |
| Node 路径 | 优先系统环境，可指定已有可执行文件 |
| 环境变量 | 应用自己的配置；Host 连接字段不由用户覆盖 |
| 目标 Agent / MCP 地址 | 可选现役平台连接，不影响普通程序托管 |

识别使用 package.json、锁文件和常见入口；不能假称覆盖全部语言或框架。
现有配置更新支持显式 `detect_settings`，仅保存可识别配置，不执行程序；界面在真实开发完成
后校验确切 canonical 输入/回合并处理当前文件，GET、轮询、重开和失败通知不自动运行。
空的可选安装/打包项显示跳过，预览复用启动命令时明确展示该实际命令，不能用空输入暗示未完成。
Node、npm、Bun 分别真实检测，不设平台版本白名单，不自动替换用户环境。
静态目录通过普通 Node HTTP 预设运行；SSR、Python 或其他服务执行自己的启动命令。

工作区导入重新识别命令并更新当前记录。普通覆盖替换文件而保留当前设置。
两者均保留外部来源，以及新来源中没有的当前业务文件；平台不检查业务 Schema 或迁移数据库。
ZIP 拒绝路径逃逸和链接，文件夹导入拒绝外部链接与目录链接。
项目、输出、复制、启动及删除检查所有自有路径组件，Windows junction/reparse point 不授予外部路径所有权。

## 5. 进程操作与真实结果

首次预览执行安装、预览；没有预览命令时执行安装、构建、启动。
首次开始使用执行安装、构建、启动；日常启动只执行启动命令。
重新构建先停止原树，再安装、构建、启动。空的非必需步骤跳过，失败即停止后续步骤。

每个应用独立串行化生命周期，慢命令不占用全局目录锁。
预览与使用共用当前文件和数据，切换先结束原树，不保留两份常驻服务。
共享 `nomi-process-runtime` 持有精确进程清理权；MCP 或网页消费者不再启动第二个程序。

主要状态为运行中、已停止和失败，安装、构建和启动只描述当前操作。
前台进程意外退出（包括退出码 0）显示失败，不自动重启或修复。
运行中仅证明受管进程存在，模型回答“完成”、文件存在或 HTTP 200 都不能代替业务功能验证。
输出限制为 32 KiB，并在公开和持久化前脱敏。

正常 Host 退出停止受管树。崩溃后重新核对 PID、OS 创建键和可执行路径；
无法确认则如实显示失败，不凭 PID 结束其他程序，不自动恢复执行。
Agent 变更操作使用现役 HostedEffectReceipts 与 canonical Effect 因果；
Host 保留实际执行与结算，已知部分失败不代表未执行，未知结果保持围栏，不能自动重放。

## 6. 程序预览

用户最新原型明确要求右侧程序预览与放大，因此现役设计支持按需普通 iframe。
它承载应用自己的 HTTP(S) 网页，关闭或收起不会停止程序，网页操作继续由应用自己的后端处理。
与 renderer 同源的页面拒绝嵌入；iframe 使用 sandbox 和 no-referrer，不传平台凭据、
不恢复 SDK handshake、window.nomi、descriptor、Surface 或前端 Bridge。
页面自身禁止嵌入时可以使用浏览器打开。

远程 WebUI 可以管理 Host 应用，但不能把 Host 的 loopback URL 当作远程电脑的网页地址。
应用提供可达地址时可正常打开；跨机代理、离线网页缓存与独立浏览器配置不在本范围。

## 7. 无感连接及支持的扩展

预览/启动后端进程自动获得 `NOMIFUN_HOST_URL`、`NOMIFUN_LOCAL_TRUST`
和可选 `NOMIFUN_AGENT_BINDING_ID`。安装与构建主动移除这些字段。
凭据复用每次 Host 启动的 LocalTrust，只留在运行内存；网页、源码常量、日志和打包产物不能获得它。
用户只选业务目标 Agent，不配置 token，不增加应用审批、SDK 或每应用权限矩阵。

应用通过自己的普通后端调用已有 REST `open / turn / observe / cancel`；
Agent/RemoteBinding 仍由原 owner 管理。更换或清除运行时目标 Agent 要求应用已停止，
设置提交失败在同一应用 gate 内按精确收据补偿；不建立第二套绑定库。

| 扩展点 | 当前边界 |
| --- | --- |
| 普通网页 / 后台服务 | 普通命令和应用自己的业务逻辑 |
| Agent 使用应用工具 | 同源 MCP 工具发现、明确选择、冻结与实际调用 |
| 工具服务附带资源 | 仅复用现役同源资源读取，不增加资源专用托管来源 |
| 应用请求 NomiFun Agent | 运行时 LocalTrust + 现役 REST 四操作 |
| Provider、Skill、主题 | 原有设置和导入入口 |
| 自动 Context / before_model / before_tool | 不提供全局 Hook |
| 平台菜单、快捷命令、系统事件 | 不恢复旧 Surface 扩展协议 |
| 专属 Cron 类型、动态领域适配器、通用 Host 数据 API | 不预建新协议或空壳 |

应用 MCP 地址归当前记录，消费者不拥有第二份可编辑登记或进程。
来源必须由 Agent/会话明确选择，默认不全局注入。目录容量、Schema、资源绑定、
切换与调用继续使用现役链。失败局部提示，不自动启动应用。
普通无 MCP 应用不刷新工具目录；相同完整目录探测不变更代际，
真实工具合同改变仍接受冻结检查，不承诺旧会话自动适配任意新 Schema。

## 8. 文件修改围栏与删除

手工导入、覆盖、删除持有现役 Session 准入围栏，在应用 gate 内复核开发引用。
活动 Turn、pending/unknown Effect、未结束 native producer 拒绝文件修改。
Agent 工具在其自身已准入 Turn 内直接调用相同应用 owner，不重入手工 HTTP 围栏。
读取、元数据/设置保存与停止应用保留；停止应用可以取消准备，但不代替停止生成。
没有这些引用的普通手工应用不因缺模型或 Agent 而受阻。

删除先停止受管树，通过原 owner 清理运行时 Agent 连接和 MCP 来源，然后移除自有文件与登记。
部分失败保留记录以重试。原来源、外部数据和其他应用不删除。
普通开发 Session 和合法 canonical 历史保留；非独占开发引用不授予整会话删除权。
在同一围栏内等待精确开发 Session 的正常 Runtime teardown，再释放闲置的确切 File/Artifact/VCS
workspace owner 缓存和原生目录句柄。其他 Session 活资源、仍被调用持有的 owner 或未确认 Git 效果
阻止释放与物理删除；不清除 canonical 事实，也不关闭无关 Session 或进程。
目录删除后不能通过历史补造工作区或恢复执行，不因删单个应用更换共享 Host 凭据。

无生产 writer 的旧独占 Session 字段、清理 helper 和专属删除 fixtures 同批退役。
旧 current registry 的精确空数组只做安全字段去除，不再写回；非空或无效引用 fail closed，
不查找、转换或删除任何 canonical Session，原文件和历史全部保留。

## 9. 已替换领域与资料保护

旧插件平台、专属开发 crate、activate/bootstrap、私有端口、SDK、Surface、Hook、
Product/Artifact/Draft/Generation、发布与交付流程已经退出正式托管入口。
当前工作区复用普通 Session 和文件工具，不以新的名称恢复它们。
新 API 不保留旧托管、草稿或专属创作的兼容写入口，也不提供 SDK fallback。
Kernel 的 package/plugin 只读元数据目录仍有独立消费者，不是旧应用领域的兼容执行入口。

已发布迁移 001–003 保持原校验；旧活动表通过正向迁移退役。
一次切换保护已有作品和 SQLite 已提交 WAL 数据，留存失败中止破坏性步骤。
不会导入旧草稿或来源会话来重建创作流水线，也不重新建设业务数据库管理。

现役 Agent 配置经原 owner 移除退役模块并生成合法新 Revision；
其他配置、不可变 Revision/Snapshot 和合法 canonical 历史保留。
退役 PluginAuthoring 只保留 typed 只读识别，不恢复旧 executor/checkpoint。
未知或部分 lineage 继续 fail closed。详见 [Agent Session 架构](../architecture/agent-session.zh.md)。

## 10. 实现与验证入口

- 普通文件、命令和进程：[托管 owner](../../crates/backend/nomifun-app-hosting/src/owner.rs)。
- 应用 HTTP 与 Session 接缝：[app_hosting.rs](../../crates/backend/nomifun-app/src/router/app_hosting.rs)。
- Agent 工具：[app_hosting_tools.rs](../../crates/backend/nomifun-app/src/router/app_hosting_tools.rs)。
- 当前产品使用：[应用指南](../guides/app-hosting.zh.md)；执行边界：[应用架构](../architecture/app-hosting.zh.md)。

必要检查包括：真实源码/产物/ZIP、同应用创建与导入、实际进程启停与崩溃恢复；
无模型手工操作、固定 Agent 普通会话、模型/推理设置、完整 canonical 历史与删除保护；
真实模型从空当前目录开发、预览、使用、修改同一应用；
LocalTrust 后端真实请求、MCP 明确选择和调用；预览放大、失败状态、remote loopback 及最低桌面尺寸。
原型、模型口头完成、预写项目和模拟工具结果不能替代各自闭环的真实证据。

定向命令为 `cargo test -p nomifun-app-hosting --offline`、
`cargo test -p nomifun-app --test app_workspace_api --offline`，
以及受影响 UI/Session 测试和 `check:desktop-ui-boundary`、`check:agent-session-boundary`、
`check:uarc-boundary`、`check:app-hosting-boundary`。验收结果与平台限制由具体实施报告记录；
本设计不把尚未执行的验证宣称为完成。
