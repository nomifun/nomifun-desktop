# 应用托管架构

侧栏「应用」管理普通本机程序。唯一托管 owner 是
[`nomifun-app-hosting`](../../crates/backend/nomifun-app-hosting/src/lib.rs)，
HTTP 入口与 Agent 管理工具直接调用同一个 owner。应用可以有网页，也可以只是后台服务；
无需 NomiFun SDK、Manifest、activate 回调或前端 Bridge。

## 应用首页与工作区

首页提供创建、导入、搜索、标签分类、卡片网格与分页，使用同一 owner 登记的名称、说明和标签。
创建、编辑与导入进入左右分栏的应用工作区：左侧复用完整普通会话，右侧提供详情、命令设置、
实际操作、输出和按需程序预览。右侧手工导入与配置不要求模型或 Agent。

工作区固定官方「内置应用」Agent（`app.developer`），模型和推理强度仍使用普通会话合同。
空应用直接创建当前 `Source` 记录，应用自有目录作为 canonical Session 的明确工作区；
不先创建外部临时项目，不在生成后再导入同一目录。`development_session_id` 仅是可验证的导航引用，
不授予工具权限、独占所有权或整会话删除权，也不成为第二份绑定账本。
重开时校验 canonical owner、普通 Conversation 用途、固定模板及确切工作区；合法引用复用原会话，
缺失会话通过原 owner 创建普通 Session，不复制 transcript 或恢复退役数据。

内置应用的不可变配置包含简短开发职责：创建或修改同时维护当前应用名称、说明、命令和真实预览。
详细连接指南仍按需调用，不向所有 Agent 常驻注入。现存空职责的当前代开发会话只在明确的
`development-session` 提交准备命令中，通过原 canonical 闲置 binding transition 更新；
不在 GET、轮询或重开时替换 Snapshot，保留模型、工具权限、资源、历史和旧不可变配置。

Agent 保存配置与预览时，右栏重新读取同一 owner；只有真实 `running`、预览模式和可嵌入地址
才显示应用网页。新受管 PID 替换旧网页，相同 PID/URL 的普通刷新和缩放保留页面状态；
用户可显式刷新网页。表单按字段保留未保存内容，其他字段继续同步实际配置，保存发送稀疏更新。
停止时隐藏网页并退出放大，但保留已经打开的预览意图，生产模式重建后恢复内嵌网页。
用户明确收起后保持关闭。暂时读取失败保留当前工作区、会话、表单和预览，在详情中提示并可刷新重试；
缓存展示不成为 Runtime 上下文、准入或重放权威。
完成后的配置响应可带精确 Session/Turn 的只读 `completion_observation`，不写入登记或查询 DTO。
已有加载网页在同 PID/URL 下收到成功文件变化观察时刷新一次；新 PID 已加载、只读回合、重复通知
和用户已收起的预览不再重复加载。只有当前已挂载页面观察到的同回合开始基线用于这一判断。

网页预览是应用自己 HTTP(S) 页面的一般 iframe，按需加载并可放大，保留浏览器打开入口。
renderer 同源地址不能嵌入；iframe 使用 sandbox 和 `no-referrer`，不注入 LocalTrust、SDK 或平台 Bridge。
远程 WebUI 不打开 Host loopback 地址，不提供跨机代理。这个预览是现役产品容器，
与已退役的 SDK Surface、descriptor 和事件通道无关。

## 文件与登记

文件夹或 ZIP 导入复制到 `{data_dir}/app-hosting/apps/{app_id}`，原来源保持不变。
设置和必要的进程观察信息按应用原子保存于同级 `registry/{app_id}.json`。
登记包含名称、说明、标签、来源类型、普通运行设置和非独占开发会话引用。
首次外部导入可以新建当前应用；工作区导入通过 `import_into` 复制到同一个应用目录并重新识别命令，
保留 ID、开发会话及来源中没有的已有业务文件。普通覆盖保持当前设置，修改后重新构建。
登记只描述当前文件和命令，没有应用版本、部署、发布、业务数据库管理或第二个能力登记库。

