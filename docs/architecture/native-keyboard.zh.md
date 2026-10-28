# 原生四键字幕板实现

本文件维护当前原生四键字幕板的运行边界。2026-10-28 用户决定将键盘收敛到 NomiFun 主进程：不再独立运行设备守护进程，不再通过 K1 唤起已经退出的应用。配套 `keyboard` 固件继续提供固定四键和 native v1 协议，无需改变硬件职责。

## 最终用户流程

键盘只发固定职责的 k1–k4，无需校准；录音来自 OS 默认或指定麦克风。NomiFun 运行时 K1 显示/聚焦或隐藏主窗；主进程退出后四键均失效；按住 K4 等待真实聆听提示后说话，松开正常识别本段；K2 无工具美化全文；K3 取消当前准备、录音、识别或美化，保留已定稿正文和窗口，空闲时无副作用。

ASR 显式支持真实流式 partial/final 和停录后提交一次的云端普通识别。同稿可在当前 UTF-16 光标插入、替换明确选区，无光标时直接续接，不添加录音边界换行；正文支持显式编辑及 IME。录音中 K2 先停止、收尾、final 再美化，K3 立即取消当次处理并丢弃临时结果，不等待 final 或美化候选。失败或取消保留此前完整正文，旧请求不能迟到落稿。

焦点在字幕内时，Ctrl+C / Command+C 按系统选区复制，不自动改为全文复制；不注册全局快捷键，不增加 Rust 剪贴板命令或 owner。用户在目标应用自行粘贴，不属于 NomiFun 的实现或快捷键。Esc 进入关闭路径，真实停采后隐藏并清空本轮，下次录音创建新文档。IME 组合中的 Esc 先由输入法处理，不同时关闭字幕。

字幕是 720×112、最小 360×80 的独立透明歌词窗口，系统字体默认 14、可调 12–32。默认只呈现有色文字，hover/focus 显示工具条，美化时字形光晕呼吸。顶部居中的琥珀/青绿/紫罗兰提示分别表示准备、真实聆听、收尾/识别。其他应用持续前台焦点不作为一期要求；主工作台 880×600 和 K1 的聚焦职责保持。

## 唯一运行链

```text
NomiFun 主进程
  ├─ 串口工作线程（唯一 owner、native v1、永久 NONE、释放门禁）
  │     ↕ 有界进程内命令与事件
  ├─ 键盘 Actor（字幕、OS 输入、模型、设置）
  │     ↔ 独立字幕 WebView / 拓展键盘设置
  │     ↔ 现有模型目录、供应商连接、凭据
  └─ 正常退出：确认真实停采 → 关闭串口 → join 工作线程 → 退出进程
```

串口线程由主程序创建和持有，不创建独立进程、登录项、设备 IPC 监听端口或主程序心跳。K1 仅操作已经运行的主窗口；隐藏窗口不会退出进程，显式退出 NomiFun 会结束全部键盘功能。设置页显示设备连接和当前进程内的就绪状态。

安装器的短命维护工具仍通过鉴权端点要求主程序真实停采并退出，再替换文件。该端点服务应用安装维护，不转发按键，也不拥有第二条录音链。已有版本的独立设备服务与其登录登记在接管串口前按安装归属退役，只允许关停、移除，不启动、恢复或继续提供键盘功能。

