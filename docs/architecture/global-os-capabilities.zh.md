# 默认可信的全局操作系统能力

产品原则是“最大权限赋予，最可靠实现，不要搞骚操作”。NomiFun 不设置工作区文件上限、写沙箱或用户权限档位。普通操作使用当前登录用户；系统身份、隐私许可和交互桌面分别通过真实平台机制完成。管理员身份不授予 TCC，不把后台服务变成登录桌面。

## 文件、默认目录与项目身份

现役 canonical `workspace.files` 工具处理本机绝对路径，以及相对 Session 默认工作目录的原生路径。合法 `..`、软链接和挂载路径由文件 owner 解析，工作目录只是相对路径锚点。读取、搜索、写入、patch、删除、重命名共用现役 FileService；不恢复 gateway 或增加另一套无限制工具。

原子写入和 patch 继续固定真实父目录句柄，校验源版本、发布前置条件和并发变化。读取、写入及 patch 跟随合法目标链接；删除和重命名操作链接本身。重命名不覆盖既存目标，跨卷失败保留真实结果。操作系统 ACL、属主和文件系统错误仍然生效；`SYSTEM_ACCESS_DENIED` 保留原始 I/O 原因，不能把所有拒绝自动归类为管理员不足。

Windows 普通文件和目录 rename 使用 pinned source 的 DELETE 句柄与 pinned destination directory，调用 `NtSetInformationFile(FileRenameInformation)` 的相对单入口名称，`ReplaceIfExists=false`；不创建 guard 子项，不复制后删除，不自动创建缺失的目标父目录。原生操作检查真实 ACL 和卷边界。ADS 使用同一底层文件内的原生 stream rename 语义；替换非空 stream 等系统不支持的原子发布保留原始错误和原字节，不以截断写模拟成功。本机 DOS、UNC、Volume GUID 及可解析的文件系统别名按真实句柄身份处理；设备对象不伪装为 regular file。

Windows 删除与发布残留清理先使用同一已校验 DELETE handle 的 extended disposition；只有 native1/50/87 表示该调用不受支持时才尝试 basic disposition，ACL/共享拒绝不触发回退，不改变readonly属性或ACL。mark成功还不是清理证明：必须关闭本应用相关句柄，再相对 retained parent 检查原名称真实缺失。共享 publication identity 在成功mark后统一消费，旧Arc不能再用缓存FileID重试；外部reader或借用身份仍持有对象而无法证明缺失时保留原 unknown/未结算事实。绝对祖先路径消失不能代替原目录对象内的缺失证明，artifact借用pin也不能被强行释放以制造成功。平台实测与未覆盖范围见Windows验证记录。
Linux 原子替换在已固定的源与 staging fd 上保留 UID/GID、mode 和内核可观察的 ACL/xattrs，复核源与目标后再发布；权限或元数据复制失败在发布前返回真实错误。普通用户无权观察的属性不能由测试补造。新 regular file 在文件系统明确不支持 `renameat2(RENAME_NOREPLACE)` 时可用同目录 `linkat` 原子 no-replace 发布，随后精确处理 staging 清理；通用重命名不采用覆盖或复制后删除降级，文件系统不支持时保留真实错误和源文件。

项目 watcher、VCS、产物和应用归属继续指向确切项目。工作区外写入不发布伪造的工作区相对文件事件，也不因为物理路径相邻而成为项目产物。Session/Turn/Event/Payload/Effect 仍是唯一 Agent 事实链。

已冻结的默认目录后来真正缺失时，Session 准入和无知识挂载的现役 workspace lease 保留原绝对身份锚点，不创建目录或重写 binding、资源、Snapshot、checkpoint。shared native scope 必须证明真正 missing 且仍匹配原冻结路径；ACL、非目录、悬空链接和重定向不能作为缺失吞掉。原签名 registry、跨进程 OS 锁和 RAII lease 继续保护该命名空间；实际知识挂载仍要求物理目录，unbound lease 不承载挂载元数据。合法绝对文件和进程 cwd 由各自原生 owner 执行；依赖已经消失目录的相对路径或 cwd 仍返回真实错误。

