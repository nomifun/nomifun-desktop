# 跨平台共享 Case 处理进度

更新：2026-09-29。调度规则见 [实施计划](IMPLEMENTATION-PLAN.zh.md)。
本表管理 675 个 Both Case 的公共根因；Windows/macOS 产品验收分别记在各自文档。
本轮 S-P0-01～05 已完成共享子断言的走查、修复与回归；不等于 675 条完整 Case 全部通过。
当前优先共享队列，相关 Windows 修复一并完成；共享阶段结算后通知并交付 macOS 接续 prompt，
再推进 Windows 专属队列。各域尚未达到完整验收门槛，当前不声明共享阶段完成。

## 当前 P0 批次

| 任务 | 对应 Case / 本批子断言 | 排查与修复 | 状态 / 验证 |
| --- | --- | --- | --- |
| S-P0-01 清单与固定合同 | REG-001/008/011/017；G0-027；AUTH-001/002 | 官方 seed、Action/effect/resource、Schema/digest 唯一；保留精确 GEN/COD 补项，不扩大条件权限 | 已验证：合同 check、官方 catalog 3/3 |
| S-P0-02 控制协议与整批预检 | G0-003～005/010；CTRL-001/002/006/007 | 采用上游已修复的可选 explanation 与一致执行、顺序控制批次、共享要求证据语义；补整批非法参数反例 | 已验证：Runtime 136/136，保留零 dispatch、幂等与正确完成状态 |
| S-P0-03 exact 产品资源 | AUTH-004/005；APAL-001；AMUL-001 | 新 Canvas Session 复用旧 product binding 时先检查精确目标与资源唯一性；修复创建入口绕过 resolver，保留合法其他资源及旧 Session | 已验证：修复前反例失败；完整 route 36/36 含最终反例通过 |
| S-P0-04 Skill 与动态上下文 | AUTH-008；EXT-006/010/012；CTRL-018/019 | 真实 Skill 正文/digest/来源/依赖/active set；steering 正文与用户账本隔离，恢复不能只用 ID | 已验证：合并后 route 正负向及 Runtime Skill 隔离/恢复测试通过 |
| S-P0-05 owner 边界与错误 | REG-014/015；G0-008/027；OBS-005/016 | 三项 owner 边界检查；Schema 漂移返回既有类型化错误和 expected/actual digest，不再只报泛化字符串 | 已验证：三项检查、digest/未知工具反例通过；不代替 UI 全验收 |

P0 验收命令：`cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`、
`bun run check:process-runtime-boundary`、`bun run check:agent-vocabulary`、
`bun run check:unified-plugin-boundary`。新增缺陷再运行直接关联的确定性测试。
全部日志放在外部 `phase-2-3/2026-09-27/shared-p0/`。

本批结果：Runtime 136、App route 36、official catalog 3 项通过；合同 check、三项边界检查、
desktop-ui-boundary、相关 Rust 格式检查及 diff 检查通过。没有新增付费模型调用。
Canvas 首次 readiness 修复未覆盖创建入口，重新编译后仍失败；补齐实际入口后才通过，
上述中间失败保留在 `canvas-pre-fix-regression.log`、`canvas-post-fix-single-rebuild.log`，
最终证据为 `app-route-and-catalog-final.log` 与 `runtime-merged-final.log`。
合并时上游新增的 steering 测试夹具漏了本地新增字段，已补空值并完整重跑 Runtime。

## 全领域公共队列

下表 Case 集合 = 实施计划对应领域中平台为 Both 的所有 ID；675 条各有且只有一个主领域。

| 队列 | 共享 Case 数 | 测试 → 排查 → 修复任务 | 后续门槛 / 状态 |
| --- | ---: | --- | --- |
| S-D01 | 85 | 工具/模型协议、注册/激活与版本；先找 Schema/admission 断点，再做生产 owner 修复 | P0 本轮；完整传输/故障矩阵待走查 |
| S-D02 | 70 | 控制状态、完成证据、事件和真实体验；保留首次失败，核对 canonical/UI 一致性 | Windows GEN 文件/产物/删除及失败停止场景 N3 通过，旧失败保留；完整控制/观测矩阵待验 |
| S-D03 | 45 | 文件/Artifact 合同、原子边界、source digest、负向隔离 | Windows 新建/既有暂存源等定向回归；macOS APFS identity、Unix 清理与扩展 ACL 保留已有原生验证；完整入口/矩阵待验 |
| S-D04 | 120 | 公共 command/args/cmd Schema、进程 owner 与清理协议 | P0 边界本轮；原生实现转 W01/M01 |
| S-D05 | 27 | Git/SSH 授权与副作用核对；独立 remote/host 夹具 | Unix snapshot 字面路径与 macOS 三类提交前 hook/身份/unknown fence、持久 commit receipt 重放已有定向回归；post-commit、remote/host 条件资源准备后继续，禁止共享生产 remote |
| S-D06 | 67 | Skill/MCP/Plugin/Browser/Computer 的发现、冻结与生命周期 | Skill 本轮；其余条件资源待走查 |
| S-D07 | 53 | 领域 owner/cardinality、跨实例与精确目标绑定 | Canvas/PAL 入口本轮；S-D07-01 修复画布名称上下文，其他平台/完整集合待验 |
| S-D08 | 103 | 五类 Agent 的产品入口与目标能力；逐角色验证，不互相代替 | 首批 Windows 四条路径已有结果；完整矩阵待走查 |
| S-D09 | 75 | 恢复 fence、取消、并发、压缩、预算与长稳；按状态边界注入故障 | P1 故障验证后安排 LONG/soak |
| S-D10 | 15 | PORT-001～015 内部端口、outbox、generation 与外部 grant 隔离 | PORT-012 队列、native rescan、UI 手动/重连对账子断言见 S-D03-21/22/24/25；其余端口及完整恢复待验 |
| S-D11 | 15 | 权限/资源/旧快照/撤权/secret 负向，验证拒绝前无副作用 | 本轮只验关联静态与资源断言；竞态仍待走查 |
| **合计** | **675** | 只统计共享 Case 定义 | 不增加 4,740 个平台结果槽 |

## 历史修复与未关闭项

- S-D04-23（`CMD-149/150`、A02/A08/A13/A17/A19 process host OS 映射子断言）：旧标准工具
  description 在 Runtime 侧写入 host OS，renderer/client OS 不参与，但 Kernel 在 owner dispatch 前
  没有一致性核对；故意错配只能靠实际命令失败暴露，无法记录要求的稳定错误。现让 exec/start 两个
  launch 工具都携带同一 Runtime host 声明；完成 Snapshot、active set 与 binding 校验后、任何 owner
  调用前比较声明与当前执行 host。错配返回 `HOST_OS_COMMAND_MAPPING_ERROR`、
  `status=not_executed`、`user_code_started=false`，并明确不得盲试另一平台；合法 Schema、权限和 Action
  不变。Engine **33 通过 / 1 ignored**、同步远端后 Runtime **187/187**。macOS 正式 Tauri 中，即使 accepted input
  声称客户端为 Windows，首次仍使用字面 `pwd [-P]`，单 effect/零错误且输出为精确 workspace。
  真实异构客户端、远端 owner OS attestation、Windows/Linux 负向正式 UI、N3/100 seed/LONG/99%
  仍开放，不关闭完整 CMD-149/150 或共享 D04。

- S-D04-21（`CMD-139`、`PROC-014/034～037/046`、A01/A02/A11/A13/A17/A19 托管进程
  生命周期子断言）：macOS 正式 Tauri 连续保留了 planner 拆分 handle、阻塞 start、跳过 poll、
  shell/raw PID 替代、取消终态误报，以及模型在合法启动前调用 File/`exec_command` 探查的首次失败；
  后者把字面 `ls -la` 当 executable，虽随后恢复，正向 Case 仍失败。现由 planner 在同一 Agent step
  spec 同时包含 start/poll/cancel-or-close 时持久化 `managed_process_only` 减权标记；Session 只物化
  已继承的 start/poll/input/close/resize/cancel Action，File、exec、VCS、Artifact、发现和委派均不进入
  工具面。Kernel 与 Runtime policy 对 reaped cancellation/terminal poll 使用相同 action-specific
  语义，不再把 `success=false` 的“已不运行”事实误算成命令失败或强制重规划。四层减权回归及
  Runtime/Engine 全组通过；macOS 新 Session 精确 start→poll READY→cancel/reaped、零工具错误、
  2 effects、UI 1/1。Windows、交互 stdin/PTY、20 次/N3、100 seed 与 LONG/99% 仍开放，不关闭共享 D04。

- S-D02-16（`CTRL-006/007`、`CMD-139`、A02/A05/A17/A18/A19 process-chain 完成证据子断言）：
  macOS 首次完成链先因 start/poll 在 cancel 时已不位于顶层可用证据而反复失败，后又把尚无合法 ID 的
  可选 `requirement_ids` 发送为空数组；严格 Schema 正确拒绝，但形成正向 UI bad case。现仅在同一
  process、同一 workspace epoch、零省略交互且 terminal cancel 明确 `cleanup.reaped=true` 时延续该
  chain 的精确 observation；其他旧/失败/跨 epoch 证据仍失效。动态完成 Schema 在尚无 requirement
  ID 时移除该属性，由 owner 收尾时生成完整输入义务；已有 ID 时只暴露精确 enum，显式空数组、未知
  ID、错误类型仍拒绝。Runtime **184/184**；macOS 修复后一次 completion report 成功、4 模型 step、
  零 control error。其他多命令/并发链、Provider/角色/平台及完整 CTRL 矩阵仍开放，不关闭共享 D02。

- S-D02-17（CTRL-006/007、A05/A09/A15/A17/A18/A19 多命令证据关联子断言）：W101 首次正式
  Tauri 中 ALPHA/BETA 两个命令均一次 exit 0/reaped，但第二个 opaque command 推进 workspace epoch
  后，第一条已结算命令从 `available_evidence` 消失。模型能看到两个结果正文，却只得到 BETA 顶层
  call ID 并误认作 ALPHA；连续三次耗尽 4,096 输出 token，Turn 以 `NOMIFUN_TASK_INCOMPLETE` 失败，
  旧 W75 语义关联缺口得到完整复现。现仅将“当时已可用、exit 0、cleanup 已证明、无省略交互”的
  命令自身 scope/exit/output 保持为不可变证据；它不延续任何文件或当前工作区状态。新回归首次
  **0/1**，修复后 **20/20**，Agent Runtime **192/192**。同一 UI 夹具最终 3 步完成、132 条事件，
  两个 criterion 各只引用匹配 call ID，零截断、2 个 returned effect，独立 31 项断言、工作区和清理
  均通过。非零/取消/交互进程链、64 项窗口淘汰、压缩恢复、其他 Provider/角色/平台仍开放，不关闭
  完整 CTRL 或共享阶段。

- S-D03-48（`ART-001/003/007`、`CTRL-007`、`CMD-132`、A01/A05/A17 文字观测与 Artifact
  边界）：macOS 正式 Tauri 的两个独立首发中，`ls -a` 均已成功且模型正确解释，但
  step spec 的“捕获完整输出…业务文件列表”和“在输出中…业务文件”均被旧词法合同
  误判为必须交付持久文件，导致 completed Turn 被翻成不可重试失败。现仅对
  `完整/原始/命令…输出` 与 `在/从/于输出中/里/内` 排除名词用法，并在确定 next-verb 前移除
  token，因此“保存命令输出为文件”仍能看到前一真实动词的 target。“输出一个文件”、
  显式格式和计数仍严格。Agent execution **112/112**；修复后正式 Execution/Attempt
  completed、`output_files=[]`、UI 1/1，旧失败 Session 未重试且保留。完整 Artifact/角色/平台
  矩阵、100 seed 与 LONG/99% 仍开放，不关闭共享 D03。

- S-D03-49（FILE-040、PORT-012、OBS-018、A05/A15/A17/A19 正式 Runtime watcher 接线与
  诊断子断言）：W71 的正式 300 文件样本虽靠模型主动重读得到正确终值，却没有独立 native dropped
  计数。W72 首轮增加有界日志后，两次真实 burst 均没有 watcher 启动/批次记录；`workspace.files`
  已在冻结 Preset 中启用且精确绑定 workspace，说明此前 unit contributor 从未进入正式 Runtime。
  根因是正式 Nomi 走 `EngineKernelSession`，只消费 initial capability/Plugin context；通用 Plugin Tool
  Session 的 lifecycle contributor 并不驱动该 Runtime。现由 `EngineKernelSession` 在 Full scope 且
  精确选择 `workspace.files` 时创建并跨 Turn 持有 `NomiWorkspaceWatchContext`，每轮模型前先消费；
  启动和非空批次只记录 event/dropped/rescan/reconciliation 四项计数，不记录路径或内容。最终正式
  Tauri `46063c30f9dd…` 中，300 文件产生 603 个通知，日志精确为 retained=256、dropped=347、
  rescan=false、requires_reconciliation=true；两回合 completed、43 条事件、零工具调用，磁盘 300
  文件摘要不变，应用/profile 清零并正式备份。Linux 精确测试 **1/1**；同步远端至
  `0fee0eda69d6…` 后正式 Tauri 重建 `7329a6f0dda9…` 亦通过，watcher 源码未变。
  首轮模型命令错误及违背只读要求写入脚本的失败另存并转后续 CMD 子断言；restricted Attempt、
  重复/乱序/rescan UI、其他 Agent/macOS、N3/100 seed/LONG/99% 仍开放，不关闭完整 FILE-040。

- S-D03-47（FILE-040、PORT-012、A05/A15/A17/A19 watcher overflow 全量对账子断言）：W71
  新增跨过 `WatchQueue`、`ContextContributor` 与 canonical JSON 的溢出回归：263 个唯一变更保留
  最后 256 个，首项为 `marker-0007`、`dropped_event_count=7`、`rescan_required=false`，上下文明示
  批次不完整并要求重读，且只消费一次；Windows/Linux 各 **1/1**。现有生产路径首次即通过，本批
  只补回归与正式证据。正式 Tauri `8c54fc75dd29…` 使用隔离 data/work/profile 和 StepFun Plan /
  `step-3.7-flash`，Session 建立后从外部并发创建 300 个文件；下一回合首个工具调用即为完整
  PowerShell 清单重读，随后所有进程/读取调用均成功，最终磁盘/UI/canonical 一致报告 300、
  `event-0001.txt`、`event-0300.txt`。两回合 completed、311 条事件、零工具错误，工作区摘要未变，
  应用/profile 清零并完成官方备份。真实 native 样本的精确 dropped 数未独立落盘；丢批/重复/
  乱序与 rescan 的更多 UI 组合、其他 Agent/平台、100 seed/LONG/99% 仍开放，不关闭完整 FILE-040。

