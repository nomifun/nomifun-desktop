# macOS Case 处理进度

更新：2026-09-27。本次执行宿主是 Windows；macOS 原生任务由 macOS 执行者接续。
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
| D03 | 145 | M01/M03 | APFS 大小写/NFC/NFD、权限、原子文件与 Artifact | 待原生走查 |
| D04 | 476 | M01 | /bin/sh/zsh、字面 argv、PTY/process group、Seatbelt 与退出码 | 旧构建部分组件通过；新基线待运行 |
| D05 | 69 | M03 | 隔离 Git remote/SSH、凭据/权限与未知结果 | 条件资源待准备 |
| D06 | 250 | M04/M05 | WKWebView、A11y/Screen Recording、MCP/Plugin/Skill | 需 macOS 权限/设备夹具，不从 Windows 外推 |
| D07 | 124 | M02/M04 | 精确伙伴/画布/知识/客服 owner 和资源 | 引用共享修复，新 Session 复验 |
| D08 | 103 | M02/M05 | 五类 Agent 专属入口/任务；独立产物断言 | 原生全矩阵待走查 |
| D09 | 375 | M06 | watchdog、setsid/丢失 ownership、sleep/wake、恢复/并发/LONG | 故障边界优先，最后 soak |
| D10 | 33 | M01/M06 | MAC-001～018、PORT；arm64 主 lane，x86 按发布范围 | 待原生走查 |
| D11 | 75 | M01/M03 | symlink、Seatbelt/ACL、旧授权和秘密隔离 | 旧组件证据有限；新基线待复验 |
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
