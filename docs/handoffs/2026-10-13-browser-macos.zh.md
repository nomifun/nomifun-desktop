# 内置浏览器 macOS 整改交接

**当前接续状态（2026-10-15，真实 macOS）**：本轮初始为 clean 的 `main=origin/main=6409993b776c95f14ebc4785b2eb4d2759a526f8`，fresh fetch 后 0/0。收尾时核对远程后继 `5259ca8e53a0e15f08d725fa15ab3f39cd2cad56` 包含该浏览器修复，已 fast-forward，构建验收时 `HEAD=origin/main=5259ca8e53a0e15f08d725fa15ab3f39cd2cad56`。用户随后授权本轮 commit+push，接续应使用包含本轮 macOS popup/History 修复及本文件的提交。无需导入此前 portable ZIP 或重复应用旧 patch。下文 `2d5aa62d`、“251 路径尚未提交”“仅 pull 不足”等均是发布前历史快照，不作为当前操作指令。

本机为 macOS 26.6.2 / 25G83 / arm64，Apple M4 / Mac16,12。本轮已修补 queued/in-flight popup Pause 准入、同文档 History reducer 及启动 scope/parser/receipt，并保留首次失败；完整静态 UI App、两个 smoke 与 controller 均实际构建 EXIT0，两份 audit App packaging-only/strict signature 通过。47 项 macOS crate unit、13 项 controller unit、34 项 JavaScript 测试、embedded UI mock gate 与 browser/desktop boundary 检查通过；最终修订后 4812 个 non-doc 来源路径比较差异 0。准确 artifact、命令与证明范围见[本轮 macOS 记录](../reports/2026-10-15-browser-macos-local-remediation.zh.md)。本轮 Git 交付已取得用户明确授权；原始百度现场尚未复现或归因，生产原生验收尚未完成。

后继提交仅含 6 个 renderer 模板/i18n 文件，与本轮 26 个本地 task 路径交集 0，fast-forward 前后本地 hash 无漂移；Rust/browser/fixture 来源不变。最新基准另完成完整静态 App build EXIT0（266.703 秒）、embedded UI mock gate EXIT0（1 pass / 171 filtered）与 desktop boundary EXIT0；`latest-source-before/after.json` 的 4812 路径漂移 0，不倒改旧批快照。最终 App 为 `target/debug/bundle/macos-fast/run-rv4A2P/NomiFun Dev.app`，证据根 `production-fixture/NomiFun Dev.app` 有完整 tree 相同的保留副本，签名后二进制 SHA `1117ff49c1cb215c51e2e7f2f114dcdffa6d1fd38cb2b0682ed9fdfe1d2505d9`；两者均未启动。旧 `run-bgIMBC`、旧 UI manifest 与 `static-app-artifact.json` 仅作初始 `6409993b` 批次历史。

computer use 明确以安全原因拒绝 Terminal；本轮未通过其他终端或 shell 启动绕过。当前原生 App 工具不传 argv/env，尚无可用的既有固定 descriptor 启动路径；NomiFun 与两个 smoke 尚未实际启动，因而没有本轮 WK native receipt、主文档请求时间线、存储重启或生产 UI 通过。系统 Safari 经直接 computer use 的 private 窗口已独立观察百度主页 68.757 秒、搜索 86.690 秒可视正常；它只是单臂 baseline，没有 HTTP 计数，不能代替 Safari17/NativeWebKit/系统 Safari 三臂对照。默认 Safari17 保持不变。

产品边界也已确认：保持原生内嵌浏览器为主场景，用户与 Agent 继续操作同页。系统浏览器只保留用户明确选择的部分场景功能及验收对照，不作为整改的主场景重构或自动回退。后续兼容、身份、存储和恢复优化须遵守这一约束。

**发布前历史：远程同步与Bili续查（2026-10-15）**

以下至“当前源码状态”前为当时 Windows 环境的发布前记录，按原时间与证据范围保留；其中未提交路径、portable 导入与“后续须携带”描述现已由上方已发布提交取代。

当前HEAD与origin/main同为 `2d5aa62d3f7daf2869fe20be457174742431238a`（Root核对0/0，workspace0.8.8）。同步时250个本地路径经备份/stash后恢复，232个逐字节保全、18个重叠整合，5个文件的文本冲突采用审核过的7份Mac候选解决。随后明确授权的main debug GUI验收配置新增1个tracked路径，当前共251个本地路径尚未commit/push；UTC03:02 fresh fetch/ls-remote仍核对2d5、0/0、无未解决冲突。`applied-macos-candidates.json` SHA `B2C61ED7E8D9B38680BC51B399FE5A0A5416C5BE1F6F0D40F6F3368AA26B4FED` 的7个应用文件已核对磁盘hash一致；`restored-source-verification.json` SHA `848C9BAC0B79A2DE9ADBFA62957D2DB6A60D9A614E4F289666B42EEFE7D79839` 记录恢复范围，无unexpected。证据在Windows本轮 `nomifun-browser-bili-sync-c0200bd3e0d84e9f9c63a6a66df90927` 临时根，后续须随portable包交接，不把目录名当Mac本机路径。

