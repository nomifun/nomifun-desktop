# Linux 全局操作系统能力验证（2026-10-17）

本页记录 Ubuntu 26.04 / WSL2 x86_64 的当前实现与本机证据。源码存在、单元测试、root 隔离夹具、portal 协议 fixture、独立 X11 虚拟桌面和默认 Agent 原生产品入口分别计证，不互相替代。真实 Wayland 系统许可、未提供硬件和其他发行版不能由本页推断通过。

## Git 与环境

目标 checkout 为 `/home/muri/code/nomifun/nomifun-desktop`。原工作树干净，旧 main 为 `bf2ebad9653fd695cede7cc4de1e605f71b7be68`；与远端因已重写历史分叉，`git pull --ff-only` 明确拒绝。旧分支完整保存为 `codex/pre-linux-global-os-20261017`，再新建跟踪远端的 main 并完成 pull。同步基线为 `d1e968bfe262c39deeddcc460f3df27ab26ff0f7`，本轮未 reset、clean、强推或改写共享历史，未提交或推送本轮源码。

已确认包含完整源码交付 `63b91d5af9851855c4c1c32d349a187a5840d430`，没有将旧 source-review patch 强套到新 main。旧基线 `0e54f6905d907951fd8c956de2a17946889e40d1` 的 tree 仍为历史合同中的 `6c82a96b37a2804888c6099f390226cdf86db84a`。

系统为 Linux 6.18.33.2-microsoft-standard-WSL2，Ubuntu 26.04 LTS，真实用户 muri/UID1000。Rust 1.99.0，已装 target 为 x86_64-unknown-linux-gnu；现有 Bun 为 1.3.1。WSLg 提供 DISPLAY 与 Wayland socket，但其 X server 带 XWAYLAND extension，不是完整全局 X11 desktop。systemd、用户 manager、D-Bus 与 AT-SPI 可用。系统安装了构建/验证所需的发行版 GStreamer 开发库、PipeWire 插件、pkexec、polkit agent 与独立 X11 测试工具；没有关闭 AppArmor/SELinux 或修改 sudoers。

## 已落地的生产路径

- Linux 管理员操作走固定 pkexec + root-owned runner，复用现役 SystemEngineOwner、ProcessSupervisor、typed readonly acquisition 和原 Effect。握手校验真实 UID、调用用户和同源 build identity，保留真实 HOME、literal argv、取消、回执 unknown 与子进程清理。认证尚未成功时不派发操作。GUI 继续使用普通用户。Linux SIGTERM/SIGINT 现在进入原 ExitCoordinator 协调退出，复用现役 Session/Effect/子进程清理，不以信号直接结束 App 假装完成清理。
- deb/rpm 安装固定 runner 与 polkit action；开发/AppImage 携带同版本显式安装资产。系统 action 对各 session 类型使用原生 auth_admin_keep，绑定固定路径与 `--stdio`；临时授权由 polkit 按真实 App subject/action 管理，到期和 subject 消失时撤销。没有自建授权、重试或恢复账本。
- Linux 文件原子替换保留 UID/GID/mode 与内核可观察 ACL/xattrs，发布前失败保持真实错误。DrvFS/v9fs 不支持 RENAME_NOREPLACE 时，新 regular file 使用同目录 linkat 原子 no-replace 发布。通用 rename 不做覆盖、复制后删除或跨卷模拟。
- Computer 正式模块和本机 resource resolver 已接入 Linux。X11 核实真实服务器、窗口管理器与 EWMH focus；Wayland 使用 RemoteDesktop/ScreenCast portal 与受限 PipeWire remote、GStreamer 帧和 portal input，不借 XWayland 假装全局支持。
- Linux 主 App 使用 WebKit 的 UserMedia permission request 与原生 GTK 当前请求确认，导航取消未结算请求，不持久化 grants。预览 iframe 显式禁止 camera/microphone delegation。真实设备/音频服务仍决定最终结果，WebUI 使用客户端浏览器自己的许可。详见[媒体证据](global-os-linux-media-2026-10-17.zh.md)。

## 证据矩阵

证据目录为 `/home/muri/nomifun-linux-evidence-20261017`。原生产品报告分别保留首次失败与新 UUID 的修复后结果；构建、root 协议 fixture 与默认 Agent 结果分别记录。最终认证线程修复的同源 Host/helper 已冻结，并完成真实用户 polkit 授予后的完整默认 General 管理员闭环。

