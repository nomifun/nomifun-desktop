# Windows Browser 重构与远程分支整合

## Git 范围

- 分支：`rf/agent-capability-platform-v2`，未合入 `main`。
- 本地 Browser 重构保全提交：`476b6bd0b`。
- 合入远程提交：`a8b1685cf`；相对原本地基线 `439bb385a` 含 48 个提交。
- 使用普通 merge 保留双方历史；无 reset、rebase、force push 或数据库迁移。

## 合并取舍

- 保留原生会话 Browser、独立 `nomi_system_browser`、可选 `nomi_local_websearch`。
- 保留远程多 Engine、插件退休、创作/会话 UI、macOS 退出清理等变更。
- 接入远程 `shutdown_and_wait`，Browser 仅在 Agent 退出得到确认后关闭；失败保持资源所有权以便重试。
- Role defaults 重载必须叠加当前宿主 materialize 的精确 Browser Role，无宿主不暴露假能力；不把内部绑定存为用户设置。
- 原生 Browser 绑定仍以 Nomi 当前执行链为准，不宣称新 Coding Engine 自动支持 Browser。
- 删除 merge 中重新出现的已退休 Fresh-v4 host 和 runtime-release fixture，不恢复旧 Browser Hub。
- 生成合同由 generator 重建，避免手工拼接摘要。

## 本轮 Windows 验证

以下为合并后执行的检查，不代替 macOS 实机验收：

| 检查 | 结果 |
| --- | --- |
| `bun run check` | 通过：typecheck、desktop boundary、i18n、主题/图标/废弃代码等仓库检查 |
| Browser、SystemBrowser、Stop/Sider、Guid、草稿/ChatLayout 定向 UI 测试 | 115 项通过 |
| `cargo check -p nomifun-desktop --bin nomifun-desktop --example browser_workspace_smoke --no-default-features` | 通过 |
| `cargo check -p nomifun-app --features browser-use --example browser_gui_fixture` | 通过；只验证编译，不计作 GUI 运行验收 |
| `cargo test -p nomifun-app --features browser-use --lib nomi_core_role_defaults` | 2 项通过 |
| `cargo test -p nomifun-app --features browser-use --lib browser` | 36 项通过；1 项需要 `NOMIFUN_SEARCH_CHROME` 的真实 Chromium 测试按声明 ignored，未计入通过 |
| `cargo test -p nomifun-app --features browser-use --test browser_workspace --test system_browser` | 3 + 1 项通过 |
| `cargo test -p nomifun-net --lib egress` | 22 项通过 |
| `cargo test -p nomifun-browser-platform --lib` | 50 项通过 |
| live provider runner 默认 / `--browser` 的 `--self-test` | 通过；仅 runner 自检，不是调用真实模型 |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check` | generator write 后检查通过 |

合并时修复了旧测试中的关闭接口、异步 build 和快照新增字段；系统打开 URL 的测试提示同步到当前选中的
conversation Browser / system-browser capability，不恢复旧 `browser navigate` 命令文案。

Rust 构建存在 unused/private-interface 等警告，以及本机缓存的 Opus 调试 PDB 缺失警告；以上列出的命令退出码为 0。
本轮未重新制作最终 Windows 安装包、未重新执行原生 GUI/真实模型/个人 Chrome 授权验收；旧 EXE 和旧 fixture
证据不能代表合并版完成这些验收。macOS/Linux 原生执行未在 Windows 代跑。

## 下一台电脑的入口

- [macOS 点位和验证清单](2026-09-16-browser-workspace-v2-macos-handoff.zh.md)
- [可直接交给 macOS Agent 的工作 prompt](2026-09-16-browser-workspace-v2-macos-work-prompt.zh.md)

Mac 先拉取包含本文的分支最新代码；只传 Git 提交和去敏验证材料，不传个人数据库、Profile 或凭证。
