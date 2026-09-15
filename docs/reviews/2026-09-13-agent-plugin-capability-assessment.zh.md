# Agent 插件能力评估：当前范围与取舍

初评日期：2026-09-13。当前决策：2026-09-15。

本文按最新产品决定重整，删除外部模型 Provider 插件及安装型/热加载 Agent Runtime 的未来方案、排期和验收欠账，不再保留相互矛盾的旧路线。旧版详细推演可查 Git 历史；已实施工作的事实和测试记录保留在[实施台账](2026-09-14-agent-plugin-openness-implementation.zh.md)。

当前代码基线：`70c28b5de86f820a535bdcca48191244196f2e2f`。其上新增范围筛选与已有页面开关移除；新 hooks 尚未实现，双平台正式制品仍待验收，结果以发布台账为准。

## 一、目标与最新结论

目标是让用户尽可能组合/替换有价值的 Agent 能力，不是把所有宿主部件变成安装型插件。

| 事项 | 当前决定 | 理由及保留边界 |
|---|---|---|
| 用户独立 capability | 保留已接线能力 | 系统与用户实现使用不同 ID，用户选择自己的同类工具并取消内置工具，不覆盖系统 ID |
| 外部插件模型 Provider | 取消，从全局开发计划删除，不再列为延期欠账 | 内置 Provider/连接/协议适配已承担模型接入；维护外部实现的选择、凭据和模型流生命周期没有当前必要收益。保留正常模型配置、Broker、合同与源码协议扩展 |
| 整个 Agent Runtime | 保留已实现的源码接入与编译打包选择；取消安装型/热加载扩展计划 | 引擎可替换不等于必须热加载；不重复实现 Engine Catalog、执行 owner 或动态加载器 |
| hooks | 只新增 before_tool、after_tool，已有 before_model 保留 | 直接覆盖业务检查与工具结果整理；其他阶段移出计划，不保留空接口或待办；见[hooks 设计](2026-09-15-agent-hooks-design.zh.md) |
| 插件 Agent 页面 | 移除实验开关，普通用户可选；新增只保留业务输入与结果展示 | 只替换会话工作区，宿主继续拥有会话状态与权限；默认内置不等于禁用插件选择 |
| 应用 Shell 替换 | 不做，移出开发计划 | 全局布局/路由及新插件插槽框架没有当前必要收益；保留原有正常导航和页面容器 |
| Rust 原生插件 | 已删除 | 不保留后端、作者 SDK、专属打包与启用入口；Rust 宿主继续保留 |

取消模型插件不等于所有未来模型协议都已支持。内置适配无法满足的新协议，可按正常模型系统需求修改源码，不自动恢复外部插件入口。

## 二、必须区分的两条开放方向

1. **系统能力可插拔化**：Agent 配置选择能力/实现，实际执行使用该实现。独立工具选择、已接通的 discovery、源码引擎选择属于这一方向。
2. **插件插入系统环节**：宿主在确定生命周期调用插件。`before_model` 及拟增加的工具阶段 hooks 属于这一方向，不证明模型 Provider、调度器或执行引擎可由该插件替换。

两者复用身份、冻结配置、授权和普通 Service 执行，不再各建目录或另一套组装系统。源码引擎接入和 JS 插件是不同交付方式，不要求所有部件具有相同装载方式。

## 三、当前插件能进入哪些环节

| 环节 | 已支持范围 | 不应扩大解释为 |
|---|---|---|
| Tool | 已接线的 PluginMount/PluginProduct 工具，独立 capability 选入 Agent | 任意系统内部调用都已改走用户工具 |
| Role/Provider | 通用选择、依赖与冻结；限实际消费者与支持来源 | 所有领域/来源对称开放，或外部模型 Provider |
| Tool discovery | 内置和受支持的 Mount/Product 策略选择、实际消费 | 任意 Catalog、预算和 schema 暴露策略已开放 |
| Context | 受支持 Mount 来源的初始及动态 Context | 整个 Prompt、记忆或历史管理已可替换 |
| Skill | 精确包身份、只读正文/资源与显式命令；共享装载器 | Product manifest 已接受 Skill，或能执行 shell/fork/Skill hooks |
| 模型请求前 | Product `agent.before_model`，调整可编辑 system 和缩减本次工具集合 | 模型调用实现替换、历史重写、增加原授权外工具 |
| 工具前后 hooks | Nomi 有配置型 shell hook 执行点；capability 插件阶段扩展仅设计 | 配置型 hooks 自动成为产品插件 API，或已覆盖所有引擎 |
| Agent 页面 | 无实验环境变量即可发布/选择，使用宿主 observe/turn/cancel | 全部交互、Shell 或会话 owner 替换 |
| Browser/Computer/MCP | 保留现有合同及实际接线范围 | 所有内置派发都已可选用户实现 |
| 规划、记忆、调度、其他深层部件 | 继续使用内置实现，不增加占位协议 | 全部必须开放才能发布 |

