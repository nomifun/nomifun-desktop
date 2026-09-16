# Coding 完成报告与观察依据（2026-09-14，未验证）

本次仍只修改本地 `rf/agent-capability-platform-v2`，没有 commit/push。
按用户要求未运行构建、测试、评测或 E2E；仅源码实施与小文件 rustfmt。
当前 Coding build 为 `host2-coding-loop12`，摘要已包含新 `completion.rs`。

## 发现的问题

原循环已记录命令启动/终态、工作区 epoch，并在结束前检查运行中进程和未关闭计划。
但 `update_plan` 的 completed 标签与用户任务的依据没有逐项对应；一次 completion review
之后，即使只剩模型的完成声明，也可能走正常 Completed。

对照本地 Codex 的 `tools/handlers/plan.rs`、`plan_spec.rs` 与
`context/guardian_review_evidence.rs`：计划工具本质是控制/展示状态，不是执行证据；
审查依据需要有界保留，并与对应的用户输入版本匹配。此处借鉴这两点，没有把 Guardian
授权逻辑复制为 NomiFun 权限来源，没有引入第二套 Session 或新的自动验证权限。

## 新的 Coding 控制能力

新增引擎内置 `report_completion`，不是 Kernel 能力或社区 runtime 的公共必选策略。
它必须单独成批调用，不能与 effect、plan、activation 或 resource 调用混用；已选工具
不能使用同名覆盖。纯文本、无工具且无计划的回答不强制调用。

提交必须覆盖当前计划每一项且不重复；没有计划时使用唯一的 `response` 项。每项包括：

- `supported`：至少引用一条本轮仍保留的、成功且可用、与当前工作区 epoch 一致的工具观察。
- `unverified`：说明未运行、用户排除、不可用或证据过期等原因；不能据此获得运行验证的权限。
- `blocked`：明确未完成的事项。计划中的 blocked 项不能在报告中伪装为 supported/unverified。

`rationale` 和总说明仍是模型的解释，不是平台认证。报告拒绝未知/遗漏/重复步骤、虚构/
重复观察 ID、过大的数据及未结束的计划或进程。全报告最多 24 KiB、16 项、每项 8 个引用。

## 观察与失效

`CompletionTracker` 保留最多 64 条调用元数据，不复制命令环境、文件正文或私有推理。
只在收到已经结算的工具结果后写入 `CompletionObservation`。进程结果还必须对应已有
命令记录：正常退出、退出码 0、cleanup.reaped、无重叠运行进程，且启动/观察 epoch 当前。
这只证明该命令被观察到正常退出，不能认定它运行了测试、更不能认定测试覆盖全部需求。

每条普通工具结果使旧报告失效；plan/activation/resource 控制也使它失效。报告绑定：

- plan revision；
- 工具/控制观察 revision；
- 本轮累计 accepted input revision（steering 新输入会改变它）；
- 工作区观察 epoch。

报告持久记录后才进入内存可用状态。新的报告提交失败会撤销旧报告，不能退回旧成功声明。
受控上下文保留观察元数据和报告摘要，compaction 不删除这些版本检查；完整报告保留在
Conversation 的原有日志。历史回放只作为带标注的数据，不复用为新回合的完成依据。

## 完成门控与产品输出

用过工具/计划的回合，正常结束前必须有当前有效报告；可以有一次有界补充报告机会，
仍没有有效报告时失败退出，模型总步数仍限制循环。未结束的进程照旧由宿主清理，报告
本身绝不是进程退出、资源释放或事务成功的凭据。

报告含 blocked 项时，不发布正常任务完成终态。报告含 unverified 项时，可以结束本轮
交付，但引擎会把未验证项及原因追加到最终文本，不仅依赖模型自行提及。此行为不把
“实现已交付但未验证”自动改成“测试通过”；原有 Agent 指令和用户范围仍优先。

## 尚未解决的边界

- 这是可追溯的完成说明，不是机器自动证明全部需求正确。计划/criteria 仍由模型表达，
  是否遗漏用户需求、某项观察是否足以支持其解释，仍需更强的任务语义核对。
- epoch 是保守的可能副作用顺序，不是工作区内容哈希。即使只是运行一个检查命令，
  也可能推进 epoch，因此更早的检查不能自动覆盖最新状态。没有按命令名白名单把它
  当作绝对只读；不为了取得 fresh evidence 而绕过用户“不要验证”的要求。
- 相同成功命令可以被模型解释为多个 criteria 的依据，系统能核对来源/时效，不能单凭
  通用工具结果判定测试覆盖或业务正确性。`supported` 刻意不命名为 `verified`。
- 新控制调用会占用模型步数；既有固定响应 fixture 与多 provider 的行为仍需要后续
  验证更新。本次没有开展这些验证，也不引用历史通过记录作为本切片证据。
- MCP/MiniApps/非 function Plugin、非制品 Skill、Git push 凭证、跨启动进程证明、
  人工隔离处理与安全 checkpoint 续跑仍待推进，整体 engine 目标未完成。
