# 语音键盘与应用维护验证

## 当前验收边界

语音输入以可自定义的系统快捷键和字幕工具栏为入口，不要求连接外置键盘。第一期保留文稿、麦克风、流式/普通 ASR、美化与取消能力，停止自动串口扫描，并将安装维护移出设备/语音模块。第一期以 `7e1981204` 提交；第二期接入默认关闭、由用户主动开启的外置键盘适配器。当前实现契约见[语音键盘与输入入口](../architecture/native-keyboard.zh.md)。

应用维护的成功不依赖模型或 USB 就绪；真实录音停止、安装路径归属、文件回滚完整性仍须得到证明。旧服务仅允许退役，不能创建或恢复守护进程。Agent Session、模型凭据和实际用户数据不参与迁移。

## 应用维护拆分（2026-11-08）

维护模块、鉴权 IPC、GUI 生命周期锁和旧 owner 退役从 `nomifun-keyboard` 移入 `nomifun-desktop-maintenance`，独立 helper 为 `nomifun-app-maintenance`。`cargo tree -p nomifun-desktop-maintenance --depth 1` 确认正式依赖仅包含 anyhow、fs2、getrandom、serde、serde_json、uuid，不链接串口、音频、模型或 Tauri。串口适配器只接受调用方给定的运行目录，不再读取维护模块内部路径。

Windows 本机已执行：

| 检查 | 结果与范围 |
| --- | --- |
| `cargo test --offline -p nomifun-desktop-maintenance --lib --bins` | 27 个 library tests、1 个 helper test 通过，含真实 Windows 文件共享拒绝、恢复与暂存清理 |
| `app-maintenance-native-smoke.mjs` | 20/20 通过；独立安装/runtime，真实 Windows 进程、文件、锁、IPC 与 NSIS hooks |
| `app-maintenance-package-smoke.mjs` | 3/3 通过；实际编译生产 NSIS 模板、关闭/运行旧版升级、生成卸载器、持续占用失败保留旧注册和用户设置 |
| `run-windows-app-maintenance-gate.mjs` | 构建隔离 fixture 后串行执行上述 20+3 场景，整体通过；这是真实执行发布门禁入口，不只是脚本语法检查 |
| `bun run check:windows-installer` | 安装路径、模板、资源及文件事务合同通过 |
| 打包与签名构建定向测试 | `bun test scripts/keyboard-packaging.test.mjs scripts/desktop-build-win.test.mjs`：8 通过、8 个 POSIX 场景按平台跳过、0 失败 |
| 语音文稿与设备协议核心 | 最终 `cargo test --locked --offline -p nomifun-keyboard --lib`：66/66 通过，含旧设置保留与新快捷键默认值 |
| Windows 原生语音 Actor | 流式与普通 ASR 两模式均通过，合成 PCM/loopback 供应商；10 组软件入口约束及 4 类更新恢复，测试进程退出 0、残留本次进程 0 |
| Windows 系统快捷键 | 实际 OS 注册、冲突、按下/松开、重绑与注销验证通过；同一正式控制器源码，独立最小依赖图；完整 Actor 证据另列 |
| 桌面语音单元测试（第一期） | `keyboard::` 69/69、`app_maintenance::` 10/10 通过；包含快捷键注册恢复、采音与维护/参数解析，原生 OS 热键证据另列 |
| 设置页与字幕 UI（第一期） | 170/170 通过；typecheck、桌面边界、i18n 与完整 `bun run check` 通过 |
| 前端生产构建 | build ID `9aa97dfb-3356-4ca9-8457-3c00dfccc6bb`，构建通过；不作为实际语音采集证据 |
| 完整正式 Tauri GUI、真实麦克风、云端 ASR、实体键盘 | 本轮维护夹具未执行，不以合成输入或测试进程替代 |
| macOS/Linux 原生运行 | 本轮 Windows 环境未执行，需在对应平台分别取得证据 |
| 签名、公证、正式安装与发布 | 本轮未执行 |