- S-D03-46（FILE-020/025/038、A05/A07/A17/A19 Unix 既有目标派发源窗口子断言）：W70 在
  Linux ext4 上把同名源替换精确放到暂存 inode/字节句柄核对之后、实际 `rename` 之前。系统调用
  因此把外来 7 字节对象放入目标，原意 8 字节暂存被保留到独立名称；发布后核对正确返回
  `outcome_unknown`，但旧代码仍以 rename 成功等同“自有暂存已消费”，错误记录
  `temporary_cleanup_unconfirmed=false`。首版弱断言假通过与收紧后的首次 FAIL 都保留。现改为只有
  目标仍匹配暂存 inode 才确认消费；身份不符时保留 unknown、无 publication identity、要求重读，
  同时显式标记临时清理未确认且不再按已复用名称清理。新增 Linux 回归 **20/20**，Linux service
  **126/126**，Windows `nomifun-file` lib **362/362**，workspace fmt 通过。该窗口的零副作用补偿、
  失败清理 check→unlink、macOS 原生执行、完整 IO fault/UI/角色与长期门槛仍开放，不关闭完整
  FILE Case 或共享阶段。

- S-D03-45（FILE-019/028、A05/A13/A17/A19 Unix 新建发布清理窗口子断言）：W69 在 WSL2
  Ubuntu ext4 夹具中把并发替换精确放到暂存 inode 身份确认之后、`unlink` 之前；旧 hard-link
  发布先正确生成目标，随后删除了占用原暂存名的外来文件，并仍返回成功。首次 FAIL、ext4 现场和
  runner 夹具错误分别保留。Linux/macOS 新建发布现复用共享原生 no-replace rename：Linux
  `renameat2(RENAME_NOREPLACE)`、macOS `renamex_np(RENAME_EXCL)` 在同一原子操作中完成“不覆盖
  目标”和消费暂存名，不再经过按路径清理窗口；既有目标、权限与内容核对保持不变。新增 Linux
  反例修复后 **20/20**，Linux service **125/125**，Windows `nomifun-file` lib **362/362**，
  workspace fmt 通过。Windows 发布路径未改变；macOS 原生执行、其他 Unix fallback、失败路径的
  身份检查→unlink、既有目标 verify→rename、完整 UI/角色与长期门槛仍开放，不关闭完整 FILE Case。

- S-D03-43（FILE-020/022/024/031/032/036、A02/A05/A14/A19 macOS APFS identity 与 Unix
  目标权限子断言）：Unix 批量 patch 只按词法路径判重，APFS case 或 NFC/NFD 等价目标会先发布
  第 1 项再在第 2 项冲突；POSIX rename 又能在父目录可写时替换 mode/ACL 只读目标。现由 macOS
  卷 `_PC_CASE_SENSITIVE` + canonical Unicode component key 在准备期拒绝等价/祖先目标；合法
  Case-sensitive APFS case 变体保持区分。Unix 替换前持有同 dev/inode 的可写目标句柄，权限拒绝
  保持旧字节并清理暂存。macOS 主卷/Case-sensitive APFS 新回归各 **20/20**，`nomifun-file`
  **464/464**；Linux、更多 Unicode case-fold、ACL 竞态、IO fault、正式 UI/角色及长期门槛仍开放，
  不关闭完整 FILE Case 或共享阶段。

- S-D03-44（FILE-013/015、A15/A17/A19 广搜后的 instruction lookup 资源子断言）：W65 正式
  Tauri 基线在独立 workspace 外部写入 400 个同标记文件；模型以 `search_files(limit=200)` 对账时，
  Runtime 会对每个唯一命中再做精确 instruction-scope lookup。首个搜索因此产生 **100** 次内部读取，
  整个回合累计 **415** 次内部读取、**2,313** 条 canonical 事件；最终虽正确报告 400、首尾文件且
  磁盘未变，但过程显示工具错误并用了 11 个模型步，首次 FAIL/RECOVERED 保留，不计正向 Case 通过。
  搜索结果进入模型前现独立限制最多 64 个唯一 instruction hit path；公开搜索上限和 owner 扫描不变，
  超界搜索已执行但 snippets 明确 withheld，且在逐命中读取前停止，不增加 Action、授权或 ambient IO。
  64 项接受/65 项拒绝回归在 Windows/Linux 各 **20/20**，W65 源码同步远端前两平台 Runtime
  各 **171/171**；同步远端 `b046314cd` 后 Windows 仍 **171/171**，Linux 在用例启动前被该远端
  Unix 文件新增但未导入 `Path` 的编译错误阻断，首次失败保留并单独转 W66，不混入本批修复。
  修复版正式样本使用相同 400 文件内容摘要，首个搜索内部读取从 100 降至 5（只含固定根/AGENTS
  预检），总内部读取 **68**、canonical 事件 **664**，最终磁盘/UI/canonical 仍一致。该单样本仍有
  可见命令/完成报告错误并用了 16 个模型步，只证明资源 fan-out 已有硬界；完整搜索 UX、真实 native
  dropped 数独立观测、其他 Agent/macOS、100 seed/LONG/99% 仍开放，不关闭完整 FILE Case 或共享阶段。

- S-D03-42（FILE-040、PORT-012、A08/A14/A15/A17 workspace rescan/dropped 上下文组合子断言）：
  既有 watcher 队列会分别记录已知丢弃数与原生 rescan 标记，但缺少跨过 pre-turn contributor 和
  system prompt 合并边界的证明。W64 新增组合回归，同时注入一个合法相对事件、一个被拒绝路径和
  native rescan；合并后的 canonical JSON 保留 `dropped_event_count=1`、`rescan_required=true` 与
  合法事件，并明确要求重读相关状态、不得据此推断其他路径未变化。绝对 workspace root 不进入提示，
  contributor 仍一次消费。现有生产路径直接通过，本批只补回归，未扩大 Action、工具或权限。
  Linux/Windows 各有 watcher 上下文 **12/12**，新增组合用例各 **20/20**；两平台 Cargo 单项也
  各 **1/1**。WSL 首次通用命令在已通过 lib 用例后误启动零匹配 integration binary，因共享构建目录
  混入异平台产物触发 loader 失败；首次 `--exact` 又因过滤名不完整执行 0 项；Windows 首次切回编译
  在 `nomifun-db` 发生 rustc 栈溢出。三项 runner/编译失败均保留，修正命令或提高编译线程栈后通过。
  正式 UI 全量对账、模型是否遵循重读提醒、完整丢批/重复/乱序、macOS、100 seed/LONG/99% 仍开放，
  不关闭完整 FILE-040、PORT-012 或共享阶段。

- S-D04-01（PROC-011/012/014、A11/A13 Unix 确定 pre-spawn 子断言）：显式 Unix executable
  不存在或无 X_OK 时，旧路径先创建 watchdog，再在 exec/wrapper 失败后共用 setup deadline 回收，
  压力下可从确定“未启动”退化为 `start_lost`；macOS 产品 owner 还未启用已有 Seatbelt 策略。
  现将显式路径诊断及 sandbox 环境准备移到任何物理 authority 之前，bare PATH 名继续走真实
  exec 并保留 ABORT/watchdog 回收反例；Engine 只把确定的 pre-spawn 类型标为
  `user_code_not_started`。macOS 产品接线与原生结果见 M01-01；Linux/Windows 行为未据此代判。
  macOS `nomi-process-runtime` 全组 **243/243**，显式路径及进程/PTY 压力回归通过；正式 UI、
  其他平台、完整 PROC/CMD/LONG 门槛仍开放，不关闭共享 D04。

- S-D04-02（PROC-011/012/014、A19 Unix 构建可移植性子断言）：同步远端 S-D04-01 后，新增的
  `validate_explicit_unix_program` 在所有 Unix 编译，却仍把 `Path` 导入限定为 macOS，Linux 因
  `cannot find type Path` 在任何 W65 用例前失败。W66 将 `Path` 改为 Unix 无条件导入，仅保留
  `PathBuf` 为 macOS 专用；没有改变 spawn、sandbox、权限或清理逻辑。Linux process Runtime 完整
  **245/245**，Agent Runtime **171/171**；Windows process Runtime lib **120/120**。首次 Linux
  完整组另有一次 PTY parent-death reap 时序失败，原样保留；定向复跑 **1/1**、随后 **20/20**，
  最终完整组通过，未加 sleep 或放宽断言。macOS 结果仍以原 M01-01 为准；正式 UI、更多 Unix
  发行版/架构与 LONG 门槛仍开放，不关闭完整 PROC Case 或共享阶段。

- S-D04-03（CMD-135/150、A08/A17/A19 host shell Schema 子断言）：W65 的 Windows 正式样本
  在 broad search 被有界 withheld 后，首个进程调用提交 `cmd: "dir /b /s burst"`；`cmd` 实际由
  PowerShell 执行，因而产生可见失败。顶层工具说明虽带 host OS，但共享基础句先给出 Unix
  `cmd=ls -la` 示例，字段级 Schema 又只写泛化 host shell。W67 首次回归确认 `cmd` 字段没有
  PowerShell 事实。现移除跨平台基础说明中的 Unix 示例，并在字段层明确 Windows 为 PowerShell、
  禁止直接使用 `dir /b`/`dir /s`，Command Prompt 必须显式 `command=cmd.exe` + 分离 args；Unix
  字段明确 `/bin/sh -c`。合法输入集合、Action、权限和 owner 执行不变。Windows/Linux Runtime
  各 **172/172**，新增平台 Schema 回归各 **20/20**。正式 Tauri `57515a20e31c…` 复验同类 400
  文件场景：首个及后续三个进程调用均使用 PowerShell 且零进程错误，最终磁盘/UI/canonical 一致；
  对账回合为 5 个模型步、264 条事件。broad-search withheld 仍是可见错误，因此仅关闭 host shell
  映射子断言；macOS/Linux 真实模型、完整 CMD-135/150、N3/100 seed/LONG/99% 仍开放。

- S-D04-04（`CMD-132/146/148`、`PROC-006/014`、A01/A11/A13 model-facing 字面 argv
  子断言）：process host 文案已要求 macOS 使用 `command`/`args`，但本批基线 JSON Schema
  反而把 `cmd` 标成“Preferred”，首个真实 StepFun 轨迹因而发出 `cmd:"ls -a"`。
  同步 S-D04-03 后进一步将普通单 executable 明确设为 `command` + 字面 `args`
  首选，`cmd` 只在 pipeline、redirection、globbing、compound syntax 或 shell script
  需要时使用；Windows PowerShell/Command Prompt 禁用语义与 Unix `/bin/sh -c` 字段级事实
  全部保留。Agent runtime **173/173**。修复后两个新真实轨迹均首次形成
  `{"command":"ls","args":["-a"]}`；最终样本恰好 1 个 process effect、退出 0、
  `reaped=true`、完整隐藏项且 `visible_failure_count=0`。异构 client/host 和故意错配
  `HOST_OS_COMMAND_MAPPING_ERROR`（`CMD-149/150`）未在本批执行，不关闭完整 CMD/PROC 或共享 D04。

- S-D04-05（CMD-135/150、PROC-014、A08/A13/A17/A19 Windows PowerShell 版本与调用形状
  子断言）：W72 首次正式计数探针先把 `powershell.exe -Command ...` 整段再次放进 PowerShell
  `cmd`，变量被外层 shell 展开而失败；后续改用字面 argv 后又提交 PowerShell 7 的 `??`，而
  Windows owner 固定调用 System32 WindowsPowerShell v1.0（5.1）。同步 S-D04-04 后，首次 Schema
  回归仍因没有 `Windows PowerShell 5.1` 事实而失败并保留。现明确 `cmd` 字符串已由 5.1 直接执行、
  不得再加 `powershell.exe`/`pwsh` 前缀；显式 executable 必须使用 `command=powershell.exe` 与分离
  `args`，且不得假定 `??`、`??=`、三元 `? :`、`&&`、`||` 等 PowerShell 7 语法。合法参数、权限和
  owner 执行均未改变。精确回归 **20/20**、Agent Runtime **173/173**；正式 Tauri
  `90a0d26b79b6…` 在隔离 workspace/data/profile 中首次形成字面 `powershell.exe` argv，唯一进程
  退出 0 且 `reaped=true`，正确返回 300、`ps51-0001.txt`、`ps51-0300.txt`。单回合 completed、
  2 个模型步、67 条事件、仅一项 process effect；300 文件摘要不变，应用/profile 清零并正式备份。
  `cmd` 直接脚本、`start_process`、N3/100 seed/LONG/99% 及 W72 的只读写入/完成参数失败仍开放。

- S-D04-06（CMD-137、A08/A11/A17/A19 accepted-input 只读工具面子断言）：W72 的正式只读
  计数任务仍向模型暴露文件写入工具，模型随后创建 `check_marker.ps1`；W74 新回归首次确认明确
  “不要修改任何文件”后 `write_file` 仍可见并失败，均原样保留。Runtime 现仅从明确的中英文
  accepted input 派生可撤销只读策略：每个模型步重新物化冻结工具面后，隐藏 workspace file、VCS、
  Artifact 的非 ReadOnly binding，保留读取与 process；同时要求 process 保持内联只读，禁止重定向、
  变更命令和临时脚本。后续 accepted input 明确允许修改时恢复原冻结工具，不新增 Action 或权限。
  保守解析与工具面恢复回归通过，核心收窄回归 **20/20**，Agent Runtime **175/175**。正式 Tauri
  `10c052e32ac6…` 中零 workspace mutation tool call、仅一个 returned process effect，300 文件及
  tree hash 不变、无额外路径，应用/profile 清零并正式备份；但首次 `exec_command.args` 被模型编码
  为 JSON 字符串，UI 保留 1 项可见异常，第二步修成数组后才完成（3 个模型步、87 条事件），因此
  不计零失败 CMD PASS。收紧撤销短语后的正式重建 `baec5c5f06e2…` 通过。opaque process 的内核级
  只读隔离、正式撤销 UI、参数字符串失败、其他 Agent/平台及 N3/100 seed/LONG/99% 仍开放。

- S-D05-04（`VCS-006/008/014`、`AUTH-009/013` 的 commit message hook 子断言）：macOS 首败确认
  仅配置的可执行 `commit-msg` 被 libgit2 commit 完全绕过。现按 `pre-commit → prepare-commit-msg →
  commit-msg` 顺序支持 default/`core.hooksPath`，以字面 `/usr/bin/git hook run` argv 进入共享 supervisor、
  30 秒 deadline、完整进程树回收与精确 repo Seatbelt。消息文件位于绑定 Git metadata 内，保留打开的
  descriptor/dev/inode，拒绝置换/symlink 读取，并限制 64 KiB、UTF-8、512 字符；hook 后重新核对 HEAD
  parent 与 staged path membership，漂移或后续失败结算 external-unknown，资源 fence 不释放。错误原因
  有界脱敏但保留类别。拒绝/脱敏、相对 hooksPath 顺序/改写、identity 拒绝、index 漂移四条各
  **20/20**，相邻 VCS **20/20**。`post-commit`、hook 文件置换竞态、fault injection、linked worktree、
  非 macOS 与正式 UI 仍开放，不关闭完整 VCS/AUTH 或共享 D05。

