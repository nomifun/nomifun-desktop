# 桌面自动更新

[English](README.md) · [发版流程](../../../RELEASING.zh-CN.md)

本页负责更新行为与产物完整性。构建、上传和发布步骤统一放在发版流程，不再维护第二份发布清单。

## 运行合同

- [tauri.conf.json](../tauri.conf.json) 管理有序 endpoint 与公钥：CrabNebula 优先，GitHub Releases 回退；更新元数据检查超时为八秒。
- [前端适配器](../../../ui/src/common/adapter/tauriUpdater.ts) 检查元数据。原生命令下载、验签并保留准确版本，安装只消费这些字节，不进行第二次下载。
- 非 GitHub 包下载或验签失败时，可从 GitHub 重试**同一版本**，不能静默选择其他版本。弹窗重新打开也以原生槽位持有的更新包为准。
- renderer 只拥有元数据检查与重启权限，不拥有 updater 插件原始下载/安装权限。[原生安装上下文](../src/updater_install_context.rs)和命令负责安装前置检查与失败恢复。
- macOS 上检查更新时同时读取原生安装上下文。若当前运行的 App 无法原地替换（磁盘映像或外置卷、App Translocation、与临时目录不在同一卷、位置不可读），界面会给出具体原因和运行路径，并提供手动下载，而不是下载一个无法安装的更新包。安装时仍执行同一前置检查并保持失败关闭。
- 安装器准入后的失败保留原生槽位，并报告手动恢复方式。已发布的坏归档必须重新生成签名产物与对应清单；仅修改应用源码不能修复已下载字节。

## 签名与产物

Updater 密钥与[系统代码签名](../signing/README.md)独立。已有私钥放在发布密钥库，
`TAURI_SIGNING_PRIVATE_KEY` 接收其**内容**，有密码时设置 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。
私钥必须匹配 `plugins.updater.pubkey`。轮换密钥不是普通发版步骤；丢失密钥会使已安装客户端无法信任后续更新包。

[更新器叠加配置](../tauri.updater.conf.json)启用 `createUpdaterArtifacts`，使用文件路径传入。
各平台命令见[构建与打包](../../../docs/contributing/building-and-packaging.zh.md)。包与相邻的 `.sig` 共同确定产物身份。

`bun run make:latest` 将本机产物合入 [latest.json](latest.json)，保留其他平台条目。
每个交付的系统/架构都需要准确的签名包条目。Linux 使用 AppImage 自动更新，DEB/RPM 保留为手动安装包。
CrabNebula 与 GitHub 必须发布相同版本、字节和签名；[云端发布助手](../../../scripts/crabnebula-release.mjs)
根据该清单核验上传输入，不要用初始化命令覆盖已有云端配置和签名身份。

## macOS 归档合同

从最终签名/公证 App 生成更新归档，设置 `COPYFILE_DISABLE=1`，不带扩展属性及 AppleDouble
条目（`._*`、`__MACOSX`）。所有条目只能属于单一 `NomiFun.app/` 根，因为安装会去掉第一层路径。
解压后的文件字节、权限和符号链接必须与最终 App 一致，包括原生库与签名。

[归档校验器](../../../scripts/lib/macos-app-bundle.mjs)使用 libarchive 的 `!mac-ext` 处理读取原始条目并检查解压布局。相关回归：

```bash
bun test ./scripts/lib/macos-app-bundle.test.mjs
```
