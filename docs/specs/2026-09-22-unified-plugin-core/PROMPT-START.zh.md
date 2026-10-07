# Unified Plugin Core：macOS 工作交接 Prompt

> 本文件供另一台 Apple Silicon Mac 上的新 Codex 会话直接使用。产品与施工权威只有同目录 `README.zh.md`；实施事实和未完成门禁只有同目录 `STATUS.md`。本交接不创建第二套状态台账。

将以下内容作为新会话的第一条消息：

---

你在一台 Apple Silicon macOS 目标机上接续 NomiFun Unified Plugin Core 的最终验收与必要修复。所有开发只在 `rf/agent-capability-platform-v2` 分支进行；不要创建或切到其他开发分支，不改写历史，不 force-push。

先检查 `uname -s`、`uname -m`、Git 工作树和远端状态。若本机已有未提交用户改动，保留并避让；从 `origin/rf/agent-capability-platform-v2` 正常快进到最新提交。完整读取仓库根 `AGENTS.md`、`docs/specs/2026-09-22-unified-plugin-core/README.zh.md` 和同目录 `STATUS.md`。若历史 Plugin 文档与规格冲突，以 Unified Plugin Core 为准。不要重做已经完成的 N1/M1 设计或恢复任何旧兼容入口。

Windows 端已实现并合入同一开发分支：唯一 `nomifun.plugin/v1` Package、本地 Plugin 身份、Action + Binding、单一 JS Runtime/SDK/Bridge/API/UI、generation DataRoot、SQLite/KV/Files/Cache、Preview、JS migration、单一 mutation journal、Chat/外部目录/ZIP/Backup 共用安装链、Agent/Desktop 消费者，以及旧 N1/M1 生产路径物理删除。此前 Windows Rust、Plugin HTTP E2E、UI、合同与边界检查已通过；本机的 macOS 原生运行和 DMG 证据仍缺失。Windows NSIS 候选安装冒烟因另一个开发树占用当前用户的 `nomifun://` 注册而停在安装前，不能视为通过。

请直接执行 macOS 剩余工作：

1. 在隔离的新 `NOMIFUN_DATA_DIR` 和工作目录下运行，绝不删除或覆盖本机现有 NomiFun/Plugin/Agent 用户数据。确认 Node、Rust、Bun、Xcode/macOS SDK 与 Apple Silicon target 可用；依赖安装使用锁文件。
2. 跑 `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`、`bun run check`、`bun run test:plugin-sdk`、`cargo test -p nomifun-plugin-platform --tests -- --test-threads=1`、`cargo test -p nomifun-app --test plugin_e2e -- --test-threads=1`。如有平台问题，定位并修复源码，再运行直接相关的 Rust/UI/边界检查。Renderer 变更后必须跑 `bun run check:desktop-ui-boundary`，不得增加 880px 以下或移动端布局。
3. 用 `bun run build:mac arm` 生成本机 arm64 `.app` 与 `.dmg`；记录完整命令、退出码、目标架构、app/DMG 路径、SHA-256、release-lock 路径及构建日志。默认包是未签名的工程验证包；只有本机确有签名/公证配置且当前 release 要求它们时才运行 `--signed`，不要把未签名结果写成签名发布证据。
4. 用构建产生的真实 release lock 运行 `bun scripts/validation/check-macos-arm64-native.mjs --release-lock <绝对路径>`，并保留结构化结果。按需要加 `--run-startup --host-binary <同一构建的可执行文件>`；先阅读脚本的入参和检查范围，不把 preflight 冒充完整 Plugin 产品验收。
5. 在目标机实际启动这个 `.app`，检查至少 880×600 的 Plugin Library/Creator/Import/Run/Config：UI-only 不起 Node，headless Service 能由真实 Agent Action 和 Desktop command 调用，mixed UI/Service 共享 DataRoot；Preview 使用临时副本；停用/回收/更新撤销旧访问。通过隔离 DataRoot 和实际进程/HTTP/UI 观察记录证据。已提交的 `plugin_e2e`、`migration`、`service_process` 测试可以提供可重复夹具，但仍需要原生 app/DMG 启动证据。
6. 复核从 Windows 合入的 Agent/Knowledge/会话切换代码与 Plugin Binding 共存；若遇到冲突或回归，保持 Agent 真实消费者能力，不恢复 Plugin Product/Role/Provider 图或旧 API。每个修复同步更新测试、调用方与唯一 `STATUS.md`，在 `rf/agent-capability-platform-v2` 正常提交并推送。
7. 终审按规格 §19–20 逐项列出证据。记录通过、失败、未运行与环境条件，提供确切命令和目标机日志位置。Windows 安装器完整冒烟仍是独立未完成门禁；在其与 macOS 证据均通过前，不把总 Goal 标为 complete。

最终向用户报告 macOS 目标机实际结果、commit SHA、未完成项及下一步。遇到真实产品选择或无法安全处理的用户数据时再请求决定；不要因为检查耗时或一次失败提前结束。

---