| 代码 | 职责 |
| --- | --- |
| `crates/backend/nomifun-keyboard/src/protocol.rs` | 有界 JSON 帧、按键名解析及板载音频首部显式拒绝 |
| `crates/backend/nomifun-keyboard/src/service.rs` | 严格 native v1/system 身份、主进程内唯一串口 owner、有界通道、NONE/Rearm、显示状态和可 join 的停止 |
| `crates/backend/nomifun-keyboard/src/desktop_control.rs`、`ipc.rs` | 安装维护用 GUI 停采端点、进程生命锁、鉴权和退出证明 |
| `crates/backend/nomifun-keyboard/src/legacy_service.rs` | 已发布旧设备服务的单次鉴权关停；没有运行或恢复入口 |
| `crates/backend/nomifun-keyboard/src/maintenance/`、`maintenance.rs`、`bin/nomifun-keyboard-maintenance.rs` | 安装期间的启动门禁、文件替换/回滚、目标安装归属与主程序运行恢复 |
| `crates/backend/nomifun-keyboard/src/document.rs` | 文档/revision/操作状态机、选区插入、美化与取消协调 |
| `apps/desktop/src/keyboard/mod.rs` | 按键/按钮共同 Actor、编辑保存屏障、音频与模型任务身份、字幕生命周期 |
| `apps/desktop/src/keyboard/audio.rs`、`capture_input.rs`、`audio_dsp.rs` | 唯一原生输入 owner、首回调就绪、真实停止回执、有界 PCM 与重采样 |
| `apps/desktop/src/keyboard/key_hold.rs` | K4 Released/Waiting/Consumed 意图，跨上一段收尾接续及撤销 |
| `apps/desktop/src/keyboard/asr_preparation.rs` | 可取消预热、绝对期限与单次完成回执 |
| `apps/desktop/src/keyboard/models.rs` | 复用现有目录/连接/凭据，双 ASR 与无工具全文美化 |
| `crates/backend/nomifun-model-invoke/src/text_transform.rs` | 单次 HTTP 文字转换，纯协议编码/解码，取消、输出预算和阶段计时 |
| `crates/backend/nomifun-model-invoke/src/streaming_asr.rs` | 真实供应商流式协议与有序 partial/final |
| `apps/desktop/src/keyboard/platform.rs`、`lifecycle.rs` | 三平台窗口、旧登录登记清理、会话变化及主进程退出 |
| `ui/src/renderer/entry.tsx` | 隔离字幕入口，主工作台不加载字幕专用入口 |
| `ui/src/renderer/pages/keyboardSubtitle/` | 唯一透明字幕、DOM/UTF-16 编辑、选区与真实状态 UI |
| `ui/src/renderer/pages/settings/KeyboardSettings.tsx` | 连接/固件、逐键开关、OS 输入、双 ASR、文本模型、字幕与主进程内就绪状态 |
| `apps/desktop/keyboard-packaging/`、`apps/desktop/nsis/keyboard-hooks.nsh` | 换文件前停采/停服与卸载清理 |

设置存于现有键盘设置文件，模型只保存现有引用；不增加第二模型目录、凭据库或 AgentSession。系统选区复制由 WebView2、WKWebView、WebKitGTK 各自原生编辑路径处理，字幕 capability 限于本窗口必要命令。

K2 直接调用 `ModelInvokeService::transform_text`：仅当前全文与美化指令，单次 HTTP 请求，不装配 Agent、不读取全局或项目 TOML、不重试、不切换密钥或模型。复用现有模型目录、凭据和无重定向连接池；只按精确模型已声明的能力选取最低可用思考档，未知能力不猜参数，输出预算受正文大小和模型上限共同约束。收到协议完成标记即返回，不等待连接空闲或 EOF；截断或异常仍保留原文。美化候选在原有字形中流式预览，只有完整成功才提交正文。

Windows 在主程序初始化时后台预备既有 COM 枚举器，不创建或播放录音流。每段仍独立真实开麦和停采；数字阶段计时区分运行期初始化、设备/格式解析、建流、播放、首输入与停止。文字调用计时区分配置解析、响应头、首帧、首个正文和完成，并记录供应商实际返回的 token 用量；不记录正文、音频或密钥。

## 并发与生命周期约束

K4 物理意图与录音/识别资源分离：忙时保留仍按住的 Waiting，前段定稿和真实停止后按新 revision/光标接续；松开撤销，开始采集才 Consumed。停止/取消/关闭/禁用/编辑失败撤销等待。K3 消费当前按住，不允许仍按住的 K4 在取消后自动重录；旧稿关闭不能自动开新稿。

