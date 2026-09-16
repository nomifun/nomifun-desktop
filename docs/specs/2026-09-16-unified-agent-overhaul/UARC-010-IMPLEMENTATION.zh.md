# UARC-010 Capability Module / Action Grant 合同实施记录

> 开始 barrier：`89a4f820d06aa75c4a248932b5c8f12d79988785`
> 平台：共享合同；Windows 执行工程验证

## 交付

- 新增 `CapabilityAuthoringPolicy`：`direct`、`dependency_only`、`platform_managed`、`internal`。
- authoring policy 与 consumer 一样使用 manifest digest 覆盖的 namespaced declaration；不会进入 host
  surface。未迁移 manifest 使用受限默认：Agent Tool/Context/EventSource 可 direct，Resource Provider、
  middleware、transport、scheduler、background service、UI 等不能成为 direct Agent root。
- `CapabilityKind` 仅保留为 Catalog 展示摘要和少数 typed provider dispatch 标识，不再限制一个 Module
  只能贡献 Action 或 Context。
- Module validator 允许同一 manifest 同时发布多个 Action、多个 Context schema 和多个 Event schema，
  并拒绝重复/空 Action、schema、host port 及非法 direct authoring。
- Catalog entry 冻结 `authoring_policy`；API Types 新增 vNext `AgentCapabilityGrantDto`、Module Action 和
  Module Catalog DTO，Action 集合是必填精确集合。
- Kernel 根据实际 contribution sets 绑定 action handler/context factory；Tool/Context dispatch 不再根据
  `CapabilityKind` 推断。
- 直接选择 action-bearing Module 时，空 Action 集合不再表示“全部”；会返回
  `ActionGrantRequired`。`platform_managed`、`dependency_only`、`internal` 直接选择返回
  `CapabilityNotAuthorable`。
- ResolvedSnapshot 对内置 Module 与 Plugin Product 一致冻结 action descriptors、effect class、resource
  requirements 和 exact action allowlist。Dependency Module 只获得 scoped dependency call 所需的声明
  actions，不能成为直接贡献。
- Kernel authority test 证明一个同时含 2 Actions、2 Context schemas、2 Event schemas 的 Module 只可
  调用授权的 1 个 Action，第二个 Action 在 dispatch 前被拒绝。

## 删除与保留

已删除的行为：

- Kernel 中“空 `action_allowlist` 自动展开为全部 declared actions”；
- 用 `CapabilityKind::Tool/ContextContributor` 作为通用 action/context dispatch authority；
- Agent 直接选择 Resource Provider、middleware、transport、scheduler、background service 等平台形式；
- 非 Plugin Product Snapshot 不能冻结 actions/action grants 的旧限制。

明确保留：

- `CapabilityKind` 作为展示摘要，以及 Resource Provider 的 typed factory dispatch 标识；
- v1 `CapabilitySelection` 外层字段名，直到 `UARC-014` 同步切换 AgentPreset vNext；它在本任务后已经
  使用 exact Module/Action 语义，不再提供隐式全授权；
- 还未迁移的空 contribution catalog placeholder；它们不产生 Action authority，由 `UARC-014/053`
  随 136-ID catalog 一起删除。

反向搜索结果：Kernel 内已不存在将空 allowlist 展开为全部 actions 的分支，也不存在用
`manifest.kind == Tool/ContextContributor` 作为通用 dispatch 准入的分支。

## 验证

通过：

```text
cargo test -p nomifun-agent-contracts --lib
  103 passed

cargo test -p nomifun-agent-kernel --lib
  60 passed

cargo test -p nomifun-api-types --lib
  533 passed

cargo test -p nomifun-agent-session --lib
  25 passed

cargo test -p nomifun-engine-core --lib
  14 passed

cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check
  passed

cargo fmt -p nomifun-agent-contracts -p nomifun-agent-kernel -p nomifun-api-types -- --check
  passed

bun scripts/check-uarc-boundary.mjs --self-test
  passed; no legacy count growth
```

额外运行 `cargo test -p nomifun-agent-control-plane --lib`：50 项中 48 项通过。两项失败精确对应
已删除的旧假设：

1. `context_order_round_trips...` 只设置 `CapabilityKind::ContextContributor`，未发布 Context schema；
2. `middleware_order_changes...` 把 dependency-only TurnMiddleware 当 direct Agent selection。

这两项由依赖本合同的 `UARC-014` 更新为 Module contribution / derived middleware 语义。没有加入
kind fallback 或 authoring compatibility 来伪造绿色；Wave 1 milestone full gate 仍须在 UARC-014
barrier 后全部通过。
