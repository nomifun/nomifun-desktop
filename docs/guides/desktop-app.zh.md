# 桌面应用

NomiFun 桌面应用使用 Tauri 外壳，在本机启动共享 Rust 后端并加载工作台。桌面窗口通过每次启动生成的本地信任凭据免登录使用；其他设备通过[连接与设备](webui-remote-access.zh.md)接入。

![NomiFun 桌面主窗口](../images/desktop-01-main-window.png)

## 安装与启动

从源码安装的环境与命令统一见[安装](../getting-started/installation.zh.md)，发布构建、代码签名与更新产物见[构建与打包](../contributing/building-and-packaging.zh.md)。

运行已打包版本需要对应系统的 WebView：Windows 使用 WebView2，macOS 使用系统 WKWebView，Linux 使用 WebKitGTK。当前 macOS 打包最低系统版本为 14.0，以 `apps/desktop/tauri.conf.json` 为准。

## 窗口和系统功能

- 主窗口默认 1280×832，最小 880×600；WebUI 也面向桌面级浏览器。
- Windows、Linux 使用应用内标题栏控件，macOS 保留原生红绿灯按钮。
- Windows、Linux 再次启动会聚焦已有实例。
- 在设置中可控制登录时启动。系统通知需操作系统允许；应用关闭后本机自动任务不会继续运行。
- 应用注册 `nomifun://` 协议；具体链接需由当前产品支持，注册协议本身不代表任意路径都可打开。

## 数据位置

| 系统 | 默认 stable 数据根 |
| --- | --- |
| Windows | `%LOCALAPPDATA%\NomiFun` |
| macOS | `~/Library/Application Support/NomiFun` |
| Linux | `$XDG_DATA_HOME/NomiFun`，通常为 `~/.local/share/NomiFun` |

开发脚本使用独立的 `NomiFun-dev` 目录。启动前设置 `NOMIFUN_DATA_DIR` 可指定数据根，所有宿主按原值使用，不追加目录后缀。同一目录只允许一个后端持有 `server.lock`。

数据根包含数据库、日志、伙伴、知识库及运行组件缓存；外部工作区可能位于目录之外。存储、备份及不兼容数据的处理以[数据与存储](../architecture/data-and-storage.zh.md)为准，不能把复制目录当作所有历史版本都可恢复的保证。退出所有使用该目录的实例后，再做文件级备份或维护。

## 本地信任与远程访问

桌面 WebView 自动携带本机信任凭据。其他进程只知道后端端口并不能因此获得权限；运行桌面应用的系统账户仍拥有 Agent 使用的文件和命令权限。

从其他电脑连接时，在[连接与设备](webui-remote-access.zh.md)启用相应入口；需要独立服务器时使用[Web 服务部署](web-server-deployment.zh.md)。

## 更新

在应用设置中检查当前发布渠道的可用更新。下载、校验及安装结果以应用显示为准；公开发布的更新需要匹配平台的签名产物。开发者维护更新端点、密钥与产物的说明位于[更新器](../../apps/desktop/updater/README.md)。

## 故障排查

- **窗口空白**：检查系统 WebView，Windows 上确认 WebView2 可用，Linux 上确认 WebKitGTK 运行时已安装。
- **数据目录被占用**：退出使用同一目录的桌面或 Web 服务；需要并行运行时为另一个实例指定独立数据根。
- **本机命令或组件不可用**：检查命令路径、缓存执行权限和实际诊断。终端、MCP 与应用托管各自使用支持的运行时，不会因打开普通诊断自动安装所有工具。组件边界见[外部工具](../architecture/external-tools.zh.md)。
- **Linux 源码编译缺库**：使用[安装](../getting-started/installation.zh.md)列出的开发包；运行时包不能替代链接所需的开发文件。
