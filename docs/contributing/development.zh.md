# 开发

[English](development.md)

安装或使用产品请看[安装](../getting-started/installation.zh.md)与
[Web 服务部署](../guides/web-server-deployment.zh.md)。

## 前置工具

- 支持 edition 2024、resolver 3 的稳定 Rust。
- Bun >= 1.3.13、Git、CMake >= 3.16 和原生 C/C++ 工具链。
- Windows：MSVC 构建工具与 WebView2。macOS：Xcode Command Line Tools。
  Linux：遵循维护中的[系统依赖清单](../getting-started/installation.zh.md#前置条件)。

首次原生构建可能下载 ONNX Runtime 等第三方构建依赖。
离线重建需保留依赖缓存。平台构建细节见[构建与打包](building-and-packaging.zh.md)。

## 安装依赖

```bash
git clone <repo-url> nomifun-desktop
cd nomifun-desktop
bun install
```

Rust workspace 以 [Cargo.toml](../../Cargo.toml) 为准，Bun workspace
包含 `ui/`。根脚本在 [package.json](../../package.json)，运行
`bun run help` 查看入口。

## 开发循环

| 命令 | 行为 |
| --- | --- |
| `bun run dev:ui` | 5173 端口的 Vite，不启动后端，API 调用可能失败。 |
| `bun run dev:web` | Vite 加 8787 端口的 API-only `nomifun-web`，关闭鉴权供本地迭代。 |
| `bun run serve:web` | 构建 SPA，并从源码启动默认鉴权的 Web 宿主。 |
| `bun run dev` | Tauri、Vite 与桌面本地信任策略下的内嵌后端。 |

关闭鉴权的开发宿主仅绑定 loopback 或隔离的可信网络。
桌面请求使用每次启动生成的本地信任密钥，后端在宿主进程内运行。

开发命令选择 dev 构建通道。桌面 dev 启动器使用按 schema 指纹区分的同级目录
`NomiFun-dev-schema-<fingerprint>`，保留以往目录。
`serve:web` 与发布构建使用 stable 通道。
隔离实例时显式设置绝对路径 `NOMIFUN_DATA_DIR`，按字面值使用。
两个后端不能同时占用同一数据目录。

数据/工作目录优先级、通道默认值和日志见[配置参考](../reference/configuration.zh.md)。
POSIX shell 示例：

```bash
NOMIFUN_DATA_DIR=/tmp/nomifun-dev bun run serve:web
```

## 验证命令

选择直接覆盖改动文件与行为的检查。纯文档改动不需要编译；
跨模块改动和发布使用更广的检查。

| 改动 | 检查 |
| --- | --- |
| 文档 | `bun run check:docs` 和 `git diff --check` |
| 脚本注册/帮助 | `bun run help --check` |
| Renderer 类型 | `bun run typecheck` |
| Renderer/UI 规则 | `bun run check:desktop-ui-boundary` 及定向 UI 测试 |
| Locale JSON/类型 | `bun run gen:i18n`，再运行 `bun run check:i18n` |
| 样式/预设 | `bun run check:theme`、`bun run check:dead-css`；见[样式规范](styling.zh.md) |
| Rust 编译路径 | `cargo check -p <crate>` |
| Rust 行为 | `bun run test:crate <crate> [cargo 参数]` 或定向 `cargo test -p <crate>` |
| Canonical Agent 存储/内容 | 先读 [Agent Session](../architecture/agent-session.zh.md)，运行 `bun run check:agent-session-boundary` 及受影响的历史/reset/consumer 测试 |
| Schema/ID/reset/restore | 遵循[数据规范](data-and-identifier-standards.zh.md)，运行受影响的 repository/schema 测试 |
| 广泛回归 | `bun run check`、`cargo check --workspace` 及相关 Rust/UI 测试 |

`bun run test:core`、`bun run test:desktop` 和 `bun run test`
提供更广的 Rust 回归层级。`bun run test:ui` 运行 UI 测试；局部修复使用文件筛选。
记录实际执行的命令，并说明不能运行的平台检查。

## 诊断

`nomicore` 可独立于应用宿主运行：

```bash
cargo run -p nomifun-app --bin nomicore -- doctor
cargo run -p nomifun-app --bin nomicore -- tool-doctor
```

`doctor` 报告终端 Agent CLI 检测结果。`tool-doctor` 读取组件/许可证目录；
`--verify` 才会明确解包并探测工具。
Remote、stdio bridge、backup/restore 命令详见
[CLI 配置](../reference/configuration.zh.md#nomicore后端参数)与可执行文件 `--help`。

## 继续阅读

- [项目结构](project-structure.zh.md)
- [后端职责](../architecture/backend-crates.zh.md)
- [前端](../architecture/frontend.zh.md)
- [构建与打包](building-and-packaging.zh.md)
- [疑难排查](../reference/troubleshooting.zh.md)
## 开发运行目录与 Windows 构建恢复

桌面开发版按数据 profile 的 SHA-256 在用户 OS 运行目录下选择
`NomiFun-dev-runtime/<hash>/application` 与 `/device`，不与安装版共用应用维护或设备锁。
运行目录位于数据集之外，搬迁/重置不会复制或封存活动 IPC。正式应用和独立安装维护工具仍用已发布目录，开发版不会清理正式版的旧登录登记。

Windows 开发启动器补齐 MSVC/Windows SDK 环境；未显式设置 `RUST_MIN_STACK` 时仅对子进程使用 512 MiB 编译栈，保留调用者覆盖，不修改终端全局环境或其他平台。首次统一后端构建可能持续数分钟。

若 Vite 已启动，但 `opusic-sys` 报 SILK SSE4.1/AVX2 函数未声明，应检查该库生成的 CMake 缓存。
`HAVE_SMMINTRIN_H` 的旧负探测与 AVX2 开启会形成不一致。停止失败构建后可仅清理该依赖的生成输出：

```sh
cargo clean -p opusic-sys
bun run dev
```

无需关闭编解码 SIMD 或修改 Cargo 注册表源码。排查时保留原 CMake 探测日志。
实测范围见[开发启动验证记录](../validation/native-keyboard.zh.md#开发启动实测回归2026-11-08)。
