# Linux 桌面原生路径与验证（2026-10-17）

本记录对应 WSL `Ubuntu-26.04`、`/home/muri/code/nomifun/nomifun-desktop` 本轮工作树。完整系统、文件、提权、默认 Agent 与发行包证据以 [Linux 总验证记录](global-os-linux-2026-10-17.zh.md)为准。这里将源码、协议、隔离原生窗口与用户实际桌面分别记录；WSLg 不等于完整 X11 或 Wayland 登录桌面。

## 现役实现

Linux desktop 已进入 Wave2 Computer 平台元数据与本机 resource resolver。仍由原 `ComputerRoleHost` 的 exact binding、原生目标锁、观察 generation 与 Effect 结算授权，每次输入继续消费已有观察；没有第二套 Agent 或授权账本。

- `nomi-computer/src/linux_x11.rs` 连接当前用户的真实 X server，查询 XWAYLAND/XTEST 与 EWMH 支持；环境变量本身不证明 backend 就绪。XWAYLAND 不充当全局 Wayland 桌面。
- X11 继续使用 xcap 截图/窗口元数据与 Enigo 输入。focus 改为向窗口管理器发 `_NET_ACTIVE_WINDOW`，等待并核对精确 active window；没有内容点击，也不要求与 focus 无关的 XTEST。窗口管理器不支持或未接受请求分别返回实际错误。
- `linux_portal.rs` 通过真实 D-Bus Introspect、版本及设备掩码探测 RemoteDesktop/ScreenCast/Clipboard；AT-SPI 另取 bus 地址并连接 Registry Ping。只读状态不创建 sharing session，不把接口存在当权限已授予。
- `linux_wayland.rs` 使用原生合并 RemoteDesktop/ScreenCast 会话：CreateSession → SelectDevices → SelectSources → Start → OpenPipeWireRemote。授权 UI 由系统提供；输入使用 RemoteDesktop D-Bus Notify 方法。没有 XWayland、compositor 私有接口、root、剪贴板粘贴或 `/dev/uinput` 替代路径。
- RemoteDesktop v2 restore token 采用 App lifetime 的 `persist_mode=1`，仅在现役 owner 内存中单次消耗和更新。App 重启重新请求原生许可；没有新的持久凭据或授权数据库。
- `linux_pipewire.rs` 仅消费获授 remote fd 与 portal stream。ScreenCast v6 优先 `pipewire-serial`/`target-object`；插件不支持时失败，不降级为可能已复用的 node id。旧 portal 使用该受限 remote 的 node id。
- 管线为 `pipewiresrc → videoconvert → videoflip(auto) → appsink(RGBA)`，单帧与 deadline 有界，按实际 VideoMeta offset/stride、可见 crop 与方向处理 pixels，结束/失败均置 NULL。逻辑尺寸和原点来自 portal；缺少原始几何不伪造 pointer 映射。后继 frame 格式改变会使原逻辑几何与 session 失效。

输入只取得现有 active session，不在输入时弹窗、恢复授权或创建新 session。成功截图记录 generation、stream 和逻辑几何，键盘也要求当前观察。session Closed、portal owner 变化、失效/撤回或输入结果未知后，旧观察不能继续输入。

caller future drop 与输入 deadline 都先设置该次 cancellation flag，再 join 精确 blocking worker 的清理。排队 worker 在获锁后、每次 native send 前检查取消；已登记的按键/按钮精确逆序 release，release 与 native Close 不被取消截断。待授权截图中断绑定 exact creator id，等待该次 Request/Session Close 回执；旧取消不能关闭新会话。未取得回执保留 cleanup/result unknown，不重放输入。

## 本机设施

实际 WSLg 环境有 `DISPLAY=:0`、Wayland socket、用户 D-Bus 与 systemd。原生 X extension 查询暴露 `XWAYLAND`，所以产品不把其 root pixels 或 X11 input 当完整 Windows/Wayland 桌面。

2026-10-17 原生探测结果：

