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
| Docker / Compose | 已提供官方 Docker Hub 镜像 [`nomifun/nomifun-web`](https://hub.docker.com/repository/docker/nomifun/nomifun-web)；本地镜像与 compose 构建仍受支持。 |
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

## `nfagent` 按需运行时

桌面安装包**不内置** `nfagent`。用户第一次使用 NomiRelay 配对、恢复或重启
Relay agent 时，桌面端才按当前 OS/架构从
[`apps/desktop/nfagent-runtime.json`](../../apps/desktop/nfagent-runtime.json)
指定的不可变 HTTPS URL 下载对应二进制。这样普通用户不使用 NomiRelay 时，
安装包不会额外携带约 6–7 MiB 的 agent。

受管理的运行时安装具有以下约束：

- 每次使用前都校验完整 SHA-256；缓存损坏时会删除并重新下载。
- 下载体限制为 32 MiB，先写入临时文件，校验成功后再原子发布。
- 缓存位置为
  `<data-dir>/runtime/nfagent/<version>/<sha256-prefix>/<asset-name>`，
  后续启动复用同一份已验证文件。
- 首次配对会先完成运行时下载和校验，再消费一次性 Relay 邀请，避免网络下载失败
  使邀请白白失效。
- 运行时 URL 不允许使用 `latest`、查询参数或 fragment；发布文件名必须与目标平台
  的固定名称一致。

本地开发或离线调试可以在启动桌面应用前设置：

```text
NOMIFUN_NFAGENT_PATH=<已存在的 nfagent 绝对路径>
```

该覆盖路径被视为开发者明确提供的可信文件，不走受管理下载及清单 SHA-256 校验，
因此不要把它作为普通用户的安装方式。

更新 `nfagent` 时，必须先在 `nomifun-net-infra` 发布新的、不可变的版本化资产，
再更新 `nfagent-runtime.json` 中的版本、URL 和 SHA-256。所有清单 URL 真正可下载
之前，不得分发引用它们的桌面安装包。具体手动上传流程见根目录
[`RELEASING.zh-CN.md`](../../RELEASING.zh-CN.md)。

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

已发布的运行时镜像：
[`nomifun/nomifun-web`](https://hub.docker.com/repository/docker/nomifun/nomifun-web)。
下面的命令会从当前 checkout 本地构建镜像。

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