canonical write/patch/delete/rename 的宿主活动写锁保留至 blocking worker 真正结束，跨默认目录的活动修改不交错。原 Store 的 owner + 真实 project root 唯一索引继续保护同项目跨 Session、跨重启的 pending/unknown；不同项目指向同一外部 native 目标时，尚无持久目标重叠围栏。现在磁盘上的链接不能证明过去目标身份；原未知操作禁止自动重放。此缺口不能由临时锁、盘符列表或新增授权/重试账本宣称解决。

`read_file` 的 text、image 和 instruction_scope 共用 `execution_identity`，缺省是当前用户。管理员读取仍是同一个只读 Action，不新增工具或文件写入权限：原生 worker 使用轻量 `nomi-native-files` 获取受限体积的真实源或目录 scope，App 复用 FileService 的 UTF-8、分页、完整 SHA-256、版本 pin、项目路径投影及图片 decoder。普通与管理员读取共用同一原生读取和 scope 扫描核心；真实缺失、访问拒绝、过大源和失效版本分别处理，拒绝不能成为 absence。

`instruction_scope` 的可选 `path_semantics` 区分内容与目录条目：默认 `follow` 为读取、写入、cwd 等发现实际目标指令；`entry` 为 delete/rename 的源和目标解析父目录并保留末级名称，链接本身只适用父目录指令。递归 entry scope 不遍历子链接目标，也不把不在删除子树内的目标计作未完成扫描；真实子目录中的命名 AGENTS 链接仍让该目录进入指令集合，随后用现役 Source follow 读取规则或保留真实 ACL、悬空链接及类型错误。请求、原生回执和 owner 投影必须匹配同一语义，缓存按身份、路径和适用语义区分；旧冻结 schema 不接受 entry 时在派发前拒绝，经原空闲合同演进取得新 schema。不能把链接删除预检重定向为目标目录删除。

已加载过的历史指令观察与当前操作的必要预检分别处理。effect 后的历史刷新只以成功重新读取的 scope 构建上下文；卷、共享、ACL、别名或返回协议变化导致旧观察不能确认时，撤下旧层及可命中的目录/递归缓存，通过原 `InstructionsUpdated` 提供带身份、路径和真实错误的有界 unavailable 数据。它不代表 absence 或许可，也不结算任何 Effect。下一次访问该目标仍须通过相同身份的完整 scope/source 预检；初始加载、冻结合同、取消、EventSink 和读取/缓存/上下文预算仍严格失败。文件删除不留下无意义的递归文件锚，只有目录保留递归扫描。该状态只是原 Turn 的上下文缓存，不另建持久账本。

原生 owner 或 transport 的未知结果不能退役为普通历史不可用状态。内部读取先记录原工具结果，再保留原 typed failure；指令 loader 即使把读取错误包装成 warning，也必须重新抛出该失败，并停止本轮的 sibling/fallback 读取。只有完整、已收束的 typed native acquisition failure 可以进入历史退役；不能从错误文案猜测成功、缺失或清理完成。

原生只读错误的 code、message、kind、raw_os_error 沿原 AppError、Wave2、Kernel 和 Agent 工具结果载荷传递；可选结构化 details 不从诊断字符串反推。普通读取保留原本地 IO source 供 pinned owner 往返，管理员读取校验 worker 的 typed failure 与 transport code 一致。投影继续有界并使用现役凭据脱敏，只有成功 acquisition 的 `Source { source: None }` 或普通 owner 已证明的真实缺失才能成为 absence；卷或共享根不可用保留真实 OS 错误。

## 进程与系统执行

Agent 的 `exec_command`、`start_process` 和后续输入、轮询、调整 PTY、取消复用 `ManagedEngineProcessOwner`。`nomi-process-runtime::ProcessSupervisor` 负责 Pipe/PTY、输出、超时、租约、watchdog、后代清理及真实终态。旧 `CapabilityPolicy`、`SandboxPolicy`、`cwd_roots`、MacSeatbelt、sandbox-exec profile 和无用途的 `bash_sandbox` 配置已经退役。不能把根目录加入允许列表来恢复旧策略。

