# IDMM canonical AgentSession 重建方案（2026-09-20）

## 决策

恢复 IDMM 产品能力，但不恢复 2026-09-18 前依赖旧 Conversation store、terminal
probe 和第二套 supervisor registry 的实现。新实现遵守以下边界：

1. AgentSession 是唯一会话、Turn、消息和取消权威。
2. IDMM 是领域监督器，不是 Runtime、Agent、Session aggregate 或全局备用模型。
3. 规则始终先执行；旁路模型只能解决规则无法安全解决的决定。
4. 所有自动写入经 `NomiCoreSessionOwner::send_session_message_idempotent`，来源标记
   为 `idmm`，不得绕过 canonical receipt。
5. 当前数据库采用单 canonical baseline；本轮不修改 baseline checksum、不要求
   用户重置真实数据。配置使用已有 `client_preferences` 的受限命名空间。
6. IDMM 不注册 Agent Capability。AgentPreset Revision 可保存
   `runtime_policy.idmm`，作为新 Session 的一次性默认值。

## Agent 默认值与 Session 覆盖

- Agent 工作台在独立的“运行策略”页签编辑 IDMM，不把它混入模块与 Action；
- 非默认策略参与 AgentPreset Revision digest，因此修改会创建新的不可变 Revision；
  缺失字段与默认关闭使用同一规范编码，旧 Revision 的 digest 保持有效；
- 创建 AgentSession 时从其精确 Revision 初始化 IDMM；创建请求重放不会覆盖已有状态；
- Guid 在选择个人 Agent 后显示该 Revision 的默认值，只有用户改动时才提交会话覆盖；
- 会话胶囊修改当前 Session，不反写 Agent；后续 Agent Revision 不追改已有 Session；
- fork 继承父 Session 已经生效的 IDMM 配置。

## 观察面

- `agent_session_heads.active_turn_id`：精确活动回合；
- `agent_turns`：终态、错误与原始来源；
- `agent_messages`：有界消息窗口；
- Nomi Runtime `AgentEngineEvent`：最后进度时间和 model/tool 阶段。

运行事件只更新进度水位，不另建事件总线。工具阶段静默只记 `safety_halt`，禁止
自动取消；model/other 阶段超过阈值才允许取消并恢复。

## 决策面

### 确定性规则

- provider fault：仅识别限流、网络、超时、网关、临时不可用、空流等信号；
- option decision：必须同时存在问题提示和至少两个结构化选项；
- safe option：排除取消项、权限/凭据/付款项和破坏性操作；
- open question：规则档等待人工，旁路档才升级。

### 旁路模型

输入是 JSON 数据信封，包含问题、安全标记后的选项和配置范围内的有界上下文；
常见 API key、token、密码和私钥会先做尽力脱敏。system contract 明确把上下文视为
不可信数据。输出只接受：

- `select_option` + 有效安全下标；
- `answer_text` + 最长 2000 字节的非破坏性短答；
- `halt`。

解析失败、模型失败或越界输出只产生失败/停止记录，不回退成任意文本发送。

## 故障转移

`agent.model_failover` 在创建 Agent binding 时解析。显式主模型保持第一候选，队列
按保存顺序去重后成为 failovers，并受 `max_switches` 限制。结果冻结在 preset
revision / Session binding；修改全局配置只影响后续新会话。

## 持久化与清理

键：`agent_session.idmm.<uuidv7>`。

值包含 schema version、Session identity、revision、完整配置、最后检查时间和最多
50 条介入。并发在进程内按 Session 串行；应用仍由单 server lock 保证唯一写者。

- Session 删除：同步删除 IDMM 键；
- Provider 删除：若命中旁路模型，清空引用、revision +1，并从旁路档降到规则档；
- Worker 发现 Session 已不存在：回收孤立键。

## 已验证范围

- detector/policy/service 本地替身测试；
- authenticated HTTP GET/PUT、默认值、严格旁路校验；
- global failover queue → immutable Chat route；
- AgentPreset runtime policy → 新 Session 一次性初始化与重放不覆盖；
- Agent 工作台、Guid 继承/覆盖、会话头部入口；
- Rust 编译、前端类型、i18n、桌面 UI 边界。

真实供应商/真实旁路模型调用不属于本轮自动测试证据，不能据此宣称真实模型验收。
