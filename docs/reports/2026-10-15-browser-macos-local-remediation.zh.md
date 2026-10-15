# macOS 内嵌浏览器本地整改与验收记录（2026-10-15）

本轮已在真实 Apple Silicon Mac 上完成完整静态 UI App、两个 smoke 与 controller 的本地构建及审计 App 签名打包，但原始任务尚未完成。queued/in-flight popup Pause 与同文档 History 的源码缺口、启动合同的 scope/parser/receipt 问题分别处理；这些本地修改与定向检查不能证明百度重复导航已解决。当前没有本轮 NomiFun/WK 原生运行、主文档请求计数、存储重启或生产 GUI 通过证据。

本记录补充[既有整改证据](2026-10-13-browser-remediation-evidence.zh.md)与[macOS 交接](../handoffs/2026-10-13-browser-macos.zh.md)。Windows 的 Bilibili A/B/A 结论和更早 Mac 结果保留各自范围，不充当本轮融合代码的实机验收。

**Git 交付说明**：用户随后明确授权 commit+push，本轮修复与文档按该授权提交至 `origin/main`。下文 `5259ca8e` 是构建验收时的源码基准；接续应使用包含本轮 popup/History 修复及本文件的提交，不能退回该基准而遗漏本轮修复。Git 交付不改变尚未取得的原生与生产 GUI 验收结论。

## 环境、源码与证据根

| 项目 | 本轮实际记录 |
| --- | --- |
| 客户端日期与时区 | 2026-10-15，Asia/Shanghai |
| 系统 | macOS 26.6.2，build 25G83 |
| CPU 架构 | arm64 |
| 硬件 | Apple M4，`hw.model=Mac16,12` |
| checkout | `/Users/muri/code/nomifun/nomifun-desktop`，`main` |
| 初始源码 | `HEAD=origin/main=6409993b776c95f14ebc4785b2eb4d2759a526f8`；fresh fetch 后 ahead/behind 0/0；初始工作区 clean |
| 最终构建基准 | fresh fetch 后发现兼容后继提交，已 `git merge --ff-only origin/main` 至当时的 `HEAD=origin/main=5259ca8e53a0e15f08d725fa15ab3f39cd2cad56`；包含初始 `6409993b` 修复，记录提交前的构建验收快照 |
| 已发布修复 | 当前源码包含 `fix(browser): stabilize embedded identity and navigation lifecycle`；无需再次导入 portable ZIP 或旧 patch |
| 本轮交付边界 | 用户随后已授权提交并推送本轮修复；保留既有实机验收边界，不改写共享历史 |
| 主要证据根 | `/tmp/nomifun-macos-browser-20261015-01a13db7` |

环境与启动限制另见证据根下 `environment-and-tool-restriction.json`。临时目录是本机原始证据位置，不代表已归档到仓库或可从其他机器直接读取。

收尾时远程新增 `5259ca8e`（`fix(templates): refine template editor layout and controls`），仅修改 6 个 renderer 模板编辑器/交互测试/i18n 文件。与本轮 26 个本地 task 路径交集为 0，fast-forward 前后这 26 个路径 hash 无漂移，备份 patch 与 `remote-advance-before.json` 保留在证据根。Rust/browser/fixture 来源没有改变；原 47 项 macOS crate、13 项 controller、34 项 JavaScript 及两个 smoke 的证据保持其已测范围。由于共享 UI 来源变化，另跑最新完整静态 App、embedded UI mock gate 与 desktop boundary，见下方最新基准记录，不倒改先前 `6409993b` 的构建或来源快照。

## 已修改问题及证明边界

### queued/in-flight popup 的 Pause 准入

复核了 `callbacks.rs::Delegate::create_popup` → 异步 `PopupCandidate` → `macos/host.rs::consume_popup`。发布基线只在创建入口检查 opener 暂停；候选等待期间的 Pause 可以越过该检查，消费端缺少准确 opener 与可撤销准入，Pause 也没有候选原生关闭结算屏障。这是源码竞态，尚无本轮 WK 实机反例，不能称为百度重复刷新的已证实根因。

本轮在既有 Page/候选 owner 上加入准确 opener、document generation、navigation sequence 与可撤销 ticket；实际 host 登记在同一准入 owner 锁内完成，Pause 撤销旧 ticket，并保留待关闭 child 的归属直至精确 native close ACK。Resume 不使旧候选重新有效。queued/claimed 的 debug 确定性阶段门用于捕获真实候选，不授权 popup、不伪造原生关闭。