- S-D05-03（`VCS-009/014` 的持久 commit receipt、HEAD/tree 与用户本地编辑隔离子断言）：macOS
  真实成功 `pre-commit` 后创建 commit，消费端故意丢弃返回；确认 effect 已持久为 `returned` 后销毁
  host，追加用户 worktree 修改并以原 Session Store 重建。同 key 调用只返回原 receipt，不再执行
  hook 或 libgit2 commit；HEAD OID、tree OID、commit count、committed blob 和 terminal event identity
  均保持，后写本地修改未被覆盖。首次即通过，精确回归 **20/20**，相邻 VCS **16/16**，无需产品
  代码修复。physical commit 与 terminal receipt 之间的崩溃/存储失败窗口仍只有 pending fence，尚缺
  commit-intent 对账；push result-loss、正式 UI、其他平台及长期门槛仍开放，不关闭完整 VCS-009/014
  或共享 D05。

- S-D05-02（`VCS-006/008`，以及 `VCS-009/014`、`AUTH-009/013` 的 hook/unknown/scope/脱敏
  子断言）：macOS 首败确认 libgit2 commit 绕过可执行 `pre-commit`，hook `exit 7` 后仍创建提交。
  现仅在 macOS 检出 default/`core.hooksPath` 的可执行 hook，执行前核对 exact repo、staged scope 与
  identity，再以字面 `/usr/bin/git hook run --ignore-missing pre-commit` argv 交给共享进程 supervisor；
  30 秒 deadline、完整进程树回收和精确 repo Seatbelt 写根同时生效，返回后再次核对 index/scope。
  repo 根不等于绑定 workspace 时拒绝执行 hook；输出有界且 secret 脱敏。
- hook 拒绝/启动失败/超时可能已修改 workspace，故 commit effect 使用既有
  `ExternalUncertainEffect`：写入 terminal unknown 后保留唯一资源 fence，同 key 与新 key 均不能
  自动物理重放。拒绝/脱敏/fence、Seatbelt 越界拒绝、identity 拒绝各 **20/20**，相邻 VCS
  **15/15**、direct-program/shell **3/3**。首次误用信任 TMPDIR 的隔离夹具及 ManagedEffect 不允许
  uncertain 结算的中间失败均保留。其他三类 commit hook、commit 成功后 result-loss 对账、hook
  fault injection、非 macOS 平台和正式 UI 仍开放，不关闭完整 VCS/AUTH 或共享 D05。

- S-D05-01（VCS-004/005、A05/A14/A17 snapshot 字面路径子断言）：W62 Linux 全组在
  `file\\1.txt` 的 discard 稳定失败；libgit2 checkout 即使禁用 pathspec 匹配，仍把反斜杠按路径
  分隔处理并返回成功，字面文件保持 staged。W63 诊断补出精确文件名；首版 one-entry 内存 index
  checkout 又在 Windows 删除邻居 index/worktree，两次中间 FAIL 均保留并撤回该方案。最终仅对
  Unix 含反斜杠的已验证 Git index 路径直接读取 HEAD blob，以同目录唯一临时名原子发布；普通
  路径继续使用 libgit2。tracked executable mode 与 symlink target 同时保留，邻居 index/worktree
  不变。Linux 两项关键回归各 **20/20**、snapshot **51/51**，`nomifun-file` 全组 **460/460**；
  Windows snapshot **50/50**。无 renderer/模型/UI 改动；macOS、非 UTF-8 Git path、更多 filemode/
  并发父目录竞态及完整 VCS/角色/长期门槛仍开放，不关闭完整 VCS Case。

- S-D03-41（FILE-040、A05/A17 Linux 删除事件归约子断言）：W61 Linux 全组唯一剩余失败
  是单文件删除通知稳定超时。W62 原生轨迹确认 inotify/notify 先发 `Modify(Metadata)`，紧接同路径
  `Remove(File)`；旧 debounce 只按 path，前者把真正删除抑制 200 ms，而 UI sink 有意忽略 metadata
  change。现以 `event_type + path` 作为去重身份，同类抖动仍合并，不同事实不互相吞掉。Linux
  纯归约与真实 ext4 删除两项各 **20/20**，Linux lib **304/304**、file_watching **14/14**；Windows
  file_watching **14/14** 及真实删除回归通过。Linux 全 crate 随后在无关 snapshot literal-path
  用例稳定失败并转 W63，不计 watcher 失败。无 renderer/模型/UI 改动；macOS、事件洪泛/乱序及
  完整 UI/长期门槛仍开放，不关闭完整 FILE-040。

- S-D03-40（ART-001/003/007、A05/A13 Unix Artifact 目录 durability 子断言）：W60 的 Linux
  全组中 11 项 Artifact 均在目录同步返回 EBADF；`cap_std::Dir` 可持有只用于 capability traversal
  的 `O_PATH` descriptor，旧实现 clone 后直接 `fsync`。W61 在同一已授权 Dir 内以 `"."` 重新打开
  可读目录句柄并同步，不做 ambient 路径解析，也不忽略其他 sync 错误。新增 Linux 目录同步回归
  **20/20**，Linux Artifact **18/18**，Windows Artifact **15/15**。Linux 全 crate 从 12 项失败
  收敛为 1 项已有 watcher 时序，最终 302/303；该项定向复核仍失败，转 W62，不计 Artifact 失败。
  macOS 主 lane、真实磁盘故障/断电 durability、完整 Agent/UI/角色及长期门槛仍开放，不关闭完整
  Artifact Case 或共享阶段。

- S-D03-39（FILE-019/020/025/028/038/039、A05/A07/A13 Unix 暂存源/清理子断言）：W60
  在 WSL2 Ubuntu 6.18 的 `/home` ext4 临时目录执行原生 Linux 夹具；源码从 `/mnt/c` 挂载，
  文件语义断言仅作用于 ext4 夹具，未把 drvfs 当 Unix 文件系统。最终核对后把暂存原地改写或
  换成同字节异 inode，旧 Unix 路径均返回成功并发布错误对象；两项首次 FAIL 及 ext4 现场复制到
  Windows 外部证据目录。现在所有 Unix
  暂存都保留 inode 身份；rename/hard-link 前以 `O_NOFOLLOW` 打开并核对 inode/调用字节，权限按
  已核对句柄继承，发布后再次核对目标身份/字节。失败清理仅删除仍属于该 operation 的 inode；
  外来同名文件保留并将 cleanup 记为未确认。新建分支同样覆盖，三项 Linux 回归各 **20/20**，
  Unix `service::tests` **124/124**；Windows `nomifun-file` **555/555**。
  Linux 全 crate 另保留 12 项非本批失败：11 项 WSL artifact 目录 fsync 返回 EBADF、1 项已有
  native watcher 时序，不能据此宣称 Linux 全组通过。身份核对到 rename、身份检查到 unlink 的
  最窄路径窗口、macOS 主 lane、完整 IO fault/UI/角色及长期门槛仍开放，不关闭完整 FILE Case。

- S-D03-38（FILE-040、AUTH-005、A09/A13/A17 watcher 尾事件子断言）：W59 把旧 native
  注册已排队的单文件事件延迟到同路径重订后分发，旧全局 callback 会按当前 path→owner 映射误投
  新 owner；首版夹具用非 canonical event path 未击中映射，假通过另存，纠正后首次产品 FAIL
  保留。单文件 watcher 现按 canonical path 分注册实例，各自捕获独立 owner fence 与 debounce；
  Office 注册也改用同一 fence。回调取得 delivery token 后才复制 owner，停止先移除 owner 并等待
  已开始发送结算，再撤销原生注册；等待期间不持注册表锁，event sink 可重入。旧注册 callback
  只看到已退休 owner 集，新注册不继承尾事件或 debounce；共享同路径的其他 owner 保留。
  `nomifun-file` **555/555**，单文件迟到、Office 迟到及发送中停止三项各 **20/20**，真实 native
  创建/删除/重订与 14 项集成回归通过。无 renderer/模型/UI 改动。活动注册容量、native callback
  永不返回、完整 rescan/dropped/乱序 UI、其他平台及长期门槛仍开放，不关闭完整 FILE-040。

- S-D03-37（FILE-040、A05/A17/A19 未监听外部变更子断言）：W58 首次反例证明完成的扁平
  清单缓存会永久遮蔽未启动 watcher 时的外部创建；显式第二次 `/api/fs/list` 仍只返回旧文件。
  现将完成清单限定为通知/活动读取的一致性快照：新的显式 API/Agent 读取在无人使用时退休旧
  快照并重新扫描；活动读者、扫描中失效、旧扫描不得覆盖新扫描及不同根隔离保持。外部创建和
  删除的直接 owner 与真实 Axum route 回归通过，三项关键回归各 **20/20**，`nomifun-file`
  **552/552**。一次无关的原生 watcher 三秒超时保留，未加 sleep；定向复跑及最终完整组通过。
  正式 Tauri `@文件` 菜单先读到 `old.txt`，随后由外部 PowerShell 创建 `new.txt`、删除旧文件；
  关闭重开后 `@new` 只显示新候选、`@old` 显示空结果。四次 `/api/fs/list` 均 200，watch route
  调用为 0；冻结二进制 `fcc93ec68118…`，1 模型步、0 effects，磁盘 hash/UI/canonical 一致，
  应用/profile 清零并正式备份。单次无 watcher 扫描期间的外部竞态、完整丢批/乱序、其他角色/
  平台及长期门槛仍开放，不关闭完整 FILE-040 或共享阶段。

- S-D03-36（FILE-020/025/038/039、A05/A07/A17 原生替换源间隙子断言）：W57 在最终
  暂存核对与 `ReplaceFileW` 之间分别换入同字节异身份对象、原地改写暂存对象；两项首次
  FAIL 均返回 unknown，但目标已被外来/错误字节占据。Windows 现紧邻原生调用记录实际派发源的
  卷/128 位文件 ID；返回后的目标仍是该对象却不匹配预期 owner/字节时，先按句柄无覆盖地退回
  原暂存名，再以原件身份句柄恢复备份。补偿仍保留 outcome_unknown，不把短暂发布改称零效果，
  也不允许盲重放；同字节外来源和被改写的自有暂存均恢复原目标，外来对象不删除。若调用后目标
  又被并发对象接管，身份不符则不补偿，保留并发目标、原件备份和待核对状态。另补 capture 后、
  调用前再次换源的最窄窗口，确认不会错误补偿，仍保持 unknown。`nomifun-file` **551/551**，
  5 项相关回归各 **20/20**；无 renderer/模型/UI 改动。身份 capture 与系统调用之间的最窄窗口、
  Unix/macOS 清理与发布、100 seed、完整角色/UI 和长期门槛仍开放，不关闭完整 FILE Case。

- S-D02-13（CTRL-007、FILE-031/039、A17/A18 失败即停与部分结果子断言）：W51 用 W45
  冻结构建、相同 StepFun Plan / step-3.7-flash、提示和三套隔离目录重验 W42 场景，首轮仍仅
  **1/3**；两次失败分别在参数错误或真实部分发布错误后继续读/补丁，43 模型步、6 effects，
  原失败均保留。Runtime 现只在 accepted user input 同时明确“错误即停”和“禁止重试”时启用
  failure-stop；首个工具失败后仅暴露 `report_completion`，要求 blocked 部分结果，不能重读、
  查历史、重规划或再做 effect。`apply_patch` Schema/说明补一行替换的精确 remove+add 形状，
  避免把 context+add 当替换。W53 首次正式复验又保留 blocked 报告误填 `evidence_paths`、摘要
  未点名两个目标的 FAIL；门禁随后注入待恢复目标和最小报告形状，blocked 条目不得伪造证据。
  最终 Runtime **169/169**，相关 4 项回归各 **20/20**，正式 Tauri 构建通过。W56 冻结二进制
  `e2fc6f5e17ac…`、GEN Revision、route/preset 和原提示，三个新 Session 均为 4 模型步、一次
  guarded patch、一次 rejected effect；错误后零文件/进程动作，部分回执明确 `first.txt` 已发布、
  `second.txt` 失败，canonical/disk/UI 独立断言全部通过。总计 12 模型步、3 effects，应用、helper、
  profile 清零并正式备份，**修复后固定样本 N3=3/3**。W42/W51/W53 的旧失败不改写；其他角色、
  普通完成分支的完整矩阵、Unix/macOS、100 seed、LONG/99% 及共享阶段仍未关闭。

- S-D02-14（FILE-015、A08/A15/A17/A19 有界搜索 UI 分类子断言）：W67 正式 Tauri 样本中，
  `search_files` 按既有资源上限返回精确 `search_context_withheld` 后，Runtime 正确要求模型完整
  核对，但 UI 把该预期有界结果汇总为“1 项操作出现异常”；首次可见失败和 canonical 结果均保留。
  W68 只对本地 `search_files`、`status=error`、四个精确字段且布尔值成立的结果增加
  `boundedResult` 分类；canonical `is_error=true`、模型重规划和可展开详情不变。格式错误、多字段、
  远端或其他工具错误继续按失败处理。Turn 回执不再把该结果计入普通错误数，改用琥珀色“搜索结果
  受限，需要完整核对”。三组定向 UI 回归 **72/72**，typecheck、i18n parity 与桌面 UI 边界通过。
  正式 Tauri `a1d254ff1c18…` 使用隔离 data/work/profile 与 StepFun Plan / `step-3.7-flash` 重验
  400 文件 burst：canonical 仍是有界错误，随后两个 PowerShell 只读调用成功，最终磁盘/UI/
  canonical 一致报告 400、`marker-0001.txt`、`marker-0400.txt`；工作区摘要未变，两回合 completed、
  214 条事件，应用/profile 清零并完成官方备份。其他有界结果、其他 Agent/平台、完整 FILE-015、
  100 seed/LONG/99% 仍开放，不关闭共享阶段。

- S-D02-15（CTRL-006/007、A05/A07/A17/A18/A19 多命令完成证据子断言）：W75 两条独立
  process 均成功，但 completion 的 alpha criterion 复用最新 beta call ID；该 ID 不能证明 alpha。
  W76 首版只要求匹配 ID，又与既有保守合同冲突：后续 process 启动会使前一 command observation
  退出 `available_evidence`，模型在不可满足约束中反复推演，6 个模型步后续写预算耗尽并 failed。
  两次首次失败均保留。最终说明要求仅引用当前 `available_evidence` 中的匹配 ID；若先前调用已不可用，
  必须用无证据的 `unverified`，不得借用最新 ID、加载历史或未经授权重跑。证据有效期、validator 与
  权限均未放宽。精确回归 **20/20**、Agent Runtime **175/175**。正式 Tauri `c90c28c56c46…`
  中两进程成功/reaped，completion dispositions 为 `[unverified,supported,unverified]`，证据为
  `[[],[zeta_call],[]]`；最终正确报告 epsilon=3、zeta=8、合计 11 并披露 epsilon/合计未验证，
  单回合 completed、5 个模型步、140 条事件、UI 零工具异常，11 文件摘要不变，应用/profile 清零
  并正式备份。多条顺序 process 结果要全部 supported 仍需可证明只读的 owner 合同或原子聚合；
  其他场景/Provider/角色/平台、N3/100 seed/LONG/99% 仍开放，不关闭完整 CTRL 或共享阶段。

