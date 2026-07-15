# 常见问题

针对反复被问到的问题给出直白、简短的回答。如需更深入的解释，请顺着
链接前往对应文档。

## NomiFun 与 nomifun 有什么区别？

**NomiFun** 是这个开源项目与面向用户的产品名：桌面应用、WebUI 界面、
代码库、工作区、GitHub 仓库和品牌都使用这个写法。

在本代码库里，小写形式 `nomifun` 仅作为字面意义上的技术标识符出现
——包名（`nomifun-app`、`nomifun-web` 等）、桌面 bundle id
`com.nomifun.desktop`、以 `NOMIFUN_` 为前缀的环境变量、仓库目录。
任何作为应用或项目品牌展示给人看的地方，都使用 "NomiFun"。

## 有托管版本吗？

没有。NomiFun 是一个自托管项目。不存在 SaaS 实例、不存在
`nomifun.com` 上的托管登录、不存在你可以注册的中心账户系统。使用它的
两种方式是：

- 安装桌面应用并在本地运行——`nomifun-desktop`。
- 在你自己控制的服务器上部署 `nomifun-web`。参见
  [Web 服务部署](../guides/web-server-deployment.md)。

你可以使用 [WebUI 远程访问](../guides/webui-remote-access.md) 把桌面
安装临时暴露给其他设备（手机、笔记本），但这是一项按实例的功能，
不是托管服务。

## 桌面应用需要登录吗？

不需要。桌面 WebView 通过 Tauri 外壳注入的每启动本地信任 token 被信任。
桌面窗口没有登录界面，但嵌入式后端并不是一个对所有 localhost 调用者都无鉴权
开放的服务。

Web 宿主与之相反：默认要求登录。两者的差异是有意的——桌面外壳可以信任
自己的 WebView，但网络可达的宿主必须有真正的鉴权边界。

## 我的数据存在哪里？

存在**数据目录**里。具体位置取决于你运行的是哪种宿主：

- **桌面端**：默认在**按用户的应用数据目录**——Windows 上是
  `%LOCALAPPDATA%\NomiFun\Nomi`，macOS 上是
  `~/Library/Application Support/NomiFun/Nomi`，Linux 上是
  `$XDG_DATA_HOME/NomiFun/Nomi`（通常为 `~/.local/share/NomiFun/Nomi`）。
  设置 `NOMIFUN_DATA_DIR=<absolute path>` 后目录会变成
  `$NOMIFUN_DATA_DIR/Nomi`（覆盖语义不变）。旧版构建把数据存在
  `<system temp>/nomifun-data/Nomi` 下；若存在这样的安装，启动时会
  自动搬迁到新位置，旧目录保留为备份。
- **Web（`nomifun-web`）**：你传给 `--data-dir`（或
  `NOMIFUN_DATA_DIR`）的任何位置，按字面值生效——不附加 `/Nomi`
  后缀。两者都未设置时，默认与桌面应用是**同一个按用户目录**，因此
  开发中的 `bun run serve:web` 与已安装的应用看到的是同一份状态。
- **Docker**：compose 文件中定义的命名卷（`nomifun-data`，挂载到
  `/data`）。

数据目录里有 SQLite 数据库（`nomifun-backend.db*`）、各智能体状态、
Bun 缓存、日志文件，以及任何嵌入式扩展数据。请像对待数据库一样备份
它。由于所有宿主默认指向同一个目录，后端用一把排他的 `server.lock`
守住它——同一数据目录上的第二个后端实例会快速失败，而不是悄悄破坏
状态。