所有异步任务绑定 document ID、revision、录音/操作 ID、激活代次；旧结果不能清掉新任务、写回新稿或关闭新窗口。编辑 begin/save/end 的 promise 还须核对任务归属，换稿释放旧 ref，旧 finally 只能释放自己。K3 不等待编辑 flush 或模型完成；取消退休当前操作身份、待执行美化和临时结果，空闲取消保留正文、原文、revision 和窗口。跨端取消 generation 同时作废 UI 在编辑保存后排队的自动动作，保留用户草稿，但旧保存应答不能重新触发已取消的美化或起录。

取消与 Esc/关闭是当前文档的控制意图，按 document ID 定位，不因同文档编辑保存刚增加 revision 而被拒绝；新文档仍严格隔离。取消另核对 cancel generation，旧代次重复取消成功返回但无副作用。编辑、录音插入和美化等正文操作仍校验匹配的 revision。

同一主进程内串口每次重新打开后先清空原生接收缓存，再新建解析器并发送 transport 握手；清理失败不建立新连接，避免上次中断的帧与新 epoch 首帧拼接。

真实输入首个非空回调才发布聆听；全零也能证明启动，但不能制造识别成功。首回调等待 5 秒，原生/Actor 启动边界 15 秒，ASR 准备绝对期限 20 秒；首次按住 K4 真正取消未完成预热并新建本次请求。超时、错误和取消都保留未完成 owner 直至真实停止，不能仅丢弃音频假报停止。

ASR 后台预热连接失败仅回收连接并退避，不覆盖正文、编辑/美化任务、通知或已有操作错误。起录和活动识别失败经过统一中文提示及真实停采路径，临时内容与待执行美化撤销。流式输入通过独立不可丢失的结束信号封口，排空已入队 PCM 后唯一 commit；terminal 关闭准入，内部输入故障与显式取消区分，不复用已结束的 sink。StepFun 完成按实际创建的 buffer/source 身份关联，不能凭唯一候选猜测最终结果。

StepFun 不强制 commit 后二遍纠错；未配置时使用供应商默认关闭，显式 `full_rerun_on_commit` 配置仍保留。两种配置都必须收到匹配的 committed 与 completed 后定稿，不以 partial 代替完整结果。

K3 在准备阶段撤销连接和起录，在录音阶段真实停采并丢弃本段，在识别阶段撤销提交/轮询和本地结果准入，在美化阶段取消请求并丢弃候选。普通 ASR 尚未上传时清空缓冲、不发请求；已提交的云端任务及流式已发送音频无法回收，但本地不再等待或接受其结果。K4 正常松开仍执行识别，不引入音量阈值猜测取消意图。

禁用总开关或 K4 取消活动采集、阻止新录音，保留已定稿正文；总开关保留逐键偏好且不影响 K1。K2/K3 单键开关只更新主机操作授权，不重建设备释放屏障，不打断当前 K4 录音或吞掉松键。取消/关闭/锁屏/休眠/输入或设备丢失/退出撤销处理，恢复后重新按键。主进程退出先确认系统麦克风真实停止，再停止并 join 串口 owner。安装更新通过鉴权端点确认这一退出流程；失败或超时阻止换文件。旧独立键盘登录登记只在属于目标安装时清除，保留用户数据。

Windows 安装器携带独立的 `nomifun-keyboard-maintenance`，在私有临时目录运行，不向未知旧版 exe 发送新命令，也不初始化数据库、模型或音频。安装包先完整解压新文件（含卸载器），维护进程持有 `maintenance.lock`、GUI 生命锁与设备 owner 锁，覆盖停服、文件替换和成功/失败/取消收尾。更新不先卸载旧版；只备份、替换本次 payload 声明的文件，失败或取消时逐文件核对恢复，不枚举用户数据。残留端点仅在取得生命锁后清理；旧版没有键盘服务是正常状态，旧 GUI 仍经安装器的应用退出检查处理。