- S-D04-07（CMD-139、PROC-033、A13/A17/A19 托管进程显式取消与来源子断言）：W77 首个正式
  `start_process → poll_process → cancel_process` 样本已得到 `state=cancelled`、
  `cleanup.reaped=true`，宿主却把所有非 `Exited` 终态统一写成 `success=false`，因此成功清理在
  canonical/UI 中显示为工具错误。首版宿主修复后，Runtime 又把同一结果计为
  `failed_commands=1` 且 completion observation 不可用，完成 Schema 收窄为
  `evidence_call_ids.maxItems=0`；继续修复后，零错误样本还暴露终态 command provenance 未携带
  中间 poll call ID。三层首次失败、截图、事件、数据库与模型轨迹均保留在仓库外。
  现仅对显式 `cancel` 且 `reaped=true` 返回成功；未回收或 `exec` 被中断仍失败，并保留
  `cleanup.errors`。Runtime 不再把成功的显式取消计成失败命令，也不把它冒充正常退出/测试成功；
  cancel observation 可作为取消/清理证据，但 `was_current_at_observation=false`。同一进程的 poll
  call ID 进入有界 interaction provenance，且不会推进 workspace effect epoch；完成说明另明确嵌套
  launch/interaction ID 只有同时出现在顶层 `available_evidence` 时才可直接引用。
  宿主精确回归 **20/20**，两项 Runtime 回归每项 **20/20**，Agent Runtime **177/177**，workspace
  fmt 与正式构建通过。最终 Tauri `64d2f85813a8…` 使用隔离 data/work/profile 与 StepFun Plan /
  `step-3.7-flash`：start、两次 poll、cancel、report_completion 五次调用全部成功；cancel 为
  `success=true`、`reaped=true`，Windows 无 console interrupt 合同的诊断仍保留。单回合 completed、
  5 个模型步、166 条事件，`failed_commands=0`、`failed_tools=0`；可用 cancel observation 精确携带
  launch ID 与两条 poll ID，完成报告一次通过。工作区摘要不变、helper 为 0、应用/profile 清零并
  正式备份。cancel/自然退出真实竞态、忽略温和中断后的升级与后代清理、PTY、其他平台/角色、
  N3/100 seed/LONG/99% 仍开放，不关闭完整 CMD/PROC 或共享阶段。

- S-D04-08（PROC-038、A13/A17/A19 cancel/自然退出线性化子断言）：W78 在暂停时间的 owner
  夹具中固定两种竞争顺序。cancel 先取得 stop ownership、进程随后自然 reap 时，唯一终态保持
  `Cancelled`；自然 reap fact 先发布、仍处于最终输出 drain 时再 cancel，唯一终态保持 `Exited`。
  两种顺序的重复 cancel 与终态 poll 都逐字段等于首次结果，`cleanup.reaped=true`，底层 wait/reap
  恰好一次；自然退出先胜时零 signal。生产实现首次即满足，未改清理、信号或权限逻辑，只新增
  两项确定性回归。首次 **2/2**、两项各重复 **20/20**，process Runtime lib **122/122**。
  真实 OS 调度竞争、并发 cancel waiter、child/grandchild、PTY、其他平台和长期压力仍开放，不关闭
  完整 PROC-038 或共享阶段。

- S-D04-09（PROC-049、A13/A17/A19 close/cancel/poll 三方并发子断言）：W79 扩展可控 owner，
  让已通过 admission 的 `close_stdin` 停在异步 owner 调用内，同时启动 60 秒 poll 与 cancel。
  阻塞 close 不持有 registry/session 锁；cancel 仍以一次 interrupt 取得 `Cancelled`、
  `cleanup.reaped=true`，poll 在期限前被唤醒并逐字段得到同一终态，随后释放的 close 独立完成。
  底层 close/interrupt/wait 各一次。生产实现首次即满足，本批只补回归与测试夹具。首次 **1/1**、
  重复 **20/20**，process Runtime lib **123/123**。真实 pipe/PTY stdin 关闭、多个 cancel/poll waiter、
  平台 I/O 失败与应用退出仍开放，不关闭完整 PROC-049 或共享阶段。

- S-D04-10（PROC-038、A13/A17/A19 并发 cancel follower 子断言）：W80 用 barrier 同时释放 8 个
  cancel waiter，只有一个调用成为清理 leader，其余 follower 等待同一 terminal。八个结果逐字段
  相等且均为已回收 `Cancelled`，只发送一次 interrupt、只执行一次 wait/reap。生产实现首次满足，
  本批仅新增回归。首次 **1/1**，8 waiter 场景重复 **20/20**，process Runtime lib **124/124**。
  真实 OS helper、多 Session 混合、cancel future drop 与平台信号失败组合、长期压力仍开放，不关闭
  完整 PROC-038 或共享阶段。

- S-D03-50（FILE-019/020/028/038、A05/A07/A13/A17/A19 Unix 失败清理子断言）：W81 在 WSL2
  ext4 上于失败清理确认 `stage.tmp` inode 后、`remove_file(path)` 前，把该名称换成同字节外来文件。
  旧实现删除外来对象，保留原暂存 inode 到独立名称，却返回
  `temporary_cleanup_unconfirmed=false`；首次失败日志、observation、保留 inode 与缺失外来名均在
  仓库外保留。Unix 没有可信 compare-and-unlink，因此未消费的失败暂存现不再按名称删除，而是保留
  供显式对账并返回 `temporary_cleanup_unconfirmed=true`；上层据此进入 outcome unknown。Windows
  继续使用 handle 绑定删除。修复后精确反例 **20/20**，Linux service **127/127**，Windows
  `nomifun-file` lib **362/362**。保留暂存的后续 owner 对账/清理、macOS 原生执行、其他 Unix
  portable fallback、磁盘/IO fault、完整 UI/角色和长期统计仍开放，不关闭完整 FILE Case 或共享阶段。

- S-D03-51（`FILE-020/022/025/038/039`、`AUTH-006/014`、`MAC-005` 既有目标扩展 ACL 子断言）：
  macOS APFS 原生探针与产品首败确认普通 `rename` 替换 inode，会让成功的 write/patch 丢失目标
  extended ACL；旧实现只复制 POSIX mode，因此 `group:everyone deny execute` 在返回成功后消失。
  现于目标 dev/inode 与 stage identity/bytes 均核对后、最终 pre-publication hook 之后，使用目标/stage
  descriptor 调用 `fcopyfile(COPYFILE_ACL)` 并 `sync_all`，复制失败在 rename 前按权限/内部错误停止，
  不忽略 ACL 错误；Linux/Windows 路径不变。hook 中后加的 `group:staff deny execute` 与原 ACL 一并
  保留。deny-write ACL 保持旧字节/ACL，按 S-D03-50 精确保留两个自有 stage、返回 unknown 并发两个
  无内容对账事件；macOS 旧“必须立即删 stage”断言据此收紧而非放宽。
- 修复后 ACL 成功保留、deny-write、最终 hook 动态 ACL 各 **20/20**；`nomifun-file` lib **308/308**，
  macOS workspace **6 passed / 1 ignored**（既有 opt-in case-sensitive APFS），fmt/diff 通过。APFS 在
  ACL copy 后至 rename 的最后系统调用窗口、xattr/resource fork/flags、非 APFS 卷、IO fault、正式
  UI/角色和长期门槛仍开放，不关闭完整 FILE/AUTH Case 或共享 D03。

- S-D03-52（`FILE-020/025/038/039`、D11 既有目标 xattr/resource-fork 子断言）：M03-06 首轮夹具
  因 `/usr/bin/xattr -p` 展示换行失败，纠正 CLI oracle 后命中真实产品反例：成功 write/patch 返回
  success，却删除 `com.nomifun.reliability.fixture`。S-D03-51 的 descriptor-bound macOS 元数据复制现
  扩展为 `COPYFILE_ACL | COPYFILE_XATTR`，仍位于最终 hook 后、rename 前并执行 `sync_all`；任何复制/
  持久化失败在目标名称改变前停止，不记录真实 xattr 值，Linux/Windows 路径不变。原生断言同时覆盖
  普通 xattr、`com.apple.ResourceFork`，以及 final hook 新增的 before/after xattr。
- 修复后普通 xattr + resource fork **20/20**、最终 hook 动态 xattr **20/20**；`nomifun-file` lib
  **309/309**，macOS workspace **7 passed / 1 ignored**，既有 ACL 回归与 fmt/diff 通过。metadata copy
  后至 rename 的最后窗口、copyfile 故障、超大 resource fork、immutable flags、非 APFS、正式 UI/
  角色及长期门槛仍开放，不关闭完整 FILE Case 或共享 D03。

- S-D04-11（PROC-047、A08/A11/A13/A17/A19 Windows pre-resume deadline 子断言）：W82 用真实
  Windows suspended process，把 Job assignment 延迟 200 ms，并把共享 process deadline 固定为
  50 ms。事务跨期后从未调用 `ResumeThread`，用户 marker 零创建；精确进程句柄在返回前已终止，
  结果为可证明 pre-spawn `SpawnFailed`。首次耗时约 220 ms，未重新获得 5 秒 setup timeout。
  生产实现首次满足，本批只扩展 audit facade 和回归。首次 **1/1**、重复 **20/20**，process Runtime
  lib **125/125**。Unix/macOS 启动阶段、刚过 resume 的 StartLost、清理失败与完整 UI/角色仍开放，
  不关闭完整 PROC-047 或共享阶段。

- S-D04-12（PROC-048、A05/A08/A10/A11/A13/A17/A19 运行中 deadline 部分效果子断言）：W83
  新增跨平台真实 helper，先写入固定 `partial effect before timeout` 文件，再保持运行 60 秒；统一
  deadline 为 1 秒，清理阶段分别为 50/50/500 ms。Windows Job 与 WSL2 Linux process group 均返回
  `TimedOut`、`cleanup.reaped=true`，磁盘保留逐字节一致的部分文件，没有删除或声称回滚。生产实现
  首次满足，本批只补 helper 与回归。两平台各重复 **20/20**；Windows process contract **11/11**，
  Linux **12/12**。持续写入截断量、后代进程、macOS、正式 UI/模型部分效果披露和 IO fault 组合仍
  开放，不关闭完整 PROC-048 或共享阶段。

- S-D04-13（PROC-050、A08/A11/A13/A17/A19 cleanup authority 分类子断言）：W84 复核既有
  两平台故障注入。Windows Job 的首次 member snapshot、supplemental snapshot 与 terminate 失败都
  保留精确 handle/Job authority 并在重试后完成清理；Unix relay 在 group quiescence 未证明时保持
  `CleanupOwned` 且 completion 仍为 Running，只有 ECHILD 的明确身份丢失才隔离，并且隔离前不向
  缓存 PGID 发信号。分类器仅把 `Unsupported` 视为永久 authority lost。生产实现和既有回归首次
  全部通过，本批只补进度证据；Windows 临时 snapshot 代表场景 **20/20**，Unix retry 代表场景
  **20/20**。macOS 真实 relay、跨重启 quarantine、组合故障与长期压力仍开放，不关闭完整
  PROC-050 或共享阶段。

- S-D04-14（PROC-042、A03/A11/A13/A17/A19 并发 start 容量预留子断言）：W85 用 barrier 在公开
  supervisor 上同时释放 32 个真实 helper start，并把容量固定为 2。Windows 与 WSL2 Linux 都恰好
  允许 2 个 helper 写出 PID，另外 30 个在物理 spawn 前返回 `capacity_exhausted`；独立目录中没有
  超额 marker。两个已准入进程均 cancel/reaped 后，新 start 能复用额度。生产实现首次满足，本批只
  新增 helper 与回归。两平台各重复 **20/20**；Windows process contract **12/12**，Linux
  **13/13**。start/shutdown 竞争、Lost/quarantine 占额、PTY、macOS 与 1,000 次长稳仍开放，不关闭
  完整 PROC-042 或共享阶段。

- S-D04-15（CMD-138、A02/A08/A17/A19 最小定向测试子断言）：W86 的隔离 Bun 仓库规定只运行
  `bun test tests/sum.test.ts`；目标测试通过，无关测试带独立失败哨兵。前三次正式 UI 均选对命令，
  但首次模型请求前未装载根 `AGENTS.md`，模型把它与源码当普通读取，Runtime 只能在 dispatch 前
  延迟整批并形成可见异常；只补工具说明及 system instruction 仍复现，第三次另保留显式空
  `requirement_ids` 被拒现场。根因是新 `ScopedInstructions` 初始 `dirty=false`，首个 model boundary
  无法物化已知根 scope。现初始标记为 dirty，在第一次模型请求前注入根指令；说明仍明确 scope
  读取合同，completion 的显式 requirement 数组至少一项，覆盖全部需求时必须省略字段。四项精确
  回归各 **20/20**，Agent Runtime **180/180**。最终正式 Tauri `b982a32a93d6…` 只有 scope 读取、
  一次目标 start、一次 poll 和完成报告，目标 **1 pass/0 fail**、无关哨兵未触发、UI 零异常；工作区
  不变，进程与 profile 清零并正式备份。其他 package manager/语言/仓库、其他 Agent/平台、N3/
  100 seed/LONG/99% 仍开放，不关闭完整 CMD-138 或共享阶段。

- S-D04-16（PROC-043、A03/A08/A11/A13/A19 session lease 子断言）：W87 复核 supervisor 已有
  到期与续期合同。无人 poll 的真实进程输出会续期；owner 认证的 poll、write、status 各自跨过
  原始 lease 后仍保持 Session。无活动 Session 到期后先进入单一 retirement，执行 cancel 并取得
  exact reap，再从 registry 移除；进行中的动作阻止 reaper claim，未 reap 的 retirement 继续占用
  容量。生产实现首次满足，本批未改 lease/清理逻辑。Windows 与 WSL2 Linux 六项精确场景各
  **20/20**（每平台 120 次），`session_registry` 各 **12/12**，两平台 helper 均为 0。首次 WSL
  内联脚本因宿主引号损坏而在测试前退出，作为夹具失败独立保留后改为落盘脚本。PTY/resize/
  close-stdin 的显式续期、macOS、应用强退、1,000 进程长稳与正式 UI/角色仍开放，不关闭完整
  PROC-043/044 或共享阶段。

- S-D04-17（PROC-034～037、A08/A11/A13/A17/A19 process tree 子断言）：W88 复核真实
  Windows Job 与 WSL2 Linux process group。Windows cancel 在无可信 console interrupt 时升级并在
  5 秒合同内精确回收 leader + grandchild；leader 先退出时，Job 后代清理完成前不发布成功。Unix
  普通 cancel 清整组，无视 SIGINT 的整组在完整 grace 后升级 SIGTERM；leader-first 仍保留真实
  exit 0，`setsid` 逃逸则在有界时间返回 `Lost`，未假报 EOF 或 cleanup 成功。逃逸进程由测试 oracle
  单独清理。生产实现首次满足，本批未改信号或 owner 逻辑。Windows 两项各 **20/20**，WSL2 四项
  各 **20/20**；共 120 次且两平台 helper 为 0。ConPTY/PTY、强制 kill 级升级、macOS watchdog/
  session、父进程死亡、应用入口及长期压力仍开放，不关闭完整 PROC-034～041 或共享阶段。

