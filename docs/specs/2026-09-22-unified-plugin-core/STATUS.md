# Unified Plugin Core 实施台账

> 权威合同：同目录 [`README.zh.md`](README.zh.md)。本文件只记录当前源码事实、验证证据与外部阻塞，不形成第二套产品设计。

## 状态

- Goal：`active`。Unified Plugin Core 源码 clean cut 已完成；macOS arm64 App/DMG 原生包与启动证据已通过，Plugin 产品 UI 因目标 Mac 锁屏仍待验收；Windows 候选安装闭环也独立未完成，因此不得标记 `complete`。
- 开发分支：`rf/agent-capability-platform-v2`。Unified Plugin Core 的两个提交已快进进入此分支，随后正常合并远端在此期间新增的 Agent、Knowledge、会话切换、Canvas 与 UI 提交。后续只在本开发分支施工。
- 起始工作树：只有本规格目录为未跟踪内容；未发现或覆盖用户无关代码改动。
- 最终边界：没有双架构、feature flag、旧 decoder、旧 API alias 或可运行兼容层。Plugin 子系统 clean-start，不转换 N1/M1 Plugin 行，也不触碰非 Plugin 用户数据。

## 最终 inventory

### 保留并收敛

- 内容寻址 Artifact Store 的目录/ZIP/bytes 共用扫描、NFC/Windows 路径碰撞、防 symlink/special-file/traversal、配额、取消、原子 staging 与篡改复核。
- 单独 Service process 的 NDJSON、队列上限、取消、超时、崩溃隔离和进程树回收；现在每个含 Service 的 Plugin 最多一个进程，UI-only 从不取得 Node lease。
- sandbox iframe、MessageChannel、调用去重、Active Artifact fence、SQLite authorizer、Credential Store 与启动时唯一 JS Runtime authority。
- Chat 模型选择/生成、统一 Agent Module/Action 消费时机、Desktop command/event 消费者及其他领域仍在使用的全局 Agent 合同。

### 重写并接线

- 唯一 `nomifun.plugin/v1` Manifest、`PluginArtifact`、本地 `plugin_id`、Action + Binding、内联 JSON Schema、统一 SDK/Bridge/DTO。
- 七表 Repository、`install_artifact`、单一 mutation journal、generation DataRoot、SQLite/KV/Files/Cache、Preview、JS migration、Previous、恢复/删除、Package/Backup。
- Chat Draft 与目录/ZIP/Backup Import 均冻结为标准文件树并进入同一 Artifact 校验、staging、runtime validation、事务提交和 activation 链路。
- Agent adapter 将 `agent.tool`、`agent.context`、`agent.before_model`、`agent.before_tool` 接到真实 Engine 消费阶段；Desktop adapter 接通 command/event/automation；Plugin Core 不依赖 Role/Provider/Consumer 图。
- 单一 `/api/plugin-drafts` 与 `/api/plugins` Router；前端只导出一个 `pluginPlatform`，Library/Creator/Preview/Import/Detail/Run/Config 共用同一产品流。Desktop WebUI 可读，本地变更严格由 Desktop shell 放行。

### 已物理删除

- N1 application 聚合、Project/Mount/Candidate/Test/Apply/AutoApply、Shared Extension Host、`nomifun-js-host`、`nomifun-js-kernel-adapter`、N1 DB/DTO/Router/CLI/合同/gate/测试。
- M1 Product/Project/ReadyRelease/Publish/AutoPublish/ActiveEpoch/PointerRevision/ServiceTest/CatalogPublication、旧 Runtime DB/DTO/Router/UI/合同/gate/测试。
- 可选 JS Runtime 持久选择与热切换、Candidate Test Host、旧 preview `Map` 假存储、Share Bundle/Prebuilt Artifact Import/Whole-App Backup 平行状态机。
- 两套 TS 类型/Bridge/locale/UI 路由、旧 MiniApp/N1/M1 当前规格、过期 review、候选脚本、fixtures 和生成物。

## DB 与 clean-start

主数据库只创建七张 Plugin 表：

`plugins`, `plugin_artifacts`, `plugin_drafts`, `plugin_credential_bindings`, `plugin_grants`, `plugin_library_state`, `plugin_mutations`。

- 新安装直接使用 canonical baseline。
- 仅 checksum `25497335d0bd8ce542d6422ba07ecf7c0ce7186032f1fbe5e3ad408b0aeaf166d8b887c3919ae27102bb91f130a6bd3c` 的精确退休 baseline 可执行一次 Plugin-only clean-start；未知、局部或手改 lineage 一律拒绝。
- clean-start 只 drop 已证明属于退休 Plugin/Runtime 的表和一个退休 Agent UI Plugin 索引，随后重建七表并刷新 schema metadata；测试证明 users、preferences 与 Agent 行不变。
- journal 同一行保存旧 Active/Previous Artifact/DataRoot、Config、Credential 引用和 Grant 快照。提交后激活失败或崩溃恢复会回到完整旧状态，不产生混合指针；快照不含 Credential 明文。
- Surface/Preview session、PID、runtime generation、Catalog index 与测试结果均不持久化；启动只清理精确 `preview-<uuidv7>` orphan。

