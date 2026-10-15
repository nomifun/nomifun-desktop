# 构建与打包

[English](building-and-packaging.md)

本地开发看[开发](development.zh.md)，公开发布操作看
[RELEASING.zh-CN.md](../../RELEASING.zh-CN.md)。桌面包在目标操作系统构建。

## 构建目标

| 目标 | 命令 | 输出 |
| --- | --- | --- |
| 共享 SPA | `bun run build:ui` | `ui/dist/` |
| Web 服务 | `cargo build --release -p nomifun-web` | `target/release/nomifun-web` |
| 桌面 | `bun run build` | `target/` 下的平台包 |
| macOS arm64 / Intel | `bun run build:mac` / `bun run build:mac intel` | 原生架构包 |
| Windows | `bun run build:win` | NSIS/MSI 产物 |
| Linux | `bun run build:linux` | AppImage/deb/rpm 产物 |
| Updater | `bun run build:updater` | 可更新包与 `.sig` |

[tauri.conf.json](../../apps/desktop/tauri.conf.json) 持有产品标识、
平台要求和 `frontendDist`；workspace 版本来自
[Cargo.toml](../../Cargo.toml)。

## Web 部署产物

运行未内嵌 SPA 的 Web 宿主前先构建前端：

```bash
bun run build:ui
cargo build --release -p nomifun-web
target/release/nomifun-web --host 127.0.0.1 --port 8787 --dist ui/dist
```

宿主需要可写数据目录，默认开启鉴权。没有预置凭据时，首次设置创建 owner。
容器可用 `docker compose up -d --build` 或已发布镜像。
Docker、反向代理、volume 和原生 systemd 操作集中在
[Web 服务部署](../guides/web-server-deployment.zh.md)和
[packaging/linux](../../packaging/linux/README.md)。

桌面包与其生产 WebUI 共用同一份内嵌前端。显式设置
`NOMIFUN_WEBUI_DIST` 时必须匹配构建身份。

## 桌面运行时组件

发布 binary 内嵌 OfficeCLI。Node/Bun 属于明确按需准备的受管组件；
后端启动不下载它们。命令解析可复用已验证缓存。
应用托管使用系统或用户配置的 Node。准入、缓存及离线行为见
[外部工具](../architecture/external-tools.zh.md)。

桌面安装包不内嵌 NomiRelay 的 `nfagent`，恢复连接和重启只复用批准的本地缓存。
缺失或版本变化时，用户明确选择下载连接组件。
每个平台固定的不可变 URL 和 SHA-256 在
[nfagent-runtime.json](../../apps/desktop/nfagent-runtime.json)。
下载有大小上限，先写暂存文件，验证后原子发布。

离线诊断可用 `NOMIFUN_NFAGENT_PATH` 指向现有绝对路径；
仍须匹配当前平台批准的 SHA-256。分发引用新 manifest 的桌面包前，
先发布所有对应资产，更新步骤见 [RELEASING.zh-CN.md](../../RELEASING.zh-CN.md)。

## 签名与更新包

Updater 签名与操作系统代码签名用途不同。使用发布密钥库中的现有
updater 私钥，不为已有安装生成替代密钥。

macOS Developer ID 签名与公证：

```bash
cp apps/desktop/signing/.env.signing.example apps/desktop/signing/.env.signing
bun run build:signed
```

构建前填写本机签名值，真实环境文件和密钥不提交。
密钥加载、Windows Authenticode、包校验和 manifest 生成见
[签名设置](../../apps/desktop/signing/README.md)与
[Updater 操作](../../apps/desktop/updater/README.md)。

macOS 使用系统 WKWebView，支持原生 arm64 和 Intel 包，拒绝 Universal。
内嵌 Browser 目标为 macOS 14+。最终 App 签名、公证完成后才生成 DMG/updater 字节。
包校验检查解包字节、权限和符号链接，并拒绝打包浏览器 runtime。
Intel ONNX 资源和许可证仍为必要项。

`bun run build:updater` 使用已提交的
[updater overlay](../../apps/desktop/tauri.updater.conf.json)。
CrabNebula Cloud 是主分发源，GitHub Releases 是回退源。发布前阅读发布手册。

## 验证

运行相关打包测试，并在目标系统构建。冒烟检查启动、本地鉴权及受影响的运行时组件。
公开 macOS 产物验证 `codesign`、`spctl` 和 `xcrun stapler`；
Web 部署通过实际反向代理验证首次设置/登录、`/health` 和 WebSocket。
记录无法运行的平台检查。