# macOS Case 处理进度

更新：2026-09-30。当前已由 macOS arm64 原生执行者接续；此前 Windows 结果仍只作共享历史引用。
规则见 [实施计划](IMPLEMENTATION-PLAN.zh.md)，公共根因引用 [共享进度](PROGRESS-SHARED.zh.md)。
覆盖 675 个共享 + 72 个 macOS 专属 Case，合计 2,366 槽。
Agent 槽数：GEN 601、COD 584、PAL 323、MM 568、CS 242、HOST 48。

## macOS 专属集合

以下集合共 72 条；其余本平台任务取目录中的 Both Case，范围包含首尾。

| 家族 | macOS 专属 ID |
| --- | --- |
| CMD | 001、003～005、012、015、017～019、021、023、025、027、030、033～034、036、038、041、043、045、047、049、051、053、055～056、058、061～062、065、067、069、071、073、076、112、120、122～123、127、132 |
| PROC/TERM/FILE | PROC-006/014/046；TERM-012；FILE-022 |
| Browser/Computer | BROW-017；COMP-008～009 |
| UI/宿主 | REAL-016～019；MAC-001～018 |

## 全领域排程

取实施计划各领域中 macOS 适用且属于对应 Agent 产品目标的槽；共享组件证据不替代本表验收。

| 领域 | 槽数 | 批次 | 原生测试、排查与修复任务 | 状态 |
| --- | ---: | --- | --- | --- |
| D01 | 425 | M02 | Session/Broker、系统代理/loopback、模型/工具 Schema 与冻结版本 | M02-01 冻结/准入与 M02-02 正式 UI/live 只读 Session 子断言通过；完整领域未验收 |
| D02 | 291 | M02/M04 | 控制/完成、原生 UI、停止/纠正及错误可见性 | M01-03/04 正式 Tauri `CMD-132/139`；M02-01/02 计划、完成与 UI 投影，M04-13 冷暂停列表及 M04-26 页外暂停证据正式 UI 子断言通过；完整 Case 未验收 |
| D03 | 145 | M01/M03 | APFS 大小写/NFC/NFD、权限、原子文件与 Artifact | M01-02、M03-01、M03-05～07 APFS/发布/ACL/xattr/immutable 子断言已验证；其余待走查 |
| D04 | 476 | M01 | /bin/sh/zsh、字面 argv、PTY/process group、Seatbelt 与退出码 | M01-01/03～05 已验 owner、`CMD-132/139`、host 映射及 login-shell 子断言；CMD 全集待验 |
| D05 | 69 | M03 | 隔离 Git remote/SSH、凭据/权限与未知结果 | M03-02/03/08～13 本地 Git/四类 hook/receipt 与隔离 loopback sshd 已验；消息 hook 的 production dependency 亦经正式 desktop build 复核；外部 host/UI 待准备 |
| D06 | 250 | M04/M05 | WKWebView、A11y/Screen Recording、MCP/Plugin/Skill | M04-01～04 已验 native CEF、Computer 权限/media/input 与 MM cold history；M04-05～08 已验 stale A11y ref，M04-09～11 已验 raw pointer、缺失绝对 launch 与用户并发输入；M04-12 修复 input cleanup 组件缺陷，M04-13～22 已验 owner crash/result-loss、Unicode、大型 A11y、Computer soak/click variants、packaged CEF 动态签名、native Browser soak/window reopen、正式 packaged Browser UI/close-to-tray；M04-23 真实 StepFun Browser/Workspace 功能链通过但限额暂停，terminal gate 保持开放；live held-cancel/IME、nested frame 与扩展待验 |
| D07 | 124 | M02/M04 | 精确伙伴/画布/知识/客服 owner 和资源 | M02-01 伙伴/画布精确绑定与 Skill 锁定向通过；正式入口 UI 待验 |
| D08 | 103 | M02/M05 | 五类 Agent 专属入口/任务；独立产物断言 | M02-01 覆盖 GEN/COD/PAL/MM 的 Session/入口子断言；原生全矩阵待走查 |
| D09 | 375 | M06 | watchdog、setsid/丢失 ownership、sleep/wake、恢复/并发/LONG | M06-01～06 已验 shutdown/写锁、start 取消/IO failure/deadline/commit failure 所有权及 macOS fork 初始化子断言；其余故障边界及 LONG 待验 |
| D10 | 33 | M01/M06 | MAC-001～018、PORT；arm64 主 lane，x86 按发布范围 | MAC-005～010/013/015/017 的本批功能断言已验；其余待走查 |
| D11 | 75 | M01/M03 | symlink、Seatbelt/ACL、旧授权和秘密隔离 | M01-01/02、M03-04～07 已验 Seatbelt/symlink/mode/旧授权/ACL/xattr/uchg；末端竞态仍待验 |
| **合计** | **2366** | M01～M06 | 平台结果独立保留 | 本 Windows 执行者不代判 PASS |

## 原生接续任务

| 任务 | 对应 Case | 测试 → 排查 → 修复安排 |
| --- | --- | --- |
| M01 路径与进程 | MAC-001～010/013/015/016；FILE-022；PROC-006/014/046；TERM-012；CMD-132/139/146～150 | 先重跑 macos process 合同；补卷属性/NFC/NFD/argv/group/Seatbelt 夹具，再做 Tauri 命令首发 |
| M02 Session 与角色核心 | D01/D02；AGEN-001；ACOD-001；APAL-001；AMUL-001 | 应用真实 owner 链路与本机代理；复验共享计划、完成、精确绑定和 Skill，最后真实模型 |
| M03 文件/Git/SSH/授权 | D03/D05/D11 剩余 | 独立工作区/remote，权限及原子性负向先行；有外部效果必须带唯一 owner 回执 |
| M04 UI/Browser/Computer/扩展 | BROW-017；COMP-008/009；REAL-016～019；MAC-011/012/017；D06 | 验原生 surface 生命周期/权限与取消，MM retry 需单独按钮/事件复现 |
| M05 条件业务 | ACSR、媒体、Channel/Robot、D07/D08 剩余 | 逐项建正式资源与安全测试账户；缺资源记阻断，能力缺项进入共享问题簇 |
| M06 生命周期与长稳 | LIFE/CONC/LONG；MAC-014/018；PORT | sleep/wake、主进程死亡、故障窗口、恢复 fence 与幂等；然后长稳统计 |

## 阶段性 P0 收尾（2026-09-29）

用户因当前算力额度要求先做阶段性 P0 收尾，余项等待后续安排。本轮按已交付证据结算，不把未跑槽
降级为通过：M01 已完成 process/APFS/argv/shell/PTY/group/Seatbelt 与正式 Tauri 关键子断言；M02
完成 Session 冻结、计划/完成、精确伙伴/Canvas/Skill 和一次正式 StepFun 新 Session；M03 完成
macOS 文件发布、ACL/xattr/immutable、Git local/remote/四类 hooks/receipt 与隔离 loopback SSH；
M04-01～03 完成 native CEF 前置、Computer 权限/media 及 TextEdit launch/input 正向。

P0 收尾保留：CEF storage timeout **1/5**、Computer optional plan discoverability 1 个公开 tool error。
M04 其余 stale/cancel/reopen/extension、全部 M05 条件业务与
M06 LIFE/CONC/LONG/soak 均明确延期；真实 Provider N3/20/99%、release signing/notarization、x86_64
也未达门槛。因此本页只表示阶段性交付点，绝不声明 2,366 槽、任一完整领域或阶段二/三全部完成。
用户后续已恢复 macOS 工作；新增批次逐项记录在下文，不倒改此处当时的 P0 结算边界。

## 已导入的 macOS 历史证据

本轮通过远端 `8228b61c3` 导入以下报告，没有在 Windows 上冒充重跑，也不将报告范围扩大到全部 Case：

- [macOS 排查记录](../../reviews/2026-09-26-macos-agent-reliability.zh.md)：本地 HTTP/SSE provider 的真实 owner 链路，
  进程 `macos_` 四项、系统代理、应用暂停恢复等；这些是组件/产品机制证据，非原模型任务全量验收。
- [公共工具与完成协议记录](../../reviews/2026-09-26-agent-tool-contract-reliability.zh.md)：最终构建 `81227bc9d6aa`，
  StepFun 贪吃蛇完成但有 1 次参数拒绝，Agnes 五子棋完成但有 3 次拒绝；另有独立浏览器产物检查。
  中间失败均保留，不能写成 first-attempt 零失败或 99% 可靠性。
- 报告所列 `/tmp/nomifun-agent-followup-*` 是原 Mac 证据位置，本 Windows 未逐文件核验原始证据。
  下一位 macOS 执行者先校验这些记录和当前提交，再给具体 Case 绑定结果。
- 当前新合并构建的原生验收待 macOS runner；依赖主机的任务标为阻断/待运行，不影响 Windows 独立推进。

完整新证据默认存仓库外 `~/code/temp/nomifun-agent-reliability/phase-2-3/<date>/macos/<batch>/<run>/`。
Git 只更新本页的批次结论与必要代码/测试，不提交完整日志或展开索引。

## 本轮原生批次

- **M01-01 process owner / Seatbelt**（PROC-006/011/012/014/016/033～041/046、TERM-012，
  MAC-005～010/015 的本批子断言）：宿主为 macOS 26.6.2 / Darwin 25.6.0、原生 arm64、
  `sysctl.proc_translated=0`，工作区位于 APFS Data 卷。首次反例证明正式
  `ManagedEngineProcessOwner` 仍用 `UnrestrictedLocalOwner`，可写绑定工作区 sibling；另保留
  Seatbelt `TMPDIR` 拒绝被误报清理未知、无执行位目标经 wrapper 过早返回活动 Session，以及
  多个临时 Tokio runtime 并跑 process/PTY 夹具时的 `PeerClosed` 首败。
- 修复后 macOS 产品 process owner 默认使用绑定 workspace 的 `MacSeatbelt` 写根；命令和环境
  preflight 在 watchdog 创建前完成，显式 Unix executable 先核对 X_OK，bare PATH 名仍保留真实
  exec/ABORT 覆盖；确定 pre-spawn 拒绝保留 `user_code_not_started`。测试夹具按单产品 runtime
  隔离，同时保留单 runtime 内 16 路 shell + 长驻 peer、8 路 PTY 并发，不以串行删除并发覆盖。
- 验证：`nomi-process-runtime --tests` **243/243**；process contract **16 项 × 20 轮**、PTY
  contract **12 项 × 20 轮**、独立 8 路 PTY 并发 **20/20**；Engine **32 通过 / 1 ignored**；
  正式 Session→Runtime→Kernel→owner 的本地 provider process 场景 **1/1**；process boundary、
  定向 fmt 与 diff 检查通过。证据：`2026-09-28/macos/m01-process-contract/`。
- 未覆盖：APFS 大小写敏感性与 NFC/NFD、文件 owner symlink/权限/原子写、quarantine 的正式产品
  失败展示、应用内 login-shell Terminal、正式 Tauri 会话区 `CMD-132/139` 首发、x86_64 lane；
  因此不关闭完整 M01、TERM-012、MAC-001～010 或任何 REAL Case。

- **M01-02 APFS identity / file permission**（MAC-001～005、FILE-022/024/031/032/036 的本批
  子断言）：主 Data 卷原生探测确认 APFS 大小写不敏感，ASCII case 变体及 NFC/NFD 名称分别指向
  同一 inode；另创建并挂载 64 MiB Case-sensitive APFS 镜像，确认 case 变体为两个 inode、NFC/NFD
  仍为同一 inode。首次反例中 case 与 normalization 双目标都在第 1 项发布后才失败
  （`published=[0]`、`retained_created=[0]`）；mode 0444 目标又因父目录可写而被 rename 成功替换。
- 修复按卷 `_PC_CASE_SENSITIVE` 构造 patch component identity：macOS 始终做 canonical Unicode
  归一化，仅在大小写不敏感卷做 case fold；等价/祖先目标在任何目录/临时文件创建前拒绝。
  Unix 既有目标在 rename 前打开并持有经 dev/inode 核对的可写句柄，mode/ACL 拒绝不再被父目录
  rename 权限绕过。根内 symlink 正向和根外、dangling、final、`.nomifun` alias 负向沿用同一 owner。
- 验证：主卷 4 项与 opt-in Case-sensitive APFS 1 项各 **20/20**；`nomifun-file --tests`
  **464/464**，另 1 个 opt-in ignored 已显式运行；正式 Session→Runtime→Kernel→file/process owner
  场景 **1/1**。测试卷 `/dev/disk5` 已精确卸载，64 MiB 镜像保留。证据：
  `2026-09-28/macos/m01-apfs-paths/`。
- 未覆盖：更多 Unicode case-fold 特殊组、ACL 动态撤权、immutable flags、目录/目标在最后系统调用
  前的剩余竞态、磁盘/IO fault、正式 Tauri UI 与 100 seed；因此只关闭本批子断言，不关闭完整
  FILE/MAC Case 或 M01。

- **M01-03 formal Tauri command-first**（`CMD-132` 及 `CMD-146～148` 的 macOS 子断言，
  `TERM-012` 组件子断言）：使用 arm64 `NomiFun Dev.app`、`com.nomifun.desktop.dev`、
  隔离 data/workspace 与已有加密 StepFun Plan / `step-3.7-flash` 配置。两个新 Session 的首次
  反例均原样保留且未点击重试：首个用 `cmd:"ls -a"`，且成功命令被误判为必须交付
  文件；首轮修复后已改为字面 `command`/`args`，但 planner 的“在输出中…业务文件”变体
  仍触发同一 Artifact 误判。
- 共享修复将普通单 executable 的 `command` + 字面 `args` 设为模型 Schema 首选，`cmd`
  只用于 pipeline/redirection/glob/compound 等真正 shell 语义。Artifact 推断现区分
  `完整/原始/命令…输出` 及 `在/从/于输出中/里/内` 的观测名词，并在动词序列中移除该 token；
  `输出一个文件`、`保存命令输出为文件` 等真交付仍 fail closed。
- 修复后以第 3 个新 Session 重新首发（不重试旧失败）：Execution/Step/Attempt 均
  `completed`，`delegation_policy=disabled`，2 个模型 step，恰好 1 个 process effect，调用为
  `{"command":"ls","args":["-a"]}`；退出 0、`reaped=true`、无 tool error，原始输出精确为
  `.`、`..`、`.hidden-case`、`inside-link`、`visible.txt`、`中文 space.txt`，UI 绿色 1/1 且
  `visible_failure_count=0`。symlink 仅列名未跟随，fixture hash/链接目标不变，应用退出后端口/进程清零。