已实际运行 `bili-post-sync-macos-portable-unit`：10 passed/0 failed/EXIT0，stdout SHA `C36CF843BDD038BB4AC2966F89E7EB57A9D0E316FF673F9B6408D2ACEC233222`；`bili-post-sync-macos-target-check` 为nomifun-browser-macos的aarch64-apple-darwin/all-targets，EXIT0，stderr SHA `392603C0B53337053D093F1D6385998C96CD055DBF859DAE69C89138403A657C`。两批各1870来源前后0差异。这不是完整desktop Host/App链接、两个smoke运行或本轮WK原生证明。同步后的Windows125项与UI145项也不能替代Mac实机。

Windows Bili持续加载已通过同S1 Profile/同静态UI的A/B/A定位并修补：旧宿主在真实popup激活时过早令opener controller不可见；修补仅在首次child文档pending保活准确opener controller，HWND仍隐藏，child原遮罩不变，commit/终态/Pause/关闭/lease失效释放。77A旧行为pending、C212三次鼠标搜索实际成功、E429撤回后再现，构成当前Windows/WebView2 155环境的应用边界证明；不泛化SDK内部机制或直接归因WK。7项定向unit与main build通过，C212在反向对照后已恢复并核验，最后PID28612真实鼠标搜索也已通过：准确Document loadingFinished距click1152.514ms（probe elapsed851ms），169.084s后真实窗口仍见同tab正常结果，没有parent切换、reload、Stop或新导航；最终app保留可视运行，不记该PID已退出。准确artifact、time/receipt及历史A166混合基线见[Windows交接](2026-10-13-browser-windows-gui.zh.md)和整改证据。

程序启停现已由Root通过computer use自主完成，静态UI通过现有Tauri/custom-protocol内嵌资源加载，0.8.8/API30/build-id1f7e0b07-b245-4f04-aaec-69c66d8d0fa4，不依赖Vite。Windows新增的debug测试data/work和X退出配置仅服务那次隔离验收，不能代替Mac的LaunchServices/固定descriptor/签名/最终receipt合同；已观察的PID退出和部分cleanup日志也不等于全资源逐项证明。后续Mac所有启停与界面操作同样由可用computer use工具自行完成，不要求用户粘贴命令或代点退出；权限Deny不绕过。

当时Mac queued/in-flight popup Pause缺口未补，原始Mac百度现场未复现/归因；默认Safari17、同机A/B、正式持久store、完整Host/两个smoke及WK生产GUI均待真实Mac。Windows本项修补不是Mac通过，也不宣告跨平台Goal完成。当时要求携带portable包/manifest，是因为修复尚未发布；当前已发布源码与本轮本地补丁状态以上方和本轮记录为准。

**当前源码状态**

