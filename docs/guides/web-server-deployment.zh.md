# Web 服务器部署

`nomifun-web` 是 NomiFun 的**无头、自托管**运行方式。它与 [桌面应用](./desktop-app.zh.md)嵌入的后端是同一个 Rust 后端，但被构建为一个独立二进制，并且会在同一个端口上同时提供 SPA (`ui/dist`)。它没有 GUI，没有 WebView，也不需要 `DISPLAY` —— 任何能运行静态二进制的 Linux/macOS/Windows 服务器上都能跑。

与桌面外壳不同，**`nomifun-web` 默认要求认证**。第一个浏览器访问者要么以交互方式创建管理员账户 (首次运行设置)，要么你通过 `NOMIFUN_ADMIN_PASSWORD` 预置凭据。

> 如果你想暴露一个*已有的*桌面安装以便远程访问，而不需要搭建服务器，请参阅 [WebUI 远程访问](./webui-remote-access.zh.md)。那是一个按实例启用的功能；本指南面向的是专用服务器。

```text
  浏览器 / 手机 / 局域网              nomifun-web（单进程、单端口）
  ┌──────────────────┐               ┌───────────────────────────────────────┐
  │  SPA + 登录       │  HTTP / WS    │  axum router                          │
  │  (ui/dist)        │ ────────────► │   ├─ /            → SPA (ui/dist)      │
  └──────────────────┘               │   ├─ /api/*       → REST handlers      │
                                      │   ├─ /ws          → WebSocket 事件     │
                                      │   └─ /login …     → 鉴权（默认开启）   │
                                      │                                       │
                                      │  进程内后端 (nomifun-app)              │
                                      │   └─ SQLite · agents · cron · channels │
                                      └───────────────────────────────────────┘
```

## 快速开始

### 直接运行二进制

```bash
cargo build --release -p nomifun-web
./target/release/nomifun-web --host 127.0.0.1 --port 8787 \
  --data-dir ./data --dist ./ui/dist
```

然后打开 `http://127.0.0.1:8787`，首次访问时让你创建管理员账户。之后，setup 端点会返回 `409 Conflict`，唯一的进入方式就是通过登录表单 (或 `NOMIFUN_ADMIN_PASSWORD`)。

![首次运行管理员设置界面](../images/webserver-02-first-run-setup.png)

### 或者从仓库通过 Cargo 运行

```bash
bun install
bun run build:ui              # 产出 ui/dist
cargo run -p nomifun-web      # 会自动使用默认 --dist=../../ui/dist
```

## CLI 参数和环境变量

下方所有参数由 `apps/web/src/main.rs` 读取。每个都有对应的环境变量，方便用于 systemd、Docker 等部署环境。