- 确定性验证：Agent execution **112/112**、同步远端后 Agent runtime **173/173**、Artifact 定向
  **15/15**；Terminal lib + parent-death **148/148**，两项 process-group 关键用例各 **20/20**；
  合并上游 host-shell/UI 修复后的精确 `.app` 仍 build/start 通过，arm64 二进制
  `fff3fa60359e…`，退出后进程/端口为 0。为遵守冻结的 3 个新 Session 预算，同步后未创建第 4 个
  付费模型 Session；最终真实模型轨迹来自紧邻同步前修复版，同步组合以全组回归和正式启动覆盖。
  证据：`2026-09-28/macos/m01-tauri-command/`。未覆盖
  `CMD-134/139/149/150`、应用内 login-shell Terminal、quarantine/App Translocation 及 x86_64 lane；
  因此不关闭完整 M01、`TERM-012`、`MAC-013`、`REAL-016/017` 或 CMD 家族。

- **M01-04 formal Tauri managed process lifecycle**（`CMD-139`，`PROC-014/034～037/046`、
  `REAL-017`、`TERM-012` 的本批 process 子断言）：宿主为 Darwin 25.6.0 / arm64、APFS，使用
  正式 `NomiFun Dev.app`、隔离 workspace 与已有加密 StepFun Coding Plan / `step-3.7-flash`。
  八个未重试旧 Session 的首次失败分别保留在 run-001/005/008/011/013/015/021/023：包括把同一
  handle 拆成四个 Attempt、用 `/tmp`/raw PID/shell kill 替代 owner、`wait_ms=1000` 后跳过 poll、
  instruction/completion 参数拒绝、取消终态误算失败、完成报告空 `requirement_ids`，以及模型先做
  `read_file`/字面 `ls -la` 探查后恢复；最终完成均不能覆盖这些 `FAIL_RECOVERED`。
- 修复后 planner 对明显的托管生命周期只持久化一个 Agent step，并把完整
  `start_process + poll_process + cancel/close` spec 标记为 `managed_process_only`。该标记只减权：
  Attempt 仅物化 start/poll/input/close/resize/cancel，不能看到 File、`exec_command`、VCS、Artifact、
  discovery 或 delegation；未增加 Snapshot Action 或权限。Runtime/Kernel 统一把
  `state=cancelled + cleanup.reaped=true` 的 cancel/terminal poll 作为成功控制观察，不计失败命令；
  同一 process chain 的精确 start/poll/cancel receipts 可在相同 workspace epoch 完成引用。
  无已发布 requirement ID 时，完成 Schema 不再暴露该字段；有 ID 时只允许精确枚举，严格数组与
  非空断言未放宽。
- 最终 run-025 新 Session 首发通过：Execution/Step/Attempt 均 `completed`，单 step、4 个模型 step，
  持久 profile 为 `managed_process_only=true`；工具序列精确为一次 start、一次 poll、一次 cancel，
  同一 `process_id`，无 read/search/exec/VCS/Artifact 调用，无 tool/control error，两个 managed effect
  恰为 start/cancel。poll 观察 `READY pid=88007 child=88008`，cancel 返回 `STOPPED signal=INT` 与
  `cleanup.reaped=true`；父子 PID 均消失、helper digest 不变。UI 绿色 **1/1**、
  `visible_failure_count=0`，应用退出后相关 listener/process 为 0。
- 确定性验证：Agent execution **114/114**、Agent runtime **184/184**、API types **517/517**、
  Engine **32 通过 / 1 ignored**（既有 Bun PATH live 检查）；Plugin/Kernel 减权、历史取消投影定向
  回归通过，process boundary、Agent vocabulary、fmt 与 diff 检查通过；正式 arm64 `.app`/DMG
  build 成功。证据：`2026-09-29/macos/m01-process-lifecycle/`。未覆盖真正 PTY 的 stdin/close/resize、
  应用内 login-shell Terminal、`CMD-149/150`、quarantine/App Translocation、x86_64 及 20 次/N3；
  因此只关闭 `CMD-139` 本批单样本和相邻子断言，不关闭完整 `REAL-017`、`TERM-012` 或 M01。

- **M01-05 host OS mapping / native login shell**（`CMD-134/149/150`、`TERM-012`、`MAC-013`
  的本批子断言）：静态走查确认 process 文案取 Runtime 二进制的 `std::env::consts::OS`，不读取
  renderer/client OS，但旧 Kernel 没有核对文案所声明 OS 与执行 owner，故无法满足错配时的
  `HOST_OS_COMMAND_MAPPING_ERROR`。另一个首次失败来自正式 Terminal Shell preset：UI 持久化
  `command=$SHELL,args=[]`，后端展开 `/bin/zsh` 后仍保留空 argv，实际不是 login shell；新增失败
  回归精确得到 `left=None,right=Some("-l")`。中间原生 PTY 回归先用 `$0` 判断 login，因 zsh `-l`
  仍保留 `$0=/bin/zsh` 而失败，已保留并改为检查真实 `$options[login]`，未放宽产品断言。
- 修复后 `exec_command` 与 `start_process` 同时声明实际 Runtime process host OS；已通过完整
  Snapshot/binding 校验的调用在 owner dispatch 前核对该声明，错配返回
  `status=not_executed`、`user_code_started=false`、`HOST_OS_COMMAND_MAPPING_ERROR`，明确禁止盲试另一
  平台命令。Shell sentinel 在 Unix 且产品默认空 argv 时补 `-l`，显式 caller argv 原样保留。
- 正式 Tauri host 子断言使用“客户端声称 Windows”作为不可信输入数据，模型仍按 macOS Runtime
  host 首发 `command=pwd,args=["-P"]`；单 step、2 模型 step、1 effect、exit 0/reaped、零其他工具/
  错误，输出精确等于 Session workspace，UI 绿色 1/1。真实客户端仍是 macOS，因此只算
  `CMD-149` 子断言；错配负向由确定性门禁验证，未伪造正式 owner。
- 正式 Tauri Terminal 从“新建终端”选择隔离目录并直接启动 `$SHELL`：DB 保持 `$SHELL`/`[]`，
  原生 PTY 为 `/bin/zsh -l`，UI/scrollback 返回 `NOMIFUN_UI_LOGIN=on SHELL=/bin/zsh`。同一 Session
  relaunch 后确认 leader `45148` 与独立 job group `45373` 活跃，再用侧栏“关闭”；约 151 ms 后两
  PID 均消失，row 保留 `exited`/`exit_code=null`。随后再次 relaunch 的 login probe 正常 `exit 0`。
  含双引号 `$!` 的 `dquote>` 已在原生交互 zsh 独立复现为 shell/BangHist 语义，产品传输字节一致，
  未以吞错修复。
- 验证：Terminal **149/149**；login + job-group 两项各 **20/20**；同步远端后 Runtime **187/187**；Engine
  **33 通过 / 1 ignored**；host mismatch、AppTranslocation classifier 定向回归、process boundary、
  Agent vocabulary、fmt/diff 及正式 arm64 `.app`/DMG build 通过。当前构建无
  `com.apple.quarantine`，未实际触发 Gatekeeper/App Translocation；x86_64 仍未运行。证据：
  `2026-09-29/macos/m01-host-os/`、`m01-terminal-login/`、`m01-mac013/`。因此不关闭完整
  `CMD-149/150`、`MAC-013/016` 或 20 次正式 UI 门槛；`TERM-012` 仅关闭本批原生主 lane 子断言。

- **M02-01 Session freeze / plan-completion / official binding / Skill lock**（`AGEN-001`、`ACOD-001`、
  `APAL-001`、`AMUL-001` 及 `EXT-006/012` 的本批子断言）：同步到 `a3a2b4f96` 后在 Darwin
  25.6.0 / arm64 原生 runner 定向验证。新 Session 创建/初始 Turn/Checkpoint 准入保持原子、幂等且
  冻结 Snapshot 与资源 owner；同一官方配置可创建不同 Session，但 binding 不漂移。可选计划保留
  原始义务并正确区分幂等重放/真实 replan；`report_completion` 只接受当前证据，关闭计划后不能被
  模型重开，且累计 tool error 不能被抹除。
- 伙伴入口固定 `companion.default` 与确切 Companion/Memory/Scheduler；创作入口固定
  `creative-studio.default` 与确切 Canvas/Asset Library。已选 Skill 的正文/digest/来源进入冻结
  Snapshot 与 system context；未选 Skill 在 provider 调用前拒绝，恢复时必须按冻结内容重水化；
  stale/missing 产品选择不能让新 Canvas Session 继承错误 target。未发现需改产品代码的新根因。
- 验证：Agent Runtime 计划 **8/8**、完成账本 **7/7**、Turn/Session/Skill **6/6**；Agent Session
  冻结/准入 **5/5**；Control Plane Skill/Revision 锁 **4/4**；App canonical Session、模型持久化、
  官方 Agent 复用及伙伴/画布入口 **9/9**。同步远端 `93b5f58e9` 后，新增 command-failure 完成计数
  3 项与 active-plan repair 1 项亦通过，当前合计 **40/40**、0 failed、0 ignored。完整日志：
  `2026-09-29/macos/m02-session-core/run-002-deterministic-post-sync/`，首次伙伴/画布三项记录另保留于
  `m02-bindings-skills/run-001-deterministic/`；合并后增量见 `m02-session-core/run-003-post-remote-command-failure/`。
- 正式 Tauri 隔离夹具的两次预检分别因 dataset work-root receipt 与 work-root owner receipt 不一致而
  fail closed，未创建 Session、未消耗模型预算；未放宽保护。成对复制 data/work 身份后的 run-005
  启动、数据库完整性与端口清理通过，但当时 macOS 锁屏阻断 UI 输入/截图，已停止应用；该缺口由
  下述 M02-02 回补。M02-01 本身不关闭完整 `AGEN/ACOD/APAL/AMUL/EXT` Case。

- **M02-02 formal Tauri new Session / live StepFun**（`AGEN-001`、`ACOD-001`、D01/D02 的本批
  只读子断言）：以当前正式 `NomiFun Dev.app`、原生 arm64、成对隔离 data/work 身份与独立 workspace
  执行 StepFun Coding Plan / `step-3.7-flash`。应用二进制 SHA-256 为 `2d2863ddacca…`；Session 使用
  `assistant.general` /「通用」、精确 workspace binding，协作关闭。run-006 首发工具读取正确得到
  `M02-SESSION-CORE-第一行`，但“逐字 / character by character”被模型解释为字符间插空格，最终
  canonical/UI 文本为 `M 0 2 - S E S S I O N - C O R E - 第 一 行`；虽 UI 绿色 1/1、零工具错误和
  零副作用，业务结果不精确，记 **FAIL_RECOVERED**，原 Session 未重试且数据库/事件/推理轨迹保留。
- 修正仅收紧验收输入语义，不改产品代码或放宽断言：run-007 使用全新 data/work、Session、Execution、
  Attempt 与哨兵，明确要求回复字节级等于第一行、禁止空格/包装。新 Session 首发通过：外层 1 个计划、
  1 个 read-only step、1 个 Attempt、1 个 delivered completion；2 个模型 step，恰好 1 个模型发起的
  `read_file`，另有 3 个 step-0 instruction read 被独立计数；零 process/mutation 调用、零 effect、
  零 tool error/command failure。工具结果、Attempt/Execution summary、canonical message 与 UI 均精确为
  `M02-SESSION-CORE-RUN07-原样`，UI 绿色 **1/1**；fixture digest 前后不变，退出后 6 个 listener、
  相关进程为 0，数据库完整性 `ok`。
- 确定性 M02-01 的显式 `report_completion` 账本回归与本批正式 Session 的外层 plan/lead-report 分层
  记录，不能互相替代。完整证据：`2026-09-29/macos/m02-session-core/run-006-formal-ui/` 与
  `run-007-formal-ui/`。未覆盖 live N3/20 次门槛及完整 Agent/EXT 家族；开发 bundle 缺 CEF 只阻断
  不相关 Browser 分片，因此不关闭完整 D01/D02 或 M02。

- **M03-01 macOS Unix publication / cleanup race**（`FILE-019/020/028/031/038/039` 及
  A05/A07/A13/A17/A19 的本批子断言）：在 Darwin 25.6.0、原生 arm64、APFS Data 卷上复核共享
  S-D03-45/46/50。首次 6 个精确竞态均通过：macOS 新建发布由
  `renamex_np(RENAME_EXCL)` 原子消费自有 stage，未进入独立的 check→unlink 窗口；同字节或已修改的
  外来 staging 名均被保留。既有目标发布在核对后发生源换名时返回
  `FILE_WRITE_OUTCOME_UNKNOWN`，不生成 publication identity，并保留
  `temporary_cleanup_unconfirmed=true`；发布前目标消失时同样保留自有 stage 供显式对账。
- 三条核心原生边界各重复 **20/20**：60 份保留 observation 精确分为原子新建 20、替换 unknown 20、
  失败 stage 保留 20，额外 cleanup window 为 0。上层 uncertainty/error projection、缓存撤销及
  same-bytes foreign rollback 隔离另 **4/4**。未发现需改产品代码的新根因；完整日志和现场位于
  `2026-09-29/macos/m03-unix-publication/`。
- 未覆盖真实磁盘满/IO fault、immutable flags、准入后的动态 ACL 撤权与最后系统调用竞态、正式
  Tauri/model/UI 文件变更投影、其他 Unix fallback、100 seed/LONG 门槛；因此不关闭完整 FILE、
  AUTH、D03 或 M03。

- **M03-02 macOS VCS literal path / index isolation**（`VCS-004/005` 及 A05/A14/A17 的本批
  子断言）：在原生 arm64 / APFS 的独立临时 repo 中复核共享 S-D05-01。首次 **4/4**：stage、
  unstage、discard、reset 对 `[]`、`!`、`*`、`?` 与 Unix 反斜杠文件名均按字面处理，不改相似邻居；
  不存在的字面路径和目录请求保持 index/worktree digest；反斜杠命名 executable 恢复原字节与执行位，
  symlink 恢复链接自身和 target text，不修改目标文件；冲突/index metadata 未被单文件操作抹除。
- 综合字面路径操作与 executable/symlink mode 恢复分别 **20/20**，隔离 TMP fixture 最终为 0；
  未访问网络 remote、SSH host 或凭据，未发现需改产品代码的新根因。证据：
  `2026-09-29/macos/m03-vcs-literal/`。
- 未覆盖非 UTF-8 Git path、Git index 的 macOS case-fold alias、并发父目录置换、submodule/worktree、
  commit/hook/identity、local/file remote push、non-fast-forward/unknown result、真实 SSH 与正式 UI；
  因此不关闭完整 VCS、SSH、D05 或 M03。