- S-D04-18（PROC-040/041、A03/A11/A13/A17/A19 shutdown/parent-death 子断言）：W89 的真实
  supervisor shutdown 先关闭 start gate，清理两条活动 Session 并按原 owner/session 报告终态，
  shutdown 后的 marker 命令在 spawn 前被拒；Windows 和 WSL2 Linux 各 **20/20**。宿主直接退出
  时，Windows `KILL_ON_JOB_CLOSE` 回收 leader + grandchild，Linux process-group watchdog 与外部
  PTY-session watchdog 均由 subreaper 精确观察并回收；Windows 一项 **20/20**，Linux 两项各
  **20/20**。生产实现首次满足，本批未改 start gate、Job/watchdog 或报告逻辑；共 100 次且两平台
  helper/harness 为 0。真实 Tauri 主进程正常/强退、Windows ConPTY parent-death、macOS、启动事务
  与 shutdown 竞争及长期压力仍开放，不关闭完整 PROC-040/041 或共享阶段。

- S-D04-19（PROC-044、A05/A11/A13/A15/A19 1,000 短进程子断言）：W90 新增默认忽略、显式
  运行的共享 soak 回归；普通套件不承担长耗时。每轮以容量 8 的公开 supervisor 连续启动 1,000
  个真实 helper，逐个要求 exit 0、exact reap、固定 32 字节输出且 dropped=0，最后 shutdown report
  为空并比较宿主资源。首个夹具因 `start` receiver 类型错误编译失败，独立保留后修正为公开
  `Arc<ProcessSupervisor>` 合同。Windows 与 WSL2 Linux 均连续两轮 1,000/1,000 通过；最终 Windows
  178.25 秒，handle 111→111、thread 13→10，Linux 171.79 秒，fd 11→11、thread 8→8，helper
  均为 0。生产逻辑无需修改。并发短进程、PTY/ConPTY、macOS、真实应用/UI 与更长 soak 仍开放，
  不关闭完整 PROC-044 或共享阶段。

- S-D04-20（PROC-047、A08/A11/A13/A17/A19 Unix setup deadline 子断言）：W91 在 WSL2 Linux
  固定三条 pre-exec 窗口。spawn gate 排队 300 ms 时只使用原 100 ms setup budget，返回前 worker
  停止、watchdog 未 fork、用户 marker 不存在；blocking worker 卡在事务入口并越过 75 ms 后，
  保守返回 `StartLost`，释放后仍不能 fork leader/watchdog；watchdog 已建但 ACK 被扣留时，100 ms
  deadline 不叠加第二轮 setup/cleanup，用户代码零执行，exact watchdog 最终只 reap 一次。生产实现
  首次满足，本批未改 Unix 启动逻辑；首轮 **3/3**，三项各重复 **20/20**，残留 owner 进程为 0。
  首次内联 WSL 脚本引号失败发生在测试前并独立保留。macOS、PTY setup、exec 临界点、真实 UI/角色
  与长期调度压力仍开放，不关闭完整 PROC-047 或共享阶段。

- S-D04-22（CMD-143、A02/A08/A17/A18/A19 长历史命令子断言）：W92 首次正式 Tauri 回合最终
  执行了 W92-001～050，但 56 个模型步骤中共有 55 次 `exec_command` 提案；step 1/18/34/36 把
  executable 的 `args` 错挂到 shell-script `cmd` 分支，step 17 又提交空对象。五次均由 Schema 在
  dispatch 前拒绝，实际 effect 仍恰为 50；最终 completion 却把可见失败数写成 0。失败 Session、
  2,499 条事件、截图、数据库与轨迹独立保留。根因是两个输入分支虽严格互斥，模型可见说明缺少
  紧邻字段的完整 JSON 对照；Runtime 又未把已累计的 `failed_tools` 放进 completion 上下文/报告。
  现明确 `{command,args}` 与 `{cmd}` 两种形状，args 只属于 command；有错误的回合必须提交动态
  const `observed_tool_error_count`，Runtime 校验、持久化并固定追加披露，后续成功不能抹掉错误。
  六项精确回归各 **20/20**，合并远端后 Agent Runtime **187/187**；旧夹具首次 **176/182** 失败后按真实累计
  数修正，不放宽合同。合并远端后的最终正式 Tauri `712cf9457e11…` 单回合 51 步、2,454 条事件：前 50 步均为
  独立 `command=cmd.exe` + JSON args，50/50 exit 0/reaped、marker 精确有序，第 51 步完成报告，UI/
  canonical 错误均为 0；工作区不变，进程与 profile 清零并正式备份。其他命令语义、Provider/模型/
  角色/平台、50 个独立用户回合及 N3/100 seed/LONG/99% 仍开放，不关闭完整 CMD-143 或共享阶段。

- S-D04-24（OBS-002、REAL-021～024、A02/A08/A17/A18/A19 工具错误披露子断言）：W93 使用
  W92 最终二进制和新的正式 Tauri 隔离 data/work/profile，按负向协议先提交一次无效
  `exec_command {cmd,args}`，再提交合法 `{command,args}` 恢复调用。首次调用以
  `INVALID_TOOL_ARGUMENTS`、`status=not_executed` 在 dispatch 前拒绝，零 process effect；第二次
  输出精确为 `W93-RECOVERED`、exit 0、`reaped=true`，唯一 process effect 为 returned。模型在第
  3 步一次 `report_completion`，显式提交 `observed_tool_error_count=1`；最终交付保留模型说明并由
  Runtime 固定追加“本回合观察到 1 次工具错误，后续成功未抹除”。单回合 completed、3 个模型步骤、
  98 条 canonical 事件，独立 24 项断言通过；工作区 tree hash 不变，cmd、应用和 profile 进程清零，
  正式备份完成。生产代码无需再改。本批只验证一种参数预检错误及 StepFun Coding Plan /
  `step-3.7-flash`；业务错误、权限/超时/unknown、其他 Provider/模型/角色/平台与长期矩阵仍开放，
  不关闭完整 REAL/OBS 或共享阶段。

- S-D04-25（PROC-015、OBS-004、REAL-004/021～024、A05/A08/A17/A18/A19 命令失败披露
  子断言）：Runtime 原已累计 `failed_commands`，但完成报告只强制披露工具结果错误；一次非零退出
  后恢复成功时，模型可以不申报命令失败。W94 新回归首次按预期失败。现增加动态 exact const
  `observed_command_failure_count`，非零时必须提交、持久化并参与报告新鲜度判断；最终交付固定追加
  命令失败数，后续成功不能抹除。若工具错误和命令失败同时非零，模型说明给出包含全部必填计数的
  单个精确 JSON 对象。首次正式 Tauri 虽正确申报命令失败数，却漏掉同一非零命令产生的工具错误数，
  completion 先被 Schema 拒绝后恢复，保留为 `FAIL_RECOVERED`。同一夹具和模型修复后精确执行
  exit 7、更新计划、恢复命令，并在第 4 步一次报告两个计数均为 1；147 条 canonical 事件、2 个真实
  process effect、工作区和清理结果独立一致。三项新增回归各 **20/20**，Agent Runtime **190/190**，
  正式构建通过。其他非零码/信号、timeout/lost、多个失败、其他 Provider/角色/平台及长期矩阵仍开放，
  不关闭完整 PROC/REAL/OBS 或共享阶段。

- S-D04-26（G0-023、PROC-048、REAL-021～024、A05/A08/A11/A13/A17/A18/A19 运行中 timeout
  披露子断言）：W96 复用 W94 最终二进制，以正式 Tauri 和 `step-3.7-flash` 运行 250 ms deadline
  的真实 Windows pipe 命令。结果为 `timed_out`；无可信 console interrupt 后按合同升级 terminate，
  1,174 ms 内 `reaped=true`，未使用 force kill。模型随后单独更新计划、执行恢复命令，并在第 4 步
  首次 completion 同时提交 tool/command 计数 1；最终 UI 固定披露两种失败均未被恢复成功抹除。
  单回合 completed、147 条 canonical 事件、2 个 returned process effect，独立 35 项断言通过；
  工作区不变，ping/cmd、应用和 profile 进程清零并正式备份。生产代码无需再改。本批未制造外部
  文件部分效果；持续写入、force-kill/lost、其他命令/Provider/角色/平台与长期矩阵仍开放，不关闭
  完整 PROC-048/REAL 或共享阶段。

- S-D04-27（PROC-011/012、CTRL-006/007、REAL-011/021～024、A02/A05/A08/A17/A18/A19
  进程 non-start 控制子断言）：W97 首次正式 Tauri 中，typed `PROCESS_NOT_STARTED` 正确证明用户代码
  零执行且不要求 replan，却也没有激活 TaskLedger；下一步看不到 `update_plan`/`report_completion`，
  模型两次 ToolSearch 均无结果，随后重放一次 non-start 和一次恢复命令才完成。9 个模型步骤、4 个
  process effect 及“摘要称工具错误 1、Runtime 实际披露 2”的 `FAIL_RECOVERED` 全部保留。现让 typed
  non-start 仅激活 Ledger/Completion 控制，不改变无需 replan、工作区证据不失效和用户代码零执行
  语义。新回归首次 **0/1**，修复后 **20/20**，Agent Runtime **191/191**。同一正式 UI 夹具随后
  精确 4 步完成：一次 non-start、一次计划、一次恢复、一次报告；tool error=1、command failure=0，
  137 条事件、2 个 returned effect，独立 31 项断言通过，工作区和进程清理一致。其他 spawn
  permission/format/cwd、其他 Provider/角色/平台及长期矩阵仍开放，不关闭完整 PROC/REAL 或共享阶段。

- S-D09-05（LIFE-020、A10/A12/A17/A19 cancel 后重启子断言）：新增完整 AppServices 重建回归。
  先在 429/Retry-After 等待中取消 Turn，确认 canonical cancelled 后关闭首个 App/数据库，再从
  同一隔离 data root 重建服务与 Router。路由发布前的 startup recovery 候选精确为 0，provider
  请求仍为 1；GET execution 保持 cancelled、checkpoint 不保留、`execution_resumed` 为零。
  首次即通过，完整 App recovery 6/6，新回归 **20/20**。生产恢复查询无需修改，本批仅补测试/
  进度；强杀中间窗口、cancelled Session 删除、真实 UI/平台及长期恢复矩阵仍开放。

- S-D09-04（LIFE-019、G0-025、A04/A05/A10/A17/A19 cancel/receipt 竞争子断言）：新增
  canonical Store 两连接事务竞争，让 Turn cancel 与同一 managed effect 的成功 owner receipt
  同时提交。最终始终只有一个 cancelled Turn terminal；已完成 effect 保留 `effect/succeeded`
  receipt 并投影为 returned，active Turn 清空，没有把 cancel 当回滚。首次短 filter 实际未命中；
  随后两次真实 FAIL 分别暴露夹具复用了 started producer、又误改生命周期 idempotency key，均保留。
  按既有合同使用原 key + 独立 owning producer 后，完整 pause/effect 11/11、新竞争 **20/20**。
  生产事务归约无需修改，本批只补回归/进度；App owner 回执、外部 unknown effect、真实 UI 与
  其他 Action/平台的 cancel 竞争仍待验。

- S-D09-03（MODEL-020/022/023、LIFE-018、A08/A10/A11/A17/A19 Retry-After 取消子断言）：
  新增正式 App 路由夹具，provider 首次返回 HTTP 429 与 60 秒 `Retry-After`，在 broker 退避期间
  通过 canonical Turn cancel 取消。Turn 在 2 秒验收预算内进入 cancelled，checkpoint 清空；再观察
  1 秒仍只有一次 provider request，零后续 model/tool attempt，且没有 completed/failed terminal。
  首次即通过，完整 App recovery 5/5，新增回归 **20/20**；Broker Retry-After 3 项和 HTTP 解析
  2 项保持通过。当前生产取消传播正确，本批只补产品路径回归/进度，无生产或 renderer 修改。
  尚未覆盖真实 provider 连接池/代理切换、取消与 effect receipt 竞争、UI 点击停止及其他平台/角色。

- S-D09-02（LIFE-016、PROC-033/039、CONC-004、A12/A13/A17/A19 进程 in-flight 暂停子断言）：
  新增正式 App 路由夹具，通过实际 `start_process` 启动写入 PID 后长驻的 OS helper，并在下一次
  provider 请求等待时发起 owner pause。独立进程快照先证明 PID 存活；paused 发布前 canonical
  `host_cleanup_proven` 已落盘，随后 PID 消失。恢复使用同一 Turn 的新 generation，保留已对账的
  start/read 历史，重新规划和读取 marker 后 completed；`workspace.process/start` 全程恰好一次，
  活句柄未进入 checkpoint，也没有重放。完整 App recovery 4/4，新增回归同源码 **20/20**。
  当前生产清理/恢复实现通过，本批仅补跨层真实进程回归及测试用 process snapshot 依赖，没有
  修改产品逻辑或 renderer。测试采用 scripted provider 做故障边界，不计真实 StepFun/UI；Browser、
  MCP in-flight、清理失败/人工证明、macOS 原生进程组及完整 LIFE-016 仍开放。

- S-D09-01（CTRL-009/010、LIFE-015/017/020/021/022、CONC-007/008、A06/A10/A12/A17/A18/A19
  pause checkpoint 子断言）：现有 owner 级 pause/resume 生产路由与 canonical Store 已做完整定向
  复核。合法恢复保持同一 Turn，只增加 execution generation/fence；暂停期间禁止新 Turn，旧
  producer 被 fence，已完成 write 不重放。错误 digest/revision、owner、Snapshot、build、active
  set、预算和 cleanup 证明均在提交前拒绝。completed 后的新 resume key 与 cancelled 后的 stale
  preparation 都拒绝；旧 key 只返回原幂等授权回执，不新增事件、模型请求或写入。新增三命令
  竞争证明 cancel 终态唯一且不可逆，pause/checkpoint/active Turn 均清空；两项新增回归各
  **20/20**。相关 Session pause 10、checkpoint 8、App 恢复 3、投影 2、UI 暂停状态 9 项通过。
  两次首次失败均为新增夹具错误（terminal head 应为 ready；terminal 后重新 prepare 本应先被
  fence），已原样保留，未修改产品状态机。本批未发现需改的生产缺陷，只补缺失回归。
  当前 pause checkpoint 授权来自 owner `/execution/resume`，与 W45 model-facing closed-turn
  `resume_task` 不同；会话区只验证暂停提示/发送阻断/停止，尚无 owner resume 正式 UI 入口，
  因此不关闭完整 CTRL-009/010、OBS-007、LIFE-016 或真实 UI/平台矩阵。

