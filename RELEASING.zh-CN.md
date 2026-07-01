# NomiFun 发版手册

本文面向实际发版操作。需要英文维护说明时看 `RELEASING.md`；日常发桌面版优先按本文执行。

## 核心概念

桌面发版有两类产物：

- **手动安装包**：用户从 GitHub Releases 下载后自己安装，例如 macOS `.dmg`、Windows `.exe` / `.msi`、Linux `.AppImage` / `.deb` / `.rpm`。
- **自动更新产物**：Tauri updater 使用的包、对应 `.sig` 签名、以及合并后的 `latest.json`。

自动更新签名和系统代码签名不是一回事：

- `TAURI_SIGNING_PRIVATE_KEY` 只用于 Tauri updater 验签，证明自动更新包没有被篡改。
- macOS Developer ID / 公证、Windows Authenticode 用于系统信任，影响 Gatekeeper、SmartScreen、未知发布者提示。
- Windows 当前没有 Authenticode 签名时，自动更新验签仍可工作，但手动安装包会有未知发布者 / SmartScreen 风险。

## 版本号

版本号的单一真源是根目录 `Cargo.toml` 的 `[workspace.package].version`。发版前只需要跑：

```bash
VERSION=1.2.3
bun run bump "$VERSION"
```

脚本会同步：

- `Cargo.toml`
- `Cargo.lock`
- 根 `package.json`
- `ui/package.json`

tag 统一使用 `vX.Y.Z`，例如 `v0.1.11`。

## macOS 发版

在 Mac 上执行。下面命令会同时产出：

- 手动安装包：`dist/desktop/NomiFun_<version>_universal.dmg`
- 自动更新包：`target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz`
- 自动更新签名：`target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz.sig`

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat apps/desktop/signing/nomifun-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

bun run build:mac --config apps/desktop/tauri.updater.conf.json
bun run make:latest
```

如果是公开分发，建议配置 `apps/desktop/signing/.env.signing` 后使用 Developer ID 签名和公证：

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat apps/desktop/signing/nomifun-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

bun run build:mac --signed --config apps/desktop/tauri.updater.conf.json
bun run make:latest
```

`bun run make:latest` 会把 macOS 的 `darwin-x86_64` 和 `darwin-aarch64` 都写入 `apps/desktop/updater/latest.json`。

## Windows 发版

必须在 Windows 机器上执行。前提是 macOS 侧已经建好该版本的 GitHub Release（含 macOS 资产
和 `latest.json` 的 darwin 条目）——本节负责在 Windows 上把 Windows 产物「补」进去。

### 一键发版（推荐）

一次性配置好后即可反复一键：

1. 从密钥库拷入 updater 私钥 `apps/desktop/signing/nomifun-updater.key`（keyID
   `F3AA272E60AA7952`，已被 gitignore，必须与 `tauri.conf.json` 内嵌 `pubkey` 匹配）。
2. 复制 `apps/desktop/signing/.env.release.example` 为 `.env.release`（已被 gitignore），
   填入 `GH_TOKEN=...`（`repo` 权限的经典 PAT，或对本仓库 Contents:read/write 的细粒度 PAT）。

然后：

```powershell
git pull
bun run release:win            # pull→校验 Release→清旧产物→构建→合并 latest.json→上传+clobber→提交推送→校验
bun run release:win -DryRun    # 只跑只读前置检查并打印计划，不构建/不上传/不推送
bun run release:win -NoPush    # 构建+上传，但不提交/推送 latest.json（改动留本地）
```

脚本自动完成：读取 `.env.release` 里的 `GH_TOKEN` 并注入 `gh`、加载签名私钥、校验
`v<version>` Release 已存在、清理旧版本残留 NSIS 产物、构建更新产物、`make:latest` 合并
windows 条目、`gh release upload --clobber`、以作者 `nomifun` 提交 `latest.json` 回 `main`、
拉取 updater 端点校验。失败会明确报错并中断。

> 未启用 Authenticode 时，手动安装包仍可能触发 SmartScreen / 未知发布者提示；自动更新验签
> 走 Tauri updater 的 minisign 签名，与 Authenticode 无关，一键脚本已覆盖。

### 手动分步（等价于一键脚本内部流程）

先拉到与当前 Release 一致的代码：

```powershell
git pull
git checkout main
```

如果是在已经发布 macOS 后补 Windows，确认 `Cargo.toml` 版本号与现有 GitHub Release 一致。

