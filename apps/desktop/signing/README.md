# NomiFun 桌面 macOS 代码签名 + 公证(Gatekeeper)

> 解决「把安装包发给别人,对方打开提示**已损坏,无法打开**」的问题。
>
> 这跟 `updater/`(自动更新签名)是**两套完全不同的密钥**,别混。本目录只管
> Apple 的 **Developer ID 签名 + 公证(notarization)**,让 App 在任何 Mac 上双击即开。

## 发布责任边界

本仓库只提供签名脚本和无密钥模板。正式发布必须使用发布方自己的 Apple
Developer 账号、Developer ID 证书、App Store Connect API Key，以及独立的
自动更新签名密钥。不要把 fork、本地开发机或历史测试密钥当成官方发布凭据。

## 为什么会「已损坏」

默认 `bun run build` 产出的 App 只是 **ad-hoc 签名**(`Signature=adhoc`,无
`TeamIdentifier`)。别人下载/传输后,文件被打上 `com.apple.quarantine` 隔离标记;在
Apple 芯片 Mac 上,被隔离 + 未正规签名公证的 App,Gatekeeper 直接判为「已损坏」。

**根治办法只有一个**:用 **Developer ID Application** 证书签名 → 提交 Apple **公证** →
**staple** 把公证票据钉进 App。之后任何人下载双击即开,无任何提示。

## 密钥绝不入库(本仓库的约定)

| 东西 | 放哪 | 是否入库 |
|---|---|---|
| 模板 `.env.signing.example` | 本目录 | ✅ 入库(无密钥) |
| 真实 `.env.signing`(身份名 / Key ID / 路径) | 本目录 | ❌ 已 gitignore |
| App Store Connect API Key `AuthKey_*.p8` | `apps/desktop/.tauri/` 或仓库外 | ❌ 已 gitignore |
| Developer ID 证书私钥 | macOS **登录钥匙串**(不是文件) | ❌ 不在仓库里 |

构建脚本 `scripts/desktop-build-signed.sh`(可入库,无密钥)在运行时 `source` 本地
`.env.signing` 注入环境变量,Tauri 据此签名 + 公证。

---

## 一次性准备(在 Apple 侧)

### 1. 生成 Developer ID Application 证书并装进钥匙串
- 最简单:用 **Xcode**(Settings → Accounts → 选中团队 → Manage Certificates → `+` →
  **Developer ID Application**),它会自动装进登录钥匙串。
- 或 developer.apple.com → Certificates → `+` → **Developer ID Application** → 按引导用
  CSR 生成 → 下载 `.cer` 双击导入钥匙串。
- 验证已就位:
  ```bash
  security find-identity -v -p codesigning
  # 应能看到:  "Developer ID Application: Your Name (TEAMID1234)"
  ```
  把引号里的**全名**填到 `.env.signing` 的 `APPLE_SIGNING_IDENTITY`。

### 2. 生成 App Store Connect API Key(用于公证,推荐)
- App Store Connect → **Users and Access** → **Integrations** → **Keys** → 生成一个
  **Developer** 角色的 Key。
- 下载 `AuthKey_XXXX.p8`(**只能下载一次**),放到 `apps/desktop/.tauri/`(已 gitignore)
  或仓库外的安全目录。
- 记下两个值填进 `.env.signing`:
  - **Issuer ID** = keys 表格**上方**那串 UUID → `APPLE_API_ISSUER`
  - **Key ID** = 表格 "Key ID" 列 → `APPLE_API_KEY`
  - `.p8` 路径 → `APPLE_API_KEY_PATH`

> 不想用 API Key 也可用 Apple ID 方式:`APPLE_ID` + `APPLE_PASSWORD`(App 专用密码,
> 在 appleid.apple.com 生成)+ `APPLE_TEAM_ID`。三选一组,二者填其一即可。

---

## 本地配置 + 构建

```bash
# 1. 复制模板(真实文件不入库)
cp apps/desktop/signing/.env.signing.example apps/desktop/signing/.env.signing

# 2. 按上面拿到的值填写 .env.signing,并把 AuthKey_*.p8 放到对应路径

# 3. 出带签名 + 公证的安装包(公证联网,首次几分钟,耐心等)
bun run build:signed
```

产物在 `target/release/bundle/{macos,dmg}/`。构建末尾会先由 Tauri 公证并 staple
`.app`,随后脚本会对最终分发用的 `.dmg` 再提交一次公证并 staple。

## 验证(发出去前自检)

```bash
APP=target/release/bundle/macos/NomiFun.app
DMG=target/release/bundle/dmg/NomiFun_0.1.0_aarch64.dmg

codesign -dvv "$APP"                       # 期望: Authority=Developer ID Application: ...
codesign --verify --deep --strict -v "$APP"  # 期望: valid on disk / satisfies Designated Requirement
xcrun stapler validate "$APP"              # 期望: The validate action worked!
spctl -a -vvv "$APP"                       # 期望: source=Notarized Developer ID  → accepted

codesign --verify --strict -v "$DMG"        # 期望: valid on disk / satisfies Designated Requirement
xcrun stapler validate "$DMG"              # 期望: The validate action worked!
spctl -a -vvv -t open --context context:primary-signature "$DMG"  # 期望: accepted
```

这些验证全过,就可以放心分发 DMG——别人下载双击即开,不再报「已损坏」。

## 常见报错

- **`The binary is not signed with a valid Developer ID certificate`**:钥匙串里没有
  Developer ID Application 证书,或 `APPLE_SIGNING_IDENTITY` 名字写错。重看准备步骤 1。
- **`ambiguous (matches ... login.keychain-db and ... System.keychain)`**:同名
  Developer ID Application 证书同时存在于多个钥匙串。删除多余副本,或把
  `security find-identity -v -p codesigning` 输出中的 SHA-1 哈希填入
  `APPLE_SIGNING_IDENTITY`。
- **`APPLE_API_KEY_PATH 必须指向 AuthKey_*.p8`**:`APPLE_API_KEY_PATH` 是 App Store
  Connect API Key 路径,不要填 Developer ID `.p12` 证书路径。
- **公证被拒 / `Invalid` 状态**:多为「未启用 hardened runtime」或缺 entitlements。
  Tauri 用 Developer ID 签名时默认开启 hardened runtime;若 App 需要特殊能力(JIT、
  加载第三方动态库等),在 `tauri.conf.json` 的 `bundle.macOS.entitlements` 指定 plist。
  查看具体原因:`xcrun notarytool log <submission-id> --key ... --key-id ... --issuer ...`。
- **只签名没公证**:别人会看到「无法验证开发者」(不是「已损坏」)。补上公证变量即可。
