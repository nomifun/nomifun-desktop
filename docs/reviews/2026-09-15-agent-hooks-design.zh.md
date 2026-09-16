# Agent hooks：现行范围与成本取舍

> macOS 接续：H1a before_tool 已实现并取得原生核心产品闭环证据，见
> [实施记录](2026-09-15-agent-tool-hooks-implementation.zh.md)。H1b after_tool 因成本偏高已移出开发计划。
> 保留 before_model、before_tool。用户后续取消会话页替换 U0/U1，普通 App UI/常驻插件不受影响。
> 当前失败、限制及正式放行以实施/发布台账为准。

日期：2026-09-15。原设计基线：`70c28b5de`。现行决定：保留已实现的 before_model/before_tool，取消 after_tool 及其专属前置改造，不保留延期卡或空接口。完成开发的能力必须具有普通用户可发现、可选择、实际生效的完整链路，不以默认关闭的实验开关交付；正式支持声明按实际验收确定。

## 1. 目标与不做项

让用户在 Agent 配置中选择插件能力，对模型请求和工具执行前操作做业务约束。插件不替代模型 Provider、Agent 引擎或会话 owner；工具后结果整理已取消。

复用 capability 发布/选择/冻结、现有 `middleware_order`、Product 普通 Service、一次性宿主装配及取消/结果未知保护。不新增 HookRegistry、第二事件总线、后台队列、动态 Runtime、Rust 插件、Service streaming 或插件专属 shell 执行器。

第一批只支持 Nomi + PluginProduct。其他引擎/来源未接入时在保存和启动时明确拒绝相应阶段，不能静默忽略，也不把现有配置型 shell hooks 宣传为产品插件能力。作用范围按真实外层工具调用验收，不声称覆盖工具内部所有嵌套动作。

## 2. 代码事实与复用点

| 现有落点 | 已有行为 | 设计含义 |
|---|---|---|
| `nomifun-agent-contracts/src/model_middleware.rs` | `agent.before_model` 精确 action/schema、Hidden、Pure、无资源声明 | 扩展时沿现有发布合同加实际阶段，不建平行 manifest 系统 |
| `nomifun-ai-agent/src/model_middleware.rs` | 精确来源/合同验证、冻结排序、绑定原 Product invoker | 增加阶段时必须同步消费者选择和准入；不能只把 schema 加进目录 |
| `nomi-agent/src/model_middleware.rs` | 可编辑 system 与本次工具子集；共享 5 秒期限、输入 256 KiB/输出 64 KiB | 已有行为保留，不借新增 hook 放宽权限或改现有合同 |
| `nomi-agent/src/engine/mod.rs` | 动态 Context 后运行 before_model；宿主可信规则随后追加 | 不开放可信授权规则、历史事实和模型凭据给 patch 修改 |
| `nomi-agent/src/tool_execution.rs` | 有配置型 pre/post tool hook 调用；工具 deadline、panic/error 处理与结果脱敏 | 是工具阶段候选落点，但不能直接把现有 post hook 当作安全插件输出口：当前 shell post hook 位于最终脱敏前，且提前返回/timeout 不经过它 |
| `nomi-config/src/hooks.rs` | 配置 shell hooks：pre 可阻止、post 返回消息、stop 执行命令 | 保留正常配置功能；不将 capability 插件转换成 shell command，不迁移为第二执行器 |
| `nomifun-ai-agent/src/plugin_tools.rs` | `NomiHostedSessionBindings` 和 retained invoker | 新消费者与既有消费者一起绑定最终作用域，不能重新引入二次绑定 |

以上是静态核对，不是新增阶段测试通过记录。

## 3. 能力面与优先级

下表区分已实现能力与取消项；取消项不生成 schema 或开放占位入口。

| 阶段 | 类型 / 用户收益 | 拟议输出与边界 | 安排 |
|---|---|---|---|
| `agent.before_model` | 现有请求变换：业务提示、缩减可用工具 | 现有 `system`/`tool_names`；不能补回前面删掉的工具 | 保留 |
| `agent.before_tool` | 执行前门禁：业务规则检查、禁止危险参数组合 | `allow` 或 `deny` + 有界原因；不改参数、不批准权限、不代替人类审批 | H1a 已实现，按实际证据验收 |
| `agent.after_tool` | 原候选：工具结果文本整理 | 安全实现依赖结果保留、文本分区及媒体结算改造 | 成本偏高，已移出开发计划 |