> updater 私钥 `apps/desktop/signing/nomifun-updater.key` 已被 gitignore、只存在于密钥库。
> 这台 Windows 构建前需先从密钥库把它拷过来，且必须与 `tauri.conf.json` 内嵌的 `pubkey`
> 匹配（keyID `F3AA272E60AA7952`），否则已安装的客户端会拒绝更新。叠加 `createUpdaterArtifacts`
> 用的是仓库内的 `apps/desktop/tauri.updater.conf.json`，以 `--config <文件路径>` 传入——
> **不要内联 JSON**，PowerShell 5.1 会把内联 `--config '{...}'` 的双引号剥掉变成非法 JSON。

### 当前无 Authenticode 签名的做法

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content apps/desktop/signing/nomifun-updater.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""

bun run build:win --config apps/desktop/tauri.updater.conf.json
bun run make:latest
```

这会生成 Windows 自动更新产物和 `.sig`。这种模式下：

- 自动更新验签可以工作。
- 手动安装包不是系统代码签名包，用户可能看到 SmartScreen / 未知发布者提示。
- 适合内部测试或临时发布，不等同于公开可信 Windows 安装包。

### 以后补 Authenticode 签名

拿到 Windows 代码签名证书后，先把证书导入当前用户证书库，再设置证书指纹（`--signed`
注入指纹仍走内联 JSON，需在 pwsh 7+ 下运行）：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content apps/desktop/signing/nomifun-updater.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
$env:WINDOWS_CERTIFICATE_THUMBPRINT = "A1B2C3..."

bun run build:win --signed --config apps/desktop/tauri.updater.conf.json
bun run make:latest
```

这才是更接近 macOS Developer ID 签名 / 公证的公开分发状态。

## Linux 发版

如果发布 Linux，必须在 Linux 机器上执行：

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat apps/desktop/signing/nomifun-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

bun run build:linux --config apps/desktop/tauri.updater.conf.json
bun run make:latest
```

Linux 不走 macOS 公证或 Windows Authenticode，但仍需要 Tauri updater `.sig`。

## 合并 latest.json

`bun run make:latest` 的逻辑是：

- 扫描当前机器生成的 updater 产物和 `.sig`。
- 写入当前平台对应的 `platforms[...]` 条目。
- 保留已有的真实平台条目。

因此多平台发版时，要让 `apps/desktop/updater/latest.json` 在各平台之间传递或提交回仓库。缺失某个平台条目时，该平台用户不会收到自动更新。

典型顺序：

1. Mac 生成 macOS 条目。
2. Windows 拉取包含 macOS 条目的 `latest.json`。
3. Windows 跑 `bun run make:latest` 后补 Windows 条目。
4. 把最终 `latest.json` 上传到 GitHub Release，并提交回 `main`。

## 创建 GitHub Release

如果是首次创建某个版本：

```bash
git add Cargo.toml Cargo.lock package.json ui/package.json apps/desktop/updater/latest.json
git commit -m "chore(release): v$VERSION"
git tag "v$VERSION"
git push origin main "v$VERSION"

gh release create "v$VERSION" \
  target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz \
  target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz.sig \
  dist/desktop/NomiFun_${VERSION}_universal.dmg \
  apps/desktop/updater/latest.json \
  --title "v$VERSION" \
  --notes "发布说明"
```

如果 Release 已存在，只是补传 Windows 或 Linux 资产：

```bash
gh release upload "v$VERSION" <new-assets...>
gh release upload "v$VERSION" apps/desktop/updater/latest.json --clobber
```

`--clobber` 用于替换已有的 `latest.json`，确保 GitHub Release 上的清单包含最新平台条目。

## 上传哪些文件

macOS 至少上传：

```text
dist/desktop/NomiFun_<version>_universal.dmg
target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz
target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz.sig
apps/desktop/updater/latest.json
```

Windows 上传 `bun run make:latest` 打印的 updater 包、`.sig`、`latest.json`。如果 `dist/desktop/` 里还有未包含的手动安装包，例如 `.msi`，也一起上传。

Linux 上传对应安装包、`.sig`、`latest.json`。

## 发布后验证

```bash
gh release view "v$VERSION" --json tagName,assets,url
curl -fsSL https://github.com/nomifun/nomifun-tauri/releases/latest/download/latest.json
```

确认：

- GitHub Release 资产里包含手动安装包、updater 包、`.sig`、`latest.json`。
- `latest.json` 的 `version` 等于本次版本。
- 每个已发布平台都有 `platforms[...]` 条目。
- 每个 URL 都指向同一个 `v$VERSION` Release。

## v0.1.11 当前状态

`v0.1.11` 已完成 macOS：

- 已上传 `NomiFun_0.1.11_universal.dmg`。
- 已上传 `NomiFun.app.tar.gz` 和 `NomiFun.app.tar.gz.sig`。
- `latest.json` 目前只有 `darwin-x86_64` 和 `darwin-aarch64`。

Windows 还需要在 Windows 机器上继续构建、上传 Windows 资产，并用 `--clobber` 替换 Release 上的 `latest.json`。