| 路径 | 当前证据与边界 |
| --- | --- |
| 前端/边界 | 前端生产构建、媒体/iframe 定向 UI 26 passed；最终 `bun run check` 完整 exit0（类型、UI/Session/进程及其他边界、i18n/主题/图标/文档），受影响 driver/desktop/Linux packaging 脚本最终49 passed / 0 failed / 395 expectations，diff check通过 |
| 管理员 runner | 原系统执行与 native-files 回归，以及新增认证 EOF/退出码、stderr 有界诊断回归通过；独立 root 协议报告覆盖受保护读写、真实 UID/HOME/cwd、只读源/指令 scope、超时、并发取消、16 MiB 控制输出、EOF 和失败回执清理；不代表 polkit 用户授权。线程创建/批准后 EOF/真实 signal 诊断共 19 项专项测试通过 |
| 普通进程/Terminal | process-runtime 完整 Linux suite 281 passed / 1 ignored（手工压力测试未执行）；首次误挂 macOS cfg 导致 PTY_COLS 编译失败，已只修正测试 cfg 后完整重跑。engine-core 57 passed / 1 ignored，再单独执行真实 Bun PATH 用例通过；外部 symlink/../cwd 的 Pipe/PTY 取消及双 PID 回收通过；Terminal 155 passed / 1 ignored，已删除宿主 cwd 下绝对 cwd 的真实子进程/PTY 回归通过 |
| MCP | 正确现役 `owner::stdio::tests::` 8 项通过，另真实 stdio timeout 后进程组 cleanup 1 项通过；较早错误过滤器命中 0 tests 的日志保留且不计为通过 |
| Node/npm | 70 项 runtime unit 已通过；官方 Node 发行的独立 Linux npm offline lifecycle/literal argv、持久 CLI、损坏缓存恢复和重启回归实际 1 passed。首次隔离 fixture 缺 sh 的失败保留，补齐仅 shell 的隔离 PATH 后通过，未借系统 Node 冒充受管发行 |
| 系统设施安装 | 同源 runner 与 policy 已通过显式安装入口安装；pkaction 校验固定 executable/argv1 与原生认证策略。最终 RT4iTT helper 与 cf41 build identity 的静态 UI Host 已配对安装/启动；真实管理员产品流程通过，GUI 全程 UID1000 |
| 文件/挂载 | file 全套 543 passed / 3 ignored；foreign-owner root 隔离用例单独通过；ext4 五项 metadata/外部路径原生回归通过。v9fs 新文件 linkat 发布已证实；现有文件 replace 在保留 staging fd 时无法完成命名身份校验，返回真实 unknown。关闭句柄后的独立 probe 证实目标存在且新字节正确，不能当作未写入而重放；首次及后续报告保留 |
| X11/Wayland | 详见[桌面验证](global-os-linux-desktop-2026-10-17.zh.md)；独立 Xvfb/Openbox 的真实 ComputerTool 路径和 private D-Bus fixture 分开记录；WSLg 缺 RemoteDesktop/ScreenCast backend，真实 Wayland grant/capture/input 未通过 |
| 媒体 | 正式 Settings 入口的真实 GTK Cancel、同文档 NotAllowedError、reload 后 fresh Allow、1 audio track live→正式 stop→ended 均通过；未启 mock、录制或上传。补充 silent graph/Pulse AUTHORIZING 超时未计通过，实际声音链/物理来源未证明；无 `/dev/video*`。详见独立媒体报告 |
| 默认 General/Minimal / system / desktop | General 与 Minimal 各自完成工作区外文件/进程闭环及 system 四阶段闭环；General 在私有原生 X11 完成后续 86 checks / 45 正式步骤（两正常 Turn，含 AX 与中文复制粘贴）。真实 polkit 管理员 7 步闭环已通过；新的 App-exit 持久服务全流程通过，旧失败报告保留 |
| 完整原生包 | 已生成真实静态 UI Host 的本地 debug deb/rpm 并验证包内 UID/mode/字节，但其早期认证代码已被替代，旧目录明确标记 superseded。最终线程修复后的同源 debug deb 已冻结并通过 control/依赖/16项 payload/字节核对，本机安装、同版本替换、UID1000 私有数据启动、卸载与手动组件恢复均通过；不把结构 fixture 当产品包 |

最终 helper stage 为 `run-RT4iTT`，SHA-256 `a88d95222a69b1f190ddf66bd1f1b3b4c81b300059c95422cfb40dbd4567a865`，build identity `cf41ea5eefe8741cbd44a86fbd5175a6d711cb6597332eafd893b4c11b3a8ada`。独立 root 协议报告 `runner-native-final-owner-thread-RT4iTT.json` 全部通过；旧 App 168501 正常退出 0 后才显式安装配套 helper/policy，安装摘要/owner/mode 与 stage 精确一致见 `system-owner-thread-install-verification.json`。该维护安装不作为用户 polkit 授权证明。

早期 runner 报告 `runner-native-final-lock-YoKTq1.json`、认证诊断版本 `runner-native-final-auth-odsND2.json` 均保留各自 stage/hash/build identity；它们显式注明 polkit authentication 未测，不作为最终线程修复版本的二进制身份。

