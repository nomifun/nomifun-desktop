# NomiFun 发布手册

[English](RELEASING.md)

本文描述维护者的发布顺序。本地构建见[构建与打包](docs/contributing/building-and-packaging.zh.md)；
签名与更新器细节由所属目录维护：
[签名](apps/desktop/signing/README.md)和
[Updater](apps/desktop/updater/README.zh-CN.md)。

## 准备

1. 使用经过审查的目标提交，保持工作树干净。
2. 更新 `CHANGELOG.md`，准备发布说明文件，对照发布行为核对文档和安装/部署指南。
3. 运行覆盖改动范围和目标平台打包的检查，确认许可证、署名及再分发权。
4. 从发布密钥库取出现有 updater 私钥，必须匹配
   [tauri.conf.json](apps/desktop/tauri.conf.json) 中的
   `plugins.updater.pubkey`。凭据保存在被忽略的
   `apps/desktop/signing/.env.release`，使用贡献者配置的 Git 身份。

破坏性数据变更必须在实际发给 GitHub 和 updater 的发布说明开头明确警告，
说明损失及升级前用户需要做什么。仅写 changelog 不够：
更新器展示 manifest 的 `notes`，回退生成会跳过 `Unreleased`。

版本真值是 `Cargo.toml [workspace.package].version`。
`bun run bump 1.2.3` 同步 lock/package 元数据；
`--tag` 还会提交并创建 `v1.2.3`。
发布工具负责这些步骤时，不要重复 bump/tag。

## 桌面发布工具

各工具在原生操作系统上运行，自动判断该版本的 GitHub Release 是否存在：
不存在时创建版本/tag/Release；已存在则追加平台资产并合并 `latest.json`。

| 平台 | 预览 | 创建版本 | 追加当前版本 |
| --- | --- | --- | --- |
| macOS | `bun run release:mac -DryRun` | `bun run release:mac -Version 1.2.3 -NotesFile notes.md` | `bun run release:mac` |
| Windows | `bun run release:win -DryRun -SkipPull` | `bun run release:win -Version 1.2.3 -NotesFile notes.md` | `bun run release:win` |
| Linux | `bun run release:linux -DryRun` | `bun run release:linux -Version 1.2.3 -NotesFile notes.md` | `bun run release:linux` |

首次创建必须提供发布说明，多行文本优先 `-NotesFile`。
macOS 默认 Apple Silicon，用 `-Architecture intel` 追加独立 Intel 包。
Windows 配置预期 Authenticode 签名；macOS 配置 Developer ID 和公证。

Windows 只读预览需同时传 `-DryRun -SkipPull`：仅 `-DryRun` 仍会在预览分支前
执行 `git pull --ff-only`。macOS/Linux 预览跳过 pull。
`-NoPush` **不是**通用离线/预览开关，
append 模式仍会上传资产。`-SkipPull` 跳过工具的 pull。
使用前阅读平台脚本：
[macOS](scripts/release-mac.sh)、[Windows](scripts/release-win.ps1)、
[Linux](scripts/release-linux.sh)。

## 分发与 manifest

CrabNebula Cloud 为主更新/下载源，GitHub Releases 为回退及归档。
两处保持同版本和逐字节相同的签名资产。
所有计划平台完成上传前，Cloud release 保持 draft。

```bash
bun run release:cloud -- draft --version 1.2.3 --notes-file notes.md
bun run release:cloud -- upload --release-id <release-id>
bun run release:cloud -- publish --release-id <release-id>
bun run release:cloud -- verify
```

按需在各平台执行 upload。手动构建时，
`bun run make:latest --collect` 扫描签名资产，合并平台条目到
`apps/desktop/updater/latest.json`，并收集至 `dist/desktop/`。
跨构建机传递与合并 manifest 时保留已有平台条目。

发布前确认：

- 计划支持的每个 OS/架构都有正确版本、URL 和签名。
- 手动安装包与 updater 包齐全。macOS 手动安装用 DMG、更新用
  `.app.tar.gz`；Linux updater 用 AppImage，deb/rpm 用于手动安装。
- Updater archive 来自最终签名/公证 App，且通过解包校验。
- 发布 URL 返回准确签名字节。从上一受支持版本验证检查、下载、安装及重启。
- 破坏性变更警告出现在 updater 的发布说明中。

## 更新 NomiRelay 连接组件

桌面安装包不内嵌 `nfagent`。恢复/重启复用有效本地文件；
下载缺失或变化的组件需要用户明确选择。
批准版本、不可变 URL 及完整 SHA-256 在
[nfagent-runtime.json](apps/desktop/nfagent-runtime.json)。

更新组件时：

1. 在 `nomifun-net-infra` 的目标干净提交构建，遵循该仓库当前验证/发布说明。
2. 发布不可变版本 Release，包含 manifest 的六个目标：
   `nfagent-{windows,darwin,linux}-{amd64,arm64}`
   （Windows 文件名以 `.exe` 结尾），另附 `SHA256SUMS`。
3. 同时更新 manifest 的版本、URL 和完整 hash。不得覆盖已发布 tag 下的字节；
   字节变化必须使用新版本/tag。
4. 确认所有 URL 公开可下载，不使用 `latest`、query 或 fragment。
   至少回下载当前平台资产，核对 hash 并运行 `-version`。
5. 完成后才分发引用新 manifest 的桌面版本。

## 服务端发布与后续检查

按维护中的[部署指南](docs/guides/web-server-deployment.zh.md)构建/推送 Web 镜像。
发布对应版本 tag，需要可复现时使用明确 tag/digest。

发布后验证下载链接、首次设置、登录和 WebSocket；
在每个发布平台从旧安装检查更新。记录已知限制，只宣布实际可用的产物。