H1a 已有独立产品价值，可以保留。H1b 的收益不足以支撑当前已证实的跨层改造成本，按用户最新授权取消；不把它换成另一个结果 owner 重构项目继续推进。

已移除 after_turn/H2、after_model、输入前处理、会话创建/关闭、压缩前后、逐 token、通用自动重试/继续执行等扩展候选及其设计，不保留“下一批再评估”的排期。原因分别是：缺少直接业务消费者、与现有输入/Context 功能重叠、或涉及过大的状态/副作用控制面。现有通知、配置型 shell hooks、上下文压缩和错误处理功能不删除；取消的是新的通用插件接入口。

## 4. 发布、选择与执行的最短路径

```text
插件发布 capability（明确阶段合同）
  → 用户选入 Agent，必要时调整现有 middleware_order
  → 原编译/引擎准入冻结身份、参数和顺序
  → Nomi 在实际阶段调用原 Product Service
  → 宿主校验返回值，再继续或按明确失败语义停止
```

- 首批一个 capability 对应一个阶段 action，沿现有精确合同验证；同一个插件包/Service 可以贡献多个阶段，无需多个插件或进程。作者模板负责减少重复声明。
- 用户入口复用 Agent 能力选择与排序，按“执行扩展”展示阶段说明；Hidden hook 不变成模型可调用 Tool。不额外要求用户配置事件总线、Runtime 或第二份绑定。
- 开发完成即在支持的 Nomi Agent 配置中正常展示，普通用户无需环境变量或开发者模式。未选择时不调用；选择后必须实际生效，不默认替用户启用任何第三方插件。
- 同一份 `middleware_order` 按阶段过滤。保持现有“显式顺序在前，其余按 capability ID”的确定性规则；不增加阶段之间的任意依赖图。
- 保存/预览和启动共享消费者准入。运行前重新检查精确 release、启用状态、授权；旧会话使用冻结选择，但停用/撤权仍立即影响调用资格。
- 沿现有装配扩展阶段消费者。必要时把当前 model-only 适配模块重命名为准确的阶段模块，但不保留两套永久兼容入口、不扩通用编译器为 Nomi 业务解释器。
- Hook 本身的 Service 调用不再次触发工具 hooks；只围绕声明的外层 Agent 工具操作调用，避免递归。一个复合工具的内部操作不因名称相似而重复挂钩。

## 5. H1a：执行前检查

### 输入与权限

最小输入是宿主生成的关联 ID、阶段、工具名称、合法参数视图及必要的回合标识。不传完整历史、模型认证材料、宿主句柄、图片字节或任意路径读取权限。参数可含业务敏感信息，用户选择时应明确该 hook 可读取什么；现有 Service 信任边界不能被“Pure”标签当作操作系统沙箱。

不为 hook 扩大原工具授权。对尚未获准、名称未知或 schema 非法的工具调用，先由宿主拒绝，不能先把数据交给 hook。边界工具若在内部才做最终资源授权，H0 必须确定安全的前置数据视图和接线点；无法保证时明确不支持该路径，不将已选择 hook 静默跳过。

敏感字段按宿主策略遮蔽，并显式标识；门禁不可在不知情的情况下接收截断输入。超限则明确失败，不默默截断后放行。首批沿用现有 256 KiB 输入、64 KiB 输出上限作为上界，在实现时为原因文本设更小限制。

### 行为

正常顺序：原工具准入/参数检查 → 配置型 pre hook（若有）→ capability before_tool → 原工具派发。任何前置门禁拒绝都不派发目标工具；最终工具/资源授权仍保留在原执行路径。

- 多个插件按冻结顺序执行；全部 allow 才继续，任一 deny 结束本次检查，后续插件不能撤销拒绝。
- deny 产生明确的“被 hook 阻止、目标工具未执行”结果，保留原 tool-call 关联；交给既有工具错误策略处理，不伪造成工具成功或自动重试。
- 技术失败、非法输出、超时、停用与撤权不得当成 allow。停止本轮后续执行并报告错误，沿既有 retained task/未知结果规则处理 hook 自身的在途调用。
- 首批不支持参数 patch，因此无需新增“修改参数后重新审批”状态机；有需求时再独立设计，不用 allow 返回值携带隐式改参。

## 6. H1b：取消决定与成本依据

只读核对发现，安全实现 after_tool 需要同时解决：