- S-D02-12（CTRL-001/006/007、REAL-013、A05/A07/A17/A18/A19 已闭合任务显式继续子断言）：
  W45 从 W44 completed Turn 的正式备份跨构建继续，要求导入全部旧要求、先更新计划，只读两个
  文件且不重放已完成修改。首次确定性 FAIL 证明历史任务仍被旧 build digest 拒绝；现仅允许在
  Session、runtime binding 与精确 Snapshot 不变时跨 engine build 导入，来源 Turn、记录格式和
  当前引用仍严格校验。`resume_task` 只在尚未规划/观察的入口暴露，成功后立即移除；导入计划
  必须先通过仅暴露 `update_plan` 的控制轮次，不能依赖 provider 对 tool choice 的自愿遵守。
  supported completion 现在必须至少引用一项当前证据；分页 `read_file` 也明确说明任意成功页的
  sha256 是整份源文件摘要，避免为取摘要读取全文。
  早期真实样本依次保留重复 resume、计划参数错、无证据完成、重复读取、provider 忽略选择和
  摘要语义误解；其中一份旧二进制虽通过 oracle，代码复核发现门禁会被通用选择归一化覆盖，
  未作为最终证明。最终 Runtime **167/167**，8 项关键回归各 **20/20**；正式 Tauri 5 模型步，
  顺序为 resume→plan→两次目标读取→plan→completion，零工具错误/效果，两文件 hash/mtime
  不变，canonical completed，严格 verdict 为 `PASS_TASK_CONTINUATION`。本项是 closed-turn
  task continuation，不是 CTRL-009/010 的 pause checkpoint/终态恢复；后两项仍开放。W42 N3
  仍为 1/3，完整角色/平台、范围变更、100 seed、LONG/99% 与共享阶段均未关闭。

- S-D02-11（CTRL-006/007、FILE-031、A05/A07/A17/A18/A19 重启后精确修复子断言）：W44 从
  W43 failed Turn 的正式备份继续，先后保留 completed-only 来源拒绝、跨构建 recovery/history
  等值拒绝，以及空计划被恢复上下文刷新误置 `needs_replan` 后首个补丁提案被拒的 FAIL。版本化
  patch 状态与只读闭合历史现可跨应用构建读取，但仍校验记录格式、Session、来源 Turn 和精确/
  模型兼容 Snapshot；运行中 checkpoint 继续绑定原构建。恢复读回只使既有计划失效，不再凭空
  创建计划门禁。18 项去重定向通过，4 项新增各 20/20；最终正式 Tauri 5 模型步，先后各读两
  文件、仅一次 second 补丁及一次成功 effect、零工具错误，first 摘要/mtime 不变，second 精确
  修复，v2 recovery 清空且 canonical completed。该结果记 recovered PASS；W42 首轮 N3 仍为
  1/3，完整角色/平台、范围变更、100 seed、LONG/99% 与共享阶段仍开放。

- S-D02-10（CTRL-006/007、FILE-031、A05/A07/A17/A18 部分任务完成判定）：W42 第 3 次
  首次 FAIL 证明多文件补丁部分发布后，只要重读目标并把事实写成 supported，原任务就会
  误结 completed。根因是恢复状态把“已重读”和“未发布的任务义务”混为一体。v2 状态现
  分别持久化待重读、未完成目标及接纳输入边界；结构化发布索引只结算已确认发布项，读取
  不能结算未发布项。精确目标的成功 write/patch 或有后续用户引用的 scope_changed 才可
  解除；不确定结果保守，零发布负向检查在重读后仍可按原合同完成。161 项去重定向通过，
  11 项新增各 20/20。正式 Tauri 4 模型步/1 rejected effect，无后续操作，second.txt 保持
  原文并列为 unresolved，终态 failed、部分正文可见。普通文本分支实测；误 supported 分支
  为确定性回归。完整角色/平台、N3/长期门槛仍开放，共享阶段未完成。

- S-D02-09（CTRL-007、FILE-031/039、A17/A18 固定样本验收）：W42 使用同一构建、GEN Revision
  和模型配置完成三个独立正式 Tauri 样本。仅第 1 次首轮通过，补到普通文本结束分支的 UI
  证据；第 2 次 blocked 报告使用 `call_...` 占位引用被正确拒绝，修正后结束；第 3 次只重读后把
  部分完成报告标为 supported，canonical Turn 错结 completed。三次各只发布一次补丁，磁盘
  保护及最终正文展示均可核对，共 16 模型步、3 rejected effects；**N3 未通过**，不合并
  recovered 与首轮结果。当前完成账本只验证引用/ID 覆盖，没有证明原任务目标已满足。
  下一批优先补部分任务误判完成的反例与修复，保留未知/失效引用的拒绝规则。W42 未改
  产品/未放宽断言，完整证据见 Windows W42；共享仍未完成。

- S-D02-08（CTRL-007、FILE-031/039、A07/A17/A18 失败交付子断言）：W41 反例确认补丁仍待
  核对时，普通最终回答先触发 completion review，再拒绝成功，重新打开模型工作。现前置
  已有失败检查，保留结果正文及恢复义务；已重读的正常完成路径仍要求有效证据。正式 Tauri
  本次走有效 blocked 报告分支，4 模型步、1 rejected patch、无后续文件操作，但结果正文
  被错误卡片折入过程区。保留 UI FAIL 后，让错误前最后结果与失败提示同时可见；历史游标
  与显示时间混用造成的首版冷加载 FAIL、迟到旧文本反例也保留并修复。123 项去重定向、
  5 项新增各 20/20 通过；同一失败会话正式冷加载正文可见，canonical 四类记录完全不变，
  0 新模型步/效果。普通文本结束分支仍只有组件证据，完整角色/平台、N3/长期门槛及其他
  完成控制问题未关闭；不把 W39/W40 的失败或整个共享阶段改记通过。

- S-D02-07（CTRL-007、FILE-031/039、A07/A17/A18 失败收尾子断言）：W39 的部分补丁失败后，
  Runtime 连 blocked 报告也要求先重读全部目标；与用户错误即停止约束冲突。W40 确定性
  首次 FAIL 保留。现校验受阻报告后将未完成步骤结为 blocked，保留全部输入义务与待核对
  目标，交付摘要并以 failed 结束；成功/仅 unverified、陈旧证据、运行中进程仍不能绕过。
  35 项定向、5 项新增各 20/20 通过。首版真实 UI 又被无效证据引用阻挡，补待核对时实际
  工具说明中的受阻格式；两次真实 UI 仍重读/重试，均停止且保留 FAIL，共 26 模型步、
  4 rejected effects。**仅组件分支已修，产品闭环未完成**；不再将增加提示视为修复证明。
  下一步核对正式模型请求的控制定义、历史/压缩后约束保留与失败报告选择；完整角色、
  原生平台及 N3/LONG/99% 门槛仍开放，不关闭 S-D02-01 或共享阶段。

- S-D03-35（FILE-031/033/039、A05/A08/A17 多文件补偿子断言）：前项发布后被同字节异身份
  文件接管，旧回滚仅比正文，误覆盖外来文件。W39 首次 FAIL 保留；现从暂存创建句柄保留
  发布身份，补偿时同时核对身份/字节，Windows 最后目标句柄再次核对，确认过的迟发失败
  也保留身份；未确认项沿用禁止回滚。真实 Tauri 首轮另发现普通 IO 错误误报平台不支持，
  模型因此声称零修改；现分类为执行失败，并给模型明确发布/恢复数量、零基索引及部分效果
  说明，仍剥离私有诊断。79 项去重定向、10 项新增各 20/20 通过。最终 UI 磁盘/回执和
  部分结果说明一致，但模型随后违反不重试约束再次调用补丁，已停止并保留 FAIL，接续
  S-D02-01 控制链；两次共 23 模型步、3 rejected effects，不计完整 UI/Case 通过。
  原生间隙、Unix/macOS、完整角色与 N3/100 seed/LONG/99% 仍未验，共享阶段未完成。

- S-D03-34（FILE-020/025/038、A05/A07/A17 替换目标确认子断言）：最后校验后目标名称被
  外来文件接管，虽已返回 unknown 并保留备份，仍把预期正文标为确认，允许当前项自动回滚。
  W38 不同内容/相同内容异身份两项首次 FAIL 保留。现持有原目标保护句柄，核对备份仍为
  该对象后才确认发布；内部标记改为 publication_verified，未确认项只请求重读并排除回滚。
  56 项定向、4 项新增各 20/20 通过，含真实原生置换后的通知/回滚与现有映射的零发布拒绝。
  0 模型/UI。文件 symlink 夹具返回 1314，单独阻断；原生间隙零副作用、换走后换回、其他
  映射组合、Unix/macOS、完整角色与长期门槛仍未证明，不计完整 Case 或共享阶段完成。

- S-D03-33（FILE-040、A05/A14/A17 清单对账子断言）：API 与 Agent 文件实例各持缓存，
  通知期间仍可读到旧清单；写/删仅失效当前根，重命名没有失效。W37 四项原生首次 FAIL
  及关闭 Office 自动预览时的正式 Tauri 首次 FAIL 保留。清单状态现由 AppServices 共享，
  各读取先按自身路径授权校验；发布/删除同步失效相交根，重命名覆盖旧名和新名。
  31 项定向、7 项新增各 20/20 通过，含热缓存越权拒绝、另一实例写入时旧扫描失效及真实
  路由接线。正式 Tauri 新建后恢复新候选、保留旧候选；前后共 6 模型步、2 returned effects。
  无 watcher 的外部变更、原生发布窗口、完整丢批/乱序、其他角色/平台及长期门槛仍开放，
  不计完整 FILE-040 或共享阶段完成。

- S-D03-32（FILE-018/038/039/040、A05/A07/A08/A09/A17 预览对账子断言）：未确认发布没有
  重读通知，无内容事件又被前端当成空白更新；保存回包还会把后续编辑误标已保存。W36 首次
  失败保留。现以不带正文的通知要求按原工作区重读，迟到回包按页签/来源/尝试隔离，保留脏
  草稿、明确读失败与重试，保存只确认实际送出的内容；补关闭重开激活及失败保存的加载状态。
  首版正式 Tauri 仍失败，进一步确认 Wave2 宿主丢弃事件；生产构造现必须接入应用的用户事件
  总线，仍按 owner 投递。72 项去重定向通过，18 项前端及 3 项原生/宿主关键回归各 20/20；
  正式 Tauri 新隔离会话的 unknown 重读、读失败告警与手动重试三项通过；三次会话合计
  9 模型步、3 pending effects，未把预览恢复记成写入成功。其他预览类型/角色、丢批乱序、
  Unix/macOS、原生发布窗口与长期门槛仍开放；不计完整 Case 或共享阶段完成。

- S-D03-31（FILE-020/038/039、A05/A07/A17 暂存发布证据子断言）：原生替换返回成功即删
  备份，暂存被改写或换成同内容外来对象仍成功；未确认结果还广播预期字节。W35 首次失败
  均保留。Windows 现按创建时身份和调用字节核对发布前后对象，确认前保留原件备份；无法
  确认则返回 unknown、记录未确认索引并排除该项自动回滚。仅有字节证明才发内容事件，其他
  不确定状态失效缓存；清理/恢复不确定性统一保留防重放保护。65 项去重定向、11 项各 20/20
  通过，含消息完整性及重启/换 key 拒绝；0 模型/UI。本批未证明原生间隙零副作用；消费者
  重读通知、目标名称/映射写入、Unix、完整 UI/角色及长期门槛仍待验，共享阶段未完成。

- S-D03-30（FILE-020/025/026 既有目标字节窗口子断言）：全文校验释放读取句柄后，最终替换
  仍允许普通写入者改动目标；W34 两项首次 FAIL 均返回成功并覆盖并发内容，日志保留。
  Windows 现通过拒绝写共享的目标句柄完成最后字节核对，并持有至原生替换返回；拒绝最终
  reparse/非普通对象，沿用 ReplaceFileW 和既有备份/unknown 恢复规则。52 项去重定向通过，
  3 项新增回归各 20/20，含同长度变更；另覆盖旧读句柄、写/删共享拒绝、ACL/命名流和恢复。
  暂存源、目标名称置换、映射写入、Unix、正式 UI/完整角色及长期门槛仍开放；未关闭整条 Case。

- S-D03-29（FILE-018、A08/A09/A17 文件候选失败子断言）：引用菜单把请求失败显示成零匹配，
  加载时仍可选择旧候选。W33 两项组件首次 FAIL 和正式 Tauri 的 409→空结果首次 UI FAIL
  保留。现显示中英错误及重试，加载/失败时不提供旧候选；请求按会话、工作区和尝试隔离，
  关闭/卸载或迟到响应不能回填。12 项定向及各 20/20、类型/翻译/桌面边界和 Tauri 构建通过；
  新隔离 Tauri 验证错误、修复夹具后重试、保持查询及继续输入，前后共 2 模型步、0 effects。
  完整权限/网络错误分类、实际多会话竞态 UI、全部角色/平台与长期门槛仍待验，共享阶段未完成。

- S-D03-28（FILE-016/018、AUTH-010 扁平清单/规则边界子断言）：旧遍历器导入根外父目录和
  Git 排除规则，跳过坏规则后仍成功，根置换还泄漏根外名称。W32 四项首次 FAIL 及正式 Tauri
  `@visible` 被父规则误滤的首次 UI FAIL 保留。清单现复用受限遍历，保留隐藏文件及根内规则
  优先级；gitdir/commondir 仅解析根内授权目标，链接项跳过，遍历/规则/预算失败不发布缓存。
  41 项定向通过，清单相关 20 项各 20/20，两万文件精确边界及超限各 20/20；正式 Tauri
  文件候选恢复、根内忽略、根外 junction 跳过三项通过。前后共 2 模型步、0 effects。
  文件候选的失败可见状态、Unix 竞态、跨重启根身份、全部角色与长期门槛仍待验；根外 Git
  元数据须由明确资源授权承载，不从工作区链接推导额外权限，共享阶段仍未完成。

- S-D03-27（AUTH-010、FILE-018 侧栏目录边界子断言）：单层目录接口以链接的根内名称放行
  根外目标，校验后还用普通路径重新打开目录。W31 直接 junction 及目标/祖先/根替换四项
  原生首次 FAIL、正式 Tauri 展示根外文件名的首次 UI FAIL 均保留。现要求 canonical 目标
  留在工作区，复用受限目录读取；普通项取枚举类型，根内链接仍可展开，根外/私有/断链保留
  为不可展开项，枚举失败不再当作完整快照。15 项定向通过，侧栏 13 项各 20/20；正式 Tauri
  的根外链接、根内链接及缓存目录转向后 403/告警三项通过。前后共 2 模型步、0 effects。
  Unix 竞态、接纳前/跨重启根身份、权限错误完整分类、扁平遍历、其他入口/角色及长期门槛仍待验。