- `crates/backend/nomifun-browser-macos/src/state.rs` 保存实际 PageSnapshot 与 NativeDialog，engine 保持既有类型 re-export。真实 navigation reducer 现在可以在 Windows 执行。
- `callbacks.rs` 的主文档 Action 样本记录公开 navigationType、HTTP method、来源 frame/origin 与 policy，保持无 WKNavigation 关联；未知 token 会使 trace degraded，已知旧 token 不影响后继。
- `engine.rs`、`navigation.rs` 实现显式 Pause/Resume。Pause 保留可用内容，主导航 policy 检查 paused；包括尚未 didStart 的主导航请求和迟到 didStart。Resume 不自动 reload。宿主显式 Navigate/Reload/Back/Forward 在原有 guard 后解除 hold。本轮已为 queued/in-flight popup 补准确 opener、可撤销 ticket、登记结算顺序及精确 native close ACK owner；源码/定向 unit 不替代下述原生竞态验收。
- `identity.rs`、`lib.rs` 提供不可变 `BrowserIdentityPolicy::{Safari17, NativeWebKit}`；Desktop Host 与 DeferredEngine 传播策略。正式入口默认仍 Safari17，popup 继承同一 Engine policy。NativeWebKit 的 UA 通过公开隔离世界读取，失败后最多三次后续 finish 读取，不发新导航；未知版本不从 OS/Safari 猜测。
- `apps/desktop/src/browser_surface/macos/host.rs`、`lifecycle.rs` 消费共享控制与策略。Reload、Stop、Pause、Resume 的 `expected_navigation_sequence` 必填，在入口和原生主线程核对；原生导航不等待 TabAutomation，Stop/Pause/Resume 不经 DialogPending 的 PendingWork wrapper。其他语义操作保留原锁。
- macOS compatibility smoke 已区分百度主页、搜索和 Bilibili，验证码不计正常访问通过；补充连续 JS/meta/replace/A↔B、资源突发、暂停/恢复和 immutable identity A/B。
- 已保留远程AppKit原生快捷键monitor，并把document generation与navigation sequence从同一Page snapshot捕获，Host验证准确target与captured序号后再emit给renderer，不改取最新序号操作后继。
- 已整合公开AppKit `post_to_main` dispatcher、nested modal下的初始化/关闭，以及panic/poison的显式watch结算；LaunchServices admission/固定descriptor、签名fixture与最终exit/cleanup receipt保留，不能转裸二进制绕过。媒体权限仍受可见活动标签、UserReady、exact document与一次性用户决定约束；隐藏/导航/关闭/Agent接管及时拒绝或排空，Agent不能授权，Deny保持Deny。
- 本轮将 History 命令的控制 fence/dispatch 与新文档 attempt 分开：提交 Back/Forward 保留真实文档状态，只有准确 WK 原生 start 才创建 Loading；首个 reducer FAIL 保留，修复后 macOS crate 最终 47 项 unit 通过。此结果不证明同文档 History 的本轮 WK GUI 行为。
- 新增第二smoke `macos_appkit_main_loop_smoke`；兼容runner保留本地manual/live/identity-ab/evidence与远程媒体审计。`--media-user-ready`须配显式`--media-permissions`，身份A/B与媒体权限分跑。上述是源码整合合同，旧远程快捷键/媒体证明不能替代融合后的Mac重验。

共享层及 Windows 后续实现仍可能演进。交接时重点核对 `BrowserTabCommand`、`BrowserLoadSummary.navigation_control`、native/dispatch 序号与诊断 projection，不凭旧快照补默认值。新控制命令必须使用与 target 同一已捕获 snapshot 的导航序号；没有 load 不能默认为零。

**在途 popup：历史缺口与本轮修补边界**

发布基线 `callbacks.rs::Delegate::create_popup` 只在入口检查 opener 的 paused 状态，随后分配真实 WK child 并异步发送 `PopupCandidate`。旧候选缺少准确 opener target 或可撤销准入，旧 `macos/host.rs::consume_popup` 等待 creating lock 后没有复查 opener 暂停；Pause 也缺少候选清理与结算屏障。因此入口允许后、登记完成前发生 Pause 时，候选可能在 Pause ACK 后登记为活动标签。这是源码缺口，没有本轮 WK 实机时间序列反例，不能写成百度现场的已证实根因。

本轮已在真实 Mac 的当前源码上补 retained candidate 的 exact opener、document/navigation 身份与可撤销准入，Pause 撤销 queued/in-flight 候选并等待精确原生 child 关闭 ACK；Resume 不得放行旧候选。Popup 定向 unit 7 项通过并已做源码审查。新增确定性 queued/claimed 阶段门及原生 smoke 用例尚未实际运行；须捕获候选阶段后再 Pause，核对准确关闭、活动标签、主文档 HTTP 计数与父文档 nonce，不能用 unit 或 sleep 代替 WK 竞态证明。

**同步前历史验证的准确范围**

| 命令或证明 | 结果 | 不覆盖的范围 |
| --- | --- | --- |
| `cargo test -p nomifun-browser-macos --lib --locked --offline` | 同步前实际reducer 9项通过；原session正常exit0。本轮portable为上面的10项 | WK/AppKit实际运行、完整desktop Host |
| `cargo check -p nomifun-browser-macos --all-targets --target aarch64-apple-darwin --locked --offline` | production/native unit APIs 类型检查通过 | Mac App 链接、签名、显示、系统数据 store 与网站 |
| Host/lifecycle/PendingWork Rust parser 与 diff check | 通过 | 完整 desktop macOS 类型检查与运行 |
| portable outcome 分类测试 | 7 项通过 | 真实百度或网站结果 |
| smoke portable metadata 检查 | 同步前使用实际Platform/Tauri/Tokio类型、替换Mac factory/原生部分时通过 | 不覆盖融合后完整Host/WK；本轮portable与Darwin crate检查另列于上文 |

**后续必须补足**

