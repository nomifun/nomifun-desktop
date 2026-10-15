# macOS 代码签名与公证

本目录负责仓库的 macOS 签名配置。完整构建与发版步骤见
[构建与打包](../../../docs/contributing/building-and-packaging.zh.md)和[发版流程](../../../RELEASING.zh-CN.md)。
[Updater 签名](../updater/README.zh-CN.md)使用独立密钥，不代替系统代码签名或公证。

## 本地配置

在 macOS 构建机复制无密钥模板，填写发布方自己的凭据：

```bash
cp apps/desktop/signing/.env.signing.example apps/desktop/signing/.env.signing
security find-identity -v -p codesigning
```

当前最终 App 签名流程要求钥匙串中已安装的 `APPLE_SIGNING_IDENTITY`，身份类型为
`Developer ID Application`。仅提供 `APPLE_CERTIFICATE` 不满足当前最终 App 的签名检查。
配置读取与公证探测见 [mac-signing.sh](../../../scripts/lib/mac-signing.sh)，最终产物流程见
[desktop-build-mac.sh](../../../scripts/desktop-build-mac.sh)。

| 配置 | 用途 |
| --- | --- |
| `APPLE_SIGNING_IDENTITY` | 已安装的 Developer ID Application 完整身份名称 |
| `APPLE_API_ISSUER`、`APPLE_API_KEY`、`APPLE_API_KEY_PATH` | App Store Connect API 公证凭据；路径可相对仓库根，脚本转换为绝对路径 |
| `APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID` | 未配置 API 三元组时使用的另一组公证凭据 |

`.env.signing`、`.p8`、证书私钥及 updater 私钥不入库，不复制到公开报告。
无密钥的 [.env.signing.example](.env.signing.example) 可以提交。
普通开发包不读本地签名配置；签名成功和公证成功分别记录，未配置公证时不宣称已公证。

## 构建与核对

```bash
bun run build:mac --signed
# Intel 独立构建：
bun run build:mac intel --signed
```

`bun run build:signed` 进入相同的最终 App 流程。脚本先处理最终 App 及原生库签名，
再按已配置的公证方式提交并 staple，之后生成发行产物。打包不携带浏览器 framework、helper
或系统 WebKit.framework，浏览器使用系统 WKWebView。

[entitlements.plist](../entitlements.plist) 是 App 的音频输入与相机资源声明，供正式包和开发包重新封装共用；
随附 ONNX 库不携带这些 App 资源声明。用途说明、系统用户授权和网页权限仍各自适用。

开发包 ad-hoc 签名、本地严格签名检查、Apple Silicon 实机运行、Intel 实机运行、Developer ID
和公证发行分别报告。不得由其中一项通过推导其他系统、机器或分发渠道全部可用。