- **M03-03 macOS local Git remote / effect receipt**（`VCS-001/003/004/006/009～014`、
  `AUTH-004/013` 的本批子断言）：正式 owner 的首次 **12/12** 定向通过。status/diff/stage 保持在
  绑定 repo；commit 只消费 staged 内容，同 idempotency key 重放原 commit receipt。真实本地 bare
  remote 的显式 branch refspec 只更新一次，重放返回原 effect receipt；push 等待并发 commit gate 后
  读取新 HEAD，同一 index state 的并发 commit 只有一个生效。
- 真实 non-fast-forward 保持 local/remote 历史不变；force、缺 credential authority 的网络 remote、
  credential-handle 替代和请求 secret 字段均在 remote contact 前拒绝。local push+receipt、non-FF 拒绝、
  commit→push 串行化各 **20/20**，共 60 次，隔离临时 repo 最终为 0；未连接网络或使用真实凭据。
  证据：`2026-09-29/macos/m03-vcs-local-remote/`。未发现需改产品代码的新根因。
- 未覆盖 identity 缺失、hook 拒绝、commit/push 后 result-loss 注入、外部本地编辑并发、
  submodule/worktree、HTTPS/SSH transport、host key/SFTP/远端 shell、正式 UI 授权旅程及长期门槛；
  因此不关闭完整 VCS、SSH、AUTH、D05 或 M03。

- **M03-04 authorization lease / stale Snapshot / exact resource**（`AUTH-001/002/004～008/014/015`
  的本批子断言）：macOS 原生 runner 上 Common scoped authority **8/8**、Agent Kernel
  authority/preflight **8/8**。expired/future/cross-Session claim、错误 Principal/owner/resource/action、
  active-set drift 及 stale dependency graph 均在 owner dispatch 前 fail closed；explicit revoke 从任意
  clone 立即撤销共享 lease，replacement issuance 撤销同 Session 旧 lease，root rotation 后不能续期。
  child/dependency 关系不隐式扩权，冻结 Snapshot 不受无关 registry publication 漂移且拒绝 foreign ID。
- revoke、replacement、cross-Session rejection 与 owner/action/resource/active-set drift preflight 四条
  竞态边界各 **20/20**；撤权/重发及 replacement lease 均不能重置仍存活的共享 request budget。
  未发现需改产品代码的新根因。证据：`2026-09-29/macos/m03-auth-revocation/`。
- 未覆盖原生文件 ACL 在准入后/最终系统调用前动态变化、TCC/Accessibility/Screen Recording、真实
  secret-bearing owner、应用重启后持久旧 grant、正式 UI 恢复提示及高并发 soak；因此不关闭完整
  AUTH、D11 或 M03。

- **M03-05 macOS extended ACL preservation / dynamic revoke boundary**（`MAC-005`、
  `FILE-020/022/025/038/039`、`AUTH-006/014` 的本批子断言）：原生探针和首次产品回归证明 APFS 上
  普通 `rename` 替换 inode 会丢目标 extended ACL；成功 write/patch 均把
  `group:everyone deny execute` 静默移除。修复在目标 dev/inode、stage identity/bytes 核对后及最终
  pre-publication hook 之后，通过已持有 descriptor 执行 `fcopyfile(COPYFILE_ACL)` 并 `sync_all`；
  ACL 复制失败在 rename 前停止，不吞错或扩大权限，Linux/Windows 路径未改。
- 动态夹具在最终 hook 新增 `group:staff deny execute`，发布后新旧 ACL 均保留；deny-write ACL 下
  write/patch 保持旧字节与 ACL，并按共享 Unix 安全清理合同保留两个自有 stage、返回 outcome unknown、
  发送两个无内容对账事件。全组同时保留旧 mode 0444 结果并把过期“立即删 stage”断言收紧为精确
  retained-stage/reconciliation 断言。首次 ACL 丢失、旧断言失败及中间诊断均独立保留。
- 最终 ACL 保留、deny-write、动态 hook 各 **20/20**；相邻 Unix 发布 **4/4**；`nomifun-file` lib
  **308/308**，macOS workspace **6 passed / 1 ignored**，fmt/diff 通过。证据：
  `2026-09-29/macos/m03-macos-acl/`。未覆盖 ACL copy 后至 rename 的 syscall-sized 竞态、xattr/
  resource fork/immutable flags、非 APFS 卷、IO fault、正式 UI 变更投影及长期门槛；因此不关闭
  完整 FILE/AUTH、D03/D11 或 M03。

- **M03-06 macOS xattr / resource-fork preservation**（`FILE-020/025/038/039`、D11 的本批子断言）：
  首个 `xattr -p` oracle 因 CLI 展示换行失败并保留；校正后真实首败确认成功 write/patch 会删除
  `com.nomifun.reliability.fixture`。macOS descriptor-bound metadata copy 扩展为
  `COPYFILE_ACL | COPYFILE_XATTR`，仍在最终 hook 后、rename 前并 `sync_all`；失败在名称改变前停止，
  不把任何真实用户 xattr 值写入日志，其他平台路径不变。
- 普通 xattr 与 `com.apple.ResourceFork` 的 write/patch 保留 **20/20**；final hook 新增的 before/after
  xattr **20/20**，20 份 observation 独立保留。最终 `nomifun-file` lib **309/309**、macOS workspace
  **7 passed / 1 ignored**，ACL 保留/拒绝回归与 fmt/diff 同时通过。证据：
  `2026-09-29/macos/m03-macos-xattr/`。
- 未覆盖 metadata copy 后至 rename 的最后窗口、copyfile 故障注入、超大 resource fork、immutable
  flags、非 APFS 卷、正式 UI 变更投影、100 seed/LONG；因此不关闭完整 FILE、D03/D11 或 M03。

- **M03-07 macOS immutable target refusal**（`MAC-005`、`FILE-020/022/025/038/039` 的本批
  子断言）：首次夹具把 `chflags` 写成不存在的 `/bin/chflags`，在产品调用前失败并保留；改用本机
  权威 `/usr/bin/chflags` 后，`uchg` 目标的 write/patch 首次即拒绝，旧字节与 `uchg` 均保持到测试
  明确恢复 flag。两个自有 stage 各保留拟写字节并返回 outcome unknown，两个无内容事件要求对账，
  未把拟写字节投影成成功或尝试提权/移除系统保护。
- immutable 精确回归 **20/20**；最终 macOS workspace **8 passed / 1 ignored**，fmt/diff 通过。
  证据：`2026-09-29/macos/m03-macos-immutable/`。未覆盖需提权的 `schg`（不设置、不绕过）、最终窗口
  内 flag 变化、成功替换时非阻断 flags、非 APFS、正式 UI 与长期门槛；因此不关闭完整 FILE、
  D03/D11 或 M03。

- **M03-08 macOS Git identity / pre-commit hook / uncertain fence**（`VCS-006/008`，以及
  `VCS-009/014`、`AUTH-009/013` 的本批不确定结果、精确范围和脱敏子断言）：首次产品反例中，绑定
  repo 的可执行 `pre-commit` 明确 `exit 7`，现有 libgit2 commit 仍创建提交，确认 hook 被绕过；首次
  Seatbelt 夹具又误把 sibling 放进系统信任的 `TMPDIR`，两次失败均独立保留。身份缺失路径原已拒绝，
  本批把签名核对提前到 tree 写入之前并保留 HEAD/index。
- 修复后 macOS 仅在 default/`core.hooksPath` 存在可执行 `pre-commit` 时，先核对绑定 repo、staged
  scope 与 identity，再以字面 `/usr/bin/git hook run --ignore-missing pre-commit` argv 进入共享
  `ProcessSupervisor`，使用 30 秒 deadline、完整进程树回收及只允许精确 repo 写入的 Seatbelt；hook
  返回后 commit 路径重新核对 index/scope。嵌套 workspace 不执行根 repo hook；诊断有界且 secret
  脱敏。拒绝、启动失败或超时按 `ExternalUncertainEffect` 写入终态 unknown，唯一资源 fence 同时阻止
  同 key 和新 key 盲重放；无可执行 hook 的 repo 保留不依赖 Git CLI 的既有 libgit2 路径。
- 拒绝 hook、脱敏及持久 unknown/replay fence **20/20**；成功 hook 的非 TMP sibling 越界写被
  Seatbelt 拒绝 **20/20**；无效 identity 保持 HEAD/index **20/20**；相邻 VCS host **15/15**、监督
  shell/direct-program **3/3**，字面元字符未执行，fmt/diff 通过。证据：
  `2026-09-29/macos/m03-vcs-hooks/`。未覆盖 `prepare-commit-msg`/`commit-msg`/`post-commit`、自定义
  stdin、timeout/descendant/HEAD-mutation 故障注入、commit 成功后 result-loss 对账、Windows/Linux
  hook、正式 Tauri/model commit 旅程及长期门槛；因此不关闭完整 VCS/AUTH、D05 或 M03。

- **M03-09 macOS committed receipt / response-loss replay**（`VCS-009/014` 的本批持久 receipt、
  HEAD/tree 与本地编辑隔离子断言）：在真实成功 `pre-commit` 后完成 commit，并故意丢弃返回值；独立
  确认 canonical effect 已为 `returned` 后销毁 host，再写入一份用户 worktree 修改并以同一 Session
  Store 重建。相同 idempotency key 只重放原 receipt，hook marker 仍为一个字节；独立 Git 核对的
  HEAD OID、tree OID、commit count、已提交 blob 与 terminal event identity 均不变，后写的用户修改
  也未被覆盖。首次注入即通过，未发现需改产品代码的新根因。
- 精确 response-loss/restart 回归 **20/20**，相邻 VCS host **16/16**，fmt/diff 通过；证据：
  `2026-09-29/macos/m03-vcs-result-loss/`。本批只覆盖 terminal effect receipt 已持久后的响应丢失；
  physical commit 已成功但 terminal receipt 尚未写入的窄窗口仍只保留 pending fence，尚无自动
  commit-intent 对账。push result-loss、其余 hook、正式 Tauri/model UI、非 macOS 与长期门槛也未覆盖，
  因此不关闭完整 `VCS-009/014`、D05 或 M03。

- **M03-10 macOS prepare-commit-msg / commit-msg**（`VCS-006/008/014`、`AUTH-009/013` 的
  消息 hook 顺序、拒绝、边界与脱敏子断言）：首次反例只配置 `commit-msg` 并 `exit 9`，旧路径仍直接
  创建 commit，hook marker 不存在，确认 libgit2 同样绕过消息 hook。修复将 macOS hook 计划扩为
  `pre-commit → prepare-commit-msg → commit-msg`，支持 default 与 `core.hooksPath`；全部通过字面
  `/usr/bin/git hook run` argv、共享 supervisor、每 hook 30 秒 deadline、完整进程树回收和精确 repo
  Seatbelt 写根执行。
- 消息 hook 使用 Git metadata 内唯一临时文件；host 持有 descriptor 并核对 dev/inode，拒绝替换或
  symlink 后的读取，限制 64 KiB、UTF-8 与产品 512 字符消息边界。hook 前后重新核对 identity、HEAD
  parent 与 staged path 集合；成功 hook 若改变 HEAD/path membership，或其后 commit 步骤失败，一律
  持久化 external-unknown 并保留资源 fence。诊断与后续失败原因有界脱敏，不吞掉原错误类别。
- `commit-msg` 拒绝/脱敏、相对 `core.hooksPath` 下两类消息 hook 顺序与改写、消息文件 identity 拒绝、
  成功 hook 后 index/path 漂移四条各 **20/20**；相邻 VCS host **20/20**，fmt/diff 通过。首次产品
  失败与一次错误 test-binary 路径的夹具失败均保留；证据：
  `2026-09-29/macos/m03-vcs-message-hooks/`。未覆盖 `post-commit` 通知、discovery→exec hook 文件置换、
  timeout/descendant fault、linked worktree/common Git-dir、非 macOS、正式 UI 与长期门槛；因此不关闭
  完整 VCS/AUTH、D05 或 M03。

- **M03-11 macOS post-commit notification**（`VCS-006/008/009/014`、`AUTH-013` 的本批提交后
  通知、失败可见性、HEAD observation 与幂等子断言）：首次反例中 libgit2 commit 已成功，但真实
  可执行 `post-commit` 完全未运行，marker 与 hook 内 HEAD 证据均不存在。修复在 commit ID 创建后才
  以字面 `/usr/bin/git hook run --ignore-missing post-commit` 执行，复用 default/`core.hooksPath`、
  supervisor、30 秒 deadline、完整进程树回收和精确 repo Seatbelt。
- `post-commit` 非零退出不把已经发生的 commit 伪报成失败；成功 effect receipt 内新增有界脱敏的
  hook status/exit code、`retry_allowed=false` 及独立 hook 后 HEAD 核对。成功 hook 若改写 HEAD，
  receipt 明确 `head_matches_commit=false`、`requires_reconciliation=true`；同 key 只重放该 receipt，
  不再次执行 commit 或通知 hook。非零通知失败与 HEAD 改写两条各 **20/20**，相邻 VCS host
  **22/22**，fmt/diff 通过；证据：`2026-09-29/macos/m03-vcs-post-commit/`。
- 未覆盖 physical commit 或 post hook 已发生但 terminal receipt 尚未落库的崩溃窗口、hook
  discovery→exec 置换、timeout/escaped descendant fault、linked worktree/common Git-dir、非 macOS、
  正式 UI 与长期门槛；因此不关闭完整 VCS/AUTH、D05 或 M03。

- **M03-12 macOS native SSH / BSD grep fallback**（`SSH-001～012` 的本批 transport/owner 子断言，
  其中 connect/auth/host-key/SFTP/shell/sudo/reconnect/cancel/concurrency 为真实 loopback sshd）：本机
  `/usr/sbin/sshd` 夹具使用随机高位端口、每测独立 host/client key、authorized_keys/known_hosts、内存
  host book 与加密凭据，不读写 `~/.ssh` 或生产 host。首次 backend 全组在 39/41 时稳定出现两项
  macOS BSD grep 失败：单文件 fallback 带出 `./-:`、`a'b:` 文件名前缀，破坏既有 `line:content`
  结果合同。
- 修复在无 `rg` 时按远端实际类型选 argv：目录继续 `grep -rnE` 保留递归路径，单文件改为
  `grep -nEh`，在 BSD/GNU 都抑制单文件名前缀；扩展 regex、leading dash、shell quote、no-match=1、
  invalid-regex status、timeout 与“不换另一引擎重试”均保持。另加真实 sshd 的 `-`、`a'b` 与递归目录
  旅程。首个 live 断言误把本地 raw stdout 的尾换行用于持久 RemoteShell，既有合同正确去掉一个尾
  换行；该夹具失败独立保留，断言按正式 shell 形状校正，内容/路径要求未放宽。
