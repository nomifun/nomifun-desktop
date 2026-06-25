# 在无 UI 的 Linux 服务器上部署 NomiFun WebUI

本目录是 **headless(无图形界面)** 部署产物。服务器上**不要**装 Tauri 桌面包(`.deb`/`.AppImage` 需要图形会话);要跑的是 `nomifun-web` —— 一个纯 axum 的 HTTP/WS 服务,后端 in-process + 同端口托管前端 SPA,**零 GUI 依赖**,可在没有显示器的服务器上运行。

## 访问与认证模型(重要)

- **`nomifun-web` 默认要求登录。** 第一次用浏览器打开时,登录框里**你输入的用户名 + 密码就成为初始管理员账号**(首启 setup,一次性)。之后该端点关闭(再调返回 409),只能用这套凭据登录。
- **桌面版(Tauri)不受影响**,仍是本机免认证。
- 默认建议先只监听 `127.0.0.1`,完成首启管理员设置或预置
  `NOMIFUN_ADMIN_PASSWORD` 后,再显式改为 `0.0.0.0` 暴露给 LAN/VPN。
  **面向公网请务必在前面加 TLS**(见下文 Caddy)。
- 可选:设置 `NOMIFUN_ADMIN_PASSWORD` 在启动时**预置**管理员,跳过交互式首启(适合自动化部署 / 想关闭首启竞态窗口的场景)。

---

## 方式一:Docker(推荐)

仓库根目录已带 `Dockerfile`、`docker-compose.yml`、`Caddyfile`。

```bash
# 在仓库根目录
docker compose up -d --build
# 默认发布 8787；首次访问会创建管理员。公开暴露前建议预置管理员或先在可信网络完成 setup。
```

`restart: unless-stopped` 已让它**开机自启 + 崩溃自拉**——装好就等于"自动开启 webui"。数据(SQLite、日志、bun 运行时缓存、agent 状态)落在命名卷 `nomifun-data:/data`。

查看启动是否就绪:

```bash
docker compose logs -f nomifun   # 看到 "nomifun-web: embedded backend + SPA on one port" 即 OK
```

### 加 TLS(公网必做)

`docker-compose.yml` 里有一段注释掉的 `caddy` 服务。启用步骤:
1. 编辑 `Caddyfile`,把 `your.domain.com` 换成你的域名;
2. 在 `nomifun` 的 `environment` 里加 `NOMIFUN_HTTPS: "true"`(让会话 cookie 带 `Secure`);
3. 把 `nomifun` 的 `ports: ["8787:8787"]` 改成 `expose: ["8787"]`(只让 Caddy 对外);
4. 取消 `caddy` 服务和 `caddy-*` 卷的注释;
5. `docker compose up -d`。

Caddy 会自动签 HTTPS 证书,WebSocket(`/ws`)自动透传,无需额外配置。应用自带登录,所以 **Caddy 不需要再配 basic_auth**。

---

## 方式二:原生二进制 + systemd(不想用 Docker)

需要一台 Linux 构建机(Windows 交叉编译这堆 C 依赖很痛,不建议;也可直接从 Docker 镜像里 `docker cp` 出二进制)。

### 1) 构建产物

```bash
bun install
bun run build:ui                      # → ui/dist(SPA,~21MB)
cargo build --release -p nomifun-web  # → target/release/nomifun-web
```

### 2) 部署布局

```
/opt/nomifun/nomifun-web      # 二进制
/opt/nomifun/web/             # = ui/dist 的内容
/var/lib/nomifun/             # data 目录(systemd StateDirectory 自动创建)
```

```bash
sudo useradd --system --home /var/lib/nomifun --shell /usr/sbin/nologin nomifun
sudo mkdir -p /opt/nomifun/web
sudo cp target/release/nomifun-web /opt/nomifun/
sudo cp -r ui/dist/.            /opt/nomifun/web/
```

> ⚠️ **bun ≥ 1.3.13 必须在「系统」PATH 上**。`nomifun` 是 nologin 账户,装到某人 `~/.bun/bin/` 里它读不到(systemd 服务也不会用你的 `~/.bun`)。可靠做法二选一:
> - **系统级安装**:`curl -fsSL https://bun.sh/install | bash`,然后 `sudo install ~/.bun/bin/bun /usr/local/bin/bun`(或用包管理器 / `npm i -g bun`);
> - **编进二进制**(免 PATH 依赖):构建时 `NOMIFUN_EMBED_BUN=1 cargo build --release -p nomifun-web`,bun 会嵌入并在 data 目录自解压(需构建环境能取到 bun)。
>
> 校验:`sudo -u nomifun -s -- which bun` 必须返回路径,否则 agent 首次 spawn 才暴露失败、错误隐晦。