1. 继续使用包含本轮 macOS popup/History 修复及本文件的 checkout；`5259ca8e53a0e15f08d725fa15ab3f39cd2cad56` 仅是本轮构建验收基准，不能退回它而遗漏本轮修复。后续远程提交须先核对包含关系与重叠改动，不导入旧 portable patch。保留无关文件，后续 Git 交付仍按用户授权执行。
2. 本轮完整 desktop Host 与 `macos_browser_compatibility_smoke`、`macos_appkit_main_loop_smoke` 已构建，两份 audit fixture 已签名打包；接续时核对本轮报告中的精确产物身份，不必重复套用首轮失败。首轮交叠构建 EXIT101 不算冻结基线结论。实际运行须使用获允许的 computer use 路径，保留 LaunchServices/固定 descriptor/签名 fixture 合同；当前工具拒绝不绕过，`--manual-sites` 的受约束输入传输仍须补足。
3. 使用全新临时数据根和 Profile 验证普通页、popup、第一次失败/Stop/Pause、已加载文档保留、未知 token、迟到 callback、旧导航控制及原生输入锁；特别验证 native controls 绕过等待后仍保留原操作 settlement owner。
4. 验证 Pause 不仅保留文档，而且阻止后续实际主文档请求。Windows 实机已经证明 NavigationStarting.Cancel 可能仍允许 GET；WK 的公开 policy 是否有类似行为目前未知。严格比较服务端 witness 与真实文档 nonce，不能把不换 URL 或不换标题当作没刷新。
5. 验证持久 Cookie/localStorage/IndexedDB 在 Runtime/App 关闭重开和进程重启后保留，不同 Session/store 隔离，临时 A/B store 删除等待原生完成；不复制用户 Safari Cookie。
6. 同机同网比较正式 Safari17、NativeWebKit 与 Safari 的百度主页、搜索和其他代表性站点；固定初始站点状态，每组独立 60 秒静置并记录正常/验证码/版本拒绝/循环 outcome。
7. 执行生产 NomiFun 窗口中的实际地址栏、菜单、Stop、Pause/Resume、失败后同位置恢复、显隐与原生输入。smoke 的 console 命令与物理输入确认不是生产 toolbar 验收。另覆盖同文档 SPA 的 Back/Forward：只有地址/历史变化、没有新的完整文档回调时，不应持续显示等待新文档的 Loading；保留文档、输入与主文档 HTTP 计数，分别检查控制 fence 和原生尝试。Windows 已用定向原生用例验证该修补及命令已返回后的 BeforeUnload Stay，这些结果不作为 WK 的通过证明，也不直接将 Windows 私有状态逻辑移植到 macOS。
8. 有实机 A/B 证据后再决定正式 UA；保持一个 Runtime 内身份一致。没有证明之前保留 Safari17 默认，也不承诺所有网站零风控。

**当前可重跑入口**

```sh
cargo test -p nomifun-browser-macos --lib --locked
cargo check -p nomifun-desktop --bin nomifun-desktop --locked
cargo build -p nomifun-desktop --bin nomifun-desktop --locked
cargo build -p nomifun-desktop --example macos_browser_compatibility_smoke --locked
cargo build -p nomifun-desktop --example macos_appkit_main_loop_smoke --locked
```

以上是开发检查/构建命令，不是原生启动指令。生产 App 须启用实际静态 UI 产物；完整实际命令与结果以本轮记录为准。两个 smoke 的既有参数分别为 `--hold-ui --manual-sites --evidence-dir <独立证据目录>` 和 `--identity-ab --hold-ui --manual-sites --evidence-dir <独立证据目录>`，参数须经固定 descriptor/controller/LaunchServices 合同传入，不能用 `cargo run` 或裸二进制代替获允许的 computer use 启动。

`--manual-sites` 包含 `--live-sites`，其源码保留原生人工输入。每站 console 的 typed/clicked 是实际操作确认，entry/back/reload/hide/show/done 作用于当前活动标签（可能是 popup）。但本轮复核发现该 reader 依赖 App stdin，而固定 descriptor 只接 stdout/stderr，NSWorkspace 没有现成 stdin 传输；当前 manual runner 没有可用 attestation 通道，scope/parser 修复不代表它已可用。须在既有启动/身份/receipt 合同下补输入传输并实机验证，不能裸启动补交互。

原生运行后分别核验 `entry_checks_passed`、`native_site_acceptance_completed`、截图、当前target、cleanup与最终launcher receipt：现有live退出口径可能仅按entry判定，native site未完成仍可能exit0，不能以CORE_PASS/exit0宣告完整站点或生产GUI通过；身份A/B与媒体审计分跑，不静默忽略不兼容选项。

后续交接输出保留完整命令结果、安全导航时间线、服务端计数、截图、OS/CPU与实际native identity，把未覆盖的最低系统、Intel、Developer ID/公证发布签名或站点限制分别表达，不新增本轮发布gate。本轮 WK 生产入口尚无新实机通过；更早历史Mac证明不倒改，也不因此宣告跨平台Goal完成。当前剩余项还包括持久 store 重启/Session 隔离、临时 store 清理、迟到 callback/旧控制/输入锁、生产 GUI 与身份三臂对照，具体清单见本轮记录。
