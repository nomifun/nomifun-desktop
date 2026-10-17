# 默认可信的全局操作系统能力

产品原则是“最大权限赋予，最可靠实现，不要搞骚操作”。NomiFun 不设置工作区文件上限、写沙箱或用户权限档位。普通操作使用当前登录用户；系统身份、隐私许可和交互桌面分别通过真实平台机制完成。管理员身份不授予 TCC，不把后台服务变成登录桌面。

## 文件、默认目录与项目身份

现役 canonical `workspace.files` 工具处理本机绝对路径，以及相对 Session 默认工作目录的原生路径。合法 `..`、软链接和挂载路径由文件 owner 解析，工作目录只是相对路径锚点。读取、搜索、写入、patch、删除、重命名共用现役 FileService；不恢复 gateway 或增加另一套无限制工具。

原子写入和 patch 继续固定真实父目录句柄，校验源版本、发布前置条件和并发变化。读取、写入及 patch 跟随合法目标链接；删除和重命名操作链接本身。重命名不覆盖既存目标，跨卷失败保留真实结果。操作系统 ACL、属主和文件系统错误仍然生效；`SYSTEM_ACCESS_DENIED` 保留原始 I/O 原因，不能把所有拒绝自动归类为管理员不足。

项目 watcher、VCS、产物和应用归属继续指向确切项目。工作区外写入不发布伪造的工作区相对文件事件，也不因为物理路径相邻而成为项目产物。Session/Turn/Event/Payload/Effect 仍是唯一 Agent 事实链。

`read_file` 的 text、image 和 instruction_scope 共用 `execution_identity`，缺省是当前用户。管理员读取仍是同一个只读 Action，不新增工具或文件写入权限：原生 worker 使用轻量 `nomi-native-files` 获取受限体积的真实源或目录 scope，App 复用 FileService 的 UTF-8、分页、完整 SHA-256、版本 pin、项目路径投影及图片 decoder。普通与管理员读取共用同一原生读取和 scope 扫描核心；真实缺失、访问拒绝、过大源和失效版本分别处理，拒绝不能成为 absence。

## 进程与系统执行

Agent 的 `exec_command`、`start_process` 和后续输入、轮询、调整 PTY、取消复用 `ManagedEngineProcessOwner`。`nomi-process-runtime::ProcessSupervisor` 负责 Pipe/PTY、输出、超时、租约、watchdog、后代清理及真实终态。旧 `CapabilityPolicy`、`SandboxPolicy`、`cwd_roots`、MacSeatbelt、sandbox-exec profile 和无用途的 `bash_sandbox` 配置已经退役。不能把根目录加入允许列表来恢复旧策略。

每次操作的 `execution_identity` 缺省为 `current_user`；明确需要系统身份时选择 `administrator`。这是执行该操作的身份参数，用户不切换权限模式。文件或安装失败后，Agent 按原任务意图选择现役系统执行路径；先核对已发生效果，再继续受阻步骤，不自动重放未知部分写入或安装。

macOS 14+ 的管理员路径是 `nomi-system-execution`，通过 `SMAppService.daemon` 注册 App 内的 LaunchDaemon，用双向代码签名要求的 NSXPCConnection 传输操作。helper 内直接链接原 Rust 监督器，保持原有进程管理；主 GUI 与普通工具不以 root 运行。系统首次批准在“系统设置 → 通用 → 登录项与扩展”完成。任务等待可取消并重新检测批准；批准只代表设施可用，实际结果还须读取终态和原生效果。

每个系统进程持有其原 XPC 连接；原连接持有确切子进程和清理证据。启动 RPC 的阻塞 worker 在等待 future 取消后仍保留连接与派发围栏；quiesce 等待此前启动派发完成，不用空句柄表证明清理。连接丢失、超时或回复无法解析保留未知，不能换一个新连接洗成“未执行”。同一系统 daemon 的 OS 批准复用，不保存管理员凭据或建立授权数据库。

管理员只读 acquisition 由同一签名 helper 的子进程模式执行，通过既有 ProcessSupervisor 管理输入、10 秒 deadline、取消、断联和真实回收；不在 daemon 内留下无法取消的文件 I/O 线程。8 MiB 源编码为 base64 后使用 16 MiB 的有界输出保留，并检查截断与完整回执。acquisition 与进程 start 一起进入 transport admission 围栏，quiesce/shutdown 等待已准入调用的真实结果。

Rust worker 经 Swift/C 入口退出时，在返回前显式写完并 flush JSON 回执；写入或 flush 失败返回真实 I/O 失败。外部运行时的 `exit` 不承担 Rust 标准输出缓冲清理。退出 0 但缺少完整回执仍是结果未知，不能当作文件不存在或读取成功。

helper 从连接的真实调用用户解析 HOME，保持用户环境；系统命令没有交互桌面资格。用户要求持久的 launchd 服务、登录项、系统计划任务是原生系统产物，其运行与卸载按任务意图处理。任务临时子进程仍须清理，不通过双重 fork 假装持久化，也不把任务结束的全树清理作用到已正式注册的独立服务。