| 参数 | 环境变量 | 默认值 | 用途 |
|---|---|---|---|
| `--host` | `NOMIFUN_WEB_HOST` | `127.0.0.1` | 绑定的 IP。`0.0.0.0` 会接收 LAN/VPN/公网流量；大范围暴露前请先预置管理员或完成首次设置。 |
| `--port` | `NOMIFUN_WEB_PORT` | `8787` | TCP 端口。提供 API、`/ws` 处的 WebSocket，以及 SPA。 |
| `--data-dir` | `NOMIFUN_DATA_DIR` | 按用户目录 | 后端数据目录 (SQLite 数据库、agent 状态、日志、Bun 缓存)。默认是与桌面应用共享的按用户位置 (`%LOCALAPPDATA%\NomiFun`、`~/Library/Application Support/NomiFun`、`$XDG_DATA_HOME/NomiFun`)。**生产环境请仍显式指定绝对路径。** |
| `--dist` | `NOMIFUN_WEB_DIST` | `../../ui/dist` | 构建好的 SPA 所在目录。**部署时请显式设置。** |
| `--admin-user` | `NOMIFUN_ADMIN_USERNAME` | `admin` | 预置首个管理员时使用的用户名。一旦管理员存在则被忽略。 |
| `--admin-password` | `NOMIFUN_ADMIN_PASSWORD` | — | 在启动时预置首个管理员密码，跳过交互式设置。一旦管理员存在则被忽略。 |
| `--insecure-no-auth` | `NOMIFUN_WEB_INSECURE_NO_AUTH` | `false` | **危险。** 完全禁用认证 (类似桌面的本地模式)。仅在 loopback 或完全可信的私有网络上使用。 |
| — | `NOMIFUN_HTTPS` | `false` | 当为 `true` 时，session 和 CSRF cookie 会带上 `Secure` 标记。每当应用通过 HTTPS 访问 (例如位于 TLS 反向代理之后) 时都应设置。 |
| — | `NOMIFUN_ROBOT_ADVERTISE` | — | 局域网机器人 (ESP32) 要拨的地址：`<ipv4>` 或 `<ipv4>:<port>`。**Docker 下必须设置**；裸机部署可不填，会自动探测局域网网卡。取值非法会直接拒绝启动。参见[局域网机器人 (ESP32)](#局域网机器人-esp32)。 |
| — | `SHELL` | 平台默认 | Agent 引擎派生进程时使用的 shell。在 Linux 服务器上若 `$SHELL` 未设置，请设为 `/bin/bash`。 |

布尔类环境变量接受 `1`、`true`、`yes`、`on` (大小写不敏感)。

错误的 `--host` (任何无法解析为 IP 的内容) 会在启动时快速失败并给出清晰错误，而不是抛出晦涩的 socket 错误。

后端启动时会对 `{data_dir}/server.lock` 取 OS 级排他锁 —— **同一数据目录只允许一个后端实例**。第二个指向同一目录的进程会快速失败，错误信息会指出当前持有者 (pid + exe)；要部署多个实例，请为每个实例指定各自独立的 `NOMIFUN_DATA_DIR` / `--data-dir`。锁在进程退出或崩溃时由 OS 自动释放，残留的 `server.lock` 文件是无害的。

### 密码与用户名规则

当管理员账户被创建时 (无论是交互式还是预置)，值都会在服务端校验：

- **用户名**：3–32 字符，`[a-zA-Z0-9_-]`，不能以 `-` / `_` 开头或结尾。
- **密码**：8–128 字符，若出现在一个小型常见密码列表中 (`password`、`12345678`、`qwertyui` …) 则被拒绝。

弱的 `NOMIFUN_ADMIN_PASSWORD` 会拒绝启动。交互式输入的弱密码会返回 `400` 并附带校验信息。

## 首次运行管理员配置

支持两种路径。

### 交互式 (默认)

不设置 `NOMIFUN_ADMIN_PASSWORD`。在新的数据目录上，安装处于 "未初始化" 状态：`GET /api/auth/status` 会报告 `needs_setup: true`，SPA 显示首次运行表单，**第一个浏览器访问者所选的用户名 + 密码会通过原子化的 `POST /api/auth/setup` 成为管理员**。该写入是一个条件性 UPDATE —— 即便两个并发的首次运行请求也无法同时获胜；输者会收到 `409 Conflict`。

> **安全提示 —— 首次运行窗口期。** 在服务器可达的那一刻起，到你完成设置之间，任何能到达该端口的人都可以认领管理员账户。在非 loopback 绑定上，服务器会记录一条醒目的警告。可通过先在受信任的隧道/VPN 上完成设置来缓解，或预置 (见下一节) 让安装在上线前就已初始化。

### 预置 (推荐用于自动化)

在首次启动前提供 `NOMIFUN_ADMIN_PASSWORD` (以及可选的 `NOMIFUN_ADMIN_USERNAME`，默认 `admin`)。引导例程会原子地哈希并存储凭据，从首次启动开始首次运行 setup 端点就会返回 `409`，没有任何窗口让别人来认领账户。

```bash
NOMIFUN_ADMIN_USERNAME=alice \
NOMIFUN_ADMIN_PASSWORD='change-me-to-something-strong' \
nomifun-web --host 0.0.0.0 --port 8787 \
  --data-dir /var/lib/nomifun --dist /opt/nomifun/web
```

预置是**幂等的** —— 一旦管理员存在，后续启动时这些环境变量会被忽略。要轮换凭据，请使用应用内的修改密码 / 修改用户名流程，而不是环境变量。

## Docker

官方 Docker Hub 镜像是
[`nomifun/nomifun-web`](https://hub.docker.com/repository/docker/nomifun/nomifun-web)。
当你想在没有源码 checkout 的机器上部署时，直接使用它。仓库也附带一个多阶段
`Dockerfile` 和一个 `docker-compose.yml`，用于从源码本地构建。下面示例使用
Docker Hub 发布的稳定滚动标签 `latest`；如需可复现部署，请固定明确版本或镜像
digest。镜像会：

1. 用 Bun 构建 SPA。
2. 从 workspace 编译 `nomifun-web`。
3. 组装一个精简的 `debian:trixie-slim` 运行时，包含 Bun、带 npm/npx
   的 Node.js 22、带 PyYAML 的 Python 3、Git 和 ripgrep。

它暴露端口 `8787`，并使用 `/data` 作为数据卷。

### 官方镜像

```bash
docker run -d \
  --name nomifun-web \
  --restart unless-stopped \
  -p 8787:8787 \
  -v nomifun-data:/data \
  nomifun/nomifun-web:latest
# 然后打开 http://<server-ip>:8787 并创建首位管理员
```

无人值守或公网部署时，请预置首位管理员：

```bash
docker run -d \
  --name nomifun-web \
  --restart unless-stopped \
  -p 8787:8787 \
  -v nomifun-data:/data \
  -e NOMIFUN_ADMIN_USERNAME=admin \
  -e NOMIFUN_ADMIN_PASSWORD='change-me-to-something-strong' \
  nomifun/nomifun-web:latest
```

### 用 Compose 从源码本地构建

```bash
docker compose up -d --build
# 然后打开 http://<server-ip>:8787 并创建首位管理员
```

`restart: unless-stopped` 让服务在主机启动时启动 —— 安装它*就是*启用它。默认的 ports 块直接发布 `8787:8787`；请先预置管理员或在受信网络完成首次设置，再大范围暴露。暴露到公网前请加上 TLS (下一节)。

验证就绪：

```bash
docker compose logs -f nomifun
# 查找：“nomifun-web: embedded backend + SPA on one port”
```

compose 文件挂载了一个名为 `nomifun-data:/data` 的具名卷，其中保存着 SQLite DB、日志、Bun 运行时缓存以及每个 agent 的状态。请像对待其他数据库一样仔细备份。

### 在 Compose 中预置管理员

取消 `environment:` 块的注释：

```yaml
environment:
  NOMIFUN_ADMIN_USERNAME: admin
  NOMIFUN_ADMIN_PASSWORD: "change-me-to-something-strong"
  NOMIFUN_HTTPS: "true"        # 在 Caddy / nginx 加 TLS 前置时启用
```

### 在缓慢的 registry 后构建

Rust 阶段接受一个 `CARGO_REGISTRY_MIRROR` 构建参数用于 cargo 注册表镜像 (例如在 crates.io 较慢的网络上)：

```bash
docker build --build-arg CARGO_REGISTRY_MIRROR=https://rsproxy.cn/index/ -t nomifun-web:local .
```

```text
$ docker compose up -d
[+] Running 2/2
 ✔ Network nomifun_default  Created
 ✔ Container nomifun-web    Started

$ docker compose logs -f web
nomifun-web  | listening on 0.0.0.0:8787 (auth: enabled)
```

## 通过 Caddy 反向代理实现 TLS

仓库附带一个用于 Caddy 2 的 `Caddyfile`。Caddy 会自动签发 HTTPS 证书 (默认 Let's Encrypt 或 ZeroSSL) 并代理到应用。`/ws` 处的 WebSocket 升级会自动透传，无需额外配置。

```caddy
your.domain.com {
    encode zstd gzip
    reverse_proxy nomifun:8787
}
```

要在 `docker-compose.yml` 中启用 Caddy 服务：

1. 编辑 `Caddyfile` 并把 `your.domain.com` 替换为你的真实域名。
2. 在 `nomifun` 服务的环境变量中设置 `NOMIFUN_HTTPS=true` (这样 cookie 会带上 `Secure` 标记)。
3. 把 `ports: ["8787:8787"]` 替换为 `expose: ["8787"]`，让只有 Caddy 对外发布。
4. 取消 `caddy:` 服务以及 `caddy-data` / `caddy-config` 卷的注释。
5. `docker compose up -d`。

应用本身已经提供了登录界面，所以**不要在 Caddy 里配置 HTTP basic auth** —— Caddy 的职责只是 TLS 终结和代理。

对于没有公网域名的仅局域网主机，可以使用一个内部名加上 `tls internal`，或者干脆不加 Caddy 直接发布端口 `8787` (应用内登录依然提供保护)。

如果你要用局域网机器人，第 3 步会切断它们唯一的入口 —— 参见[局域网机器人 (ESP32)](#局域网机器人-esp32)。

## 其他反向代理：必须保留原始 Host

实时更新 (流式回复、工具调用、任务/队列状态) 全部走 `/ws` 上的 WebSocket。
握手时会做浏览器来源校验：浏览器发送的 `Origin` 必须与后端看到的
authority 一致。如果代理**把 `Host` 改写为上游地址** (nginx 的默认行为 ——
`proxy_set_header Host $proxy_host`)，这个匹配就会失败：每次 `/ws` 握手都被
`403` 拒绝，而普通 HTTP API 请求不受影响。表现出来的症状就是 WebUI 一直停留在
"执行中"，只有刷新页面才能看到最新状态。

握手按顺序接受以下来源：

1. `Origin` 与请求 `Host` 一致；
2. `Origin` 与 `X-Forwarded-Host` 的第一个条目一致 (由代理设置)；
3. `Origin` 出现在 `NOMIFUN_ALLOWED_ORIGINS` 中 (逗号分隔的完整来源，例如
   `NOMIFUN_ALLOWED_ORIGINS=https://nomi.example.com`)。

Caddy 无需任何配置 (它默认保留 `Host`)。nginx 需要转发原始 authority 和升级头：

```nginx
location / {
    proxy_pass http://127.0.0.1:8787;
    proxy_http_version 1.1;
    proxy_set_header Host $host;                 # 保留浏览器侧的 authority
    proxy_set_header X-Forwarded-Host $host;     # 多级代理链的双保险
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header Upgrade $http_upgrade;      # /ws 的 WebSocket 升级
    proxy_set_header Connection "upgrade";
    proxy_read_timeout 300s;                     # 空闲 WS 连接会超过 60s 默认值
}
```

如果前置的代理或隧道 (某些内网穿透 / 端口转发产品) 无法配置转发这两个头，
请把 `NOMIFUN_ALLOWED_ORIGINS` 设置为用户在浏览器里实际输入的公开来源。
被拒绝的握手会在服务端以 `WARN` 级别记录具体的 `origin` / `host` /
`forwarded_host` 值，`docker compose logs | grep "rejected websocket"`
可以直接看到需要配置的内容。

## 局域网机器人 (ESP32)

`nomifun-web` 同样提供机器人网关 —— 设备面 `/robot/*` 与管理面 `/api/robots*`
就在桌面版使用的同一个 router 里。唯一与宿主环境相关的事情是：**告诉机器人该连
到哪个地址。**

机器人只会从 OTA 响应里获知这个地址一次，之后直接以明文 `http`/`ws` 拨过去。所以
服务端必须知道自己哪个地址是**从机器人所在网络能到达的**：

- **裸机 / systemd 跑在局域网主机上** —— 无需配置，会自动探测本机的局域网 IPv4
  (优先取路由默认出口的那张网卡)。
- **Docker / Podman —— 必须设置 `NOMIFUN_ROBOT_ADVERTISE`。** 在容器里自动探测
  只能得到容器自己的地址 (`172.17.0.2`)，局域网里的设备根本路由不到。这个问题在
  原理上无法靠探测解决：可达地址属于宿主机，机器人拨的端口是端口映射的宿主机一
  侧，这两件事进程都看不见。

```bash
# Compose (或本地 .env)：填运行 Docker 那台机器的局域网 IP。
NOMIFUN_ROBOT_ADVERTISE=192.168.1.50

# 端口被改过映射 (`-p 9000:8787`)：写机器人要拨的端口，而不是进程绑定的端口。
NOMIFUN_ROBOT_ADVERTISE=192.168.1.50:9000
```

规则：

- 取值为 `<ipv4>` 或 `<ipv4>:<port>`。不带端口时，广告的是本进程实际绑定的端口。
- **只支持 IPv4 字面量。** 主机名和 IPv6 会被拒绝 —— 设备拿到的是一条裸的
  `ws://<ipv4>:<port>` URL，既没有 TLS 也没有名字解析。带证书的公网主机名属于
  (尚未发布的) 远程中继阶段，不在这里。
- 取值非法 (不是 IPv4 字面量、端口不合法、`0.0.0.0`) 会**直接让启动失败**，错误信
  息里会点名这个变量。这里故意不做静默回落到自动探测：那会让人以为配好了，实际
  却是坏的。
- 未设置和设为空字符串等价 (都走自动探测)，因此 Compose 可以无条件透传这个变量名。

验证实际广告出去的地址。该接口需要 owner 认证，最方便的办法是在已登录的 WebUI
标签页的 DevTools 控制台里执行：

```js
await fetch('/api/robots/endpoints').then(r => r.json())
// { ota_urls: ["http://192.168.1.50:9000/robot/ota"], lan_enabled: true }
```

在 `--insecure-no-auth` 的宿主上可以直接
`curl http://<server-ip>:8787/api/robots/endpoints`。

`"lan_enabled": false` 或 `ota_urls` 为空，说明没有发布出任何可达地址 —— Docker
下就是漏了 `NOMIFUN_ROBOT_ADVERTISE`。这里打印出来的 URL 就是要填进固件 OTA 输入
框的那一个。启动日志给出同样的信息：搜
`robot: advertising this endpoint to devices`。

**机器人不走 TLS 反向代理。** 设备面就是上面那个地址上的明文 `http`/`ws`，所以即
使浏览器是通过 Caddy 的 443 访问，那个端口也必须在局域网内可达。如果你为了只暴露
Caddy 而把 `ports: ["8787:8787"]` 换成了 `expose:`，机器人就没有入口了：请重新发布
该端口 (只对局域网发布即可) 并让 `NOMIFUN_ROBOT_ADVERTISE` 指向它。把设备面挂到
Caddy 上**不能**替代这一步 —— 固件拨的是它被告知的那条裸 IPv4 URL，不带 SNI，也不
做证书校验。

## systemd (Linux 服务器，无 Docker)

仓库包含 `packaging/linux/nomifun-web.service` 以及一份长篇 Linux 部署指南 `packaging/linux/README.md`。

### 构建产物

你需要一台 Linux 构建主机 (从 Windows 交叉编译 C 依赖很痛苦 —— 最简单的变通是用 `docker cp` 从 Docker 镜像中提取二进制)。在 Linux 上：

```bash
bun install
bun run build:ui                      # → ui/dist (~21MB)
cargo build --release -p nomifun-web  # → target/release/nomifun-web
```

### 布局

```
/opt/nomifun/nomifun-web    # 二进制
/opt/nomifun/web/           # ui/dist 的内容
/var/lib/nomifun/           # 数据目录 (由 systemd 的 StateDirectory 创建)
```

```bash
sudo useradd --system --home /var/lib/nomifun --shell /usr/sbin/nologin nomifun
sudo mkdir -p /opt/nomifun/web
sudo cp target/release/nomifun-web /opt/nomifun/
sudo cp -r ui/dist/. /opt/nomifun/web/
```

### Bun 必须在系统 `PATH` 上

Agent 引擎需要 **`bun ≥ 1.3.13`** 作为运行时依赖。由于服务以一个 `nologin` 系统账户运行，安装在某个用户 `~/.bun/bin/` 下对它来说是不可见的。请二选一：

- **系统级安装**：`curl -fsSL https://bun.sh/install | bash`，然后 `sudo install ~/.bun/bin/bun /usr/local/bin/bun`。
- **嵌入二进制**：使用 `NOMIFUN_EMBED_BUN=1 cargo build --release -p nomifun-web` 构建。Bun 会被打包进二进制中，并在首次运行时自解压到数据目录。

验证：`sudo -u nomifun -s -- which bun` 必须返回一个路径。否则首次 agent 派生会以一个晦涩的错误失败。

### 安装 unit

```bash
sudo cp packaging/linux/nomifun-web.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nomifun-web
sudo systemctl status nomifun-web
```

附带的 unit：

- 默认绑定 `127.0.0.1:8787`。只有在首次设置完成或已配置
  `NOMIFUN_ADMIN_PASSWORD` 后，才应把 `NOMIFUN_WEB_HOST` 改成
  `0.0.0.0`。
- 设置 `NOMIFUN_DATA_DIR=/var/lib/nomifun` 以匹配 systemd 管理的 `StateDirectory=nomifun`。**保持这两者同步** —— 如果你删除该环境变量行，数据目录会静默回退到服务用户的按用户目录 (`$XDG_DATA_HOME/NomiFun`，通常是 `~nomifun/.local/share/NomiFun`)，与 systemd state 解耦。
- 以专用的 `nomifun` 用户运行 (`User=nomifun`、`Group=nomifun`)。
- 失败时以 3 秒退避重启。
- 应用适度的硬化 (`NoNewPrivileges=yes`、`PrivateTmp=yes`)。**不要添加** `ProtectHome=yes` 或严格的 `ProtectSystem` —— agent 引擎需要读写操作员指定的文件，过度沙箱化会破坏核心功能。

要在 TLS 代理后启用 HTTPS cookie，请取消注释：

```ini
Environment=NOMIFUN_HTTPS=true
```

要预置管理员而不是交互式设置：

```ini
Environment=NOMIFUN_ADMIN_USERNAME=admin
Environment=NOMIFUN_ADMIN_PASSWORD=change-me-to-something-strong
```

```text
$ sudo systemctl status nomifun-web
● nomifun-web.service - NomiFun web host
     Loaded: loaded (/etc/systemd/system/nomifun-web.service; enabled; preset: enabled)
     Active: active (running) since Tue 2026-06-25 09:12:03 UTC
   Main PID: 12345 (nomifun-web)
     CGroup: /system.slice/nomifun-web.service
             └─12345 /usr/local/bin/nomifun-web --host 127.0.0.1 --port 8787 …
nomifun-web[12345]: listening on 127.0.0.1:8787 (auth: enabled)
```

## Linux 运行时依赖

| 依赖 | 是否必需 | 说明 |
|---|---|---|
| `glibc` + `ca-certificates` | 是 | sqlite 是静态链接的，TLS 使用 rustls —— **不需要 openssl，不需要 libsqlite**。 |
| `bun` ≥ 1.3.13 | **是** | Agent 执行运行时。1.1.38 有 stdin bug；不要使用。Docker 镜像里已包含。 |
| Node.js 22 + `npm` / `npx` | 推荐 | 许多 MCP stdio 服务器以及 OfficeCLI 自动安装链路会使用。Docker 镜像里已包含。 |
| Python 3 + PyYAML | 推荐 | 支持 Agent 的 Python 脚本模式和随附的 Python 内置技能。Docker 镜像里已包含。 |
| `git` | 推荐 | 技能发现和一些内置工具会使用。 |
| `ripgrep` (`rg`) | 推荐 | 代码搜索后端。缺失时回退到 `grep`。 |
| `DISPLAY` / X11 / WebView | **否** | `nomifun-web` 是完全无头的。 |

## 安全检查清单

- **任何公网部署都要使用 TLS。** 通过明文 HTTP 传输的 cookie 和登录凭据可能被嗅探。在 TLS 代理后请设置 `NOMIFUN_HTTPS=true`，让 session cookie 带上 `Secure` 标记。
- **强管理员密码。** 校验器会拒绝长度低于 8 字符的密码以及一些显而易见的字典条目，但它并不强制执行强度评分 —— 请选择长且随机的内容。怀疑被泄露时，请通过应用内流程修改它；修改密码端点会轮换 JWT 密钥，使所有现有会话失效。
- 对于任何在你能进行交互式设置之前就已可达的主机，请用 `NOMIFUN_ADMIN_PASSWORD` **关闭首次运行窗口期**。另一种做法是先保持 `127.0.0.1`，完成设置后再显式绑定 `0.0.0.0`。
- **`--insecure-no-auth` 默认是敌对的。** 它完全禁用认证；*任何*能到达该端口的人都会成为拥有 shell、文件和 agent 访问权限的特权用户。仅在 loopback 绑定或完全可信的私有网络上使用。当它在非 loopback 地址上启用时，服务器会记录警告。
- 后端拥有终端、文件系统和 agent 执行能力 —— 远程运行它，本设计上等同于给自己开通了对该主机的远程代码执行。Auth + TLS 是底线，不是上限。请像对待 root 凭据一样对待数据目录和管理员密码。

## 故障排查

**`invalid --host '<value>'`。** 请传入一个 IP 字面量 (`127.0.0.1`、`0.0.0.0`、显式接口 IP)。不解析主机名。

**HTTPS 下 cookie 无法保留。** 设置 `NOMIFUN_HTTPS=true` 以加上 `Secure` 标记。否则浏览器会在 HTTPS 响应中拒绝该 cookie。

**回复/任务状态只有刷新页面后才更新 (WebUI 一直显示"执行中")。** `/ws` WebSocket 握手被拒绝了 —— 在服务端日志中查找 `rejected websocket upgrade` (WARN，包含具体的 `origin`/`host`/`forwarded_host` 值) 或 `GET /ws` → `403`。几乎总是因为反向代理改写了 `Host`：参见[其他反向代理：必须保留原始 Host](#其他反向代理必须保留原始-host)。

**在 systemd 下 agent 命令失败并报 `bun: command not found`。** 请系统级安装 bun (参见上面的 bun-on-PATH 一节) 或使用 `NOMIFUN_EMBED_BUN=1` 重新构建。

**机器人始终连不上 (固件的 OTA 检查成功，但看不到会话)。** 服务端没有发布出可达地址，于是 OTA 响应里的 websocket URL 是空的。先看 `/api/robots/endpoints`；Docker 下请把 `NOMIFUN_ROBOT_ADVERTISE` 设为**宿主机**的局域网 IP —— 参见[局域网机器人 (ESP32)](#局域网机器人-esp32)。

**健康检查。** 使用 `GET /health` 作为进程存活探针；只有在调用方还需要设置 / 认证状态时，才使用 `GET /api/auth/status`。

## 另请参阅

- [以桌面应用方式运行 NomiFun](./desktop-app.zh.md)
- [WebUI 远程访问](./webui-remote-access.zh.md) —— 把现有桌面安装变成一个可远程访问的服务器 (无需另置一台机器)。
- `packaging/linux/README.md` —— 更深入的 Linux 笔记 (主要是中文；本指南涵盖了其英文部分)。
- `apps/web/src/main.rs` —— 参数、环境变量和引导顺序的真相之源。
