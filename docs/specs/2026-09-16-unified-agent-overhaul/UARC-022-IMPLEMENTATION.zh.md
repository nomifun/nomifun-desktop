# UARC-022 Skill、MCP、Plugin 与 Connector Module 实施记录

> Wave 1 barrier：`4b65cf019e9e6942c5c3458d6db64d868c241bd2`
> Wave 2 实现提交：`efe80298f`
> 平台：Windows 实现与验证；macOS shared compile 仍待 Mac 主机

## 交付

- Skill 只通过 frozen package/body/resource locks 与受控 command/context consumer 进入 Session；旧
  `skill.catalog/describe/invoke/hooks` 不再作为直接 authoring Capability。
- 每个 MCP remote tool 物化为一个 namespaced exact Action，绑定 server UUIDv7、canonical tool key、schema
  digest、materialization revision 和 exact `mcp_server` resource。模型不能提交 server/tool selector。
- MCP ResourceProvider 只读取 Session 已绑定 server，per-server invoke 只在 matching frozen tool lock 存在时授予；
  broad `mcp.connect/oauth/resource/tool_proxy` 不进入 Snapshot 或 Runtime admission。
- Plugin Tool、Context、Event 与 middleware 从 contribution 描述生成；`CapabilityKind` 仅展示，phase/handler
  由真实 contribution contract 决定。Role/Context/Tool 调用保留独立 host-owned Turn 与 tool operation identity。
- 官方模板从当前 Catalog 冻结完整 exact Action allowlist；依赖 Capability 由 Compiler 形成 dependency-only graph，
  不再要求用户直接选择 Channel/Robot 内部 lifecycle 条目。
- 唯一保留的 MCP client route 是进程签发的固定 Gateway `nomi_delegate`；它没有 server/tool selector，且与
  per-tool owner 互不替代。

## 物理删除

- 删除 `nomi-agent/src/mcp_capability_tools.rs`。
- 删除从未有生产 `activate()` caller 的 `nomi-agent/src/lazy_mcp.rs`、Repository connector、AI factory
  MCP repository/OAuth 注入和 manager cleanup wrapper。
- 删除 generic MCP proxy 注册、allowlist marker expansion、connect/resource selector tools和旧 runtime source lookup。
- 非空 legacy device MCP transport 现在 fail closed；`UARC-042` 必须改接 materialized Robot Actions，不能恢复代理。

## 验证

```text
cargo test -p nomi-agent -- --test-threads=1
  lib 638 passed；全部 integration/doc targets passed
cargo test -p nomifun-ai-agent --lib -- --test-threads=1
  548 passed；新增 retirement focused test 1 passed
cargo test -p nomifun-ai-agent --test plugin_tool_consumer -- --test-threads=1
  41 passed
cargo test -p nomifun-ai-agent --test factory_provider_integration -- --test-threads=1
  5 passed
cargo test -p nomifun-agent-kernel -- --test-threads=1
  61 passed
cargo test -p nomifun-agent-control-plane -- --test-threads=1
  49 passed
cargo test -p nomifun-agent-domain-wave4 -- --test-threads=1
  20 passed
```

`nomifun-mcp` 全 package targets、Plugin Platform 全 targets、Contracts 108、App middleware 7、Plugin graph 1、
MCP/resource focused tests均通过。Contract generator check、default App check、Browser/Computer feature check、UARC
boundary self-test 与 `git diff --check` 通过。

## 平台状态

- Windows：verified。
- macOS：pending；本任务没有 Mac shared compile/connector transport 证据。