相关实现为 [popup owner](../../crates/backend/nomifun-browser-macos/src/popup.rs)、[Page/candidate 生命周期](../../crates/backend/nomifun-browser-macos/src/engine.rs)、[WK callback](../../crates/backend/nomifun-browser-macos/src/callbacks.rs)与[Host 消费](../../apps/desktop/src/browser_surface/macos/host.rs)。Popup 定向 unit 7 项通过，源码审查已完成；这只证明已运行的纯状态/定向范围。新增的原生阶段用例仍须实际启动后核对活动标签、child close、主文档请求计数及父文档 nonce，当前均未取得。

### 同文档 History 不应制造永久 Loading

本轮保留首次 reducer 失败 `/tmp/nomifun-macos-history-before-fix.log`：`history_submission_preserves_the_loaded_document_without_native_start` 实际 FAIL，断言 `Requested != Finished`。它证明 reducer 将 History 提交直接变成等待新文档的状态；没有将此 unit 失败扩写成已经复现的生产 GUI 故障。

修复在提交 Back/Forward 时推进控制 fence/dispatch，保留当前真实文档阶段、进度、问题与 native attempt 绑定；仅与该次命令准确关联的 WK 原生 start 才建立新的主文档 Loading，避免用 URL、最新序号或 metadata 采样推断新文档。旧 token、旧控制与迟到 callback 的归属继续分开。相关实现为 [navigation reducer](../../crates/backend/nomifun-browser-macos/src/navigation.rs)、[callback 原生关联](../../crates/backend/nomifun-browser-macos/src/callbacks.rs)与[Host receipt](../../apps/desktop/src/browser_surface/macos/host.rs)。

修复后 macOS crate 最终 47 项 unit 全部通过；原日志复制到证据根 `nomifun-macos-history-final.log`，SHA-256 为 `f75ff4845f55ff0d253bfa882177a588ebd9e507c8c687ec51d126fbd8e7f0fe`。首次 reducer FAIL 保留，后继通过另记。SPA Back/Forward 的真实 WK 页面内容、输入、Loading、document nonce 与主文档 HTTP 计数仍待原生运行。

### 启动 scope、参数解析和最终 receipt

本轮修正开发/审计入口的 scope、应用参数解析及 receipt 校验，并增加 packaging-only 的 `scripts/validation/package-macos-native-fixture.mjs`，复用现有签名 fixture 与固定 descriptor 合同，为 compatibility/AppKit 两个入口准备独立数据根及产物。脚本只打包，不执行原生启动、关闭或界面操作。

相关 JavaScript 定向测试实际为 34 pass、108 assertions。该结果证明 parser、scope 与打包/receipt 的已测逻辑，不代表已通过 LaunchServices 启动、已得到 native final receipt，或已完成原生清理。此脚本不是绕过 computer use 限制的方案。

最终源码复核还发现独立未补足项：`--manual-sites` 的 [console reader](../../apps/desktop/examples/support/macos_browser_compatibility_live.rs)读取 App stdin，而现有固定 descriptor 只接 stdout/stderr，NSWorkspace 启动没有 stdin 传输。当前没有可用的 console attestation 通道；scope/parser 修复不等于 manual runner 已可用。后续须在既有身份、descriptor、准入及 receipt 合同下实现输入传输并实机验证，不通过裸启动补交互，也不把缺失的 typed/clicked/done 确认写成通过。

## 初始 6409993b 基准的构建记录与首轮失败