完整生命周期与 `work-dir` 语义请参见
[配置参考](./configuration.zh.md#数据目录与工作目录的语义)。

## 支持哪些智能体与提供商？

NomiFun 作为 ACP（Agent Client Protocol）后端运行的"智能体 CLI"
包括 `claude`、`codex`、`gemini`、`nomi`、`codebuddy`、`qwen` 与
`opencode`。每一个都是一个独立的 CLI，需要你自己在系统上安装；
NomiFun 会从 `PATH` 上发现它们，并据此填充注册表。运行
`nomicore doctor` 即可看到你的安装能识别到哪些。

至于裸模型访问（例如提供商密钥、自定义的 OpenAI 兼容端点），系统
通过 `/api/providers/*` 与应用内的设置 UI 支持可配置的提供商。API
密钥由你提供；NomiFun 会以静态加密的方式把它们存到数据目录。

并不存在一个会调用某个托管 NomiFun 端点的内置智能体——根本就没有
这样的端点。你配置的每一个智能体 / 提供商都由你自己控制。

## NomiFun 真的纯本地吗？

应用逻辑与你的数据是本地的。但你接入的智能体未必——大多数 CLI 智能体
都会向各自的提供商（Anthropic、OpenAI、Google 等）发起出站调用。那
是你与该智能体之间的事。

NomiFun 自身在网络上做的事：

- 可选的更新检查（system info / check-update 端点）。
- 扩展市场（`/api/hub/*`）——仅在你主动使用时。
- 你已配置的智能体与提供商所做的——通常是去 LLM 提供商的 API 调用。

二进制中没有遥测管道、没有 analytics SDK，也没有 `SENTRY_DSN` 集成。
后端不会自行回传任何数据。

## 扩展与技能——它们由谁来运行？

扩展（主题、设定、频道插件、设置标签页）由 `nomifun-extension` 从
数据目录加载。技能则是一组提示词/指令，按会话被解析进智能体的上下文。
两者都是数据目录下的本地文件；市场流程只是把它们下载到该目录。

智能体 CLI 二进制不属于扩展——它们是 NomiFun 通过 ACP 协议作为子
进程派生的外部 CLI。

## 我能在与 UI 不同的机器上跑智能体吗？

可以——这正是 `nomifun-web` 的用途。把 Web 宿主部署到你希望智能体
（及其 CLI 和它们的网络访问）所在的机器上，然后从任何浏览器访问
SPA。参见 [Web 服务部署](../guides/web-server-deployment.md)。

如果只是想从手机或另一台笔记本做更轻量的远程访问，又不想另起一台
服务，[WebUI 远程访问](../guides/webui-remote-access.md) 可以把
现有的桌面安装暴露到 LAN 上。

## 许可证是什么？

**Apache-2.0**，在工作区 `Cargo.toml` 中声明。你可以在标准的
Apache-2.0 条款下使用、修改、再分发，并将代码打包——包括用于商业
产品——只要保留许可证与声明完整即可。

## 有预构建安装包吗？

还没有官方公开发布渠道。桌面包可以本地构建，macOS Developer ID 签名已通过
`bun run build:signed` 脚本接好，updater 产物可用 `bun run build:updater`
生成；但还没有官方 installer feed 或公开 registry。当前受支持的安装路径是：

- **桌面端**：`bun install && bun run build:ui && cargo run -p
  nomifun-desktop`（或 `cargo build --release -p nomifun-desktop`）。
- **服务端**：从源码构建（`cargo build --release -p nomifun-web`）
  或 `docker compose up -d --build`。

预构建安装包发布后，会从项目 README 与
[新手入门指南](../getting-started/) 链接出来。

## 我把管理员密码搞丢了

在 `nomifun-web` 上，带内的恢复流程是仅本地的 WebUI 路由
（`POST /api/webui/reset-password`）——你可以从运行该服务的同一台
机器上调用它，它会生成一个全新的随机密码并打印出来。从远程机器
无法通过 API 恢复密码。

兜底方案是停止服务、通过 `installation_identity.owner_user_id` 找到安装
所有者，直接编辑该用户的 `password_hash` 列，然后重启。最简单的重置
就是把哈希设为空字符串——下次启动
就会把当前安装当作需要重新进行首次启动设置，下一个访问者就可以
认领管理员。

桌面本地 WebView 没有密码。WebUI 远程访问有自己的管理员密码，因为它会被其他
浏览器访问。

## 有"单二进制"构建吗？

服务端有：`nomifun-web` 是一个静态链接的 Rust 二进制，再加上它要
提供的 `ui/dist/` 目录。SQLite 是静态链接的，TLS 用的是 rustls——
运行时没有 `libsqlite`、也没有 `openssl` 依赖。用
`cargo build --release -p nomifun-web` 即可构建。

智能体运行时（bun）默认*不*嵌入；请把它装到系统级，或者用
`NOMIFUN_EMBED_BUN=1` 构建标记把它打包进二进制。参见
[bun-on-PATH](../guides/web-server-deployment.md#bun-must-be-on-the-system-path)
小节。

桌面外壳同样可以产出单一二进制（`nomifun-desktop`），但用于分发时，
通常你想要的是 `bun run build` 通过平台原生打包流程产出的
产物（前提是安装包签名已配置好）。

## 另见

- [配置参考](./configuration.zh.md)
- [API 概览](./api-overview.zh.md)
- [疑难排查](./troubleshooting.zh.md)
- [Web 服务部署](../guides/web-server-deployment.zh.md)
- [作为桌面应用运行 NomiFun](../guides/desktop-app.zh.md)