- shared `nomi-ssh` **83/83**、backend `nomifun-ssh` **96/96**（含真实 pool lifecycle **19/19**），
  全部零 SKIP；三条本地 grep 形状各 **20/20**，真实 sshd grep **20/20**。最终远端 fixture、sshd
  进程与 listener 均为 0；首次失败遗留的一个精确 `/private/tmp/nomifun-ssh-grep-*` 目录检查后移入
  macOS Trash，可恢复。fmt/diff 通过；证据：`2026-09-29/macos/m03-ssh-native/`。
- 未覆盖独立管理的非 loopback host、正式 Tauri/model/UI 授权旅程、真实应用进程重启下的 remote
  in-flight、远端 write 发布后断线的独立 digest 对账、escaped remote descendant 及 LONG/flood 门槛；
  因此不关闭完整 SSH、D05/D11 或 M03。

- **M03-13 message-hook production dependency gate**（`VCS-006/008` 的正式产品构建子断言）：进入
  M04 原生 CEF smoke 的首次 current-source desktop example build 在启动前失败；M03-10 使用的
  `tempfile::NamedTempFile` 只存在于 `nomifun-app` dev-dependencies，故 unit tests 可编译而 production
  dependency build 无法解析 crate。完整首败保留于 `2026-09-29/macos/m04-cef-native/run-001/`。
- 将同一 workspace-pinned `tempfile` 从 dev-only 移至 normal dependency，不新增版本或 lock 漂移，
  保留原有安全临时文件实现。随后正式 `cargo build -p nomifun-desktop --example browser_cef_smoke
  --no-default-features` 通过，相邻 VCS host **22/22**、fmt/diff 通过；证据：同批
  `run-002-prod-dependency-fix/`。该结果只修复产品构建门槛，CEF native conformance 尚未在本批执行，
  不关闭 BROW/M04。

- **M04-01 native CEF/Tauri conformance prerequisite**（`BROW-017`、`MAC-017` 的本批 native
  child-surface/lifecycle 前置子断言）：current source 构建 arm64 Tauri example、固定 CEF 152.0.6 /
  Chromium 152.0.7977.83 与五类独立 helper，并以 fresh bundle/profile 运行。修复 M03-13 的 production
  dependency 后，首个真实 native run 已到 `renderer_crash_settled`，随后 storage context A 的
  `Page.navigate` 30 秒 settlement timeout；结果 `passed=false`、`shutdown_error=null`，main/helper 均
  清零，首败独立保留，未放宽 timeout/assertion。
- 后续四个独立 bundle/profile 连续 **34/34**、`shutdown_complete=true`：覆盖 native child、trusted
  Unicode/click/drag/wheel、input gate、dialog、frame input/geometry/upload、stale chooser、file picker
  cancel、download/cancel、permission deny、semantic observation/viewport capture、RunGuard stop、popup/profile、
  download publication、renderer crash、Conversation storage isolation/tab recreation/clear 与跨 context 保留。
  最终 owned process/listener 为 0，harness 未改工作区。证据：
  `2026-09-29/macos/m04-cef-native/`。
- 脚本明确 `productAcceptance=false`；当前保留的 **1/5** storage navigation timeout 也使长期/99% 门槛
  继续开放。尚未覆盖正式 packaged NomiFun Browser Resource/Agent Session、Tauri window close/reopen 后
  新 surface identity 与旧 frame/session fence、真实模型、release signing/artifact 或 soak；因此不关闭
  完整 `BROW-017`、`MAC-017`、D06 或 M04。

- **M04-02 macOS Computer permission/media**（`AUTH-012`、`COMP-008/009`、`MAC-011/012` 及
  `AGEN-013` observe-only 子断言）：在 Darwin 25.6.0 / macOS 26.6.2 arm64、APFS/owners enabled
  主机上，以 Developer ID 签名正式 Tauri app、每次独立 `NOMIFUN_DATA_DIR` 和独立 loopback vision
  断言执行。首次稳定 dev identity 已有 Screen Recording grant，旧 `--computer-denied` 假设失效；
  随后的 A11y 成功结果又夹带约 857 KiB screenshot，触发无工具压缩，而夹具拒绝。后续还依次保留
  压缩重用 call ID、typed multipart 解析缺失，以及当前 plan/completion guard 首次拒绝等失败，均未
  以放宽断言覆盖；证据：`2026-09-29/macos/m04-computer-permissions/run-003～009-*`。
- 根因一是 canonical `computer/a11y.observe` 复用旧组合 observe 并机会性采屏，把 Accessibility 与
  Screen Recording TCC、上下文预算错误耦合；现 canonical A11y 只返回 AX tree/ref cache、清除旧
  capture geometry，像素必须显式走 `computer/observe`，旧组合工具仍保留 overlay。根因二是原生
  screenshot 以 base64 JSON 文本穿过 Kernel；现只对精确 platform-builtin `computer/observe +
  screenshot`、当前 generation/Snapshot 及 ImageInput route 恢复单一 typed PNG，文本/持久 history
  不再携带像素。正式 canonical PNG 在 JSON hop 前压到 1.5 MiB（base64 ≤2 MiB），拒绝空、超限、
  非法 base64/PNG；避免旧 5 MiB 上限先被通用 4 MiB result 截成坏 JSON，旧 direct 路径不变。
- granted 正向 `run-010-final-ui`（Session `01a0ec40-9fb5-7603-8e8e-3920b1c01d88`）中设置 UI 明确
  两项已授权；5 个模型步精确调用一次 A11y（109 elements、零 pixels）和一次 screenshot，typed PNG
  到达 vision fixture，67-event durable projection 无 base64，`turn/completed`、fixture failure=null。
  UI/最终完成仍公开记有初次 plan guard 的 1 个工具错误。fresh bundle
  `com.nomifun.desktop.reliability.denied` 未点击授权：`run-012-denied-a11y` 和
  `run-013-denied-screen` 各自只启用一个 Action，工作台分别只列 Accessibility / Screen Recording
  缺口；两次精确调用均以 `ROLE_HOST_PROVIDER_FAILURE` 结束、无输入/截图 fallback，普通会话仍可开始，
  各 2 个模型步并 `turn/completed`。
- `nomi-computer` **94 passed / 7 ignored**；A11y no-pixels 与 macOS 权限维度各 **20/20**，Computer
  typed-media 3 项 **20/20 批**，role-host fence **4/4**，高熵截图 transport bound、正式 fixture
  `browser-use,computer-use` check、UARC boundary、fmt/diff 通过。三份 backend/companion DB 共六次
  immutable integrity 均 `ok`，最终 app/fixture process/listener 为 0；凭据前缀扫描为 0。最终证据与
  verdict：`2026-09-29/macos/m04-computer-permissions/run-010-final-ui/`、`run-012-denied-a11y/`、
  `run-013-denied-screen/`、`run-015-transport-hardening/`、`verdict.md`。
- 尚未通过 System Settings 人为制造“一项已授予、另一项拒绝”的 live TCC 组合；当前独立性来自
  可注入 permission 回归、两项 fresh-denied 正式 UI 和 granted 正向。撤权/热变更、真实 input/launch、
  stale window、多显示器/scale、held-input cancel、100-cycle/soak、release、真实 StepFun 及完整
  `AGEN-013` 仍开放，因此不关闭完整 `COMP-008/009`、`MAC-011/012`、D06 或 M04。

- **M04-03 macOS Computer launch/input**（`COMP-002/003/006`、`MAC-011`、`AGEN-013` 的本批
  正向子断言）：使用同一 exact-source Developer ID-signed `com.nomifun.desktop.dev`，每次 fresh
  data/Session、loopback fixture 和唯一 `computer-input.txt`。Mac 锁屏连续三轮时未绕过 UI：fixture
  保持 0 model call、文件 `seed`、DB 完整并清零进程；解锁后才由正式 Tauri 工作台选择仅允许
  A11y/input/launch 的 Agent 并发送验收请求。
- 首个解锁运行错误假设 atomic `computer/launch` 必须先被 plan gate 拒绝；Runtime 合同明确 Computer
  原子动作不自动激活 workspace ledger，故一次 launch 正确以 returned `managed_effect` 结算，fixture
  独立失败且零 input。第二次运行 8 个 effect 已正确执行并保存 `alpha XbetaY`，但 fixture 未校验被
  schema 拒绝的 `report_completion`，UI 正确显示 `NOMIFUN_TASK_INCOMPLETE`；两个首败均原样保留。
- fixture 修正为区分 optional plan 未暴露与产品权限错误，最终使用 Runtime canonical `input_0`、一个
  evidence-backed criterion 及 `observed_tool_error_count=1`，并在输出最终文本前验证 completion，拒绝
  `INVALID_TOOL_ARGUMENTS`/`not_executed`/unexposed-tool 假成功；未改产品 Action、权限、effect 或 planning
  policy。
- 最终 `run-008-final-ui`（Session `01a0ec7b-2f2d-7c30-86e3-8fbfe9aa9186`）为 20 model steps /
  299 events：1 次 launch、9 次 fresh A11y observe、7 次 input；依次 set `alpha beta`、`cmd+right`、
  `option+left`、type `X`、`ctrl+e`、type `Y`、`cmd+s`。8 个唯一 effect 全为 returned，未重放；
  TextEdit AX 与磁盘均为 `alpha XbetaY` 且无 edited marker，后续普通 X/Y 输入也证明 modifier 已释放。
  `completion_reported` 为 plan revision 1 / observation revision 19，覆盖 `input_0` 并公开唯一初始 plan
  tool error；`turn/completed`。fixture `input_verified=true`、failure=null；两 DB `ok`，TextEdit/app/
  fixture/listener 清零。确定性辅助：key **12/12**、launch **6/6**、main-queue/drag 各 **1/1**、
  role-host fence **4/4**。证据：`2026-09-29/macos/m04-computer-input/`。
- 仍未覆盖长 drag cancel、注入式 key/button release failure、用户并发焦点、Unicode/IME/layout、
  缺失 app、owner crash/result-loss、
  撤权、真实 StepFun 与 soak；窗口移动/缩放、same-PID 窗口切换及跨 App 焦点变化后的 stale 拒绝另由
  M04-05～08 覆盖，初始
  optional plan 仍以一次公开错误激活，不满足零工具错误体验。
  因此不关闭完整 `COMP-002～006/010～014`、`MAC-011`、`AGEN-013`、D06 或 M04。

- **M04-04 old MM failure/retry cold-history**（`AMUL-001`、`OBS-008/014`、`MGMT-013` 的 macOS
  历史恢复子断言）：只读扫描本机 10 个 NomiFun 数据库，未发现 Windows 原
  `WIN-MM-PROVISION-01` 或等价多模/创意失败 Session，故未冒充同一历史。新增隔离
  `--creative-failure` fixture，经正式 Tauri `creative-studio.default` Canvas 产生真实失败，再冷启动
  同一安装；不以成功首发替代失败历史。
- 首个 HTTP 400 夹具失败得太早，UI 显示静态 retry，但 Canvas `pendingTurn` 尚在、canonical Session
  已 ready，属于合法的提交未确认重试而非目标旧失败，完整保留于 `run-003-failure-ui/`。随后改为已
  接纳 SSE 的确定性协议终态；`run-005-terminal-failure-ui/` 精确产生 1 个 model call、1-step
  `turn/failed`，错误为 tool-call finish without Tool Calls，Session 回到 ready、Canvas
  `pendingTurn=null`，两 DB `ok`。停止并冷启动后相同 Turn/event/Canvas terminal state 不变，未产生
  第二个模型请求，owned process/listener 清零。
- 用户恢复后以同一 data、同一 failed Turn 再次正式 Tauri 冷启动：Canvas 显示原用户消息、
  `MM_HISTORY_FAILURE_FIXTURE` 和公开错误卡片；不存在 running spinner 或“重试这条消息”按钮，全程
  未点击 retry。冷读只把 canonical 两条消息 ID 一次性 reconcile 到 Canvas `messageIds` 并前进
  `updatedAt`；Agent events 仍逐字节相同，Turn/events/effects 保持 **1/30/0**，nodes/connections 与
  `pendingTurn=null` 不变，3 秒稳定窗口无循环写或新请求。最终两 DB `ok`、process/listener 为 0。
  Creative UI **23/23**、failed-history backend **1/1**；未改产品 retry/persistence 实现。证据：
  `2026-09-29/macos/m04-mm-retry/`。该等价隔离旧失败子断言 **PASS**；原 Windows 数据仍未冒充为
  macOS 复跑，且不关闭完整 AMUL/OBS/MGMT、D06 或 M04。

- **M04-05 macOS stale Accessibility observation**（`COMP-002` 前台焦点变化及 `COMP-010/011` 的
  零盲目输入/结算子断言）：正式 Tauri 首败在 TextEdit generation 2 observe 后切回 NomiFun，旧 ref
  仍成功对后台 TextEdit 执行 `SetValue("STALE_SHOULD_NOT_APPEAR")`；TextEdit AX 和最终 autosave 均
  证明错误输入，effect 误为 returned。根因是 macOS AX invoke 只比较 generation，未消费 observer
  dirty，也未核对 frontmost PID。
- macOS AX semantic invoke 现于动作前拒绝 generation 漂移、dirty window/focus/layout/value、无 live
  observer 或 frontmost PID 变化；显式 PID snapshot 不因无关前台 App 失效，但仍要求 observer/clean/
  generation。首修已阻止输入，却被 host 当 external unknown，产生 `cleanup_unproven` 暂停；进一步只对
  精确“stale + no pixel fallback”内部签名映射 `ROLE_HOST_STALE_OBSERVATION_GENERATION`，effect 结算
  rejected，其他 provider/join/backend 错误仍保持 uncertain。
- 最终 `run-011-final-ui`（Session `01a0ecf6-662c-7702-bdbc-dea173da1262`）在相同焦点切换后一次旧
  ref 调用即被拒绝；1 个 launch returned、1 个 input rejected、零 unknown/pending/pixel fallback，
  Turn **5 model steps / 69 events** completed。UI 明确报告未向后台 TextEdit 输入；TextEdit AX 与磁盘
  均精确 `seed`，fixture `stale_input_rejected=true`、failure=null。AX fence / 零效果 classifier /
  typed host mapping 各 **20/20**，相邻 AX **3/3**、Computer host **5/5**；两 DB `ok`，进程/监听清零。
  证据：`2026-09-29/macos/m04-computer-stale/`。
- 尚未覆盖 OCR/pixel-only ref、right/double-click pixel fallback、用户并发输入、
  held-input cancel、crash/result-loss 与 soak；因此只关闭本批
  `COMP-002` frontmost semantic-ref 子断言，不关闭完整 Computer、D06 或 M04。