| 设施 | 真实结果 | 含义 |
| --- | --- | --- |
| RemoteDesktop portal | missing，Introspect 未导出接口 | 不能开始真实全局输入 grant |
| ScreenCast portal | missing，Introspect 未导出接口 | 不能获取真实获授 PipeWire 桌面 stream |
| Clipboard portal | missing | 没有宣称跨应用剪贴板接口已完成 |
| AT-SPI bus/Registry | available，独立 bus 连接与 Ping 成功 | 证明设施可达，不证明任意应用都导出树 |
| GStreamer factories | pipewiresrc/videoconvert/videoflip/appsink 有系统安装依赖 | 不等于已取得桌面 sharing grant |
| 摄像头 | 无 `/dev/video*` | 未提供可验收的 Linux camera 设备 |
| 麦克风 | WSLg Pulse socket 存在，`/dev/snd` 只有 timer | 不据此断言物理麦克风可用或已许可 |

媒体 caller 已核查：语音输入沿 renderer `navigator.mediaDevices.getUserMedia({audio:true})` 与 MediaRecorder，设置中的麦克风测试取得 tracks 后立即 stop。camera/microphone 系统库存仍为 unknown，实际 WebKit/renderer/设备操作承担结果。Linux 第一方 Browser 宿主与 attached Chrome 仍是缺口，未借本轮桌面实现重写既定 Browser 引擎，也没有把它归为权限拒绝。

## 已运行验证

证据目录为 `/home/muri/nomifun-linux-evidence-20261017`，不进入仓库，不保存私有桌面截图、用户录音或摄像内容。

| 检查 | 结果 | 证据层级 |
| --- | --- | --- |
| `cargo test --locked -p nomi-computer -p nomi-a11y -p nomifun-agent-domain-wave2` | Computer 147 pass、9 native/平台用例 ignore；AT-SPI 14 pass；Wave2 29 pass | Linux 编译、确定性与 native protocol/buffer 回归 |
| Wayland private D-Bus tests | 9 项真实隔离 bus 协议测试通过（包含在 Computer 全套） | 实际 zbus wire/生命周期；不是 compositor UI |
| GStreamer tests | 7 项通过（包含在 Computer 全套） | 真实 buffer/管线，包括旋转、crop、stride/offset、timeout 后 NULL |
| native desktop facilities probe | 2 pass，输出上表实际设施 | 当前 WSLg 只读原生设施 |
| `bash scripts/validation/linux-computer-native.sh` | 1 pass | 私有 Xvfb/Openbox 原生 ComputerTool |
| GTK acceptance oracle | 启动、PID/XID、widget 全局 bounds 与初始 state 可读并回收 | fixture 自检，不是默认 Agent 成功证据 |
| 默认 General 完整产品入口 | 34 个检查、18 个正式步骤通过，canonical Turn completed，pending/unknown=0、cleanup_proven=true | 真实普通用户 Tauri App、原默认 Agent 与独立私有 X11/DBus/AT-SPI；下节给出范围与回收证据 |
| shell/Python syntax、`git diff --check` | 通过 | 源码/验证入口 |

全套日志为 `desktop-final-tests.log`，设施日志为 `desktop-facilities-native.log`，隔离 X11 日志为 `desktop-x11-script-native.log`。私有 D-Bus 最新定向日志为 `desktop-portal-cancel-final.log`。

隔离 X11 用例通过产品 `ComputerTool::execute_authorized` 创建并操作其自身窗口：窗口列表/精确 EWMH focus、focus 前后 pointer 不变且没有 ButtonPress/Release、真实 PNG screenshot、目标窗口收到 ctrl/shift/t press 且每个 key 均 release。脚本使用未占高位 display、private Xauthority 与 cookie、禁用 TCP、记录并回收自己的 Xvfb/Openbox PID；不使用 `-ac`，不修改 WSLg 的 `/tmp/.X11-unix`。可给脚本传 executable + literal argv，使正式 App 与验收 driver 在同一私有显示全生命周期运行。

首次失败原样保留：

- `desktop-tests-first-protocol-failure.log`：私有 portal fixture 的 `version` 属性被 zbus 默认导出成 `Version`，修正正式 wire 名后协议测试通过。
- `desktop-wave2-first-contract-failure.log`：旧测试把合法空白文件名当无效路径；改用 NUL 拒绝例并保留空白路径正例后 29 项通过。
- `desktop-x11-first-facility-failure.log`：初始 user-local Xvfb 缺硬编码 `/usr/bin/xkbcomp`；安装标准测试设施后通过。
- `desktop-x11-script-first-displayfd-failure.log`：WSLg 占有 filesystem Unix socket 目录，`-displayfd` 不接受该 listener 状态；改用独立未占高位 display 的 abstract Unix transport 后通过，未更改宿主目录。

