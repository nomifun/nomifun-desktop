# NomiFun 桌面端自动更新说明

本文只说明自动更新链路。完整发版操作看根目录 `RELEASING.zh-CN.md`。

## 工作方式

应用内自动更新基于 Tauri 原生 updater：

```text
正在运行的 App
  -> 请求 apps/desktop/tauri.conf.json 里的 updater endpoint
  -> 优先请求 CrabNebula Cloud 动态更新接口
  -> CrabNebula 请求失败时检查 GitHub Releases latest.json
  -> 判断是否有更高版本
  -> 下载当前平台对应的更新包
  -> 非 GitHub 源下载或验签失败时，用 GitHub 重新确认同一版本并重试
  -> 用内置 pubkey 校验 .sig
  -> 安装并重启
```

下载与安装是两个严格分离的阶段：原生命令下载、验签并保留指定版本的更新包；安装命令只消费这个已下载包，不会检查更新，也不会在“安装”操作背后再次下载。渲染层只拥有更新检查权限，不能直接调用 updater 的原始下载或安装接口。

当前 endpoint 顺序：

```text
https://cdn.crabnebula.app/update/nomifun/nomifun-desktop/{{target}}-{{arch}}/{{current_version}}
https://github.com/nomifun/nomifun-desktop/releases/latest/download/latest.json
```

更新清单检查超时为 8 秒。安装包下载由 Rust 原生命令负责；如果 CrabNebula
返回了更新但安装包下载或验签失败，命令会固定使用 GitHub endpoint 重新检查
同一版本并下载。切换源时渲染层会清零进度，避免两次下载的字节数累加。

## 密钥区别

自动更新使用一把 Tauri updater 私钥：

```text
apps/desktop/signing/nomifun-updater.key
```

发版时把私钥内容写入环境变量：

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat apps/desktop/signing/nomifun-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
```

这把密钥只负责 updater 验签，不负责系统信任：

- macOS Gatekeeper 仍需要 Developer ID 签名和公证。
- Windows SmartScreen / 未知发布者仍需要 Authenticode 签名。
- 没有 OS 代码签名时，自动更新验签仍可工作，但手动安装体验不够可信。

## 构建自动更新产物

仓库内置了一个叠加配置 `apps/desktop/tauri.updater.conf.json`（内容是
`{"bundle":{"createUpdaterArtifacts":true}}`），用 `--config` 叠加它即可产出 `.sig`。
**务必传文件路径，不要内联 JSON**：Windows PowerShell 5.1 会剥掉内联 `--config '{...}'`
里的双引号、变成非法 JSON；文件路径没有引号，各平台都稳。

> 新构建机（如这台 Windows）构建前，需先把已被 gitignore 的私钥
> `apps/desktop/signing/nomifun-updater.key` 从密钥库拷过来，且它必须与 `tauri.conf.json`
> 内嵌的 `pubkey` 匹配（keyID `F3AA272E60AA7952`），否则已安装的客户端会拒绝更新。

macOS：

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat apps/desktop/signing/nomifun-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

bun run build:mac --config apps/desktop/tauri.updater.conf.json
bun run make:latest
```

Windows 无 Authenticode 签名：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content apps/desktop/signing/nomifun-updater.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""

bun run build:win --config apps/desktop/tauri.updater.conf.json
bun run make:latest
```

Windows 有 Authenticode 签名（`--signed` 注入证书指纹仍走内联 JSON，需在 pwsh 7+ 下运行）：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content apps/desktop/signing/nomifun-updater.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
$env:WINDOWS_CERTIFICATE_THUMBPRINT = "A1B2C3..."

bun run build:win --signed --config apps/desktop/tauri.updater.conf.json
bun run make:latest
```

Linux：

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat apps/desktop/signing/nomifun-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

bun run build:linux --config apps/desktop/tauri.updater.conf.json
bun run make:latest
```

Linux 会同时产出 `.AppImage`、`.deb`、`.rpm` 时，`latest.json` 的 updater
条目使用 `.AppImage`；`.deb` / `.rpm` 作为手动下载安装包上传到 Release。

## GitHub 兜底 latest.json

`bun run make:latest` 会扫描当前机器的 updater 产物和 `.sig`，把对应平台写入：

```text
apps/desktop/updater/latest.json
```

同一个版本如果分多台机器构建，需要把最新的 `latest.json` 带到下一台机器继续合并。最终上传到 GitHub Release 的 `latest.json` 必须包含所有已发布平台。CrabNebula 上传辅助脚本也会读取这份清单，并要求本地 `.sig` 内容与清单完全一致后才允许上传。

## CrabNebula Cloud 发布

先在 CrabNebula Cloud 创建应用和 API Key，把下面配置写入已被 gitignore 的
`apps/desktop/signing/.env.release`：

```dotenv
CN_API_KEY=<your_api_key>
CN_APP=nomifun/nomifun-desktop
```

不要在这个已有项目上直接运行 `cn bootstrap`：它可能生成新的 updater
密钥或覆盖配置，导致已经安装的客户端无法验证后续更新。

确定或 bump 版本号后，每个版本只创建一个隐藏草稿：

```bash
bun run release:cloud -- draft --version <version> --notes-file notes.md
```

记录返回的 release ID。在 Windows、macOS、Linux 构建机上完成构建和
`make:latest` 后，使用同一个 ID 上传本机产物：

```bash
bun run release:cloud -- upload --release-id <release-id>
```

所有计划发布的平台都上传后再公开发布并验证：

```bash
bun run release:cloud -- publish --release-id <release-id>
bun run release:cloud -- verify
```

`upload` 会显式传入 CrabNebula 的 public/update platform，不依赖 Tauri
目录自动发现；只有本地 updater 包旁的 `.sig` 与合并清单一致时才上传。

## GitHub Release 资产

macOS 需要同时上传：

```text
dist/desktop/NomiFun_<version>_universal.dmg
target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz
target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz.sig
apps/desktop/updater/latest.json
```

Windows 上传 `bun run make:latest` 打印的 updater 包、`.sig`、`latest.json`。如果还有额外手动安装包，例如 `.msi`，也上传。

如果 Release 已经存在，补平台时用：

```bash
gh release upload "v<version>" <new-assets...>
gh release upload "v<version>" apps/desktop/updater/latest.json --clobber
```

## 验证

```bash
gh release view "v<version>" --json tagName,assets,url
curl -fsSL https://github.com/nomifun/nomifun-desktop/releases/latest/download/latest.json
```

确认 `latest.json` 的版本、平台 key、URL 和 Release 资产一致。

CrabNebula 和 GitHub 必须发布相同版本、相同安装包与相同 `.sig`。首次包含
CrabNebula endpoint 的版本仍必须上传 GitHub：旧客户端内嵌的是 GitHub-only
配置，需要先通过 GitHub 更新一次，或者由用户从 CrabNebula 手动安装一次。