## 删除默认目录后的会话准入

原生旧失败 Session 在默认目录已被验收 driver 删除后，新的 cleanup Turn 实际收到 HTTP400 `WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE`，尚未接受 Turn/Effect。此证据并非文件工具缺陷：调用链在 App project directory fence、Session frozen workspace materialization 两处重复要求目录存在，随后 unbound 知识租约也严格 canonicalize，阻止了本应独立的绝对文件目标与显式 cwd。

已在两处 App **已冻结 binding** 的运行入口复用现役 `canonical_workspace_root`，保持原 owner、project key、目录读写 fence 和 Session 身份；选择新 workspace 的 Create 校验不变。无 active KnowledgeService plan 的 unbound 租约允许绝对、干净的缺失冻结锚点，仍使用同一个签名 registry 与用户级 OS file lock，验证最近存在祖先身份，不创建默认目录或 `.nomi/knowledge`。目录重建的不同知识绑定与活动 unbound 冲突，替换为其他目标的 symlink 不能沿用原租约；实际知识挂载的 bound acquisition 仍要求存在的真实目录。没有使用 None lease、跳过 runtime fence 或新建恢复账本。

租约修复有旧实现真实 NotFound red 与修后完整 16/16 组记录；App 还在 Effect resource key 中修正同一缺口：仅 files/process 的 NotFound 分支复用租约的纯身份 resolver，成功分支与原 JSON keyshape 不变；native canonical ancestor+suffix 保持 Windows 原有 namespace，避免目录消失后分裂 unknown 围栏。Windows 专项用例已加，本轮不声称 Windows 原生执行。早期 `final-native-auth` 和中间 `final-native-complete` Host 保留明确 superseded/pre-process-effect 标记。

最终 App 定向汇总 `app-admission-projection-reset-evolution-verified.json`（SHA-256 `c3268da53d61e6b823bfe22a36c01bd98705b9a5a7a8c8583c5398c5395a0b48`）证明 14 项 App 测试及另 1 项 control-plane 演进测试全部通过：管理员读取投影3、canonical reset3、旧 Session 演进4、真实 router/runtime/owner 缺目录集成1、files/process 同 Effect key且 VCS strict1、owner physical fence1、pending/unknown host-restart fence1、Minimal旧空能力不可变演进1。reset 的一份过时测试夹具只写旧 active pointer，已改为现役 companion_threads 注册接口；原失败保留，生产归档/超时未改。缺目录集成先正常首 Turn，再删除已选目录，验证新 Create 仍拒绝该缺失路径；原 Session 新 Turn 完成绝对文件操作和真实 `/bin/pwd` 显式 cwd，再按正式 completion 合同结算，不重建目录或回填 binding/history。

最终 frozen Host 为 `/home/muri/nomifun-linux-evidence-20261017/native-host/accepted-linux-20261017/nomifun-desktop`，SHA-256 `c69467c4090728494cfd9f7f2f0e19bb3b0a714fb7e957c646a58a67cb5e751d`。`desktop-accepted-linux-build.log` 成功，`accepted-linux-build-inputs.sha256` 在构建前后复核不变，helper build identity 仍是 cf41。该 Host 原生 `missing-default-chat.minimal-1792213619535-229fe846-92c7-4a6b-a01e-a8534409d6bf.json`（SHA-256 `e73610e21b908093c278ec0b327a530cc4302b8bdd4426c602fb25612d0a90bf`，Session `01a14841-9963-7d21-a40e-a983e5070640`）16 checks / 14 正式步骤通过。两真 Turn 的整份 frozen binding SHA 均为 `bc6bb202eb485f55e6f79bbd2903ffd81d2c55ab24506f7feab809647176c1e2`；默认目录每步均保持缺失，合法绝对 read/write/hash-patch/read 与外部 cwd 的 PID246497 / UID1000 运行、cancel/reaped170ms/terminal/PIDgone 全通过。Session、owner、project key未换绑，外部内容无伪造 workspace path；两 Turn 均 completed/cleanup proven/pending0/unknown0/completion delivered。

同一 Host 的正常目录 General 报告 `files-process-assistant.general-1792213653216-15fff076-00e2-4eff-b650-d30c9656deab.json`（SHA-256 `fd206b8ac5ee3b9d4941358fd927afe7b17de140a2be8bee099eded25565106c`）21 checks / 17 正式步骤通过，覆盖原普通外部路径与 symlink/cwd/六项文件/取消清理。App245757 最后正常 SIGTERM exit0。此前管理员、system、desktop AX/clipboard、媒体证据的精确 Host 仍分别是以下各报告记录的 d2cb 版本；其对应 sys-execution/Computer/media 实现未变。本轮没有把较早整机报告改写为在 c694 Host 上重新运行。