现有 PUT 和 `update_app` 支持非持久的 `detect_settings: true`，在同一应用 gate 内从当前文件
补充可确定的普通配置，不执行程序。省略字段保留当前值，明确 null 清空对应可选设置；
已有前台命令及明确跳过的准备步骤不被检测覆盖。用户开发回合结束的界面补配置请求核验
确切已接受输入、canonical 操作完成、最新回合和闲置/未知效果围栏；不能把通知中的消息 UUID
直接当成操作 ID，不从模型回答或投影借用完成权。查询和重开无检测副作用。
完成通知可能先于准入读锁释放，只有确切完成配置通道允许最多等待两秒取得同一写围栏，
取得后仍完整核验上述 canonical 条件。手工改文件立即拒绝并发，不重放模型或工具效果。
只有确切当前工作区的成功文件效果才触发回合完成后的补配置，只读完成保持当前登记；
历史工作区资源不能代替当前 canonical binding。
ZIP 拒绝逃逸路径和链接；文件夹复制拒绝外部链接与目录链接，内部文件链接物化为文件。
项目子目录和静态产物目录必须位于托管副本内。
平台复制、写入、启动和删除前检查从托管 root 到目标的所有自有路径组件，包含 root 本身；
Windows junction、mount point 等 reparse point 不会成为新的所有权锚。路径被替换时拒绝操作，
保留登记和外部数据，正常停止仍使用既有精确进程权。

## 生命周期

每个应用独立串行化准备、设置、启停和删除；全局目录锁只用来查找应用。
安装和构建为一次性命令，失败即停止后续步骤。首次预览执行安装和预览；没有预览命令时执行
安装、构建和启动。首次开始使用执行安装、构建和启动；日常启动只执行启动命令。
重新构建执行停止、安装、构建和启动。空的非必需命令跳过。

预览与使用运行同一份当前文件和业务数据，切换先停止旧树。前台命令由共享
[`nomi-process-runtime`](../../crates/shared/nomi-process-runtime/src/lib.rs)
持有精确进程树清理权；MCP 客户端不启动第二个程序。不采用守护进程叠加、自动重启或故障修复。
静态产物预设运行普通 Node HTTP 服务；SSR 和其他产物按实际启动命令运行。

主要状态是运行中、已停止和失败，准备阶段只是当前操作。前台进程意外退出，即使退出码为 0，
也标为失败。正常停止和切换不标失败。

在发布启动结果前，已配置的本机 HTTP(S) 地址最多等两秒 TCP 监听；不发送 HTTP、TLS 或凭据，
不探测远程域名。等待可由原取消/Host 退出打断，仍检查进程真实退出，超时不伪造业务失败或成功。
端口监听只防止通常的首次加载竞争，网页内容和业务功能仍须实际验证。
输出有 32 KiB 上限，Host 连接凭据和已知环境 secret
在公开或持久化前脱敏，两个输出流分别处理。正常 Host 退出停止各应用；下一次启动依据
PID、OS 创建键和可读的可执行路径核对旧进程，无法确认时失败，绝不只根据 PID 操作其他程序。

## 环境与无感连接

优先查找系统 Node；自定义路径通过真实执行检测，不设置版本白名单。
Node、npm、Bun 分别检测，其他工具由实际命令报告错误。缺失时提供官方 Node 下载入口，
不自动替换现有环境。

Desktop 与 Headless 每次 Host 启动创建本机 LocalTrust。实际服务监听成功后更新本机地址，
仅在预览和启动进程环境注入 `NOMIFUN_HOST_URL`、`NOMIFUN_LOCAL_TRUST`，以及可选
`NOMIFUN_AGENT_BINDING_ID`。安装与构建主动移除这些变量。连接凭据留在后端内存，
不写入设置、网页或日志。网页通过自己的普通后端调用已有 REST 接口。
共享连接按安装 owner 信任；外部登录、安装级 token、管理员初始化和 CORS 仍走原边界。

## Agent 与 MCP

「NomiFun 应用开发」是 Agent 工作台的 `app.hosting` 系统能力候选。
只有应用工作区的官方 `app.developer` 模板默认选入全部十个动作；其他官方模板默认不选择。
用户显式保存的模块和动作进入同一冻结 Revision/Snapshot；既有个人选择和旧会话不被默认值更新扩权。
候选提供 `guide/current/import/list/update/preview/rebuild/start/status/stop` 十个托管动作。
每轮准备只将最终已授权且启用的托管动作 schema 提前展示，保持原有冻结身份与执行检查，
不依赖可能截断的搜索结果找齐当前应用管理工具。未选择能力的 Agent 不增加这组上下文。
`current_app` 仅从确切 canonical 工作区寻找当前应用；`update_app` 原子更新同一应用的元数据与命令，
不允许作者连接绑定或会话所有权。应用工作区导入明确指定当前 app ID，避免重复登记。
只读 `read_app_guide` 的详细后台连接说明仅在工具被明确调用时加载，
不作为所有 Agent 的系统上下文。应用代码仍使用原有已授权的工作区、进程等工具编写，
不建立新的创作流水线、App checkpoint 或全局 Context/Hook。

