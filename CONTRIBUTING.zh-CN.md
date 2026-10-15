# 参与 NomiFun 开发

[English](CONTRIBUTING.md)

先阅读 [AGENTS.md](AGENTS.md)，了解仓库边界和验证规则。
使用贡献者配置的 Git 身份，保留无关改动。

## 从这里开始

| 任务 | 维护中的说明 |
| --- | --- |
| 搭建开发环境 | [开发](docs/contributing/development.zh.md) |
| 找到所属模块 | [项目结构](docs/contributing/project-structure.zh.md) |
| 修改渲染器 | [前端架构](docs/architecture/frontend.zh.md)和[样式规范](docs/contributing/styling.zh.md) |
| 修改 schema、ID、reset 或 restore | [数据与标识符规范](docs/contributing/data-and-identifier-standards.zh.md) |
| 修改 Agent 存储或内容 | [Canonical Agent Session](docs/architecture/agent-session.zh.md) |
| 构建或打包 | [构建与打包](docs/contributing/building-and-packaging.zh.md) |
| 准备发布 | [发布手册](RELEASING.zh-CN.md) |

## 贡献约定

- 每个 PR 聚焦一个问题。产品方向、架构、数据库、安全边界和新增依赖应尽早讨论。
- 遵循现有模块职责和实现方式。后端功能逻辑放入所属 crate；渲染器的宿主操作通过 platform/adapter 层。
- 支持 880x600 及以上的桌面窗口和桌面级 WebUI 浏览器。窄面板使用容器感知布局，不增加移动端渲染器布局。
- 行为变化时同步英文、简体中文翻译及成对文档。
- 当前说明集中在维护中的指南、参考和架构目录。删除被替代的计划及重复说明，由 Git 历史保留，不另建开发指令归档。
- 不提交凭据、隐私数据、构建产物或无再分发权的素材。贡献使用仓库的 Apache-2.0 许可证。
- 遵守[行为准则](CODE_OF_CONDUCT.md)；通过 [SECURITY.md](SECURITY.md) 私下报告漏洞。
- 使用 AI 辅助的贡献也需要承担相同的审查与正确性责任。

## 验证与提交

选择直接覆盖改动的最小检查，命令地图见[开发](docs/contributing/development.zh.md#验证命令)。
文档改动运行 `bun run check:docs` 和 `git diff --check`，不需要完整构建。
未运行的检查应记录原因。

提交标题应具体，推荐 Conventional Commits，例如
`fix(conversation): preserve selected model after retry`。送审前：

- 说明问题、改后行为和实质风险。
- 列出实际执行的检查；可见 UI 改动附截图。
- 用户或运维人员需要发布说明时，同步文档、两个语言和 `CHANGELOG.md`。
- 检查 diff 与暂存文件，排除无关改动和隐私数据。
- 准确关联 issue：完全解决使用 `Fixes`，部分进展使用 `Refs`。