Windows 在同一共享进程准备入口为 canonical Disk/UNC 路径选择兼容 Win32 拼写，避免 PowerShell 5.1 在 extended UNC cwd 下把 provider 位置落到其他目录。候选从原 UTF-16 构造，不 trim、折叠大小写或损失编码；只有长度、组件语义均适用，且两份真实 metadata 句柄的磁盘类型、卷与 128 位 FileID、目录类型相同时才选择兼容拼写。特殊组件、ADS、长路径或任何证明失败均保留原 canonical 路径，不拒绝原本合法的路径，不用 Set-Location、映射盘或 shell 改写伪造 cwd。这是选择时的身份核对，不宣称提供跨 CreateProcessW 的原子目录 pin；Pipe、ConPTY、Terminal 和管理员进程复用该入口。

每次操作的 `execution_identity` 缺省为 `current_user`；明确需要系统身份时选择 `administrator`。这是执行该操作的身份参数，用户不切换权限模式。文件或安装失败后，Agent 按原任务意图选择现役系统执行路径；先核对已发生效果，再继续受阻步骤，不自动重放未知部分写入或安装。

macOS 14+ 的管理员路径是 `nomi-system-execution`，通过 `SMAppService.daemon` 注册 App 内的 LaunchDaemon，用双向代码签名要求的 NSXPCConnection 传输操作。helper 内直接链接原 Rust 监督器，保持原有进程管理；主 GUI 与普通工具不以 root 运行。系统首次批准在“系统设置 → 通用 → 登录项与扩展”完成。任务等待可取消并重新检测批准；批准只代表设施可用，实际结果还须读取终态和原生效果。

每个系统进程持有其原 XPC 连接；原连接持有确切子进程和清理证据。启动 RPC 的阻塞 worker 在等待 future 取消后仍保留连接与派发围栏；quiesce 等待此前启动派发完成，不用空句柄表证明清理。连接丢失、超时或回复无法解析保留未知，不能换一个新连接洗成“未执行”。同一系统 daemon 的 OS 批准复用，不保存管理员凭据或建立授权数据库。

管理员只读 acquisition 由同一签名 helper 的子进程模式执行，通过既有 ProcessSupervisor 管理输入、10 秒 deadline、取消、断联和真实回收；不在 daemon 内留下无法取消的文件 I/O 线程。8 MiB 源编码为 base64 后使用 16 MiB 的有界输出保留，并检查截断与完整回执。acquisition 与进程 start 一起进入 transport admission 围栏，quiesce/shutdown 等待已准入调用的真实结果。

Rust worker 经 Swift/C 入口退出时，在返回前显式写完并 flush JSON 回执；写入或 flush 失败返回真实 I/O 失败。外部运行时的 `exit` 不承担 Rust 标准输出缓冲清理。退出 0 但缺少完整回执仍是结果未知，不能当作文件不存在或读取成功。

helper 从连接的真实调用用户解析 HOME，保持用户环境；系统命令没有交互桌面资格。用户要求持久的 launchd 服务、登录项、系统计划任务是原生系统产物，其运行与卸载按任务意图处理。任务临时子进程仍须清理，不通过双重 fork 假装持久化，也不把任务结束的全树清理作用到已正式注册的独立服务。

Windows 的管理员路径在同一个已安装 executable 的早期子模式运行独立 UAC runner，使用 `ShellExecuteExW(runas)` 取得并保留真实进程句柄。GUI 维持登录用户身份，不安装后台系统服务、不保存认证信息；首次认证由 Windows 自己的界面完成。runner 在该 GUI 生命周期内复用，每个逻辑 client 保留独立的现役 ProcessSupervisor，不能因一个 Session teardown 而取消其他 client 的任务。

Desktop、WebUI 与既有 CLI host 在解析命令、初始化 runtime 或打开数据集前，进入 App 库的同一个 Windows early entry；Computer extension 是否注册由库自身编译 feature 决定。WebUI 不因此强制开启 Computer 能力，Cargo 统一 feature 时也使用相同入口。

