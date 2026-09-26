# Windows Case 处理进度

更新：2026-09-27。当前宿主 Windows。执行顺序见 [实施计划](IMPLEMENTATION-PLAN.zh.md)。
覆盖 675 个共享 + 82 个 Windows 专属 Case；按适用 Agent 展开为 2,374 槽。
Agent 槽数：GEN 600、COD 594、PAL 323、MM 567、CS 242、HOST 48。
共享 P0 提交/推送后开始 W01；不重复维护共享问题的根因文本。

## Windows 专属集合

以下集合共 82 条，其余本平台任务取目录中的 Both Case；范围包含首尾。

| 家族 | Windows 专属 ID |
| --- | --- |
| CMD | 002、006～009、013～014、016、020、022、024、026、028、031～032、037、039、042、044、046、048、050、052、054、057、059、063～064、066、068、070、072、074、077、079、113、124～126、131、133 |
| PROC/TERM/FILE | PROC-005/013/021/045；TERM-011；FILE-021 |
| Browser/Computer | BROW-016；COMP-007 |
| UI/宿主 | REAL-001～015；WIN-001～018 |

## 全领域排程

每个领域的精确家族/ID 范围见实施计划；只取 Windows 适用且该 Agent 有产品目标的槽。

| 领域 | 槽数 | 批次 | 平台测试、排查和修复重点 | 状态 |
| --- | ---: | --- | --- | --- |
| D01 | 425 | W02 | 正式 Session/模型/工具面，中文参数、必填拒绝、版本冻结 | 待走查 |
| D02 | 302 | W02/W04 | 计划/完成、日志与 UI 终态；用户纠正/停止；零红色正向任务 | 首批 COD 有回归；完整集合待走查 |
| D03 | 145 | W01/W03 | Win 路径、共享锁、原子 write/patch/delete、Artifact | 待走查 |
| D04 | 476 | W01 | executable/args/cmd、PowerShell/cmd、编码、Job/ConPTY、退出与清理 | CMD-147 首批通过；新基线待走查 |
| D05 | 69 | W03 | 本地隔离 Git remote、SSH 夹具；取消/未知副作用 | 条件资源待准备 |
| D06 | 247 | W04/W05 | WebView2 profile、Computer A11y、MCP/Plugin/Skill | Skill 首发已回归；其余待走查 |
| D07 | 124 | W02/W04 | 精确产品目标、Knowledge/Companion/Canvas/Customer owner | 引用 S-P0-03；本机入口待回归 |
| D08 | 103 | W02/W04/W05 | GEN/COD/PAL/MM/CS 分角色的正式任务 | 四条旧构建路径有证据；其余待走查 |
| D09 | 375 | W06 | Job 子孙进程、崩溃、恢复、撤权、并发与 LONG | 先故障注入，后 soak；待走查 |
| D10 | 33 | W01/W06 | WIN-001～018 与 PORT，盘符/路径/共享锁/宿主终态 | 待走查 |
| D11 | 75 | W01/W03 | 越界路径/junction、旧授权、无资源与跨 owner 拒绝 | 组件负向优先，待走查 |
| **合计** | **2374** | W01～W06 | 结果按 Case × Agent × Windows 独立判定 | 未跑不算通过 |

## 近期可领取任务

| 任务 | 对应 Case / 断言 | 测试与修复安排 | 状态 |
| --- | --- | --- | --- |
| W01-A 命令形状与启动 | G0-003/004/010；PROC-001～004；WIN-008/009；CMD-131/133/147 | 先验共享进程合同与 Windows argv/policy；失败保留 command/args、exit/cleanup；修 owner 后回归两种合法入口及混合形式拒绝 | 等共享提交后运行；组件结果不能代替全部 UI/CMD PASS |
| W01-B 路径与权限边界 | AUTH-009/010；FILE-021；WIN-002～007/017 | 有空格/中文、保留名、UNC/ADS、大小写、junction、锁与长路径夹具；每条先确认可用测试再判覆盖 | 待走查，跨盘/ACL 等缺夹具单独阻断 |
| W01-C Job 与终端清理 | PROC-027～036/045；WIN-010～012/014/016 | 进程树、stdin/ConPTY、leader 先退出、编码、模拟锁；核对真实 cleanup receipt | 待走查，不以终端文本或 parent 退出代替清理 |
| W02 正式核心入口 | AGEN-001/014/017；ACOD-001/008；APAL-001；AMUL-001 | 新 Session/冻结快照，复测资源/Skill 合同与已修首发；工具副作用由磁盘/DB 只读 oracle 判定 | 共享合入后排程 |
| W03 文件/Git/SSH | D03/D05/D11 剩余适用 Case | 先根内读写负向，再隔离 remote/SSH 与外部效果；逐簇修复 | 待走查 |
| W04 UI/扩展与恢复 | REAL、OBS、D06；旧 MM retry | 先复现旧失败会话按钮/状态/Promise 与 canonical Turn，定位 spinner，保留首次失败 | 待走查，不重写旧 Snapshot |
| W05 条件业务资源 | ACSR、媒体、Channel、Robot 及其 D07/D08 Case | 建最小正式入口/模型/测试租户；无前提不计 PASS，不擅自扩权 | 部分 BLOCKED_FIXTURE |
| W06 生命周期与长稳 | LIFE/CONC/LONG、WIN-015/016/018 | 逐状态故障注入、取消/重启/lease、宿主 UI/API 一致性，最后长稳统计 | 待走查 |

## 已有结果与本批记录

- 2026-09-26，旧构建 `4d525c84d384`：APAL-001 名字正确（1 模型步）、AMUL-001 默认 Skill 首发成功（3 步）、
  GEN CMD-147 命令与 7 项属性准确（2 步）；各自零工具错误，原失败保留。
- 旧构建 `e67a8810d01d`：ACOD-001 相同只读请求 5 模型步、零工具错误、三文件哈希未变；此前回归的 1 次计划错误保留。
- 这些结果来自此前隔离 Tauri；新合并构建未执行的 Case 仍未验收，不能直接移植旧 PASS。
- 本轮 Windows 证据根：仓库外 `phase-2-3/2026-09-27/windows/`。本页只保留批次结果和残余断言。
- 下一步：共享 P0 commit + push 后，从 W01-A 开始；不重建 2,374 行日志/状态文件到 Git。
