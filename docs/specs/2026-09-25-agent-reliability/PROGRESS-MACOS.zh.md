# macOS Case 处理进度

更新：2026-09-29。当前已由 macOS arm64 原生执行者接续；此前 Windows 结果仍只作共享历史引用。
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
| D02 | 291 | M02/M04 | 控制/完成、原生 UI、停止/纠正及错误可见性 | M01-03/04 正式 Tauri `CMD-132/139`；M02-01/02 计划、完成与 UI 投影子断言通过；完整 Case 未验收 |
| D03 | 145 | M01/M03 | APFS 大小写/NFC/NFD、权限、原子文件与 Artifact | M01-02、M03-01、M03-05～07 APFS/发布/ACL/xattr/immutable 子断言已验证；其余待走查 |
| D04 | 476 | M01 | /bin/sh/zsh、字面 argv、PTY/process group、Seatbelt 与退出码 | M01-01/03～05 已验 owner、`CMD-132/139`、host 映射及 login-shell 子断言；CMD 全集待验 |
| D05 | 69 | M03 | 隔离 Git remote/SSH、凭据/权限与未知结果 | M03-02/03/08～11 本地 Git 字面路径、commit/local remote、四类 commit hook/身份、unknown fence 与持久 receipt 重放子断言通过；网络/SSH 条件资源待准备 |
| D06 | 250 | M04/M05 | WKWebView、A11y/Screen Recording、MCP/Plugin/Skill | 需 macOS 权限/设备夹具，不从 Windows 外推 |
| D07 | 124 | M02/M04 | 精确伙伴/画布/知识/客服 owner 和资源 | M02-01 伙伴/画布精确绑定与 Skill 锁定向通过；正式入口 UI 待验 |
| D08 | 103 | M02/M05 | 五类 Agent 专属入口/任务；独立产物断言 | M02-01 覆盖 GEN/COD/PAL/MM 的 Session/入口子断言；原生全矩阵待走查 |
| D09 | 375 | M06 | watchdog、setsid/丢失 ownership、sleep/wake、恢复/并发/LONG | 故障边界优先，最后 soak |
| D10 | 33 | M01/M06 | MAC-001～018、PORT；arm64 主 lane，x86 按发布范围 | MAC-005～010/013/015 的本批子断言已验；其余待走查 |
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