Windows、Linux 当前共享接口对尚未实现的管理员适配返回明确 `SYSTEM_EXECUTION_NOT_AVAILABLE`。它们需要目标平台设计、实现和原生验证，不能把 macOS helper 编译成功写成三平台已完成。

## 命令、环境与依赖

`nomifun-runtime::prepare_process_launch` 和 `process_environment` 是 Agent、Terminal、MCP stdio 及组件调用的共同准备入口。每次启动重新组合现有工具、用户 PATH、持久 CLI 安装位置及已经准备的组件；不在线程运行期间反复写全局环境。原生程序使用 executable + literal argv，只有明确 shell 调用解释 shell 语法。Windows Terminal 的既有 shim 路径保留；用户 `.cmd/.bat` 在 Agent 低层的统一启动仍待 Windows 接续实现与原生参数验证。

缺少任务必要的受管运行时自动准备；状态查询只读。Terminal、MCP 与 Relay 的额外下载确认及专用状态/UI 已退役，保留真实进度、校验、重试与取消，启动请求不因 UI 进度轮询而重放。供应器保留下载完整性、并发单飞、临时解包、取消和原子发布。

Node 使用完整官方发行目录，保留 npm/npx、模块、launchers、合法链接和执行位；完整发行身份及缓存布局使旧单文件缓存失效。Bun 保留本身用途。用户工具安装根独立于受管运行时缓存和 Agent 数据，升级、清理运行时及清空 Agent 不删除已装用户工具。用户自有 `.npmrc`、prefix、PATH、registry、代理、证书和安装方式继续生效；系统安装、原生 installer 和服务使用通用进程及系统执行接口，不强行改为 npm 用户安装。

CLI 附带 Skill 必须进入现役技能库导入与发现路径，写到第三方 Agent 目录不表示 NomiFun 已就绪。当前活动 Turn 的技能快照保持冻结，后续使用通过已有刷新/选择机制。

## 默认个人 Agent 与旧会话

默认聊天 `chat.minimal` 正式具备 `workspace.files` 六个动作和 `workspace.process` 七个动作；资源由原 resolver 自动供给。`assistant.general`、伙伴等原有相应能力继续使用同一工具路径。

当前代旧官方 exact-empty 聊天会话在空闲新 Turn/模型准备边界，经 `default_local_os_evolution` 的精确合同证明与原 binding CAS 追加正式转换，仅补这两根能力及其真实资源。旧 Revision/Snapshot、历史事实和 Session 身份不回填；活动、暂停、Remote、Attempt、未知效果和 native checkpoint 围栏不放宽。用户明确裁剪、专用机器人以及委派工具裁剪保持原任务分工语义，不从显示名称推断扩权。新发布未授权的描述符可以演进元数据，已选 Action、效果、资源和实际授权不因此扩大。

指令 scope 与其 AGENTS 分页读取继承待执行调用的真实身份，缓存按 `(identity, native path)` 隔离；普通读取不能借用管理员缓存。每个真实串行效果结算后、下一调用预检前失效指令缓存，最后一次效果后的模型准备也继续刷新。指令变化时保留未执行回执，读取新上下文后重新提出调用，不重放已有结果。普通内部读取可以省略身份字段使用正式缺省；管理员读取必须被冻结 schema 明确接受。旧会话通过现役空闲 ContextOnly 合同转换取得新 schema，精确 native checkpoint 或 frozen binding 不因新参数被私下改写。

## 桌面、媒体、网络与真实边界

Computer 继续在登录桌面的 App 进程执行，每次操作按实际需要检测 Accessibility 或 Screen Recording。输入授权后必须重新观察，旧 ref、截图或未知输入不重放。Browser 保持 Windows WebView2、macOS WKWebView 的已定结构；网站媒体许可还要满足实际宿主的 AVFoundation/TCC。

权限库存包含实际 PID 和可解析的真实 App bundle 身份。AX/CG 返回 false 只证明 `not_granted`；FDA、Automation 和局域网没有适用于所有目标的全局可靠探针，保留 unknown，实际目标操作再给真实结果。Camera/Microphone 使用原生状态；使用说明与 Apple Events automation entitlement 随 App 打包。详见[系统许可](system-permissions.zh.md)。

普通监听、本机/局域网访问、原生安装、服务与系统设置复用上述通用路径。SIP、UAC、安全桌面、TCC、ACL、网络设施和实际设备存在性仍按系统真实限制表达。Linux Computer 产品入口、X11/Wayland/portal 和 Windows 高完整性桌面路径仍有工程缺口，不能归因于用户权限不足。

## 验证与平台接续

完整证据、首次失败和三平台状态以[本轮验证记录](../validation/global-os-2026-10-16.zh.md)为准。确定性 fixture 不等于模型推理能力或正式 App 原生验收，单个文件/进程回归也不等于整个系统矩阵。

可复制的新会话交接：[Windows](../handoffs/global-os-windows.zh.md)、[Linux](../handoffs/global-os-linux.zh.md)。交接完成仅表示接续材料齐全，不表示目标平台实现或验收完成。
