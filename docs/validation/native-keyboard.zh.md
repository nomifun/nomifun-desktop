# 语音键盘与应用维护验证

## 模型选择范围回归（2026-11-09）

只读核对用户当前模型配置与语音键盘设置：普通 StepFun 的 `stepaudio-2.5-asr-stream` 已启用，明确保存 `speech_recognition` / `stepfun.asr_stream`，使用普通 `/v1` 连接，没有健康失败观察；语音键盘保存的是 `asrMode: platform`、`asrModel: null`。旧页面按当前方式隐藏另一目录，所以合法的流式模型无法在普通方式下被发现；独立目录缓存还没有订阅供应商变化与后端重连。

ASR 下拉现在同时按流式、录音后识别分组；用户选择模型时，只提交一次包含精确引用与相应方式的设置更新。切换方式不自动挑选其他模型；缺少候选提示以两个目录的并集为准，就绪仍按已保存方式及精确引用校验。配置变化和重连刷新目录，删除或禁用后保留不可用引用。普通 ASR、美化手选目录复用本地调用校验，但不因临时健康失败消失；自动路由继续过滤健康失败模型。

同类审计确认并修复文件转写、聊天录音入口和桌面伙伴 ASR 只按任务标签放入不支持执行器的问题；选择器从后端 manifest 筛选执行器/传输方式，后端在上游请求前再检查并固定配置。伙伴保留其支持的 Deepgram 增量协议。视觉默认模型下拉补齐调用端要求的工具调用能力约束。协议目录失败属于未确定状态，保留已存标签并显示重试；共享后端重连订阅恢复失败请求，不把一次读取失败变成永久空列表。

合并后的 11 个前端测试文件为 207/207 通过、1405 个断言，覆盖语音键盘设置/自动保存/就绪、跨方式选择、目录刷新、视觉默认范围、语音协议范围、错误重试与共享重连。定向 Rust 验证：模型手选健康过滤 1/1、自动默认选择 3/3、流式 ASR 协议 23/23、文件 STT 集成 9/9、STT 错误语义 2/2、伙伴语音 8/8 通过。首次 STT 集成运行的四个上游替身用例等待超过 60 秒；精确停止测试进程后，将该本地夹具客户端设为无代理及 5 秒超时，完整重跑通过。该调整不改变生产网络配置。

桌面桥接以 `cargo test --profile dev --locked --offline --no-default-features -p nomifun-desktop --bin nomifun-desktop keyboard::models::tests -- --test-threads=1` 执行，7/7 通过，覆盖失败健康观察后的两类目录、协议隔离、普通识别提交/轮询、取消及错误诊断；本轮定向 Rust 共 53 项通过。typecheck、桌面 UI 边界、语音边界、i18n 和文档检查通过。

隔离预览 `ui/test/voice-keyboard-preview.html?state=model-scope` 使用相同方式/模型名组合，所有设置、目录与音频行为仍为内存模拟。此次内嵌浏览器因连接超时未取得渲染证据，独立浏览器启动被工具审批策略拒绝；没有以组件交互测试代替原生窗口验收。临时预览 HTTP 服务已精确停止。没有修改用户真实语音设置、读取密钥、录音、调用真实云端模型或替换已安装应用；实际语音转写和其他平台本轮未执行。

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

第一期提交 `7e1981204` 后加入 `externalKeyboardEnabled`（默认 false）和六种连接状态：disabled、starting、connecting、connected、stopping、error。软件快捷键总开关与设备模式独立，K2/K3/K4 的硬件偏好保留且在界面显示。当前 K1 控制 NomiFun 主窗口，K4 通过按住说话打开字幕板，软件字幕快捷键独立控制字幕；映射纠正与本次证据见下文。Linux 窗口验证未在本机原生执行。

已执行的第二期定向检查：语音/协议核心 67/67、桌面 `keyboard::` 74/74（包含 5 项适配器测试）、UI 183/183、typecheck、i18n（8574 键）、桌面边界与完整 `bun run check` 均通过。适配器测试使用实际 `Transport::start_validation` 工作线程和文件锁，包含默认关闭不创建运行目录、成功连接后关闭并 join、阻塞启动竞争关闭/退出后不得发布迟到实例，以及等待互斥锁的旧关闭请求不得停止新实例。它们不枚举 USB、不打开真实串口。

