# 项目结构

[English](project-structure.md)

本文用于定位源码职责。具体契约见[架构总览](../architecture/overview.zh.md)；
workspace 成员以 [Cargo.toml](../../Cargo.toml) 和 [package.json](../../package.json) 为准。

| 路径 | 职责 |
| --- | --- |
| [apps/desktop](../../apps/desktop) | Tauri 桌面壳、原生集成、内嵌后端及打包配置。 |
| [apps/web](../../apps/web) | 独立 `nomifun-web` 宿主，同端口提供 API、WebSocket 和 SPA。 |
| [crates/backend](../../crates/backend) | 应用及功能 crate；详见[后端职责](../architecture/backend-crates.zh.md)。 |
| [crates/agent](../../crates/agent) | 提供商无关类型、提供商客户端、配置及浏览器/电脑自动化基础实现。 |
| [crates/shared](../../crates/shared) | 确实跨 backend/agent 使用的共享工具。 |
| [ui/src/common](../../ui/src/common) | 跨宿主 API client、adapter、配置及共享类型。 |
| [ui/src/platform](../../ui/src/platform) | 宿主 bridge 基础层。 |
| [ui/src/renderer](../../ui/src/renderer) | React 页面、组件、hooks、服务和样式；详见[前端架构](../architecture/frontend.zh.md)。 |
| [scripts](../../scripts) | 本地检查、开发启动及发布工具；`bun run help` 列出入口。 |
| [packaging/linux](../../packaging/linux) | Linux 服务与部署文件。 |
| [docs](../README.zh.md) | 维护中的用户、运维、贡献者与架构文档。 |

两个应用宿主都在进程内链接 `nomifun-app`。其独立 `nomicore` binary
用于诊断、stdio MCP bridge 和 Remote 操作；应用宿主不会 spawn 它。

后端访问 agent 类型通常通过 `nomifun-ai-agent` 及其
`nomi_config` / `nomi_types` 再导出。bridge 例外需记录并用 feature gate，
不要随意添加直接 `nomi-*` 依赖。共享进程所有权属于
`nomi-process-runtime`，详见[进程执行契约](../architecture/backend-crates.zh.md)。

## 构建输出

| 命令 | 输出 |
| --- | --- |
| `bun run build:ui` | `ui/dist/` |
| `cargo build -p nomifun-web` | `target/<profile>/nomifun-web` |
| `cargo build -p nomifun-app --bin nomicore` | `target/<profile>/nomicore` |
| `bun run build` | `target/` 下对应平台的桌面包 |
| `docker compose build` | 本地 Web 宿主镜像 |

构建产物、本地数据集和依赖目录不提交。
详见[构建与打包](building-and-packaging.zh.md)。