| 检查 | 已有事实 | 边界 |
| --- | --- | --- |
| 首批 UI build | EXIT0；证据根 `ui-build.stdout.log` / `ui-build.stderr.log`；后续 `6409993b` 静态 App 的 manifest 为 0.8.8 / API30 / `frontend_build_id=affe9e60-0cfa-4076-9e44-712261f3d6ab` | 属 `6409993b` 历史批次，只证明 renderer 产物构建，不证明生产 App 已加载或访问静态资源 |
| 首轮完整 Rust build | EXIT101；保留 `build-first.stdout.log` / `build-first.stderr.log` | 该次 build 与并行源码编辑交叠，出现新 `PopupCandidate` API 与尚未写入 module 的不一致；不能称为冻结发布基线的产品构建失败，也不计成功 |
| 首次 static App 包装命令 | EXIT1，0.181 秒；保留 `static-app-build.*` | Bun 吞掉 separator，Tauri 因 `--locked` 位置而拒绝；未进入编译，不算产品构建失败 |
| compatibility smoke 类型检查 | EXIT0，2m 43s；`cargo check -p nomifun-desktop --example macos_browser_compatibility_smoke --locked --offline` | 证据根 `nomifun-macos-browser-20261015-smoke-coverage-check.json` 指向实际工具 transcript；没有完整 raw 文件，且临近结束有 watchdog 编辑，最终 frozen build 另记；不是链接或运行证明 |
| 6409993b 冻结源码 desktop build | EXIT0，299.630 秒，Rust `Finished` 4m 28s；`static-app-build-attempt2.receipt.json` | 该批完整静态 UI App 已构建，现保留为历史；本轮最终 App 为下方 `5259ca8e` 产物；均未启动 |
| 两个 smoke 与 controller build | EXIT0，86.496 秒；`native-examples-build.receipt.json`；`cargo build -p nomifun-desktop --example macos_browser_compatibility_smoke --example macos_appkit_main_loop_smoke --example macos_application_controller --locked` | 只证明实际编译/链接，未启动；后续仅 compatibility 两处 timeout 表达式修订，窄 rebuild 另记 |
| compatibility 最终窄 rebuild | EXIT0，16.754 秒；`compatibility-final-rebuild.receipt.json`；`cargo build -p nomifun-desktop --example macos_browser_compatibility_smoke --locked --offline` | 证明修订后 compatibility 入口构建，不是 WK 原生运行 |
| controller unit | EXIT0，13 passed / 0 failed，214.965 秒；`controller-unit.receipt.json` | controller 合同定向 unit，不是实际 App admission、退出或清理证明 |
| Embedded UI 定向 gate | EXIT0，1 passed / 0 failed / 171 filtered，27.049 秒；`NOMIFUN_REQUIRE_EMBEDDED_WEBUI_TEST=1`，测试 `generated_custom_protocol_context_contains_a_valid_embedded_webui_snapshot` | mock generated context 验证静态 assets，不能称原生 UI 或实际资源访问通过；原始命令与结果见 `final-targeted-stages.json` |
| Browser platform boundary | EXIT0，5.901 秒；`browser-boundary.receipt.json` | 边界检查，不代替 WK 原生验收 |
| Desktop UI boundary | EXIT0，0.232 秒；`desktop-ui-boundary.receipt.json` | 保持桌面 880×600 最小边界，不代替生产窗口操作 |
| 完整 App 签名核对 | strict codesign EXIT0；本地 ad-hoc | 不等于 Developer ID/公证发布验收 |
| AppKit fixture packaging-only | EXIT0；`appkit-fixture/package-report.json`，deep strict signature 核对通过，链接系统 WebKit | 只打包，不启动；不证明 AppKit main-loop 用例实际运行 |
| Compatibility fixture packaging-only | EXIT0，0.220 秒；`compatibility-fixture/package-report.json`，供给核对与 deep strict signature 通过 | 只打包，不启动；不证明任何 fixture/站点/Pause 原生结果 |

首轮 stderr 中 `wait_ready` / `wait_admission_stage` / `admit` 等缺失诊断来自编辑交叠的时间窗口。保留原始失败日志，后续冻结构建另记结果；不覆盖历史，也不据该次结果宣称源码基线已被完整验证。

该批构建前已保存 `final-source-before.json`，冻结 4812 个 non-doc 来源路径。第二次 static 构建通过显式 `main(['--','--locked'])` 正确传参，日志为 `static-app-build-attempt2.*`，实际 EXIT0。该批 `final-source-after.json` 与修订后的 `final-source-amendment-before-rebuild.json` 全部 4812 路径比较差异为 0。这是 fast-forward 之前的证明，后继源码使用独立 `latest-source-before.json` / `latest-source-after.json` 记录。

`final-source-amendment-before-rebuild.json` 将初次冻结以来仅有的两处来源变化单独记录：`macos_browser_compatibility_smoke.rs` 与 `support/macos_browser_compatibility_probe.rs` 的组合 timeout 预算。没有 production source 变化；上述 compatibility 窄 rebuild 用于覆盖修订后来源，不把前次已构建 artifact 冒充最终例程。

该批历史静态 App 为 `/Users/muri/code/nomifun/nomifun-desktop/target/debug/bundle/macos-fast/run-bgIMBC/NomiFun Dev.app`。签名后 Mach-O 为 536,401,632 bytes，SHA-256 `b3902f2e767e29a13081b5f7483d5df778635dbf8600edcbdf4adaeb11a144c6`，App 普通文件合计 536,562,407 bytes。`static-app-artifact.json` 保留该 `6409993b` 基准签名前正式 build identity、签名后 tree/library manifest 与旧 UI manifest；这些原始/签名 artifact 不冒充最终 `5259ca8e` 版本。此历史产物未原生运行，不由 build/codesign 成功推导实际静态资源可访问。