## 默认 Agent 原生产品证据

验收由本地 scripted provider 驱动正式默认 Agent、ToolPlan、canonical Session/Turn/Event/Effect 和实际宿主工具；不声称联网模型推理。主 App 使用普通用户 UID1000、真实 HOME、仓库外私有 XDG/dataset，凭据与 owner JWT 仅保存在 0600 仓库外文件。没有读取日常聊天或通过数据库补造事实。

- Minimal 文件/进程报告 `files-process-chat.minimal-1792208636495-36507a95-947d-4207-b1e5-8929ee63eb04.json`（Session `01a147f5-8f25-7a92-a5ff-c1df7223f60c`）与 General 报告 `files-process-assistant.general-1792208710409-2df60f68-804a-4b48-8fe7-011ac049e819.json`（Session `01a147f6-b075-70a0-a883-39ef92a41f32`）通过：六项文件操作、绝对路径/合法 `..`/symlink、外部链接 cwd 中真实 running 进程、取消后 terminal/reaped/确切 PID 消失，以及本次 fixture 删除。外部产物使用 native path，没有伪造 workspace path。
- Minimal system 报告 `system-chat.minimal-1792208212545-baaf6c39-1943-43f0-99e4-ebe223583871.json`（Session `01a147ef-16dd-71b2-b191-6547d5a8a5fc`）与 General 报告 `system-assistant.general-1792208311440-6d7b969b-a98f-4e1a-bee9-d9c69a74a5ff.json`（Session `01a147f0-99df-7bc3-b849-1694fbc929bb`）各完成 29 checks / 4 canonical Turns：隔离用户安装/卸载、literal argv、配置修改恢复、唯一 systemd user manager 配置键写读与清除、transient service 的真实 PID/cgroup/UID/cwd、loopback HTTP 和取消清理。首次 systemd fixture 对 shell `$$` 的错误由系统 `--expand-environment=no` 正确表达 literal argv 修复，失败报告未被覆盖。
- General 桌面报告 `desktop-assistant.general-1792210386206-73bdc896-9ba7-4fd8-87bb-da467063b91e.json`（Session `01a14810-41d2-7712-8534-a413f5dfd825`）在独立真实 Xvfb/Openbox/Xauthority、私有 D-Bus/AT-SPI 环境完成 34 checks / 18 正式步骤，包括截图 vision attachment、窗口枚举、精确 focus、click/type/Enter/scroll、Ctrl+q 退出同一 GTK fixture PID、reaped/消失与 fixture 删除。它不代替 WSLg Wayland 系统许可或实体桌面验收。

后续隔离 X11 补验 `desktop-assistant.general-1792212688669-49ca7f9a-01ba-474a-99f8-412ae66ef2b6.json`（Session `01a14833-6481-7710-bb91-22b515f23a07`）完成 86 checks / 45 实际步骤，分两个正常用户 Turn（23/22 步）并各自完成 GTK 生命周期，未调高默认预算。真实 AT-SPI tree 的 UUID accessible name、active XID 与受管 PID 精确绑定；正式 ref 动作执行 native EditableText whole-value setter 和 Action click，拒绝 pixel fallback。正式 Ctrl+C/V 将包含中文和 ✓ 的随机值从 source Entry 复制到 sink Entry，独立 GTK oracle 验证并恢复最初空的私有 clipboard。现役 Computer 没有独立 clipboard read/write operation，因此该项只证明 GUI keyboard copy/paste。两 GTK 均 Ctrl+q/exit0/reaped/PIDgone/delete；App196444 正常 exit0，11 个 owned PID 全部回收。来源报告与 cleanup 汇总见 `desktop-ax-final-evidence.json`。首个 header 假设错误和单 Turn 超出默认模型预算的失败分别保留，均在原 Session 新 cleanup Turn 删除确切残留目录；跨 Turn 旧 process ref 真实拒绝，没有放宽身份合同。

新增 opt-in Node 生命周期组合的最终标准 Cargo 报告 `node-lifecycle-cargo-final.json`（SHA-256 `e01269a20d2a6d00a2a1314192d99a65f1eabc6ea28bf41a8304f408c02cfdbb`，1 passed / 13.32 s / exit0；较早直接组件重链报告保留独立身份）通过公开 runtime、McpOwner、canonical Store 与 Agent-only reset API，在真实 UID1000 / HOME `/home/muri`、私有 XDG 下验证离线官方 Node。真实 MCP tools/call、literal argv 与确切进程回收通过；Store 两个已接受/取消 Turn 的 1 Session / 2 Turns / 9 Events 经 reset 清零，用户与已安装 CLI 保留，同一 CLI dev/inode/SHA 在 reset、runtime cache replacement 与独立进程重启后仍可执行，再正式 npm 卸载并删除 fixture。原 Node 单独用例的 HOME 为 fixture；此组合用例另证真实 HOME。Store Turns 不作为直接 CLI/MCP 的 Agent Effect 归因，也不声称已覆盖真实 Desktop App clean cut、退役 schema 启动 clean cut或跨版本升级。详见[Node 验证](global-os-linux-node-2026-10-17.zh.md)。

