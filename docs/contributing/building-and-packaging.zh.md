# 构建与打包

本页说明当前 **NomiFun** monorepo 能产出的发布物：React SPA、`nomifun-web`、
Tauri 桌面包、updater 产物、Docker 镜像和 Linux systemd 部署文件。

日常开发循环见 [`development.zh.md`](development.zh.md)。部署运行见
[`../guides/web-server-deployment.zh.md`](../guides/web-server-deployment.zh.md)。

## 当前状态

| 产物 | 当前状态 |
| --- | --- |
| SPA (`ui/dist`) | `bun run build:ui` 构建；桌面和 Web host 都使用它。 |
| `nomifun-web` | 支持的自托管 binary；默认开启鉴权。 |
| Tauri 桌面包 | `bun run build` 为当前 OS 构建。 |
| macOS Developer ID 签名 + 公证 | 已有 `bun run build:signed` 包装脚本；需要本机 Apple 签名配置。 |
| Tauri updater 产物 | `bun run build:updater` 会生成 updater `.sig`；生产 endpoint/key 管理仍需发布配置。 |
| Docker / Compose | 支持本地构建与 compose 运行；本文不承诺公开 registry 镜像。 |
| Native Linux + systemd | `packaging/linux/` 提供 unit 和说明。 |
| Windows 签名 | 需要外部代码签名证书；仓库内未配置。 |

## SPA

```bash
bun run build:ui
```

输出目录是 `ui/dist/`。

桌面构建通过 `apps/desktop/tauri.conf.json` 的 `frontendDist` 打包该目录。
`nomifun-web` 通过 `--dist` / `NOMIFUN_WEB_DIST` 服务它；从仓库内运行时，
默认路径相对 `apps/web` 指向 `../../ui/dist`。

## Web Binary

```bash
bun run build:ui
cargo build --release -p nomifun-web
```

运行要求：

- 已构建的 SPA 目录；
- 可写数据目录；
- `PATH` 上有 Bun，除非构建时使用 `NOMIFUN_EMBED_BUN=1`；
- 默认鉴权/admin 初始化流程，或仅在可信本地开发中显式使用 `--insecure-no-auth`。

示例：

```bash
target/release/nomifun-web --host 127.0.0.1 --port 8787 --dist ui/dist
```

未预置 `NOMIFUN_ADMIN_USERNAME` / `NOMIFUN_ADMIN_PASSWORD` 时，首次浏览器访问会创建管理员。

## 桌面包

```bash
bun run build
```

该命令调用 Tauri build，先构建 SPA，再在 `target/release/bundle/` 下生成当前
OS 的安装包/应用包。

产品身份来自 `apps/desktop/tauri.conf.json`：

- `productName: "NomiFun"`
- `identifier: "com.nomifun.desktop"`
- 版本来自 workspace package metadata
- dev URL `http://localhost:5173`
- bundled frontend `../../ui/dist`

桌面包应在目标 OS 上构建。跨 OS 桌面打包不是当前支持流程。

## macOS 签名与公证

ad-hoc 签名产物只适合本地测试，不适合发给别人。生成 Developer ID 签名并公证的 DMG：

```bash
cp apps/desktop/signing/.env.signing.example apps/desktop/signing/.env.signing
# 填写本机 Apple 签名/公证信息
bun run build:signed
```

真实 `.env.signing` 与 Apple 私钥不会入库。包装脚本在
[`scripts/desktop-build-signed.sh`](../../scripts/desktop-build-signed.sh)，详细配置见
[`apps/desktop/signing/README.md`](../../apps/desktop/signing/README.md)。

## Updater 产物

```bash
bun run build:updater
```

该命令启用 Tauri `createUpdaterArtifacts`，在安装包旁生成 `.sig`。这些签名只给
Tauri updater 使用，不等于 OS 信任：macOS 仍需要 Developer ID 签名/公证；
Windows 仍需要代码签名证书。

生产发布仍需补齐：

- 生产 updater 密钥管理；
- 托管 `latest.json` endpoint；
- 发布 channel 策略；
- renderer 中下载、应用、重启的完整流程。

见 [`apps/desktop/updater/README.md`](../../apps/desktop/updater/README.md)。

## Docker

```bash
docker compose up -d --build
```

根 `Dockerfile` 用 Bun 构建 SPA，用 Cargo 构建 release `nomifun-web`，再把 binary
和 `ui/dist` 复制到 slim runtime image。Compose 启动一个 `nomifun` 服务，
端口 `8787`，`/data` 作为 `NOMIFUN_DATA_DIR`。

启动后访问 `http://<server>:8787`。如果没有预置管理员，第一个能访问到的浏览器
会看到首次管理员设置页。

`docker-compose.yml` 中的 Caddy 服务默认注释。需要 TLS 时可启用它或使用其他反向代理；
浏览器通过 HTTPS 访问时设置 `NOMIFUN_HTTPS=true`。

## Native Linux + systemd

见 [`packaging/linux/README.md`](../../packaging/linux/README.md)。基本形态：

```bash
bun install
bun run build:ui
cargo build --release -p nomifun-web
sudo cp target/release/nomifun-web /opt/nomifun/
sudo cp -r ui/dist/. /opt/nomifun/web/
sudo cp packaging/linux/nomifun-web.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nomifun-web
```

systemd 环境中，如果 agent 子进程需要 shell，请显式设置 `SHELL`；nologin 服务用户
通常没有可用 shell。

## 分发前检查

- `cargo check --workspace`
- `bun run build:ui`
- 桌面包在目标 OS 上构建并 smoke test 启动。
- macOS 分发前验证 `codesign`、`spctl`、`xcrun stapler`。
- Web/Docker 验证首次管理员设置、登录、`/health` 和目标反向代理下的 WebSocket。