Windows 本地命名管道拒绝远程连接，每个 RPC 校验确切 parent/client 与 runner/server PID；parent 的创建时间、可执行文件、登录 Session 与 source-derived build identity 也必须匹配。请求和回复有体积及时间上限，OVERLAPPED I/O 通过原生事件等待；取消后等待内核完成才释放缓冲和 OVERLAPPED。回复绑定原 connection/request identity，完整接收后才确认管道回执。facility 关闭先封住 identity/start/acquire/desktop 准入，等待已准入调用再获取全部 supervisor 的关闭证明。派发失败或结果丢失保留 unknown，不换 runner 重发。普通/over-the-shoulder 管理员身份来自 UAC 的真实 token；命令显式环境保留原登录用户的 USERPROFILE/HOME 等值，环境名称按 Windows 原生大小写比较合并，显式 PATH 不能被 inherited Path 覆盖。管理员 token 的账号和 HKCU 仍是系统真实身份。

管理员 source/scope 获取通过同一 runner 的 `--readonly-acquire` 子进程，复用原进程监督、typed acquisition、Windows pinned native-file adapter 和原 FileService 投影；普通读不借用管理员缓存。runner 独立观察 parent 进程句柄，宿主退出先关闭准入再回收临时进程树。正常 App 关闭在数据库关闭前取得原监督器的 exact shutdown 并确认 runner 真正退出，供升级、卸载安全替换同一 executable。它没有持久的系统注册项；已正式注册的独立计划任务或服务按用户意图保留。

Windows 高完整性桌面桥也使用该 runner 的同登录 Session，在现役 Computer owner 的 canonical 观察/效果/设备锁之内派发。每个 client 的原 ComputerTool/UIA cache 独立存在；切换执行者废弃旧观察并要求 fresh observe，不重放输入。每次真实设备调用核对 Default input desktop，锁屏与安全桌面交回系统界面，不切换或自动输入认证。具体编译、原生 UAC、桌面、签名与安装验收分开记录于 [Windows 验证](../validation/global-os-windows-2026-10-17.zh.md)，源码接入不等于这些项目已通过。

Windows 聚焦使用一次原生 `SetForegroundWindow`。目标不存在或系统在任何变更前明确拒绝时，返回确定未执行并废弃旧观察；系统接受后，恢复最小化窗口及有界响应/前台核对失败仍是 uncertain。不会附加线程输入、注入按键或改前台锁策略来争抢焦点。既有 uncertain Effect 继续由同一物理桌面的持久未结算围栏保护；后续修复、换 Session 或升级 App 均不抹除它，只有现役 owner 对真实结果的核实对账能追加结算事实。

高完整性桥的 remote focus 在派发前可传递 App 已具备的前台资格：仅当真实前台 PID 属于当前 App、线程和输入桌面均为 Default、原 facility 未关闭且 retained helper 进程句柄仍存活时，对该句柄取得的确切 PID 调用 [AllowSetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow)。不接受模型或 RPC 提供的 PID，不向所有进程开放，不主动激活 App；系统拒绝时不重复请求或注入输入。此资格可由后续用户输入撤销，成功委托也不能证明下一次聚焦成功，仍须保留真实 native 结果及原 Effect 结算。

Linux 使用固定 `/usr/bin/pkexec --disable-internal-agent /usr/libexec/nomifun-system-runner --stdio`，由发行版 polkit 完成系统认证。普通 GUI 继续使用登录用户；runner 通过原监督器管理 Pipe/PTY 与 typed readonly worker。每连接使用有界帧和并发 RPC，握手核对真实 root、`PKEXEC_UID` 对应的调用用户及同源 build identity。调用用户的 HOME/USER/LOGNAME 从系统用户记录取得，不使用 root HOME；认证等待可取消，认证完成前不派发操作。丢失回执保持 unknown，断联和输出写失败进入原监督器清理，不能用新连接重放未知操作。

Linux 的 pkexec 必须由拥有原 child 整个寿命的专用创建线程启动；不能在返回后会被回收的临时 blocking thread 上创建。pkexec 的父线程死亡保护保持启用，client 关闭 stdin 后创建线程仍等待原 runner 完成 EOF cleanup 并实际退出回收。原生认证错误保留真实 exit code/signal 与有界 stderr 诊断，不把无回执或信号退出概括为密码错误。