## 默认 General 的隔离 X11 完整产品入口

本次使用冻结 Host `/home/muri/nomifun-linux-evidence-20261017/native-host/final-native-auth/nomifun-desktop`，实际 App PID `169013`、UID `1000`。`global-os-linux-x11-session.py` 启动本次专用 Xvfb/Openbox、private Xauthority、private session D-Bus 与由其激活的 AT-SPI bus/Registry；真实 X server 不含 XWAYLAND 扩展并关闭 TCP，driver 另核对精确 abstract Unix socket 属于该 Xvfb PID。保持真实 `/run/user/1000` 和登录 HOME，仅把本次 App 的 XDG data/config/cache 放入私有 session 目录。没有修改 WSLg socket 目录、读取真实用户 A11Y 树或用环境变量伪装 Wayland grant。

App 数据由原 dataset coordinator 在全新 marked `native-data` 初始化，随后通过真实 owner setup/login 取得隔离 owner JWT。与主验收 App 不共享数据库、owner 或 Session。`global-os-linux-native.mjs --scenario desktop` 只从正式 `assistant.general` 模板创建 Agent 和 Session，没有追加 Action、资源或权限。driver 核对 App 的 executable、普通用户 UID、精确数据库 fd/loopback listener，以及实际 App 的 DISPLAY、Xauthority、private session/AT-SPI 地址；AT-SPI 地址另由该 private bus 的正式 GetAddress 返回值证明。

Session `01a14810-41d2-7712-8534-a413f5dfd825` 的 34 个检查（含正式工具发现）与 18 个正式步骤全部通过：

- 通过原 `start_process` 启动唯一 GTK fixture，按随机 token、实际 PID、XID、窗口标题和只读 `xprop` 绑定同一对象。
- 原 Computer 工具列窗口、精确聚焦，6 张真实 PNG 经实际 vision-input provider attachment 进入模型请求；核对精确 call ID、PNG signature、编码长度、IHDR 尺寸与摘要，物理显示为 1280×800。报告不保留原始截图或全局窗口/AX 正文。
- 每次后继输入前取得新截图 generation，输入后 generation 退休为 0。原 Computer 完成一次按钮点击、精确随机文本输入、Enter 激活与滚动；fixture 自身 GTK 事件分别证明 click_count=1、文本匹配、enter_count=1、scroll_value>0。坐标来自该受控 fixture 的实际 widget bounds；这是确定性产品路径验收，不是外部模型视觉定位或推理验收。
- 原 Computer `Ctrl+q` 正常关闭本次 GTK；同一受管 process 的正式 poll 返回 exit0、cleanup.reaped=true，宿主只读证明确切 PID 消失，最后由原文件工具删除本次 fixture。canonical Turn completed，pending/unknown 均为 0，cleanup_proven=true，desktop_root=false。

之后仅对已验证 executable/UID/私有数据库 fd 的 App PID `169013` 通过 pidfd 发送 SIGTERM，走现役 App 退出协调。launcher 证明 App、private session bus、Openbox、Xvfb 均 exit0/reaped，9 个本次 private bus/App 服务子进程也全部 reaped；独立只读检查确认合计 13 个 PID 已不存在，并确认主 App PID `168501` 仍存活。

本次证据（全部在仓库外，不含 JWT、密码或 private bus 地址）：

- `/tmp/nomifun-global-os-owner-linux-desktop-fdb11bca-d181-4ba1-94a4-3a1f5a211943/evidence/native-product/desktop-assistant.general-1792210386206-73bdc896-9ba7-4fd8-87bb-da467063b91e.json`
- `/tmp/nomifun-global-os-x11-8731acde-f6fc-4d61-a13a-b61f89cb19f3/cleanup.json`
- `/tmp/nomifun-global-os-owner-linux-desktop-fdb11bca-d181-4ba1-94a4-3a1f5a211943/evidence/desktop-cleanup-independent.json`

此项证明真实 Linux App 的默认 General、正式 canonical ingress、原 Computer/native process/file owners 在隔离原生 X11 虚拟桌面完整连通。它不替代用户实际 WSLg 全局桌面、真实 Wayland sharing grant、实体多屏、管理员身份、外部模型推理或最终用户视觉验收；这些边界继续按总验证记录与下节分别保留。

