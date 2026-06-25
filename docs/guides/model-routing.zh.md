# 模型故障转移队列

当前模型路由设置背后的实现是**模型故障转移队列**，不是多凭据轮询池。

它允许 Nomi 引擎会话在检测到提供商故障时，按你配置的顺序尝试备用模型。
ACP/CLI 智能体不包含在这个功能里，因为它们的提供商调用发生在外部运行时内部。

## 它做什么

- 全局默认队列存储在 `agent.model_failover`。
- 单个会话可以通过 `extra.model_failover` 覆盖。
- 只作用于 Nomi 引擎会话。
- 可被 IDMM 的故障监视流程使用。
- 不会在 API Key 之间分摊负载。
- 不会让所有 CLI 智能体共享同一个模型池。

## 什么时候使用

当一个 Nomi 引擎会话需要在临时提供商/模型故障后自动换用备用模型时，使用模型
故障转移。

常见队列：

```text
主模型 -> 便宜备用模型 -> 更强备用模型 -> 人工检查
```

这个队列解决的是可靠性，不是额度聚合。如果所有配置的提供商都不可用，或者
prompt / tool 状态本身无效，故障转移也无法让这一轮成功。

## 与 IDMM 的关系

IDMM 有独立的故障监视和决策停滞监视。模型故障转移属于故障侧：当某个提供商
故障被判定为可恢复，且该会话启用了故障转移时，IDMM 可以让会话运行时按配置
队列重试。

AutoWork 位于更上一层：它负责让标签队列继续认领和推进需求，而 IDMM / 模型
故障转移负责尽量让每个已认领的回合活下来。

## 真相来源

- `crates/backend/nomifun-conversation/src/model_failover.rs`
- `crates/backend/nomifun-conversation/src/failover_seam.rs`
- `crates/backend/nomifun-app/src/router/model_failover.rs`
- `crates/backend/nomifun-idmm/src/policy.rs`

本页旧版本曾把该功能描述成多凭据 round-robin 路由。那不是当前实现，不应作为
运维或用户指南使用。
