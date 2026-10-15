# 常见问题

[English](faq.md)

## 哪些内容在本地运行？

桌面应用及内嵌后端在本机运行，独立 `nomifun-web` 在部署服务器执行工具和 Agent。
配置的模型服务、MCP、渠道、更新及可选下载可以发起网络请求。
所选提供商或网关拥有自己的数据与计费政策。

NomiFun 是自托管软件。可选的独立运营模型网关属于提供商选择，
不代表托管的 NomiFun 工作区或中央账号，详见[模型路由](../guides/model-routing.zh.md)。

## 桌面应用需要登录吗？

桌面 WebView 通过每次启动的信任密钥直接打开，不展示登录页。
其他浏览器需要鉴权，独立 `nomifun-web` 也默认需要登录。
详见[连接与设备](../guides/webui-remote-access.zh.md)。

## 数据在哪里？

Stable 默认目录：Windows 为 `%LOCALAPPDATA%\NomiFun`，
macOS 为 `~/Library/Application Support/NomiFun`，
Linux 为 `$XDG_DATA_HOME/NomiFun` 或 `~/.local/share/NomiFun`。
显式 `NOMIFUN_DATA_DIR` / `--data-dir` 按字面值使用。
Docker 使用挂载数据卷，开发通道使用独立数据根。

同一数据根只能由一个运行中的后端占用。工作目录优先级、存储迁移和备份行为见
[配置](configuration.zh.md)与[数据存储](../architecture/data-and-storage.zh.md)。
被替代的 Agent 数据集不导入当前 canonical lineage。

## 支持什么 Agent 和提供商？

会话使用内置 `nomi` runtime。模型提供商和密钥在应用中配置，密钥静态加密。
第三方 Agent CLI 是独立程序，可在[内置终端](../guides/terminal.zh.md)
使用自己的鉴权及审批流程。`nomicore doctor` 显示检测到的终端 CLI。

## 应用和技能由谁运行？

应用托管在受管应用目录中持有前台进程。技能向 Agent 上下文提供指令，
不持有应用进程。详见[应用托管](../guides/app-hosting.zh.md)。

发布构建内嵌 OfficeCLI，Node/Bun 准备需要明确选择并验证，
后端启动不下载它们。应用托管使用系统或用户配置的 Node。
详见[外部工具](../architecture/external-tools.zh.md)。

## 可以从其他电脑或手机使用吗？

桌面浏览器可访问独立 Web 宿主或 Desktop WebUI。
共享渲染器支持至少 880x600 的桌面级视口；
手机使用独立 NomiFun Mobile 客户端。
配对和可选 relay 传输见[连接与设备](../guides/webui-remote-access.zh.md)。

## 如何安装和更新？

使用[安装指南](../getting-started/installation.zh.md)、
[项目 README](../../README.zh-CN.md) 中的当前下载入口，或
[Web 部署指南](../guides/web-server-deployment.zh.md)。
桌面使用签名更新包，见[桌面更新](../guides/desktop-app.zh.md#更新)。
官方 Web 镜像为 `nomifun/nomifun-web`，需要可复现时固定版本或 digest。

服务端由 Rust binary 和构建后的 SPA 目录组成，也可选择内嵌 SPA 的构建。
桌面分发使用平台包，详见[构建与打包](../contributing/building-and-packaging.zh.md)。

## 忘记管理员密码怎么办？

Desktop WebUI 使用应用内可信的
**连接与设备 → 其他电脑浏览器**账号/密码入口。
Loopback 浏览器请求不会获得桌面本地信任。
修改启动环境变量 `NOMIFUN_ADMIN_PASSWORD` 不会替换已有账号。

独立宿主应保留数据目录，参阅[鉴权排查](troubleshooting.zh.md)。
不要通过清空密码 hash 或暴露无鉴权监听器来恢复。

## 名称和许可证是什么？

产品名使用 **NomiFun**；小写 `nomifun` 保留在 crate、环境变量等技术标识中。
项目采用 [Apache-2.0](../../LICENSE)，再分发时保留许可证与 notices。

## 其他参考

- [配置](configuration.zh.md)
- [API 概览](api-overview.zh.md)
- [疑难排查](troubleshooting.zh.md)