# Windows 接续任务：默认可信的全局操作系统能力

本文件记录当前 Windows 合同与交付边界。接续时先核实实际 checkout、环境及最新回执，按新增任务选择检查；不要重复执行已经完成的业务，也不要重放未结算操作。源码、编译、原生验收和正式签名发行是不同事实。

本轮两个发布阻塞已修复并取得新构建原生证据：显式 native 文件动作的跨项目持久冲突围栏，以及 Windows UAC 取消在管理员只读/指令预检中的 typed 错误链。生产冻结为 `53da7b1cf5d5bb9eca6fd3ed255625de7d3a2199`、tree `860cd8804a50311d3b601971eabc3beb1ba332ea`，包含普通合并 main 至 `ba107fa011ddc8cf92d68033504a6bdbf4f774df`。debug `a78a41e7…`、release/NSIS b69b、实际安装版 `72d9e82d…` 各有独立完整摘要、业务和退出回执；后续验收器/文档提交不冒充另一次产品构建。完整结果及最终 Git 交付按[验证记录](../validation/global-os-windows-2026-10-17.zh.md)和同一个 [PR26](https://github.com/nomifun/nomifun-desktop/pull/26) 核对。

原数据根的 Desktop230/UNC336、原 pending1、owner/JWT 与旧故障资料完整保留。新 App 的两个项目均在新增修改 Effect 前受到 legacy 围栏拒绝；管理员允许/等待及真实 raw1223 取消已分别验证。取消专用脚本的原两份 FAILED 保留：一次实际点击允许，另一次命中了验证器的失败 Tips/checkpoint/digest 假设；后者通过同一原 Session 的独立 GET 补证核实，没有重提 Turn 或认证。独立正向夹具的65项文件/进程/UNC/Browser业务及实际安装版业务通过，均不追认旧 pending。媒体、其他管理员账号矩阵与正式签名不在这些证据内；候选未签名，日常0.8.4未替换。

用户于2026-10-18明确选定简化边界：保留 NomiFun 自己准入的显式文件 Action 持久冲突围栏，接受用户或其他程序并发编辑文件、链接和 staging 名称的外部竞争风险，由用户协调。这替代先前要求抵御任意外部并发的严格对象一致保证；不增加权限模式、确认流程或文件系统拦截设施。已有部分效果、unknown、cleanup 与禁止自动重放保持，旧失败和未结算事实不改。原生 source-link 干预的同卷移动、跨卷 metadata 变化已实测并保留，不能把它们写成“外部目标未受影响”。

## 产品合同

最高原则：**最大权限赋予，最可靠实现，不要搞骚操作。** NomiFun 是用户完全信任的个人 Agent。目标是文件和卷、程序与进程、应用安装、系统配置、服务和后台任务、桌面与跨应用操作、网络和已经提供的设备能力。npm 是一条依赖回归，不能代替操作系统能力。

普通操作使用真实登录用户；需要系统身份、交互桌面或独立许可时，由应用内部选择可靠的 Windows 原生执行路径。无需用户切换权限模式，不让整个 GUI 长期以 Administrator/LocalSystem 运行。不增加权限分级、可信开关、策略中心、命令白名单、授权数据库或第二套 Agent 后端。允许建立充分可靠的 helper/runner/service，生命周期依系统需要确定，复用现役任务、结果和取消链。

workspace 只承担默认目录、项目归属、watcher 和 VCS 语义；不能限制本机绝对路径、合法 `..`、链接、其他盘符、UNC 和进程 cwd。不得用把所有盘符加入允许列表模拟放开。项目外文件不能伪造为项目内产物。保留 OS ACL、UAC、UIPI、安全桌面和账号认证；不关闭 UAC 或其他系统保护，不向模型或日志暴露凭据。

用户明确的“只读”“不要执行”“停止”始终有效。Session/Turn/Event/Payload/Effect 是唯一事实链；保留登录身份、owner、冻结 binding、效果结算、取消、native checkpoint 和清理证明。未知执行结果不能自动重放，不能新增授权或重试账本。

## 先核对完整源码与环境

仓库名称为 `nomifun-desktop`，使用目标电脑实际路径。本次 Windows 初始 checkout 为 `C:\Users\muli\workplace-c\nomifun-desktop`，在 `main`、HEAD `d1e968bfe262c39deeddcc460f3df27ab26ff0f7`，当时 tracked/staged/untracked 均干净。该提交已包含此前共享实现与 Git 整合，本次 Windows 改造在其后工作树继续。最终完整提交、推送与交付以实际 Git 回执和当前源码为准，不能把初始 HEAD 当作本次改造已提交的证明。

优先通过 Git 取得包含当前 Windows 实现、测试、构建资产和文档的完整交付。先检查 HEAD、diff 与全部必要新增源码，不重复应用历史快照，不只复制 tracked diff。遇到无关改动保留它，另建合适的干净 checkout；不得 `reset/clean` 或覆盖其他写入者。历史 source-review 的 BASE、摘要和旧恢复操作保留在 Git 历史；本文件不再提供旧 patch 恢复步骤，不更改历史 BASE、不猜新摘要、不混用导出批次。若实际交付需要 bundle，须由交付者冻结完整源码并提供当批实际清单与摘要，不能用旧基线强套当前 checkout。

Windows 实现的历史起点为 `1b9c317ecaf5a5b7f70de382defbaf6214849fff`；91410、925及其变体属于此前交付来源，具体合并、首次失败和原生结果保留在验证记录及 Git 历史，不再作为当前构建输入。当前生产冻结以本文件开头的53da为准；不 rebase、force 或覆盖共享历史。

925的876项Rust、14项App、480项UI及旧8dbc/b4安装周期仅证明当时来源，不计入53da的新二进制结果。本轮9e9b的完整 Host98与最终53da的 App13、scheduler2、UI/边界、原生及安装证据分别列于验证记录，不混成一次完整重跑。原34d pending1/336事实与BoHTo0仍保留，不重放或代填owner核实；跨项目围栏本轮已修复，外部并发边界仍按用户明确决定执行。

完整源码继续通过同一个PR26交付，最终提交、普通推送及合并以实际Git回执为准。737c的robot变化与ba107fa的委派取消变化均已普通合入当前53da，并补足取消用例与完整检查。最终合并若仅增加本轮验收器/文档，明确记录该差异；若再引入产品变化，须针对实际来源补验，不借旧binary宣称新源码已验收。

开始前执行并保留结果：

```powershell
Get-Location
git status --short
git branch --show-current
git rev-parse HEAD
git diff --stat HEAD
git diff --name-status HEAD
git ls-files --others --exclude-standard
rustc -vV
rustup target list --installed
bun --version
```

完整阅读 `AGENTS.md`、[Agent Session 架构](../architecture/agent-session.zh.md)、[当前全局能力架构](../architecture/global-os-capabilities.zh.md)、[Windows 原生验证记录](../validation/global-os-windows-2026-10-17.zh.md)，并核对 `git diff HEAD` 与必要 untracked 源码。较早 macOS 事实与三平台历史快照见[原总验证记录](../validation/global-os-2026-10-16.zh.md)，不能当成当前 Windows 待设计清单或本机证据。renderer 最小桌面合同为 880×600，不添加手机布局或移动验收。

本次实际机器为 Windows 10 Pro 22H2 x64、19045.6466；Rust 1.99.0、`x86_64-pc-windows-msvc`、Bun 1.4.2。登录用户为 `DESKTOP-NF7D5NP\muli`、Session 1，App 是普通 medium token。后续环境必须重新核对，不能沿用旧 PID、API、端口、组件身份或 token。原始命令与原生报告在本次仓库外 `windows-native/` 证据目录；路径、交付和摘要按 Windows 验证记录及实际回执。私有 owner JWT、数据集、凭据、截图、二进制和缓存不提交。

## 当前源码已经接通的入口

以下是实现状态，不是整个平台已验收声明。具体命令、首次失败、修后结果、App/组件身份和报告摘要统一维护于 [Windows 验证记录](../validation/global-os-windows-2026-10-17.zh.md)，不在交接中复制易过期的通过清单。

| 能力 | 当前实现与必须保持的合同 |
| --- | --- |
| 全局文件与 cwd | canonical FileService/native-files 与进程 owner 接受本机路径，Windows 使用 pinned handle、真实 metadata、ACL、reparse 和卷身份。外部收据为绝对 `native_path`、`workspace_path=None`，不制造项目事件/文件卡；默认目录后来删除也不能阻断合法绝对目标。 |
| 原生 rename/ADS | 普通文件和目录使用 pinned source DELETE 句柄、pinned destination directory 与 `NtSetInformationFile(FileRenameInformation)`，不覆盖目标，保留跨卷错误。ADS 使用真实 stream 语义，不把不支持的原子发布改为截断写或复制删除。设备对象不冒充 regular file，旧全面拒绝 alias/ADS 的断言不能恢复。 |
| 同 executable UAC runner | `nomi-system-execution` 在同一安装 executable 的 early entry 使用 `ShellExecuteExW(runas)` 和 authenticated bounded named pipe；Desktop/WebUI/CLI host 复用 App 库入口。每个逻辑 client 使用原 ProcessSupervisor，GUI 保持登录用户；没有第二 Agent 后端、凭据缓存或持久系统注册项。 |
| 管理员进程 | 原 exec/start 的 `execution_identity: current_user` 或 `administrator` 选择该次身份，poll/terminal/write/close/resize/cancel/quiesce/shutdown 复用原 owner。Pipe/ConPTY/Job、exact identity、deadline 和 watchdog 管理生命周期；UAC 允许或 ShellExecute 成功不是任务完成。 |
| 管理员只读 | 正式 `workspace.files/read` / `read_file` 的 text、image、instruction_scope 及分页，经同 runner 的受监管 `--readonly-acquire` worker 获取 typed acquisition，再由原 FileService 投影；绝不 ordinary fallback、shell cat 或复制 decoder。 |
| 高完整性桌面桥 | `windows_computer.rs` 经同登录 Session 的 runner extension 复用现役 ComputerTool/UIA、generation/ref/Effect/operation lock。切换执行者废弃观察并要求 fresh observe，未知回复不重试输入；安全桌面和锁屏真实交接，不切桌面或输入认证。 |
| 合法前台资格 | focus 仅一次 `SetForegroundWindow`；任何变化前明确拒绝为 typed 未执行，accepted 后恢复/响应/前台核对不明仍 uncertain。remote focus 仅在 App 真实前台、Default input/thread desktop、facility 开放且 retained helper handle 存活时，对其确切 PID 调用 `AllowSetForegroundWindow`；不主动激活、不用 ASFW_ANY/AttachThreadInput/Alt 注入/改前台锁，委托成功也不证明 focus 成功。 |
| 共享脚本启动 | Agent Pipe/PTY、Terminal、MCP 和 raw ChildProcessBuilder 复用 `.cmd/.bat/.ps1` encoder，Terminal 私有 wrapper 已退役。native executable 保留 literal argv，明确 shell 保留 shell 语义；batch 无法无损表达的 CR/LF/NUL 在启动前明确失败，不借二次解析执行参数。 |
| 工具环境 | `prepare_process_launch`、typed `process_environment_for` 和原生大小写环境键合并按目标准备。显式 PATH 不混宿主 PATH，仍按原合同跟随持久 user CLI/已验证 runtime fallback；保留真实 USERPROFILE/HOME、registry、代理/证书。WinPS 的 ambient PSModulePath 补可信系统 Modules 前缀且完整保留 custom；显式值包括空不改、其他程序不加工，不默认 ExecutionPolicy Bypass。 |
| 完整 Node 与 CLI | 官方完整 Windows distribution 校验整个树，npm/npx 经 node+JS 入口+literal argv；持久用户 CLI 与 Agent dataset/runtime cache 分离。用户 prefix/`.npmrc`/PATH 等继续生效，自动准备保留进度、取消与完整性，不加二次下载确认/重放。 |
| Browser 全局上传 | Windows WebView2 原上传入口已使用全局 native 来源和 pinned snapshot worker，保留 metadata/摘要/取消、retained File 生命周期、单次16文件/64MiB及runtime128文件/256MiB预算。ADS 无法准确投影 Web File.name 时在 effect 前明确拒绝。 |
| instruction Entry/Follow | 正式 instruction_scope 的 `path_semantics` 区分 follow 内容与 delete/rename 的 entry；entry 固定父目录并保留末级名称，不遍历被删链接目标。schema/owner/worker/projection/cache 语义一致，不能把 unlink 预检变成删除目标。 |

管理员只读仍只有原 read Action/ReadLocal/resources/冻结 ToolPlan；其他文件动作、VCS、search 不接受 selector，管理员写入经现役 process。Source 上限8MiB，text 严格UTF-8、完整源SHA/expected_sha256、分页/BOM/行尾和图片格式/4MiB/envelope预算保持原投影。只有真实缺失的 Source None 是 absence；拒绝、悬空链接、非目录祖先、变化和类型错误保留 typed code/message/kind/raw_os_error，不被 missing_ok 吞掉。worker exit0/reaped 还须完整有界 typed JSON 与匹配 receipt，显式 write_all/flush 不能退回缓冲退出假成功。

指令/source缓存按执行身份、native path 和适用语义隔离；默认或显式 current_user 不借管理员缓存。管理员 cwd 发现与每页 AGENTS/source 使用同一身份；效果在原 ToolCompleted 落盘后按既有规则及时失效，下一调用重新预检。新规则延期用新call ID，不能倒推启动前已读。旧 strict schema 内部普通读省略 selector；管理员 schema 未明确接受则派发前拒绝，不降级。旧会话只沿原 idle/CAS/teardown/unknown/native-checkpoint 合同演进，不回填历史或强迫新建 Agent。

named pipe 核对两端确切PID、parent创建时间/映像/登录Session及来源派生组件身份，拒绝远程连接；有界请求/回复与 OVERLAPPED cancel/join 保持原生缓冲生命周期。facility 先关闭准入，再等待既有调用及 supervisor 的真实关闭，最终等待 runner 句柄实际 signaled。`op: terminal` 只读真实终态，不 poll/续lease/消费running输出，晚stdin/resize也先取真实终态。断联、结果不完整和宿主中断保留unknown，不换连接重发；cleanup exact 不追认先前物理效果。

## 必须保留的事实与未完成项

canonical write/patch/delete/rename 捕获同一次实际执行持有的 native pins，并将 typed target facts 保存在原 Effect started payload；Store 在原准入事务内检查所有 pending/unknown，不增加目标表、授权或重试账本。原 owner/project/resource 与活动锁继续保护各自边界。普通文件 Entry 比较对象和命名空间；publication 与动态目录明确保守保护实际原生卷，避免新对象及后来硬链接绕过。旧 explicit-file Effect 若没有充分 target facts，保守阻断无法证明不相交的新文件修改；不从当前 symlink/FileID 回填过去，不结算原34d。opaque process 保持原 project/resource 围栏，不能声称覆盖任意 shell 的全部文件副作用。精确覆盖与限制见[当前架构](../architecture/global-os-capabilities.zh.md)。

管理员只读与指令预检使用原生 owner 的 typed `SystemExecutionFailure`，沿 Kernel/Runtime、原 canonical failed Turn、API 和 renderer 传递真实 code、kind/raw OS error 及已知未启动/清理证据。启动前1223不是资源未绑定、ACL拒绝或用户 Stop；已派发/断连 unknown 不能降级为未执行取消。无自动再认证、普通身份 fallback 或重放，真正的 Forbidden/binding 语义保留。真实 UAC 验收由用户在系统窗口完成，脚本不操作安全桌面。

较早普通 Calculator focus 真实失败曾产生unknown，原failed报告和事件不改。用户亲自核实“那次聚焦失败”后，经原 Session owner 接口单次追加attestation/reconciliation，当前rejected、pending/unknown=0；随后同原Session新Turn已准确清理本次Calculator，当前升级保留证据为完整230事件前缀，含原80和原78 uncertain前缀。不能把较早80事件快照当作当前全部事实。修后mock、迟到的前台状态、JWT、重建数据集或新Session均不消除unknown；后续继续沿实际owner/Session，不重放原输入。

候选App30552的UNC patch另有真实未结算失败：原文件当前恢复为before，但仍有before/after两份备份，原336事件与pending1完整保留。用户对owner verification回答“不知道”，不得代写核实或调用对账API；原share/文件不能清理或重放。独立、物理与UNC均不重叠的16项原生控制实测UNC的FileDispositionInfoEx返回87，而basic FileDispositionInfo须全部自身metadata/DELETE handles关闭后才确认absent。这仅用于兼容修复，不恢复原操作丢失的错误或追认其结果。若后续正常升级App，必须保持同DataRoot/JWT与原336前缀、owner/binding/resource/Effect身份；仅真实typed native restart quarantine可将原pending转unknown，不能settle、自动重放或换dataset。具体报告与摘要见Windows验证记录。

用户最新明确未查看旧UNC材料，只关心当前Windows开发与最终测试，并要求由本任务自主完成、判断测试资源无用清理有用保留。不要再要求用户确认过程材料；本轮将原34d未结算故障作为有用证据保留，不把自动化指示代填为owner亲自核实。已完成且归属、使用状态可证明的其他夹具自主清理。此保留不阻止其他不相交资源的源码修复、验收、安装和交付。

早期App30552已物理退出，旧API/PID不能继续使用。单次现役退出请求后原15进程含helper全部gone，但stdout明确因原UNC runtime cleanup未证而在bounded重试后失败退出；exit_code旧观察为null，不能称完整清理成功。Browser13业务通过而其归属快照BoHTo0仍残留，独立记录FAIL，保留为有用故障证据。现役Browser close已有native退役后显式files.close，未为残留挪unknown producer围栏或改Browser生产。后续App22600/16a的Minimal、管理员和新Browser回归通过；其exact App exit0、15进程gone及5aWxzd快照absent均已独立证明。再后App9028/12cf也已正常退出0、9进程gone。第一批91410生产冻结的debug为d22d473b开头、组件853a949a开头，沿原数据根/JWT和230/336双守卫升级；UNC16、管理员8、Minimal12及Browser13业务已通过。具体收尾、安装周期、实际计数和后继构建结果以Windows验证记录为准，不沿用旧PID/API。

早期UNC链发现WinPS5.1的extended UNC cwd使provider误落C盘；两拼写原生控制已确认plain UNC语义正确。共享request准备现以raw UTF-16及真实metadata句柄的卷/FileID/类型核对选择兼容拼写，不能在driver里Set-Location或降低cwd断言。修后共享进程253项通过；12cf候选已证明provider/literal语义正确，随后暴露share移除后的历史scope读取失败。最终typed native error和历史scope/fatal owner围栏修复已进入91410生产冻结，合并Runtime458项通过，d22d候选完整UNC16已原生通过。通用新scope延期的driver修复仅接受canonical零派发证明，以原参数和fresh call继续，原失败及同Session cleanup保留；不重放未知调用。唯一NSIS实际安装/卸载循环按Windows验证记录的独立产物与报告消费。

验收App现役启动代码会改写nomifun:// URI登记，本次缺首次启动前的原值。用户已在具体目标问题下授权按最优方式处理；第一批91410最后一份默认测试App退出且独立卸载库存核对完成后，已pin/hash核对日常0.8.4安装 `D:\Users\muli\AppData\Local\Programs\NomiFun\nomifun-desktop.exe` 与当时d22 handler归属，仅将两个既存默认值指回日常安装，32/64视图读回匹配。第一批f65ea425回执SHA-256 `22b576731a690069c9651d2ac2c9419bf80d68352166d78de0399b8571464798`，不启动App、不新增/删除键或改ACL。此为指定目标修复，不冒充历史恢复；用户的“旧数据不在乎”不结算原UNC、不扩大为删数据集。后续fixture在每次launch前后及close只读记录精确HKCU键type/长度/摘要，不保存未知raw command，不自动恢复。新close observer在派发前保留exact进程句柄并核对creation，真实等待后读取exit code；进程退出仍与整体资源清理分开。

Terminal合法leading-space cwd与recursive entry scope中的命名指令链接已按现役入口修复，定向检查与首次失败分别保留。candidate App的普通/高完整性桌面、Browser、SCM及其他任务已有各自实际结果，按每份binary与报告的范围读取 [Windows 验证记录](../validation/global-os-windows-2026-10-17.zh.md)。第一批debug与release/NSIS已核对同一91410生产冻结与853a949a组件身份，完整安装周期另有实际成功回执；不同中间构建的结果仍不能合并成最终源码全矩阵认证，未覆盖项保持未覆盖。

另有真实接续失败：保留计划任务的原Session在App重启后，因默认workspace已被夹具清理，POST新Turn返回`WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE`。按明确授权，仅恢复原exact夹具的已知空workspace并核对owner/marker/无reparse父级，随后原Session新Turn完成任务停止/注销、确切PID消失和canonical目录删除，pending/unknown=0。原400、原system失败报告与事件前缀均保留；宿主没有操作Task。App准入、知识unbound lease及files/process Effect key现复用真实missing的原冻结anchor，不创建目录，VCS/artifact仍要求物理root。候选4b暴露第二接缝的失败保留；App22600/16a的`--missing-default`完整20项已通过，包括外部绝对文件链、start/running/cancel/reaped、确切PID退出、默认目录持续absent及原冻结身份不变。后续新构建结果按来源分别记账。

| 验收面 | 必须分别记录的结果与不能替代的范围 |
| --- | --- |
| 文件与卷 | workspace外read/write/search/patch/rename/delete/cwd，盘符/UNC/分隔符/链接/ACL/属主/metadata/父目录并发、受保护目录及实际外接/网络卷。已有C/D、ACL、junction等结果不代表全部文件系统；OS未提供的大小写目录能力单独记未覆盖。 |
| 程序与进程 | bare/明确shell/Pipe/PTY/GUI、普通和管理员身份、真实环境/cwd、literal空参数/中文/引号/元字符，cancel/timeout/host exit/后代/natural exit/late I/O的确切PID/创建时间与清理。 |
| 管理员只读/指令 | ordinary ACL真实拒绝，administrator text/hash/page/image/scope同投影、缓存隔离、cwd新规则延期/freshcall、batch effect→read；取消/断联/unavailable不ordinary fallback。 |
| 应用安装 | 唯一NSIS variant实际canonical install/uninstall同Session新Turn，以及独立打包bootstrap、安装后正常setup/login/退出/uninstall；严格唯一product/id/protocol/mainBinary/currentUser、资源/registry/进程与数据保留。host直接installer不冒充默认Agent能力，exit0还须独立读回，unknown不重装；另需真实可回滚的系统权限操作证据。 |
| 配置、服务/任务与网络 | UUID HKCU恢复、SCM或Task Scheduler注册/运行/查询/停止/卸载、本机/局域网访问，App真正退出后的同一任务/PID存续与原owner重连清理。阶段完成/等待checkpoint不等于整体成功；失败在原链清理，不留任务等构建。 |
| 普通/高完整性桌面 | 分别截图/UIA/freshref/focus/click/type/独立结果/关闭自有应用、多屏与登录桌面；委托mock不代替焦点，runner成功不代替高UIA/input；安全桌面准确交接。 |
| Browser | WebView2真file input/原生chooser，外部绝对/相对../UNC/reparse/卷、ACL/race、name/bytes/mtime/retained File/dispose/异常关闭；上传、原unlink失败与Entry修后canonical cleanup分别按实际binary记账，不能互相追认。macOS WKWebView既有UnsupportedAction不借此改引擎。 |
| UAC与账号 | 允许/拒绝/取消/无管理员/设施缺失/over-the-shoulder不同账号/认证中断；真实token/SID/HKCU与原登录环境，凭据只进系统UI，不循环认证或重放部分安装。 |
| 媒体设备 | 用户选择保持设备状态；实际库存缺可用输入，WebView2 audio/video均NotFound、无track，清理有证。**采集、相机及拒绝/撤回/恢复隐私矩阵未覆盖**；不启用禁用设备或替用户改隐私，不把无设备/unknown写成granted或拒绝。 |
| 依赖/持久工具 | 完整Node/npm/npx、本地bin/lifecycle、后续Turn/MCP/重启/cache删除修复/卸载；专门依赖回归不代替OS安装/服务/桌面，Agent clean cut不删用户工具。 |
| 打包/签名 | 从现役`bun run build:win`及release lock产出，核对实际渲染NSI/资源/架构/安装路径/升级退出/卸载。当前无可用代码签名证书；unsigned variant只记unsigned验收，不声称正式Authenticode发布。 |

本机已有Windows/MSVC/WebView2/真实UAC条件，不能沿用“源机只有macOS，Windows待设计”的旧说明。区别工程缺陷、构建故障、待原生验收、设备/账号/网络卷/签名条件不足和OS真实拒绝。此前MSBuild tracking的tlog故障已有实际exe、目录只读与短路径对照，不等于缺编译器，不因此改系统LongPaths/UAC/ACL。缺设备或保护边界也不等于Windows不支持。

## 接续执行与检查

1. 对照当前Git、53da生产冻结及Windows验证记录的最新回执，核实实际executable、组件身份、验证驱动和最终交付commit。实现、源码检查、App原生及NSIS结果分别消费，不沿用旧构建摘要，不把历史候选称作当前最终产物；保留历史失败、数据集和owner事实。
2. 原数据根继续使用原owner/JWT及Desktop230/UNC336双守卫，不重新setup/login或新建数据集解决旧pending。独立positive和NSIS安装版回归各用明确标记的新数据根并正常WebView2 setup/login，单独标记来源；其成功不证明旧pending已解决。均核对port公告/PID/token与实际caller，再用`global-os-native.mjs`公开默认`chat.minimal`/`assistant.general`场景；不手补grant/resources。本机scripted provider不代表联网模型推理。
3. 先运行无需新账号/设备/联网安装的普通链，再按条件运行管理员、系统任务、Browser、桌面、installer独立场景。失败先查原canonical结算；仅在已结算允许的原链新Turn准确cleanup，不重跑未知业务。保留命令、首次失败、修后结果、原生readback与cleanup。
4. installer唯一variant先readonly prepare；canonical安装/卸载已完成且pending/unknown=0、唯一对象不存在后，再独立bootstrap周期，不并行installer。NSIS uninstaller的`_?=<exact root>`是最后无引号raw tail，现役固定ProcessStartInfo原生调用同步等待真实PID；一般argv引号或外层exit0不能证明嵌套卸载完成。
5. 只操作本轮唯一夹具，保留日常App/注册/工具/配置及其他数据集。foreground/锁屏/授权不足准确交回用户，不造权限开关；媒体按用户选择保持未覆盖。

先核对`package.json`、Cargo feature与测试名，选择最小有效检查。受影响process/system/native-files/file/runtime/Terminal/MCP/Computer/A11y/App、canonical history/reset/consumer、取消/native recovery跑相应测试；跨模块收尾跑`bun run check`或足够的现役检查。相关边界包括`check:process-runtime-boundary`、`check:external-tools`、`check:agent-session-boundary`、`check:windows-installer`、`check:docs`；renderer/UI-rule变化另跑`check:desktop-ui-boundary`及定向UI/typecheck。遵守仓库rustfmt约束，不整仓格式化。交叉cfg check、mock、PowerShell AST、库级UAC、完整App产品链分别记账，不能互代。

## 禁止恢复与交付要求

不得恢复workspace文件/cwd containment、`cwd_roots`/盘符允许列表、CapabilityPolicy/SandboxPolicy/DenySpawn、MacSeatbelt/sandbox-exec或“受限默认+可信模式”双轨。不得恢复额外runtime下载确认、MCP私有PATH/shell split、Terminal独立shim、隐式ExecutionPolicy Bypass、foreground绕过、安全桌面输入、旧gateway/transcript/投影回上下文/双写或第二binding/checkpoint/授权/重试账本。structured command不因args空拆整条命令，不trim合法路径和argv，保留NUL/预算/类型/存在性/生命周期校验。

不能用`MOVEFILE_COPY_ALLOWED`、复制后删除、截断ADS或重写OS错误迎合旧断言；不能permission denied后无条件提权重放。shutdown exact不证明过去effect未发生，不能从当前磁盘/前台/进程状态倒推未知结果。子任务工具裁剪、冻结合同、owner、设备operation lock、Activity、Effect与native cleanup围栏按真实用途保留。

交付可审查完整源码、当前架构/生成合同、检查与原生证据，明确实现/编译/原生/打包/签名各层和真实缺口。最终摘要依实际commit/push/build/native回执，未经要求不提交私有证据/凭据/缓存；使用配置Git identity，不force-push或改写共享历史。交接更新或一条成功回归不能作为整轮Windows完成声明。
