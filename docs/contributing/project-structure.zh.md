# 项目结构

这是 **NomiFun** 的当前仓库地图。后端分层详见
[`../architecture/backend-crates.md`](../architecture/backend-crates.md)，运行时总览见
[`../architecture/overview.md`](../architecture/overview.md)。

## 顶层布局

```text
nomifun-desktop/
├── apps/
│   ├── web/                      nomifun-web：独立 Web/API host
│   └── desktop/                  nomifun-desktop：Tauri 桌面壳
├── crates/
│   ├── agent/                    7 个 nomi-* crate，提供商、配置及桌面自动化基础实现
│   ├── backend/                  54 个 nomifun-* crate，HTTP/WS 后端
│   └── shared/                   4 个跨层共享 crate
├── ui/                           React SPA，Vite + UnoCSS，唯一 Bun workspace
├── docs/                         当前文档、专项历史决策、外部 skill 与图片资源
├── packaging/linux/              nomifun-web systemd unit 与部署说明
├── Cargo.toml                    Rust workspace
├── package.json                  Bun/Tauri/Cargo 入口脚本
├── Dockerfile                    nomifun-web 容器镜像
├── docker-compose.yml            Web host compose 示例
├── Caddyfile                     可选 TLS reverse proxy
├── README.md
└── STATUS.md
```

Cargo workspace 当前成员：

```toml
members = ["crates/agent/*", "crates/backend/*", "crates/shared/*", "apps/web", "apps/desktop"]
```

## App Host

| 路径 | Binary | 职责 |
| --- | --- | --- |
| [`apps/web`](../../apps/web) | `nomifun-web` | 独立 Web/API host。进程内启动 `nomifun-app`，同端口提供 API、WebSocket 和 SPA。默认开启登录；`--insecure-no-auth` 仅供可信本地开发。 |
| [`apps/desktop`](../../apps/desktop) | `nomifun-desktop` | Tauri 桌面壳。进程内启动同一个后端，选择空闲 localhost 端口，注入 `window.__backendPort` 与 `window.__nomiLocalTrust`，WebView 通过本地信任 token 访问后端。 |

两个 host 都直接链接 `nomifun-app`。`nomicore` 仍作为 `nomifun-app` 的独立
binary 存在，用于诊断、stdio MCP bridge、canonical Remote 调用和无头场景；桌面/Web host
不会 spawn 它。

## Crate 分组

| 目录 | 前缀 | 数量 | 职责 |
| --- | --- | --- | --- |
| [`crates/agent/`](../../crates/agent) | `nomi-*` | 7 | 提供商、配置及桌面自动化基础实现，尽量保持独立。 |
| [`crates/backend/`](../../crates/backend) | `nomifun-*` | 54 | HTTP/WS 后端、数据层、认证、Agent Store、Unified Plugin、cron、knowledge、terminal、companion、public gateway 等。 |
| [`crates/shared/`](../../crates/shared) | mixed | 4 | 真正跨 agent/backend 使用的共享工具。 |

模型/工具循环位于 `nomifun-agent-runtime`。`nomifun-engine-core` 将已准入的
工具计划投影到 `nomifun-agent-kernel`，应用宿主提供具体能力 adapter；
`nomifun-mcp` 持有 MCP 连接与调用。

## Agent 层接缝

后端代码需要 agent 类型或执行能力时，默认应通过
[`crates/backend/nomifun-ai-agent`](../../crates/backend/nomifun-ai-agent)。
它再导出常用的 `nomi_config` 和 `nomi_types`。

当前 workspace 仍存在少数 feature-gated 直接依赖例外：`nomifun-app` 与
`nomifun-gateway` 为 browser/computer-use bridge 工具直接触达部分 `nomi-*`
crate。新增后端 crate 时不要随手添加 `nomi-*` 依赖；若确实是 bridge/facade
例外，需要用 feature gate 并在架构文档中说明。

## 关键目录

| 路径 | 内容 |
| --- | --- |
| [`ui/src/common/`](../../ui/src/common) | 跨 host 的 API client、类型、adapter、工具函数。 |
| [`ui/src/platform/`](../../ui/src/platform) | host bridge：storage、logger、theme、平台能力。 |
| [`ui/src/renderer/`](../../ui/src/renderer) | 页面、组件、hooks、服务、样式和 renderer 入口。 |
| [`docs/getting-started/`](../getting-started) | 安装与首次运行。 |
| [`docs/guides/`](../guides) | 用户任务指南。 |
| [`docs/architecture/`](../architecture) | 当前架构说明。 |
| [`docs/reference/`](../reference) | 配置、API、FAQ、troubleshooting。 |

## 制品位置

| 构建 | 输出 |
| --- | --- |
| `bun run build:ui` | `ui/dist/` |
| `cargo build -p nomifun-web` | `target/<profile>/nomifun-web` |
| `cargo build -p nomifun-app --bin nomicore` | `target/<profile>/nomicore` |
| `bun run build` | `target/<profile>/bundle/<format>/...` |
| `docker compose build` | 本地镜像 `nomifun-web:local` |

更多打包细节见 [`building-and-packaging.zh.md`](building-and-packaging.zh.md)。