## 受保护的非 Plugin 同名合同

- `channel_plugins` 属于 Channel 连接器域，继续有真实生产消费者，不在 Plugin clean-start 范围。
- `apps/desktop/src/native_api_plugins.rs` 是 Tauri native plugin 包装，不是 Unified Plugin Core。
- Agent 合同独立 schema 中的 `plugin_packages/plugin_mounts/plugin_configs/plugin_states` 是既有全局 Agent Module/Capability Store 合同；Rust 产品类型和 wire source 已收敛为 `AgentModuleId` / `agent_module`，Unified Plugin Core 不读写或依赖这些表。按权威规格“不得误删其他核心领域仍在使用的全局合同”保留；主数据库的退休 N1/M1 `plugin_mounts` 已由精确 clean-start 删除。

## 验收证据

### 产品闭环

- Chat UI-only：生成 → Preview → Save → Open；DB/KV/Files 在 Surface 关闭重开后仍存在，runtime observation 始终 `stopped`，Node 数为零。
- Chat headless：生成含 Service 的 `agent.tool + desktop.command` Action，经本机代码确认后进入 live Binding registry，continuous Service 为 `running`，真实 Desktop command 调用成功。
- mixed：UI Action 与 Service 共用同一 generation DataRoot；headless/mixed 生命周期停用、移入回收站、永久删除会撤销 Binding、Surface 和 Service。
- Agent：真实 Engine surface 发现、冻结、选择并调用 `agent.tool/context/before_model/before_tool`；Artifact 漂移、停用和取消均 fail closed。
- Directory/ZIP/Chat bytes 得到同一 Artifact digest；Backup 恢复复用同一 journal/generation/install 链，Package 无用户数据，Backup 无 Credential 明文并要求显式 rebind。
- Preview 使用正式 SDK/Bridge/Service/storage adapters 的临时 DataRoot；同一 Draft reload 保留临时数据，保存/Discard/启动清理均不会合并到生产。
- 同 dataVersion 只切代码并复用 DataRoot；dataVersion migration 同时覆盖 SQLite/Files，失败保持旧 Artifact/DataRoot；Previous code/data 回退与数据损失确认已覆盖。
- 权限扩张、secret slot 与本机 Service 信任由 Router 和 Core 双层确认；不变权限保留显式撤销，不重复授权；配置 schema、Credential 引用和 UI network CSP 均强制执行。

### 已通过命令（2026-09-23，Windows x64）

- `cargo fmt --all -- --check`
- `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- write`，随后 `check`
- `cargo test -p nomifun-agent-contracts --lib`：65/65
- `cargo test -p nomifun-api-types --lib`：516/516
- `cargo test -p nomifun-db -- --test-threads=1`：完整 crate（lib 307/307，所有 integration targets 通过）
- `cargo test -p nomifun-plugin-platform --tests -- --test-threads=1`：57/57
- `cargo test -p nomifun-app --lib -- --test-threads=1`：446/446
- `cargo test -p nomifun-app --test plugin_e2e -- --test-threads=1`：8/8
- `cargo test -p nomifun-app --features "browser-use computer-use" --test official_preset_catalog_integrity -- --test-threads=1`：3/3
- Agent Control Plane 46/46、Kernel 55/55、Runtime 50/50、AI Agent 321/321 + consumer 20/20、Engine Core 17/17、JS Runtime 1/1。
- `cargo test -p nomifun-desktop -- --test-threads=1`：144 passed，3 个需要已安装 Chrome/公网的环境测试按声明 ignored。
- `cargo check --workspace --all-targets`
- `bun run test:plugin-sdk`：6/6
- `bun run test:ui`：3561/3561
- `bun run check`：包含 typecheck、`check:desktop-ui-boundary`（1908 renderer sources，最低 880x600）、i18n/theme/icon/dead-css、installer、process/browser/UARC/Unified Plugin/Agent vocabulary 边界，全部通过。
- `bun run build:ui`：production build 通过；仅保留既有 chunk-size/dynamic-import 提示。
- 独立生产 WebUI host 在隔离 `build.noindex/unified-plugin-webui-smoke/data` 上启动：`cargo build -p nomifun-web --features static-webui` 通过，实际 `GET /health` 与 `/` 均为 200，页面含应用根节点；未登录 `GET /api/plugins` 为预期 403。只停止了本次启动的进程，未改动现有用户协议注册。
- `$env:CARGO_BUILD_BUILD_DIR='C:\Users\rika0\AppData\Local\Temp\nfb-unified'; bun run build:win x64`：合并前首次 `rust-lld` 进程以 Windows `0xc0000409` 瞬时退出；资源检查正常，原命令重试成功并完成 NSIS。该历史包为 60,659,091 bytes，SHA-256 `885ddf2617bd29196c22f2c3381efa71f47354bd8c570cebdbea57f136b5dd8b`；下方平台门禁记录合并后的新包。
- `git diff --check` 与 staged/unstaged 范围检查通过。