- S-D03-26（FILE-018/040、A08/A17 读取失败可见性子断言）：文件树读取失败被返回为空数组，
  UI 只留控制台错误，旧列表没有过期提示。W30 两项组件首次 FAIL 和正式 Tauri 首次 UI FAIL
  保留。现以 null 表示无可用结果，保留失败状态并展示告警/重试；重试保留原搜索条件、重读
  失败分支，其他目录成功及较旧请求不能清掉新错误。初版同步重试残留错误及类型错误另留痕。
  24 项定向、9 项各 20 次、类型/翻译/桌面边界与 Tauri 构建通过；新隔离 Tauri 的旧快照
  告警、无快照失败、恢复后重试三项子断言通过。两次准备共 2 模型步、0 effects。
  原生权限/网络错误完整 UI 矩阵、其他角色与平台、100 seed 和长期门槛仍待验，共享阶段未完成。

- S-D03-25（FILE-040、PORT-012 消费者重连子断言）：共享传输层已发送重连/积压恢复信号，
  会话文件树却未订阅。W29 首次组件 FAIL 与正式 Tauri 在真实断线后保留旧文件树的首次 UI
  FAIL 均保留。接入既有 conversation.reconnected，复用当前工作区的有界合并与卸载清理。
  22 项定向、3 项各 20 次、typecheck、桌面边界与 Tauri 构建通过；另一个隔离 Tauri 在网络
  子进程中断后自动重连、重读并与磁盘一致，主应用/浏览器/renderer 保持不变。两次准备共
  2 模型步、0 effects。真实服务端积压、连接内 native rescan、乱序/重复矩阵、完整角色、
  其他平台与长期门槛仍待验，不计完整 FILE-040/PORT-012 或共享阶段完成。

- S-D03-24（FILE-040、PORT-012 / A17 文件树重读子断言）：成功的空目录响应被 UI 忽略，根
  刷新又直接复用已加载子目录的旧节点。W28 三项组件首次 FAIL，以及正式 Tauri 中根/子目录
  已空但刷新后仍显示旧文件的两项首次 FAIL 均保留。现接受空结果，重读已加载分支，旧请求与
  卸载后的响应不得回填；不扩大读取到未加载目录。13 项定向检查、9 项各 20 次、typecheck、
  桌面边界及 Tauri 构建通过；新隔离 Tauri 的根清空、子目录清空、空目录再新增三项手动对账
  通过。两次正式会话准备共 2 模型步、0 effects。自动丢批/重连触发、失败重读的可见诊断、
  100 seed、完整角色、其他平台和长期门槛仍待验，不计完整 Case 或共享阶段完成。

- S-D03-23（FILE-040 原生监听与清单恢复、A13/A17 子断言）：真实 Office 创建/文件删除通知
  送达期间，清单 API 仍返回旧缓存；首次监听和重订还复用订阅空窗前的旧结果。W27 三项首次
  FAIL 保留。App 现将监听接到实际文件路由的 cache owner，回调在事件过滤/debounce/发送前
  失效相交目录清单，原生错误/rescan 和订阅成功同样失效；弱引用不延长 owner 生命周期。
  Windows watcher/缓存 39、原生/路由接口 19、App 监听上下文 12 项通过，新增 6 项各重复 20 次。
  未覆盖未监听期间的主动刷新、完整 UI 丢批对账、迟到回调/尾事件、扁平遍历边界、
  其他平台和长期门槛，不计完整 FILE-040 或共享阶段完成。

- S-D03-22（FILE-040、PORT-012 原生事件缺口子断言）：native rescan 标记被丢弃，监听错误又
  将未知损失量记为 1。W26 两项首次失败保留；新增可选 rescan_required，未知缺口与可计数的
  本地丢弃分开，空事件也交付对账提示。按路径分量严格编码，不能表示的名称计 dropped，避免
  有损转换指向另一文件。旧四字段批次仍可读取，普通输出保持原编码，新字段经发布 Schema 校验。
  Windows 16 项定向通过，新增 6 项各重复 20 次；生成检查另发现此前 Cargo.lock 摘要漂移，
  保留失败并由正式生成器刷新三个摘要文件后通过。无模型/UI；重订、传输/消费者全量对账、其他
  平台与长期验收仍待验，不计完整 FILE-040/PORT-012 或共享阶段完成。

- S-D03-21（FILE-040、PORT-012 队列子断言）：单个不可表示路径会使已排队的正常事件也被
  整批丢弃且不留计数；交付后 debounce 仍会吞掉同路径的新变化。W25 两项首次失败保留。现按
  原有路径合同入队，拒绝项单独计 dropped，正常事件保留；交付时清合并状态，批内仍可合并。
  Windows 队列/原生监听 7 项、事件合同 1 项通过，7 项各重复 20 次；编译器一次访问违规崩溃
  另保留，同源码重试后通过。未覆盖 native rescan 信号、完整传输/消费者恢复、UI 与长期统计。

- S-D03-20（FILE-040 watcher 事件隔离支撑、AUTH-005 子断言）：Office watcher 共用全局
  debounce，同一物理文件在重叠工作区只通知其中一个；停止重订也继承旧抑制状态。W24 两项
  首次失败保留；debounce 现归各 workspace 注册所有，最后 owner 停止时清理。Windows 41 项
  通过，新增 3 项各重复 20 次；真实双 watcher 分别收到正确 workspace，停一留一继续工作。
  活跃订阅容量/抑制尾事件、缓存接线、批次队列和真实 UI 仍待验。

- S-D03-19（FILE-040 watcher 生命周期支撑、AUTH-005/A13 所有者与清理子断言）：停止监听时
  重新解析已删除文件/工作区，找不到原 canonical key，却返回成功并留下登记。W23 两项首次
  失败保留；现保存按 owner 隔离的请求别名，停止用原目标，改指向的活动别名先要求停止原订阅。
  原生撤销仅成功或 WatchNotFound 才清登记，未知错误保留并报告。Windows 38 项通过，新增
  4 项各重复 20 次，删除重建后的真实事件恢复；不代表缓存接线、队列/落后 UI 或完整 FILE-040 通过。

- S-D03-18（FILE-040 的失效/重读、并发与取消子断言）：已失效的旧扫描会重建旧缓存，甚至覆盖
  较新扫描；写/删事件又早于失效发出，订阅者拿到旧列表。W22 三项首次 owner 失败保留。缓存
  条目身份现在控制发布资格，失效撤销旧扫描；最多一次有依据的重读，持续变化返回冲突，取消/失败
  释放未发布条目。变更事件先失效再发送，保持不同根隔离。Windows 33 项通过，新增 6 项各重复
  20 次通过；未覆盖底层清单遍历竞态、外部 watcher 接线、事件洪泛/落后 UI 与完整生命周期统计。

- S-D03-17（AUTH-009/010、FILE-016/018 目录读子断言）：两级目录树在校验后、子目录预取前换链
  均返回根外名称；子目录拒绝列举还被表示为空。W21 三项首次 owner 失败保留。树及子项分类现
  复用受限目录游标，无法完成预取则明确失败，保留真实授权根而非把显示根当作权限。Windows
  17 项定向检查通过，新增 5 项各重复 20 次通过；合法其他授权根、隐藏项、根内链接入口与排序
  保留。扁平清单/缓存、独立侧栏入口、其他平台/文件系统、正式 UI 与统计验收仍待验。

- S-D03-16（AUTH-009/010、FILE-011/018 元数据子断言）：元数据和非递归指令范围仍在校验后按
  路径 stat；短暂父目录换链分别返回根外大小、把根内文件报告为目录。W20 两项首次 owner 失败
  保留；两入口现复用受限元数据句柄，并仅把真实的缺失祖先/目标分类为 missing。Windows 108 项
  通过，新增 5 项各重复 20 次通过；父目录拒 RD/S、文件拒数据读的合法 metadata/子目录搜索通过，
  校准后的拒绝不伪装缺失。初版 ACL 夹具未形成原生拒绝及其恢复失败均保留，临时 ACL 已恢复。
  普通清单、既有文件发布窗口、其他平台/文件系统、完整 UI/角色及统计验收仍待验。

- S-D03-15（AUTH-009/010、FILE-013～018 子断言）：旧搜索 walker 即使关闭 parent matching，
  仍打开工作区上方的 ignore 文件；旧配置组件探针另确认子目录置换可提供根外规则。W19 保留
  owner 首次失败、组件反例及大小写兼容中间失败；根直接换链原已有 incomplete，其严格抛错
  夹具判定过严，已保留并纠正分类。搜索现按受限目录遍历，只在绑定根内探测仓库标记，规则通过
  有界 reader 读取并计入字节/解析预算；不能完整验证规则时停止该子树并明确 incomplete。
  Windows 109 项通过，新增 9 项各重复 20 次通过；保留 ignore 优先级/嵌套仓库/显式文件、隐藏
  属性、大小写敏感性与准确 offset/SHA。普通清单、更多权限/文件系统/Unix、真实 UI 和统计仍待验。

- S-D03-14（AUTH-009/010、FILE-011/012 的递归枚举子断言）：`instruction_scope` 校验后临时换链
  再恢复，工作区根及子目录两项均把 13 项根外元数据记为完整扫描。Windows W18 两项首次 owner
  失败及夹具 UUID 错误保留；枚举现沿受限根打开目录并持有原生只读游标，名称和类型来自同一
  目录对象，身份/边界冲突直接拒绝。原有隐藏/忽略目录发现、链接 incomplete 和预算语义保留。
  Windows 99 项通过，新增 5 项各重复 20 次通过；文件清单/搜索 walker、Unix、其他文件系统、
  100 seed、完整模型/UI/角色及长期验收仍待验。

- S-D03-13（AUTH-009/010、FILE-034～039 子断言）：校验后替换父目录可删除根外文件/目录，
  递归目标在 DELETE 检查后改名也会误删并发目录。Windows W17 四项首次失败保留；删除现沿
  已核对的根句柄逐段打开目标，递归使用有界原生目录游标及逐项 DELETE 句柄，不跟随嵌套链接。
  空目录不要求列举权限，部分失败保留 unknown/fence。Windows 99 项通过，新增 7 项各重复
  20 次通过；中间目录游标拒绝错误及修正证据保留。其他文件系统/Unix、IO 故障、100 seed、
  完整 UI/角色与长稳仍待验；此结论不覆盖普通文件枚举或既有文件发布。

- S-D03-12（FILE-034/035/036/037/038 子断言）：删除权限检查后仍按路径删除，最终文件名被普通
  或 POSIX 改名置换时会误删并发文件。Windows W16 两项首次失败保留；普通文件分支现持有拒绝
  删除共享且不跟随末端链接的 DELETE 句柄，以同一句柄删除，保留只读/ACL/共享拒绝及旧读者。
  Windows 77 项通过，新增 4 项各重复 20 次通过；打开前父目录/目标置换、递归删除及其他平台仍待验。

- S-D03-11（AUTH-009/010、FILE-018/025/026/031 子断言）：补丁预检与内容匹配各自直接按路径
  读取，父目录临时换链可接受根外同内容来源，读取中的同名置换也未拒绝。Windows W15 四项首次
  失败保留；预检、回滚和发布前匹配现复用受限、有界且核对文件身份的 reader，保留原 guard。
  Windows 95 项通过，新增 4 项另重复 20 次通过；最后一次读取到原生替换之间的竞态、Unix、
  目录枚举/删除、完整 UI/角色及统计门槛仍未关闭。

- S-D03-10（FILE-019/028/038/039 子断言）：新建发布在暂存后关闭写句柄，再按文件名 hard-link；
  普通改名、POSIX 替换和原地改写均可使其发布错误字节，三项首次失败保留于 Windows W14。
  Windows 新建分支现持有拒绝其他写入/删除共享的创建句柄，以同一句柄不覆盖重命名，失败按句柄
  清理；发布后错误仍保留 unknown。Windows 93 项定向检查通过，新增 4 项另重复 20 次通过。
  仅关闭新建分支的暂存源窗口；既有文件 ReplaceFile、Unix、100 seed、完整 UI/角色及长稳仍待验。

- S-D03-09（FILE-019/020/025/028/038/039、OBS-016 子断言）：失败临时文件和成功后的备份名
  被换成其他文件时，旧清理会误删；部分替换失败还会把其他文件当原件恢复。三个首次失败现场
  保留于 Windows W13。共享发布逻辑不再清理已经消耗的临时名；Windows 保留原对象的元数据
  句柄及卷/128 位文件 ID，清理、恢复先核对身份，再对该句柄操作，恢复不覆盖并发出现的目标。
  不匹配/清理失败保留 unknown 和原有 fence。Windows 95 项定向检查通过，日志见 W13；
  发布前最终文件/暂存文件置换、Unix 未消耗临时文件的清理竞态、正式 UI/其他平台仍待验。

- S-D03-08（AUTH-009/010、WIN-007、FILE-019/020/024/025/028 子断言）：父目录准备只返回路径，
  句柄在发布前释放；Windows 三种写入在目录换链后均修改根外文件。准备结果现持有经身份核对的
  目录句柄及按关闭删除的空保护文件，覆盖发布和清理；保留原子替换的 ACL/ADS 与失败结算。
  新 ACL 反例同时发现目录打开额外申请 SYNCHRONIZE，读取/写入路径解析改用原生元数据权限。
  Windows 97 项定向回归通过，首次和中间失败及原子权限夹具校正见 W12；最终文件/临时文件置换、枚举/删除、
  跨重启根身份、正式 UI 及其他平台仍待验，不计完整 Case 或全部文件竞态关闭。

- S-D03-07（AUTH-009/010、WIN-007、FILE-018 子断言）：路径校验后短暂置换父目录 junction，
  旧 reader 实际读到根外字节，恢复路径后仍返回成功；同名不同文件替换也未被识别。Windows
  内容读取改为相对受限目录句柄逐段不跟随新链接，并核对句柄的实际路径及读前/读后文件身份。
  文本/二进制、搜索和补丁读取复用此边界，旧 Agent 全文入口也统一到 8 MiB 合同。Windows
  81 项通过，首次失败和原生夹具见 W11。目录枚举、写/删竞态、跨重启根身份及其他平台仍待验。

- S-D03-06（FILE-032/039、OBS-016、LIFE-006/007 子断言）：Artifact 未知结果判断扫描完整错误
  文本，普通目录名/临时文件冲突中的相同字样也触发 unknown，补丁错误因此无法正常结算。
  改为只识别 Artifact owner 的 Conflict 错误及明确前缀；原始文件和不属于本次操作的临时文件
  保留，已知失败结算 rejected，重开 host 后新操作可执行；真实 post-link 置换/回滚不确定仍
  为 unknown。Windows 50 项定向检查通过，首次失败见 W10；无模型调用、无完整 UI/macOS 结论。

- S-D01-01（REG-005/008、整批预检）：Windows W07 后两次真实 GEN 中，一次 8 步零错误，
  另一次在第 2/5 步将 ToolSearch 与 write/delete 混批，产生 4 个拒绝结果，随后恢复。
  实际工具说明已有互斥规则。W09 补可选的并行调用偏好，仅 ToolSearch 与其他工具同时暴露时
  请求单调用；映射到 OpenAI/Responses 与 Anthropic 系协议，原整批拒绝、授权和普通只读并行
  保留。Windows 199 项定向检查及同构建 GEN 8/9/8 步三次真实场景通过；25 次 HTTP 200，
  实际请求均带 false，每模型步一调用，文件/回读/答复完整。旧 FAIL 不改写。
  **该 GEN 场景 N3 通过**；Gemini 当前无对应请求字段，其他模型/角色/macOS 与长期统计仍待验。