AppKit audit App 为证据根下 `appkit-fixture/NomiFun AppKit Main Loop Audit.app`，scope `appkit_main_loop_audit`，独立数据根为同级 `data`。供给的未签名/复制前二进制 SHA-256 为 `181cc7250e78caaa7872a514437b803ce1cbd0af4bf9829c4b2870bb7f984035`，签名后与打包二进制同为 `761035f044e298e48ff629483643c7678b27b04b18baab47b2099c669d8f64e6`；package report 明确 `packaging_only=true`，没有 AppKit 测试运行结论。

Compatibility audit App 为证据根下 `compatibility-fixture/NomiFun WK Compatibility Audit.app`，scope `browser_compatibility_audit`，独立数据根为同级 `data`，打包参数为 `--evidence-dir <fixture evidence> --hold-ui`。最终窄 rebuild 的供给二进制 SHA-256 为 `11d407972fd961be42ff2e418f4b5dba3f6d7fc1ea0311b75e0445622ea2a130`，签名后与打包二进制同为 `128819fa2e165b8682ff75fe61e3d59bb0f54d0d2bbf8102e135e908a4ec3803`，供给签名前核对和 signed cache 一致性均为 true。controller 冻结副本位于 `/Users/muri/code/nomifun/nomifun-desktop/target/macos-dev-app/controller-SPTpwA/macos_application_controller`；准备产物不代表已通过获允许的工具调用启动。

## 最终 5259ca8e 基准与最新静态 App

| 最新窄重验 | 实际结果 | 证明边界 |
| --- | --- | --- |
| 完整静态 App | EXIT0，266.703 秒；`latest-static-app-build.receipt.json` | 已构建最终 checkout 的完整 App，未原生启动 |
| Embedded UI gate | EXIT0，1 passed / 0 failed / 171 filtered，58.706 秒 | mock generated context 验证最新静态 assets，不证明原生 UI 访问 |
| Desktop UI boundary | EXIT0，0.552 秒；2115 sources，min 880×600 | 桌面边界检查，不代替生产窗口操作 |
| 最新来源快照 | `latest-source-before.json` → `latest-source-after.json`，4812 个 non-doc 路径，漂移 0 | 单独证明最新三项执行期间来源稳定，不覆盖或改写前批快照 |

以上实际命令、起止时间、退出码及未启动标记见 `latest-validation-stages.json`。最终 App 为 `/Users/muri/code/nomifun/nomifun-desktop/target/debug/bundle/macos-fast/run-rv4A2P/NomiFun Dev.app`，`latest-static-app-artifact.json` 记录 `HEAD=5259ca8e53a0e15f08d725fa15ab3f39cd2cad56`，签名后 Mach-O SHA-256 为 `1117ff49c1cb215c51e2e7f2f114dcdffa6d1fd38cb2b0682ed9fdfe1d2505d9`，Mach-O 536,401,632 bytes，App 普通文件合计 536,562,407 bytes；strict signature EXIT0，仍是本地 ad-hoc。最新 UI manifest 为 0.8.8 / API30 / `frontend_build_id=a84bfeb8-8d4f-4a32-891c-66bcb69cb236`。

为避免后续构建替换证据，最终完整 App 另复制到 `/tmp/nomifun-macos-browser-20261015-01a13db7/production-fixture/NomiFun Dev.app`。`production-fixture-copy.json` 核对该 copy 与 `run-rv4A2P` 的完整 file tree 相同、签名后二进制 SHA 相同、Mach-O 为 arm64、strict signature EXIT0。最终 App 与复制品均未启动，静态资源 runtime access 仍未验证，也没有 Developer ID/公证发布验收。上述签名 smoke/controller 来源没有与 6 个上游 UI 文件重叠，不因本次 fast-forward 扩大其原生验收范围。

## 原生启动限制与未取得的 receipt

本轮 computer use 调用 `cua.getApp('com.apple.Terminal')` 被工具明确拒绝：`Computer Use is not allowed to use the app 'com.apple.Terminal' for safety reasons.` 未通过其他终端、shell 启动应用、裸二进制或新 wrapper 绕过该拒绝。

当前可用原生 `cua.getApp` 只接受 App 名称/路径，不传递既有固定 descriptor 所需的 argv/env；本轮尚无经允许工具可用的固定 descriptor 启动路径。现有 LaunchServices、controller、签名 fixture、主线程与最终退出/清理合同继续生效。NomiFun 和两个 smoke 均尚未实际启动，因此没有本轮 WK native receipt、站点主文档时间线、HTTP 服务端 witness、store 重启证明或生产 GUI 通过。

