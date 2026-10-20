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

Linux 的 `build` 和 `build:linux` 在打包前构建并冻结对应架构的管理员 runner。
deb/rpm 将 runner 和 polkit action 安装到固定系统路径；AppImage 携带显式安装脚本，
挂载镜像本身不能安装宿主认证设施。`build:fast` 准备 debug runner 并打印手动安装命令；
开发启动不修改系统文件。安装、升级/卸载、原生认证要求和检查见
[Linux 系统执行组件说明](../../apps/desktop/linux-system-execution/README.md)。

## Computer worker

平台构建先运行 `scripts/computer/build-worker.mjs`，在独立工作区构建并从 Cargo compiler-artifact 取得真实 executable，复制为 `apps/desktop/binaries/nomifun-computer-worker-<target>[.exe]`。Tauri externalBin 将其打入应用；开发入口设置 `NOMIFUN_COMPUTER_WORKER`，macOS 开发 bundle 使用其已签名副本。

macOS 先签 worker 再封 App；Windows --signed 先用相同证书签并验证 worker，再交 Tauri。实际证书、公证、安装与 self-check 是单独的发布验收，不由构建脚本的存在证明。

Linux worker 的当前构建依赖为 X11、Xi、XTest、xkbcommon（Debian/Ubuntu：libx11-dev、libxi-dev、libxtst-dev、libxkbcommon-dev）；运行依赖为 libx11-6、libxi6、libxkbcommon0，RPM 使用 libX11、libXi、libxkbcommon。旧 Computer 的 GStreamer 预检和专用插件依赖已删除；Browser、音频、portal 与系统执行的既有依赖保留。AppImage 含 worker，但仍须在目标桌面验证动态库和实际 backend；原生 Wayland capture 未启用。

`bun run check:computer-boundary` 核对 vendor/补丁、冻结派生、旧引用零残留与主 workspace 不链接驱动。许可原文和 worker 依赖署名随现有 component-notices 资源一起打包。

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

发布 binary 内嵌 OfficeCLI。实际任务缺少必需的 Node/Bun 时，现役供应器自动准备
批准的组件，不要求额外下载确认；只读状态和后端启动不触发下载。Node 使用完整官方
Node/npm/npx 发行目录。命令解析可复用已验证缓存，后续新进程立即取得最新工具环境。
应用托管保留系统或用户配置的 Node。准入、缓存及离线行为见
[外部工具](../architecture/external-tools.zh.md)。

桌面安装包不内嵌 NomiRelay 的 `nfagent`；连接、恢复和重启在需要时自动准备批准的
连接组件，保留进度、真实失败、重试和取消。只读状态查询不准备组件。
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

App 的 [Info.plist](../../apps/desktop/Info.plist) 与
[entitlements.plist](../../apps/desktop/entitlements.plist) 是开发签名和正式签名共用的
宿主权限声明：音频输入、摄像头及 Apple Events 使用现有原生路径；本地网络带有用途说明。
它们不替用户批准 TCC，也不表示麦克风、摄像头或登录桌面必然存在。

管理员执行组件由正式 bundle 携带，并通过原生 ServiceManagement 注册；其程序、launchd
plist、Mach service 和签名身份必须一致。helper 与 App 使用同一 Apple 签发的团队身份，
不能用临时 ad-hoc 签名冒充管理员执行可用。系统进程提供管理员执行，Computer、Browser 和
媒体仍在原登录用户的桌面宿主运行。组件注册、升级和卸载必须验证系统原生状态；一般进程提权
不替代完全磁盘访问、辅助功能或网站设备许可。完整边界见
[系统许可与执行身份](../architecture/system-permissions.zh.md)。

`bun run build:updater` 使用已提交的
[updater overlay](../../apps/desktop/tauri.updater.conf.json)。
CrabNebula Cloud 是主分发源，GitHub Releases 是回退源。发布前阅读发布手册。

## 验证

运行相关打包测试，并在目标系统构建。冒烟检查启动、本地鉴权及受影响的运行时组件。
公开 macOS 产物验证 `codesign`、`spctl` 和 `xcrun stapler`；
Web 部署通过实际反向代理验证首次设置/登录、`/health` 和 WebSocket。
记录无法运行的平台检查。命令测试、签名检查和原生产品验收分别记录；不能把 Terminal
的许可、临时测试 App 或安装器退出码当成正式 NomiFun App/helper 全部就绪。首次授权、拒绝、
撤回、重新授权和必要重启后的任务继续应使用可回滚测试用户或隔离机器验证。