应用内更新使用可撤销的键盘暂停，撤销按住意图、等待真实停采、不等待云端完成，不把预检当成永久退出。预检期间保留当前串口线程；取消以新 generation 重新走释放屏障，不重建进程或自动恢复录音。预检失败或调用取消时恢复 Actor 准入并保留已验证安装包，系统锁屏状态仍有效，不自动开始录音。手动安装和应用内更新共用安装器维护协议；成功后只按原主程序运行意图恢复实际安装路径的应用，应用内更新的 GUI 重启保留 NSIS `/R` 和原始参数。卸载从删除文件开始后不再重启应用；无法确认回滚完整时要求修复，不能启动半更新程序。维护进程只在安装期间运行，不增加常驻服务或永久“更新中”标记。

字幕错误位于正文外，简短状态可展开详情，不加入字数、编辑或复制。设置页只显示当前配置请求和本页主动测试反馈；系统输入状态与 ASR 操作失败保持分离。

## 平台与部署

| 范围 | Windows | macOS | Linux |
| --- | --- | --- | --- |
| 生命周期 | 主程序持有串口线程；退出后键盘失效 | 主程序持有串口线程；退出后键盘失效 | 主程序持有串口线程；退出后键盘失效 |
| USB | COM、独占、稳定设备 ID | /dev/cu.*、独占与热插拔 | ttyACM/ttyUSB，303a:1001 专属 uaccess 规则 |
| 输入 | CPAL/WASAPI 默认或指定输入 | CoreAudio、麦克风用途声明与 audio-input entitlement | CPAL/ALSA；部署时确认默认输入与 PipeWire/PulseAudio 桥接和权限 |
| K1 | 原生显示/解除最小化，根 HWND 判断前台后隐藏 | AppKit 主线程激活或隐藏，Dock 重开 | GTK present/隐藏，由窗口管理器激活 |
| 字幕 | 主线程 ShowWindow/SetWindowPos、原生可见性与置顶 | AppKit 显式首击编辑与原生隐藏回读 | GTK map WebKit、显式点击编辑、工作区定位与隐藏回读 |
| 字幕编辑 | WebView2 系统选区复制、IME、Esc | WKWebView 的 Command+C、IME、Esc | WebKitGTK 系统选区复制、IME、Esc |
| 安装/更新 | 新安装包维护进程持锁停服、可恢复替换文件及恢复主程序 | 原生 updater 替换前停采并退出主程序；不登记键盘后台任务 | deb/rpm pre-install 以对应用户停止主程序和退役旧 owner |
| 卸载 | 包内维护工具停服并清理本安装拥有的 Run 值 | 随包 macOS 卸载脚本，先停应用并清属于该安装的旧登记 | deb/rpm pre-remove 区分升级/卸载；AppImage 使用随包脚本 |

Linux 默认窗口路径使用 XWayland，GNOME/KDE 会话都需要相应服务端及系统剪贴板桥接，字幕通过 WebKitGTK 原生编辑路径使用它。正式 deb/rpm 提供 XWayland 依赖和专属 udev 规则；AppImage 的宿主依赖与权限由用户环境提供，具体安装步骤见[打包说明](../../apps/desktop/keyboard-packaging/README.md)。纯 Wayland 无 XWayland 不属于当前窗口后端。

AppImage 的每次运行可能使用不同临时挂载目录；当前按操作系统实际可执行路径核对安装归属，运行中跨挂载的自动停机保守拒绝。更新或卸载 AppImage 前须先手动退出旧实例，没有活动参与者后可继续；不通过环境变量或文件名猜测归属。固定安装路径的 Linux 不涉及这一限制。

Windows 安装器每个维护阶段最多等待 90 秒，依次确认 GUI 停采退出、串口 owner 退出及文件替换/回滚；缺少串口 descriptor 不能跳过 GUI 停采。普通退出会结束串口线程，K1 不再唤起应用。直接删除 .app/AppImage 不触发卸载回调，删除前可执行随包脚本或 `--app-maintenance-unregister` 清理该安装的旧登记；用户数据保留。Windows 新维护流程的证据不能替代 macOS/Linux 原生包更新验收。