- **M04-06 macOS same-PID window-move stale observation**（`COMP-002` 窗口移动子断言）：复用
  M04-05 的 exact-source Developer ID-signed 正式 Tauri app，但使用新的隔离 data/Agent/Session
  `01a0ed01-31b1-7ae0-b47d-8bcdce884dd0` 和一次性 TextEdit 文件。Runtime 在 TextEdit 同一 PID、同一
  前台窗口中取得 generation 2 / `textarea="seed"` 后暂停；随后只通过原生标题栏 drag 移动该窗口，
  TextEdit 在移动前后均保持前台和 focused，未切回 NomiFun，再释放旧 ref input。
- macOS observer 已订阅 `AXWindowMoved`；本次 generation 与 frontmost PID 均未改变，窗口移动通知使
  Snapshot dirty，旧 ref 在任何 AX/pixel 动作前以 `ROLE_HOST_STALE_OBSERVATION_GENERATION` 被拒绝。
  1 个 launch effect returned、1 个 input effect rejected，零 pending/unknown/pixel fallback；Turn
  **5 model steps / 69 events** completed。正式 UI 显示旧 observation 已拒绝；TextEdit AX 与磁盘均为
  `seed`，fixture `stale_observed=true`、`stale_input_rejected=true`、failure=null；两 DB `ok`，最终
  TextEdit/app/fixture process 与两个 listener 均为 0。证据：
  `2026-09-29/macos/m04-computer-stale/run-014-window-move-ui/`。
- 本批未发现新的共享或产品根因，故不改 `PROGRESS-SHARED`；fixture 完成文案仍沿用“切换前台窗口”的
  M04-05 描述，判定依据为独立 UI 动作、typed error/effect、事件、AX/磁盘与清理证据，不用该文案代替
  断言。窗口 resize / same-PID 多窗口另由 M04-07/08 覆盖；上述其余 Computer 边界仍开放，因此
  不关闭完整 `COMP-002`、Computer、D06 或 M04。

- **M04-07 macOS same-PID window-resize stale observation**（`COMP-002` 窗口缩放子断言）：以新的
  隔离 data/Agent/Session `01a0ed10-7d70-7703-b7b1-b645f659218b` 复用同一签名正式 Tauri app。
  Runtime 在前台 TextEdit generation 2 / `textarea="seed"` 后暂停；保持同一 PID、同一前台 focused
  窗口，从右下角以 window-relative `[670,438] -> [560,350]` 执行原生 drag，截图尺寸由约
  **673×439** 变为 **562×351**，再仅释放一次旧 ref input。
- `AXWindowResized` 通知使 Snapshot dirty；input 在任何 AX/pixel 动作前以
  `ROLE_HOST_STALE_OBSERVATION_GENERATION` 拒绝。launch/input effect 分别为 returned/rejected，零
  pending/unknown/pixel fallback；Turn **5 model steps / 69 events** completed。TextEdit AX/磁盘仍为
  `seed`，fixture `stale_observed=true`、`stale_input_rejected=true`、failure=null；运行中及停止后两 DB
  均 `ok`，最终 TextEdit/app/fixture process 和两个 listener 为 0。证据：
  `2026-09-29/macos/m04-computer-stale/run-016-window-resize-ui/`。
- 本批不新增共享结论或产品代码；UI 完成文案的旧“切换前台窗口”表述仍只作展示，不作为 resize 判定。
  same-PID 多窗口身份另由 M04-08 覆盖；用户并发输入、held-input cancel、crash/result-loss 与 soak
  仍开放，不关闭完整 `COMP-002`、Computer、D06 或 M04。

- **M04-08 macOS same-PID multi-window stale observation**（`COMP-002` 窗口切换子断言）：以新的隔离
  data/Agent/Session `01a0ed15-2ed6-7e00-bd34-a0e481e34cb4` 复用同一签名正式 Tauri app。Runtime 在
  `computer-input.txt` 前台窗口取得 generation 2 / `textarea="seed"` 后暂停；随后对仍在前台的
  TextEdit 执行 `cmd+n`，创建并聚焦第二个“未命名”空窗口，再释放第一窗口的旧 ref input。
- OS 进程快照证明两窗口始终只对应唯一 TextEdit PID `97002`；`AXCreated` / `AXMainWindowChanged` /
  `AXFocusedWindowChanged` 观察使 Snapshot dirty，旧 ref 以 `ROLE_HOST_STALE_OBSERVATION_GENERATION`
  在任何 AX/pixel 动作前拒绝。launch/input effect 分别 returned/rejected，零 pending/unknown/pixel
  fallback；Turn **5 model steps / 69 events** completed。第二窗口仍空白；以 `cmd+grave` 切回第一窗口
  后 AX 与磁盘均为 `seed`。fixture stale 两项为 true、failure=null；运行中/停止后两 DB 均 `ok`，最终
  TextEdit/app/fixture process 与两个 listener 为 0。证据：
  `2026-09-29/macos/m04-computer-stale/run-017-same-pid-multiwindow-ui/`。
- 本批不新增共享结论或产品代码；上述可审计动作、唯一 PID、两个窗口的独立内容及 typed effect/event
  共同判定，不以 fixture 沿用的旧完成文案代替断言。用户并发输入、held-input cancel、crash/result-loss
  与 soak 仍开放，不关闭完整 `COMP-002`、Computer、D06 或 M04。

- **M04-09 macOS raw pointer input**（`COMP-003` 的 move/click/scroll、`COMP-001` 同窗口
  A11y/screenshot 坐标关联及 `COMP-013` bounded screenshot 回归子断言）：新增最小
  `--computer-pointer-input` 确定性夹具模式；仓库外构建 arm64 AppKit 目标，以独立状态文件记录原始
  NSEvent。每次 raw input 前必须取得 fresh `computer/observe` Screenshot generation，并把
  **1568×882** screenshot 坐标映射回 **2560×1440** 屏幕；A11y generation 只用于确认目标窗口，不能
  给 raw pointer 授权。
- 首轮目标 App 只接受 argv、被 `computer/launch` 无参数启动后退出；修为隔离 Info.plist 状态路径。
  随后两次错误使用 A11y generation，均由 role host 以 `ROLE_HOST_STALE_OBSERVATION_GENERATION`
  在动作前拒绝，目标计数全零；另保留 typed screenshot JSON 形态误判、AppKit inactive first-click、
  launch/前台 settle 窗口不足及 view-only scroll 断言等夹具失败。click/scroll 最终以目标的本地/全局
  原始 NSEvent 监控判定，未以 tool returned 代替命中事实；所有中间运行、DB 与零残留清理均保留于
  `2026-09-29/macos/m04-computer-pointer/`。
- 最终 `run-023-final-ui`（Session `01a0ed44-c5d6-7420-bfd6-a4c1557e873c`）为 **13 model steps /
  164 events**：5 次 A11y 等待/稳定观察、3 次 bounded screenshot、1 次 launch 及 move/click/scroll
  各 1 次；4 个 effect 均唯一 returned，零 rejected/pending/unknown。产品映射精确为 move
  `(177,719) -> screen (289,1174)`、click `(312,643) -> (510,1050)`，scroll 最终光标 `(740,929)`；
  目标记录 move=2 个 OS motion event、click=1、scroll=1、delta=-3，fixture `pointer_verified=true`、
  failure=null，Turn completed，正式 UI 明确完成。durable events 中 inline PNG 为 0；运行中/停止后两
  DB 均 `ok`，最终 target/app/fixture process 和两个 listener 为 0。结合 M04-03 的 key/text/modifier，
  `COMP-003` 所列 action 集在 macOS 正向路径通过。
- 本批只扩展确定性夹具，未发现需改共享层或产品输入实现的新根因，故不改 `PROGRESS-SHARED`。尚未覆盖
  right/middle/double/triple click、drag cancel、OCR/pixel-only refs、多显示器/DPI、用户并发输入、
  crash/result-loss 与 soak；因此不关闭完整 Computer、D06 或 M04。合并并发共享 settlement 修复后，
  macOS post-merge Computer role-host **9/9**、owner-success settlement **1/1**、fixture build 与 fmt 通过。

- **M04-10 macOS missing absolute Computer launch**（`COMP-006` 缺失应用路径及 `OBS-005/016` 错误
  可见性子断言）：正式 Tauri 首败请求唯一、明确不存在的隔离 `.app` 绝对路径；旧
  `open::that_detached` 仅成功派生 `/usr/bin/open` 即返回，UI 随后暂停，managed launch effect 错误
  returned，而路径、应用进程和窗口均不存在。完整首败保留于
  `2026-09-29/macos/m04-computer-missing/run-002-first-ui/`。
- 产品在 Computer launch owner 的 OS 调用前新增 absolute target/app existence gate，不猜替代路径；
  Engine Core 对精确 `computer/launch` 的 provider failure 投影固定安全指引，不含真实私有路径，明确
  “target path does not exist / Do not guess / No successful launch”。首修正式运行已让 effect rejected，
  但旧 Kernel 仍只给模型泛化 `ROLE_HOST_PROVIDER_FAILURE`；该 31-event 中间失败独立保留于
  `run-006-final-ui/`，未用 code-only 结果关闭 Case。
- 最终 `run-010-final-ui`（Session `01a0ed6c-7414-7b12-8ed8-9734f4195e26`）为 **2 model steps /
  34 events**：唯一 launch effect 为 rejected，bounded observation 保留 owner 的精确不存在原因；模型
  可见 tool error 三处持久投影均含安全恢复指引，Turn completed。正式 UI 明确“不存在的应用路径已被
  拒绝，未猜测替代路径”；目标始终不存在，无替代 app/process/window，fixture
  `missing_launch_rejected=true`、failure=null；运行中/停止后两 DB 均 `ok`，app/fixture process 和两个
  listener 为 0。launch **7/7**、guidance **1/1**、两次正式 Tauri build、codesign、fmt/diff 通过。
- 本批公共根因另记 `S-D06-04`。相对缺失 app 名、安装后又删除的 TOCTOU、显式 opener 异步失败、撤权、
  Windows/Linux 原生 opener 与 soak 仍开放，因此只关闭 macOS 缺失绝对路径子断言，不关闭完整
  `COMP-006`、Computer、D06 或 M04。

- **M04-11 macOS user/Agent concurrent input**（`COMP-011` 焦点/内容变化检测、零争抢循环及
  `COMP-010` 零盲目重放的本批子断言）：新增最小 `--computer-concurrent-user` 确定性夹具模式，复用
  M04-05 已交付的 macOS AX observer dirty / generation fence；正式 Tauri 使用 Developer ID-signed
  arm64 app、隔离 data/Agent/Session `01a0ed96-3270-7ac1-8235-06082022ddcd`。Runtime 在 TextEdit
  `textarea="seed"` 上取得旧 ref 后暂停；外部用户动作只通过原生 TextEdit AX 把前台值改成精确
  `USER_OWNED`，释放旧 ref 前磁盘仍为 4-byte `seed`，未用磁盘写入冒充 UI 并发。
- 两个夹具首败独立保留：`run-002-final-ui`（Session
  `01a0ed8a-226b-7530-9719-97b614b136a4`）把 AX `setValue` 与 `cmd+s` 合并后，TextEdit 阻塞于
  `_NSDocumentSerializationSemaphore wait`，未释放 input effect；`run-003-final-ui`（Session
  `01a0ed90-8831-70a3-ad76-b56db2753ec7`）又被外部文件写入附加换行污染，触发 TextEdit 冲突提示及
  夹具 exact-byte 失败，虽产品已拒绝旧 input，仍不计通过。两次均保留事件/样本/DB，并完成精确清理。
- 最终 `run-005-final-ui` 为 **5 model steps / 69 events**：唯一 launch effect returned；用户值变化使
  observer dirty，旧 `set_element_value("STALE_SHOULD_NOT_APPEAR")` 在任何输入前以
  `ROLE_HOST_STALE_OBSERVATION_GENERATION` rejected，零 pending/unknown/重试循环，Turn completed。
  夹具只在结算时重新读取磁盘仍为 exact `seed` 后才置
  `stale_input_rejected=true`、`concurrent_user_preserved=true`；随后 TextEdit 正常 autosave 于 effect
  结算约 15 秒后把磁盘变为 exact 10-byte `USER_OWNED`，最终 AX/截图与磁盘一致，且均无 stale 文本。
  正式 UI 明确显示用户内容保持；运行中 DB `ok`，停止后以 immutable 只读方式复核两项均 `ok`，最终
  TextEdit/app/fixture process 和两个 listener 为 0。证据：
  `2026-09-29/macos/m04-computer-concurrent/`。
- 本批未发现新的共享或产品根因，只提交可重复的正式 UI 夹具与本页结论，故不改
  `PROGRESS-SHARED`。`COMP-012` 的 held drag/key cancel 与 release cleanup、`COMP-010` owner crash/result
  loss、right/middle/multi-click、OCR/pixel-only、多显示器/DPI 和 soak 仍开放；因此只关闭
  `COMP-011` 本批 macOS 子断言，不关闭完整 Computer、D06 或 M04。

- **M04-12 macOS held-input cleanup hardening**（`COMP-012` 的 timeout/release 组件子断言，
  **部分完成**）：走查确认原生 Computer 输入以 10 秒 timeout 包住 `spawn_blocking`，超时后直接丢弃
  handle；已开始的 drag/key worker 可继续持有输入并产生迟到效果，但上层已经收到失败。新增回归在
  10 ms 测试 deadline 返回时仍精确观察 `pressed=true`，首败保留于
  `2026-09-29/macos/m04-computer-cancel/run-004-first-failure/`，未用 sleep 或放宽断言掩盖。
- 产品修复使 timeout 后继续 join 同一已准入 worker，原生任务未退出前 Engine 不能得到 cleanup
  completion；返回仍是明确 uncertain error，要求重新观察且禁止自动重试。drag/key 新增统一 pressed
  obligation guard：OS press 前记录，逆序释放，首次 release failure 只重试精确剩余项，连续失败保留
  unproven obligation 并进入 Drop best-effort，不吞错。长拖拽确定性夹具另提供 dispatch 前显式 hold，
  便于后续物理/可用 out-of-band cancel runner 复验，不自动宣告通过。
- 验证：`nomi-computer` **99 passed / 7 ignored**；timeout join、release retry、连续 release failure 保留
  三项新增回归通过；Engine Tool Host 的 cancelled caller retained effect/settlement **1/1**；最终 fixture
  build、fmt/diff 通过。Developer ID-signed 正式 Tauri 与 arm64 AppKit 目标的多次隔离运行均实际记录
  mouse-down/drag/mouse-up，最终 `pressed=false`；其中 `run-015-formal-ui` 还保留超时后 effect unknown、
  `cleanup_unproven` 暂停及失败退出残留的完整证据，后续精确清理且两 DB `ok`。