Linux `.deb`/`.rpm` 将 runner 和固定 action 安装为系统文件，开发构建与 AppImage 携带同版本显式安装入口。action 绑定固定 executable 和 `--stdio`，各 session 类型均使用系统的 `auth_admin_keep`，不安装 sudoers、不保存密码，也不由 GUI 启动时偷偷写系统目录。缺组件、无认证代理、拒绝、取消、超时与未知结果分别表达。实际认证和原生产品通过范围见 [Linux 验证](../validation/global-os-linux-2026-10-17.zh.md)。

三平台的实现、编译、系统认证和真实产品验收各自计证；某一平台的 runner 或隔离夹具不能代替另一平台的原生证明。

## 命令、环境与依赖

`nomifun-runtime::prepare_process_launch` 和 `process_environment` 是 Agent、Terminal、MCP stdio 及组件调用的共同准备入口。每次启动重新组合现有工具、用户 PATH、持久 CLI 安装位置及已经准备的组件；不在线程运行期间反复写全局环境。原生程序使用 executable + literal argv，只有明确 shell 调用解释 shell 语法。Windows `.cmd/.bat/.ps1` 使用进程运行库的共享 encoder，Supervisor 与原 MCP ChildProcessBuilder 复用同一实现；Terminal 私有 PowerShell wrapper 已退役。批处理采用实际 cmd 编码并禁用 delayed expansion，无法无损表达的 CR/LF 参数在启动前明确拒绝；PowerShell 脚本通过数据字符串数组传递参数，保留系统执行策略，不从脚本文本另建启动后端。