真实 release App 的 Node 后继 Turn 与重启补验已单独通过。正式 installed Host `b4a9306d…`、UID1000/真实 HOME、私有 XDG 和显式请求 PATH 下，默认 Minimal Session `01a14881-747e-71c3-8c68-774082e6dafa` 完成 3 个真实 Turn / 9 项正式步骤；离线 npm install/postinstall、literal argv CLI、新 Turn 不重装调用、App277137 正常退出与 App278572 同数据重启、原 JWT/owner/整份 frozen binding 保持、同一 CLI dev/inode/字节/mode0700，以及最终 npm uninstall/fixture 删除均通过。主体报告 `node-lifecycle-chat.minimal-1792217805802-5a13dd49-cda0-47ce-a6c8-245bd5b9978a.json` 的 SHA-256 为 `02170f4281104130d6bb0608f7cd72f157db2fa20f791d497ac80672a0647e16`。此 passed 主体复用已验证 cache；自动准备证明来自保留为 failed 的独立第三轮真实 bare-node 工具结果与 `third-auto-canonical-effects.json` 的只读 canonical 查证：Node prepare 与 npm install 两条 Effect 实际为 returned，不能用 runtime/progress 单独替代。早期四轮夹具失败与各自 cancel/cleanup 零 unknown 均保留，没有修改生产源码、默认权限或预算。MCP/reset 仍按前述组件层级计证；本补验不扩称真实 Desktop clean cut、跨版本升级或公网模型推理。完整环境、原件和结算见[Node 验证](global-os-linux-node-2026-10-17.zh.md)。

正式媒体报告 `linux-media-production-report.json` 记录普通 UID 的同一 cf41 Host、独立 X11/私有 XDG 的 Settings 麦克风请求：GTK Cancel→同文档真实 NotAllowedError→正常 reload→fresh GTK Allow→真实 1 条 audio track live→正式回调 stop→ended。只读取轨道/许可元数据，未读 PCM/频谱、MediaRecorder/ASR、录制、保存或上传内容。Pulse 新 context 在 AUTHORIZING 超时，补充 silent graph 的观察回包也未被 collector 完整保留，graph state/clock/APIclose 不计通过；不能从已有 live track 推断实际声音来源或音频图成功。App203704 最后 SIGTERM 正常 exit0，renderer、Xvfb/WM/private bus/8 services/observer 均回收，残留进程0、inspector endpoint关闭。物理麦克风与摄像头仍按独立记录的边界判断。

最终 owner-thread App（SHA-256 `d2cb042593ec33ba4dd0b82e5ec31f026c1d49030f6a94b435220c2a2d793c04`）PID191280 / UID1000，通过真实 TTY 的进程专用 agent PID191836（无 fallback）完成首次原生密码认证。General 管理员报告 `administrator-assistant.general-1792212035337-3e62f0af-c1ae-499b-8c62-2911655cd6bc.json`（SHA-256 `df0c25b4398bb651f20817ae63492f4903b6c0c2b3e1ee1b0a0bf55c7e509f24`，Session `01a14829-6d74-7a02-8043-f5719f279117`）在 04:40:35.332Z–04:41:15.056Z 完整通过。7 个正式步骤均通过；10 checks 中 9 passed、1 为真实新 AGENTS 上下文的 defer，随后 fresh call 完成。普通 UID1000 对 root0700 写入真实拒绝，管理员 root0600 写读、root cwd 下 AGENTS 只读预检与规则延期、UID0/cwd/登录 HOME `/home/muri` 全核实。真实运行 PID192928 经独立 poll、cancel、terminal receipt（reaped=true，155 ms）及系统只读 PID absence 校验后清理 root fixture。canonical outcome answered、completion delivered、cleanup proven，pending/unknown 均为 0，无 retained checkpoint。原生 polkit 对此 App subject 的保留上下文供同轮多次 acquire/process 使用，没有重复保存或传递密码。

同一最终 Host 重启后的管理员补验另分新 Session，均由真实 native polkit 保留上下文完成，未保存密码或借维护 root 执行产品操作：