独立维护报告位于仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-app-maintenance-native-refactor-20261108.json` 与 `C:\Users\rika0\AppData\Local\Temp\nomifun-app-maintenance-package-refactor-20261108.json`。每个报告记录 helper/fixture 和模板的 SHA-256、用例结果、进程或安装身份及未执行范围。临时安装使用唯一产品/注册表身份，结束后清理本次实例；没有替换真实 NomiFun、修改真实登录项、录制环境声音或读取凭据。

发布门禁入口的最终报告为 `C:\Users\rika0\AppData\Local\Temp\nomifun-app-maintenance-release-gate-20261108\native.json` 和同目录 `package.json`，原生 20/20 与模板 3/3 均通过。它们使用重新命名后的应用维护入口和产品身份。

## Windows 替换失败回归

原安装报错发生于 payload 解压之后、版本注册提交之前：`maintenance failed; application files were restored: replace ... 拒绝访问。 (os error 5)`。只读检查时，本机 exe 没有只读属性，当前用户具有 FullControl；失败时没有保存占用者快照，不能据此指认某个杀毒软件或历史 PID。

原实现把应用生命锁释放当作文件可替换条件，随后仅尝试一次 `fs::rename`。真实 Windows PE 夹具经鉴权确认停机并释放 `desktop.lock`，但继续存活 2 秒时，未修复 helper 产生与用户相同的错误 5；真实文件句柄仅占用 150ms 也能使未修复实现失败。新增原生进程退出等待及 5/32/33 有界发布重试后，上述场景通过。持续占用仍失败，逐字节恢复旧应用，禁止启动未证明完整的混合文件树。

首次故障调查中的未修复 helper、RED/GREEN 日志和报告保留在仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-installer-investigation-20261108-09a46edead084fe09bc5dfa3c24a7705\`。这些是故障复现与修复证据，不作为当前语音快捷键的验收。

`release:win` 与签名 RC 脚本已接入 `run-windows-app-maintenance-gate.mjs`：构建后、发布或接纳产物前，使用本次 helper 执行原生维护与生产模板夹具。数据库启动检查继续独立执行；不能以它代替操作系统安装行为。

## 软件语音验证要求

- 未连接设备时可通过软件入口使用语音、文稿编辑、取消和美化；ASR 未配置只限制需要该模型的动作。
- 快捷键按下/松开共用 Actor，重复按下只起录一次；禁用、取消、关闭、锁屏、睡眠和更新不能恢复旧按住意图。
- 重复/非法/被系统占用的快捷键明确失败；注册与设置写入恢复旧配置，不能出现页面与实际绑定不一致。
- 停采等待真实回执，单一麦克风 owner；字幕按需创建，语音初始化失败局限在功能状态，工作台和应用维护端点仍可用。
- 测试使用独立 profile、合成 PCM 和 loopback 模型时明确标注，实际麦克风、云端模型和系统快捷键触发分别列证据。
- 仅验证桌面 880×600 及以上，不新增移动端视口、触摸布局或手机验收。

## 外置输入验收边界

用户主动开启时才枚举/连接串口，关闭后停止并 join 设备线程，释放设备独占锁。硬件动作和系统快捷键复用同一 Actor，不新增录音器、ASR 连接、文稿或后台进程。设备断开、连接失败、旧按键帧和重连均不能破坏软件输入；启用/禁用和按住录音的竞争需要单独验证。

## 第一期桌面界面证据

实际设置组件使用明确标注模拟 API 的隔离夹具，在 1280×900 和 880×600（含 224px 设置侧栏）验证布局和快捷键原子提交。DOM 宽度不超过支持的桌面视口，没有页面异常；将听写组合修改为 `Control+Alt+D` 只产生一次完整设置写入。截图与报告保存在仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-voice-keyboard-ui-5b64b9faefc048cdab1b022e2c7da685\`，临时浏览器、Vite 与界面夹具均已清理。没有注册实际系统快捷键、录制麦克风或调用真实模型。

## Windows 系统快捷键原生证据

`voice_shortcut_validation.rs` 在测试专属的 880×600 原生窗口中运行，向该窗口注入输入之前核对前台 HWND 和 PID。使用 `Control+Alt+Shift+F23/F24`：F24 已被 OS 保留时，修改绑定明确失败且 F23 旧绑定继续收到 Dictate 按下/松开；释放占用并重绑后，F23/F24 分别路由 Toggle/Dictate；注销后 OS 可重新保留这两个组合，控制器不再收到事件。进程退出码为 0，已确认结束。

构建使用仓库外最小 Cargo 工程，其 bin 直接指向原 example，使用相同 `shortcuts.rs`、Settings crate、Tauri 2.11.2、global-shortcut 2.3.2 及仓库 vendor 补丁，不编入应用后端聚合 crate。正式控制器 SHA-256 为 `c0424513626788638c1780bf4eaf7386204e7ff6bd67248bec7df8435e8ee90f`，构建和执行期间源码未变。`native-report.json`、`provenance.json`、构建与执行日志位于仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-voice-shortcut-native-minimal-pcos6oc_\`。此用例不创建 WebView，不打开麦克风、串口、供应商或正式配置；只证明系统快捷键控制器链路，不能替代完整语音 Actor 的合成测试。

## 第一期原生语音 Actor 验收

同一生产 Actor、采音 owner 与文稿流程，在独立 Windows Job、私有数据/WebView 目录中完成 streaming/platform 两模式。每种模式的软件验证均通过：无设备前置、硬件逐键偏好不限制软件、空闲不创建供应商连接/音频/字幕、旧资源未停止时所有采音入口拒绝、按住/重复/松开、字幕切换保留正文、无设备美化、快捷键松开不能停止 UI 录音、编辑或 ASR 准备期间撤销不得迟到起录，以及取消/会话挂起/更新/重绑/禁用后需要新的释放与按下。流式握手撤销子用例仅适用于 streaming；platform 不声称执行流式协议。

两模式分别覆盖取消就绪维护、停采期间取消、原生停止超时及外部维护超时四个更新恢复场景，并检查真实停止回执、单一 owner、设置保留和旧按住意图不恢复。音频为明确合成 PCM，模型为本机 HTTP/WebSocket fixture，不打开系统麦克风、真实 USB 或用户模型凭据。最终报告为 `C:\Users\rika0\AppData\Local\Temp\nomifun-voice-phase1-20261108-2b7f20f3\native-streaming-green.json` 与同目录 `native-platform-green.json`；两者均 `passed=true`、进程退出 0、`timedOut=false`、本次存活进程 0。前端 UUID 为 `9aa97dfb-3356-4ca9-8457-3c00dfccc6bb`，二进制 SHA-256 为 `fbf07eda02afe256d6c72d5037ee89df29cd5bbeeba7bf47dae717c1592603cf`。

保留首次原生失败报告 `native-streaming.json` / `native-platform.json`：它们在更新恢复测试探针中将 Windows 文件锁争用错误 33 当成异常，并非超时。探针改为与 fs2 的原生争用码比较，补充实际 Windows 锁回归（1/1 通过）后，两模式复跑通过。完整后端首次 custom-protocol 构建另遇编译器栈溢出，临时将 `RUST_MIN_STACK` 提高为 512 MiB 后构建通过；没有改变仓库或用户的默认环境，构建日志与测试结果分别保留。

## 第二期可选外置适配器

第一期提交 `7e1981204` 后加入 `externalKeyboardEnabled`（默认 false）和六种连接状态：disabled、starting、connecting、connected、stopping、error。软件快捷键总开关与设备模式独立，K2/K3/K4 的硬件偏好保留且在界面显示。K1 复用字幕显示切换；已退役的 K1 主窗口激活实现与对应 Linux 验收断言一并移除，其他平台窗口、拖动、编辑及复制场景保留。Linux 变更未在本机原生执行。

已执行的第二期定向检查：语音/协议核心 67/67、桌面 `keyboard::` 74/74（包含 5 项适配器测试）、UI 183/183、typecheck、i18n（8574 键）、桌面边界与完整 `bun run check` 均通过。适配器测试使用实际 `Transport::start_validation` 工作线程和文件锁，包含默认关闭不创建运行目录、成功连接后关闭并 join、阻塞启动竞争关闭/退出后不得发布迟到实例，以及等待互斥锁的旧关闭请求不得停止新实例。它们不枚举 USB、不打开真实串口。

设置页实际组件在 1280×900 / 880×600 下验证可选区默认关闭、保留硬件偏好、K4 关闭提示和停止期间等待原生释放。各连接状态均不阻断软件识别就绪；关闭状态只在原生 disabled 后确认。该界面夹具的设备、麦克风与模型均为模拟，未登记 OS 快捷键。截图与报告：`C:\Users\rika0\AppData\Local\Temp\nomifun-voice-keyboard-ui-phase2-8f8511452b5c4bcb9ed5957ba7610ce1\`，临时浏览器/Vite/夹具已清理。

补测实际出厂默认快捷键时发现 Ctrl+Shift+Escape 被 Windows 保留，已将默认取消改为 `CommandOrControl+Shift+X`。更新的原生快捷键用例先验证全部四个 `Settings::default` 绑定真实注册成功、随后全部释放，再执行 F23/F24 冲突/按下/松开/重绑/注销验证；不向默认组合注入输入。最终 `native-report.json` 确认 `defaultPlanRegistered=4`、`defaultPlanReleased=true`、`ok=true`。报告与源码/二进制哈希保存在前述 `nomifun-voice-shortcut-native-minimal-pcos6oc_` 目录，早期 F23/F24 报告另存 `native-report-phase1.json`。

最终前端生产构建 ID 为 `defe4152-17ed-40f5-b2ff-abc42570b369`，API 合同版本仍为 54。真实麦克风、云端供应商、实体外置设备、macOS/Linux 原生执行与正式打包发布不属于这些夹具的通过证据。

最终第二期原生语音报告：`C:\Users\rika0\AppData\Local\Temp\nomifun-voice-phase2-20261108-423bcf9a\native-streaming.json` 与同目录 `native-platform.json` 均通过，进程退出 0、未超时、本次存活进程 0。每种模式包含 10 组软件入口约束、14 组外置适配器约束及 4 类更新恢复。外置约束实际验证默认无 owner、主动开启后单一 owner、快捷键关闭仍可使用 K1–K4、设备丢失只停止硬件采集、可恢复故障保留发现线程、先使用硬件后进行 UI 本地检测仍隔离、USB 故障和关闭不影响软件采集、终止实例不能由迟到启动消息复活、关闭完成时 owner 已 join、旧帧在关闭/重开后仍被拒绝，以及硬件专属设置不取消软件等待识别。等待流式连接子用例仅适用于 streaming，platform 报告明确不适用。

两报告记录相同二进制 SHA-256 `e10af04c3ff409f4fa26215f0d2000091318eb02595eace41474a833263f0283` 和最终前端 UUID。音频为合成 PCM、设备帧为隔离注入、传输工作线程/文件锁为真实运行，供应商为 loopback fixture；不把它们写成实体串口、真实麦克风或云端成功。最终控制器 OS 例子亦已按最终 Settings 源码 SHA-256 `8f2ec8e4b5d059c7698a1d86912d8655425195a7786f48732795d8c28a1de72a` 重建并复跑，默认四组注册成功；其二进制 SHA-256 为 `933436ee882ec1fafb04cf920784c37d337a0fb02c7763f24ffb0c64ff9698d9`，运行前后源码未变。

本轮未替换用户当前正式安装、未修改真实登录项/模型凭据/用户数据，未执行签名或发布。已有 `.tmp/` 中的其他工作保持不变并排除在提交之外。

## macOS 源码级隔离加固（2026-11-08）

按后续要求完成源码级链路加固，本轮不启动 NomiFun、GUI、真实麦克风、串口或真实 launchctl 测试。

- macOS 正常启动与维护只使用 `NomiFun/app-maintenance`；可选设备使用 `NomiFun/external-keyboard`。原 `keyboard-service` 的锁、JSON、目录权限与登录项不再参与应用或语音准入。
- 当前应用 stop/unregister 不查询、解析或清理旧登记。旧清理只在正常启动后的独立后台任务执行；加载任务与磁盘文件各自尽力处理，任一失败不抑制另一分支、不向核心调用方返回失败。
- 语音启动移除 legacy startup_error 参数与封锁。退出与更新共用宿主停止标志，迟到初始化不能打开采音。
- 采音所有权由同一原生生产者的代次证明管理。Actor 通道关闭、应答超时、设备 join 或快捷键注销错误，在没有原生采音 owner 时不再阻断应用退出/更新；旧回执、Drop、超时不能伪造新 owner 已释放。Mac 替换应用文件前关闭新准入并再次核验真实停止证明。
- 外部维护在回复停止成功前设置同一宿主标志，再确认可能迟到的运行期并核验生产者；可选设备清理有界、快捷键清理不占用 Actor。
- Windows/Linux 已发布安装器、包钩子协调协议与 Agent/backend 的关闭及检查点安全约束保持。

本轮宿主实测与编译检查：

| 检查 | 结果/范围 |
| --- | --- |
| 共享维护库与 helper 单元测试 | 33 + 1 通过；含持有旧目录全部锁、损坏旧描述符而现代启动/停机仍正常的实际文件锁夹具 |
| 桌面维护入口与旧项隔离夹具 | `app_maintenance::` 15/15 通过（包含下方旧登记清理夹具），没有运行应用主入口 |
| 捕获/更新停止证明测试 | 17/17 通过；无 owner 的丢失 Actor/超时继续、真实 owner 未确认仍拒绝、旧/重复回执隔离、宿主迟到初始化、最终 fence 不被取消移除 |
| 可选适配器测试 | 6/6 通过；含阻塞清理的有界返回，无真实串口或 GUI |
| 旧登记清理纯夹具 | 9/9 通过；使用临时文件和 fake query/bootout，没有调用系统 launchctl |
| 包装与卸载定向检查 | `bun test scripts/keyboard-packaging.test.mjs scripts/desktop-build-mac.test.mjs scripts/desktop-build-win.test.mjs`：20 通过、11 按平台跳过、0 失败；POSIX 路由使用 Git sh 与临时假程序 |
| Windows 编译 | 桌面 bin 与更新后的原生 Actor example compiler check 通过；没有执行 example |
| Apple Silicon 共享维护模块 | `cargo check --locked --offline -p nomifun-desktop-maintenance --lib --bins --target aarch64-apple-darwin` 通过 |
| Apple Silicon 平台/旧项清理源码 | 仓库外 SDK-free 最小工程直接引用实际 `app_maintenance/platform.rs` 与 `legacy_cleanup.rs`，`cargo check --lib --tests --target aarch64-apple-darwin` 通过 |
| 完整 Mac 桌面编译 | 已尝试，`ring` 的 C 构建因本机缺少 `cc` 退出；本机也没有 Xcode/macOS SDK。不得将上述模块检查等同完整 App 构建通过 |

完整仓库 `bun run check`、文档与 Windows 安装合同检查通过。新 macOS 卸载脚本还验证含空格 App 路径、只调用现代 stop/unregister 一次、历史警告不改变成功、当前应用真实停止失败仍非零、缺少可执行文件拒绝，以及 POSIX 语法。Info.plist 的语音用途文案已改为快捷键/字幕/可选硬件，audio-input entitlement 保持。

本轮最终日志位于仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-macos-source-final-20261108\`；捕获与适配器定向结果在 `nomifun-voice-proof-unit-20261108`，包装报告在 `nomifun-macos-source-hardening-6499c35e79fe4f53b6653613a4b37c0e`。SDK-free 源码工程为 `nomifun-macos-maintenance-source-check-cw2h8x3w`，从仓库目录使用其 `--manifest-path` 以保持相同 Cargo source 配置。

仍未验证：完整 Apple Silicon/Intel macOS App 链接、系统快捷键注册/冲突/按住与释放、TCC 与真实音频、真实 USB、真实 LaunchAgent 清理、签名/公证/DMG/updater 的实际安装升级卸载。原先 Windows GUI/合成 Actor 报告保留原构建标识，不冒充本次重新执行。未修改正式安装、真实登录项、凭据或用户数据；原 `.tmp/` 无关工作保留。