设置页实际组件在 1280×900 / 880×600 下验证可选区默认关闭、保留硬件偏好、K4 关闭提示和停止期间等待原生释放。各连接状态均不阻断软件识别就绪；关闭状态只在原生 disabled 后确认。该界面夹具的设备、麦克风与模型均为模拟，未登记 OS 快捷键。截图与报告：`C:\Users\rika0\AppData\Local\Temp\nomifun-voice-keyboard-ui-phase2-8f8511452b5c4bcb9ed5957ba7610ce1\`，临时浏览器/Vite/夹具已清理。

补测实际出厂默认快捷键时发现 Ctrl+Shift+Escape 被 Windows 保留，已将默认取消改为 `CommandOrControl+Shift+X`。更新的原生快捷键用例先验证全部四个 `Settings::default` 绑定真实注册成功、随后全部释放，再执行 F23/F24 冲突/按下/松开/重绑/注销验证；不向默认组合注入输入。最终 `native-report.json` 确认 `defaultPlanRegistered=4`、`defaultPlanReleased=true`、`ok=true`。报告与源码/二进制哈希保存在前述 `nomifun-voice-shortcut-native-minimal-pcos6oc_` 目录，早期 F23/F24 报告另存 `native-report-phase1.json`。

最终前端生产构建 ID 为 `defe4152-17ed-40f5-b2ff-abc42570b369`，API 合同版本仍为 54。真实麦克风、云端供应商、实体外置设备、macOS/Linux 原生执行与正式打包发布不属于这些夹具的通过证据。

最终第二期原生语音报告：`C:\Users\rika0\AppData\Local\Temp\nomifun-voice-phase2-20261108-423bcf9a\native-streaming.json` 与同目录 `native-platform.json` 均通过，进程退出 0、未超时、本次存活进程 0。每种模式包含 10 组软件入口约束、14 组外置适配器约束及 4 类更新恢复。外置约束实际验证默认无 owner、主动开启后单一 owner、快捷键关闭仍可使用 K1–K4、设备丢失只停止硬件采集、可恢复故障保留发现线程、先使用硬件后进行 UI 本地检测仍隔离、USB 故障和关闭不影响软件采集、终止实例不能由迟到启动消息复活、关闭完成时 owner 已 join、旧帧在关闭/重开后仍被拒绝，以及硬件专属设置不取消软件等待识别。等待流式连接子用例仅适用于 streaming，platform 报告明确不适用。

两报告记录相同二进制 SHA-256 `e10af04c3ff409f4fa26215f0d2000091318eb02595eace41474a833263f0283` 和最终前端 UUID。音频为合成 PCM、设备帧为隔离注入、传输工作线程/文件锁为真实运行，供应商为 loopback fixture；不把它们写成实体串口、真实麦克风或云端成功。最终控制器 OS 例子亦已按最终 Settings 源码 SHA-256 `8f2ec8e4b5d059c7698a1d86912d8655425195a7786f48732795d8c28a1de72a` 重建并复跑，默认四组注册成功；其二进制 SHA-256 为 `933436ee882ec1fafb04cf920784c37d337a0fb02c7763f24ffb0c64ff9698d9`，运行前后源码未变。

上述第二期报告包含当时错误的 K1 字幕切换断言，不作为当前 K1 主窗口映射的验收。该断言与脚本接纳字段在本次纠正中替换，新的映射证据另列，不沿用原报告的通过结论。

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

## 开发启动实测回归（2026-11-08）

收到 `bun run dev` 无法启动后，本机实际执行完整入口。Vite 正常就绪，但主程序尚未运行即在 `opusic-sys 0.7.5` 的 C 构建失败：缓存将 `HAVE_SMMINTRIN_H` 记为 false，同时启用 AVX2，导致 `x86_silk_map.c` 引用未声明函数。独立 MSVC 14.44、Ninja 与相同 Cargo C 参数的新探测全部通过；仅强制该缓存负值即可复现同一错误，仅移除该负探测即可恢复。首次探测无诊断退出的原因仍未知。

保留失败现场并只隔离本库的生成 CMake build 子目录后，实际 `cargo build -p opusic-sys` 默认 SIMD 构建通过（7.55s）；原有 Opus 编解码采样数量/能量往返测试 1/1 通过。没有修改注册表中的依赖源码、升级库版本或关闭 SIMD。

另修复两个开发入口问题：Windows 子进程在无显式覆盖时使用已验证的 512 MiB Rust 编译栈；开发应用按 profile 选择独立、不可变、位于数据树之外的 IPC，跳过正式版历史清理。正式版/独立安装维护工具保持已发布目录，继承开发环境变量不会让 helper 加入开发 IPC。Tauri 的 `cfg(dev)` 取自依赖的 custom-protocol 模式，而不是 debug/release 优化级别。

定向检查：开发启动器 21 通过、2 个 macOS lifetime 用例按平台跳过；IPC 路径/作用域 11/11 通过；Opus 原生编解码往返 1/1 通过。没有改变 Session 数据、存储搬迁或重置实现。

实际再运行 `bun run dev`：只指定新建临时 `NOMIFUN_DATA_DIR`，保留正常 `LOCALAPPDATA`，编译完成后产生开发进程 PID 51464，主窗口存在且响应，Vite HTTP 200，已观察到超过 100 次界面发起的 API 200 响应。安装版 PID 6564 同时存活，其全局桌面端点仍属于 6564；开发端点位于独立 profile hash 目录。最后通过相同开发 scope 的 `--app-maintenance-stop` 正常关闭，命令和完整 dev runner 均退出 0，5173 已无监听，安装版仍运行。验证不依赖关闭安装版或删除旧开发数据。

证据保存在仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-dev-repro-e207b65176674605ae2b17cfd9ee57c1\`：初次/恢复日志、`startup-report.json`、正常退出日志、Opus 构建/往返结果。受控缓存缺陷及恢复报告在 `nomifun-opus-fresh-probe-osVEqL`。失败生成目录已改名保留，没有递归删除整个 target 或 build.noindex。

运行中同时观察到开发更新源 localhost:59999 不可达（开发配置明确无更新 feed），以及外部非受管知识 MCP broker 的用户级管道创建警告。后者的名称按 SID 共享，与安装版共存可能争用，未实际打开管道确认持有者；它不属于此次启动失败，知识 HTTP 与受管能力入口仍初始化。本轮没有放宽管道 ACL/首实例保护或修改该外部 MCP 路由，其多 profile 注册不是本次启动验收范围。

本轮证明 Windows 开发窗口、前后端初始化、开发/安装版维护隔离及正常退出；不将其扩大为完整模型、音频、USB、外部 MCP 或其他操作系统的功能验收。测试使用临时开发 profile，正式安装与原 `.tmp/` 无关工作保留。

## 开发版实体串口连接排查（2026-11-08）

用户确认“等待设备”截图来自 `bun run dev`。Windows 实际枚举出状态正常的 COM4（USB VID 303A / PID 1001）；只读进程句柄快照证明正式安装的 NomiFun 持有对应 `\Device\USBSER000`，此时独占打开 COM4 返回错误 5。开发/正式版 IPC 隔离不意味着实体串口可共享。这次占用证据只解释设备连接，不替代先前安装文件替换失败的独立复现。

经用户明确授权，调用安装版应用维护正常停止命令，确认该安装的进程退出。随后使用 serialport 4.10.1 的真实串口探针，以生产相同的 115200 与 DTR=false 打开 COM4，仅发送 transport/get_device_info。实体固件返回硬件 vibe_keyboard、固件 `0.3-screen.1`、native protocol 1，以及 policy_ack/wait_release/system audio 能力，证明本机 USB 分类和当前原生固件协议兼容；没有刷写固件、切换 DTR、录音或调用模型。

源码根因是发现阶段将匹配端口的 open 错误直接忽略，最终转换为无设备。现保留所有候选端口的完整错误链和自动重试提示，仍优先尝试后续可用设备。核心 71/71 通过，包括无匹配不打开、首端口占用继续尝试、全部失败保留真实原因、释放后下次扫描恢复四项回归。UI 新增 connecting + COM4 占用详情用例 1/1 通过（10 个断言），验证显示原因且软件快捷键、云端测试入口不受影响；桌面 UI 边界检查通过。

按相同临时开发 profile 再次运行完整 `bun run dev`，实际开发主窗口的外置连接区已显示 COM4「已连接」与固件 `0.3-screen.1`，系统快捷键同时「已就绪」。这是实际 Tauri 窗口与实体串口证据，不使用模拟设备状态。日志与仅含元数据的协议探针保存在仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-keyboard-physical-cd0d96c8c26a46769490975dfd5dc200\`。

用户实际按松 K1 后确认字幕窗口出现，证明当时的实体按键传输与 Actor 分发已通，但同时指出该应用映射错误：K1 应显示/隐藏 NomiFun 主窗口，K4 才负责字幕板。该次结果不作为纠正后 K1 主窗口行为通过的证据。真实麦克风、云端 ASR 与其他平台仍需单列结果；串口握手或界面已连接不作为这些功能通过的证据。正式安装的文件、模型凭据与原 `.tmp/` 无关工作未改动。

## 实体 K1 主窗口映射纠正（2026-11-08）

恢复原有跨平台 `platform::toggle_main`：主窗口在前台时隐藏，隐藏、最小化或在后台时唤回。Windows 使用根 HWND 的前台身份，覆盖 WebView2 子窗口持有键盘焦点的情况；macOS 保留 AppKit 主线程唤回，Linux 保留 GTK present。K1 分发只调用主窗口控制，窗口调用失败保留为外置连接局部诊断，不修改字幕正文、字幕显隐或当前语音操作。系统字幕快捷键仍控制字幕；K4 仍通过原按住说话链路打开字幕板、松开结束识别。

Windows 实际以原临时 profile 运行更新后的开发程序（PID 52044），实体 COM4 再连接成功，设置页显示纠正后的 K1/K4 说明。用户按要求先点击 NomiFun 主窗口，再连续两次按松实体 K1，并明确确认“两次分别隐藏、唤回，正确”。该结果证明本次 Windows 实体按键到主窗口的实际链路，没有用注入帧或系统快捷键替代按键。

用户另提供本机固件仓库 `C:\Users\rika0\code\esp32\keyboard`，只读核对其 HEAD `40d041d`：设计文档的固定操作表明确 K1 为主窗口唤回/隐藏、K4 为系统语音按下/松开；`vk_usb_native.c::send_button` 仅发送原生 down/up/click 与 k1–k4 身份，业务行为由桌面端分发。该仓库当前源码版本为 `0.3-screen.2`，本次实体握手仍为 `0.3-screen.1`，分别记录，不把源码版本当作已刷写版本。没有修改固件仓库或设备固件。

这次实体复测使用 `bun run dev --no-watch`，避免并行修改验证源码时反复重启窗口。之前普通 watcher 在后台重启时触发 Vite/Node 的未处理 TCP `ECONNRESET`，原日志仍保留；该开发服务器故障不计作 K1 映射回归通过，另行核查。日志位于仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-k1-main-window-57a3cfd935994a54b97775ca93ae7159\dev-k1.log`。