- engine 中成功结果跨 post 等待、并行批次取消和 accepted-root 恢复的保留问题；
- manager 的取消/失败处理不能先把已完成工具改记为 Error；
- 媒体 sink 的 Delivered 只是内存接管，Conversation 在失败终态还会回滚待交付附件；
- 当前 ToolResult 没有可编辑文本、宿主警告和资源引用的独立边界，不能把成功正文整段授权给插件替换。

这已超出增加一个回调的成本。按用户“成本高可以不实现并移出开发计划”的授权，
取消 after_tool 及专属结果 owner/媒体子集结算改造，删除未开放的合同草案，不保留
H1b-A 等前置卡或延期计划。运行时继续明确拒绝不支持阶段，既有 shell post hooks 保留。
审查发现的既有风险仍记录在实施台账；取消能力不等于修复这些风险，也不自动授权另一轮重构。

## 7. 时限、并发与结算

- before_model 既有 5 秒总期限不变；before_tool 共享最多 5 秒，并受原工具绝对 deadline 约束。工具本身的执行期限不延长。
- before_tool 检查超时不得派发目标工具。after_tool 已取消，不为它新增结果消费阶段或调度器；既有 shell post 的审查风险见 §6。
- 同一工具的 hook 顺序串行；不同工具继承现有合法并发策略，不加全局锁。Service 作者需遵守现有并发调用约定；身份包含本次 invocation，不能只靠模型给的 tool-call 字符串隔离。
- 取消后不启动后续 hook 或工具，不自动重放。沿原 retained invoker 等待实际退出/有界回收，既有未知效果保护不削弱。
- 诊断只记录阶段、capability/release、关联 ID、耗时、状态与安全摘要；不要把完整参数、system patch、返回正文或凭据复制进日志/恢复上下文。

## 8. 实施卡与验收

| 卡片 | 实施范围 | 退出条件 |
|---|---|---|
| H0 接线确认 | 沿 Nomi 工具准入、实际派发、结果保留/结算定位；列内置/Mount/Product/MCP/复合工具覆盖矩阵及其他引擎拒绝行为 | 每个声称支持的调用点有真实 owner 与安全视图；不产出空接口，不全仓重构 |
| H1a 首个纵向阶段 | before_tool 合同、发布准入、模板、现有选择/排序、消费者与生产调用接线 | 用户发布/选择的检查真实阻止或放行一次目标操作，未选 hook 时原路径不变 |

H1a 的跨层实现已完成，剩余按实际支持范围验收；H1b 不再是实施卡、发布门禁或 U1 的依赖。

验收至少包括：

1. 不设置实验变量的普通用户完成发布/选择/保存/新会话/真实 Node/实际工具/模型后续调用的纵向测试，不只 mock hook trait；不能只交付后端接口而没有发现/使用入口。
2. 无 hook 的行为回归；当前 before_model、配置型 shell hooks 保持原语义。
3. 未知阶段/错误合同/不支持引擎或来源保存拒绝；顺序、旧会话冻结与停用验证。
4. allow/deny、异常、非法 JSON、超限、timeout、用户取消；失败门禁下目标工具执行计数为零。
5. 脱敏、来源标识、宿主不可编辑字段、递归防护、并行工具隔离、后续撤权。
6. 覆盖实际声明的内置/Mount/Product/MCP 调用路径；没有支持的来源清楚拒绝，不能以单个演示宣称全路径。

## 9. 跨平台与交付

Windows 开发时复用定向 Rust、真实 Node、合同生成检查及必要 UI 测试；涉及选择 UI 时补桌面边界检查。跨层实现卡收口再运行 `bun run check`，不每修改一个字段重跑全仓。

macOS arm64 TODO：同一提交下验证已支持 Node hook 的取消与进程回收、并发、空格/Unicode 路径；安装制品验证选择/保存/重启与实际 hook 效果。before_tool 是普通 Service，不执行配置型 shell command，因此不新增 Bash/PowerShell 兼容实现；现有配置 hooks 不因本次成本取舍被删掉。

跨 OS 接续时先读发布台账与本设计，确认哪张 H 卡已实际实现，再跑对应用例。不得将本文设计作为实现证据，或借机恢复模型插件、动态 Runtime、Rust 插件、Service streaming、UI 专用订阅。

当前 release 按 before_model、before_tool 及其他保留能力验收；双平台和制品未完成项仍如实保留。after_tool 与会话页替换 U0/U1 均不再是交付条件；普通带 UI App/常驻插件继续保留，源码、原生验收及正式发布证据分别记录。