## 最终平台门禁

- 合入远端新增提交后，合同生成器 write/check、App all-target 编译、`bun run check`（1920 renderer sources）、UI 全量 3582/3582、Plugin HTTP E2E 8/8、Plugin Platform 全套、App lib 457/457、带 Browser/Computer feature 的官方 Preset 3/3、DB schema/index/reset 11/11 已复核。Canvas 六种节点和 Knowledge 旧绑定默认只读的两个远端过期断言已修正。
- 合并提交 `a4a2e25ddfb66e07ef598c9e5da3bc320b52066d` 的 Windows x64 release/NSIS 在当前分支上重建成功：安装器 60,970,183 bytes，SHA-256 `a96c0845bfdd297247bdc8e843407713f4a5cd42f80bc4e6d3b370c42975135d`；本地测试包未签名。候选脚本在此 clean HEAD 通过 native host、source checkpoint、work-root 与安装器 Artifact 检查，然后因已有用户级 `HKCU\Software\Classes\nomifun` 协议注册安全停止；该键仍指向另一开发树的 debug Desktop。安装、启动、WebView2/backend、进程树与卸载检查未执行，不能记为通过。
- 候选 harness 自测通过；Authenticode admission 单测通过。本地 `build:win` 明确关闭签名，最终安装器的 `NotSigned` 状态符合本地测试包预期，不是签名发布证据。
- 完整候选冒烟需要用户明确允许临时处理现有协议注册，或在无既有 NomiFun 注册的干净 Windows 账户/主机运行。结构化结果输出到命令 stdout；隔离运行目录限定在 `build.noindex`，不纳入 Git。
- 上一轮主机是 Windows，当时无法生成权威规格要求的 macOS 目标机证据。旧架构的历史 macOS 文档已明确标为不可用于本次验收。
- macOS 目标机接续操作已写入同目录 [`PROMPT-START.zh.md`](PROMPT-START.zh.md)，供另一台 Apple Silicon 机器在同一 `rf/agent-capability-platform-v2` 分支执行。
- Windows 候选环境保护仍是独立未完成门禁；macOS 包/启动证据见下方，Plugin 产品 UI 仍待目标机解锁。Goal 保持 `active`，没有已知 Plugin 源码或测试失败。

## macOS arm64 目标机接续（2026-09-23，进行中）

