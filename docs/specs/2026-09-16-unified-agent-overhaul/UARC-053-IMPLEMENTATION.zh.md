# UARC-053 退役 Capability、Store、IDMM 与 Browser 入口实施记录

> 实现提交：`55ea23d1035ee87bc3702bc23cae396e74511ff4`
> 平台：Windows shared/runtime/UI 已验证；macOS shared source pending，真机闭环仍由 Wave 8 完成

## 交付

- 用 `UARC-053-RETIRED-CAPABILITY-IDS.json` 冻结并逐个扫描 135 个旧 operation-level Capability ID；
  正式目录只保留 Module/Action 合同。`workspace.artifacts` 的同名复用被显式记录为新的多 Action Module，
  不作为旧 ID 兼容读取。
- 删除 capability retirement/legacy contribution 合同、兼容映射、旧 Capability 投影与 Browser 专用入口；
  catalog、seed、first-party contribution 与 generated envelope 全部改用 Module/Agent 术语。
- 物理删除旧 `ConversationService`、Conversation/Message repository/model、私有 stream/transcript、旧 Session
  adapter、旧 Runtime façade、Agent IDMM API/DB repository，以及只服务这些路径的测试和脚本。
- `nomifun-conversation` 收敛为 canonical AgentSession 产品 façade：generation-5 `AgentSessionStore` 是唯一事实源，
  只保留 turn delivery、product Agent、Creation ingress、Creative Studio Session 和全局 model failover 配置等
  仍有 owner 的小型 adapter。
- Cron、Channel、Requirement、AgentExecution、Creation、Knowledge、Plugin Surface 与 Workshop 的交叉引用改为
  `agent_sessions` / `agent_messages` 或 typed Session Port；测试夹具同步切换，不再为了测试构造第二套 Store。
- canonical AgentSession 删除 saga 在 Store fence 前证明 Runtime 退出，并在 tombstone 前撤销 Browser、Plugin
  Surface、Knowledge、Cron、Requirement 与 SSH 资源；无调用者的 Conversation delete hooks 被删除。
- 删除 Local Websearch 专用 renderer、旧 product-chain/compaction live smoke、退役 automation-session audit，
  根静态 Gate 改由 UARC scanner 统一阻止回流。设置页、Guid、Companion MCP 与 Provider 使用提示均使用
  canonical Agent/Session 语义。

## 删除与保留边界

- 实现提交涉及 253 个文件，净删除 91,161 行；旧 Runtime/Store/IDMM 的源码与测试只存在于 Git 历史。
- UARC 扫描中 legacy Capability projection/authoring ID、legacy Agent Store、private transcript、Agent IDMM、
  Browser dedicated entry、multi-Runtime/compatibility/old-loop 各组均为 0 个 production match。
- published migration 与 fresh-v4/schema 文本仍由 `UARC-054` 独占：当前剩余 413 个 fresh-v4/schema、2 个
  historical selector 和 AutoWork migration/field 命中不具有生产 reader/writer reachability，本任务未改写既有
  migration checksum。
- Windows Browser 继续使用 WebView2；macOS 继续保留独立 CEF child NSView 底座，没有引入 WKWebView。

## 验证

```text
cargo test -p nomifun-agent-contracts --all-targets -- --test-threads=1
  105 passed
cargo test -p nomifun-agent-execution --all-targets -- --test-threads=1
  103 passed
cargo test -p nomifun-conversation --all-targets -- --test-threads=1
  21 passed
cargo test -p nomifun-requirement --all-targets -- --test-threads=1
  53 passed
cargo test -p nomifun-cron --all-targets -- --test-threads=1
  155 passed
cargo test -p nomifun-companion --all-targets -- --test-threads=1
  271 passed
cargo test -p nomifun-db --lib -- --test-threads=1
  326 passed
cargo test -p nomifun-app --lib --all-features -- --test-threads=1
  507 passed
cargo test -p nomifun-app --test nomi_core_route_gap --all-features -- --test-threads=1
  29 passed
cargo test -p nomifun-channel --all-targets -- --test-threads=1
  451 passed
cargo test -p nomifun-realtime --all-targets -- --test-threads=1
  102 passed
bun test --cwd ui
  3541 passed
```

App all-target compile、contract generator write/check、`bun run check`、production UI build、880×600 desktop
UI boundary、dead CSS/i18n/icon、route inventory、UARC/Browser/Process scanners 与 self-tests、rustfmt 和
`git diff --check` 均通过。Windows Credential Manager 隔离的 StepFun Coding Plan
`step-3.7-flash` selected-model smoke 通过 canonical AgentSession → official Runtime → durable projection；
密钥未进入 argv、Cargo/build-script 环境、仓库或日志。

## 平台状态

- Windows：verified；删除集 production reachability 为 0，replacement UI、Store/Session 删除闭环、商业模型
  与 desktop Gate 已验证。
- macOS：pending；本提交只提供 shared-source handoff 输入，未声称 CEF、TCC、原生交互、arm64 app、
  DMG 或签名结构完成。