## 最终 Host 的 AT-SPI 与私有剪贴板补验收

前节 18 步证明截图、窗口与输入连通；当时的 private AT-SPI bus/Registry 仅是设施证据，不能据此宣称默认 Agent 已实际读取 tree 或执行 ref 动作。本节补足这一边界，保留前节及所有首次失败报告。

本次使用最终冻结 Host `/home/muri/nomifun-linux-evidence-20261017/native-host/final-native-owner-thread/nomifun-desktop`，SHA-256 为 `d2cb042593ec33ba4dd0b82e5ec31f026c1d49030f6a94b435220c2a2d793c04`，build identity 为 `cf41ea5eefe8741cbd44a86fbd5175a6d711cb6597332eafd893b4c11b3a8ada`。全新 marked dataset、private X11/session D-Bus/AT-SPI 与 XDG data/config/cache 由 UUID `c109224d-f04d-4f0d-a841-b9204212edf5` 隔离；App PID `196444`、UID `1000`，仍保留真实 HOME 与 `/run/user/1000`。主验收 App 不参与这些窗口操作。

默认 `assistant.general`、原正式 Agent/Session 及其原 Action/resource grants 未增加权限或预算。最终 Session `01a14833-6481-7710-bb91-22b515f23a07` 中两个正常用户 Turn 共完成 **86 个检查、45 个实际工具步骤**：

| 正常用户 Turn | 实际步骤 | 模型步 | canonical 结果 |
| --- | --- | --- | --- |
| `desktop-native` | 23 | 45 | completed，pending/unknown=0，cleanup_proven=true |
| `desktop-clipboard` | 22 | 43 | completed，pending/unknown=0，cleanup_proven=true |

每个 Turn 都从正式 `start_process` 新建并拥有自己的 GTK fixture，最后自行 `Ctrl+q`、正式 poll、精确 PID 消失和文件删除；没有跨 Turn 保留临时受管进程。独立 GTK PID 分别为 `215902`、`216172`，均 exit0/reaped。第二个 Turn 仍使用同一默认 Agent 与 Session。

- `computer_inspect {action: observe}` 真实读取 private AT-SPI tree。每个节点的 accessible name 包含本次随机 UUID，driver 要求唯一 text/button refs，并只读核对真实 X11 active window 仍是同一受管 GTK 的 XID；没有用 CLI 读取 A11Y 结果或注入 GUI 动作。Linux 快照正文没有 `app_name` header，测试不把这一未投影字段当作窗口身份依据。
- `computer_input {action: set_element_value, ref, expected_generation}` 的完整 native EditableText setter 与 `click_element` 的 native Action 返回精确原生语义回执；任何 unavailable、错误、stale 或 pixel fallback 都失败。每次后继动作取新 tree/generation，动作后 generation 退休，fixture 自身事件核对完整文本与第二次按钮点击。
- 第二个 GTK 有独立 source/sink Entry。原 Computer pointer/key 完成 source 聚焦、全选、`Ctrl+C`、sink 聚焦与 `Ctrl+V`；包含中文和 `✓` 的完整随机 marker 同时被 GTK 的真实异步 clipboard oracle 与 sink changed 事件读回。原生 AX button Action 触发 fixture 恢复本 private X11 初始空 clipboard，再由新 tree 和独立 oracle 证明 empty、restore_count=1。第一 Turn 的 clipboard 未改动。

这证明现有 Computer key/ref 能完成隔离原生 X11 中的 GUI 跨控件复制粘贴；**现役 Computer schema 没有独立 clipboard read/write operation**，不能把本结果写成已提供全局剪贴板 API、Wayland Clipboard portal 或跨应用通用接口。测试只触及其专用 display 上的 clipboard，未读取或修改日常 WSLg/Windows 剪贴板。

最终产品报告：

`/tmp/nomifun-global-os-owner-ax-c109224d-f04d-4f0d-a841-b9204212edf5/evidence/native-product/desktop-assistant.general-1792212688669-49ca7f9a-01ba-474a-99f8-412ae66ef2b6.json`

中间原生证据保持原状态：