UI 映射与外置状态相关 7 项测试通过（44 个断言），翻译生成器 6 项自测及 8575 个键检查、桌面 UI 边界检查通过。Linux 窗口夹具恢复专属 880×600 主窗口的激活/隐藏验证，并检查字幕显隐、位置和正文不变；本机只有 Windows，Linux 原生执行与 macOS AppKit 运行均未进行，不能将源码恢复视为对应平台验收。

原生 Actor producer 的 K1 字幕/文稿/录音变动负例测试 1/1 通过；报告接纳规则 4/4 通过，拒绝旧 K1 字幕映射报告和缺少独立软件字幕、K4 录音、活动采音隔离证据的报告。该纯检查结果与实际原生窗口执行分别记录。

更新后的 Windows 原生 Actor 实际执行 streaming / platform 两种模式均通过，主进程退出 0、未超时、私有 Job 清理后本次存活进程 0。专属主窗口实际覆盖前台隐藏、再次唤回聚焦、最小化恢复，核验字幕显隐、位置、尺寸、文稿与操作身份不变；在 K4 合成录音期间再次执行 K1 往返，原采音保持。K4 的共用录音/松开识别、独立软件字幕快捷键及原有更新恢复回归同时通过。使用真实 Windows/Tauri 窗口、明确合成 PCM 和 loopback 模型，不打开 OS 麦克风或实体串口。