详细支持矩阵以[架构收敛记录](2026-09-15-plugin-architecture-convergence.zh.md)为准。未实现能力不是一张必须清零的“全栈插件”账单。

## 四、模型与 Runtime 的代码去留核对

本轮核对了合同、Product 发布/执行、Agent 装配及引擎注册路径。结果是：没有发现仍需删除的外部模型 Provider 或动态 Agent 引擎专属实现；不能为了产生代码删除量误删正常产品设施。

| 代码 | 核对结果 / 处置 |
|---|---|
| `nomifun-agent-contracts/src/chat_model.rs`、`nomifun-chat-model-broker/src/contracts.rs` | Broker 直接复用同一套模型数据类型，不是待用的插件模型 schema；保留 |
| `nomifun-chat-model-broker/src/engine_port.rs`、App `engine_session_host.rs` | 源码引擎访问现有 Broker 的入口；路由/凭据/重试仍由宿主拥有；保留 |
| `nomifun-ai-agent/src/runtime_catalog.rs`、App `runtime_engines.rs` | 真实源码引擎注册、精确 build 选择和派发；注册在宿主装配后关闭，没有安装型引擎上传/热注册入口；保留 |
| AgentPreset 的 `runtime_engine`、Engine SDK、官方 Nomi/Coding 注册 | 已有产品选择/组装功能，不是热加载占位；保留 |
| `model_middleware.rs`（合同、Agent adapter、Nomi） | 已消费的 `before_model`，不是模型 Provider；保留 |
| 普通 Role/Provider、Node runtime、Plugin Runtime | 其他插件能力及 JS 执行所需；“Runtime”或“Provider”同名不能作为删除依据 |
| Service 流式前置、Rust 原生插件后端 | 上一收敛提交已删除，本轮不重复删除或恢复 |

原报告中的远程模型功能卡、模型 Role/受管连接/流式接线方案、JS Runtime 完整替换路线及其人周估计已移除。历史架构迁移事实不等于这些取消项仍有待办。

## 五、Agent 页面与应用 Shell 的区别

```text
Nomifun 应用（Shell 管理导航与全局布局）
├─ 导航栏 / 会话列表 / 全局入口
├─ Agent 会话工作区 ← 插件 Agent 页面替换这里
│  ├─ 历史展示、业务表单、结果布局
│  └─ 经授权调用宿主读取、发送、取消
└─ 插件管理 / 模型设置 / 其他系统页面
```

| 用户诉求 | 应归属哪里 |
|---|---|
| 把某 Agent 的聊天区改成客服工单或研究报告工作区 | Agent 页面 |
| 调整该会话内的输入框、消息布局和结果展示 | Agent 页面，具体宿主操作须有现成 API |
| 改整个应用侧栏、首页、全局路由和多工作区组织 | Shell |
| 改 Agent 推理/工具调度逻辑 | 执行层能力或 hooks，不是 UI 替换 |

二者会在视觉布局上相似，但所有权不同。Agent 页面不能通过访问父窗口或直接操作宿主状态扩张为 Shell；Shell 是应用外壳，不是 PowerShell/Bash。筛选结论是仅保留页面内业务输入和结果展示，不开发 Shell、全局插件路由/命令系统或新的局部插槽框架。现有页面直接开放给普通用户选择，不默认加载第三方页面。

## 六、Skill 与装配改造的收益

Skill 链没有闭合时，注册/发现并不保证 Agent 真能读取正文和资源，用户会遇到“已安装但不生效”。现行只读包模式已有真实装载/命令消费；收敛后发现和执行共用验证路径，命令发现不解码无关图片，但仍校验整个制品完整性。可执行模式不属于这次闭环。

Agent 宿主依赖现在收集后单次绑定到最终执行作用域；保留身份、冲突、取消、未知效果保护，删去隐藏消费者二次绑定。通用编译器负责引用/结构/顺序，Nomi 消费者负责具体 hook 合同。这是减少中间状态，不是放宽安全校验。

## 七、下一步与发布边界

- 此前收敛代码的 213 项测试证据见[发布台账 §2.6](2026-09-15-plugin-release-readiness.zh.md)；本轮页面开放的定向测试、聚合检查与 Windows desktop check 单独记录于 §2.7，不以旧证据代替。
- 发布剩余项是 Windows x64/macOS arm64 正式制品、安装后业务、适用签名与最小实模验收；不因取消两个扩展方向而省略。
- 筛选后唯一 hooks/UI 清单见[剩余计划 §0.1](2026-09-15-agent-plugin-remaining-work-and-decisions.zh.md)：已有页面开放、工具前后 hooks、页面业务输入和结果展示。保留能力完成开发后必须普通用户可发现、可选择、实际使用，不能只交付隐藏接口；未实现的新能力不提前纳入 release 声明。
- 不追加兼容层、模型插件、热加载引擎、Rust 插件或全局 Shell 项目。后续实施以最近一张有真实消费者的功能卡为单位，不按历史全量路线重开工程。
