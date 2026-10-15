# NomiFun 文档

[English](README.md) · [项目 README](../README.zh-CN.md) · [发布镜像](https://www.nomifun.com/zh/docs/)

这里维护当前 Desktop 产品及桌面级 WebUI 的文档。按下面的任务进入；产品指南保存在本仓库，NomiFun Portal 发布镜像。实现事实以当前源码和稳定的架构文档为准。

## 开始使用

- [项目介绍](getting-started/introduction.zh.md)
- [安装](getting-started/installation.zh.md)
- [快速开始](getting-started/quick-start.zh.md)
- [产品生态](architecture/product-ecosystem.zh.md)

## 使用与部署

- [桌面应用](guides/desktop-app.zh.md)
- [模型与路由](guides/model-routing.zh.md)
- [Agent 与预设](guides/presets.zh.md)
- [创意工坊](guides/creative-studio.zh.md)
- [应用托管](guides/app-hosting.zh.md)
- [伙伴](guides/companions.zh.md)
- [小智机器人](guides/xiaozhi-robot.zh.md)
- [MCP 与技能](guides/mcp-and-skills.zh.md)
- [Computer / Browser Use](guides/computer-browser-use.zh.md)
- [终端](guides/terminal.zh.md)
- [SSH 会话](guides/ssh-sessions.zh.md)
- [需求 / AutoWork](guides/autowork-requirements.zh.md)
- [智能决策（IDMM）](guides/intelligent-decision.zh.md)
- [定时任务](guides/scheduled-tasks.zh.md)
- [渠道](guides/channels.zh.md)
- [WebUI 远程访问](guides/webui-remote-access.zh.md)
- [远程能力 API](guides/remote-capability-api.zh.md)
- [Web 服务器部署](guides/web-server-deployment.zh.md)

## 理解架构

- [系统总览](architecture/overview.zh.md)
- [后端 crate](architecture/backend-crates.zh.md)
- [前端](architecture/frontend.zh.md)
- [Agent 引擎](architecture/agent-engine.zh.md)
- [Canonical Agent Session](architecture/agent-session.zh.md)
- [Agent 执行](architecture/agent-execution.zh.md)
- [模型供应商](architecture/model-providers.zh.md)
- [语音](architecture/voice.zh.md)
- [通信](architecture/communication.zh.md)
- [渠道准入](architecture/channels.zh.md)
- [浏览器平台](architecture/browser-platform.zh.md)
- [应用托管](architecture/app-hosting.zh.md)
- [数据与存储](architecture/data-and-storage.zh.md)
- [ID 系统](architecture/id-system.zh.md)
- [外部工具](architecture/external-tools.zh.md)
- [外部知识 MCP](architecture/external-knowledge-mcp.zh.md)

## 开发、验收与发布

- [贡献约定与流程](../CONTRIBUTING.zh-CN.md)
- [仓库指令](../AGENTS.md)
- [项目结构](contributing/project-structure.zh.md)
- [开发与检查](contributing/development.zh.md)
- [样式合同](contributing/styling.zh.md)
- [数据与标识符规范](contributing/data-and-identifier-standards.zh.md)
- [构建与打包](contributing/building-and-packaging.zh.md)
- [发版流程](../RELEASING.zh-CN.md)
- [签名](../apps/desktop/signing/README.md)
- [更新器](../apps/desktop/updater/README.zh-CN.md)
- [Linux 打包](../packaging/linux/README.md)
- [浏览器验收](validation/browser-platform.zh.md)
- [连接链路验收](validation/connectivity-stack.md)
- [截图清单](images/SCREENSHOTS.md)
- [外部 Agent 接入 Skill](skills/drive-nomifun/SKILL.md)

## 查询细节

- [配置](reference/configuration.zh.md)
- [API 概览](reference/api-overview.zh.md)
- [故障排除](reference/troubleshooting.zh.md)
- [FAQ](reference/faq.zh.md)
- [变更记录](../CHANGELOG.md)
- [安全](../SECURITY.md)
- [行为准则](../CODE_OF_CONDUCT.md)

## 维护规则

- 用户步骤放在 `guides/`，当前合同放在 `architecture/`，配置/API 查询放在 `reference/`，贡献者流程放在 `contributing/`，可复用的验收方法与明确的覆盖缺口放在 `validation/`。优先更新已有归属页。
- 有中英文兄弟文件时同步修改。仅有中文的技术页在英文索引明确标注，不链接不存在的译文。
- 实现细节链接源码入口，不重复 crate 数量、路由清单或生成 schema。路由以 [Router.tsx](../ui/src/renderer/components/layout/Router.tsx) 为准，命令以 [package.json](../package.json) 和 `bun run help` 为准。
- 共享 Desktop renderer 支持 Tauri 与桌面浏览器，最低 **880×600**。独立 Mobile 客户端及其 wire 合同不决定 renderer 布局。
- 退休设计时，将仍有效的决定合入归属页，修复消费者，同时删除旧计划、报告与无用素材。运行时嵌入指南、随产品分发的 Skill、测试夹具、vendor 文档和许可证通知各有消费者，不能只因扩展名是 Markdown 就删除。
- 历史计划、交接单、状态快照与逐次验收证据由 **Git 历史**保留。使用 `git log --all -- <path>` 和 `git show <commit>:<path>` 追溯，不新建容易被误读为现役要求的归档树。仍需执行的问题放回现役归属页或验收页，并准确保留证据边界。
- 运行 `bun run check:docs` 检查本地链接、图片与 Markdown 锚点，再运行 `git diff --check`。纯文档编辑不需要全量构建；代码或合同改动按 [AGENTS.md](../AGENTS.md) 运行相关检查。