两份通过报告为前述 `nomifun-k1-main-window-57a3cfd935994a54b97775ca93ae7159` 目录内 `native-streaming-green.json` / `native-platform-green.json`，均记录二进制 SHA-256 `179793af6ad5280b09604519ec6c48ae1e20681e25434fc339fffd857b451b77`。原生例子沿用现有前端产物 stamp `defe4152-17ed-40f5-b2ff-abc42570b369`，不把它标为本次新 UI 构建；本次设置文案由实际开发窗口与 UI 测试验证。

首次完整 streaming 报告另存 `native-streaming-red-projection.json`：K1 检查已通过，随后旧的本地检测维护断言要求 UI 快照同步更新，产生失败。原生生产者先确认停止、再投递 Actor 状态，维护按真实停止证明返回符合当前契约。夹具改为只读检查同一原生所有权证明，再等待 UI 投影；不改变生产停采规则。修正后上述两模式复跑通过，保留失败证据。

## 开发热重启的 Vite 断连回归（2026-11-08）

本次普通 Tauri watcher 重启后端时，Vite 6.4.3 / Node 24.18.0 曾因未处理的原始 Socket `read ECONNRESET` 退出 1，连带终止 beforeDevCommand。独立随机 loopback 夹具复现：非 HMR 的 upgrade 请求没有连接错误处理者，客户端 RST 触发相同异常栈。原事故没有保存 URI，因此只证明同类连接缺口，不能唯一指认原始请求。