当前自动化、实机和未执行的平台证据见 [键盘验证记录](../validation/native-keyboard.zh.md)。过去独立服务架构的验证记录由 Git 历史保留，不作为当前单进程方案的验收结果。

## 构建与开发

| 范围 | 入口 |
| --- | --- |
| 核心 Rust | `cargo test -p nomifun-keyboard --lib --tests --locked` |
| UI | `bun test --cwd ui src/renderer/pages/keyboardSubtitle/KeyboardSubtitle.test.tsx src/renderer/pages/keyboardSubtitle/subtitleText.test.ts src/renderer/pages/settings/KeyboardSettings.test.tsx` |
| UI 合同 | `bun run typecheck`、`bun run check:desktop-ui-boundary`、`bun run check:i18n` |
| 打包生命周期 | `bun test scripts/keyboard-packaging.test.mjs` |
| Windows 安装原生回归 | `node scripts/validation/keyboard-upgrade-native-smoke.mjs --helper <绝对路径> --fixture <maintenance_fixture 绝对路径>`；独立 LOCALAPPDATA、真实 NSIS/锁/IPC，无音频、串口或真实登录项 |
| Windows 更新暂停恢复 | `node scripts/validation/keyboard-native-update-recovery.mjs --binary <绝对路径> --mode streaming --frontend-build-id <当前 UUID> --report <绝对路径>`；另运行 `--mode platform`，合成输入、私有 WebView/模型 fixture、独立 Job 管理全部子进程 |
| 原生 Actor | `keyboard_native_validation` example + `scripts/validation/keyboard-native-actor-smoke.mjs`（streaming/platform）；仅在 validation feature 和私有 profile 中注入合成输入 |
| X11/Wayland 窗口与系统编辑 | `keyboard_window_validation` example；`scripts/validation/keyboard-x11-smoke.mjs`、`keyboard-wayland-smoke.mjs` |
| 普通程序 | `cargo build -p nomifun-desktop --locked --features tauri/custom-protocol --bin nomifun-desktop -j3` |

自动化使用固定合成 PCM、本机协议与隔离窗口，不读取用户凭据、不调用云端模型或录制环境。覆盖 K3 各阶段取消、取消后恢复、迟到结果与字幕内系统选区复制/Esc 及 IME。原生窗口、输入权限、设备丢失、睡眠恢复和安装更新需要在对应平台检查。日志、截图和运行报告由调用方写入仓库外的私有输出目录，不纳入 Git。

### macOS 构建与部署检查

1. 在目标 Mac 为 `aarch64-apple-darwin` 和 `x86_64-apple-darwin` 构建普通应用，通过 `bun run build:mac` 检查打包、签名、实际 bundle ID、麦克风用途声明和 audio-input entitlement。
2. 安装后检查不存在新建的键盘 LaunchAgent；已有同安装旧登记在接管前移除。检查 K1 前台隐藏、后台聚焦和单串口 owner；显式退出后四键均无效且没有遗留设备进程。
3. 处理麦克风允许/拒绝及恢复，检查默认/指定输入、设备切换和丢失；两种 ASR 分别检查首次 K4、首尾音、松键定稿、取消及禁用/关闭停采。
4. 在 Retina、多屏和工作区切换下检查透明、尺寸、位置、状态与工具条、字号、IME/emoji/选区插入；在字幕获焦时检查 Command+C 复制选区，以及 Esc 关闭清稿、IME 组合 Esc 不关闭。分别在准备、录音、识别和美化时按 K3，检查真实停采、保留正文、拒绝迟到结果和下一次重新按住 K4 可恢复。
5. 检查锁屏/睡眠、USB 重连释放门禁、主程序退出、原生更新和卸载的串口释放及旧登记清理。共享逻辑测试不能替代目标平台的原生检查。