- `desktop-assistant.general-1792212293409-93d58c1a-5683-412c-82f6-91b50647303f.json`：首次 driver 错误要求 Linux AX header 包含 app_name，真实 tree 已返回 3 个节点。原 Turn 已取消并结算 cleanup_proven=true、pending/unknown=0；随后改为 UUID accessible name 与真实 active XID 绑定，不改生产 backend。
- `desktop-assistant.general-1792212384445-f0884c9f-fb94-44c2-9781-192c961a7234.json`：独立 23 步、44 个检查通过，三次 fresh tree、native setter/Action 与退出回收均通过。
- `desktop-assistant.general-1792212484246-cf7b067f-1847-4bfb-a2ea-e986d24243b4.json`：36 步单 Turn 已实际完成中文复制、粘贴与 clipboard 恢复，但第 65 个模型步触及现役默认预算，最后观察和完整正常退出流程未完成，因此整轮仍是 failed。原 Turn 已取消且 cleanup_proven=true、pending/unknown=0。最终用两个各自完整生命周期的正常用户 Turn 完成验收，未扩大默认 budget，也未重放 unknown 效果。

本次所有产品步骤结算后，对已核对 frozen executable、UID、精确 data/display/Xauthority 的 App PID `196444` 通过 pidfd 发送 SIGTERM，走现役正常退出协调。private launcher 实际 exit0；App 自身 exit0/reaped，session bus、Openbox、Xvfb 和 private bus/App 子服务合计 **11 个 owned PID** 均 reaped，独立只读检查确认全部 PID 不存在。证据为 `/home/muri/nomifun-linux-evidence-20261017/desktop-ax-final-evidence.json` 与 `/tmp/nomifun-global-os-x11-c109224d-f04d-4f0d-a841-b9204212edf5/cleanup.json`。

两个早期失败 Session 随后各有单独正常用户 cleanup Turn，原失败报告 SHA 前后保持不变，仅删除本次残留 fixture，并确认 PID 不存在与 cleanup_proven=true、pending/unknown=0。现役 process reference 严格绑定原 exact Turn；对已结束 Turn 的旧 process_id 发起跨 Turn poll 被真实拒绝为 `PROCESS_REFERENCE_INVALID`、not_executed/control_applied=false。这份失败和取消结算也保留，未将拒绝伪称为一次新的 cancelled/reaped 回执，也未重试旧 process control。最终 delete-only cleanup 依赖原 Turn 的真实取消/回收证明与精确 PID 消失：

- budget failed 后的独立记录：`desktop-assistant.general-1792212484246-cf7b067f-1847-4bfb-a2ea-e986d24243b4.fresh-cleanup-9e470462-07c8-4ed4-9068-d8ba9122c7c8.json`
- header assertion failed 后的独立记录：`desktop-assistant.general-1792212293409-93d58c1a-5683-412c-82f6-91b50647303f.fresh-cleanup-01576e5e-5ef3-4ec6-aa3b-7fc8307381b1.json`

验证入口只改 test driver 与 GTK oracle。`bun test scripts/validation/global-os-native.test.mjs scripts/validation/global-os-desktop-cases.test.mjs` 为 27 pass；Linux 入口的 3 个 Node tests、Node syntax、Python syntax 与 diff whitespace 检查通过。日志为证据目录下 `desktop-ax-driver-tests.log`；一次误用 Node 运行 Bun test 的 runner 错误保存在 `desktop-ax-driver-tests-first-node-bun-mismatch.log`，不计产品失败或通过。

## 尚需对应原生组合验收

当前没有把 GNOME、KDE 或所有 Linux compositor 宣称为通过。至少仍需在明确 compositor/portal 版本、实际登录用户与正式 App 上验证系统共享授权授予/取消/拒绝、真实 PipeWire 桌面帧、pointer/键盘、撤回/断连、monitor/scale 变化和 App 重启。App lifetime token 在重新启动后不恢复；这会重新显示系统许可。

Wayland 标准 RemoteDesktop 不提供可移植窗口列表、任意窗口 focus 或真实 global cursor 查询，这些操作明确失败；不能把最近发出的 pointer target 当观察值。X11 实体多屏/负原点/实际登录桌面、提供设备后的正式 WebKit 媒体调用也尚需独立证据。默认 Agent 与正式包的最终结果以总验证记录为准，隔离原语或脚本供应器不等于模型推理与用户视觉验收。