导入与启停使用已有 `HostedEffectReceipts` 和 canonical `agent_effects`，准入核对确切的
Session、Turn、operation、Snapshot 和已提交工具因果。Host 保留操作与结算一起运行；
成功、已知失败（可能有部分变更）分别记录，worker 结果不明保留未结算围栏，不能自动重放。
普通无 MCP 应用不发布工具目录；相同完整目录的重复探测不改变代际，实际目录变化仍保留冻结检查。

可选 `mcp_url` 的来源归当前应用设置，现役 MCP 客户端只读发现和调用。
应用工具必须由 Agent/会话显式选择，默认不全局注入；连接失败局部提示，不阻塞普通会话。
目录容量、工具合同、冻结资源和 canonical binding transition 复用现役链。
调用 Agent 复用 REST `open / turn / observe / cancel` 和现有 Agent/RemoteBinding owner。
设置准备与启停通过同一个应用 gate 协调，不新增进程或绑定账本。
更换 Agent 要求已停止且不持有未确认进程，复用现役 RemoteBinding 同 ID CAS。
设置保存失败时在同一个 gate 内按原 owner 的精确收据补偿；清理未确认则保留失败登记和连接引用。
Host 退出先取消准备并停止托管树，再等 HTTP 请求 drain，避免安装请求挡住自身清理。

## 项目目录与多个普通会话

一个应用对应一个独立项目，项目根目录固定为该应用 ID 的自有托管目录；不同应用不共用项目根目录。
即使从相同外部来源导入，每个应用也得到独立副本。命令的相对子目录不改变项目身份。
该目录直接复用普通会话的 canonical workspace 身份，不新增项目数据库或会话归属账本。
`GET /api/apps/{id}/development-sessions` 按确切 owner、普通 Session purpose 和**当前** typed workspace
分页查询项目历史，包含归档状态；只接受现役显式工作区资源身份，默认工作区的安装级占位参数
不代表项目目录。不以历史资源索引、标题、最新导航引用判断成员。
`development_session_id` 只保存最近打开的会话，选定编辑器用 URL 中的 Session 身份保持独立，
轮询和其他窗口改变最近导航不会切换正在查看的会话。相同目录、其他 Agent 的会话可阅读，
通过普通会话入口打开；内置应用编辑器只接入 Host 核验的 `app.developer` binding。

新会话沿用普通 Session opening 和明确模型选择，使用稳定创建 key 重试；结果未知时重试同一 key。
精确 canonical opening 查询恢复已经创建的 Session，不因应用改名再创建一份；请求参数冲突拒绝。
显式完整删除 Session 按现役隐私合同清除 opening 与所有内容，不保留另一份创建 key 账本。
源码、命令、预览和设置属于应用，聊天内容属于各自的普通 Session；切换仅重新挂载会话区域。
完成同步验证确切的项目 Session 和已结束 Turn，不要求它等于最近导航引用。

新建显式目录的普通 Session 优先复用当前同 owner、同 canonical root 的既有 scoped 项目记忆
resource 身份，首次建立时才使用 owner 与 canonical root 的摘要。同项目共享原有 `project.memory`
状态，不同项目隔离。Agent 切换和正式存储位置迁移保留该逻辑身份，未改动旧会话的冻结
默认记忆 binding。现役项目记忆使用 Host 内存 persistence，本轮不改变其重启合同；
源码与 canonical Session 内容持久，记忆共享限定在当前 Host 生命周期。
Runtime 只读取各自合法 canonical 内容和已授权记忆，不能拼接其他会话投影。
普通会话列表继续按现役工作区分组，应用入口仅登记现有显示路径偏好。