- `administrator-read-matrix-assistant.general-1792212497383-7d5bc111-792f-42ed-bb50-89154921abd5.json`（SHA-256 `71319b6d22f3b3027eac4f5cf8a413eff8c4e6dc76cfb5fe42b2aef7eba19c69`，Session `01a14830-8124-7262-874c-dbd1f2c21248`）14/14 正式步骤通过。root0700 下 root0600 文本按 UTF-8 页读取，正确 next_offset/完整源摘要和行选择；省略/default 与显式 current_user 的 missing_ok 均真实拒绝；管理员真实缺失单列。错误 hash 与真实原地变更后的旧 hash 都为 FILE_CONTENT_CHANGED，新读得到真实新内容/新摘要。管理员 PNG 的下一次实际 provider 请求中核对 CRC、8×6 全 48 像素 RGB，而非把 base64 文本当图片。此为真实默认 Agent 产品链，不声称联网模型推理或同 batch 调度矩阵。最终 root fixture 删除，completed/cleanup proven/pending0/unknown0。
- `administrator-system-service-assistant.general-1792212579515-cfb6ab6f-b1ab-45d2-a806-a9ae6fdc4858.json`（SHA-256 `01947940ddf991e6681d05d79b8d90bc100153904b32e74d79a1866954555979`，Session `01a14831-bb99-73a1-aec8-bde1bf385090`）7/7 正式步骤通过。唯一 transient **system** unit 为 `nomifun-global-os-system-4e27bf621b31442bb4ca83617f4a02f5.service`，MainPID214111 / UID0 / system.slice cgroup / root0700 cwd 均由系统真实状态核实；root 标记由管理员 read_file 读回。系统 stop/collect 后 unit not-found、MainPID0、PID 消失，再删 fixture。它与普通身份的 systemd **user** 服务报告分别计证，未触现有日常 unit。

上述主 App195460 最后经 SIGTERM 正常 exit0；其测试专用 TTY agent 已结束。保留被冻结的 App/helper 及原报告，没有运行中的管理员测试进程。

Minimal 的首次持久服务 App-exit 验收在等待管理员交互时超过原十分钟 deadline；失败报告与 awaiting_app_exit checkpoint 保留。随后在原 Session 通过新的 canonical cleanup Turn 回收了唯一服务/PID，pending/unknown 均为 0，不能把这一清理报告当作 App 退出后存续通过。最终配套 Host 的新 UUID 回归 `system-chat.minimal-1792212104775-061cab81-b88c-48e6-99aa-21ef4f39c90c.json`（SHA-256 `076590d7d97b29ab982b28ec688f9325da253343d5d04c9a16ab8af1b84615c6`，Session `01a1482a-7c27-7b50-823b-f6ff338ba878`）完整通过：30 checks / 27 unique 步骤。App191280 经 SIGTERM 正常 exit0；原 systemd service PID195051/cgroup/marker 在其退出后保持存活。重启同一 frozen Host、私有 XDG 与 dataset 得到 App195460，driver 核对新公告、同数据库、原 JWT/原 Session owner，随后新 cleanup Turn 停止/注销唯一服务并删除 fixture。前后 settlement、cleanup 均 completed/cleanup proven、pending/unknown0、completion delivered。最终 unit not-found/inactive/MainPID0，PID195051 已消失。

默认 Minimal 的原生首次验收暴露 workspace_context 对合法外部 alias 的额外 canonical spelling 重定向。已在现役 owner 中取消这一次拼写 deferral，保持原调用参数、规范化指令身份缓存与真实 AGENTS 变更的新规则延期。对应 workspace_context 8 项与 AGENTS/readonly/serial executor/history/unknown-resume/scope-policy/workflow 消费者通过；修改没有恢复 projection context、双账本或放松未知效果围栏。

## 真实平台限制与未覆盖

WSL 工具启动的进程位于 init.scope，不属于图形 logind session；GNOME polkit agent 实际返回 No session for pid。系统管理员产品授权需要适用于实际调用进程的原生 agent。此环境使用真实 TTY 上的 `pkttyagent --process PID,STARTTIME`；PID 必须为启动 pkexec 的实际 Linux App，STARTTIME 必须来自其 `/proc/PID/stat` 第 22 字段，不能使用 Windows launcher 或验收 driver 的身份。口令仅在原生 agent TTY 输入，GUI 保持普通用户，授权规则保持不变。

