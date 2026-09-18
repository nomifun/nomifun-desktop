# UARC-054 canonical Agent Store 基线压缩实施记录

> 实现提交：`5d250dc3ab097b29898f5800ec299af4d10a2f7a`
> 平台：Windows shared/runtime/UI 已验证；macOS shared source pending，真机闭环仍由 Wave 8 完成

## 交付

- 将 112 份历史迁移压缩为单一 `001_canonical_baseline.sql`。新安装直接创建 generation-5 Agent Store，
  不再先创建再删除 Conversation、旧 Runtime、IDMM、fresh-v4 或旧 Capability 投影。
- 为已安装数据库保留一次精确、带认证的旧 lineage cutover：只接受受支持的历史迁移集合，迁移非 Agent
  配置后切换到 canonical baseline；未知、篡改或半迁移 lineage fail closed。
- 删除 `nomifun-v4-root`、旧 root coordinator、displaced migration、published-migration 测试矩阵、开发环境
  seed/import helper、Conversation artifact 类型以及只服务历史 schema 的 repository/model/test。
- Agent reset、备份/恢复、data-root relocation 与启动探针统一使用 canonical Store。启动探针以只读连接验证
  schema、外键和 owned indexes；Agent-only reset 保留 Provider、模型、Cron 等非 Agent 配置。
- 统一 Runtime 补齐隐藏 `before_model` / `before_tool` middleware、真实 Node cancellation、Action/Event/Effect
  因果链、不可变 Turn authority 与 canonical turn cancellation。业务拒绝作为模型可见结果，合同/技术失败终止
  Turn，隐藏 observation 只持久化 digest。
- 新增 exact-current-plan `ToolSearch`：只发现非 deferred 且已激活的定义，每次最多返回 5 个当前 ToolPlan
  候选；越界、重复或无效输出原子失败，不产生部分激活。
- 修正消息分页游标、Resource binding identity 与 visible action 的 `turn_started` 因果关系；Browser/Process
  仍使用统一 Resource/Capability 模型，没有恢复旧 Browser Session 入口或 WKWebView。

## 删除与保留边界

- 实现提交涉及 261 个文件，新增 10,693 行、删除 35,446 行；历史 schema、fresh-v4 root 与 legacy seed
  helper 仅保留在 Git 历史中。
- UARC scanner 中 multi-Runtime、old loop、compatibility、legacy Capability、legacy Agent Store、private
  transcript、Agent IDMM、Browser dedicated entry 与 fresh-v4 parallel root/schema 均为 0 个 production match。
- 保留非 Agent 配置表、canonical Agent-only reset、产品域自有数据与已安装数据库的精确一次性 cutover；不提供
  永久兼容 reader/writer、旧 Runtime 入口或旧 Store 投影。
- Windows Browser 保持 WebView2；macOS Browser 保持现有独立 CEF child NSView 底座。本任务只验证共享源码，
  未替代 Wave 8 的 Mac 真机、TCC、CEF、打包与签名证据。

## 验证

```text
cargo test -p nomifun-agent-contracts --all-targets -- --test-threads=1
  100 passed
cargo test -p nomifun-agent-runtime --lib -- --test-threads=1
  43 passed
cargo test -p nomifun-agent-session --all-targets -- --test-threads=1
  35 passed
cargo test -p nomifun-db --lib -- --test-threads=1
  305 passed
cargo test -p nomifun-app --lib --all-features -- --test-threads=1
  486 passed
cargo test -p nomifun-app --test nomi_core_route_gap --all-features -- --test-threads=1
  29 passed
cargo test -p nomifun-app --test startup_smoke -- --test-threads=1
  4 passed
cargo test -p nomifun-app --test plugin_product_discovery -- --test-threads=1
  8 passed
cargo test -p nomifun-app --test nomi_core_live_provider_smoke before_tool_smoke::tests -- --test-threads=1
  8 passed
bun test --cwd ui
  3538 passed
```

DB all-target integration suites（含 fresh initialization、旧 lineage cutover、reset、backup、schema）、Cron、
Requirement 与 Workshop 定向套件均通过。contract generator write/check、55 份 JSON contract closure、
`bun run check`、production UI build、880×600 desktop UI boundary、UARC/Browser/Process scanners、rustfmt 与
`git diff --check` 均通过。

Windows Credential Manager 隔离的 StepFun Coding Plan `step-3.7-flash` 真实模型 smoke 通过：
`before_tool.publish_select`、`before_tool.allow`、`before_tool.deny`、`before_tool.continuation` 四阶段均为
`pass`。证据来自 canonical AgentSession、official Runtime、真实 middleware、native write owner 与持久化
Action/Event/Effect；凭据未进入 argv、Cargo/build-script 环境、仓库或日志。

## 平台状态

- Windows：verified；canonical baseline、旧库 cutover、reset/backup/startup、统一 Runtime middleware、
  commercial-model evidence、完整 UI 与静态门禁均已验证。
- macOS：pending；共享 schema/runtime source 可交接，但本 Windows 主机没有声称 CEF child NSView、TCC、
  原生交互、arm64 app、DMG 或签名结构完成。