同一现役数据集的位置迁移离线校验应用登记 UUID、真实 installation owner 与精确自有根，
只重写副本登记中的目录，并通过原 canonical 位置转换更新项目资源参数；源码、设置、逻辑资源
身份、权限和原历史保持各自合同。未确认的应用进程阻止迁移，不在离线检查中尝试结束进程。
详见 [数据集位置迁移](agent-session.zh.md#同一现役数据集的位置迁移)。

当前 portable backup/restore 和 factory reset 尚未完整纳入 `app-hosting` 根目录，不能据此保证应用
登记、项目文件与会话引用的完整恢复或清除。本次位置迁移的验证不覆盖这些独立流程。

## 删除与历史保护

手工导入、覆盖和删除先持有目录与全部当前项目 Session 的现役准入围栏，并在应用 gate 内复核导航引用。
目录围栏只使用现有内存锁注册表，不持久化；普通创建、fork、准入、warmup 和 binding 切换先获取
相同目录的读围栏，防止会话查询与文件变更之间出现新的活动 Session。
进入 `deleting` 的 Session 不出现在普通历史中，但仍按确切 owner 和当前显式项目目录保护文件，
直到现役删除清理完成；重启后的空 Runtime 缓存不能替代这个检查。
活动 Turn、pending/unknown Effect 或尚未结束的 native producer 拒绝这些文件操作；
Agent 导入在自身已准入 Turn 内取得相同目录和项目 Session 围栏，核验确切 Snapshot、工具因果和
本次 pending Effect 收据；仅允许该调用自身的待结算效果，其他活动、pending/unknown 效果仍阻断。
同时复用现役工作区原生写锁，避免本回合其他文件工具与导入竞争；拒绝按原 Effect 合同结算为
执行前拒绝。它调用同一应用 owner，不重入手工 HTTP 围栏。
只读详情、元数据/设置保存和停止应用仍使用原 owner。未确认结果不能自动重放。

删除先结束进程树，由原 owner 清理应用运行时的 Agent 连接引用，再删除自有目录和登记。
部分失败保留当前登记供重试。原导入目录、外部数据、普通开发会话及其他应用保留。
`development_session_id` 不构成整会话删除授权；Session 的合法事件和 payload 不因应用删除而改变。
在同一准入围栏内，删除按 canonical opening 来源核验经当前应用入口创建的多个普通 Session，
等待其正常 Runtime teardown（无 kill reason），再释放确切工作区的闲置 File/Artifact/VCS owner
缓存及其原生目录句柄。合法旧导航引用保留现役释放语义；共享目录的无关 fork 不被自动终止。
其他 Session 的活动资源、
仍被调用持有的 owner 或未确认的 Git 效果保持围栏，不能通过驱逐缓存绕过结算；不终止无关进程。
这个资源释放不写入或删除 Session、Turn、Event、Payload、Effect 与资源定义。
删除后原工作区路径不可执行，不能靠保留历史补造目录或恢复权限。共享本机凭据不会随单个应用删除而更换。

没有生产 writer 的旧 `exclusive_session_ids` 字段及整会话删除分支已退役。
仅当前应用 registry 中的精确空数组 `[]` 可安全读入，并在正常持久化时去掉；公开设置不接受该字段。
非空或无效值拒绝启动读取，保留文件、登记与全部 canonical 历史，不查询或转换其中的 Session 引用。

旧托管领域、SDK、Surface、Hook、专属创作和交付流程已退出正式入口。
已发布迁移保持校验不变，通过正向迁移移除旧活动表；切换保护旧作品和 SQLite 已提交 WAL 数据。
不可变 Agent Revision/Snapshot 与合法 canonical 历史保留，退役 PluginAuthoring 仅作 typed
只读识别。详见 [Agent Session 架构](agent-session.zh.md)。

## 验证入口

真实 Node/HTTP/进程树、崩溃恢复、覆盖、删除范围和环境检测：
`cargo test -p nomifun-app-hosting --offline`。
应用元数据、无模型手工导入、普通开发 Session 和活动/未知效果文件围栏：
`cargo test -p nomifun-app --test app_workspace_api --offline`。
受影响 UI 执行 `bun run check:desktop-ui-boundary`，Session 接缝执行
`bun run check:agent-session-boundary`，旧域残留执行 `bun run check:app-hosting-boundary`。
托管管理 UI 支持 Tauri 与桌面级 WebUI，最低 880×600。

English: [app-hosting.md](app-hosting.md)。使用说明见 [应用托管指南](../guides/app-hosting.zh.md)。