- 但本机自动化 runner 在 held mouse 期间不能对退到后台的 NomiFun Stop 控件执行 AX/坐标动作：分别
  fail closed 为 stale element、no frame/no window；合成按键也不能替代真实物理全局 hotkey。相关时序
  失败完整保留于 `m04-computer-cancel/run-011/015/021/023/025/027/029/035/040/042/`，没有一次被改记
  PASS。因尚无正式证据证明 cancel event 发生于 `pressed=true`，本批不关闭 `COMP-012`、Computer、
  D06 或 M04；key-hold、真实 out-of-band cancel、OS release failure 当时仍待后续 runner，
  crash/result-loss 另由 M04-13 覆盖。

- **M04-13 macOS input owner crash / result loss**（`COMP-010` 的本批完整 macOS 子断言）：新增
  `--computer-input-crash` 确定性夹具模式及独立 arm64 AppKit 文本目标。Developer ID-signed 正式
  Tauri app 以隔离 data/Agent/Session `01a0ee3c-6854-7f83-8273-263152077159` 启动目标并取得稳定
  A11y observation；夹具随后唯一 dispatch 65,536 个 `X`。目标首次记录 **20** 字符时，runner 在
  精确核对 executable path 后只向 Nomi owner PID `36389` 发送 `SIGKILL`，不终止目标或 fixture。
  macOS 已排队输入最终稳定在 **44,480 chars / 2,224 change events**，因此不是“零副作用”；crash
  image 则精确止于 **5 model steps / 67 events**、`tool/call-started` + `effect/started`，Turn/head
  running、唯一 `computer/input` 为 `external_uncertain_effect/pending`，无 terminal/tool result，DB `ok`。
- 同一签名 app、同一 data 冷启动没有重放。该 Computer-only Session 没有 canonical workspace，自动
  recovery 三次明确拒绝后按既有有界策略原子提交 seq 68 `effect/uncertain`（`outcome=unknown`、
  `process_restart_external_reconciliation_required`）、seq 69 recovery-blocked 与 seq 70 turn/paused；
  pending=0、unknown=1、`cleanup_proven=false`。正式 UI 显示“执行已暂停 / 资源清理状态尚未确认 / 当前
  会话暂不接受新消息”，输入与发送禁用并提供“结束本回合”。连续 10 秒 target **44,480→44,480**、
  fixture model calls **5→5**、events **70→70**、input effects **1→1**；无第二次 dispatch、模型调用或
  输入。产品按钮结束回合后 head ready、Turn cancelled（seq 71），unknown receipt 仍保留；最终 DB
  `ok`，target/app/fixture 及 `57744/60965/57261` listener 均为 0。
- 定向对照：startup pending-external-effect quarantine **1/1**、暂停 UI **2/2**、fixture build、fmt 与
  diff 通过。没有发现新的共享产品根因，故不改 `PROGRESS-SHARED`；完整证据在
  `2026-09-30/macos/m04-computer-crash/`。本批关闭 `COMP-010` 的 macOS owner crash/result-loss
  子断言；`COMP-012` 真实 held cancel/key hold、其他手势/显示器/OCR 与 `COMP-014` soak 仍开放，
  不关闭完整 Computer、D06 或 M04。

- M04-13 后续 UI 审计修正上述“无共享根因”结论：同一正式截图虽证明主会话页暂停与发送禁用，侧栏
  悬浮卡却仍显示“活跃状态 运行中”。公共根因及修复记为 `S-D02-22`：startup quarantine 成功后向
  精确 owner 补发 canonical `turn.paused`，让既有列表订阅重读 durable Session；不改变
  `COMP-010` 的 unknown/零重放结论。集成回归 **1/1**、暂停/HoverCard/列表 refresh **4/4**。
  post-fix Developer ID-signed arm64 App（executable SHA-256
  `ec6caacdba1a81c42ab5acbbceba0b5f4d81237a334d6ffa016d709a92d40eb5`）从原 seq-67 crash image
  冷启动；复制旧 work-root marker 的首次夹具启动被产品 fail closed，纠正隔离绑定后再次得到
  paused/unknown，且 `turn.paused` 后出现新的 `/api/agent-sessions` canonical refresh。解锁后同一
  正式 UI 的详情页仍显示暂停/发送禁用，侧栏悬浮卡精确显示“活跃状态 执行已暂停”；产品按钮结束
  回合后 head ready、seq 71 cancelled，unknown receipt 保留。最终 DB `ok`，App 与 `63143` listener
  为 0。证据在 `m04-computer-crash/run-009～011-*`；该 OBS 冷启动投影子断言通过。

- **M04-14 macOS Unicode direct text input**（`COMP-004` 的 Unicode scalar、组合字符、emoji 与零
  shortcut 副作用子断言）：新增最小 `--computer-unicode-input` fixture 与仓库外 Developer ID-signed
  arm64 AppKit 文本目标；目标以 exact content、UTF-8 bytes、Unicode scalar、UTF-16 units、文本变化、
  modifier/shortcut 事件作独立 oracle。输入固定为 precomposed `é`、decomposed `e + U+0301`、中文、
  かな、普通/ZWJ+肤色 emoji 及非 BMP 音符的混合串，不切换系统输入法或键盘布局。
- 首个 data setup 因 fixture 漏编 `computer-use` feature 而被 `CAPABILITY_NOT_MATERIALIZED` 拒绝，零
  模型/目标副作用，保留于 `run-002-formal/`。feature-correct 首次正式 Turn 已物理输入正确内容，但
  fixture 错把 A11y 对不可见 U+0301/U+200D 的可审计 `\\u{...}` 展示转义当作内容丢失，Turn 以
  `EXECUTION_MODEL_INVALID_REQUEST` 暂停；target 此时已精确记录 **58 UTF-8 bytes / 31 scalars /
  36 UTF-16 units / 2 changes / 0 modifier / 0 shortcut**，失败完整保留于 `run-004-formal/`。修复仅把
  A11y 断言改为精确匹配其既有转义合同，原始 target content/三种长度与零副作用断言不放宽。
- 最终 `run-006-formal`（Session `01a0f016-f09d-7922-94cc-0372b8044a6e`）为 **9 model steps /
  113 events**；唯一 launch/input effect 各 returned，input args 保留原始 scalar，最终 A11y 结果保留
  `\\u{301}` / `\\u{200d}`，target exact content 与三种长度全部一致、modifier/shortcut 仍为 0。
  Turn completed，正式 UI 明确完成，运行中及停止后 DB `ok`，target/app/fixture 与 `53368/53299`
  listener 均为 0。fixture 双 feature build、fmt/diff 通过；完整证据：
  `2026-09-30/macos/m04-computer-unicode/`。
- 本批未发现共享产品代码根因，不改 `PROGRESS-SHARED`。真实 IME composition/候选窗、切换输入源、
  Dvorak/其他 layout、dead key 与 shortcut 组合仍开放；因此只关闭 `COMP-004` 的 macOS Unicode 直接
  文本子断言，不关闭完整 `COMP-004`、Computer、D06 或 M04。

- **M04-15 macOS large Accessibility tree**（`COMP-013` 的 node-budget/incomplete 子断言）：
  新增 `--computer-large-a11y` fixture、仓库外 Developer ID-signed arm64
  AppKit 目标及最小组件回归。独立目标实际创建 **200** 个可操作按钮，记录首项
  `AX_ITEM_000` 与尾部哨兵 `AX_OMITTED_SENTINEL_199`；current-source signed Tauri 的 canonical
  `computer/a11y.observe` 只返回 **120 elements / 3,409 chars**，明确包含
  `a11y tree truncated to the node budget` 与首项，不包含已由 target 证明存在的尾部哨兵，且零 pixels。
- 正式 Session `01a0f029-0ec1-7442-87a7-132f0e7a2867` 已 completed，**8 model steps / 100 events**，
  唯一 launch effect returned；A11y read 不制造 effect。fixture `large_a11y_verified=true`、failure=null，
  运行中 DB `ok`。新增组件回归及 `nomi-computer` 全组 **100 passed / 7 ignored**，同时证明 ref 120
  可用、ref 121 不存在。解锁后正式 UI 明确“结果标记 incomplete，不能把省略节点当作不存在”；原生
  目标尾部截图直接显示被 canonical snapshot 省略的 sentinel。最终 DB `ok`、SHA-256
  `0b8029c1d01e0b03306457969516ca1bb0f02b830f65676e37ea3dcec9a0cbd3`，target/app/fixture 与
  `55520/55439` listener 均为 0。证据在 `2026-09-30/macos/m04-computer-large-a11y/`。本批关闭
  `COMP-013` 的 macOS A11y node-budget/incomplete 子断言；OCR-large、截图之外的 pixel-only 目标及
  真实模型如何使用 incomplete 仍开放，不关闭完整 `COMP-013`、Computer、D06 或 M04。

- **M04-16 macOS Computer 100-cycle soak**（`COMP-014`）：新增 `--computer-soak` 确定性 fixture 与
  仓库外 Developer ID-signed arm64 AppKit 文本目标；固定执行 100 组 fresh A11y observe → 单字符
  type，每组 call ID 和 Effect operation ID 唯一。目标逐轮核对完整前缀，并独立记录 content、change
  events、modifier/shortcut 与 window focus-loss；任何重复、遗漏、陈旧 generation 或焦点漂移均当轮
  fail，不等最终状态掩盖。
- 首次 fixture build 因既有 `/status` 单个 `json!` 已逼近宏递归上限，新增字段后在编译期失败；完整
  保留于 `m04-computer-soak/run-001-fixture-build/first-failure.log`。修复把同一顶层 status JSON 分成
  三段 Map 合并，字段和 wire 形状不变，未提高 recursion limit 或删减断言；修复后双 feature build
  通过。
- 最终 `run-003-formal`（Session `01a0f052-59c5-7a72-80dd-a0d078912552`）由 current-source
  Developer ID-signed Tauri 执行：fixture **210 model calls**，canonical **207 model steps / 2,510
  events**；其中 3 次 compaction，仍精确保留 **100 distinct observe + 100 distinct input**，严格
  `observe_000→input_000 … observe_099→input_099`，order/index violation 均 0。唯一 launch effect 与
  100 个 input effects 全 returned，101 started/succeeded、0 failed/uncertain/pending/unknown。
- 独立 target 最终为 exact 100-byte/100-scalar 模式串、**100 change events / 0 modifier / 0 shortcut /
  0 focus loss**；Turn completed，正式 UI 明确 100 轮唯一结算且文本/焦点/清理稳定。运行中与停止后
  DB 均 `ok`，最终 snapshot SHA-256
  `aa8446bbe4dfd15f2ef83a9116b940a4b8c1f8de71791599dcca5ea86b051ce2`，target/app/fixture 与
  `59822/59744` listener 均为 0。证据：`2026-09-30/macos/m04-computer-soak/`。
- 本批未发现共享产品根因，不改 `PROGRESS-SHARED`。该 macOS `COMP-014` 子断言通过；更长 LONG/
  99% 统计、真实模型、并发用户干预、sleep/wake 与 release 构建仍按各自 Case 开放，不关闭完整
  Computer、D06、D09 或 M04。

- **M04-17 macOS raw click variants**（`COMP-003` 的 right/middle/double/triple 扩展子断言）：新增
  `--computer-click-variants` fixture 与仓库外 Developer ID-signed arm64 AppKit 原始事件目标；四个
  手势使用分离坐标，每次前均取得 fresh `computer/observe` screenshot generation，独立 target 按
  event type、global screen point、down 次数及最大 clickCount 判定，不以 tool returned 代替命中。
- 原生 target 首次 Swift `@main` build 漏 `-parse-as-library`，在生成 app 前失败并单独保留。首个正式
  `run-002-formal` 的 right-click effect 已 returned，映射坐标 `(177,719)→screen (289,1174)` 与 target
  `(290,1175)` 只差 1 px，但只装 local monitor 的 oracle 记录 0；Turn 暂停，完整 DB/status 保留，未改记
  PASS。新 target 复用 M04-09 已验证的 local/global 双 monitor，并按 type+timestamp+button+clickCount
  去重，以全局屏幕坐标判命中；产品 input 实现不改。
- 最终 `run-004-formal`（Session `01a0f069-d7f4-7190-af03-cd2cea2bb7eb`）为 **15 model steps /
  188 events**；4 次 fresh screenshot 与 4 次 input call 均唯一，launch + 4 个 input effects 全 returned，
  无 pending/unknown/retry。target 精确记录 right **1**、middle **1**、double **2 / max clickCount 2**、
  triple **3 / max clickCount 3**、unexpected **0**；正式 UI 明确原生命中与唯一结算。运行中及停止后
  DB `ok`，最终 SHA-256 `de64ff97b44a083b1a94645d002bc8f64aef9daf0dfb7eea9e00c01ef4047f7f`，
  target/app/fixture 与 `62477/62404` listener 为 0。证据：
  `2026-09-30/macos/m04-computer-click-variants/`。
- 当前宿主只有一个 online 主显示器（Q2790PQ，2560×1440，mirror off），因此不以本批截图缩放冒充
  `COMP-005` 多显示器/不同 scale PASS；该 live Case 明确阻断。本批无共享产品根因，不改
  `PROGRESS-SHARED`；关闭上述 macOS click-variants 子断言，held cancel、IME/layout、multi-display
  与 release 仍开放，不关闭完整 Computer、D06 或 M04。

- **M04-18 macOS packaged CEF dynamic signing prerequisite**（`BROW-017`、`MAC-017` 的正式
  product-bundle 签名/Seatbelt 前置子断言）：current-source arm64 host、固定 CEF 152.0.6 / Chromium
  152.0.7977.83 与五类 Helper 首次经 Developer ID 深度签名后，路径级及运行中
  `codesign --verify` 均通过，Helper 也实际进入 Seatbelt；但两个独立启动都在约三分钟后的 Chromium
  on-device utility 初始化时稳定报告 `errSecCSInfoPlistFailed (-67030)`。首次正式 UI 因 Mac 锁屏保持
  **0 turn / 0 effect / 0 model call** 后清理，未用 API 提交冒充 UI PASS；签名首败与锁屏运行分别保留于
  `2026-09-30/macos/m04-browser-packaged/run-002-formal/`、`run-003-signature-diagnostic/`。
- 根因是 Chromium 动态 peer 校验会把 outer bundle 的内存 `Info.plist` 重新序列化为 canonical XML，
  而原主包和脚本生成的 Helper plist 字节并非该序列化；因此普通磁盘/动态 `codesign` 可通过，带
  canonical plist bytes 的 Chromium 校验仍 fail closed。staging 现于签名前用系统 `plutil` 将主包和
  五个 Helper 的 plist 规范化为 XML，不改签名身份、entitlement、runtime hardening 或 CEF 校验。
