# 简介

**NomiFun** 是一个面向 AI agent 工作流的本地优先工作台。它把多种
CLI agent、内置 Nomi 引擎、模型提供商、MCP 服务、技能、终端、计划任务
和远程 WebUI 收拢到同一个 Rust + Tauri monorepo 中。

> 想立刻开始？请先读 [安装](installation.zh.md)，再读
> [快速上手](quick-start.zh.md)。完整文档地图见 [`../README.zh.md`](../README.zh.md)。

![NomiFun 引导页](../images/gs-01-introduction-hero.png)

## 它解决什么问题

真实的 AI 工作流经常被拆散在多个地方：一个终端跑 Claude Code，一个终端跑
Codex，浏览器里开着自托管页面，旁边还有单独的 MCP 服务和项目脚本。
NomiFun 的目标不是再做一个聊天框，而是把这些运行时接到同一个工作区：

- **一个会话入口，多种 agent。** 会话可以选择内置 Nomi、Claude Code、
  Codex、Gemini、Qwen、OpenCode、CodeBuddy 等后端。
- **一个模型目录，多处复用。** 在 `/models` 配好 Anthropic、OpenAI、
  Bedrock、Vertex 或兼容接口后，支持这些模型的 agent 和助手可以复用。
- **一个工作区，不只是消息流。** 会话有工作目录、文件树、预览面板和后端
  管理的 PTY 终端。
- **后端驱动的自动化。** 计划任务、AutoWork、IDMM、WebUI 远程访问、
  MCP 暴露和频道能力都由 Rust 后端持久化管理。
- **桌面与 Web 共用后端。** Tauri 桌面端和 `nomifun-web` 自托管服务使用
  同一套 `nomifun-app` 后端与同一份 React SPA。

NomiFun 更适合已经在用 agent 做真实工作的用户。它要求你理解 API key、
本地数据目录、CLI agent 安装和自托管边界；它不是零配置的 SaaS 聊天产品。

## 两种运行方式

| 模式 | 二进制 | 鉴权模型 | 典型用途 |
| --- | --- | --- | --- |
| 桌面应用 | `nomifun-desktop` | 桌面外壳使用本地信任 token 访问嵌入式后端；远程浏览器仍需登录 | 单机工作站、日常开发 |
| Web 服务 | `nomifun-web` | 默认开启登录；首次访问创建管理员 | LAN/VPN/VPS 自托管 |

桌面模式会在进程内启动 `nomifun-app`，监听一个随机 localhost 端口，并通过
每次启动生成的本地信任 token 让 WebView 免登录访问。WebUI 远程访问打开后，
额外的 LAN 监听器仍然要求远程浏览器登录。

`nomifun-web` 则在一个端口上同时提供 SPA 与 API，默认端口是 `8787`。Docker
和 systemd 部署都走这条路径。

## 当前功能地图

- **会话与工作区**：`/guid` 创建会话，`/conversation/:id` 运行会话。
- **模型配置**：`/models` 管理提供商、模型、凭据和全局故障转移队列。
- **助手与技能**：`/assistants` 管理助手；其中 `tab=skills` 管理技能。
- **MCP**：`/mcp` 管理 MCP server、连接测试、OAuth 和 agent 配置同步。
- **开放能力**：`/open-capabilities` 管理 WebUI 远程访问、MCP/API 暴露等外部入口。
- **桌面伙伴**：`/nomi` 管理伙伴、远程频道绑定和 companion 相关设置。
- **终端**：`/terminal-new` 创建、`/terminal/:id` 运行后端 PTY。
- **计划任务**：`/scheduled` 管理 cron 触发的会话任务。
- **AutoWork**：`/requirements` 管理需求看板和自动执行。

更多内部结构见 [`../architecture/`](../architecture/)，用户指南见
[`../guides/`](../guides/)。

## 项目状态

NomiFun 仍在活跃开发中，但已经不是旧的 Electron 多仓迁移状态。当前仓库是
Rust workspace + Tauri desktop + Web host 的单仓结构。顶层
[`../../STATUS.md`](../../STATUS.md) 记录当前状态；历史设计稿与审计记录不在
仓库中保留，需要时请查阅 git 历史。

## 接下来

- [安装](installation.zh.md)
- [快速上手](quick-start.zh.md)
- [开发环境](../contributing/development.zh.md)
- [Web 服务部署](../guides/web-server-deployment.zh.md)