首次 General 管理员产品验收使用进程专用 `--fallback` agent，但 App PID 156099/start 274088 与 agent PID 160070 的 login1 `GetSessionByPID` 均实际返回不属于任何已知 session。原生 polkit 127 的[agent 选择源码](https://github.com/polkit-org/polkit/blob/127/src/polkitbackend/polkitbackendinteractiveauthority.c#L1678-L1739)在保存 process fallback 后查找 session，查找失败便提前返回，尚未恢复 fallback。因此存活的进程 agent 不代表该无 session 调用者能够使用它；`--fallback` 只应在核实版本及真实登录 session 后选择，不能作为 headless/WSL 通用参数。

首次失败报告 `administrator-assistant.general-1792209365292-d9f3c048-1936-4264-affc-db41717ea68d.json` 保留管理员指令预检的 `SYSTEM_EXECUTION_AUTHENTICATION_UNAVAILABLE`，没有 root 操作或 Effect，精确取消后 cleanup proven、pending/unknown 均为 0。只读诊断 `polkit-fallback-no-session-diagnosis.json` 保存实际 PID/start time、cgroup、login1 结果与源码依据；原始 pkexec stderr/exit 的 details 未进入该 AppError 报告，不能声称已经恢复它们。已结束原测试专用 agent，使用去掉 `--fallback` 的同 App 进程专用原生 agent 重试，结果另记新 UUID 报告；换 agent 命令本身不证明认证或管理员操作通过。

首次验收的无 session 调用者命中当时系统 action 的 `allow_any=auth_admin`，平台不为下一次 pkexec 保留认证。随后源码中的安装策略统一使用 `auth_admin_keep`；它仍要求首次管理员认证，由原生 polkit 管理同 App subject/action 的短期复用、过期与撤销，不保存密码或无条件允许。实际新策略下的授予与重复调用结果以新的原生报告为准，不能从配置值推断通过。

认证失败诊断已修正：握手 EOF 后先关闭 stdin，给原 pkexec 短暂有界自然退出窗口，保留真实 126/127；仅取消仍存活的 child。对外错误消息包含原生 exit 与最多 2 KiB UTF-8 诊断，后台继续 drain stderr 并最多保留 64 KiB details，不收集原生 agent 的口令流。新增 stdout 先关闭再退出 126/127、超出保留上限仍持续 drain、公开消息 UTF-8 上限测试，与原取消/timeout 测试共 15 项通过；对应重建 Host/helper 的产品认证结果见本页已通过的新 UUID 报告，不能把这些确定性 child fixture 当作用户原始认证失败的 stderr。

本机还复现了认证创建线程的生命周期缺口。现役调用在 `spawn_blocking(Self::connect)` 的临时 Tokio blocking thread 中创建 pkexec；pkexec 127 本身设置 `PR_SET_PDEATHSIG(SIGTERM)`，Linux 将父身份绑定到创建子进程的线程。该线程空闲回收会在 App 与 TTY agent 仍存活时终止原生认证。两个仓库外安全夹具分别直接和通过同版本 Tokio 证明：父进程仍活、创建线程退出后 child 真实 signal 15。报告为 `pdeathsig-thread-lifecycle-probe.json` 与 `pdeathsig-tokio-blocking-probe.json`，不涉及 PAM、密码或特权操作。源码已修复为拥有 child 整个寿命的创建线程，保留系统保护与真实 cleanup；修前正式 LinuxClient 回归真实失败，修后 19 项通过，其中批准后 client drop 的 child 仍能在 EOF 后延迟 100 ms 完成 cleanup marker 并退出回收。真实 self-SIGTERM fixture 记录 signal 15，stderr 遇 EINTR 继续 drain，其他读取错误进入同一有界诊断。独立复查通过；新的用户原生认证与完整管理员产品已通过；旧失败不作为密码错误证明。依据：[Linux PR_SET_PDEATHSIG 线程语义](https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html)、[pkexec 127 实现](https://github.com/polkit-org/polkit/blob/127/src/programs/pkexec.c#L739-L744)。

WSLg 的 portal 缺 RemoteDesktop/ScreenCast 后端；安装 PipeWire 插件不产生 compositor 授权。Wayland 全局窗口枚举/聚焦与 cursor query 没有可移植的正式接口，现役工具保留明确不支持，不通过点击窗口中心或 XWayland 绕过。Linux 原生 Browser host 仍是此前已知未接入的独立工程边界，没有借本轮重写 Browser 引擎。

未提供外接物理卷、网络挂载、其他 LAN 设备或摄像头，不声称其通过。活动文件修改仍走原宿主锁；不同项目指向同一外部 native 目标的 unknown Effect 尚无跨项目持久目标围栏，原任务禁止重放仍有效，没有增加第二本账。

## 本地调试包与正式构建

最终静态 UI Host 构建日志为 `desktop-accepted-linux-build.log`（成功）；Host SHA-256 为 `c69467c4090728494cfd9f7f2f0e19bb3b0a714fb7e957c646a58a67cb5e751d`。最终包为 `/home/muri/nomifun-linux-evidence-20261017/native-packages/final-native-complete/NomiFun-final-debug-NomiFun_0.8.12_amd64.deb`，183672752 bytes，SHA-256 `2a075fa519a2c5fb4dcd79cce27f9885b2102b7e1c735b9a9724f69f189bb267`。包内 Host SHA-256 为 `f5e9f37b915966670149b196b4a9fd2d7d82b4d205149f1050260134655ef7d3`，仅有 Tauri 标准 bundle-type 标记三字节 `UNK`→`DEB`（offset52232818），其他字节与 frozen Host 完全一致；不能直接称两者 SHA 相同。helper/policy 与 RT4iTT 精确一致。

真实包内 16 个 regular payload 均为 root:root，无 setuid/setgid，control 只有 control/md5sums，没有 maintainer scripts 或 conffiles。Ubuntu 的实际 dpkg 依赖解析通过；原生 GTK t64 Provides 满足 libgtk-3-0，不添加假依赖。包内字节/control/依赖报告与分阶段 `package-install-validation.py` 在同目录。本机 `package-install-replace-result.json` 通过原生 dpkg 安装历史调试包、再以最终包完成 0.8.12 over 0.8.12 同版本替换，两次 dpkg --verify 均无差异。`package-installed-host-smoke.json` 证明实际 `/usr/bin/nomifun-desktop` 在 UID1000 / HOME `/home/muri` / 全新私有 XDG+dataset 下启动，拥有确切私有 DB 和公告 PID，正式 `/health` 返回200/status ok/version0.8.12，SIGTERM exit0/PIDgone。首次脚本错用 `/api/health` 的404及正常退出报告独立保留，不算产品失败。随后 `package-remove-restore-result.json` 证明原生卸载、全部16项regular payload消失，再恢复与此前一致的手动 RT4iTT runner/policy（root:root、0755/0644、精确摘要、无dpkg归属）。当前 nomi-fun 包已卸载，没有保留测试用 installed Host。此前 .deb/.rpm 保留 superseded 身份，不作为最终版本。

上述是先完成的 debug 批次。随后已在独立干净源码快照 `298d61cc9ed5691ea9561a5fdd0c4642fc23319c` 实际执行 `bun run build:linux x64 -- --bundles deb`，exit0，完成官方自动 release-lock 与补充实际 runner/policy 的 complete lock。正式包 SHA-256 为 `6b29c8332fb473666cafc35a267826c1abdfab6870dba362c79e133241719900`；实际安装 release Host 的 UID1000 bootstrap/health、Minimal missing-default 与 General files/process 均通过，并已正常退出、卸载、恢复手动组件。原仓库 main/HEAD/index/config未改，构建只在私有快照本地提交，未推送或公开发布。详细二进制身份、源快照、安装/替换/卸载与证据边界见[正式构建记录](global-os-linux-release-2026-10-17.zh.md)。跨版本升级、RPM/AppImage原生安装、其他发行版与CPU仍未覆盖。

## 未覆盖与交付边界

真实 polkit 授予与保留上下文的管理员产品已通过；原生 126/127、signal、timeout/取消/reap 有确定性 native child 回归。无管理员账户、完整交互拒绝/取消、登录用户切换与全部认证设施故障的产品矩阵未全部执行，不能由成功授予推断。Wayland grant/revoke/restore、多屏、实际用户桌面、物理设备声音链、其他发行版/CPU、跨版本升级和公开发行保持未覆盖（正式 release-profile deb/lock/本地安装产品回归已通过），具体缺设施与工程边界已在各节列明。Linux Browser host 为原有独立工程边界，未借本轮重写引擎。

仓库外新的 Linux 源码审查包目标为 `/home/muri/nomifun-linux-evidence-20261017/source-review-linux-final-v3`，使用本次实际 main BASE，包含 tracked patch 与全部必要 untracked regular 源码、逐文件摘要及 bundle 摘要；它不修改原 macOS 历史 BASE，也不包含凭据、环境文件、数据集、聊天、缓存或编译产物。测试报告、失败历史与二进制留在证据目录，未加入 Git。原工作树未提交或推送，旧 main 已保留；正式构建的本地私有快照提交另计。

## 可重复入口

```sh
export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:$HOME/.local/bin:$PATH"
cargo test --locked -p nomi-system-execution -p nomi-native-files
cargo test --locked -p nomifun-runtime --lib node_acceptance::official_node_offline_npm_lifecycle_argv_and_restart -- --ignored --exact --nocapture
cargo test --locked -p nomifun-file
cargo test --locked -p nomifun-engine-core
bun run check:process-runtime-boundary
bun run check:agent-session-boundary
bun run check:desktop-ui-boundary
bun scripts/prepare-linux-system-execution.mjs --profile debug
cargo build --locked -p nomifun-desktop --features tauri/custom-protocol
```

默认 Agent 的 Linux 原生入口为 `scripts/validation/global-os-linux-native.mjs`，私有 fixture 通过真实 App dataset coordinator 引导，再使用 `global_os_fixture_owner` 与 `global-os-linux-fixture-login.mjs` 取得只保存在仓库外 0600 文件中的 fixture credentials/JWT。脚本不打印凭据、不读日常数据、不回填历史；macOS 历史 continuation pin 保持不变。