已选系统 WinPS5.1 或隐式 PS1 的环境准备保留原 task 配置来源：继承 `PSModulePath` 时，在其完整原值前补实际 Windows 目录下 WinPS 的系统 Modules 路径，保留父脚本临时追加的自定义模块目录；用户显式值（包括空）仍完全覆盖。Node、cmd、用户 pwsh 和其他程序保持其环境。Agent shell 使用同一 typed target 准备，管理员 owner 与底层启动器复用同一原生兼容入口。此处理针对 Microsoft 记录的[中间进程继承 PS7 模块路径问题](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_psmodulepath#starting-windows-powershell-from-powershell-7)，不改注册表、用户 profile 或进程全局环境；启动器不默认加入 ExecutionPolicy Bypass。

缺少任务必要的受管运行时自动准备；状态查询只读。Terminal、MCP 与 Relay 的额外下载确认及专用状态/UI 已退役，保留真实进度、校验、重试与取消，启动请求不因 UI 进度轮询而重放。供应器保留下载完整性、并发单飞、临时解包、取消和原子发布。

Node 使用完整官方发行目录，保留 npm/npx、模块、launchers、合法链接和执行位；完整发行身份及缓存布局使旧单文件缓存失效。Bun 保留本身用途。用户工具安装根独立于受管运行时缓存和 Agent 数据，升级、清理运行时及清空 Agent 不删除已装用户工具。用户自有 `.npmrc`、prefix、PATH、registry、代理、证书和安装方式继续生效；系统安装、原生 installer 和服务使用通用进程及系统执行接口，不强行改为 npm 用户安装。

CLI 附带 Skill 必须进入现役技能库导入与发现路径，写到第三方 Agent 目录不表示 NomiFun 已就绪。当前活动 Turn 的技能快照保持冻结，后续使用通过已有刷新/选择机制。

## 默认个人 Agent 与旧会话

默认聊天 `chat.minimal` 正式具备 `workspace.files` 六个动作和 `workspace.process` 七个动作；资源由原 resolver 自动供给。`assistant.general`、伙伴等原有相应能力继续使用同一工具路径。

当前代旧官方 exact-empty 聊天会话在空闲新 Turn/模型准备边界，经 `default_local_os_evolution` 的精确合同证明与原 binding CAS 追加正式转换，仅补这两根能力及其真实资源。旧 Revision/Snapshot、历史事实和 Session 身份不回填；活动、暂停、Remote、Attempt、未知效果和 native checkpoint 围栏不放宽。用户明确裁剪、专用机器人、已注册渠道访客以及委派工具裁剪保持原任务分工语义，不从显示名称推断扩权。新发布未授权的描述符可以演进元数据，已选 Action、效果、资源和实际授权不因此扩大。

指令 scope 与其 AGENTS 分页读取继承待执行调用的真实身份，缓存按 `(identity, native path)` 隔离；普通读取不能借用管理员缓存。每个真实串行效果结算后、下一调用预检前失效指令缓存，最后一次效果后的模型准备也继续刷新。指令变化时保留未执行回执，读取新上下文后重新提出调用，不重放已有结果。普通内部读取可以省略身份字段使用正式缺省；管理员读取必须被冻结 schema 明确接受。旧会话通过现役空闲 ContextOnly 合同转换取得新 schema，精确 native checkpoint 或 frozen binding 不因新参数被私下改写。

## 桌面、媒体、网络与真实边界

Computer 普通操作在登录桌面的 App 进程执行，每次操作按实际需要检测原生许可。Windows 高完整性目标使用同登录 Session、经现役 UAC runner 认证的独立 Computer adapter，保留同一物理目标 owner、generation 与 Effect；切换后先返回确定未执行，再要求新观察，旧 ref、截图或未知输入不重放。安全桌面和锁屏按真实系统限制拒绝，不切换或自动输入。Browser 保持 Windows WebView2、macOS WKWebView 的已定结构；网站媒体许可仍由实际宿主与 OS 隐私许可决定。

权限库存包含实际 PID 和可解析的真实 App bundle 身份。AX/CG 返回 false 只证明 `not_granted`；FDA、Automation 和局域网没有适用于所有目标的全局可靠探针，保留 unknown，实际目标操作再给真实结果。Camera/Microphone 使用原生状态；使用说明与 Apple Events automation entitlement 随 App 打包。详见[系统许可](system-permissions.zh.md)。

普通监听、本机/局域网访问、原生安装、服务与系统设置复用上述通用路径。SIP、UAC、安全桌面、TCC、ACL、网络设施和实际设备存在性仍按系统真实限制表达。Linux Computer 已接入正式模块与资源选择：完整 X11 后端核实真实服务器、窗口管理器及 EWMH 激活结果，不以点击窗口中心代替聚焦，也不把 XWayland 当作全局桌面。Wayland 使用系统 RemoteDesktop/ScreenCast portal、受控会话与 PipeWire/GStreamer 帧，通过同一 Computer 工具链观察和输入；缺 backend、尚未授权、session 关闭与失效必须如实返回，输入不能隐式重新弹出授权后继续旧动作。Wayland 没有通用的全局窗口枚举/聚焦和指针查询接口，这些操作保留明确限制，不用 XWayland 规避 compositor。

独立 X11 虚拟桌面测试、portal 协议 fixture 与真实用户桌面授权分别计证，见 [Linux 桌面验证](../validation/global-os-linux-desktop-2026-10-17.zh.md)。Windows 普通和高完整性桌面的具体来源、真实结果与未覆盖项见 [Windows 验证](../validation/global-os-windows-2026-10-17.zh.md)，源码存在不替代原生验收。

## 验证与平台接续

完整证据、首次失败和三平台状态以[本轮验证记录](../validation/global-os-2026-10-16.zh.md)为准。确定性 fixture 不等于模型推理能力或正式 App 原生验收，单个文件/进程回归也不等于整个系统矩阵。

Windows 本机实现、MSVC 构建、UAC 与产品链的后续独立证据见 [2026-10-17 Windows 原生接续记录](../validation/global-os-windows-2026-10-17.zh.md)。原 macOS 记录中的 Windows 待验状态是当时快照，不覆盖后续本机事实。

可复制的新会话交接：[Windows](../handoffs/global-os-windows.zh.md)、[Linux](../handoffs/global-os-linux.zh.md)。交接完成仅表示接续材料齐全，不表示目标平台实现或验收完成。