- 修复后的独立 bundle（host executable SHA-256
  `4e4c4bcc50f141cdf39950a7b0d6ad5301bd2fd6ebccb0195af33a43a14d788f`）六份 plist 均与再次
  canonical roundtrip 逐字节相同，deep/strict 签名通过；同等 on-device utility 触发后
  `process_requirement/-67030` 为 **0**，主进程仍 dynamically valid，五类 Helper 保持 sandboxed。
  packaging contract **6/6**，Browser platform boundary 通过，停止后 App/Helper/listener 为 0、DB
  `ok`。证据：`2026-09-30/macos/m04-browser-packaged/run-004-fixed-product/`、
  `run-005-fixed-signature-soak/`。
- 本批为 macOS packaging 根因，不改 `PROGRESS-SHARED`。正式 packaged Browser AgentSession 的
  navigate/Unicode type/trusted click witness 仍须在解锁后以新隔离 UI 运行；Tauri 窗口关闭/重开、
  新 surface identity 与旧 frame/session fence 也仍开放，因此只关闭本签名/Seatbelt 前置子断言，
  不关闭完整 `BROW-017`、`MAC-017`、D06 或 M04。

- **M04-19 macOS native CEF 100-cycle soak**（`BROW-018` 的本批平台子断言）：在 Mac 仍锁屏、正式
  packaged UI 不可操作期间新增独立 `--soak-only` native runner，不用 API 提交冒充 UI。Developer
  ID-signed arm64 Tauri/CEF bundle 在同一 ephemeral runtime、同一 tab 上执行固定 100 轮
  navigate → readiness → fresh semantic observe → Unicode type → fresh observe → native click → final observe；
  下一轮还必须以精确 `StaleTarget` 拒绝上一 document 的未消费 ref。
- 首次 compile 漏引 `BrowserRuntimeFactory`；前三次正式运行又依次暴露 runner 把隔离 evaluation world
  当页面 world、以页面全局变量作 oracle，以及把跨 document fence 错写为 `StaleObservation`。产品均
  fail closed，runtime close / engine shutdown 正常；失败完整保留于
  `2026-09-30/macos/m04-browser-soak/run-001-build/`、`run-002-formal/`、`run-003-formal/`、
  `run-004-formal/`。修复后的 oracle 只从隔离 world 读取页面事件监听器写入的 DOM `data-*`，不穿透
  world；跨 document 则精确要求 `StaleTarget` 99 次，未放宽为多个可接受错误。
- 最终 exact-source `run-007-final-source` 完成 **100/100**，document generation **3→102**、旧 ref
  `StaleTarget` **99/99**；每轮页面独立证明 exact `soak-NNN-中文`、click count 1 与
  `event.isTrusted=true`。全过程始终 1 runtime / 1 tab，零 download/dialog/permission 残留、零 cycle
  error；首 20 轮 median/p95 **311/504 ms**，末 20 轮 **316/524 ms**，最大 **524 ms**，无随序号
  退化。全部 Helper 在第 10 轮前已启动，之后无新增；`shutdown_complete=true`，最终 App/Helper 为 0，
  deep/strict 签名及六份 plist canonical roundtrip 通过。
- 本批只扩展确定性 runner/页面 oracle，未发现新的产品或共享根因，故不改 `PROGRESS-SHARED`。
  runner 仍明确 `productAcceptance=false`；正式 packaged AgentSession UI、真实模型、Tauri 窗口重开
  identity/fence、睡眠唤醒及更长 LONG/99% 门槛仍开放，因此只关闭 `BROW-018` 的本批 macOS native
  100-cycle 子断言，不关闭完整 Browser、D06/D09 或 M04/M06。

- **M04-20 macOS native CEF Tauri window close/reopen**（`BROW-017`、`MAC-017` 的本批平台
  lifecycle 子断言）：新增独立 `--window-reopen-only` runner；隐藏 guard window 只用于在真实 main
  window 销毁期间保留 Tauri event loop，不承载 Browser surface。旧 main window 上创建 runtime
  generation 51 / document generation 2，取得 fresh semantic ref 后用原生 `confirm` 留下 owned pending
  work，再真正关闭该 NSWindow。
- 关闭后 cancel/finish 精确 drain pending dialog，旧 runtime 随后 close；旧 snapshot 与旧 ref action 均
  精确返回 `WorkspaceClosed`。以同一 `main` label 新建 Tauri window 时 NSWindow identity 不复用；新
  runtime generation 52 / 新 tab identity 均唯一，旧 target 对新 runtime 精确返回 `TabNotFound`。新
  surface 随后以 `reopened-中文` 和页面 DOM oracle 证明 click count 1、`event.isTrusted=true`。
- 首次 build 的 cargo check 实际通过，但外部 `tee` 目录尚未创建导致证据捕获命令非零，已单独保留且
  未冒充产品失败。正式 `run-002-formal` 首轮即全部通过，`shutdown_complete=true`；Developer ID
  deep/strict 签名通过，无 `process_requirement/-67030`，最终 App/Helper 为 0。证据：
  `2026-09-30/macos/m04-browser-window-reopen/`。
- 本批未发现产品或共享根因，不改 `PROGRESS-SHARED`。runner 仍明确 `productAcceptance=false`，且只
  覆盖 native Tauri/CEF window、runtime/tab/ref 与 pending-dialog owner；正式 packaged NomiFun
  AgentSession UI、close-to-tray renderer attach/detach、nested frame 及真实模型仍须单独验证。因此只
  关闭 `BROW-017/MAC-017` 的本批平台 lifecycle 子断言，不关闭完整 Browser、D06 或 M04。

- **M04-21 formal packaged NomiFun Browser AgentSession**（`BROW-001/003/015/017`、`MAC-017` 的
  本批正式产品子断言）：用户解锁后以 current-source Developer ID-signed arm64 NomiFun、固定 CEF
  152.0.6 / Chromium 152.0.7977.83、fresh data/Agent/Session 和正式 Tauri UI 提交
  `Verify the packaged native Browser surface.`。此前 `run-002` 锁屏运行保持零 Turn；本次首个
  `run-008-formal-ui` 已真实 navigate/type/click 并收到 trusted witness，但 fixture 的
  `report_completion` 漏必填 `observed_tool_error_count=1` 且发送空 `requirement_ids`，产品正确拒绝；
  最终 Turn failed，完整 DB/UI/status 保留，未改记 PASS。
- 同一首败还暴露正式产品的 macOS CEF profile 根因：CEF 要求 disk profile 是
  `root_cache_path` 的直接子目录；产品 root 为 `browser-v3`，canonical Session profile 却在
  `browser-v3/agent-sessions/<hash>`，CEF 因此明确记录 `Cannot create profile` 并退回 OffTheRecord。
  修复只把 macOS CEF root 移到既有 canonical profile 父目录 `browser-v3/agent-sessions`；共享
  `BrowserProfileStore`、hash identity、清理边界及 Windows 路径不变。fixture completion 另补精确累计
  tool-error count、在 summary 公开披露，并省略空 requirement 数组。
- 最终 `run-011-formal-ui`（Session `01a0f0b1-7e6d-7113-aead-5a2b099c5457`，host executable
  SHA-256 `ce1890564f853902224f83707c3afa58490885844393a385dd8d073d633a9622`）为
  **10 model steps / 148 events**；2 次 navigate、Unicode type、trusted click 四个唯一 Effect 全
  returned，零 pending/unknown。页面独立 witness 为 count 1 / exact `Agent 主界面真实输入` /
  `trusted=true`；正式 UI 同时显示计数 1、文本和“已收到真实点击”，关闭 Browser panel 后显示完成总结
  及累计 **1** 个 plan-guard tool error。
- canonical completion 为 plan revision 1 / observation revision 9，两个 criterion 均由 final observe
  `gui-native-7` 支持并覆盖 `input_0`；`turn/completed`、head ready。唯一 64-hex Session profile 已在
  新 root 直接落盘，冷读 `History` 为 `ok` 且只在该 profile 记录 loopback URL（visit count 2），global
  Default 无该 URL；App 日志不再出现 profile/CEF/process-requirement 错误。backend DB `ok`、inline PNG
  0，最终 App/Helper/fixture/listener 为 0。完整证据：
  `2026-09-30/macos/m04-browser-packaged/run-008-formal-ui/`、`run-009-fix-build/`、
  `run-010-fixed-product/`、`run-011-formal-ui/`。
- 该 profile 根因是 macOS CEF host 专属，未改共享层，故不改 `PROGRESS-SHARED`。本批关闭正式 packaged
  Browser navigation/input/trusted-page-result 与精确 persistent profile 子断言；真实 StepFun、产品
  close-to-tray renderer attach/detach、nested frame/popup 的正式 UI 以及 release/notarized artifact 仍开放，
  不关闭完整 `BROW-017/MAC-017`、D06 或 M04。

- **M04-22 formal close-to-tray Browser lifecycle**（`BROW-017`、`MAC-017`）：使用另一个 fresh
  data/Agent/Session `01a0f0bb-b05f-7442-b59f-2fb84bb4ce92`，由正式 signed Tauri UI 提交相同 Browser
  Turn；native child 已显示页面且 Agent 仍为 running 时点击 macOS 主窗口关闭按钮。产品按既有合同
  隐藏到 tray 而不退出：CUA inventory 明确 App 仍 running，隐藏期间 Turn 到达 completed/head ready，
  loopback witness 为 count 1 / exact Unicode text / `trusted=true`。
- 当前 macOS CUA 没有 `computer.launch_app` 函数，该首次 runner 尝试原样失败且未影响 App；随后按
  支持的绝对 bundle path 重新取得同一 running App，正式主窗口恢复。重开后 Browser page 精确保留原
  URL、计数 1、`Agent 主界面真实输入` 和“已收到真实点击”，没有新模型调用或 Effect。随后通过正式
  Session tool 关闭/重开 Browser panel，renderer detach/reattach 后仍只显示一个 tab 与同一页面状态。
- reattach 后 0 秒与 5 秒两次独立核对均为 **10 model calls / 148 events / 4 returned Effects / 0
  unsettled / 1 witness**，无重放或重复 surface；App 日志无 profile/CEF/process-requirement 错误。停止后
  backend DB 与 Session profile History 均 `ok`，目标 URL 仍只在绑定 profile，最终
  App/Helper/fixture/listener 为 0。完整证据：
  `2026-09-30/macos/m04-browser-close-to-tray/run-001-formal/`。
- 本批未发现产品或共享根因，只提交本页结论。结合 M04-20 的真正 Tauri window 销毁/重建、唯一
  NSWindow/runtime/tab identity、旧 target/ref fence 与 pending-dialog drain，以及 M04-21 的正式
  packaged AgentSession/profile，`BROW-017` 与 `MAC-017` 的 macOS 功能验收槽现为 **PASS**。这不关闭
  其他 Browser Case、完整 D06/M04；真实 StepFun、nested frame/popup 正式 UI、release/notarization 与
  LONG/99% 仍按各自槽开放。

- **M04-23 real StepFun Browser + Workspace functional chain**（真实 Provider 的本批单次预算冻结
  子断言）：只读本机 `NomiFun-dev` 已加密 Provider，确认 exact `stepfun-plan` endpoint 与
  `step-3.7-flash`；一次性仓库外 launcher 在内存 AES-GCM 解密后只把 key 放入既有 live wrapper 环境，
  Cargo/argv/App/fixture 文件和日志均不接收明文。`run-001` 先选到无 credential 的 production data，
  在任何请求前 fail closed；`run-002` 首次 App 又因未传 exact `NOMIFUN_WORK_DIR` 被 work-root receipt
  拒绝，均原样保留。
- 修正 App 启动后，`run-002` 的真实模型仍把 Workspace 看成空目录并在 29 calls 时由 UI 停止。DB
  证明 Session binding 已准确冻结 `work` root；根因是 fixture 把 `app.js` 放在上层 `work/`，而当前
  `default-workspace` 合同正确物理化为 `work/conversations/<session_id>`。fixture 现只在 exact managed
  Session child 创建源码，HTTP `/app.js` 也读取同一路径；新增原生路径回归覆盖 macOS `/var` →
  `/private/var` canonical alias，且 root-level `app.js` 必须不存在。
- 最终 `run-004` 由正式 signed Tauri UI 提交 exact loopback URL 与 repair 任务，真实
  `step-3.7-flash` 得到 trusted witnesses **[2, 2, 1]**，`changed_source_served=true`，最终绑定源码为
  exact `function nextCount(value) { return value + 1; }`。第二个 2 是模型在 patch 成功前过早 reload/click，
  未从历史中删除；一次 patch payload 被精确 rejected，随后一次 patch returned。Browser effects 为
  navigate **7 returned**、act **3 returned**，最终零 pending/unknown、inline PNG 0。
- 本轮冻结上限为最多 32 次真实 Provider 请求；Runtime 在 **30 model steps / 635 events** 后因 fixture
  call cap 投影 `EXECUTION_MODEL_RATE_LIMITED`，`host_cleanup_proven=true`、head paused。功能链完成但
  未提交 canonical completion，因此不记 terminal PASS；正式 UI “结束本回合”后 Turn cancelled/head
  ready，最终 DB `ok`。wrapper 被 Ctrl-C 后两个 detached fixture 曾各保留唯一 listener，均在核对
  exact PID/executable/argv 后定向 TERM 并清零；App/Helper/listener 最终为 0。
- 凭据审计扫描本批 **869** 个仓库外文件、Git diff 与进程 argv，exact credential match 为 0；不提交
  launcher、DB、日志或凭据。完整证据：`2026-09-30/macos/m04-browser-live-stepfun/`。本批只修 fixture
  的 exact Session source placement，不改 `PROGRESS-SHARED`；真实 Provider 功能链子断言通过，但
  terminal、N3/20/99% 与效率门槛保持开放。暂停 UI 同时把 canonical `cleanup_proven=true` 错显示为
  “资源清理状态尚未确认”，作为下一公共 D02 问题簇处理，不在本条冒充已修复。

- **M04-24 pause cleanup projection follow-up**（`OBS-006/007/014/015`、`LIFE-015/016` 的本批
  macOS 来源/共享前端子断言）：复用 M04-23 首次正式 UI、DB 与事件，不重写原结果。canonical pause
  已明确 `cleanup_proven=true`，错误文案来自 `turn.paused` 后首个 authority GET 的短暂旧投影，而非
  macOS host cleanup 失败；共享修复及边界记为 `S-D02-23`。