- S-D01-02（PROC-001/003、CTRL-006/007、A02/A17/A19 模型数组参数形状子断言）：W72 的
  `report_completion.requirement_ids` 与 W74 的 `exec_command.args` 均曾把数组编码为 JSON 字符串；
  前置 Schema 正确拒绝且零 dispatch，但合法任务出现可见错误。W75 两项首次说明回归确认字段虽为
  `type=array`，模型说明没有明示“实际 JSON 数组、不得传字符串”。现只增强 process `args` 以及
  completion 的 `criteria`/`requirement_ids`/`evidence_call_ids`/`evidence_paths` 模型可见说明，
  保留严格类型校验、整批原子拒绝与零字符串强转。两项精确回归各 **20/20**，Agent Runtime
  **175/175**。正式 Tauri `668062e479e6…` 中 step-3.7-flash 首次形成真实 `args` 数组，唯一进程
  退出 0 且 `reaped=true`，UI 零异常，正确报告 300、`shape-0001.txt`、`shape-0300.txt`；单回合
  2 个模型步、67 条事件。第二个隔离样本按要求执行两条命令后，`report_completion.criteria` 和两组
  `evidence_call_ids` 首次均为真实数组，UI 同样零异常，3 个模型步、121 条事件；12 文件摘要不变，
  两次均应用/profile 清零并正式备份。但 alpha criterion 错把 beta call ID 作为证据，因此只通过
  数组形状子断言，精确 evidence 语义关联另转后续 CTRL 批次。其他数组字段/Provider、N3/100 seed/
  LONG/99% 仍开放，不关闭完整 REG/CTRL/PROC 或共享阶段。

- S-D01-03（LONG-008、ACOD-017、A05/A09/A15/A17/A19 压缩夹具子断言）：W92 额外
  `native_coding_reliability` 长度压力回归为 **1/2**；W95 在当前基线原样复现。产品发出合法的
  no-tools compaction 请求，scripted provider 却要求每个请求都有原生工具并 panic，重试后将
  Execution 错误暂停为 provider unavailable。夹具现按压缩专用提示和空工具面区分请求，用独立
  task/compaction 计数响应有界摘要；压缩后原始 tool message 已移出当前窗口时，要求
  `available_evidence` 仍含当前 `gomoku/index.html` 路径，磁盘、effect、完成和未验证范围断言均保留。
  最终一次请求轨迹精确为 3 个任务请求 + 1 个 canonical compaction，请求数与事件数相等，写入只
  执行一次且 Turn completed；完整文件 **2/2**，原失败 Case **20/20**。生产 Runtime/Provider
  编码无需修改。本批只修验证夹具并证明一次压缩；20 次连续压缩、真实 Provider/UI、pause/restart、
  其他 Agent/平台及 LONG 统计仍开放，不关闭完整 LONG-008/ACOD-017 或共享阶段。

- S-D01-04（REG-005/008、LONG-008、AGEN-002/005、ACOD-017/019、A01/A05/A09/A15/A17/A19
  条件工具面子断言）：W98 用 `browser-use,computer-use` 构建运行 W95 已修的完整
  `native_coding_reliability`。Coding、General 原生伪调用纠错、General ToolSearch 动态发现及委派
  provider 暂停四项均通过；Browser/Computer 只绑定或发现，重型 Browser Runtime 启动计数保持 0。
  selected workspace、单次文件写、压缩后 evidence、未验证范围和暂停错误分类原断言全部保留。
  首轮 **4/4**（另 2 个显式真实 Provider 用例按设计 ignored），随后完整条件套件 **20/20**，共
  80 个非忽略测试执行零失败。生产代码无需修改。本批没有运行被忽略的真实 Provider、真实 Browser/
  Computer 动作或正式 UI；多次压缩、其他角色/平台及 LONG/99% 仍开放，不关闭完整 Case 或共享阶段。

- S-D01-05（MODEL-002/003/029/034/035、A01/A02/A09/A16/A17/A19 真实 Provider wire 子断言）：
  W99 从既有加密 StepFun 配置和已固定 Session 只读复制 route，先以正式 App 编码器把生产请求送入
  本地 capture server，再将同一请求分别以 streaming/non-streaming 方式直连 `step-3.7-flash`；工具面
  收窄到精确 `exec_command`，所有返回工具仅解析、不执行。两种请求均 HTTP 200、
  `finish_reason=tool_calls`，各恰好一个原生 `exec_command`；arguments 均为有效 JSON，文本 tool markup
  为 false、普通 content 为 0 bytes、实际执行工具数为 0。独立 18 项断言通过，streaming/non-streaming
  分别约 1,398/992 ms；凭据和原始模型正文未写入制品或日志。生产代码无需修改。本批仅一次真实
  Provider 样本，未保留脱敏原始 frame，也未走正式 UI/owner dispatch；其他协议/模型、N3/100 seed/
  failover 与 LONG/99% 仍开放，不关闭完整 MODEL Case 或共享阶段。

- S-D01-06（MODEL-002/003/029/034/035、A01/A02/A09/A16/A17/A19 精确 wire N3 子断言）：
  W100 在两个新的隔离 App/capture server 中重复 W99 的只读 StepFun probe，与 W99 合计三个独立样本。
  3 次 streaming 和 3 次 non-streaming 请求全部 HTTP 200、`finish_reason=tool_calls`，每次恰好一个
  原生 `exec_command`，arguments 均为有效 JSON，零文本伪调用、零普通 content、零工具执行；共 54 项
  独立断言通过。streaming 耗时 1,398/1,302/1,407 ms（中位 1,398），non-streaming 为
  992/1,587/1,712 ms（中位 1,587）。**该精确 route/model/function 的只读 wire 场景 N3 通过**；
  不外推到 owner 执行、其他工具/协议/模型、原始 frame、正式 UI 或完整 MODEL Case，共享阶段未完成。

- S-D02-06（OBS-008/014、LIFE-015/019 子断言）：原生暂停已持久化，前端却忽略暂停通知并
  持续转圈。现按通知重读 canonical 状态，停止活动显示、呈现公开原因并阻断新发送，保留原
  回合及队列所有权；结束回合复用 cancel/释放确认。Windows 99 项定向检查和原失败 Session
  的 Tauri 冷读/结束回合通过，0 新模型步，原事件/效果/快照与文件字节保留。热态有实际 hook
  回归，新模型故障的正式 UI 热态、resume 授权/预算及其他角色/macOS 待验；GEN 原失败仍保留。

- S-D02-05（CTRL-007、AGEN-014/017 子断言）：受保护的 Artifact 发布也误使源文件观察失效。
  owner 回执补绑定根摘要，Runtime 仅延续同根且确定未修改的文件/产物观察；不透明/失败效果
  保留失效边界。summary Schema 明确其为唯一最终答复；Windows 191 项定向检查通过，实际
  StepFun 发送记录确认当前证据及说明未被编码器丢弃。新 GEN 在第 6 步因供应商不可用暂停，
  已完成的 4 项文件效果正确，但 UI 仍转圈且刷新后未显示暂停原因，最终交付未验，仍 FAIL。
  该暂停投影缺口转 W04，原三次 GEN FAIL 保留；证据见 Windows W07，不计 macOS 或 N3 通过。

- S-D02-04（CTRL-007、WIN-005/007 子断言）：完成证据只比较模型参数路径，中文不相干改动会
  误使证据失效，参数中的不同名称也不能证明 junction 别名不重叠。文件 owner 现在返回根摘要及
  实际解析路径；Runtime 只用该观察保留确定不相交的文件证据，缺失/畸形/跨根、失败与不透明
  mutation 均保留失效边界。Windows 200 项定向检查通过，证据见 W07 owner-paths；
  Artifact 证据有效期及最终交付仍需继续修复，GEN 原三次 FAIL 不关闭，macOS 未验。

- S-D02-03（REAL-010、OBS-008/014）：真实命令已取消/reaped，但历史仅接受 Runtime ToolCompleted，
  漏掉取消后的宿主结算，UI 显示“已运行”且无输出。补相同 Turn/call 的有界宿主结算读取、原生
  已清理 process 取消语义、前端终态/标题和停止确认后的历史刷新。Windows 118 项定向检查及
  旧记录冷加载、新 Tauri COD 停止样本通过；历史 canonical 行/绑定/Snapshot 不变。首次 UI
  失败保留，其他角色/复杂清理/macOS 未验，证据见 Windows W04。
- S-D09-01（取消统计）：W04 两个真实样本各有 2 个 model_step_started，但 SDK 外层取消固定
  model_steps=0。W06 补失败反例并按 Turn 保留已记录进度，输出关闭后冻结，旧写入不计入新 Turn；
  准备/一模型步/两模型步取消及 cleanup/late writer 共 14 项通过。无新增付费调用，旧历史不回写；
  修复后的真实模型计数、强杀和跨重启统计仍待验，不计 N3 通过。

- S-D03-05（FILE-037/038、LIFE-006/007 子断言）：Windows 非空目录根拒删仍先删除子项；递归
  中途错误被结算普通 failed。新增原生 DELETE 权限预检；递归错误/任务异常作为删除结果未知，
  保留 pending fence 并清除旧文件列表缓存。Windows 原生 ACL、重启/同 key/新 key/其他写拒绝、
  诊断读与替换保留 DACL/命名流共 60 项定向回归通过。首次失败见 `windows/w03-acl/`；
  其他平台、完整 UI、进程强杀与目录/ACL 并发仍未验。

- S-D03-04（WIN-005、FILE-028/032）：Windows 尚不存在的大小写别名绕过字节路径去重，导致
  multi-file patch 首项已发布、后项才失败。准备期按原生父目录大小写规则逐段判重，拒绝别名及
  祖先冲突，保留 case-sensitive 目录中的合法不同文件。根消失/查询失败不向根外回退或猜测。
  Windows 非管理员原生夹具及定向 50 项通过，首次和中间失败见 `windows/w03-case-alias/`；
  目录置换、其他文件系统/Unicode 等价组、正式 UI 与 macOS 仍未验。

- S-D02-02（AMUL-001、OBS-008/014、MGMT-013 子断言）：旧 Canvas 失败历史硬编码 complete，
  无 pending 的已结算消息仍显示无效 retry。改为由 canonical Turn 补终态并保留消息 ID/公开
  错误；前端恢复 failed/stopped 不冒充完成，重试仅开放给未确认 pending 的末条失败消息。
  Windows 59 项定向测试及原旧会话 Tauri 冷加载/刷新通过；原 Snapshot/绑定、canonical 行及
  画布图未变，0 新模型步/效果。旧“spinner”实际复现为静态重试图标与空白错误卡片，历史失败
  保留；只关闭该展示/无效入口问题，其他恢复与 macOS 未验。证据见 Windows W04。

- S-D03-03（FILE-032/039、OBS-005/016）：W03 复现路径中的标识文本触发错误 unknown 分类，
  以及文件/其子文件同批创建时先发布再失败。前者改为识别 owner 错误前缀，后者准备期检查
  canonical 文件目标的重复/祖先关系；不削弱真实不确定发布的保留与核对。Windows 32 项回归
  通过，首次失败见 `windows/w03-file-errors/`；新建大小写别名和其他平台仍未验证。

- S-D02-01（CTRL-007、AGEN-014/017、OBS）：Windows 三个新 GEN 回合的副作用均完成，
  但都出现一次失效完成引用及后续恢复。提示增强未解决；随后把当前有效 path/call ID 编入
  Runtime 控制工具 Schema，复用实际暴露 Schema 的整批预检。补失效路径提示，保留旧证据 epoch、
  失效/缺失/失败引用拒绝和 Kernel 授权。Runtime 138 项通过；真实模型仍提交不允许的路径，
  最后一次被预检拦截，且最终摘要遗漏所需细节。**问题仍开放**，需继续核对传输与模型交付策略。
  不将守住拒绝边界或最终 recovered 当作正向体验 PASS。证据见 Windows W02。
- S-D07-01（AMUL-001）：画布规划上下文未携带已有名称，真实模型因此无法回答。现在仅携带
  相同 Canvas ID 对应的有界 title；未知时保留 null，节点选择/资源权限不变。context 6 项、
  TypeScript/桌面边界及 Windows 新 Session 同提示回归通过。旧 MM retry spinner 与该问题分开。

- S-D03-02（AUTH-009/010、FILE-036、WIN-007/017）：文件删除入口在 canonicalize 后才删除，
  根内 junction 会被替换成目标目录，空相对路径则指向整个工作区。改为先拒绝根删除并检查原始
  entry 的链接类型；递归删除普通父目录仍由原生 API 只移除内嵌链接。发布临时名不再拼接目标
  basename，避免合法的 255 字符文件名使临时名超限。Windows 首次 3 FAIL 及修复后证据见
  `phase-2-3/2026-09-27/windows/w01b-links-length/`；并发路径置换、macOS/正式入口仍未验。

- S-D03-01（FILE-019～021/038/039；WIN-006/014）：W01-B 的真实 handle 反例确认 Agent
  `write_file` 仍经 `std::fs::write` 原地截断，deny-delete 下错误返回成功；覆盖既有文件还误报
  `created=true`。改为与 patch 共用完整临时文件发布，明确创建/覆盖意图，保留字节上限与失败清理。
  发布或清理结果不确定时保留 durable pending fence，换 key/重启不能盲重放。
  Windows 用保留 ACL 的原生替换并先验证 write/delete 访问；旧文件备份只在成功后清理，部分
  原生失败仅向不存在的目标恢复，出现并发目标则保留原件并要求核对。原始及中间失败见
  `phase-2-3/2026-09-27/windows/w01b-atomic/`；本轮证据仅支持 Windows 组件子断言。

- 2026-09-26：PAL 身份与绑定、MM Skill 首发、进程启动、工具可见性、模板 Action 已有针对性修复；
  Windows 真实路径及其构建身份见 Windows 文档；旧失败证据仍在仓库外。
- 2026-09-27：远端 `8228b61c3` 已提供控制预检、首次计划说明、完成直接收尾、工具参数与模型截断修复。
  [已有公共合同报告](../../reviews/2026-09-26-agent-tool-contract-reliability.zh.md)作为历史证据导入，
  新增修复先检查是否已被该实现覆盖，避免重复或倒退。
- MM 旧失败会话的空白卡片/静态无效 retry 已由 Windows W04 原会话复现并修复；原失败不改记成功，
  M04 及其他实际运行中停止/恢复仍待验。
- 11 个条件能力候选等待对应产品/资源/授权前提，不为完成列表扩权。
- 本批共享 P0 已随 `85a079fc0` 提交并推送；Windows W01 已接续，见平台进度。
