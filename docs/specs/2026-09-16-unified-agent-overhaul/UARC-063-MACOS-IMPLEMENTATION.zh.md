# UARC-063 macOS 完整产品与 arm64 制品验收

## 1. 结论

UARC-063 的核心交付已完成：当前 Apple Silicon 源码生成了包含固定 CEF framework 与五个 helper 的
Developer ID arm64 `.app`，并由该 App 生成、签名、公证和 staple 了 arm64 DMG。release lock、DMG
挂载后 App/CEF 同一性、两种全新数据目录启动、外部受保护 API 拒绝和进程树清理均通过。

本轮明确不把用户已移出完成门槛的 Keychain 持久化、attached Chrome Remote Debugging、Screen Recording
granted 路径和真实 IME composition 重新列为 blocker；它们记录为 `not_run`，不删除产品已有能力。
UARC-064 与 UARC-070 保持 pending。

## 2. 精确坐标

```text
branch:                    rf/agent-capability-platform-v2
UARC-061 implementation:   a39e2bfee807ec1fe1cea0cea957663a66c03558
UARC-061 fix:              686735489
UARC-062 implementation:   1c37ddd8059e2b1ae1c0419b2c37fd3908cbce23
UARC-062 fix:              750b8f5c66803df8117fb901388edb4f88c13243
live model contract fix:   6c093d900521eed5db061a270c7fb7c86c679912
UARC-063 artifact source:  6ab013f68bfcdae46ffaeecf471bd4933ed34a67
platform:                  macOS 26.3 (25D125), Apple M4 arm64
CEF / Chromium:            152.0.6 / 152.0.7977.83
```

## 3. 实现

- `scripts/desktop-build-mac.sh` 默认并只允许当前受支持的 arm64 产品目标；先构建 `.app`，再构建固定
  `nomifun-browser-cef-helper`、装配 CEF、完成 nested signing，最后从该 App 生成 DMG。
- DMG 在 Apple 公证前单独使用 Developer ID 签名；公证成功后 staple，再生成 release lock。
- `check-macos-arm64-native.mjs` 新增 App 与挂载 DMG 内的 CEF metadata、framework、五个 helper、arm64
  架构、nested signatures 和同一性检查；同时验证 DMG 签名、公证票据、产品启动和进程清理。
- 产品启动探针遵循 Tauri Desktop 的真实契约：通过 `NOMIFUN_DATA_DIR`/`NOMIFUN_WORK_DIR` 启动并读取
  `port.json`，而不是把 Desktop 误当旧 CLI host。外部 `/api/capabilities` 必须保持 403；catalog 由产品
  WebView 的 per-boot local-trust secret 访问。

## 4. 核心证据

| 项目 | 结果 | 证据 |
| --- | --- | --- |
| Browser/CEF | pass | 34/34 native（含真实 renderer crash 投影）、正式 Agent/Kernel/CEF、真实 StepFun `[2,1]` trusted witness、Command-Q 后 host/helper=0 |
| Process/PTY/Computer | pass | Process 240、Terminal 146、Computer 92；physical modifiers、Terminal UTF-8/resize、Command-Q cleanup |
| arm64 App | pass | host、CEF framework、五个 helper 均为 arm64；Developer ID deep/strict seal |
| arm64 DMG | pass | HFS+ image verify、挂载 App/CEF 与 staged App 哈希一致、Applications link 正确 |
| DMG Developer ID | pass | `codesign --verify --strict` |
| Apple notarization | pass | final submission `ec5f0545-61fc-4bbd-a6ca-ca843cc77a3c` accepted；staple validate passed |
| release lock | pass | source、host、package、LICENSE、NOTICE 全部哈希一致 |
| product startup | pass | absent root 与 pre-created empty root 均 `/health` 200；外部 protected API 403；两次 process cleanup 均无残留 |
| packaging/validator tests | pass | 11 passed / 0 failed；self-test passed；`build:mac --check arm` passed |

权威报告：

- `build.noindex/uarc063-delivery/uarc-macos-report.json`
- `build.noindex/uarc063-delivery/check-macos-arm64-native.log`
- `build.noindex/uarc063-delivery/build-mac-signed.log`
- `build.noindex/uarc063-delivery/dmg-sign-notarize.log`
- `build.noindex/uarc063-cef-crash/run-A0upQe/native-result.json`
- `build.noindex/uarc-live-model-scope/browser-live-product-v5/fixture-status.json`
- `build.noindex/uarc-live-model-scope/browser-live-product-v5/cleanup.json`

## 5. 制品

```text
App:
  target/aarch64-apple-darwin/release/bundle/macos/NomiFun.app
App host SHA-256:
  39d021de265b79ad51540fbe5e868a49f3912d3e102c79fbb096f2cf12d9ae53
DMG:
  dist/desktop/NomiFun_0.7.6_aarch64.dmg
DMG SHA-256:
  25059e0e0d6199d8727ab5734f2bd608a698c2a1dc5020d32cec8617c5e52565
Release lock:
  dist/desktop/NomiFun_0.7.6_aarch64.release-lock.json
```

## 6. `not_run` 与已知限制

- Keychain：不作为本轮要求；用户提供的 StepFun credential 只经隔离 wrapper/stdin 使用，制品扫描 0 命中。
- attached Chrome Remote Debugging：按用户产品决定不运行，不影响 managed CEF 完成。
- Screen Recording granted/Retina screenshot：按用户产品决定不运行；denied 路径的失败投影已通过。
- 真实中文/日文 IME composition：按用户产品决定不运行；Terminal focus、Unicode、resize 已通过。
- Gatekeeper interactive assessment：本机系统级 Gatekeeper assessment 被关闭，记为 `not_run`；Developer ID、
  DMG 公证和 staple 均由独立工具验证通过。
- 真实 StepFun 修复流程已经完成页面复现、源码 patch、重载和 trusted click `2 -> 1`，但模型随后在通用
  `report_completion`/compaction 环节达到 `MaxOutputTokens`。这不是 native Browser/CEF 或制品失败；不把
  该回合终态冒充 `turn/completed`。

## 7. 后续平台工作

Mac 核心阶段完成后，仍必须回到 Windows 执行 UARC-064；只有其完成后才能执行 UARC-070。本文不声明
整个跨平台 UARC 完成。
