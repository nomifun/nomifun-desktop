# macOS Case 处理进度

更新：2026-09-28。当前已由 macOS arm64 原生执行者接续；此前 Windows 结果仍只作共享历史引用。
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
| D01 | 425 | M02 | Session/Broker、系统代理/loopback、模型/工具 Schema 与冻结版本 | 已导入部分历史验证；新构建待原生复验 |
| D02 | 291 | M02/M04 | 控制/完成、原生 UI、停止/纠正及错误可见性 | 已导入公共合同修复；完整 Case 未验收 |
| D03 | 145 | M01/M03 | APFS 大小写/NFC/NFD、权限、原子文件与 Artifact | M01-02 APFS/权限/symlink 子断言已验证；其余待走查 |
| D04 | 476 | M01 | /bin/sh/zsh、字面 argv、PTY/process group、Seatbelt 与退出码 | M01-01 原生 owner 子断言已验证；正式 Tauri/CMD 全集待验 |
| D05 | 69 | M03 | 隔离 Git remote/SSH、凭据/权限与未知结果 | 条件资源待准备 |
| D06 | 250 | M04/M05 | WKWebView、A11y/Screen Recording、MCP/Plugin/Skill | 需 macOS 权限/设备夹具，不从 Windows 外推 |
| D07 | 124 | M02/M04 | 精确伙伴/画布/知识/客服 owner 和资源 | 引用共享修复，新 Session 复验 |
| D08 | 103 | M02/M05 | 五类 Agent 专属入口/任务；独立产物断言 | 原生全矩阵待走查 |
| D09 | 375 | M06 | watchdog、setsid/丢失 ownership、sleep/wake、恢复/并发/LONG | 故障边界优先，最后 soak |
| D10 | 33 | M01/M06 | MAC-001～018、PORT；arm64 主 lane，x86 按发布范围 | MAC-005～010/015 的 process 子断言已验；其余待走查 |
| D11 | 75 | M01/M03 | symlink、Seatbelt/ACL、旧授权和秘密隔离 | M01-01/02 已验 Seatbelt 与文件 symlink/mode；ACL/旧授权待验 |
| **合计** | **2366** | M01～M06 | 平台结果独立保留 | 本 Windows 执行者不代判 PASS |

## 原生接续任务

| 任务 | 对应 Case | 测试 → 排查 → 修复安排 |
| --- | --- | --- |
| M01 路径与进程 | MAC-001～010/013/015/016；FILE-022；PROC-006/014/046；TERM-012；CMD-132/146～150 | 先重跑 macos process 合同；补卷属性/NFC/NFD/argv/group/Seatbelt 夹具，再做 Tauri 命令首发 |
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