## 系统 Safari 独立对照

Safari 可以经直接 computer use 正常打开。本轮使用独立 private 窗口，避免复制或导入日常用户 Cookie。以下只是系统浏览器独立 baseline：

| 场景 | 实际可视观察 | 证据边界 |
| --- | --- | --- |
| 百度主页 | 独立静置 68.757 秒后可视正常 | 当时 CUA inline 截图；另存 `safari-private-baidu-home-history-revisit.png` 是后续历史回访，不是初次 60 秒快照；没有主文档 HTTP 计数或原生 WK trace |
| 百度搜索 | 独立静置 86.690 秒后可视正常 | 最终证据为 `safari-private-baidu-search-after60.png`；`safari-private-baidu-search-final.png` 实为 58.601 秒的中间观察，不计为 60 秒完成证据；截图位于证据根 |
| Bilibili 主页 | 已加载观察后独立静置 141.867 秒，可视正常，有登录推广 | `safari-private-bilibili-loaded.png` 与 `safari-private-bilibili-after60.png`；没有登录，推广不记为验证码；不是 Bilibili 搜索子页验收 |

系统 Safari App 版本为 `26.6.2 (21624.5.1.11.3)`；没有读取本轮页面的实际浏览器 UA，不把 App 版本、OS 版本或兼容 token 写成实际 WK runtime 版本。上述观察没有 Safari17/NativeWebKit 两臂，不能称同条件三臂 A/B，不足以调整正式默认身份。当前默认仍为 Safari17，也不从一次可视正常推算风控故障率或承诺零风控。

完整单臂记录与截图 SHA-256 见证据根 `safari-baseline-summary.json`。搜索首次 `typeText` 没有可见输入、站点按 placeholder 发起的那次不计正式观察；随后以原生粘贴确认输入并点击搜索，再计算上表的 86.690 秒。本轮没有解验证码或自动刷新。Bilibili 已加载观察与最终观察时间分别为 `2026-10-15T04:11:05.591Z`、`2026-10-15T04:13:27.458Z`。独立 private 窗口于 UTC 04:13:56.709 关闭，UTC 04:14:01.753 执行 Cmd+Q，UTC 04:14:58.076 的 computer use 状态核对 Safari 列表为空，另存 `safari-quit-verification.json`；此退出观察不充当 NomiFun 的 native cleanup receipt。

## 尚未完成的验收与下一步

1. 通过获允许的 computer use 启动路径，实际运行当前 checkout 的完整静态 UI App 与两个 smoke；确认准确构建身份、LaunchServices admission、descriptor、native exit 与 cleanup receipt。工具拒绝不绕过。另补 `--manual-sites` 在固定 descriptor 下的受约束输入传输与实际 attestation；现有 stdout/stderr 不能代替 stdin。
2. 在可控 fixture 上执行 queued/claimed popup 确定性阶段 Pause/Resume，核对候选撤销、准确 child close、活动标签、实际主文档请求计数与父文档 nonce；源码/unit 通过不替代这些结果。
3. 覆盖 Stop、Pause/Resume、首次失败、保留旧内容、未知/迟到 callback、旧导航控制、精确 target 与同一 snapshot 的 navigation sequence、原生输入锁、同文档与跨文档 Back/Forward。
4. 验证持久 Cookie/localStorage/IndexedDB 在 Runtime/App 关闭重开及进程重启后保留；不同 Session/store 隔离，临时 store 清理等待原生完成，不复制用户 Safari Cookie。
5. 在生产 NomiFun 窗口操作地址栏、标签、菜单、Stop、Pause/Resume、失败恢复、显隐及原生输入，采集安全主文档时间线、触发来源、attempt/document 身份与截图。区分重定向、子资源、SPA 与主文档重复请求。
6. 同机同网、受控初始状态下比较 Safari17、NativeWebKit 和系统 Safari 的百度主页、搜索及代表站点；各组独立静置至少 60 秒，正常/验证码/版本拒绝/循环分别记录，验证码不计正常通过。
7. 最低支持系统、Intel、Developer ID 签名与公证发布验收仍未覆盖；本机 arm64 开发构建与本地 fixture 签名不扩大为这些结论。

本任务保持未完成；仍缺融合后 macOS 原生与生产 UI 证据，原始百度现场也尚未复现、归因或证明修复。本轮没有改变原生内嵌浏览器为主产品形态、默认 Safari17、权限/媒体/输入边界或 canonical Agent Session 数据合同。