- 本机 `uname -s` 为 `Darwin`、`uname -m` 为 `arm64`；Node 24.12.0、Rust 1.96.0、Bun 1.3.14、Xcode 26.5、macOS 26.5 SDK 和 `aarch64-apple-darwin` target 可用。`bun install --frozen-lockfile` 成功且未改依赖。
- 从 `origin/rf/agent-capability-platform-v2` 正常快进到 `e4cf0f86facb6a9d4ac366ad9a411f102b6e249b`。本机预存的 `scripts/run-dev.mjs` 与 `scripts/run-dev.test.mjs` 未提交改动未被覆盖，也不纳入本次修复。
- 隔离根目录：`/Users/muri/.codex/validation/unified-plugin-core-macos-20260923.Mj7t4D`；其 `data/` 与 `work/` 专用于本轮，`logs/` 保存各命令输出和退出码。既有 NomiFun/Plugin/Agent 用户数据未被删除或覆盖。
- `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`、`bun run check`（含桌面 UI 最低 880×600 和 Unified Plugin 边界）、`bun run test:plugin-sdk` 均退出 0；对应日志分别为 `logs/contract.log`、`logs/check.log`、`logs/plugin-sdk.log`。
- 首轮 `cargo test -p nomifun-plugin-platform --tests -- --test-threads=1` 在 macOS 专属 Service 进程环境断言失败：Node 即使通过 `env -i` 启动，也会注入 `__CF_USER_TEXT_ENCODING`。只在 macOS 测试断言中允许这个精确键；生产 `env_clear`、Host 环境白名单与敏感变量检查保持不变。定向 `service_process` 7/7 和 Plugin Platform 全套 57/57 重跑已通过；详见 `logs/service-process-retest.log`、`logs/plugin-platform-tests-retest.log`。
- 原生预检脚本检查 Git 跟踪工作树必须干净。为避让上述用户改动，另建同一 HEAD 的 detached 验收检出 `source/`，只在其中安装锁定依赖与生成未跟踪构建物，不改跟踪源码；没有创建或切换开发分支。
- `cargo test -p nomifun-app --test plugin_e2e -- --test-threads=1` 为 8/8；`cargo test -p nomifun-app --lib engine_plugin_bindings -- --test-threads=1` 为 4/4，覆盖真实 Agent tool/context/hook 消费、取消与更新失效。Agent 选择、Knowledge 面板、会话跳转和 Plugin 导航的定向 UI 为 61/61，Plugin Bridge/边界 10/10。日志为 `logs/plugin-e2e.log`、`logs/agent-plugin-binding-targeted.log`、`logs/agent-knowledge-plugin-ui.log` 和 `logs/plugin-ui-targeted.log`。
- 在原工作树执行 `bun run build:mac arm` 时 App/DMG 已生成，但最终 release-lock 签发因两个预存用户改动使 Git 跟踪工作树不洁而拒绝，命令退出 3。该次不能作为通过的包证据，日志 `logs/build-mac-arm.log`；旧同名 dist 产物在构建前保存到 `work/previous-dist/`，首次产物保存到 `work/first-build/`。
- 同一提交的干净验收检出先执行 `bun install --frozen-lockfile` 和 `bun run check`（均退出 0），再执行 `bun run build:mac arm`（退出 0）。构建源提交 `d312868712910b9b6ed6b632a534a4e71d7ea56e`，目标 `aarch64-apple-darwin`，App 的 Mach-O 仅 `arm64`；完整构建日志 `logs/clean-build-mac-arm.log`。
- App：`source/target/aarch64-apple-darwin/release/bundle/macos/NomiFun.app`，其 `Contents/MacOS/nomifun-desktop` SHA-256 为 `484d6488c7572312e4e670fdac2735c16651af2cf4949aa9f4fd27cdf2bcd91a`。DMG：`source/dist/desktop/NomiFun_0.7.6_aarch64.dmg`，SHA-256 为 `19eb17be9e3dff35dad1bcc1f8dccabff9d9de53f05571e4628865d7322fa9be`。真实 lock：`source/dist/desktop/NomiFun_0.7.6_aarch64.release-lock.json`，SHA-256 为 `c6cf0bdfa9ae831eb678f3e4112ea49aec7c680856efa55fb6b0ee43e1dac03b`；其中 host/package 哈希与独立计算一致。
- 本轮为**未签名工程验证包**：App/CEF nested 仅 ad hoc 签名；未运行 Developer ID、DMG 签名、公证或 Gatekeeper 发布评估，不得写成签名发布结果。
- `bun scripts/validation/check-macos-arm64-native.mjs --release-lock <上述绝对 lock> --run-startup --host-binary <同一 App 可执行文件> --report <logs/macos-native-preflight.json> --log <logs/clean-build-mac-arm.log>` 退出 0。结构化结果 26 项、无失败/阻塞：原生 arm64、lock 来源、App/CEF helper、DMG verify/mount 同一性、空/新建 DataRoot 两次 HTTP 200 与进程树清理均通过；DMG 签名/公证和生命周期检查明确为 `not_required`。该脚本是 Host/package preflight，不是 Plugin 产品验收。
- 从上述 `.app` 在本轮 `data/` 与 `work/app/` 实际启动 PID 15062，`data/port.json` 指向 `127.0.0.1:55102`；外部 `GET /health` 为 200，未获 WebView local-trust 的 `GET /api/plugins` 为预期 403。隔离 DB 恰有七张 Unified Plugin 表且初始 Plugin 行数为零；初始进程树 Node 数为零。日志为 `logs/native-app.log`、`logs/process-tree-initial.json`、`logs/native-health-body.json`。UI 工具报告 macOS 当前锁屏；已请求目标机手动解锁，Library/Creator/Import/Run/Config 和 DataRoot 操作仍待可见 UI 验收，不能记为通过。
- 同一 DMG 已只读挂载到 `work/mounted-dmg/`，其中 App 可执行文件 SHA-256 与 lock 完全一致，架构为 arm64。前一 App 在 PID 15062 收到终止信号并清理退出后，直接从挂载 App 启动 PID 15462；新 `port.json` 为 `127.0.0.1:55264`，`GET /health` 为 200，初始进程树仅 App 加三个 CEF Helper，Node 数为零。证据：`logs/native-dmg-app.log`、`logs/native-dmg-health-body.json`、`logs/process-tree-dmg-initial.json`。这证明 DMG 内 App 可以原生启动，但无 Plugin 安装/调用的 UI 验收仍受锁屏阻塞。
