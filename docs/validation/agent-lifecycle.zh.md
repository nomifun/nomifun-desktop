# Agent 生命周期验证记录

更新时间：2026-10-24。稳定行为合同属于 [Agent Execution](../architecture/agent-execution.zh.md#生命周期约束) 和 [Agent Session](../architecture/agent-session.zh.md)，资源与退出边界同时遵守 [Browser](../architecture/browser-platform.zh.md)、[全局 OS](../architecture/global-os-capabilities.zh.md) 和 [Computer Use](../architecture/computer-use.zh.md)。本页仅记录验证范围、构建证据和未验项，不是施工计划，也不授予安装、提权、提交、推送、发布或修改真实数据的权限。

原 lifecycle 施工计划与启动 prompt 已由上述正式合同及本验证页替代，历史设计可查 Git。U01–U13 长期约束保存在 Execution 架构；Computer 专项的实施阶段、授权和新 worker 验证继续由其[现役专项](../handoffs/computer-use-rebuild.zh.md#12-单份执行进展)管理，不继承旧 Computer 的原生通过记录。

## 当前 Windows 验证与恢复

以下各项按其真实构建和执行集合计证，不能把重叠用例相加，也不能将早期通过扩大到后续改动。私有日志、可执行文件身份、隔离 fixture 与报告留在原 `build.noindex`／`.git` 证据目录；本页不复制数据库、凭据、用户正文或工作产物。

| 范围 | 最新已完成结果 | 证据与边界 |
| --- | --- | --- |
| App 完整 lib | **1307 通过／4 原 ignored／0 失败** | `task-restart-app-full2.log`，桌面实际Browser／Computer features，默认测试栈、2线程。完整运行正常结束，未出现native AV；原ignored不计通过。 |
| Execution／Runtime | **193 通过／535 通过＋1 原 ignored** | 子任务重启修复后的完整集合；保留用户并发、原预算及正常控制；原 ignored 不计通过。 |
| Session | **402 通过／0 失败** | `task-restart-session-full2.log`；当前完整运行通过。更早393项的分批记录仅属于历史构建，不再作为本轮总计。 |
| DB／UI／静态 | **DB lib321、Execution repository90、UI交互51通过；完整 `bun run check` 通过** | `task-restart-db-full1.log`、`task-restart-db-repository-full1.log`、`task-restart-ui-final.log`；集合分列，不累计重叠用例。 |
| Step Plan 真实2子 | **2 Full／2 Attempt／1 delegate／1父Turn；重叠23284ms** | 两份原结果完整消费，父整合文件并真实进程验证；wait_calls=1、status_calls=1、parent_model_steps=10、wait_model_calls=0。报告 `attempt-handoff-live-two.json`，build `6cd8a8f054dabbaa96ee0de1f243a7740c2c81f8d35a0aa2a52d6d0646b454ce`。 |
| Step Plan 真实8子 | **8 Full／8 Attempt0／1 delegate／1父Turn；默认max_parallel=4，domain/native peak=4，四路重叠35969ms** | 八份原结果完整消费，父读取八原文件、只写集成文件，验证进程exit=0/reaped=true；wait_calls=1、status_calls=1、parent_model_steps=9、wait_model_calls=0；最终答复2879 bytes且含验收标记。报告 `attempt-handoff-live-eight-evidence.json`，build `d14262bceb5e2bd17629424037ca90374e81888c5e5b08ca67eb548b401208ce`。两子与八子的build分别保留，不冒称同一构建。 |
| 实模隔离与保留 | **正常关闭、明文凭据审计通过** | step-5-preview凭据仅私有stdin进入测试；8子隔离fixture `subagent-native-30POzj` 在专用`.git`目录保留，供原canonical事实复核。保留选项不放宽测试、关停或凭据审计。 |
| 指定真实会话恢复 | **`01a16826-d942-73b0-b9a3-557c69215e6b` 原Turn在seq3081完成，Attempt仍12，其他会话保持原样** | 正式ReplaceWithExisting与原native resume，未重跑子任务或假造终态；原数据／工作目录备份与逐表审计留原位置。证据 `attempt-handoff-real-final-delivery.json`。 |
| 原游戏产物复核 | **独立副本10项JS语法／47项冒烟通过** | 只运行副本中的原canonical脚本，不修改真实工作目录。证据 `attempt-handoff-restored-game-verification.json`。 |
| Desktop／worker | **子任务重启版本的dev Desktop与独立worker构建通过** | `task-restart-desktop-build.log`／`task-restart-worker-build.log`，身份见`task-restart-desktop-identity.json`；channel=dev、OfficeCLI内置开启、官方Vite模式。未对真实数据启动新构建或迁移旧checkpoint。01:18原build的启动核验仍保留，但不能当作本轮新build启动证明。构建不等于安装、发布或各平台原生支持证明。 |

本轮受控贯通验证包括原输入Busy/丢ACK、prelink准备、旧owner失权、同Step幂等Retry、兄弟不重跑、父来源控制、真实问题回答、纯WAIT冷恢复、精确Stop与最终父交付。空创建退休覆盖原Attempt持久裁决、Queued后继保留、同writer输入/link竞争与真实两连接WAL。完整实现合同见架构；模拟故障与真实模型证据分别保留。

最终dev宿主于2026-10-24 01:18（Asia/Shanghai）启动，Desktop SHA256为`09e4d842623a5465854e90022566e438a88d0072d53784c11b9c7c4a81a39fad`，相邻worker为`4031cc7a2e4d05297fa1913307dfc9c10b7a1624d548271f988be457abd7123c`。真实目标仍为原Turn的`turn/completed` seq3081、Execution completed/version40、12 Attempts及max_parallel=4；九个子Session的canonical历史逐行与备份一致。另两条受保护会话的状态及seq137/1588未推进。过程中的raw Cargo首次候选误用stable身份，已通过现有认证退出接口正常关闭并按正式dev配置重建；只撤销了确切指向该误启动候选的stable协议command/icon值，其启动前的stable handler未留快照，未猜测重建。正确dev协议现指向最终候选，共享键盘服务保留。开发窗口标题仍沿现有源码显示`NomiFun`，不能用窗口标题代替编译身份核验。

## 2026-10-24 子任务重启与技术暂停交接

对应真实诊断会话`01a16b9b-c3e5-7312-a110-a499a9c00a0e`：7个孩子已交付，SA-3的模块自测71/71通过，但4次完整报告的summary依次2371／2363／2130／2130字符，均被旧2048字符字段限制拒绝。原native Turn以`EXECUTION_INTERVENTION_REQUIRED`暂停，领域Attempt仍Running；纯观察job被计为推进者，父All等待继续Park；已受理的“重启执行”Steer又排在未ACK的InitialInput之后。只读证据`session-01a16b9b-wait-audit.json`保留在私有`build.noindex`。本轮未改写该真实会话或跨build恢复其checkpoint，旧匹配Runtime可执行文件与身份已单独保留。

正式合同现归[子任务重启与暂停交接](../architecture/agent-execution.zh.md#子任务重启与暂停交接)。完整报告沿既有48KiB整包预算，移除独立2048字符上限；handoff沿原80KiB整包预算，严格历史发布的短引言仍是短引言。重启只使用原Execution事件派生pending，保留原Step与父来源；没有新表、第二执行账本或并发调整。原InitialInput按精确canonical准入ACK；技术暂停保留typed身份，真人native恢复、普通重试、原生清理和User Stop分别验证。成功受理不反复唤模型，新blocked回执按原来源交给父一次。

| 范围 | 本轮结果 | 私有证据与边界 |
| --- | --- | --- |
| Contracts／Runtime | 133通过／535通过＋1原ignored | `task-restart-contracts-full1.log`、`task-restart-runtime-full1.log`；原ignored不计通过。 |
| Execution | 193通过 | `task-restart-execution-full2.log`；默认测试栈，原并发与控制合同保留。 |
| Session | 402通过 | `task-restart-session-full2.log`；含7项Restart native writer直达／排队、fence、伪Actor、Stop与丢ACK回归。 |
| DB／仓储 | 321通过／90通过 | `task-restart-db-full1.log`、`task-restart-db-repository-full1.log`；含原暂停核验、准入ACK、Queued推进竞态、未知效果及Stop优先。 |
| 父子贯通 | 定向1项通过 | `task-restart-app-e2e7.log`；本地受控provider，Session／Runtime／Effect／命令与恢复使用生产链，不是外部实模。两child、4次真实参数拒绝、一次真人Restart及其同key重放、一次native授权、原child Attempt／Turn与原parent Turn均保持、健康兄弟仅调用一次、原文件仅写一次；父最终交付，两child均completed/result_ok=true。等待不轮询模型；恢复按现役规则做一次必要计划更新及一次有效报告，不重跑原文件操作。 |
| UI／静态 | 最终单次51项交互通过 | `task-restart-ui-final.log`；取代有重叠的前两次48／41项记录，不累加。typecheck、i18n、desktop boundary通过，完整`bun run check`通过。 |
| Desktop／worker | dev Desktop及配套worker构建通过 | Desktop SHA256 `115f64ef30d1d1a04dbfa7e0260449edafd058f7086f2516de8aca4fec4bb6bd`，worker `0ffad594e7858a87a0a1b5dc9b3d916753132909db6fa8f0dfab841cba0ca67f`。未启动真实数据、打包或安装。 |
| App完整 | 1307通过／0失败／4原ignored | `task-restart-app-full2.log`，563.43秒；首次1305通过／2个旧cleanup测试未处理新增的typed pending／4原ignored。两测试已按真实pending合同更新，相关22项及完整重跑均通过；生产代码未因此放宽。 |

初轮失败保留：新Scheduler fixture的FK顺序、新Session权限反例的非法actor UUID均已修正测试数据，未放宽生产规则；完整Session及Execution随后通过。首个App筛选为0项，未计证明。后续贯通夹具曾在plan未广告时调用control、或在native恢复后遗漏必要replan，导致本地provider断言退出；修正为正式文件写入／读取后建原plan、native恢复只更新原plan再报告，文件操作仍一次。最终受控贯通通过，失败日志保留，不把夹具错误解释成供应商故障，也不以放宽原交付断言取绿。

## 当前限制与失败记录

- 早期 App 与 Session 候选各出现一次 Windows **0xC0000005**，当时未跑完的集合不能计整体通过。两次访问违规尚未归因；缺少有效现场证明，不能把它归因于已修复的其他栈／布局问题。后续完整App运行和Session完整运行未复现、默认栈重复定向20次通过，只证明这些重跑成功，不能宣称已经证明修复原AV。
- 8子首轮父状态为completed，但旧`EIGHT_PARENT_FINAL_MISSING`混淆答复缺失与缺验收标记；原fixture未保留，因此无法进一步判定那次具体内容，也不能算通过。测试现区分空答复与非空缺marker，先核验全部工作事实再检查marker；原prompt和验收条件不变，后续独立8子运行完整通过。首次retained runner因SDK定位变量缺失未执行，属于工具链准备失败。
- 前一构建的dev实际启动已核验；本轮重启修复的新Desktop／worker已构建，未对真实数据启动或跨build恢复旧checkpoint。发布包装以及新整图macOS/Linux原生测试仍须分别记录。Windows测试、旧平台原生通过和早期真实模型均不能代替这些证据。
- 已证明并替换“SQLite暂态Busy被判为失权，继而丢掉原业务推进责任”的永久卡住链路；真实SQLite屏障用例也证明独立owner能在相同争用下收束。历史最初持锁者、确切SQL及其长持锁原因缺少完整原始记录，仍不能唯一还原。pool耗尽、connection scope总时长均不能独自证明连接泄漏或实际持锁时长。
- 真实会话的安全恢复是本次明确授权的操作；隔离验收通过不授权对其他真实会话执行恢复、重试或数据修复。另一旧paused会话与受未结算Effect保护的3条输入保持原状态。

## 平台证据边界与待验项

本表是当前边界。后文历史10.4表中“macOS/Linux未运行”仅是当时快照，已被历史10.5/10.6的实际运行部分更新；不能抹掉后续原生证据。旧平台build通过也不等于本轮新整图通过。

| 平台／范围 | 已有证明 | 仍未证明／适用边界 |
| --- | --- | --- |
| Windows | 本轮共通Rust/UI、2/8真实模型和指定会话恢复；历史原构建已有Job/ConPTY/父死、WebView2、Attached Chrome、真实Edge桌面控制及稳态证据 | 当前构建的完整原生组合不能借旧build通过；真实高完整性/UAC多client、权限等待、睡眠／锁屏、系统退出／重启、真实chooser选择与下载字节等未验组合按原证据边界保留。新Computer以其现役专项为准。 |
| macOS | 历史v4实际AppKit/Cmd-Q、WK basic/pending dialog与chooser取消、精确Stop、父死、Attached Chrome、FD/PTY稳态和桌面WebUI均有原构建证据 | 本轮新整图未在macOS运行；TCC撤销／授权等待取消、SMAppService/XPC管理员多client、睡眠／锁屏与系统注销等受控场景仍待验。Intel、最低macOS14、正式签名/notarization与发布范围未由这些记录证明。chooser取消不等于真实选文件/保存下载字节。 |
| Linux | 历史WSL2/X11隔离图已验证pipe/PTY、精确Stop、SIGINT/SIGTERM、SIGKILL父死、原生资源退出与稳态 | 本轮新整图未在Linux运行；真实GNOME/KDE Wayland portal授权／拒绝／撤销、pkexec多client、托盘／GUI Quit／重启／系统退出、睡眠／锁屏、桌面浏览器WebUI仍缺证据。WSLg/XWayland、私有D-Bus及mock不替代真实Wayland。 |
| 当前provider能力 | Desktop provider注入决定适用矩阵 | Linux当前未注入managed/Attached Browser，相关原生项N/A且不新增后端；Computer新worker支持范围、Wayland限制和待验项引用现役Computer架构／专项，旧实现native结果不能当新worker证明。 |
| 跨平台共同规则 | 原owner、generation、原生退出与资源收据分别验收 | 不按进程名/PID缺席猜清理；不误杀用户Chrome、共享WebContent、合法后台写者或外部owner服务。环境缺失不成为Agent的新运行资格，也不授权安装、提权、锁屏或影响日常桌面。 |

## 现役验证入口

按实际改动、cfg/features、平台provider和源码／构建身份选择下列入口；检查非零实际用例数。构建、模拟、真实native、实模和发布包装分别计证。

```text
cargo test --locked -p nomifun-agent-execution --lib
cargo test --locked -p nomifun-agent-session --lib
cargo test --locked -p nomifun-app --test native_execution_recovery --test desktop_agent_shutdown
cargo test --locked -p nomi-process-runtime --test parent_death --test pty_contract --test process_contract
bun run check:agent-session-boundary
bun run check:process-runtime-boundary
bun run check:browser-platform-boundary
bun run check:desktop-ui-boundary
bun run check:docs
```

macOS还使用原`darwin_registered`、`darwin_application`及[Browser原生入口](browser-platform.zh.md)，需要真实AppKit主循环、原controller与LaunchServices回执；打包或普通cargo run不替代原生运行。Linux沿现役Desktop features和[Linux验证记录](global-os-linux-2026-10-17.zh.md)核对实际X11/Wayland、portal及权限；不默认调用会安装依赖或打包的脚本。真实模型使用现役`run-nomi-core-live-provider-smoke.mjs --subagent-smoke`或`--subagent-eight-smoke`；凭据仅私有stdin，保留fixture必须先正常关停和通过明文审计。

环境记录包含OS／架构、源码／构建身份、工具版本、普通／管理员身份、桌面会话、实际provider、权限、隔离范围、命令／非零用例数、失败与原生退出回执。资源稳态记录同负载多轮、合法长期任务、句柄／FD／进程／PTY、cleanup backlog、SQL量及可取得的CPU/RSS，不要求allocator立即返还全部内存。

## 共同行为验证矩阵

以下ID保留原验收含义，与具体构建结果分离。新改动只补其相关竞争与负向验证；不能把矩阵中的目标直接记为已通过。

| ID | 场景 | 必须满足 |
| --- | --- | --- |
| T01 | 两个独立子任务运行，停止一个 | 精确目标终止，无关兄弟继续，Execution 不被默认取消。 |
| T02 | 停整个协作 | 所有适用未完成工作受取消约束；迟到回执不重开；既有整组取消用例保留。 |
| T03 | DAG 菱形／共享汇总节点，停止一个委派范围 | 归属与依赖不混淆；无关分支不被误取消，真实缺失输入有解释。 |
| T04 | 停止与子任务派发同时发生 | 停止范围内已存在和竞态派生均不能绕过；不靠只枚举当前列表。 |
| T05 | 父 Turn 正常结束／前台切走 | 已授权委派继续，可从任务入口手动控制。 |
| T06 | 原生可恢复任务暂停／继续 | 不创建替代 Attempt，不重复已结算工具，不丢原进度；当前不安全时不显示已冻结。 |
| T07 | 无 checkpoint／不兼容／unknown effect 时继续 | 不重跑、不伪造恢复成功；准确显示原因，停止与清理仍可操作。 |
| T08 | 多个等待者同时唤醒 | 重新获取原预约；不超出原 Execution／Participant 上限，不失败、不 kill、不循环暂停。 |
| T09 | 后台 shell 持续写、模型表面等待 | 不误判无写者、不强制只读；现有共享工作区合同不被破坏。 |
| T10 | Stop／Steer 慢、失败、重复、重启 | 常规控制因果顺序和幂等保持，用户 Stop 可抢占；其他 Attempt 持续结算；不退出整个调度集合。 |
| T11 | 两租约均有效、仅 Execution 过期、仅 native 过期、均过期四种组合 | 同一原 Turn 正确交接；不因暂时无终态退休 link 或误取消，不重复效果；旧代际迟到结果不覆盖新恢复决定。 |
| T12 | 停止持久化后立即崩溃重启 | 不恢复已取消业务，只继续必要清理；未持久确认的状态不冒充确认。 |
| T13 | 旧控制／旧 cleanup 与后继 Turn 竞争 | 后继不被取消、不丢资源，旧命令不重新绑定当前目标。 |
| T14 | 通知在订阅边界丢失／UI 重连／owner 通知关闭 | cursor／receipt 补读能收敛；不永久等待，不以模型调用探测状态。 |
| T15 | 不同 Agent 使用不同 Attached Chrome tab | 底层支持的独立操作可推进；同 target 保序，现有 claim／incarnation 不失效。 |
| T16 | Chrome 调用准入与 disconnect 并发 | 关准入后无漏登记新调用；已登记者被准确取消与收束，不误杀用户浏览器。 |
| T17 | 新 Computer worker 排队、执行与收尾时 Stop | 排队撤回零效果；在途保留实际结果，超时强杀 unknown 且不重放；真实平台回收独立验收。 |
| T18 | 单个可独立终止 worker 被用户强制结束 | 不结束共享 host、用户 Chrome 或健康兄弟；已提交外部效果不冒称撤回。 |
| T19 | Turn 成功且知识回写仍在进行 | 显示原成功，回写自然完成；自动退休不 cancel_all，不拖成新的主任务完成门槛。 |
| T20 | Session 后台进程跨 Runtime soft recycle | 原 owner、原句柄与控制有效，无命令重启；上层无用引用可按真实责任退休。 |
| T21 | cleanup 前 100 项持续失败，存在第 101 项及不断新增项 | 后续项得到处理；失败不挡新业务，循环有合理节奏。 |
| T22 | 已无活动 Turn 的完成 child | 不仅返回 cleanup success；真实 Runtime 退休与后台资源状态分别可验证。 |
| T23 | 等待／暂停期间清理纯派生缓存，注入加载失败 | Session／Turn／Attempt、cursor、调用次数、计数、控制事实不变；无 send／retry／新进程。 |
| T24 | 停止本次自动运行、停用后续计划、应用正常退出 | 分别走正确 owner；语义和重启行为不互相替代。 |
| T25 | 任意 Agent 模型不响应、异常、恢复或清理中 | 用户能找到目标、提交停止、查看未确认资源；按钮不依赖模型回复。 |
| T26 | 固定负载多轮短任务完成＋独立长期任务 | 废弃任务、订阅、Host、句柄、缓存无逐轮累积；合法长期工作不被 kill 或清空。 |
| T27 | 普通构造失败／可归属异常／归属未知 | 可证明局部的失败隔离；未知共享归属不被吞掉或错误释放。 |
| T28 | 原有工具与产品回归 | 伙伴、普通会话、协作、自动任务、shell／文件／Browser／Computer／MCP／Terminal、已授权资源与长任务仍可使用。 |
| T29 | 同一 Attempt 的 Steer 永不返回，此时用户 Stop | 停止不等待 Steer 业务完成，可持久接受并送达原 owner、禁止后续工作；原 Steer 仍由 owner 收束，迟到回执不能复活任务。 |
| T30 | 完成一批任务，用户不关闭任何历史会话 | 任务专用和无剩余责任的托管 browser／进程／连接自动退休，资源数量回落到必要基线，不依赖用户点击 close；历史仍可查询。 |
| T31 | 用户手工浏览／编辑、跨 Turn 继续浏览，同时另一任务完成 | 用户现场与活任务不被任务完成／隐藏面板误关；无用户保留责任的任务专用资源仍能自动退休，不把所有 Workspace 永久 pin。 |
| T32 | 真正 Quit，存在隐藏面板、旧 Session 和已终态驻留资源 | 所有应用自有 browser／headless／PTY／MCP／worker／连接按原 owner 收束，不等用户逐个关；历史、Profile／登录与产物保留。 |
| T33 | crash 或 OS 强杀宿主，存在应用自有子进程与重资源 | 三平台适用 Job／watchdog／parent-death 路径实际收束精确独占资源，不靠 finally 或等下次启动才杀活子进程；共享 Chrome／WebContent 不误杀。 |
| T34 | SSH／知识源等清理失败，Browser 等其他资源可安全关闭 | 各域准入先关闭，互不依赖的资源清理仍被执行；真正共享依赖继续保留，不跳过所有 Browser cleanup，也不假报全部退出成功。 |
| T35 | 重开已自动退休资源的历史会话 | 查询历史不启动业务；用户明确使用时正常懒创建资源，保留适用 Profile，不重跑原 Turn／Attempt／网页输入／命令；不声称任意网页 JS／表单已完整恢复。 |
| T36 | App 退出时连着用户 Chrome，包含原有标签和任务临时标签 | NomiFun 连接／claims／overrides 全部收束；用户原有浏览器和页面保留，仅精确独占且可退休的临时页面按真实归属处理。 |
| T37 | 普通任务后台子进程与已合法移交的独立服务并存 | 普通应用私有子进程不逃逸真正退出；独立服务按既有外部 owner 合同保留；Agent 文本不能自行授予跨退出持久性。 |

故障注入尽量使用屏障、可控通知和虚拟时间，不通过随机 sleep 得到偶然绿色。检查实际执行的测试数与 feature，不能接受“命令成功但相关用例被过滤为零”。

## 原生平台验证矩阵

| ID | 场景 | 平台要求与通过标准 |
| --- | --- | --- |
| OS01 | 启动与取消竞争、宿主骤退、孙进程／leader 先退出 | 三平台 pipe＋PTY；使用原 owner 的身份与退出证明，无误杀其他 Session，无旧 PID／PGID 重用伤害；逃逸或无法证明的对象明确未确认。 |
| OS02 | EOF、中断、取消、强停与终端关闭 | Windows 单独验证 ConPTY；macOS／Linux 验证 PTY job control。关闭输入不等于退出；进程、管道、PTY／HPCON 与原生关闭任务都可核对。 |
| OS03 | managed Browser 在导航、modal、popup、下载或 chooser 中停止／退出 | Windows WebView2 与 macOS WK 分别真实验收；保留主事件循环至收束，旧 callback 不命中新对象；Linux 按现有 provider 不适用并注明源码依据，不要求新增后端。 |
| OS04 | Attached Chrome 跨 target 并行与断连竞争 | Windows／macOS；关准入后无漏登记调用，释放 claims／override 有原回执，用户 Chrome 不被关闭；Linux 当前 Desktop 不适用。 |
| OS05 | 新 Computer 排队、权限等待与执行中停止 | 三端分别验证私有 worker 的精确 owner、用户 Stop、崩溃、应用退出与输入释放；Windows 普通完整性、macOS TCC、Linux 后端按本次实现及实际环境计证。 |
| OS06 | 单任务 stop、单 client close、管理员设施 shutdown | 三平台分别验证三个作用范围；共享 runner／daemon 的其他 client 继续；认证等待可取消，设施重连不重放 unknown。 |
| OS07 | 关闭窗口、明确退出、重启、crash／强杀、系统退出入口 | 托盘保持合法后台工作，已完成资源继续自动退休；真正退出覆盖所有旧／隐藏托管载体。macOS 补 AppKit／Cmd-Q，Unix 补 SIGINT／SIGTERM；强杀另验原生父死收敛，无关清理失败不跳过可安全释放资源。 |
| OS08 | 系统睡眠／唤醒、锁屏／解锁及租约过期交错 | 核对该平台时钟与唤醒行为，覆盖 T11 四种组合；不新增静默／睡眠 TTL，不重置原预算，不因界面不可用自动从头开始。已有明确任务期限仍按原合同生效。 |
| OS09 | Runtime 退休与资源稳定性 | Windows 记录 native handles／Job／ConPTY；Unix 记录 FDs、进程、watchdog、PTY；按实际支持记录 WebView／WK、portal／PipeWire／worker。固定负载结束后无废弃资源逐轮累积，后台服务不误杀。 |
| OS10 | Desktop 与桌面级 WebUI 的用户控制 | 共享后端合同保持一致；UI 卡住／重连不使控制依赖模型，原生主线程繁忙不阻止宿主接收控制；WebUI 断开不是取消任务，renderer 不新增移动端规则。 |

## 历史构建与平台证据

以下保留原构建身份、命令、计数、原生回执、首次失败与未运行边界。原编号仅用于识别历史记录；其中“当前／最终／正在／待验”等词限定在该记录的日期和构建，不构成新的施工入口。是否已由后续验证覆盖，以本页前部和各原始报告为准。原提交／推送授权仅属于当时任务，不向后续工作转授。

### 2026-10-23／24 启动交接修复的中间构建与恢复记录

用户已授权完整实施、继续定位数据库争用并安全恢复 Session `01a16826-d942-73b0-b9a3-557c69215e6b`。起点 `c3eb1a85b0970e868f47d84d15007e08c3d5c87e`，保留已有 129 项工作区改动，不提交／推送。以下替换此前仅设计阶段的进展说明；不对真实库注入故障、不跳过 checkpoint build 围栏、不伪造终态。

**根因证据。** 原 Execution 仍为用户配置 max_parallel=4。17:51:42.687 原04提交 Running，17:51:58.555 heartbeat 的 SQLite Busy 被错误裁决为失去租约，旧代丢弃借用的 running_jobs；17:52:10.100 新代把 Missing 当作永久 Review 并停用 link，实际应用没有重启。原04只有 opening/active-set/ready，0 accepted/Turn/Effect；Running 不证明模型执行。on_started 还等待 UI outbox drain，Running 的发布延迟27.404秒，业务交接不应依赖发布完成。新建04b不会正式结清原04，且历史 delegated_work 取消选择为空，不等于取消自身。显式WAIT省略cursor时错误重复报告相同Review，形成33次wait/34次status；自动WAIT在model99沿原cursor停驻。原始只读证据保留在 `build.noindex/session-01a16826-*-evidence.json`，不再保留一份竞争设计。

**已实现合同。** 原 TaskTracker 独立托管 Step job；heartbeat 区分暂未确认和确定失租约，换代不丢原调用。InitialInput只保存完整原request，operation key从原Attempt派生；on_linked原writer提交精确link和原意图，不等待发布。canonical accepted+started同writer核验lease/Step/Attempt/link与完整原输入。Session输入owner持续负责至原native claim/SDK接管或精确终态；重复accepted ACK不是重新发送许可，父问题回复共用该owner。稳定Step下幂等Retry、精确历史Attempt控制、父来源TaskControl、Review正式核验与ReplaceWithExisting均沿现有事实链；未新增表或第二账本。显式／自动／重放WAIT共用scope cursor，逐Step保留消费与分页，不因租约元数据或已消费结果移除重复调用模型。UI接入核验及正式已有成果替代。

**数据库争用。** begin_agent_write诊断贯穿原事务，区分pool等待、BEGIN等待、writer持有及commit/rollback ACK；不改pool、timeout或用户并发，不加诊断SQL。真实SQLite、5连接、1原writer+3 peers+outer waiter回归2/2通过：旧borrowed future停止poll时，即使release已到writer仍无法COMMIT；独立owner同负载可以完成。历史首个writer未能唯一确认，不能用包含BEGIN等待的27.463秒connection scope冒充实际持锁时长。Busy→后续明确永久准备拒绝也必须重新分类，不能永远沿用第一次Busy；已补反例，待最终运行。

**已完成验证。** 默认线程栈、2测试线程：Session383/383、DB lib315/315、Runtime fresh 533通过/1原ignored（含纯WAIT新矩阵）、Execution lib181/181、Execution repository86/86。App输入owner故障6/6；父重试完整交付及原输入Busy恢复2项通过；ExistingChild续作、四Full子并行、同writer authority及WAIT分类均通过。UI交互14/14，typecheck、desktop UI boundary、canonical boundary与fresh `bun run check`通过。最后补充的prelink准备Busy、Busy→永久拒绝和纯WAIT冷恢复尚待fresh编译重验；当前不把旧二进制通过扩大到后续修改。首次失败的时钟fixture、旧outbox观察、source准备错误依赖Running link及输入owner迟到terminal竞态均已按真实合同修复，失败记录保留在私有build.noindex证据。

**编译边界。** App check3 rustc栈失败；拆出boxed输入owner后check4给出async layout深度130超过既有限制128；共享Session writer再加boxed边界后check5通过（3m08s）。未提高编译器栈或recursion limit，测试exe运行前移除RUST_MIN_STACK。candidate3测试构建完成；等待最后修改冻结后统一编译lib和实模integration。

**真实恢复。** 原库和工作目录已备份至 `build.noindex/attempt-handoff-original-preserved/`：DB SHA256 `342fa312bbabec22f972b6d7c42205eaf31c18b8ed05fa9eee01b3a7c1573fb0`；ZIP SHA256 `c4582b70e0b3a864cb5786380a1acc459e6fc146fb4fc168326d0ad0427720a5`。先在副本执行正式ReplaceWithExisting，逐表确认仅领域变动、canonical及非Agent数据不变；随后对真实库执行同一正式命令。原04的0准入与04b原typed成功正文均核实，Execution38→39→40并completed，Attempt仍12，8份真实成果未重跑。临时维护example已删除，driver仅留ignored证据。

原checkpoint168/seq2996/model99的build为bd51…，匹配旧exe SHA256 `26AECC3B4DDF31E799C286A40D4D6105E194119481E0347FE8D9AEB41FCE78D2`。原版本自动恢复被seq2997同request的纯AwaitingChildren尾部挡住，正规quarantine写2998/2999并暂停；不是未决业务效果。逐项验证14个process Effect的exited/reaped、33个returned Effect、原host正常退出及无exact Session残留进程，形成 `attempt-handoff-original-cleanup-proof.json`（SHA256 `54b46e6cc2955829ac70583770a7701409766d18ad6cbfce0fb9ec36c06e973c`）。通过原WebView既有认证fetch调用正式execution/resume，HTTP200 authorized、seq3004；未提取或修改认证秘密。保留原Turn/build/Snapshot/预算，父从model99继续到104，最终在seq3076/3080写入完整答复、seq3081写turn/completed并回到ready。真实恢复已完成：Attempt仍12、max_parallel仍4、9个子Session全部canonical事件与备份逐条一致，未重新执行孩子。恢复后仅发生状态核对、计划修正和完成报告；原回合host_cleanup_proven也已写入。证据 `attempt-handoff-real-final-delivery.json`。在独立文件副本重新运行原canonical测试脚本，10个JS语法和47项冒烟均通过，真实工作目录未改；证据 `attempt-handoff-restored-game-verification.json`。已通过原WebView正式process exit关闭本轮启动的兼容host，调试端口已退出，未杀共享键盘服务。另一旧paused Session保持seq1588，另一受未结算Effect保护的3条输入仍未领取且head137；不得借恢复顺带执行。

最后交叉审查补充：Stop与prelink准备竞争时，通用删除前仅事务外读link不足以证明空，且prepare失败漏清理；现正替换为原opening/零输入/零Turn/零Effect/零link同writer的空创建退休，合法新输入获胜则保留，先退休则拒迟到输入，使用原deleting恢复责任而不新建账本。candidate3全App回归已超过1000项通过，发现一个已有静态边界测试对CRLF不兼容，已仅规范测试读取的行尾，待fresh重验。

全App candidate3实测未整体通过：1301项中1103通过后，除CRLF静态测试外，后台问题回复用例稳定复现真实活性断层——无推进者分类把合法等待用户回答误判成技术暂停，第三条已accepted输入因原Turn paused不能claim，正在修复等待分类与原输入接回合同。随后test进程以Windows 0xC0000005退出，未运行尾部不能算通过，已单独分配按原产物和默认栈诊断；不提高栈或放宽deadline掩盖。新源码最终验收须覆盖这两处。

分层 fresh 进展：Runtime533通过/1原ignored，Execution185/185通过；`bun run check`再次全过。Session393项候选在374通过、1条旧错误文案断言失败（已修为native recovery proof）后发生同类0xC0000005，不能计整体通过。纯WAIT三个Store定向已过；空创建第一版fixture的FK破坏方式已修，另补queued successor不得删除，并将输入竞争加强为真实两连接WAL（仅测试后改，待重编）。原生崩溃已跨App/Session复现；只读检查未找到仓库直接SQLite句柄共享或hook，diagnostic guard未改变原事务连接流程。现有dump缺失，因此已用独立故障夹具验证私有DEBUG_ONLY_THIS_PROCESS取证runner，正运行Session完整测试取得现场；不更改栈、并发、超时或全局WER。

candidate4 App定向新证据：后台真实问题入口原稳定失败例已通过（32.1s），生产Journal纯WAIT恢复1/1及CRLF边界1/1通过；runtime_collaboration 18/19通过，单槽已受理回答误标attention暴露dispatcher观察遗漏。已按原typed DecisionInput证明修 snapshot/live WAIT 两处：只能据此查询真实dispatcher，getter为真才can_progress，outbox本身不授执行权；原单槽断言/并发/超时保留。为避免陈旧产物，精确中止尚未使用凭据的live compile-only App rustc并保留依赖，开始candidate5 App lib+live integration同图编译。

原生崩溃取证的边界：一条子Agent深入转储诊断被平台安全检查拦截，后续仅普通日志/源码/限定测试，不绕过。受控Session完整393项已正常跑完，392通过/1旧文案失败、无nativeAV；原plain两项最早未结束候选按默认栈/双线程重复10轮共20次全过。故仍未定位原AV，不能把受控重跑当作已修复。

2026-10-24 继续验收：Session最新plain完整393项中392通过，仅新WAL fixture使用OPEN_MEMORY源VACUUM造成目标不落盘；已改正式init_database创建，再双连接WAL重开，7项相关定向全过（其余392的生产代码未变）。App完整1305项：1300通过/1机器人管理fixture失败/4原ignored，无nativeAV；失败fixture刻意shutdown整个Execution，新的无owner出口正确暂停，另审计确认真实initial planner此前漏计owner。现已实现同步规划guard、局部typed数据库重试、原已产plan保留、确定失败结算责任保留、commit丢ACK后的实际scheduler交接及先交接再publish；fixture以真实规划barrier保活、Stop抢占，正在candidate7编译/测试，不假装status等于owner。

StepFun Plan step-5-preview真实2子完整通过：1次delegate/2个Attempt/1父Turn，Full孩子真实重叠23284ms，2份原结果消费、父文件整合及原生进程验证均完成；WAIT期间0模型调用。证据 `attempt-handoff-live-two.json`，build digest `6cd8a8f054dabbaa96ee0de1f243a7740c2c81f8d35a0aa2a52d6d0646b454ce`。真实8子首轮父state completed、0工具/参数/Kernel拒绝，但EIGHT_PARENT_FINAL_MISSING失败；该旧断言混淆正文缺失与marker缺失，不能推断没交付，也不能计通过。仅测试/runner已分开FINAL_EMPTY/FINAL_MARKER_MISSING，在保持原prompt/所有断言的前提下先核验全部工作事实再查marker；加入原flag的受控保留与明文凭据审计，待同任务取证重跑。首次retained启动的白名单环境缺Windows SDK定位变量，报WINDOWS_TOOLCHAIN_UNAVAILABLE且未执行；已从宿主仅收集非敏感工具链/系统路径并验证TOOLCHAIN_OK，不改变用户环境。

桌面dev候选已在独立输出目录编译成功（不覆盖仍被共享键盘服务持有的旧exe），路径见 `attempt-handoff-desktop-executable.json`；规划owner新修复后需fresh构建，不计最终候选。真实指定会话已交付且本轮未再重放，其他真实任务不用于故障测试。

当时待验的纯WAIT、空创建退休与隔离2/8子实模已由本页最新Windows结果更新；该记录中的中间构建不升级为最终证据。macOS/Linux新整图原生验证仍单列。

### 2026-10-23 父 Turn 收尾责任与结果恢复（已实施并完成 Windows 验收）

用户授权完整实施。起点为 `c3eb1a85b0970e868f47d84d15007e08c3d5c87e`，保留原工作区 55 项已修改／未跟踪内容；未改写真实故障会话、用户数据库或重启现役桌面进程，未 commit／push／发布。隔离实模使用用户指定 StepFun Plan `step-5-preview`，凭据仅经无回显 stdin 进入测试内存，未进入源码、日志、环境或参数。

原 SDK 现在保留真实输入、原出口、首个有界脱敏错误及未完成资源现场，原 Session owner 接管确切 Runtime 的收尾。`runtime/native-settlement-requested`／`settled` 复用 canonical Event／Payload，不新增表、列、第二账本或启动权限。暂停后只准补交已准入观察、清理及原出口；Stop 和后继围栏优先，真人 Pause 保持暂停。初始化前缀先确认再改变围栏，claim 后先保留原 Journal；SDK 受理后持续负责，外层不再以固定超时制造第二次暂停。ACK 丢失只确认原事实，不重跑模型、工具、输入或委派。

同 root／同 SDK 的恢复使用既有 opaque SDK task identity 识别原任务，原发送才更换身份；registry release 与收尾重试不改变它。出口缓存和 preclaim 单飞绑定确切 SDK／task，身份缺失不放行。worker 等待后重新核验身份，canonical native generation 继续独立约束写入／通知。冷结算校验完整 journal 序列，仅比较原 receipt 代内的出口，保留旧代真实 Pause。原工具及 MCP owner 保留真实返回及原待补交记录，业务失败、返回合同错误和物理释放分别结算；typed Busy／Locked／PoolTimedOut 只重交原不可变写入，不改变池容量或期限。

调度故障恢复局限于原 Step／Attempt，健康兄弟、原控制及 Stop 继续推进；正常与恢复共用 typed canonical 结果消费，预算／进度不重置。父指导与精确 QuestionReply 区分：明确答案引用原子 Session／Turn／Question Event／request，复用原 Attempt 的 DecisionInput outbox，保留 Agent 来源，不伪造真人同意。已答待容量不再误报未答，原父等待让出既有容量；native 取消及 cleanup ACK 在领域终态之后仍唤醒原观察者，不追加镜像终态或模型轮询。

八子实模另触发 HEAD 已存在的并行 caller 自锁：消费首结果时停 poll 兄弟，兄弟的准入 future 却可能持有首结果写入所需的 Host 锁。现于原循环同时推进剩余 caller 和单一串行结果处理；不新增 spawn／资源 owner，临时返回队列限于原批次，快结果不等无关慢调用，下一模型及重放保持原提案顺序。旧 cutpoint 实证 21.51s 失败，同 cutpoint 新图 1.76s 通过；暖恢复旧缓存 observed=1 及正式 release 后 21.12s 永久重试均先反证，新图三个真实冷／暖／release 回归各通过。原语音取消超时亦已转绿。

退役了 bool cleanup-pause 交接、永久 settlement failure 标记、按 Session 再挑 Runtime 的交接分叉、旧 admission=None 缓存放行、source 空 lead-report／suppressed ACK、编号任务的强制预承诺，以及没有生产调用者的 `open_journal` 包装入口。显式编号交付校验、非 source 真实报告、知识回写、资源及 native checkpoint 安全仍保留；Windows 路径使用统一正式 codec。现役中英文 [Agent Session](../architecture/agent-session.zh.md)／[Execution](../architecture/agent-execution.zh.md) 文档已同步。

| 最终受影响验证 | 结果 | 说明 |
| --- | --- | --- |
| App 完整 lib | **1224 通过／0 失败／4 原 ignored／0 filtered** | 默认栈、2 线程、519.33s；SHA `32423E73F29E2E640198DF260D93B3ACE9F9D493CFFA806A100A31225F1D8B08`。 |
| SDK／Session | **357／383 全部通过** | 最终 token 图、默认栈、2 线程、各 exit 0。 |
| Runtime／Execution | **525／173 通过** | Runtime 保留 1 项原 ignored；含并行 caller、快结果、局部恢复及精确取消通知。 |
| Contracts／DB repository | **131／78 全部通过** | DB 最后 writer／reader 变更以 fresh 78 为准，包含精确问题、binding／clear floor／Stop 拒绝矩阵。 |
| File／EngineCore | **433／67 通过** | 各保留 3／1 项原 ignored；Windows canonical path 夹具修正后通过。 |
| Native recovery／desktop shutdown／MCP | **8／2／17 全部通过** | 最终 clean 图、默认栈、2 线程、各 exit 0。 |
| Journal 冷恢复 | **14/14 通过** | 包含 Pause→resume→新 claim→handoff→cold 正反例；属于 App 全包，不重复累计。 |
| 八子验收 SQL 合同 | **1/1 通过** | 正式全迁移内存库逐条 prepare 同一组 SQL，并拒绝空时间、错绑定及重复身份。 |

先前 DB lib 314/314 是较早图的阶段证据，最后 narrow proof／export 变更由上述 fresh DB repository 78 覆盖。不同构建图不合成虚假的全量总数，定向回归不重复计数，原 ignored 不算通过。完整 `bun run check`、正式 contract generator、桌面 Browser／Computer feature 编译及 live runner 自检通过；最后文档修改另跑文档校验。

最终生产 build digest 为 `bd51c316fc7a7d6bdea845c7eb95bf121ce417e77f1ab83c757b015e6aa91e36`。双子实模 **1/1、87.74s**：两 Full、重叠 **19017ms**、原父 **1 Turn**、原生写入 Session **3**、最终投影补写 **0**。八子实模 **1/1、133.46s**：**8 Full／8 唯一 Attempt0／1 次委派**，既有 max_parallel **4**、domain／native 实际 peak **4**、四路重叠 **32886ms**；父同一 Turn 消费八份确切结果、读八个原文件并整合第九个文件，真实验证进程 exit=0 且 reaped=true；原生写入 Session **9**、最终投影补写 **0**、父未代写子文件。两实模均正常关闭、通过文件凭据审计和外层日志审计。双子 exe SHA `94A6BBA06DD8C8462AB02144E19590F9B93D65708B2EB3759D7087D8401AADBF`；八子 exe SHA `B861E3BF3D879C0F305D1574792FBEF38570F222317163378731142AD371445D`，仅八子测试读者修正使测试 exe 不同，生产 digest 相同。

中间失败证据保留：首次八子 903.76s 暴露旧 caller 锁环；修后首轮八子九个 Turn 正常完成，却因测试误读不存在的 attempt.conversation_id 列而失败，未计成功。验收读者改走现役 exact link／nullable 时间后先通过上述无凭据合同测试，再取得最终八子成功，没有添加冗余列或放宽断言。Journal 同副本曾一次 Windows 原生 **0x80000003**，原样重跑 14/14、最终 App 全包及 native 27 均通过。匹配 exe／PDB 和 COFF 离线定位不能解释触发原因，WER 无 dump；该一次异常仍未归因，不能宣称已解决或修改 SQLite／线程栈来掩盖。见 `build.noindex/turn-settlement-journal-native-trap-evidence.json`。

统一验收证据为 `build.noindex/turn-settlement-acceptance.json`，具体 SHA／参数／退出码／日志分项保存于 `build.noindex/turn-settlement-*`；实模 proof 分别为 `turn-settlement-live-two-clean-final-evidence.json` 和 `turn-settlement-live-eight-contract-final-evidence.json`。源码／模拟故障／隔离真实模型／未验证平台保持区分。没有重放原故障会话，不宣称新验证了 macOS／Linux；整个 lifecycle 专项既有平台待验沿本节其余记录，不以本次父任务切片验收代替。

### 2026-10-22／23 Codex 协作架构复核与可靠性修复（已实施并验证）

本轮从已本地提交、未 push 的 `c3eb1a85b` 接续。用户授权保留现有能力、并发设置和 Canonical Session，参考本地 Codex 的独立运行及通知协调，完整修复来源误判、等待查询放大、暂态数据库读取失败和交接问题。没有改连接池容量、借连接期限、SQLite 锁期限或数据库代际，没有触碰真实用户库、重启现役桌面进程或重放旧任务；实模凭据仅允许 stdin。

当前实现：真人一次委派要求依据原 canonical intake、owner、producer 与命名空间，内部任务信封还须既有确切 Execution 来源 scope，角色文字和 Host 续发不会制造强制递归。按 owner／Execution 和原 ActiveTurn 通知等待，先订阅后读；稳定无变化的 30 秒兜底仅两条批量 domain／native cursor SELECT，变化后读取四条同事务窄状态及增量最小证明，不再逐秒读取配置／参与者／全事件日志。WAIT 准入、等待与 STATUS 消费共用可取消的 typed 只读退避；写命令、损坏、I/O 和撤权不被伪装成读取重试。原来源、分页完整覆盖、回执、模型预算和恢复围栏保持。DB 诊断在已接入的读取事务及写锁准入边界记录等待／连接范围、pool 状态和来源，不插入 SQLx driver hooks、额外 SQL 或控制权；返回写事务之后的全部 writer 持有期尚非该准入 guard 的覆盖范围，不能据此声称已得到完整连接持有链。

只读排查又确证 12 个旧进程均为原 first-party failed exit（exit=1、is_error=true、已 reaped、无 cleanup errors），其共享正文逐叶摘要正确；旧读者否认启动而反复写回，构成已证实的额外写事务压力。现将业务失败与物理生命周期分离，保留原失败和原归属，不导入或改写旧记录。旧日志仍缺当时各连接的持有跨度，不能宣称该重试源是全部阻塞的唯一原因。端到端还发现合法约 81 KiB 单段助手内容误走 64 KiB 内联分支，已统一既有 canonical Payload 写入；整包发现 clean-cut 先于 Cron 012 迁移却使用新列，已通过正式阶段类型修复清除顺序，保留作者配置，不转换旧 Agent 值。

最终整包曾在 1007 项通过后因 Question 用例栈溢出中止，未算通过。固定测试 exe SHA `0bef193b…f7be3f`、不设置 `RUST_MIN_STACK` 的 Windows 调试三次复现 `0xC00000FD`：216 层有限调用链仍在 Session 初始化，未进入模型或 WAIT；默认约 2 MiB 栈已用 2,033,416 B，Knowledge validator 再申请 37,072 B 时触顶。不是递归委派。已在唯一 `append_event_tx_with_policy` 建立 boxed Future 边界；原 body 规范 LF 后逐字节相同，原 borrowed transaction、校验顺序、Send 与取消语义保持，不 spawn、不提高线程栈、不跳过用例。修后 fresh Session、App 与实模均已完成，证据如下。

验证：Runtime 完整 **517 通过／0 失败／1 原 ignored**，DB **314/314**、Common **260/260**、Gateway **110/110**；这些包完成验证后未再修改其源码。共享 append 堆边界之后，fresh Session **374/374**、Execution **168/168**，均为默认栈／2 线程／无过滤。新 App 副本 SHA `b3ff74bc28d5f12a9cf0c6a8f579fea203aee99834f00d08d3b7a805e22c6dea`：完整 **1181 通过／0 失败／4 原 ignored／0 filtered**，436.36 秒；另先行确认原 Question 默认栈 **1/1**、HTTP 新回归 **3/3**（四 Full 实际 peak=4、冻结 Skill／附件／工具发现、重复委派、81,042 B 单段／分段结果、三页完整覆盖与父原 Turn 集成进程）。以上主包合计 **2924 通过、5 原 ignored**，定向复跑不重复计数。App 四项为原 restart child helper、ACP936夹具、标准 token／LanmanServer夹具和手动真实浏览器夹具，本轮未新增或修改 ignore。前轮夹具发现调用未呈现 plan 控制及误计 Kernel effect 已修夹具，合法长段、clean-cut 顺序和栈溢出的真实失败均保留记录，没有调高存储／栈上限或跳过失败。

最终 `bun run check` 全部通过；`cargo check --offline -p nomifun-app --lib --tests --no-default-features --features browser-use,computer-use` 通过，2 分 18 秒，只有原 browser_workspace 测试支持的三条 dead-code warning。此项是桌面 feature 编译证明，不冒充 Browser／Computer 或 macOS／Linux 原生验收。最终日志：`build.noindex/subagent-{runtime-final,db-verified,common-verified,gateway-verified,session-heap-full,execution-heap-full,heap-app-full,desktop-features-heap-final,static-heap-final}.log`。本轮未 commit／push，未重启现役桌面程序或修改真实会话数据；整个 lifecycle 专项的其他原生平台待验仍沿本节既有记录。

最终堆边界图实模 **1/1**，77.26 秒：Step Plan `step-5-preview`，两个 Full 子任务保留带动作词的 SubAgent 角色前缀，实际重叠 **18,836 ms**，强制委派拒绝 0、一个 Execution／两个 Attempt，父 **1 Turn**、三个原生写入 Session、最终投影补写 0，真实文件整合和进程验收成功；正常关闭与明文凭据审计通过。native build digest `d756308aa50a3dcc912131074b1b4e46b5e54d8205fa72dd6466c393aea85282`，实模 exe SHA `8346b9ad0c2414775e4b0fae91725825708d36b72a8181be08f4d653b7b48f51`。凭据只走无回显 stdin，不进环境、参数、源码或证据。完整 JSON 证据位于本机任务证据目录的 `subagent-codex-reliability/subagent-live-result.json`；旧图的 21,719 ms 实模与 1007 项后中止整包单独保留，不能混用为最终图证明。

### 2026-10-22 子 Agent 并发与继续执行（已实施并验证）

用户已授权完整替换：保留既有能力、用户并发设置和 canonical 事实链；参考本地 DeepSeek Harness 的独立子会话与消息协作，复用现有 Session／Execution／资源 owner，不引入 Team 账本、固定驻留配额或 best-effort 交付。起点为 `f5a57a602ffbe0a67ed271d2b81435194480a0da`，工作区干净。使用隔离数据库、目录、受控 HTTP 模型及正式 Step Plan `step-5-preview`；实模凭据只经标准输入进入测试内存，未写入源码、命令参数或证据。用户随后授权仅本地 commit；提交身份以 Git 记录为准，不 push、打包安装或发布。

现役合同：Full/ReadShell 只限制权限，不独占完整 Step；真实 I/O、原生目标、Unknown、取消与清理围栏保持。父 Agent 保留完整工具；首次明确委派只约束一次，受理后继续原 Turn，以 typed AwaitingChildren 让出模型容量并等待，恢复前在原 writer 取得既有 Execution/Participant 容量，预算不重置。`agent_status` 消费确切 Session／Turn／terminal 的完整分页结果，`agent_message` 通过原命令事实幂等指导运行中孩子，或以新 Step／Attempt 继续原子 Session；纯 FIFO 不伪装 DAG 依赖。父 Agent 自己整合、检查并正常交付。

完整继承与交接：首次派发、运行中消息和完成后续发共用原 accepted 输入及命令前实际应用的 SteeringInputs；附件和显式 Skill 进入普通输入准备，工具来自冻结 Snapshot，队列与未来输入不越过因果边界。自然答案／录音交接读取本次工具之前最后一个已应用输入，保留原 presentation／socket／binding 校验；同一引导 key 在 Running／Completed 重投仍返回自己的原输入 ID，操作与结果继续引用原 claim，读回执不新增事件或模型调用。等待前的正文通过原 PublicOutputCommitted 取得公开交付证明。Robot 问题通知／播放以唯一事务校验器读取原正式 Store 输入与 Effect 证明，以 Returned Effect 真实事件授权，不再等待父回合结束；Pending／Unknown 保留原投递责任，明确撤权则拒绝交付。MESSAGE 共享长参数、目标、内容摘要与稳定身份校验；丢回复只观察原 owner 回执，不重发。取消读取所有原 Attempt／问题／Decision Turns 的确切 native 终态；业务取消不冒充结果交付，早期真实写入不会被新 queued Attempt 遮掉。workspace_change_sources 是唯一来源数组；删除读取原撤销事实，不恢复控制权限。

退役：整 Step 工作区互斥、未调用 embedded invocation 类型、synthesize 自动开关、source 自动最终报告 writer、Robot 子结果回播、字符串化 tasks 修补分支，以及 plan_gate DTO／配置／物理列均已移除；015 前向迁移保留既有 baseline／迁移字节及其他配置。明确的 synthesis DAG、问题与控制、历史只读、非 source 现役报告保留。现役中英文 Agent Session／Execution 文档已同步。

验证：最终 App 完整 lib **1171 通过／0 失败／4 原 ignored／0 filtered**，**474.64s**，无跳过，含全部 Companion／Question／Robot、HTTP 并行 barrier、同子 Session 续发、fork-only、精确 Stop、丢回复恢复、选择继承与全部原消费者。固定独立测试副本 SHA-256 `837f4cc25523626af0acb17841364331ae854ed351bb323682f76a82a492580c`，日志 `build.noindex/session-evidence-app-final-full.log`。Runtime **513 通过／1 原 ignored**、Execution **166/166**、Session **372/372**；最后事务读取器接线后，原受理输入 **4/4** 与 Effect **31/31** 定向复核通过（属于前述 Session 用例，不重复累加）。DB Execution **76/76**、迁移／ID／reset **13/13**、Contracts **126/126**、Wave5 **8/8**、EngineCore **67 通过／1 原 ignored**、Gateway **110/110**、nomi-types **6/6**。Windows cmd 混合分隔符原生回归 **2/2**，live 证据辅助单测 **24 通过／17 显式实模用例默认 ignored**。保留失败记录；首轮 App 15 失败、后续仅剩 Question 的 1170／1／4 均不是最终通过证据。

验证边界：后端 libtest 使用既有两线程参数，真实并发用例内部的并发不变；一次默认 32 核并发批次变慢后主动停止，不计通过或具体用例失败。Windows debug 栈以原生展开证实为测试 poll 临时帧叠加，只在共享 Fixture 构造边界盒化；默认栈、场景与断言保留，两个 fatal 用例在最终整包中通过。人工 crash 撤权测试明确保留旧 writer 的正常关闭拒绝与可发现身份，不把已恢复业务回执当作完整 cleanup 证明。App `--lib --tests --no-default-features --features browser-use,computer-use` 检查通过，**4m46s**；未以此声明真实 Browser／Computer 操作或其他平台原生验收。正式生成器、Session／Desktop／process 边界、文档、raw schema probe 与 live runner 自检均通过。

正式最终图实模 **1/1、116.37s**：`nomi_core_subagent_integration_reaches_live_stepfun` 经生产 Session／Runtime／Execution 路径，两个 Full 子 Agent 实际重叠 **22127ms**，各自写入并读回独立文件；原父 Turn 收齐结果、集成、运行严格完整内容验证，确切进程 exit=0／reaped=true。证据为 `full_children=2 parent_turns=1 native_write_sessions=3 final_projection=0 integrated=true verification_process=true`，应用正常关闭及凭据明文审计通过。最终测试程序 SHA-256 `76c7730fc87386583485f91ec7014fc5f850032d593d048b4a966db9400b4265`，运行 build digest `eebf820e2a782e238484f345a05c4bc987d91e08f268c0e40e7091343789f973`，证据在本任务 `subagent-live-result.json`。首轮 findstr /x 误判无末尾换行文件，未完成父交付且关闭报 background；失败记录保留。修正测试为完整内容比较并以本机正确／额外换行／错误内容三例验证后，中间图曾取得 82.06s／23279ms 通过；最后 Robot／回执修正重新构建后才取得上述最终证据。正式 runner 为 `--subagent-smoke`，未操作真实用户库或重启现役桌面进程。

首次失败实模的隔离 Profile `C:/Users/rika0/AppData/Local/Temp/.tmptnVsmH` 清理被执行工具自动安全审核以 `blocked by policy` 拒绝，未提供更具体原因，目录保留；没有绕过审核。其余验证产物保留于本任务证据目录与忽略的 build.noindex，不冒称测试产物已全部删除。

宿主整包复现命令：`cargo test --offline -p nomifun-app --lib --no-default-features -- --test-threads=2`。本次先生成程序，再对校验 SHA 的独立副本执行相同完整集合，避免 Windows 运行中程序锁阻碍另一构建；未过滤用例。默认 Channel transport features 和桌面 Browser／Computer 条件编译与这组运行证据分别记录。

以下为原平台交付时的证据快照；历史编号保留用于识别原证据。当前结果与剩余边界以本页前部为准，不把旧构建或一次 Windows 通过扩大为新构建或三平台通过。

| 项目 | 该次记录值 |
| --- | --- |
| HEAD／分支 | 原实施由8c3a5cb93快进至cfee705634ccd59ae1d621b13d9ed1f95709478e，实施／平台prompt保存为a9c41c38fa276654d34ce020cd22a4a3c3b730f9。交付前整合origin/main084c7dc8d0962567075be6bcc089b39bd032fba8的8个提交；实际交付HEAD及推送状态以Git记录为准。用户仅明确授权本次commit＋push，后续平台任务不继承此授权；未安装或发布 |
| 工作区保护 | 初始AGENTS.md及两份专项文档已保留并纳入用户授权的本次提交。故障注入只用临时DB、目录、专用新Profile及受控进程；未操作日常用户数据。整合保留上游OAuth010，尚未发布的Cron迁移改011；新native一律新dataset，不改写旧临时库或真实数据的迁移回执 |
| 当前阶段 | Windows交付快照见10.1–10.4；macOS本机实施与原生接续见10.5；Linux编译、适用回归、隔离X11与真实Desktop原生验收及必要修复见10.6。真实Wayland portal、管理员设施、托盘／GUI Quit／重启／系统退出入口、睡眠/锁屏与桌面级WebUI浏览器控制仍按各节待验，不声明全部平台验收完成 |
| 本次上游整合 | 实施a9c41c38f＋origin/main084c7dc8d整合源已冻结；OAuth010原字节保留、Cron011及所有静态include／schema／boundary同步。MCP auth沿原HostShutdownStage独立retained worker关闭，正式typed诊断可证明资源收束而业务Unknown保留；新增真实阻塞owner竞争1项及合法／5损坏诊断回归1项通过。官方write/check、完整bun check、UI56、DB73、Contracts118、Session331、Cron169、MCP248+19+11、App1218+5ignored、四integration48、稳态3及新Desktop三native全部通过。以下构建与退出证据已更新为本合并图 |
| 已验证构建快照 | 最终合并图App libtest SHA256 `74f6c91bd4b5dda83f64239844608b9616f6bc37c94ebc62dfba0487661b9537`（UTC2026-10-19T06:04:05.4224637Z，346943488B）；production Desktop `661681a63e3b15fa4190ebefcf78268940b533c4b4f69cd36809426193bdf84a`（UTC2026-10-19T06:14:41.4297828Z，341702144B）。构建时为a9c41c38f＋084c7dc8d的冻结合并工作树，后续仅更新文档。debug夹具原setup/login发普通JWT，不导出trust |
| 已收束验证 | 当前App **1218通过／5原ignored／0失败，363.18s**；Contracts **118/118、0.22s**；Session **331/331、107.84s**；Cron **169/169、13.49s**；DB **73/73**（执行53、reset2、ID6、OAuth迁移3／仓储9）。MCP单元 **248/248、0.82s**，auth integration **19/19、13.71s**，connection integration **11/11、3.45s**。原Execution155/155库源未变；合并图另以当前App／DB执行53／恢复8覆盖整合链路 |
| 环境限制 | 原Windows交付时暂无macOS/Linux主机；该历史限制不覆盖10.5／10.6实跑。Linux为WSL2/WSLg，无真实Wayland portal后端、pkexec或桌面浏览器，具体阻塞见10.6。各平台不擅自提权、锁屏、睡眠、安装或启停服务 |
| 该次遗留边界 | Linux修复当时为未提交差异；后续提交与推送不继承历史授权。真实GNOME/KDE Wayland portal、pkexec管理员、托盘／GUI Quit／重启／系统退出入口、睡眠/锁屏、桌面浏览器WebUI未验。验证需核对源码／构建身份；缺证据不成为Agent运行门槛。当前状态见本页平台边界。 |

### 10.1 实现与验证

| 工作包 | 当前行为与实际证据 |
| --- | --- |
| P0 | 完整读取施工/启动/现役架构，按当前HEAD重核R01–R14和原资源owner；无新broker、第二执行账本、机器配额、任务TTL、worktree或声明门槛 |
| P1 | task / delegated_work / task_and_delegated使用原Step/委派事实及同SQLite writer围栏，DAG不作所有权。局部Stop不聚合取消；派生/父retry/adopt/replan受停止约束。原Attempt精确cleanup覆盖automation和软删除，原Lifecycle保留提交后信号，即使HTTP waiter消失 |
| P2 | 调度pause只停新派发，原运行工作保留。native pause/resume走同Turn/checkpoint/Event意图及原Execution/Participant预约；四租约组合读原observer，不cancel+retry。修复Paused时误关整个Session ProcessScope：native recovery8/8现保同PID/owner/starts=1，继续后可poll/精确Stop |
| P3 | 每Attempt常规控制FIFO单在途、跨Attempt独立，User Stop持久围栏/信号抢占。原Steer future、receipt/cursor补读与恢复交接保留；查询故障不授权重跑 |
| P4 | 原nonce/generation/Arc Runtime自动退休，知识回写自然join；Process/SSH/Browser独立资源owner保留。暂停relay结束后首次正式取消也调同精确退休；duplicate旧Stop不得取当前warmup载体，join前排除旧operation，锁内仍完整复核。已完成资源不靠关闭Session回收 |
| P4 Browser结算补齐 | 原repository单读快照及typed结束证明覆盖Completed automation根、Failed、canonical/scoped/aggregate取消与softdelete/fatal/source取消；无新ledger或普通API可见性变化。同Session活动任务、合法重试、用户页面与下载仍保护，跨Session委派不误当根Browser用途。当前dispatcher／BrowserService **28/28、27.57s**（14项新使命回归）；早期DB并发过载按测试并发2修正，未改产品并发／期限／断言 |
| P4 使命闭合交接 | 旧Attempt／真实current分离、Step终结重唤醒原outbox、正式replan/supersede/skip/adopt证明及active后继deferred已落。当前DB执行 **53/53、48.18s**；原Execution **155/155**库源未变。target读取原正式System DecisionRequested／settlement的canonical引用，清queue仍同continuation，不回initial；App使命14例包含于当前Browser28和完整1223，保留Question Completed及原Attempt事实 |
| P4 来源结束补齐 | Cron NewConversation创建路径捕获原tuple，正式结算封存live回执，跨合法job/run删除保原清理权限；当前Cron169/169含10项来源／删除竞争。私有互斥Cron／Remote typed来源写原accepted input，普通请求不能声明，旧重放不retrofit，不扩AgentInputSource。Remote取消沿原coordinator singleflight／shutdown，不占business64 semaphore，NeverInstalled不warmup或永久重试。Existing/InputRequired、Remote Ready保持续用途。当前source **3/3、3.26s**及完整App通过；无新ledger／TTL／重跑／声明门槛 |
| P4 Computer | 本专项旧 Computer 路径与其原生/桥接测试已经退役。由新的私有 Cua worker、宿主 InvocationControl、公平队列和 canonical Effect 承接；当前源码与分层证据只见[Computer 专项进展](../handoffs/computer-use-rebuild.zh.md#12-单份执行进展)，旧计数不作为新实现证明。 |
| P4 Quit | Terminal、MCP auth原worker与Runtime／producer／Browser独立清理，挂起auth不挡Terminal实际delete hook或Browser close；仅观察超时不abort／重发原worker，drop外waiter后仍join。聚合前收原worker真实settled结果，native/storage仅在所有原owner已证明后关闭，域错误不跳过可独立释放资源。新增实际auth guard竞争与当前全量／三native通过 |
| P5 | 只读Activity汇总原domain/runtime/resource owners，逐实例Stop及cleanup/unknown可见；AllStop是有限原源快照。Cron停原reservation，AutoWork停原claim、保配置。AutoWork仅原root用lead Session，child独立ChildAttempt，root/child/整组控制不混淆，Remote走原binding fence |
| P6 | 现役架构及正式schema/event同步，替代路径/fixtures同批退役；强Workspace缓存按原Weak实例票据退休，Session/Companion/delete纯gate缓存不启动任务。资源稳态、原生退出、真实Edge控制、生成/边界检查见下 |

| 验证组 | 最新实际结果与范围 |
| --- | --- |
| App完整与定向 | 最终74f6c91b完整1223：**1218通过／5原条件ignored／0失败，363.18s**，日志 lifecycle-merge-app-full-tests.log。同产物Browser **28/28、27.57s**、source **3/3、3.26s**，新增auth独立关停和strict诊断proof各1通过；新进程稳态 **3/3、11.98s**。5个ignored为原需父驱动／浏览器或Windows opt-in条件项，未冒称全部默认运行。原snapshot5b为1213/5ignored，仅保留历史，不混作本轮 |
| App 集成 | 合并图扩为四组，**48/48**：browser_workspace21/0.82s（b06e1fe3）、desktop_agent_shutdown2/33.40s（38a29bbd）、native_execution_recovery8/29.21s（b6aa6880）、mcp_e2e17/22.62s（21c6f199）。构建身份 lifecycle-merge-app-build.jsonl，独立 lifecycle-merge-{name}-tests.log。原writer锁unknown→显式retry及background Process同PID／owner／starts=1暂停继续通过，观察期未改 |
| Canonical／DB／Execution | 当前Session331、DB执行53/reset2/ID6/OAuth迁移3/仓储9均通过，完整73；原remote6与Execution155对应库源未变。涵盖持久精确停止、原继续key、预约／四租约、公平清理、FIFO抢占、丢awaiter提交、专用取消证明、真实continuation及旧Attempt使命闭合。合并图以当前App完整和恢复8补整合链路；不把原库测试冒称新图重跑 |
| 自动化／AI | 当前Cron **169/169、13.49s**；Requirement58/58、AI349/349为原实现且库源未改。真实App root/child原Session／claim／grant／physicalRoot隔离及source取消CAS包含于当前完整回归 |
| Computer／System | 该行旧 Computer/a11y/桥接证据已退役；新实现见 Computer 专项。原 System 的 23/23 为当时来源的历史证据，不改写为本次源码重跑。 |
| Process／SSH／Browser | process-runtime **156/156**；Windows parent-death/pipe/PTY及ConPTY/ACP opt-in native共 **27通过**。SSH后端 **47/47**、nomi-ssh **61/61**；Core **66通过/1条件ignored**，另真实Bun1/1。Browser Platform **116通过/2条件ignored**；Attached协议50、真实Chrome3及新增pending Promise跨target/disconnect1通过 |
| 归档／原资源 | App Process资源 **5/5**、Journal **15/15**；Frozen V1黄金数据/raw cursor/decoder carry/EOF/ACP936通过。16MiB原Frozen字节用同agent_payloads分块，重复ACK重验损坏不伪修复，普通/伪装/孤立payload仍计预算；128 roots/1024 chunks查询计划无相关逐行子查询 |
| MCP补验 | 合并图MCP **248单元＋19 auth integration＋11 connection integration全部通过**；App MCP e2e17、原outcome及strict诊断新回归包含于当前全量。仅原资源物理证明可用，业务Unknown／noReplay仍保留；foreign／损坏／active owner不借此放行。新661681a6三native各13原句柄通过。原request_lifecycle14与Windows literal argv1为之前证据，不冒称Unixstdio通过 |
| 普通清理成本 | 保留has_unsettled_effects快速检查及owner/auth前提，仅确有pending/unknown才读完整清理证明；Runtime／Browser复用同canonical proof，不读历史启动任务。当前稳态baseline query3687，三轮3887/4008/4043，无模型／工具重跑。新661681a6三native通过；SQLx计数口径／trace开销见10.2 |
| 真实失败驱动修复 | 已修活动Turn进程归档循环、MCP物理／业务Unknown混淆、reconciliation原producer取消误走新work准入、138-byte合法Operation误用128-byte key上限、无workspace阻退休、Robot完成双快照缺源、AutoWork非终态CAS竞争及正式使命结束未交接旧Browser。均有原DB／失败日志和回归，当前5b全量／f3d三native通过。原失败目录01a151d2-c027…、01a151d5-d0dd…、01a151e7-3e9d…保留；不将失败计为通过，不缩目标、观察期、产品期限或清除Unknown |
| 剩余业务围栏 | Runtime/Browser复用原canonical MCP资源关闭证明；无Browser workspace不承担Browser退休。其他domain/Browser未知及native/user-page/download保护不变。其余has_unsettled调用已核对为新工作/恢复/权限切换/停泊交接/破坏性操作边界，保持严格，不把可退休资源解释为业务已成功或可重跑 |
| UI | 合并图专项＋MCP管理 **56/56、10文件360断言**，typecheck与desktop边界通过。原实施曾全跑5543项：5540通过/3失败；ImageLightbox隔离复跑过，另外两项原cfee基线GuidAgentPresetLaunch.structure／ConversationQuestionLocator.structure断言未改；本轮未重跑全UI，不宣称全产品全绿 |
| 正式检查 | 合并图通过官方agent-v2-contract write→check，未手改generated。完整bun run check exit0：2205 renderer、8016 keys/33模块、3089生产文件UARC零异常及所有现役边界；日志 lifecycle-merge-check.log。现役中英文Session／Execution／Browser合同及平台prompt已同步；最终docs138文件/1421links与diff复核见 lifecycle-merge-docs-final.log。仅880×600及以上 |
| 构建故障已收束 | 旧测试exe运行时重链触发Windows文件锁、一次rustc STATUS_ILLEGAL_INSTRUCTION均已在不删缓存／不回退源码的同flag重编中收束。当前source新fixture UserId错误及两配置／同步失败均已修，保留合法snapshot、精确终态与无重放断言；当前5b完整1218和f3d生产图通过。未把失败运行算通过 |

### 10.2 资源稳态与Windows原生证据

| 项目 | 证据与限制 |
| --- | --- |
| 最终T26/OS09 | 合并图74f6c91b新进程／128MiB stack／SQL trace，**3/3、11.98s**。9短Session三轮+1合法长任务：baseline handles322/RSS171360256/query3687；三轮handles324固定，RSS176246784/176398336/176762880、CPU4968/6515/8265ms、tasks(4,7)、disposed3/6/9、model4/7/10，query7574/11582/15625（3887/4008/4043）。长任务自然结束handles313、registered0/disposed10/query16061。6真文件Session：baseline handles305/RSS169033728/query18880；三轮cache0、handles315固定，disposed2/4/6、model4/8/12、fileReads2/4/6，RSS178946048/179847168/180195328、CPU12296/14328/16171ms，query22216/25557/28907（3336/3341/3350）。SQLx完成查询含夹具观察、不含内部trigger、不存SQL文本；trace有成本，不代表生产绝对CPU/RSS。日志 lifecycle-merge-stability-tests.log |
| 最终真Desktop Quit | 最终661681a6：host41888 exit0/coordinator=true，pipe27480/ConPTY45836/MCP40880及10 WebView2共 **13个原句柄全退出**；树外sentinel41292观察时存活。新root C:/Users/rika0/AppData/Local/Temp/nomifun-lifecycle-01a152cc-6c76-7435-8361-bacad5b56870 |
| 最终真parent-death | 同661681a6，只kill原host10864（exit-1/coordinator=false，不冒正常退出证明）；pipe47560/PTY20940/MCP28920及10 WebView2共 **13个原句柄全退出**；sentinel36192保留。新root C:/Users/rika0/AppData/Local/Temp/nomifun-lifecycle-01a152cc-cf06-7f4e-a97b-e5657662f37e |
| 最终直接Stop／独立资源 | 同661681a6，原138-byte Stop→Runtime released；pipe4540/process01a152cd-369e-7720-9ec5-7df394593c40、PTY22236/process01a152cd-3b42-75c0-8e63-e11f3fbfa817和owner_generation01a152cd-3492-7f41-8e2f-275f11c1a255六身份不变，原Bun句柄仍alive。MCP20760已exit而Unknown/noReplay保留，model5/start2/MCP1不增加；host7392 Quit exit0/coordinator=true后全13退出。root C:/Users/rika0/AppData/Local/Temp/nomifun-lifecycle-01a152cd-2021-76e2-a06a-d2d591afa954 |
| 真实资料／凭据审计 | 三个新root各原13+host+Node+sentinel共16确切PID均无残留；JWT三份删除，DB3805184/3604480/3805184B及Profile／fixture sentinel保留。观察时树外sentinel存活，后仅夹具owner关闭自身sentinel／Node。原PS a738477db0427c40f725389a559cf38aaf37f4d42cb4bc62c9705df569995a70、Node f4ce30a7fd57a2e8cad601289dd09280d8cfbf44cba7a08759f87cd6fd538c56及138-byte目标／45s观察期不变。各post-run-audit.json留证 |
| 现有OS服务保护边界 | 最终三例各自before/after只读LanmanServer CIM均Running、PID5436、创建UTC2026-10-17T02:50:20.4820180Z一致，证据为各root os-service-observation.json；SYNCHRONIZE-only访问错误5，未提权或启停。仅证明现有服务保留，不替代新服务交接、原服务句柄连续性或管理员场景 |
| 最终OS10 | 实际Edge+原cookie认证/WS，d717 backend与driver记录hash一致；原ignored fixture **1/1通过、27.74s**。单Stop仅Alpha，Beta Preparing保留；有限AllStop接受2、owner未join前execution_stopped=0，分别release后消行，turns/models=0。1280×800/880×600无溢出，renderer.errors/messages/requestFailures全空，880截图已视觉核对。证据 `C:/Users/rika0/AppData/Local/Temp/nomifun-activity-browser-Aala91` |
| 最终Windows opt-in | d717的ACP936+UTF8混合text/cursor21/冻结空Poll **1/1通过**；普通token net.exe session真实denied5/exit2、原native reaped **1/1通过**；LanmanServer只读已Running，未启停/提权 |
| 最终T36 | d717原AppServices真外部Chrome **1/1通过**：external_pid22824存活、原2页DOM/草稿保留、claims与focus override释放、storage关闭；随后仅夹具owner收自身Chrome。证明provider/AppServices原生退出缝合，不冒充完整canonical模型授权或Tauri host退出 |
| WebView2其他原生 | 此前独立example构建的interaction/dialog-close/close-all三入口通过：真实用户编辑跨hidden/任务结束/后继保留，独占task-only载体自动退休。chooser最终exe SHA `af410d71f65185644531ff5c58763f27477b5ee6525e40f6bf247b1d3b337826`（UTC2026-10-18T23:22:28.8636606Z）三旗标user-files/user-downloads/picker均通过；证据 `C:/Users/rika0/AppData/Local/Temp/nomifun-chooser-native-71c2191005c043fa8231d46c54c79890/run3.json`。修的是单次trusted点击前实时几何夹具，没改生产权限/重放/期限；非人工真实选文件或保存下载字节证明 |

环境：Windows NT10.0.26200/x64，普通用户，rustc1.97.1/Bun1.3.14，ACP936未改，Chrome155.0.8059.82/Edge155.0.4283.45。所有原生夹具使用专用临时数据库、目录和新Profile；日常Chrome未接入。原生过程和构建身份细节另见现役browser-platform验证文档，不作为另一份接续计划。

### 10.3 T01–T37覆盖索引

| ID | 已执行依据／余项 |
| --- | --- |
| T01–T04 | DB执行53/Execution155及实际App AutoWork child原例：三scope、归属非DAG、派生writer fence、局部不聚合，兄弟/root产物/claim保留；原Source Stop非终态CAS竞争4例及当前完整App已收束 |
| T05 | 原Attempt独立Session/调度与Activity控制；父Turn正常结束不形成取消授权，前台切走不处置授权任务 |
| T06–T08 | Session331/DB执行53及remote-reset-id14/Execution155/current native recovery8：同Turn/checkpoint继续、未知/build/Snapshot限制、原预约；实际Bun同PID/owner/starts=1保留 |
| T09 | Process156/App Process5及T20实际原生组合，后台写者不因模型等待/Turn结束被清理，无声明/只读/worktree新门槛 |
| T10、T29 | blocked_steer_preserves_fifo_while_siblings_settle_and_user_stop_preempts与App丢awaiter/重复ACK，Stop独立持久围栏/精确信号，原Steer仍join |
| T11–T14 | 四租约、同Turn只读observer、持久取消恢复、原key/cursor/订阅补读、原running-successor隔离；新8轮warmup旧Stop同Arc/nonce/无新增退休强引用通过。实际OS睡眠/唤醒另待验 |
| T15–T16 | Attached50+真实Chrome3+pending Promise跨target/disconnect1通过；另一target可读/释放，关准入并join原调用，原页/Chrome保留 |
| T17–T18 | Computer 部分改由本次私有 worker 接入与专项进展承接；旧 Computer/a11y/App 桥计数只属退役来源，不作为新实现验收。其他 System 责任与原事实保持。 |
| T19 | App原知识回写自然join/失败保责任，主成功不改写；退休不cancel_all，不把回写变新主任务完成门槛 |
| T20 | 真Bun跨原Runtime退休、原WeakHost释放、显式后继stdin/poll、同native handle/creator/retention/digest、starts=1与最后exact Stop通过；fixture非法stdin wait_ms已移除，arguments_rejected=0 |
| T21–T23 | 第101项公平keyset清理、完成child原Runtime与资源分离、真正workspace缓存退休+factory失败、原native pause/checkpoint/budget/cursor/调用次数不变及Stop后carrier注销通过 |
| T24–T25 | 当前Cron169、App Activity与UI56／真OS10：本run／claim与计划分开，规划／preparation／异常／清理中可发现可直接Stop；当前source3和Browser14使命回归包含于完整1223。Requirement58为原源未变 |
| T26–T28 | 最终稳态3、AI349/原构造unknown隔离及各工具回归见10.1；MCP补14+1通过。完整UI两项专项外HEAD原结构断言未修，不称全产品全绿 |
| T30–T31 | Runtime/真文件缓存稳态、Browser任务页退休、自然process reap/原Frozen回放、SSH exact close；用户编辑与hidden/跨Turn原生证据通过，不能以历史Session永久pin全部Runtime |
| T32–T34 | 当前661681a6真实MCP＋pipe／ConPTY＋10 WebView三native各13原句柄退出，两正常Quit协调器成功；各16PID／JWT／data审计完成。当前shutdown integration2、blocked auth实际竞争和full App通过；未运行平台不扩写通过 |
| T35–T36 | 历史GET/缓存不发业务、显式后继正常懒创建、原生Browser关闭重开与最终外部Chrome AppServices1通过；不承诺任意JS/表单恢复 |
| T37 | 原Process/Job父死与树外sentinel保留；当前三native均同场LanmanServer只读CIM前后身份／Running保留。新建／交接独立服务及原服务句柄连续性未验（标准token错误5），未提权、安装或启停服务；Agent文字不授予跨退出权限 |

### 10.4 Windows 交付时的原生待验边界（macOS 后续见 10.5）

以下保留 Windows 交付时的待验快照；本轮已获得 macOS 主机，新增实测与剩余边界统一接在 10.5，不用旧环境说明覆盖本轮结果。Linux 原生状态未因此变化。

| 项目 | Windows已验／未验 | macOS／Linux |
| --- | --- | --- |
| OS01–OS02 | pipe/PTY/Job/parent-death及ConPTY/ACP共27 native通过，exact handle/owner；不靠裸PID消失 | 两平台watchdog/FD/PTY/job control/EOF/时钟交错未运行 |
| OS03 | WebView2导航/modal/chooser/退出及任务退休原生夹具已验；人工文件选择、保存下载字节/活跃授权下载组合未验 | WK未运行；Linux当前Desktop无managed provider，保持不适用，不为专项补后端 |
| OS04 | 协议50+隔离真Chrome3+pending read并行1+最终AppServices T36通过；任意CPU阻塞renderer/物理输入组合未穷尽 | macOS未运行；Linux当前Desktop未注入Attached provider，保持不适用 |
| OS05–OS06 | 原native排队/token/共享64slot及普通token拒绝通过；真实高完整性/UAC、权限等待/锁屏、物理键鼠执行中Stop及管理员多client组合未验 | macOS AX/TCC/主队列、Linux X11/真实Wayland portal授权/拒绝/撤销及各自管理员设施未运行；源码审查不算编译/native |
| OS07 | 当前661681a6三native各13原句柄退出、两普通Quit协调器成功，JWT删除/data保留/各16PID无残留；三轮LanmanServer CIM前后保留。新服务交接、系统退出／重启及其他worker组合未验 | AppKit/Cmd-Q、Unix信号/系统退出入口未运行 |
| OS08 | 四租约逻辑已验；真实睡眠/唤醒、锁屏/解锁未执行，不能擅自打断日常桌面 | 两平台真实桌面/时钟/睡眠未运行 |
| OS09–OS10 | 本节稳态/SQLx及最终真Edge两桌面尺寸通过；主原生线程任意繁忙/UI断线组合仍有待验范围 | 各自FD/WK/WebContent/portal及Desktop/WebUI原生控制未运行 |

待验不是新的Agent工作门槛。当前未运行项按原要求保留，不伪造通过，不为测试擅自提权、锁屏、安装或修改服务。后续有主机/独立受控桌面后按现役源码、上述精确构建与同矩阵补验。

### 10.5 macOS 接续（2026-10-19）

本轮从本机实际 checkout 接续。10.1–10.4 的 Windows 结果保留为原平台证据；此前“无 macOS 主机”是当时环境快照，本轮结果只以本小节实跑证据更新，不扩大为 Linux 或全部矩阵通过。

| 项目 | 本轮证据与剩余动作 |
| --- | --- |
| 当前收束状态 | 本轮本机授权范围内的实施、编译、适用回归和原生验收已完成；原生验收生产图为下述v4。受控睡眠/锁屏、权限撤销/授权等待、管理员设施及跨平台余项仍明确待验，不能称全矩阵或发布验收完成。下方首失败与v1–v3是各自冻结图的历史证据，不改名为v4。实施阶段未自动交付；用户随后明确授权本次commit＋push，Git交付见本节记录。未安装、提权或发布 |
| 本次 Git 交付授权 | 用户在本轮验收完成后明确要求commit＋push。交付前fetch发现origin/main新增设置UI提交a7a7e0110及整合提交1b28fb1e6；19上游文件与本轮59专项文件无交叉，已安全快进并逐一复核59文件SHA保持。只暂存本轮明确清单，使用原配置NomiFun noreply身份，不改写历史或force-push。上游仅UI/i18n变化，Rust及原生验收源码未漂移；v4原生回执仍归原构建，合并图UI检查单独记录，最终提交与远端SHA以Git记录和交付回执为准 |
| 合并图交付检查 | 快进至1b28fb1e6后的完整 `bun run check`、`bun run build:ui`（22.11s）均exit0；Browser/ChatLayout原204项与5个上游设置测试文件的合并定向UI共263/263、1629断言、13文件、6.02s。最后文档检查与暂存差异检查另执行。仅UI/i18n上游整合，原43份apps/crates验收源码SHA未漂移，未重复Rust530或将旧v4原生App冒称新设置UI整图的原生复验。日志在原根证据目录 `git-delivery-{check,ui-build,ui-tests}.log` |
| 稳定合同与后续入口 | Runtime原flight、Browser用途/门禁及HTTP/资源/storage关停合同已并入现役Execution/Browser/Companion架构。当时跨平台新整图及独立受控场景尚待验；现役合同归入架构，平台缺口归入本验证页，不以旧平台结果冒充当前证明 |
| 最终 v4 构建／源码身份 | 同test/browser-conformance/custom-protocol构建exit0，1m29s；preparer增量构建exit0，7.47s。Desktop source `a5ee8ae5be100052727d00ad894c348f9e0a77d0a0a9679b6a5ccf0481fd0187`（513962912B），独立ad-hoc App `768bfbeabf82f7190fffb8e457ed5d1fa8e54e1ff3b2a5b7b91e32fbd3f27b50`，controller仍`8ff8cec0…`；preparer `40e83927ddf8ffd14d1778ef056478928cda6dea944d7449ed82fc05507a6c4f`。metadata `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-static-package-v4.json`，源码/fixture变更SHA集合 `source-build-v4-identity.json`，无安装/发布 |
| 最终 v4 SDK／App／必跑集成 | App libtest `b068b3abc1e18e89787c7e8e8c28338a08f767864d9900e8258ca51207917b4a`（506618560B，发现1230）；不重叠定向142=Runtime reliability92＋新double direct/background retirement1＋journal15/MCP10/effect receipt13/workspace8/writeback3，全过。AI全库360通过／1原公网search ignored，联合fixture18/18；最终desktop_agent_shutdown2/2、25.69s（`b336f6b5…`），native_execution_recovery8/8、42.18s（`847069d2…`）。合计530通过／0失败／1原ignored，不把App1230发现数计通过。新case证明原Kernel weak已evict后两个直接及两个后台退休都结束，旧Runtime Weak归零；真Bun后台writer原identity/持续写、模型数、canonical Unknown/cursor与后继warmup保持。无carrier的资源retry缺完整flight仍保守Conflict，不以None放行。证据 `/tmp/lifecycle-macos-retirement-v4-final-evidence.json` |
| 最终 v4 T36／真实 Computer 拒绝 | 同b068 AppServices T36 1/1、2.11s，外部原Chrome92609及两页DOM/draft保留，claims/override/连接/storage闭合后仅夹具原owner关闭自己的Chrome，证据 `/tmp/nomifun-macos-attached-t36-v4-final-identity.json`。同签名768bf实际App AX94744／Screen95341两新root均PASS：各自实时bundle/PID/权限before/after `not_granted`，正式typed `SYSTEM_PERMISSION_REQUIRED / not_executed`，原Kernel result23与Runtime result24精确关联，单proposal/单结果、provider2calls/hold1；原Turn Stop→cancelled/producer=false/pending0/unknown0/noReplay，原hold结束。两App/controller/preparer正常exit0/cleanup/final_verified，随机password删除。证据 `/tmp/nomifun-lifecycle-macos-20261019-computer/actual-app-denial-summary.json`；这是host preflight真实拒绝，不冒称AX遍历/capture API内核拒绝或授权弹窗/撤销。首AX driver错把MessageProjection当renderer DTO的失败保留，修原canonical引用读取后换root，未改产品/权限 |
| v4 长观察首夹具失败 | 首root `nomifun-lifecycle-01a15349-73fa-7c5f-8e9a-c2c0131a8103` 把同原Session/op两条owner settled误当重复业务；独立源码核对terminal relay与首次Stop各可持原Runtime观察者，原registry物理注销仅1次，第二worker约1s后复用成功flight，业务Model5/start2/MCP1不变。未改生产，只修harness允许>=1且同scope全部true/已观察前缀不可变，仍要求registry1、成功后无新unconfirmed/missing-owner与原代际保持。首例仅1采样不计70s通过；原FAIL、单独AppKit cleanuptrue/五原NOTE_EXIT和私有失败现场保留，审计 `cmdq-v4-first-longcase-readonly-audit.json`；新root实际通过另列 |
| 最终 v4 原生长观察＋GUI＋Cmd-Q | 新root `/private/var/folders/vp/ljg8x71x2hjdv5ng9xr_5xnr0000gn/T/nomifun-lifecycle-01a1534b-8d1a-73ac-9e84-05729c39256e`，原host96431、同签名768bf。原registry注销成功和原owner full teardown证明后，70s只读观察55采样及截止后再次全量核对PASS；无missing-owner/成功后unconfirmed增长，原Model5/start2/MCP1、Unknown=1/noReplay、原两个Process targets及三原pipe/PTY代际保持。之后同App的完整WK basic PASS（18场景模型调用、总19含seed），人工中文draft跨原Turn完成及原生close-button/恢复保留，两process target前后身份不变；唯一新pending Agent Confirm有准确request/target/operation及真实UI输入门禁。真实Cua Cmd-Q后原controller exit0/cleanup=true/final_verified，五原generation NOTE_EXIT；正常清理与首失败严格分列。原typed PNG、GUI JPEG/AX、键盘回执、canonical原事实和持久数据保留，最终退出时序/审计另取同root `result.json` 与根证据目录 |
| 最终 v4 独立审计 | `/tmp/nomifun-lifecycle-macos-20261019-root/cmdq-v4-final-combined-readonly-audit.json` 与 `cmdq-v4-final-retirement-readonly-audit.json` 均audit_passed=true。窗口70596.804958ms，首末样本单调时钟跨度70305.536833ms；physical carrier1、原scope合法settlement2均true，唯一early active-MCP warning早于物理成功，首次成功后至日志尾unconfirmed0、missing/unproven0。真实键盘→全部五原NOTE_EXIT **412ms**，不以GUI-ready等待203893ms冒充Quit延迟。五原代际已闭合，三个有原记录的辅助PID补查absent（无独立knote的辅助不扩写成generation证明）；JWT/随机password已删，DB quick_check=ok且只读审计前后SHA不变。basic最终PNG338×597/55499B；原draft三AX同URL/姓名/选项，唯一pending工具/原Turn以cancelled seq318结束，不推断缺失callback witness。首次v4失败result SHA复核未变 |
| 同步与保护 | 初始 `main` 工作区干净；`git pull --ff-only` 返回 Already up to date，HEAD=`74646fd874057b70d8b606a18ad1985196ca7e96`，目标提交 ancestor 检查 exit 0。未 stash、reset、提交或推送；后续差异均按专项文件 owner 协调 |
| 环境 | macOS 26.6.2（25G83）、arm64、uid 501 普通进程；rustc/cargo 1.96.0、Bun 1.3.14。Desktop 注入 WK managed 与 Attached Chrome；Computer 使用现役 macOS 主队列/AX；不安装、提权、修改 TCC、锁屏或睡眠 |
| 启动与构建（首轮） | 已完整读取根 AGENTS、方案/启动文件和 Session/Execution/Companion/Browser/全局 OS 架构及 browser 验证文档。按原命令构建 Desktop/browser-conformance；`bun run build:ui` exit 0（20.54s）。构建、回归与原生运行分别留证，尚不把进行中的构建记为通过 |
| 执行分工 | 复用同一 Cargo build 目录串行编译，分别执行 Darwin process/PTY/父死、WK/AppKit/controller、Computer 主队列与只读 TCC、Attached Chrome、App 恢复/退休稳态及桌面 WebUI 控制。证据置任务专用临时目录，首次失败单独保留；结果与具体未验项继续写本小节 |
| 首轮构建／检查 | 原要求 `cargo build --locked --profile test -p nomifun-desktop --bin nomifun-desktop --features nomifun-app/browser-conformance` exit 0，6m11s；binary SHA256 `d07085260a783ea0e830941adf9f83a352eac2e49e7486351e9fab0549048c3a`（505213040B，无 static WebUI，不作为 GUI 证据）。静态 UI 构建另已通过，带 `tauri/custom-protocol` 的后续构建及原生回执见本小节最终图。完整 `bun run check` 首轮 exit 0；独立 Session/Process/Browser/Desktop 边界也全部通过。日志 `/tmp/nomifun-lifecycle-macos-20261019-root/`，后续修复图须另记 |
| OS01–OS02 首轮 | 原五组 integration 实跑：darwin_application 6（4行为＋2helper）、darwin_registered 10（8＋2helper）、parent_death 3、process_contract 19、pty_contract 14；共52顶层执行项／48行为项，0失败。pipe/PTY父死、leader先退出、job-control独立PGID、EOF/真实SIGINT、并发PTY隔离与exact reap通过；不以helper或子进程输出重复增加数量。原日志及构建身份 `/tmp/nomifun-lifecycle-macos-process-20261019/`；macOS多轮FD/PTY稳态另在执行 |
| Computer 适用回归 | `cargo test --locked -p nomi-computer -p nomi-a11y -p nomi-system-execution --lib`：a11y 37通过／1条件ignored，Computer 133通过／8条件ignored，System 14通过；合计184通过／9条件ignored／0失败。mock/协议取消不替代真实AX输入、TCC撤销或管理员XPC批准；真实主队列与静默权限状态另取证。日志 `/tmp/nomifun-lifecycle-macos-20261019-computer/regression-first.log` |
| OS05 主队列 native | 原 `macos_main_queue_lifecycle` 真实 CFRunLoop，3轮 queued 撤回后 effects=0、admitted 调用仍 join 原结果、后继健康调用在主线程执行；FD baseline4→4/4/4，exit0/0.867s。binary SHA256 `2117b3bdb0affa8e33deba6681a9ea5ae2d0658a68ade2b68ba9bc3d5676456c`，证据同 Computer 目录 `native-run-hashed.json`/`manifest.json`。只读 CLI TCC 前后 AX=true、Screen=true，因此真实 denied probe 如实 not_run；该无绑定 Info.plist 的 CLI 身份不证明 NomiFun bundle 已获权。首次误用无 hash 的 example 路径 exit127 保留，不计产品失败或通过 |
| AppKit 主循环 native | 本轮四 examples 按 test/browser-conformance 构建通过（10m59s含锁等待）。原 controller/LaunchServices 实跑主循环 smoke **2/2**：嵌套 NSModalPanelRunLoopMode 中 worker completion各1次、deadline0，原 DeferredEngine get/shutdown 真实 settled；PID67170 kernel exit0、authenticated cleanup=true、final_verified=true。证据 `/tmp/nomifun-lifecycle-wk-native-yV3STn` 和 `/tmp/nomifun-lifecycle-wk-appkit-native.log`。controller SHA256 `beceafc8ac7d8fdc3ae7b5e7c3aef54bc9a24d3eeb4a2cd0c543970784f20fce`；这不替代实际 Desktop 的 Cmd-Q／组合退出 |
| 保留首失败 | Attached 原生首轮非零1例在 NotReady 失败：原 Unix launcher 只开 debugging pipe，而夹具等 DevToolsActivePort。已只为 test/conformance 的自有新 Profile 补显式端口入口，普通生产 Unix 仍 pipe；原Type缺陷及后续真实修复证据见本小节。WK compatibility 首轮在新增夹具 Draft 定位失败：HTML误传入旧 helper 的 script 槽，已修夹具并在后续独立新root通过；原PID67367 exact exit1、cleanup=false，不计通过。原日志 `/tmp/nomifun-macos-attached-native-first.log`、`/tmp/nomifun-lifecycle-wk-compat-native-first.log` 保留 |
| 已证实产品修复 | 隔离 Chrome 端口协议修正后，原 Type 空串替换真实失败（残留“真实输”，`/tmp/nomifun-macos-attached-keyboard-first.log`）。现 Type 用 `ControlOrMeta+A`，Attached 复用原 input dispatch 的 macOS 编辑 commands，保持原 pressed/release owner、两次焦点核对及公共 Press 限制；基础 trusted 输入/清空/focus trap 的新原生运行已通过。另一 iframe fixture 的 End 真实滚动语义暴露位置假设，按原严格 actionability 修测试，不扩大生产 click 容错；最终四例及 AppServices T36 结果见本小节 |
| App 必跑集成 | 原指令两 integration 实跑 **10/10**：desktop_agent_shutdown 2/2（26.86s），native_execution_recovery 8/8（29.11s）。覆盖原写入一次、writer-lock真实失败后显式retry、同Turn/checkpoint/预算继续及背景进程原PID/owner/starts=1，不重放未知效果。日志 `/tmp/lifecycle-macos-app-integrations-first.log`，两binary身份 `/tmp/lifecycle-macos-app-integration-identities.json`；本轮构建在键盘修复前，这些场景无Browser动作，最终 Browser 图另验 |
| 中间源码构建（保留） | 键盘修复冻结后，Desktop＋controller 以 `--profile test --features nomifun-app/browser-conformance,tauri/custom-protocol` 构建 exit0（5m54s含锁等待）；静态 binary SHA256 `bf08953e1fa8f625f2fe4e289734c2c0c34b84543c1048be41c60de187dc82c8`、513788688B。独立 adhoc 测试 App 签名后 SHA256 `fa407643e50e12fa2ab481c4701e3b5a2d2fd95aea887ef2b1ea5e691b82c5ca`，冻结 controller `8ff8cec099d93a105327c67fc23874fdd6fcccbc7651abf689be4d38fb5e2b04`；只构建/签名在任务缓存，未安装。metadata `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-static-package.json`，实际退出仍另取证 |
| 共通／App桥（当时冻结图） | 同图 Session **322/322、114.38s**、Execution **156/156、38.13s**、Cron **169/169、15.48s**，合计647无失败/ignored；使用macOS实际数量，不套Windows数。结果与各binary哈希 `/tmp/lifecycle-macos-core-results.json`。当时 App libtest SHA256 `5486b9aed5441855deb6632221772e278001b5fbe68a297bb0d7d6dc53547e42`（499662720B，发现1228项）；Computer桥整个模块 **26/26、8.91s**，不替代物理设备/TCC撤销 |
| OS09 App 稳态 | 当时冻结 App 独立进程、SQL trace、原固定负载 **3/3、7.66s**。9短Session＋1长任务：baseline FD43/RSS194379776/CPU2819ms/query3687；三轮FD43固定，RSS197427200/197672960/197820416，CPU3381/3937/4497ms，query7474/11261/15058（增量3787/3787/3797），tasks固定(4,7)、disposed3/6/9、model4/7/10。长任务自然结束FD35、registered0/disposed10/query15449。6真文件三轮cache0/FD39固定，disposed2/4/6、model4/8/12、reads2/4/6，RSS201179136/201261056/202620928；query21569/24869/28255（增量3300/3300/3386，baseline18269）。历史/原文件保留，无重跑；trace含夹具观察且有成本，不作为生产绝对CPU/RSS承诺。日志 `/tmp/lifecycle-macos-runtime-retirement-stability-first.log` |
| OS09 Process 稳态 | 新 Darwin fixture 当前源码 **1/1、1.25s**，另 process lib187/187＋startup_probe1/1。4并发短进程预热，三轮各2pipe＋2PTY，12个历史witness保持；长进程原generation/started_at不变，FD13→13/13/13、threads11，原直接子进程集合固定，最后自身stdin EOF自然reap后FD9/children0/admitted0/retained_jobs0/quarantine0。RSS5242880→5472256/5718016/5799936B；fixture宿主getrusage CPU16.585→28.637/40.233/51.712→52.513ms，不包含子进程。早期Mach ticks单位标签更正和所有首日志保留；证据 `/tmp/nomifun-lifecycle-macos-process-20261019/stability-final-evidence.json` |
| OS04 AppServices | 首冻结 App 真外部Chrome **T36 1/1、2.16s**：原Chrome72038在AppServices shutdown后仍存活，原两页nonce/草稿保留，Nomi claims/override/连接释放、重连围栏和storage关闭通过；随后仅夹具原owner关闭自己的Chrome。日志 `/tmp/nomifun-macos-attached-t36-first.log`。两页在attach前由外部夹具创建，当前Attached API无新建tab，不冒称应用新建临时页退休或Tauri宿主退出 |
| OS10 桌面 WebUI | 最终 App 原 ignored fixture **1/1、26.59s**，实际Chrome155.0.8059.82/headless、原cookie认证/WS、新dataset/context。单Stop仅Alpha，Beta Preparing保留；有限AllStop accepted2，原owner未join前stopped0，分别release后消行，turns/model_requests=0。1280×800/880×600无横向溢出，renderer errors/messages/requestFailures全空；880截图已视觉核对。证据 `/var/folders/vp/ljg8x71x2hjdv5ng9xr_5xnr0000gn/T/nomifun-activity-browser-tPXjOA`。UI控制回归34/34、288断言（首次漏ui preload环境失败另保留，未改产品/测试）；不代替WK或任意主线程卡住/重连组合 |
| 实际 Desktop 首轮完整组合 | 首个静态图 fa407643e 的 AppKit／SIGINT／SIGTERM／parent-death／Stop→Runtime退休→AppKit 五例均已完整通过，各新root。每例捕获原host、pipe、PTY launcher＋Bun payload、MCP共5个原generation的NOTE_EXIT；PTY父子unique_id/pid_version匹配，树外sentinel存活，原3Session/owner/event、legacy Profile哨兵/storage-generation保留，JWT/随机password已删。parent-death只杀原host：host65.55ms、全部5原NOTE_EXIT84.90ms，controller signal9/无正常receipt如实记录。正常退出32–40s的真实延迟另作为下面缺陷修复，不把它写成即时退出。完整来源/各root/退出时序 `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-resources-series.json` |
| 实测驱动修正 | 首actual图曾因preparer与Desktop work_root不同被原dataset guard拒绝（保原exit1/cleanuptrue）；下一root暴露Mac PTY ToolResult.pid是session launcher而Bun marker为payload，未改生产PID合同，改用原代际父子证明并均入库存。系统sqlite CLI readonly错误14保留，现用已有Node24 DatabaseSync readOnly真实读同WAL库，不用immutable。所有失败与分立cleanup回执保留，未算通过，不扫描进程名或裸PID清树 |
| 新真实缺陷：退出 drain | 本轮实际静态UI的 `/api/companion/matting-model` 请求持续39021ms；原Desktop先等listener，5s观察失败反复重试且跳过AppServices，直到请求自然完成才开始Runtime清理。已在原HostShutdownStage拆分资源/存储关闭证明，先封HTTP/Relay准入、并行通知原资源owner，再join原listener，全部确认后才storage。原Companion MODNet供给补关闭准入、取消网络/排队、join已开始atomic publish的原worker，不丢Future或增加timeout；模型缓存均在各隔离data/companion/models。原缓存读取open/metadata/bounded-read/hash也由同活动owner持有，丢HTTP waiter仍join原blocking I/O；最终17项MODNet回归和同图真实Desktop退出见下 |
| 新真实缺陷：WK pending dialog | 独立empty native case在原Confirm挂起的service.shutdown中失败，原service ACK=false、ownView0、controller cleanup=false保留。Mac close原先先销毁WK再join已drain dialog的evaluate回调，可能把原完成洗成UNCONFIRMED；现只对已yield且drain确认的原dialog worker在销毁前join，其他需close才能收束的操作保持原顺序、unknown仍严格。默认WK导航/history/popup/四chooser在独立root已完整normal receipt通过；修复图default/empty/nonempty已原生通过（见下），完整GUI的pending Quit已在v3/v4分别取证 |
| U13 非空 WK 真实用途修正 | 原Mac Agent Create要求真实shown/bounds，而旧set_surface(true)无条件永久pin，导致成功非空Agent任务页无法进入task-only退休子集；此缺口未改称N/A或用空carrier替代。现复用同一WorkspaceUses，Slot记录真实native锁ACK；明确Agent自动展示且全程无人类可操作用途、最新hide/detach确认后，才允许原正式使命证明进入退休。任何人工打开/页面命令、缺省来源、未确认门禁或enable前仍可能visible/Attachment均先单调pin；不延长Agent锁、不改Windows原surface路径。原13＋新13退休回归26/26、platform全lib127/127已过；失败门禁worker不能伪造ACK，正式WK非空三轮已原生通过（见下） |
| 来源 UI 与夹具修复 | 原native Attachment新增内部来源（缺省User），macOS自动事件候选仍须core真实锁证明；UI manual intent单调提升，不被自动事件降级；Session key原重挂载与迟到event/link隔离保持，无新用户模式或移动布局。Browser/ChatLayout全目录204/204、1335断言/typecheck/desktop边界通过。App两处原fixture修正：快命令完成后poll是归档只读，mock改选当前schema允许的原poll/launch证据；Activity根canonicalize通过原storage guard。原Runtime89首轮88过/1失败后原例1/1，Activity14/14＋manual1ignored；新test binary7885c27d，原5486证据不改归属，首日志/重现DB均按实际保留范围记录 |
| 最终生产源码构建 v3 | Desktop/controller四examples以 `cargo build --locked --profile test -p nomifun-desktop --bin nomifun-desktop --example macos_application_controller --example macos_appkit_termination_probe --example macos_appkit_main_loop_smoke --example macos_browser_compatibility_smoke --features nomifun-app/browser-conformance,tauri/custom-protocol` exit0（9m42s含锁等待）。source `c7baf937a4aa66d2cbe10fa9d4507abab1ac9c785ffb96e01bafc35814ef1239`、513941760B；任务缓存ad-hoc测试App `5ede9352a4fd7a6ae70d820efe82d8e0895663668ef441bd2965c48e4e74af54`，冻结controller `8ff8cec099d93a105327c67fc23874fdd6fcccbc7651abf689be4d38fb5e2b04`。metadata `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-static-package-v3.json`；未安装或发布 |
| 最终退出／恢复回归 | App libtest `b3a5ba49ec2511a197a596cfbccccd09ad4166ab906516849189225a65270ef2`（506518448B，发现1229）；Desktop/listener19、Services28、Browser gate28、Companion matting17、desktop_agent_shutdown2、native_execution_recovery8，合计102通过／0失败／1原opt-in ignored。实际HTTP屏障证明先封准入、资源关闭而storage仍开，最后HTTP退出才关storage；被drop的cache-reader也保原I/O owner。完整1229未运行，不扩大定向数。证据 `/tmp/lifecycle-macos-shutdown-final-evidence.json` |
| 最终 Desktop 单元／UI | 最终Desktop单元surface-source8＋AppKit termination10＋macOS lifecycle3，21/21；binary `35da1c9f8812fc8c003da4a24494e976faf44936514ba4b2f77e592d45cf1ae8`。Browser平台127/127；Browser/ChatLayout204/204、typecheck、desktop边界通过。完整 `bun run check` 修复图v2 exit0；最终文档与新增夹具检查另记，不套用为全量UI/App测试 |
| 最终 WK/AppKit 组件 native | v2冻结WK源码与v3相同（v3仅增MODNet cache-I/O owner）。同原controller/LaunchServices独立4root：AppKit2 modal case、default导航Stop/queued与claimed popup/精确history nonce与NativeFinished/四chooser取消、empty与nonempty各3轮均exit0＋cleanup=true＋final_verified。非空每轮原Create=1、正式Agent show/hide ACK与exact detach后才使命退休；独立用户页nonce/草稿跨轮保留，原Confirm Stop一次、旧target/dialog不影响后继、pending shutdown与原factory ACK，最终ownView0。binary `fadc47a5373d2f1a11af6b1eed94b1fa9f2934c0d1dc92a82346c9857a41a31f`，签名 `249ac4eded3c89a97e16a61e3dd3d957de1e33f135bc4db02d79ecf127c6435a`；证据 `/tmp/nomifun-wk-v2-frozen-5zcdvaao/final-native-summary.json`。草稿是semantic DOM输入，另补真实GUI输入，不冒称保存文件/下载字节 |
| 最终 Attached Chrome | 当前engine真实四例共7.96s（跨target与原pending disconnect/typed/trusted输入），协议组58（51核心＋7helper，另4ignored）、launch38及mac commands2按原过滤数记录，不重复汇总重叠项。最终App b3a5ba49 的T36 **1/1、1.96s**：外部原Chrome83707仍存活，两页原DOM/草稿保留，claims/override释放并关storage；随后仅夹具原owner清自己的Chrome。证据 `/tmp/nomifun-macos-attached-evidence.json`、`/tmp/nomifun-macos-attached-t36-final-staged-shutdown.log`；未连接日常浏览器，不冒称完整Tauri host附着Chrome组合 |
| 最终实际 Desktop 五组合 v3 | 同签名5ede9352的AppKit、SIGINT、SIGTERM、parent-death、Stop→Runtime退休→AppKit五新root全部PASS；原host84306/84600/84887/85189/85477与各原pipe/PTY launcher/PTY payload/MCP共5原代际均有kernel NOTE_EXIT。正常全部退出500.16/479.55/479.75/450.58ms，强杀父死87.89ms（controller signal9／无普通receipt如实分列）。normal均exit0/cleanup=true，listener timeout0；AppKit例原matting HTTP pending→409精确shutdown原因，client未abort，worker准入另由17回归证明。原三Session/owner/event/legacy Profile/generation保留，树外sentinel存活，JWT/随机password删除。总证据 `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-resources-series-v3.json`，早期32–40s慢图另保留 |
| 真实 GUI 首失败与阻塞处理 | 首basic链原form type/select/scroll/click/submit已到达，但夹具仍在旧tool text局部找PNG，现役adapter把同batch typed附件另发紧邻user媒体消息，导致 `First WK screenshot missing typed image`。原failed report与canonical事件保留，修夹具按唯一call marker/邻接batch/PNG IHDR/target尺寸严格验证，必须新root补实际字节。期间原生工具报告Mac锁定；只对原host85960请求普通AppKit收尾，五原generation已exit0/cleanup=true，Cmd-Q driver因无真实shortcut receipt正确FAIL；不把锁屏当验收。用户现已手动解锁，继续新root GUI链 |
| 真实 Cmd-Q v3 | 新root原host87456，真实Cua native `Cmd+Q` 操作回执与App路径/PID绑定；从键盘调用开始到全部5原NOTE_EXIT **399ms**，controller exit0/cleanup=true/final_verified，原listener关闭、树外sentinel存活、三Session/owner/events/Profile/generation保留，JWT/随机password已删。人工粘贴完整中文draft，经面板hide/show原值保留；原native close-button操作后两原process target身份不变。此前Cmd-W实际关闭的是Browser tab，未冒称window-close或任意网页恢复。证据新root `nomifun-lifecycle-01a15332-a068-703d-a411-908b685a627e/{cmdq-native-summary,resources-before-close,resources-after-close}.json`；basic首截图解析通过后因按钮在窄pane fold外再次FAIL，另保留并修fixture可见布局，不放宽生产actionability |
| 新真实缺陷：迟到 Runtime 退休 | 长GUI观察在Stop已收原cleanup成功证明后，仍有8次 `Original effect resource cleanup owner is unavailable`（1/2/4/8/16/32/60/60s退避）。canonical原MCP仍Unknown但local confirmed/remote not_required、settled与host_cleanup_proven均保留；原teardown成功后exact Kernel弱缓存已移除，迟到退休却重新查Weak并永久pin expected Runtime到Quit。已定位为完成交接缺陷，已用原Runtime retained teardown flight的成功证明收束，不将None视为成功、不重建/重跑、不洗白Unknown；本修复已另构建v4并通过Stop长观察和真实退出（见本小节当前行） |
| 完整 GUI WK basic／pending Cmd-Q v3 | 新联合preparer `d7e8d9b89e45e1f35a3e5c3845603f8c08c8e3d41c2492dcedef8289d98b637a`：截图只接受同tool batch紧邻媒体中的唯一call marker/typed PNG，完整decode核对target/media_type/宽高；计数按钮仅移到提交同行，未加重试/放宽生产refs。新root `nomifun-lifecycle-01a15338-850f-7037-98fa-55d66e2940b8`，真实App PID90112；原basic **PASS**，18本scenario模型请求（总19含seed）、semantic form/select/scroll/click、unsupported drag、旧doc ref拒绝、截图后原ref单次点击count=1，最终338×597 PNG已视觉核对。人工中文粘贴draft跨原Turn完成仍保留。之后仅一次新pending场景，原Confirm request/target/operation和`confirm-open`一次、UI禁用toolbar/Agent输入锁均留证，再真实Cmd-Q取得全部原5代际退出与normal cleanup=true。原失败不重放、不覆盖；脚本/GUI/typed PNG与canonical原事实各自保留 |
| 当前跨平台与未运行边界 | Windows的surface dispatch保持原User分支，但共享Slot native ACK/unlock前用途pin及本次SDK teardown证明有共通变化，旧Windows原生证据不算当前整图复验；本轮不重复Windows实施。Linux/Intel/最低macOS14/正式签名与notarization未运行。AX/TCC实际App前置拒绝及原Turn Stop已验；真实权限撤销/等待授权时取消、物理输入中Stop、SMAppService/XPC管理员多client、睡眠/唤醒/受控锁屏和系统重启退出未运行，当前日常桌面不具已授权独立受控环境，不擅自改变权限或打断系统。chooser只证明取消/关闭，未声称真实选文件、保存下载字节或活跃授权下载组合；完整App1230与全产品全量UI未跑。仅关闭精确自有view/context与原进程，不要求共享WebContent进程归零 |

### 10.6 Linux 接续（2026-10-19）

本节只记录本机 Linux 实跑；不以 Windows/macOS 结果冒充 Linux，也不以 WSLg/XWayland、私有 D-Bus 或 mock 冒充真实 Wayland。证据目录为仓库外 `/tmp/nomifun-lifecycle-linux/`（下称 E），不含 JWT、密码或用户数据。每条证据注明源码／构建；“最终图”专指下表最终构建。

| 项目 | 本轮证据与边界 |
| --- | --- |
| 同步与保护 | 初始 `main` 干净，`git pull --ff-only` Already up to date，HEAD 含 `74646fd874057b70d8b606a18ad1985196ca7e96`。用户告知远端 macOS 更新后，先把全部本地差异备份为仓库外 patch 与逐文件 SHA，只还原本轮测试生成的 ts-rs 绑定，快进至 `a7d79aea2` 再 3-way 重放；稳态采样改用上游共用 Unix `native_fds`。溯源用的父提交 detached worktree `/tmp/nomifun-lifecycle-linux/wt-084c7dc8d` 已 `git worktree remove` 并 prune。未 stash、reset、提交或推送 |
| 环境 | WSL2 内核 6.18.33.2-microsoft-standard-WSL2、Ubuntu 24.04.4 x86_64、32 核／31GiB；rustc/cargo 1.97.1、Bun 1.3.14、Node 24.4.1、Python 3.12.3；普通 uid1000，未用 sudo。WSLg 1.0.73.2（Weston），`:0` 带 XWAYLAND；`XDG_CURRENT_DESKTOP/XDG_SESSION_TYPE` 未设，logind 会话无 seat |
| Provider 适用性 | `apps/desktop/src/main.rs` 3446–3451 在非 Windows/macOS 令 `browser_resources=None`、`attached_chrome=None`：OS03、OS04 与 T15/T16/T36 在当前 Linux Desktop 不适用；本专项未新增后端 |
| 构建身份 | 规定命令 `cargo build --locked -p nomifun-desktop --bin nomifun-desktop --features nomifun-app/browser-conformance`。首图（HEAD `74646fd`）`b8fe5f31…` 及中间图 `c47b1dd2…`、`08fa2ca4…`、`32a49e93…`、`e9665b56…` 均已被替代。**最终图** `c7838d3a84a63524fb765a1049366cfb5a27f4626e12efbfaa3debd763a01234`（949727656B），源码＝已推送 `a4f61f246`；其后只改验证脚本与本节文档，不进入二进制。阶段提交已推送：`64da9b7ca`（调度修复与 Linux 验证）、`3c500d768`（合同清单再生成）、`47e5cb1e6`（Linux 权限与网关测试去环境依赖）、`f860a35e8`（X11 排队输入撤回）、`a4f61f246`（退役不可达 owner-released 写入） |
| 产品修复 | ① Execution 调度竞态：回答在中断对账快照后提交时，原 Attempt 被取消、Execution 判“无可调度步骤”失败；暂停中已准入回答被当作中断调用而转为 review 阻塞。现中断对账跳过任何带待投递 write-ahead 的 Running Agent Attempt，投递候选与空转 `continue` 共用同一谓词，空转失败只按判定快照版本 CAS 提交且与 `fail_if_active` 共用租约／版本权威判断；4 个确定性回归，Execution lib 160/160 ② 每个保留进程都已由清理逐个 reap，App 清理后的 owner-released 写入不可达，已退役并改为失败关闭；既有该事实仍可读取核验 |
| 测试对齐（非产品变更） | ① `pool_lifecycle` 两项（Windows 因无 sshd 跳过）按现役退休中断与断连 Lost 合同更新，3×19/19 ② 4 个仅 Unix 的进程退休夹具改为在真实 canonical Turn 内正式派发 ③ DB 四租约用例写入时取时 ④ `darwin_stability` 改名 `unix_process_stability` 并扩至 Linux /proc ⑤ X11 新增排队输入撤回原生用例：在途拖拽后排队的按键在派发前撤回，无原生效果，拖拽保留真实结果；入口取消用例按实改名 |
| 最终图原生：退出与父死 | `linux-desktop-lifecycle.mjs`（复用共享资源侧车）在私有 Xvfb/Openbox 与隔离数据集运行，pidfd 绑定原 PID＋启动时间，只向核实的 App 发信号，树外同镜像 sentinel 必须存活，App 退出后以 /proc 扫描 launcher 子收割者下的孤儿。E/native-lifecycle/final7-*：`sigterm` exit0、`sigint` exit0、`parent-death`（SIGKILL）-9，各次 App 原后代全部退出（≤484ms／≤469ms／≤32ms），孤儿 0，sentinel 存活，模型请求 5。均为信号退出，不是 GUI Quit |
| 最终图原生：Stop | final7-stop：同一观察窗口覆盖 Turn Stop 与随后的单个 ProcessInstance 精确 Stop——MCP 子进程及其 watchdog、pipe 子进程及其 watchdog 共 4 个退出（≤920ms），PTY 与 App、sentinel 保持；Runtime 释放、MCP Unknown 保留且不重放，启动与模型请求不增加；随后 SIGTERM 时其余原后代 ≤218ms 退出，孤儿 0。精确 Stop 现为共享侧车的请求式阶段（`stop-process.request`），可重复 |
| 最终图原生：X11 与 T28 | `linux-computer-native.sh`（私有 :90、禁 TCP、私有 Xauthority）4/4：焦点／截图／输入、入口取消、排队输入撤回、在途拖拽 Stop。T28 默认 Minimal files-process 19/19、General desktop 86/86（E/t28-final7，会话 `01a1572a-6de3…`／`01a1572a-de25…`） |
| 回归（源码注明） | 阶段提交前分别重跑受影响套件：Execution lib 160/160、调度模块 46/46、App process host／wave2／resource／unified runtime 共 178 通过（1 原 ignored）、desktop_agent_shutdown 2/2、native_execution_recovery 8/8、`pool_lifecycle` 19/19、DB `recovery_handoff`、`unix_process_stability`、wave2 lib 32/32、agent-contracts 127/127、`agent-v2-contract check`、computer_permissions 4/4、网关关停用例 20 次＋模块 3 次、`nomi-computer` lib 154/154，`check:docs` 与 agent-session 边界通过。更早源码的完整 App lib 与其余回归见本节前述阶段，未因后续局部改动全量重跑 |
| OS 映射 | OS01/OS02 见上。OS05 X11 焦点／截图／输入、入口取消、排队输入撤回与在途拖拽 Stop 通过，真实 Wayland 未验。OS06：单任务停止＝Turn Stop 与单个 ProcessInstance 精确 Stop（最终图）；管理员单客户端关闭因无 `/usr/bin/pkexec` 未运行。OS07：只验证 SIGTERM／SIGINT 退出与 SIGKILL 父死；关闭窗口到托盘、显式 GUI Quit、“重新启动应用”与系统退出入口未运行。OS08 睡眠／锁屏未执行。OS09 见上。OS10 无桌面浏览器，WebUI 浏览器控制未执行 |
| 已修存量 | `bundled_role_inventory_matches_native_action_resource_contracts`：冻结清单早于浏览器描述/上传 schema 变化及 Computer 桌面目标中的 Linux，已用 `target_inventory` 与 `agent-v2-contract write` 再生成；`non_macos_computer_permissions_do_not_block_launch`：Linux 映射提为纯函数并以构造桌面状态测试；`default_platform_shutdown_waits_for_gateway_only`：改为探测无监听而非重绑端口 |
| 暂缓与观察 | 按用户要求不在 WSL 强行操作：真实 Wayland portal 授权／拒绝／撤销；OS06 管理员；OS07 托盘／GUI Quit／重启／系统退出；OS08；OS10 WebUI 浏览器。共享侧车改动后的 Windows/macOS 流程未在本机重跑。观察：`9e7ecc3c6` 新增的 `--keyboard-service` 为分离的独立服务，App 退出后存活属设计；驱动只记录后按 pidfd 关闭本次运行自己产生的实例 |
| 证据保留 | E 下保留 final7／final8 原生报告、t28-final7 与回归日志；只保留被最终证据引用的夹具根（无 JWT/凭据），其余带标记的一次性夹具根已删除 |

### 10.7 过程展示与只读详情接续（2026-10-20）

本次用户已授权按确认稿实施：命令／工具可展开、文件默认摘要、子智能体只留委派与终结记录、全部平铺，无单次耗时，顶部总工作时间保留。起点为 `c3e750542` 干净工作区；原实施已按用户后续授权提交为 `249b66d87`。后续文件清单与完成回执沿用同一展示合同；提交、推送与远程整合状态以 Git 记录为准。未安装或发布，不使用真实用户数据作故障实验。

| 项目 | 当前证据与边界 |
| --- | --- |
| 事实与读取 | 原 Event／Payload 按需只读工具详情；委派记录从原 Execution 事件与源 Session 因果读取，不新增表、正文镜像或生命周期账本。原调度、控制范围、并发和资源退休合同保留 |
| 入口与清理 | 普通／伙伴／只读任务及创作过程使用共享展示核心；退役旧 footer、双轨摘要、未使用字段和不可能状态分支。历史完成记录固定原 Attempt，当前操作须显式进入当前任务 |
| UI 与静态验证 | UI 范围 92 个文件／787 项通过，后续快照修正定向 13 项通过；最终 `bun run check` 与 `bun run build:ui` 通过。真实共享组件完成 880×600 与 1280×800 桌面浏览器验收，验证展开、文件摘要、平铺、历史入口和手动开合保持 |
| 子智能体视觉复核 | 对照本地 DeepSeek 的操作／摘要行，退役逐任务全宽状态导航；共享投影仅收拢同源、同阶段相邻记录，默认摘要、名单平铺、原 Attempt 分别导航，按记录数计数。共享查看意图使过时读取不能覆盖后续选择、返回或 Session 切换。本轮 6 个文件／67 项定向回归通过，最终类型、桌面边界、国际化、文档及生产 UI 构建通过；880×600 与 1280×800 验证同源 5 条委派及 2 条完成默认呈现为两个摘要，保留完整展开和历史入口。后端事实、存储与调度未改动 |
| 文件清单与完成回执 | 文件展开退役通用正文读取，仅显示共享 typed 摘要派生的路径与有依据的补丁行数；同 Session／Turn／过程组的相邻确认活动合并，分页读取去重。原生完成报告仅隐藏已确认的无错误回执，失败与未知身份保留，最终回答不变。展示 contract 42、官方生成器 write/check 与持久 schema 隔离通过；后端文件投影 3、实时 3、历史 8、schema 1 项通过。文件分组模型 16、详情及 wire 策略 33、完成策略 61 项定向回归通过，最终普通会话 28／创作 3 项与类型复核通过；完整 `bun run check`、`bun run build:ui` 通过。真实共享组件以固定测试数据完成 880×600／1280×800 视觉复核：展开只见文件清单、完成正文只显示一次，文件展开不请求原始详情。未改变 canonical 存储、完成审核、调度或控制权限 |
| 思考正文连续性 | 原思考全文保留但正文通道切换把尾部拆成独立范围；现由共享纯分类器、独立正文／思考归属和首次 step＋call_id 工具边界统一实时与历史。退役单一 activity、正文封口关闭思考和重复参数／确认重开边界；内部指令审计不占模型调用边界集合。原生事实、512KiB 思考预算、16KiB 缓冲、权限、恢复和并发合同保持；无新增表或持久字段。空推理别名按原优先级跳过真正空串，不过滤标点／空白／短词。Session 完整 lib 334 项、Runtime 与 Contracts 生命周期各 1 项、实际 App 缓冲组件 16 项及 Journal 冷读 12 项、Broker 解码 4 项及 UI 模型／普通流／创作历史 54 项通过；官方契约 check 无漂移，完整 `bun run check` 与 `bun run build:ui` 通过。固定数据的真实共享组件在 880×600／1280×800 展开完整句尾，现役历史从原事件只读重建分段，不改写用户记录；未调用真实模型或发布；提交与推送状态以 Git 记录为准 |
| 后端验证 | Session 328 项、Execution 161 项、Execution repository 59 项、Activity 相关 18 项、工具详情 HTTP 1 项和展示／持久 schema 隔离 1 项通过；Activity 原有手工原生浏览器用例 1 项未运行。生成器 check、边界规则 6 项及精确 Attempt／伙伴投影相关回归通过，不以此宣称全部产品／原生生命周期场景完成 |
| 实模与测试驱动 | `step-5-preview` coding 烟测最终通过：真实写入 1 次、读取 1 次，无工具错误，并通过新只读 API 核验原调用参数和结果。曾暴露测试驱动栈溢出；场景 Future 分配及 HTTP 请求分派现有明确边界，不增线程栈、不改生产 Runtime；16 项无凭据证据测试通过，包含请求超时后实际析构证明 |
| 凭据与数据 | 用户提供的 StepFun 凭据仅允许临时测试输入，不写入源码、命令日志或产物；现役隔离测试 runner 保留 Cargo／工具子进程环境清除与 stdin 交接边界 |


### 10.8 定时任务统一工作台接续（2026-10-20，已实施并验证）

用户已明确授权完整实施以及提交、推送。起点 `1d385e072` 工作区干净；本轮改造首先提交为 `c36dd629c`。现行合同已同步至 `agent-session.zh.md` 和中英文定时任务指南。

本轮确认并根治两条独立故障链：旧 Cron 未准备 Agent 所需资源，导致模型请求之前失败；宿主把编号后台规则拼入用户正文，又会误触真实编号任务的 exact-action 保护，让写入提案反复在 dispatch 前被挡。现在 Cron 使用工作台 Agent，默认 `assistant.general`；原文进入 canonical accepted content，固定后台规则从原 typed source 注入既有 native instructions。未修改 runtime 的多步骤判断、授权、效果保护或时限。

| 范围 | 最终行为与验证 |
| --- | --- |
| 选择与资源 | 共享工作台选择器，独立保留已配置模型。显式重选 Agent 才读取其当前作者配置；仅换模型/项目沿原 exact binding 派生并保留合法资源。新建、Product、Gateway 共用宿主准备；恢复只校验原冻结授权。API Cron35、HTTP 本地错误1、AI typed 分类1通过 |
| 数据与调度 | Cron全目标 **136/136**（130 lib＋6 skill-file），DB相关 **74/74**（Cron repository24＋迁移/reset/ID/provider50）。012前向归一不修改既有迁移或原Agent工件；缺授权保留定义/模型并停用。reserve事务推进下一轮，终态不推进；持续Session在首次Turn前固定。历史来自原reservation；last_run_id只索引确切summary来源。CLI配置、重复快照/历史表、伪重试和成功后绑定死分支均退役 |
| 原始输入与历史 | 原Skill-only空/空白正文及inject_skills保真；当前模型输入、关闭历史、鉴权历史与跨Agent/fork上下文仅从原选择派生引用数据，不读最新Skill或补权。Session整包 **336/336**，Contracts整包 **123/123**；Skill library **128通过/3原有外网ignored**。已有历史不改写 |
| 宿主与贯通回归 | 前轮宿主/认证/provider/typed错误/digest分组51项；本轮source、Skill和历史分组14项通过（集合有重叠，不累加为总数）。Cron HTTP最终 **36/36**，含新建/持续模式真实原生文件写入、原用户正文/系统通道分离、末尾LF字节保真、模型局部修改不换Agent/挂载及幂等派发 |
| 界面与检查 | 最终受影响19文件 **230/230**；完整 `bun run check`、UI生产构建、正式contract write/check、live runner自检、diff空白检查通过。真实隔离浏览器在880×600和1280×800验收默认通用、个人预设、独立模型和菜单/页脚；旧CLI截图及旧文本调度Skill/prompt wrapper已删除 |
| 实际模型/定时器 | 最终隔离生产Nomi-core图，StepFun Step Plan `step-5-preview` **1/1通过，42.75s**。实际write_file与start_process（exit0/reaped）、磁盘固定内容、原生提案/dispatch/result因果、重复key不重复执行、真实At定时器触发并停用、持续2轮同Session、作者默认不变全部证明。运行时build digest为 `ee6983382cecbe4f3fbcb0f2cbdec1d3a2d462d93ab327f4dcbfb9c10ab3d93c` |
| 实模证据边界 | 早期一次通过不能替代最终图；复测曾复现24/25步空转与18个未派发write提案，推动上述原文/系统通道修复。修复后5步完成真实文件/进程，但模型两次省略样本LF；实模任务因此改用严格固定无LF标记，LF保真由受控模型经过相同生产HTTP/原生文件路径单独验证。未更改deadline、模型调用预算、runtime gate或工具参数来迎合样本 |
| 数据与凭据 | 所有执行/迁移实验使用内存或临时数据库、隔离目录；没有在真实用户库上试验。实模密钥只经无回显stdin交给临时Profile，正常关闭与明文落盘审计通过，不进入源码/命令参数/报告。真实用户历史与配置的升级由现役012合同处理 |

本轮未执行Windows/Linux原生打包、物理Tauri设备/TCC/浏览器验收或发布；不把本机隔离renderer/Nomi-core验证当作这些平台证明。缺授权旧启动配置须明确重新选择工作台Agent后启用；已有Session的Agent/项目修复仍走其显式配置切换，不自动改写原授权。

构建同时使用 `--target-dir /tmp/nomifun-cron-workbench-validation-target` 与 `--config 'build.build-dir="/tmp/nomifun-cron-workbench-validation-target/build.noindex"'`。只精确中断本任务自身误排图的Cargo，未中断用户桌面开发进程。

提交前正常合并远端 `add2284fc`，保留其 Computer 私有 worker、actual-attempt projection 和清理接线，重新生成合并后的契约摘要。开启 `computer-use` 的 macOS 贯通复测另外发现：初始工具表把当前平台缺少 Computer capture 当成整次 General 启动失败，随后 `Conflict` 文本又被误归为模型供应商 schema 错误。现初始表沿用 Kernel 的精确平台投影，只省略已知但无本平台定义的 Computer 动作；未知动作、异常投影和其他 schema 仍失败。宿主准备失败保留 typed `Internal` 来源，不能被错误文本中的 `tool schema`、401、503 改写归属。跨平台 canonical 授权、原资源绑定和实际模型调用合同保持；当前 macOS Computer 原生能力仍按其专项计划补验。

合并验证：Contracts **122/122＋bin 5/5**、正式生成器 check、受影响 UI **230/230**、最终完整 `bun run check` 通过。开启 `computer-use` 的鉴权 **18/18**；修正后的 Cron HTTP **36/36**、工具表模块 **19/19** 通过，包含 General 普通工具、原生文件末尾 LF、严格重编译、未知动作拒绝与 typed 本地错误归属。原“所有 canonical 输入必须是单个 object”的测试已按现行 Computer 跨平台授权集合逐项校验各平台严格输入，不放宽生产模型 schema 规则。文档检查与工作区/index 空白检查通过；原实模证据归属于上表的合并前构建，本次平台隔离复测使用受控模型，没有追加真实模型或原生 Computer 支持声明。

### 10.9 普通输入投递、引导与时间线（2026-10-21，已实施并验证）

本轮按用户确认的全局方案接续：空闲立即发送、工作中默认排队、显式引导进入原任务，重整正文与队列的唯一展示位置；不增加配置、不扩大到独立调度器或第二账本。当前合同集成至 `agent-session.zh.md`／`agent-session.md`，本节只记录当前实施和验证，替换先前已被本轮设计取代的交接说明。

- 原 `/messages` 受理和现有 Host owner 保留输入、wake 与精确执行责任；提交后先交接 wake，再做观察读取／通知。重投已消费或取消输入只确认原事实，不推进无关后继。
- typed `SessionInputView` 贯通 ACK、通知、队列与历史，明确 placement、原 Turn、root／steering 与排序锚点；消息正文仍仅存原 accepted payload。读取、缓存和展示不获得执行权限。
- 普通排队项只在 composer 队列出现；空闲首项直接进入正文，显式引导先出现在正文尾部，原生接收后以同一身份进入原回合的实际边界。用户气泡下不显示投递／执行状态文字，typed 视图仍用于业务投递、队列控制和编辑权限。过程按引导分段，分页／迟到事实不再跨边界整轮合并；纯 UI 展开记忆不保留 Runtime。
- 队列编辑改为内联、保存时原子替换；正文不写进控制事实，附件／技能／有效意图／跨来源位置／hold 保留。显式 clear／pause 只作用于队列项，原 Stop 仍 hold 其受理截止批次；准备阶段 Stop 通过原 source 精确取消，等待原准备 owner 收束，不伪造 Turn。
- UI/API 合同从实际 HEAD 的 45 递增至 46，保留同期 Computer 合同；不修改物理表、历史迁移、Agent 数据代际或 checkpoint。旧 pending 通用提示、回草稿前先删除输入的编辑路径、跨引导整轮聚合及旧按钮断言随替换退役。

同日后续按用户要求移除用户消息气泡下的投递／执行状态文字。下表既有视觉证据归属于移除前的首次集成版本，保留原截图与回执，不作为此次文案移除后的视觉验收证明。

| 范围 | 本轮最终证据 |
| --- | --- |
| Canonical 输入 | 最终输入／队列／准备停止定向 **35/35**，含跨来源替换位置、hold、领取竞争、重复键与重启。Session 整包首轮 **370/371**；唯一失败为旧用例在终态后继续使用旧队列版本，修正观察版本后纳入上述 35 项通过，不把首轮整包改记为全过 |
| Host 贯通 | 最终 **11/11**，17.14s；生产 HTTP 及受控本地模型验证空闲立即到达、忙碌 FIFO、终态后新发送、并发空闲唯一领取、引导精确进入原 Turn。准备停止验证持久围栏、原 owner 收束与跨分支幂等键；显式暂停只 hold 真正排队项，不撤回正在准备的引导 |
| 正式契约与存储保护 | Contracts 整包 **126/126**、API agent_platform **17/17**、数据库 Agent reset **3/3**，正式生成器 write／check 通过；不新增物理表、迁移或数据代际 |
| Renderer | Messages 完整同进程 **629/629**（55 文件、3371 断言）；composer／队列相关 **91/91**，另有消息合并、wire、runtime／identity 定向回归通过。各集合有重叠，不累加为总数。629 项保留工具结果摘要回执，未冒称保留完整 stdout |
| 静态与构建 | 完整 `bun run check`、UI 生产构建、最终 TypeScript 检查和差异空白检查通过；最终文档和 Session／桌面 UI 边界另执行。保留现有构建 chunk-size／deprecated 警告，不扩大修复范围 |
| 视觉与边界 | 使用生产组件的隔离浏览器在 **880×600、1280×800** 验证：空闲正文准备且无队列条、紧凑队列、引导前／后过程顺序、内联编辑和可见 Stop，均无横向溢出。临时 fixture、浏览器页和测试 Vite 已清理；不作为物理 Tauri 窗口或真实外部模型验收 |

本轮证据目录为 `C:/Users/muli/.codex/visualizations/2026/10/21/01a15e6a-3be5-7b81-b6e5-d249464be2e5/`，含截图、视觉回执及 `input-queue-validation/` 日志。首轮 Windows ONNX 链接因继承的离线变量失败，随后只清除测试进程变量，使用已有分发缓存正常重编；沙箱首次运行在测试临时 workspace 的共享 authority 文件初始化失败，最终同一测试程序经所需权限运行。旧 Pause 用例按本轮确认合同补齐 queued-held／原 Turn steering 断言后，最终 11 项全部通过；失败记录保留，不改生产锁路径、依赖或权限规则。

保留用户原有 `Cargo.toml`／`bun.lock` 改动，不纳入本轮提交。用户在验证完成后明确授权本轮 commit＋push，交付身份以 Git 记录为准；未打包安装或发布。未执行物理 Tauri 窗口、其他平台构建或真实外部模型验收，此前专项的实模报告不作为本轮最终图证明。