增加仅 `apply:serve` 的服务器 upgrade socket 守卫：已有 WS/proxy 处理者继续处理；无人接管的断连只结束本连接并记录原因，意外错误保留错误码与堆栈。不添加进程级异常捕获，不改变 Host/Origin/token 检查或依赖版本。6 项测试、25 个断言和 typecheck 通过；独立 Vite 与项目原配置均在真实 RST 后继续 HTTP 200、认证 HMR 101。证据含源码哈希，位于 `C:\Users\rika0\AppData\Local\Temp\nomifun-vite-reset-c469b12374bf4d149e23091428945acb\report.json`。

随后正常停止 no-watch 开发实例并退出 0，恢复普通 `bun run dev`。实际 watcher 经 `mod.rs`、原生验证例子及内容不变的专属窗口例子时间戳三次触发后端重启，三次均记录守卫接住 `ECONNRESET`，Vite 持续在线，最终 `/` 与 `/@vite/client` 均为 HTTP 200，开发主进程 PID 40920 存活并响应 API 200。完整日志为同一 K1 证据目录内 `dev-watch.log`。该结果是实际热重启证据，不以临时关闭 watcher 作为最终修复。

## 真实 StepFun ASR 握手与额度拒绝（2026-11-08）

用户按住实体 K4 后，旧提示仅显示“暂时无法连接语音识别服务，尚未开始录音”。实际日志在收到按住后约 150ms 报准备失败，`active_capture=false`、没有麦克风 owner；错误发生在 ASR 准备阶段，不能据此认定串口断开或录音设备失败。旧 Actor 将 typed InvokeError 转成字符串后只按关键词分类，认证、HTTP 拒绝甚至含 `WebSocket` 的网络失败都可能误标 `protocol`，并且丢弃具体原因。

