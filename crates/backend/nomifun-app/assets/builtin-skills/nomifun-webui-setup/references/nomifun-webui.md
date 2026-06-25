# NomiFun WebUI 远程访问指南

NomiFun 有两种浏览器访问方式：

- **桌面实例远程访问**：在现有 NomiFun 桌面应用中临时开启 LAN/VPN 访问。
- **独立 Web 服务器**：使用 `nomifun-web` 在服务器上无头运行同一个后端和 SPA。

桌面实例的远程访问配置都应通过 **Open Capabilities / Remote Access**
面板完成。不要引导用户编辑旧版远程访问配置文件，也不要安装
`@nomifun/webui` 之类的 npm 包。

## 选择路径

| 场景 | 推荐路径 |
| --- | --- |
| 手机或同一 WiFi 设备访问正在运行的桌面应用 | 桌面 Open Capabilities |
| 通过 Tailscale / VPN 访问自己的电脑 | 桌面 Open Capabilities + VPN 地址 |
| 云服务器、NAS、长期在线部署 | `nomifun-web` |
| 需要固定端口、systemd、Docker 或 TLS 反代 | `nomifun-web` |

## 桌面实例：Open Capabilities

1. 打开 NomiFun 设置。
2. 进入 **Open Capabilities / 开放能力**。
3. 打开 **Remote Access / WebUI** 区域。
4. 按需启用远程访问服务。
5. 使用面板展示的访问 URL、二维码或访问令牌流程连接。

如果用户只需要同一局域网访问，确认手机和电脑在同一网络中，并让用户使用
面板显示的 LAN 地址。若用户使用 Tailscale，使用 Tailscale 分配给电脑的
IP/域名访问，不需要把端口暴露到公网。

## 独立服务器：`nomifun-web`

服务器部署不要走桌面设置界面。使用 `nomifun-web`：

```bash
nomifun-web --host 127.0.0.1 --port 8787 \
  --data-dir /var/lib/nomifun \
  --dist /opt/nomifun/web
```

需要 LAN/VPN 访问时，可在完成首次管理员设置或预置管理员后显式绑定：

```bash
NOMIFUN_ADMIN_USERNAME=admin \
NOMIFUN_ADMIN_PASSWORD='change-me-to-something-strong' \
nomifun-web --host 0.0.0.0 --port 8787 \
  --data-dir /var/lib/nomifun \
  --dist /opt/nomifun/web
```

公网访问必须放在 TLS 反向代理之后，并设置 `NOMIFUN_HTTPS=true`。

## 常见排查

### 无法从手机访问

- 确认桌面 Open Capabilities 面板中的远程访问服务正在运行。
- 确认手机与电脑在同一 LAN，或两端都已连接同一个 VPN/Tailscale 网络。
- 检查系统防火墙是否阻止 NomiFun 监听端口。
- 如果使用 `nomifun-web`，确认 `--host` 不是只绑定在 `127.0.0.1`。

### 首次登录或密码问题

- 桌面远程访问按面板展示的信息登录或走访问令牌流程。
- `nomifun-web` 首次访问会创建管理员；任何对外可达部署都建议预置
  `NOMIFUN_ADMIN_PASSWORD`，避免首次设置窗口被别人抢占。
- 忘记自托管管理员密码时，优先使用本地可信恢复流程或恢复备份。

### 端口问题

- 桌面远程访问端口由运行时管理，通常不需要手工配置。
- 服务器部署用 `--port` / `NOMIFUN_WEB_PORT`。
- 排查端口占用：

```bash
lsof -i :25808
lsof -i :8787
```

## 安全建议

- 只在可信网络内开启桌面远程访问。
- 公网部署必须使用 TLS。
- 不要关闭认证，除非只绑定 loopback 或完全受信私网。
- 令牌和管理员密码按远程代码执行级别保护。

## 相关文档

- [WebUI 远程访问](../../../../../../../docs/guides/webui-remote-access.zh.md)
- [Web 服务器部署](../../../../../../../docs/guides/web-server-deployment.zh.md)
- [远程能力 API](../../../../../../../docs/guides/remote-capability-api.zh.md)
