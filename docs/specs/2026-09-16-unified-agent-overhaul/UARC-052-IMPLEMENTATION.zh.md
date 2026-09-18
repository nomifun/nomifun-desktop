# UARC-052 单一官方 Agent Runtime 实施记录

> 实现提交：`48fbfb09c`
> 平台：Windows shared/runtime/UI 已验证；macOS shared source pending，真机闭环仍由 Wave 8 完成

## 交付

- 物理删除 `nomi-agent`、`nomi-cli`、旧 Nomi factory/manager、私有 transcript persistence、
  multi-Runtime catalog/registry/selector、旧 restart compatibility 与重复 Runtime 测试。
- `nomifun-coding-engine` 收敛并重命名为 `nomifun-agent-runtime`；公开类型、错误、工具计划与生命周期
  使用统一 Agent Runtime 语义，不再暴露 Coding Runtime 产品身份。
- 产品组合只构造一个 `OfficialRuntimeProvider`、一个固定 `OfficialRuntimeFactory` 和一个
  `OfficialRuntimeHost`。生产 handle 不再有 generic registered/foreign family 分支；诊断 API 收敛为
  singular `/api/agent-runtime`。
- 从旧实现中只提取仍有 owner 的 domain adapter、middleware contract、process registry 与 output
  protocol contract；这些组件由统一 Driver 消费，不形成第二执行循环。
- Session 打开、取消、关闭、history、steering、attachments、skills 与 tool surface 全部经统一 Runtime
  host；删除 `uses_nomi_session`、旧 recovery hook、task-local plugin session wrapper 与旧 Browser factory
  resolver。
- 新增 migration `112_unified_runtime_build_identity.sql`，在不改写已发布 migration 099 checksum 的前提下，
  将安装库的不可变字段约束切换为 `runtime_build_binding`。migration 099 中仅存的两个旧 selector 文本
  由 `UARC-054` baseline squash 物理删除。

## 删除与保留边界

- UARC 扫描中 `runtime.multi_family_literals`、`runtime.old_nomi_loop_paths`、
  `runtime.multi_runtime_infrastructure_paths`、`runtime.compatibility_branches` 与
  `store.private_nomi_transcript` 均为 0。
- `runtime.preset_selector` 仅余 migration 099 的 2 个 immutable lineage 命中；没有生产 Rust/TypeScript
  reachability，也没有为其保留兼容读取或写入。
- Windows Browser 仍使用 WebView2；macOS 保留现有独立 CEF child NSView 底座。本任务没有新增或恢复
  WKWebView。

## 验证

```text
cargo test -p nomifun-agent-runtime -- --test-threads=1
  40 passed
cargo test -p nomifun-ai-agent --all-features --lib -- --test-threads=1
  452 passed, 1 ignored explicit real-Chrome/network case
cargo test -p nomifun-ai-agent --all-features --test plugin_tool_consumer -- --test-threads=1
  23 passed
cargo test -p nomifun-conversation --lib -- --test-threads=1
  335 passed
cargo test -p nomifun-app --all-features --lib -- --test-threads=1
  511 passed
cargo test -p nomifun-app --test nomi_core_route_gap --all-features -- --test-threads=1
  29 passed
cargo test -p nomi-process-runtime --test architecture_contract -- --test-threads=1
  16 passed
```

DB ID schema 20/20、focused Runtime host、focused UI 7/7、typecheck、production UI build、880×600
desktop UI boundary、Browser/Process scanners 及 self-tests、UARC scanner 及 self-test、rustfmt 和
`git diff --check` 均通过。Windows Credential Manager 隔离的 StepFun Coding Plan
`step-3.7-flash` selected-model smoke 通过 canonical Agent Session → unified Runtime → durable projection；
密钥未进入 argv、Cargo/build-script 环境、仓库或日志。

## 平台状态

- Windows：verified；唯一官方执行链、Store/Session 投影、UI 诊断和静态平台边界均通过。
- macOS：pending；本提交只提供 shared-source handoff 输入，未声称 CEF、TCC、原生交互、arm64 app、
  DMG 或签名结构完成。