只读核对本次已存模型配置：普通 StepFun、官方 `/v1` 基础地址、`stepaudio-2.5-asr-stream` 与 `stepfun.asr_stream` 匹配，无自定义 endpoint 或额外供应商参数。官方当前[双向流式 ASR 文档](https://platform.stepfun.com/docs/zh/api-reference/audio/asr-stream)仍支持此模型，16k PCM 与客户端 commit 的配置符合规范；未据此猜测模型下线或改动固件。

从实际开发窗口的“供应商与密钥”展开同一已存模型，执行“健康检测”。该既有入口只做相同流式握手并关闭，不打开 OS 麦克风、不上传环境音频；它会写回能力健康观测。真实请求在 177ms 后得到 `HTTP 402`，脱敏后的服务端原因为 `You exceeded your current quota, please check your plan and billing details`，健康分类 `insufficient_quota`。这证明当前 API Key 所用服务账户/项目受到额度或计费拒绝；没有读取或修改密钥、充值、切换供应商或静默改用其他可能计费的模型。远端具体余额、预算与账户状态未查询。

修复保留 typed kind、HTTP 状态、已有脱敏边界内的有界供应商诊断；HTTP 402 显示中文额度/余额排查提示，失败前后仍明确未开始录音或保留已有正文。生命周期日志只记录类型、HTTP 和真实失败代次，不记录服务端正文、用户词典、声音或凭据。握手端复用统一有界错误解析与凭据脱敏，严格配置回执及原有录音准入不放宽。

本次定向协议测试 `cargo test --locked --offline -p nomifun-model-invoke streaming_asr::tests --lib` 为 23/23 通过，包含 HTTP 拒绝、错误响应体上限、服务端错误与密钥回显脱敏、严格回执和网络原因。桌面 `keyboard::models::tests` 6/6 通过，覆盖 typed 分类、HTTP 402、提示边界、流式/文件协议隔离与原有模型操作；以 `cargo test --profile dev --no-default-features -p nomifun-desktop --bin nomifun-desktop keyboard::models::tests -- --test-threads=1` 执行，只运行所选测试。桌面检查日志保存在仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-asr-handshake-40d6d2c28c224521922e2dfca854790f\desktop-model-tests.log`；原始运行日志仍在前述 `dev-watch.log`。账户额度恢复后的真实语音转写、macOS/Linux 本次运行未验证，不能将模拟测试或一次额度拒绝写成真实识别成功。

## 语音键盘配置页重构（2026-11-08）

本次只重构 renderer 的信息结构、控件布局和呈现，不修改后端、模型/设备协议或用户已保存的语音偏好。实际重构前截图确认：首次检查和错误信息占据大半首屏，识别、词典、美化和测试混在长页，核心模型与麦克风需要多次滚动才能配置。新页面采用四个任务分区，识别模型与麦克风直接呈现；快捷键和可选设备在较宽内容区并排组织，窄内容区使用单列，字幕样式有字号预览，本地/云端检查在独立分区明确说明音频去向。低频参数与技术详情折叠，保存失败仍保留草稿和重试入口。

本次定向 `KeyboardSettings.test.tsx`、`keyboardSettingsAutosave.test.ts`、`keyboardReadiness.test.ts` 共 130/130 通过、606 个断言。原组件 58 项场景保留，新增分区/草稿/检查 owner/错误详情/默认折叠和缺少输入设备的交互回归；没有以隐藏元素查询跳过实际导航。typecheck、i18n（8606 键、6 项自测）、桌面 UI 边界、theme/theme-vars、dead-css 检查通过。共享 PageHeader 的 18px/26px 标题契约保持。

使用 in-app browser 运行真实页面与224px设置导航的隔离预览，在1280×900与880×600、浅色/深色、中英文检查布局、控件边界和标签键盘导航。实际走过本地模拟检查跨分区继续/停止、保存失败后草稿与重试保留、错误摘要进入技术详情、设备占用局部显示，以及无输入设备的就地提示。页面宽度未超过视口；抽查正文/说明文字的浅色对比度为14.55/6.58，深色配色亦已目视检查，不将抽查宣称为完整无障碍合规。

预览入口为 `ui/test/voice-keyboard-preview.html`，所有原生、权限、设置与音频行为均为明确内存模拟，未知fetch不回落真实后端。场景选择使用同源完整导航，避免产品的跨挂载草稿恢复协调器污染另一个测试场景。截图、审查记录和测试日志保存在仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-voice-settings-redesign-RMoQW2\`；遮挡截图已在审查记录中标为拒绝，不作为通过证据。本轮不录制环境声音、不调用真实云端模型、不改变系统权限，macOS/Linux原生窗口没有重新执行。

Windows 真实开发窗口已确认显示四个分区、原有 StepFun 流式模型和系统麦克风；与重构前同尺寸截图比较，核心识别模式、模型和麦克风现已同时出现在首屏。最终原生截图为 `14-after-native-top.png`，没有在真实窗口启动录音或修改语音设置。收尾时 in-app browser 控制连接超时，未能确认预览标签常驻和视口重置；此前的视口/交互证据已保存，原生窗口核查完成，隔离预览HTTP服务仍可独立访问。

## 快捷键键帽与编辑状态（2026-11-08）

默认用真实 `kbd` 元素显示组合，Windows/Linux 的 CommandOrControl 显示 Ctrl，macOS 显示 ⌘；原始字符串只在点击后的编辑框出现。显示别名不回写存储，未知或不完整输入不静默丢弃。预览与输入共用网格占位，失焦只结束编辑，Enter 在非 IME 时结束编辑并回焦预览；四项仍统一 Apply，录音期间禁止编辑。

组件回归 72/72 通过、493 个断言，涵盖 macOS 键帽、四项原值读取、空绑定、失焦/Enter 不提交、IME、跨分区草稿、冲突及禁编辑。typecheck、i18n（8608 键）、桌面边界、theme/theme-vars、dead-css 与图标检查通过。Windows 真实开发窗口确认 Ctrl + Shift + K / Space 的键帽样式，点击后仍为 `CommandOrControl+Shift+K`，不改内容按 Enter 返回键帽，Apply 保持禁用；没有注册新快捷键或启动录音。截图位于仓库外 `C:\Users\rika0\AppData\Local\Temp\nomifun-shortcut-keycaps-NLlZoZ\native-keycaps.png`。macOS 仅有组件渲染证据，本轮没有 Mac 原生运行。