- 新确定性回归证明首个未证明 pause 后的 cleanup-proven snapshot 会胜出，同时证明连续两次相同未证明
  pause 仍保留 `cleanupRequired`，没有靠吞错或放宽断言制造通过。相关 reconcile/hook/notice
  **18/18**、UI typecheck、desktop UI boundary 与 diff check 通过；首次错误测试入口产生的 2 个 DOM
  harness 失败和修复后日志分别保存在
  `2026-09-30/macos/m04-pause-cleanup-projection/run-001-tests/`、`run-002-tests/`。
- 为冻结真实模型预算，本批没有再次调用 StepFun，也没有 post-fix 正式 Tauri 截图，故只记公共竞态修复
  与确定性验证；M04-23 terminal、真实 Provider N3/20/99%、完整暂停/断线 UI 矩阵仍开放。

- **M04-25 native drag cancellation/release**（`COMP-012` 的有界 drag 期间真实取消子断言）：
  macOS 26.6.2 / arm64 / APFS 上复用 Developer ID-signed 正式 Tauri App
  `ce1890564f853902224f83707c3afa58490885844393a385dd8d073d633a9622`；相关 Computer/cancel/retained
  effect 源码与当前分支无 diff。前两次 runner 分别缺 Browser feature、Computer host materialization，
  在 Turn 前 fail closed；第三次中断仅有 3 条 Session 初始化事件，首次失败均保留。
- fresh Session `01a0f112-5e48-7d80-bf2f-be65e77ea170` 由正式会话 UI 提交任务。仓库外监视器只读隔离
  owner 的既有签名凭据，在内存生成短期 JWT，经原鉴权向产品 `/turns/cancel` 发送唯一请求；未认证
  对照为 403，已认证读取/取消为 200。seq 101 已取消后的 **17 ms** 独立采样仍为 pressed；目标唯一
  mouseDown/mouseUp、7 个 drag 事件，最终释放，实测按住 **106.6 ms**。input effect returned，seq 105
  `host_cleanup_proven`，8 model steps / 106 events，零 pending/unknown；正式 UI 显示“已取消执行”
  并保留真实拖拽结果。取消后无模型重开，退出前后 DB/备份均 `ok`。
- UI runner 在关闭目标后读取 AX 曾重新启动目标，已核对唯一新 PID 后定向 TERM；最终
  App/Helper/target/fixture 与两个 listener 为 0。普通证据 45 文件签名凭据匹配为 0，全部制品在
  `2026-09-30/macos/m04-computer-held-cancel/`。本批无产品根因/源码改动、付费请求为 0；只关闭
  有界 drag 的取消/释放子断言，长时 hold、key hold、原生 release-failure、N3 与完整 COMP-012 仍开放。

- **M06-01 native shutdown impact verification**（`PROC-040`、`LIFE-019/023/024/029`、`CONC-014`
  的本批子断言）：合入 `c8fea229f/abf6ed4d9/c3b3dc9ac` 后在 macOS arm64 / APFS 复核
  `S-D04-30/S-D09-46/47`。首次普通 shutdown **3/3**；导出 PID 证据的重跑却在长 poll 前失败：
  marker 移到仓库外，helper 仍绑定仓库 cwd，Seatbelt 正确拒写。首败保留，夹具现 canonicalize 隔离
  evidence root，并将 cwd 与唯一 capability root 同时绑定该目录；生产权限/预算与 6 秒断言不变。
- 修复后原生长 poll 首次 + 20 repeats **21/21**，独立 marker/report/PID disappearance **21/21**；
  唯一 Cancelled/reaped 唤醒原 60 秒 poll，重复 shutdown 返回原报告，helper 残留 0。DesktopServer
  正常退出/真实 SQLite writer lock 两项在前后两个源码快照均 **2/2**；4 个独立 DB 和唯一文件核对
  `ok/cancelled/head ready/one returned write/one cleanup witness`，未重放写入。当前 merged host/journal
  清理投影与未准入回执拒绝 **4/4**，Rust fmt/diff 通过。
- 全证据：`2026-09-30/macos/m06-native-shutdown/`。本批只修证据夹具根绑定，付费请求 0；没有正式
  Tauri renderer、真实满盘/fsync、跨重启/lease 过期或其他角色证据，因此不关闭完整 PROC/LIFE/M06。

- **M04-26 pause evidence beyond event page**（`OBS-006/007/014/015`、`LIFE-015/016`）：修正
  M04-24 的竞态归因；实际是前 500 事件页缺少 seq 635 的 pause，公共根因/修复见 `S-D02-24`。
  原生 pause/resume/唯一写入反例首次 field=null，修复后 **21/21**；Store exact/cursor/foreign/false
  **1/1**，相关 UI **18/18**、typecheck/desktop boundary/build 通过。初始两个测试夹具编译/事件注册
  失败、GUI ID 重用/压缩后缺历史均单独保留，未改写为产品 PASS。
- 正式 Developer ID arm64 前后样本均 pause **seq 605 / 41 model steps / 48 loopback requests**，
  4 effects 全 returned、零 pending/unknown；首败三个 projection 缺 `execution_pause`，当前源码 App
  `19634d451f452d97185a4cba3f8a6f0c8e445c881adf99700c5f3e986dcd3251` 三次均返回 exact reason/
  `cleanup_proven=true`。正式截图从“资源清理状态尚未确认”改为“任务尚未完成，已完成的操作会保留”，
  暂停时输入禁用；UI 结束回合后 head ready/Turn cancelled，输入及 API 发送能力恢复。
- macOS 26.6.2 / arm64 / APFS；deep/strict 签名、DB/备份 `ok`，最终 owned App/Helper/fixture/
  listener 0。49 份普通证据的签名凭据匹配 0，付费 Provider 调用 0。全证据：
  `2026-09-30/macos/m04-pause-event-page/`；只关闭该投影子断言，M04-23 live terminal、其他角色/
  平台、多窗口/断线、N3/99%、release/notarization 与完整 D02/M04 仍开放。

- **M06-02 native start delivery/cancel**（`PROC-039/040/042`、`CONC-004`、`LIFE-029` 的本批
  子断言）：macOS 26.6.2 / arm64 / APFS 原生复核 S-D04-31。首轮 **5/5**，首次重复却 **4/5**：
  PID marker 已证明 child 执行，drop caller 后 shutdown 空报告、返回当时 child 仍活；之后由底层 poller
  回收，未用独立 fallback。首败在 `run-002-repeats/repeat-01`，未覆盖为 PASS。固定 COMMITTED
  窗口的反例另确认外层取消返回 StartLost 并丢失 native owner；新增测试的首个缺函数限定编译错误单列。
- 公共根因/修复见 `S-D04-32`：Supervisor 已持有的 Unix worker 在 commit 后继续交付精确 owner，
  注册原 Session 的 retirement/cleanup witness，独立底层 future-drop 路径不变。修复后 Pipe 五场景
  首次 + 20 repeats **105/105**、PTY 两场景 **42/42**；独立 marker/唯一启动/报告/PID 消失
  **147/147**，fallback 0。固定窗口反例 **21/21**，独立底层 drop 两相邻 **2/2**、
  deadline-before-fork **1/1**、shutdown **8/8**、registry shutdown **3/3**，fmt/diff 通过。
- 全证据：`2026-09-30/macos/m06-start-delivery/`；本批不调用模型或正式 Tauri，不改 Windows 原
  结果。native 失败/unknown、worker panic、失去 ownership、更多 descendants/角色及完整
  PROC/CONC/LIFE、N3/100 seed/LONG/99% 仍开放。

- **M06-03 committed start IO failure**（`PROC-039/040/042/047`、`CONC-004`、`LIFE-029`）：
  macOS arm64 / APFS 的固定故障窗口已让真实 child 执行，随后注入 stdio wrap failure；首败
  shutdown report 为空、返回时 PID 尚未回收，之后 poller 才完成清理。完整首败保留于
  `run-001-first-failure`；首次修复编译漏 Unavailable Drop 分支、重复第 5 轮读到未写完 PID 行均单列。
- 公共根因/修复见 `S-D04-33`：启动错误携带 native owner 回到 Supervisor，先注册原 Session，
  按原 retirement/清理预算完成或保留真实未决状态；普通 start 返回原 StartLost/code/PID，并发
  shutdown 维持取消优先并保留原 owner 清理报告。
  测试只接收换行结尾的完整 PID 行，原 2 秒发布/6 秒回收上界不变，未加 sleep 或放宽断言。
- Pipe/PTY × 普通 start error/并发 shutdown 四场景，最终首次 + 20 repeats **84/84**；
  独立 marker/报告/PID 消失 **84/84**，错误从未改写为成功，首败与 marker-failure PID 亦已消失。
  process Runtime lib **149/149**、session registry **13/13**、Rust fmt/diff 通过，付费调用 0。
  证据 `2026-09-30/macos/m06-start-failure/`。本批只有原生组件证据；setup deadline/commit handshake
  失败、worker panic、清理失败与未知恢复、更多角色/descendants、正式 Tauri 及完整 Case/LONG 仍开放。

- **M06-04 startup deadline ownership**（`PROC-039/040/047`、`CONC-004`、`LIFE-029` 的本批
  子断言）：macOS arm64 / APFS 固定原生 COMMITTED 后的 worker 窗口。首败 caller 在约 105 ms
  收到 StartLost，但 worker 仍被持有时 shutdown 已空报告返回，PID 未回收；原 worker 后续清理，
  首败日志/JSON/PID 保留于 `run-001-first-failure`。
- 公共根因/修复见 `S-D04-34`：setup deadline 通知只结束 caller 等待，Supervisor 原 worker 继续
  持有 JoinHandle/预留/准入租约；迟到 commit 只交原 owner 做 retirement，不能成为成功 start。
  standalone deadline/drop 合同和原 setup budget 不变，未开始的原 worker 放行后仍不得 fork。
- Pipe/PTY committed shutdown、PTY quiesce、Pipe pre-fork 四场景首次 + 20 repeats **84/84**；
  独立 PID **63/63**、零 fork/零 marker **21/21**，100 ms setup budget 下 caller 最大 **105 ms**，
  原 caller 350 ms/cleanup 6 秒断言未放宽。原生报告在 worker live 时均未完成，最后准确回收。
  最终 Runtime lib **153/153**、session registry **13/13**、fmt/diff 通过，付费模型调用 0。
  中间 raw PTY watchdog-after-COMMITTED 检查曾 PeerClosed（152/153），首败保留在 `run-007`，
  该独立注入窗口在本批仍开放，未用最终快照通过关闭它；接续根因与结果见 M06-05。
- 全证据 `2026-09-30/macos/m06-start-deadline/`；本批只有组件/原生证据，未跑正式 Tauri、Linux/
  Windows 原生；commit handshake 失败、worker panic、清理失败/unknown recovery、更多角色/
  descendants、完整 N3/100 seed/LONG/99% 与全部阶段门槛继续开放。

- **M06-05 macOS watchdog fork initialization**（`PROC-041/046/047` 的 native watchdog
  异常、精确回收与未开始 deadline 子断言）：宿主 macOS 26.6.2 / 原生 arm64 / APFS，付费调用 0。
  旧 raw PTY PeerClosed 单跑及 20 repeats 均通过，未据此覆盖首败；同时间系统 crash report
  证明 watchdog PID 13729 在 `fork → libSystem_atfork_child → _notify_fork_child` 内因
  `os_once_t is corrupt` 被终止，尚未进入 BootReady/COMMITTED。原日志和 crash 另存保留。
- 公共合同修复见 `S-D04-35`：两个 macOS watchdog 启动入口均在宿主、fork 前完成 libnotify
  无注册副作用的初始化，等待仍消耗原 setup budget；耗尽后零 watchdog/user fork。确定性夹具
  只固定初始化未完成窗口，不篡改私有系统锁；Pipe/PTY 修复前 **0/2**，先完成原 owner 清理再失败。
- 修复后初始化等待/预算耗尽四场景首次 + 20 repeats **84/84**；pipe/PTY COMMITTED 后
  watchdog 失效 **42/42**，其中历史 PTY 用例 **21/21**，仍为 lifecycle failure、精确 SIGKILL
  和 leader/watchdog 各一次 reap。独立进程探测 **126/126** PID 消失，零 fork 窗口 **42/42**，
  100 ms setup 下 caller 最大 **103 ms**，原 350 ms/cleanup 3 秒上界未放宽；Runtime **157/157**、
  registry **13/13**、process boundary、定向 fmt/diff 通过。证据 `2026-09-30/macos/m06-pty-watchdog/`。
- 仅收敛该首次失败的 macOS 通知库初始化根因；未验证其他系统 fork callback、真实 Tauri 强退/
  sleep-wake、失控 descendants、worker panic/unknown recovery、其他平台/角色、release 与
  N3/100 seed/LONG/99%。不关闭完整 PROC/M06 或阶段二、三，Windows 原结果未改写。

- **M06-06 executed start / commit failure**（`PROC-040/050` 的 startup error、shutdown 责任和
  清理重试子断言）：macOS 26.6.2 / 原生 arm64 / APFS，付费调用 0。PID marker 先证明 user exec，
  再固定 COMMIT/pre-COMMITTED watchdog 退出及 cleanup hold。首败 start/shutdown 提前完成，
  原 owner 不在报告、PID 尚未回收；随后底层清理完成。首败 JSON/log 保留于 `run-001`。
- 公共根因/修复见 `S-D04-36`：Supervisor 的原 start worker 接回带原握手错误的 cleanup-only
  owner，注册原 Session 后等待真实 group/leader/watchdog 回收；不发布可用 handle，不误报
  user-code-not-started。普通 start 保留原 code/PID/清理证明，并发 shutdown 保留取消优先和报告。
  独立平台 raw/drop 路径不变，无精确 group anchor 仍不能生成成功清理证明。
- 强化收据检查另两次 **2/4** 暴露正常退出等待覆盖了先前 cleanup 诊断；`run-008/009` 原样
  保留。现正常等待不覆盖实际诊断，原握手错误及清理未证明事实均写入最终收据，未放宽断言。
- Pipe/PTY × 普通错误/并发 shutdown 最终首次 + 20 repeats **84/84**；独立 marker/错误/报告
  **84/84**、物理 PID 消失 **168/168**，leader/watchdog 均恰好 reap 一次，fallback 0。
  外部 anchor 负向 **1/1**、Runtime **162/162**、registry **13/13**、boundary/fmt/diff 通过；
  原 2 秒 marker/6 秒收敛上界不变。全证据 `2026-09-30/macos/m06-commit-failure/`。
- 本批只有原生组件证据；caller drop 与握手失败组合、pre-exec deferred cleanup、其他 owner
  transfer failure、worker panic/未知恢复、正式 Tauri/其他平台角色及完整 Case/LONG/99% 仍开放。
  Windows 原结果未改写，不关闭完整 M06 或阶段二、三。
