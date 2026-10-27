# 疑难排查

[English](troubleshooting.md)

先确认实际进程日志、选中的数据目录和宿主模式。
[配置](configuration.zh.md)解释参数及目录优先级，
[API 概览](api-overview.zh.md)用于定位所属路由。

## 网络与端口

| 现象 | 检查 |
| --- | --- |
| `invalid --host` | 使用 `127.0.0.1`、`0.0.0.0` 等 IP 字面量，不接受 hostname。 |
| `address already in use` | Unix 用 `lsof -i :8787`、PowerShell 用 `Get-NetTCPConnection -LocalPort 8787` 定位监听者；停止目标进程或换端口。 |
| 其他电脑无法连接 | Web 默认绑定 loopback。检查目标绑定地址、防火墙、反向代理和实际发布端口，广泛暴露前完成 owner 设置。 |
| Desktop 远程地址不可用 | 以[连接与设备](../guides/webui-remote-access.zh.md)显示的地址和状态为准；LAN 与 relay 独立控制。 |

## 鉴权与 CSRF

没有预置凭据的全新独立 Web 宿主返回
`GET /api/auth/status` 的 `needs_setup: true` 属于正常状态，
暴露服务前完成首次设置。Setup 返回 `409` 表示 owner 已存在。
启动管理员环境变量不替换已有账号。

登录后反复 `401`，用浏览器开发工具检查请求与响应：

- Cookie/Bearer token 是否到达同一部署 origin。
- 代理是否保留 cookie 和鉴权 header。
- HTTPS 部署设置 `NOMIFUN_HTTPS=true`，启用 `Secure` 和
  `SameSite=Strict`；Secure cookie 不会通过普通 HTTP 发送。
- 服务端时钟是否正确，密码修改是否已使旧会话失效。

`403 CSRF token validation failed` 要求 `nomifun-csrf-token` cookie
与 `x-csrf-token` header 相同。检查 origin/cookie 策略及代理重写。
公开 login/setup 路由豁免；登录后变更遵循当前鉴权策略。
桌面使用注入的本地信任路径。

忘记密码时，可信 Desktop 应用可在
**连接与设备 → WebUI**管理其 WebUI 账号。
Loopback 浏览器或独立宿主连接不会获得桌面信任。
独立宿主应保留数据并收集鉴权错误/日志，由维护者协助恢复。
不要清空密码 hash、删除数据库或通过暴露无鉴权模式恢复访问。

## WebSocket 断连

策略关闭码 `1008` 通常表示缺少 token 或 token 过期。
重新获取 `GET /api/ws-token` 后连接，确认代理保留 upgrade 和所用鉴权传输。
正常关闭码为 `1000`。

服务端心跳默认 30s、超时 60s。连接停止收事件时检查代理 idle timeout 和网络变化。
自定义客户端应退避重连，忽略未知事件类型。

## Agent CLI 与受管组件

会话使用内置 `nomi` runtime；`claude`、`codex`、`gemini`
等终端预设启动的是另外安装的 CLI。

```bash
nomicore doctor
nomicore tool-doctor
```

诊断使用与应用相同的用户、PATH 和数据根。
安装缺失的第三方 CLI 或修正 PATH 后重启。
`tool-doctor` 读取组件目录；`tool-doctor --verify` 才会明确解包/探测现有组件，
不打开数据库、不下载 Node/Bun。Web 宿主可执行文件不提供这些子命令；
诊断 binary 可用 `cargo build --release -p nomifun-app --bin nomicore` 构建。

发布构建内嵌 OfficeCLI。必需的 Node/npm/npx 与 Bun/`bunx` 命令优先使用有效用户工具，缺失时自动准备批准的受管制品。Agent、Terminal、MCP 共用每次启动环境，托管应用进程也复用该环境。
Systemd 下确认服务账号可读/执行缓存，按需设置 `SHELL`。
详见[外部工具](../architecture/external-tools.zh.md)。

NomiRelay 连接、恢复与重启复用批准的本地 `nfagent`，缺失或版本变化时自动准备当前批准制品，保留进度、重试与取消。离线 `NOMIFUN_NFAGENT_PATH` 仍须符合 manifest hash。

## Office 预览

组件准入失败时，用同一数据根执行 `tool-doctor --verify`。
无效/加密文档、不支持格式、资源上限及写入不稳定有独立错误；
进程提前退出属于渲染失败。界面提供重试时可重试，也可系统打开或下载。

Artifact frame 空白时，反向代理必须保留完整
`/api/office-preview-artifacts/{capability}/` 路径和 query。
请求身份、capability 和 nonce 必须匹配当前预览。
不要放宽 CSP 或豁免任意 API。HTTP ready 和 iframe load 不证明已经显示成功。
详见 [Office artifact 契约](../architecture/external-tools.zh.md)。

## 数据目录与数据库

`unable to open database file` 先检查实际服务账号的目录所有权、磁盘空间和
挂载权限。Docker bind mount 必须对容器 UID 可写，命名卷可简化所有权。
在允许预期数据路径访问时保留部署加固。

`data directory ... is already in use` 时关闭提示的占用者，或改用其他
`NOMIFUN_DATA_DIR` / `--data-dir`。
不要删除 `server.lock` 绕过仍存活的持有者。
进程退出时 OS 释放锁，遗留文件无害。同通道宿主共享默认值，
开发脚本选择独立数据根。

可读但不兼容的数据库 lineage 由启动 dataset coordinator
按当前破坏性 baseline 处理重建。支持 schema 的损坏和 I/O 失败保持关闭；
restore 不获得重建权限。不要用迁移条数推断应用版本顺序。
详见[数据存储](../architecture/data-and-storage.zh.md)。

## Docker 构建与启动

先看 `docker compose logs nomifun` 与 `docker compose ps`，
检查端口映射、构建后的 SPA 路径和数据卷权限。
没有管理员密码时首次设置仍可用，不代表启动崩溃。

选择镜像源前先定位失败阶段：

| 阶段 | 运维配置 |
| --- | --- |
| 基础镜像 metadata / TLS timeout | Docker registry 配置，或可信 `BUN_IMAGE`、`RUST_IMAGE`、`RUNTIME_IMAGE` |
| apt 包 | `APT_MIRROR` |
| JS 包 | `BUN_REGISTRY` |
| crates.io | `CARGO_REGISTRY_MIRROR` |

只使用可信且兼容的镜像源，详见[部署指南](../guides/web-server-deployment.zh.md)。

## 日志与问题报告

日志写 stdout 和默认 `<data-dir>/logs` 下的按日文件，除非显式覆盖。
查看 `docker compose logs nomifun`、`journalctl -u nomifun-web`
或本地 `nomicore.log`。定向追踪可向 `nomicore` 传入
`--log-level info,nomifun_mcp=trace`。

报告包含应用版本、OS、宿主模式、准确错误、复现步骤和相关脱敏日志。
排除凭据、隐私 prompt 与 token。用 `rg` 搜索准确后端错误可找到所属检查。