### 3) 安装并开机自启

```bash
sudo cp packaging/linux/nomifun-web.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nomifun-web      # 开机自启 + 立即启动 = 自动开 webui
sudo systemctl status nomifun-web
```

同样建议在前面摆一个 Caddy/nginx 做 TLS,并在 unit 里设 `Environment=NOMIFUN_HTTPS=true`。

> ⚠️ unit 里 `Environment=NOMIFUN_DATA_DIR=/var/lib/nomifun` 必须与 `StateDirectory=nomifun` **保持一致**。若你改 unit(比如注释掉 `User`/`Group` 改用 root)时不慎删掉那行,数据会悄悄落到服务用户的按用户目录(`$XDG_DATA_HOME/NomiFun/Nomi`,通常是 `~nomifun/.local/share/NomiFun/Nomi`),与 systemd 托管的 `/var/lib/nomifun` 脱钩。

---

## 启动参数 / 环境变量

| 参数 | 环境变量 | 默认 | 说明 |
|---|---|---|---|
| `--host` | `NOMIFUN_WEB_HOST` | `127.0.0.1` | 监听地址。`0.0.0.0` 会对 LAN/VPN/公网可达；请先完成管理员设置或预置密码，公网必须放在 TLS 之后 |
| `--port` | `NOMIFUN_WEB_PORT` | `8787` | 同时服务 API、`/ws`、SPA 的端口 |
| `--data-dir` | `NOMIFUN_DATA_DIR` | 按用户目录 | 数据目录(db / 日志 / bun 缓存 / agent 状态)。默认是服务用户的按用户目录(Linux 为 `$XDG_DATA_HOME/NomiFun/Nomi`);生产请显式指定绝对路径 |
| `--dist` | `NOMIFUN_WEB_DIST` | `../../ui/dist` | SPA 静态目录。部署时**必须**显式指定 |
| — | `NOMIFUN_HTTPS` | `false` | 设 `true` 时会话/CSRF cookie 带 `Secure`(仅在 TLS 后用) |
| `--admin-user` | `NOMIFUN_ADMIN_USERNAME` | `admin` | 预置管理员用户名(仅预置时用) |
| `--admin-password` | `NOMIFUN_ADMIN_PASSWORD` | 无 | 设了就在启动时**预置**管理员、跳过交互首启 |
| `--insecure-no-auth` | `NOMIFUN_WEB_INSECURE_NO_AUTH` | `false` | ⚠️ 关闭认证(桌面式 no-auth)。只在仅 loopback / 受信私网可达时用 |

直接命令行示例:

```bash
nomifun-web --host 127.0.0.1 --port 8787 \
  --data-dir /var/lib/nomifun --dist /opt/nomifun/web
```

---

## Linux 运行时依赖

| 依赖 | 必需性 | 说明 |
|---|---|---|
| glibc + ca-certificates | 必需 | sqlite 静态链接、TLS 用 rustls,**无需 openssl / libsqlite** |
| `bun` ≥ 1.3.13 | **必需** | agent 执行;1.1.38 有 stdin bug。Docker 镜像里已含 |
| `node` / `npm` / `npx` | 推荐 | 用户启用的 MCP stdio server(`npx -y …`) |
| `git` | 推荐 | skill 发现、部分内置工具 |
| `ripgrep`(`rg`) | 推荐 | 代码搜索后端(缺则退化到 `grep`) |
| DISPLAY / X11 / WebView | ❌ 不需要 | `nomifun-web` 零 GUI 依赖,纯 headless |

---

## 安全须知

- **务必走 TLS**(公网):cookie 与登录凭据明文传输会被嗅探。TLS 后记得 `NOMIFUN_HTTPS=true`。
- **首启竞态**:从服务起来到你完成首启 setup 之间,理论上谁先访问谁就能占下管理员账号。要彻底关掉这个窗口,用 `NOMIFUN_ADMIN_PASSWORD` 预置;或先监听 `127.0.0.1` / 内网 / 隧道完成首启,再改为 `0.0.0.0`。
- **`--insecure-no-auth` 谨慎**:它把认证完全关掉(任何能连端口的人 = 特权用户,可执行 shell / 读写文件)。只在纯 loopback 或完全受信的私网用。
- 后端有终端 / 文件 / agent 能力,等价于"远程运维 = 远程代码执行(对你自己)"——这正是产品本意,但也因此**认证 + TLS 是底线**。
