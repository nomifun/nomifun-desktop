# Linux / WSL 实施结果与真实 Desktop 验收交接

日期：2026-09-11。性质：**development preflight，不是 Linux Native RC PASS**。

本文第 1–8 节保留首轮实施时的证据快照；该轮随后已提交为 `6ece83336`，合并远端后
以 `9c6800f7f` 推送。用户明确收敛范围后的 **Linux 专项续查见第 9 节**，以该节更新的
验证状态为准。第 5 节的通用业务/测试问题只作历史交接，不再由 Linux lane 调查或修改。

## 1. 结论与权威状态

本轮在 WSL2 的真实 Linux 用户空间中完成了 Linux 编译、运行时/平台定向测试，
通过 WSLg 启动实际 GTK/WebKitGTK Desktop，并在其 WebView 内验证 Agent、Plugin、
MiniApp 页面，以及 UI-only / Service MiniApp 的构建、发布和运行主流程。
这不是用 HTTP health 或模拟 DOM 代替桌面启动。

仍不能据此宣称所有 Linux 发行版、真实 X11/Wayland 桌面、安装升级或三平台 RC 已通过。
Computer 当前选中的 bundled provider 明确不支持 Linux，Browser 实机浏览器控制、
真实模型调用、发行包原生安装等也尚未验收。应用集成测试还暴露了下述共享测试问题。

权威顺序保持不变：

1. 已完整阅读仓库 `AGENTS.md` 和本目录原有全部 25 份规范/证据文件。
2. 分支为 `rf/agent-capability-platform-v2`，实施基线为 `aaaed14c6`；本报告描述该基线
   加本轮未提交工作区改动，不是已冻结 source cohort。
3. `PHASE-N1-M1-CLOSURE-TODO.zh.md` 当前快照/条目表仍为权威：31 项 closed，
   `RC-WIN-01 pending-validation`，`RC-MA-01`、`RC-LD-01 external`，
   `RC-MERGE-01 blocked`。未修改 PHASE、GLOBAL、closure/manifest JSON 或 release record。
4. 当前产品仍为 NomiCoreApplication / Nomi engine；没有把 Fresh-v4、旧 Codex sidecar
   或临时脚本当成产品运行时，也没有弱化 exact revision/digest、CAS 或 Runtime 切换授权。

近期已检查的关键提交：`aaaed14c6`、`aa046760a`、`ee2fdb396`、`b04a85bc6`、
`e38a099a4`、`0d516867a`、`08e66b472`。既有 macOS/Linux managed Node tar.gz 支持和
Linux release-lock 链予以复用，不另起一套实现。

## 2. 环境与隔离

| 项目 | 本轮环境 |
| --- | --- |
| 系统 | Ubuntu 24.04.4，WSL2，Linux x86_64；工作区在 Linux 文件系统，不是 `/mnt/c` |
| 图形 | WSLg；`DISPLAY=:0`，`WAYLAND_DISPLAY=wayland-0` |
| 工具链 | Rust/Cargo 1.97.1，Bun 1.3.14，Node 24.4.1 |
| 桌面依赖 | GTK 3.24.41，WebKitGTK 2.52.3，libsoup 3.4.4，AppIndicator 0.5.90 |
| 独立产品数据 | `/tmp/nomifun-linux-acceptance-20260911`，由 `NOMIFUN_DATA_DIR` 指定 |
| 临时证据 | `/tmp/nomifun-linux-*.log`、`/tmp/nomifun-linux-evidence/` |

未读取个人模型密钥，未执行真实付费模型调用，未安装系统软件，未改变系统默认浏览器、
桌面权限或用户正常产品数据。`sudo -n` 不可用；没有越过密码提示。保留用户原有的
未跟踪文件 `get-docker.sh`。未提交、推送、重写历史或操作远端 macOS 工作区。

注意：启动产品时，既有 managed free-model 后台任务可能执行目录发现/刷新；本轮没有
主动发起模型生成请求。这种后台目录变动也会影响第 5 节提到的默认路由复用测试。

## 3. 本轮修复

### LNX-01：PATH 中 Node 符号链接使 MiniApp Service 不能启动

位置：`nomifun-miniapp-platform/src/service_process.rs` 及对应真实 Node 集成测试。

Node probe 已能验证实际 executable，但首次自动选择保留的 PATH alias 可能仍是 symlink。
MiniApp Service factory 原来直接拒绝该路径，造成 Build 能用、Service Test/启动不能用。
本机原有两个 `service_runtime` 测试即因此失败。

现在先要求绝对路径，再在 factory admission 时 canonicalize 一次并固定物理 executable；
每次启动仍验证 regular file/non-symlink 和被选中的 executable digest。回归测试验证：
合法 symlink 可启动和调用；alias 后续改成 dangling link 不改变已准入 executable；
错误 digest 仍被拒绝。没有放宽 Runtime 身份校验。

### LNX-02：Unix 路径被按 Windows 分隔符/有损 UTF-8 比较

位置：`nomifun-js-runtime/src/service.rs`。

原实现把所有平台的反斜杠替换成 `/`，会把 Linux 中两个不同路径误判为同一 executable。
Linux/Unix 现在使用原始 `OsString` 进行字节精确比较；非 UTF-8 文件名也不会被有损转换
成另一个含替换字符的路径。Windows 原有分隔符和大小写处理保持不变。
回归同时覆盖不同路径拒绝、真实 symlink alias 接受、非 UTF-8 与替换字符不等价。

### LNX-03：Linux 打包可能把旧 package 锁到新 Host

位置：`scripts/desktop-build-linux.sh`、`scripts/desktop-build-linux.test.mjs`。

- 从任意 cwd 调用时先进入仓库根目录。
- 补充 GTK/WebKitGTK preflight 和依赖安装提示。
- 每个 target 构建前只清理该 target 的生成型 Linux package/signature；不删除其他平台、
  其他 target 或已收集的 dist 文件，防止上次 `--bundles rpm` 残留被本次 deb 构建收集。
- 每个 target 单独检查 package 数量，不能用第一个架构的成功掩盖第二个架构没有产物。
- 新增 Linux 隔离 shell fixture 行为测试；非 Linux 跳过这些实际 shell 测试。

### LNX-04：登录 shell 的 stdout 继承可绕过启动超时

位置：新增 `nomifun-runtime/src/shell_env_linux.rs`；`shell_env.rs` 仅加 Linux 分发；
`nomifun-runtime/Cargo.toml` 和 `Cargo.lock` 仅增加该 crate 的 Linux `libc` 依赖边。

旧逻辑在等 shell 后无界 join stdout reader。即使 shell 自身退出，只要后台子进程保留
stdout，Desktop 主线程仍无法继续。回归先用保留 stdout 的 2 秒子进程证实旧实现等待
2.001 秒；这种等待本身不受既有 5 秒 shell timeout 约束。

Linux 新实现使用启动线程内的非阻塞 pipe 读取，5 秒总期限和 1 MiB 输出上限；shell
退出后只排空已到达字节，不等后代关闭 pipe；退出所有路径清理独立探测进程组并回收
直接子进程。不留下 reader thread，保留后续修改进程环境的单线程前提。
这是正常子进程生命周期清理，不是阻止恶意 `setsid` 逃逸的 sandbox。

新增 5 项 Linux 测试：继承 stdout、阻塞/持续噪声超时、进程组清理、输出上限、
128 KiB 合法 PATH。macOS 原函数体不改，以免干扰并行 macOS 实施；同类 macOS 风险
应交给该 lane 自行复现/决定是否采用类似方案。

### 验证工具

新增 `scripts/validation/linux-webkit-smoke.mjs`：使用 WebKitGTK 自带的 inspector
协议，在真正 Desktop WebView 中执行验证。显式要求 Linux、loopback inspector、
隔离 data root 和 output；核对 WebView 后端端口与该 root 的 `port.json` 相同。
所有 API 调用沿用产品初始化脚本的 local-trust admission，不把 token 或 Surface
capability 导出到结果。输出始终标记 `development-preflight`，即使在非 WSL Linux
运行也不会自动升级为 Native RC evidence。

此脚本会创建两个 MiniApp，因此只能对明确用于测试的数据目录运行。界面按钮定位支持
en-US / zh-CN；API 流程和 UI iframe 流程分别检查。`--quit` 请求产品正常退出；实际
进程/端口清理仍应结合日志和 OS 状态核实。

## 4. 已执行验证

以下是实际执行结果，不把未执行项目合并为 PASS。重复运行的测试不重复累计为独立覆盖数。

| 检查 | 结果 | 本机日志/证据 |
| --- | --- | --- |
| js-runtime / js-host / js-authoring / nomi-process-runtime 定向组 | 326 passed，2 ignored；包含真实 Node、Linux process group/parent-death/watchdog | `nomifun-linux-runtime-tests.log` |
| 修改后 js-runtime 复跑 | 27 passed，1 ignored；含新增 Unix identity 回归 | `nomifun-linux-runtime-final.log` |
| 官方 Node LTS 下载 ignored test 单独执行 | 1 passed；真实下载、摘要校验、Linux tar.gz 安装与 probe，约 89 秒 | `nomifun-linux-node-download.log` |
| miniapp-platform / plugin-platform / plugin-service / v4-root | 132 passed；原先两个 symlink 失败已因生产修复消失 | `nomifun-linux-platform-tests.log` |
| Linux 启动 shell 定向组 | 24 passed；新增 Linux 5 项包含在内 | `nomifun-linux-shell-final.log` |
| App lib，启用 browser-use,computer-use | 560 passed，3 ignored | `nomifun-linux-app-tests.log` |
| App route-gap，默认 features | 最终整组 32 passed；复用测试曾间歇失败，见第 5 节 | `nomifun-linux-routes-final.log` |
| startup_smoke | 5 passed | `nomifun-linux-startup-tests.log` |
| 原生 Desktop debug 编译 | 通过，GTK/WebKitGTK 原生链接成功 | `nomifun-linux-desktop-build.log`、`nomifun-linux-desktop-build-final.log` |
| 前端 production build | 通过；有大 chunk 提示，不是编译失败 | `nomifun-linux-ui-build.log` |
| Linux build script | 5 passed；`bash -n` 通过 | `nomifun-linux-build-script-tests.log` |
| WebKit smoke 语法 | `node --check` 通过 | 仓库脚本 |
| 实际 WSLg WebView | 下述两类产品操作及 iframe UI 操作通过 | `nomifun-linux-evidence/linux-webkit-preflight.json` |
| 含嵌入前端的 debug deb | Tauri build/bundle 通过；从 deb 解包后的 executable 启动且不依赖 Vite，整套 WebView smoke 再次通过 | `nomifun-linux-standalone-build.log`、`nomifun-linux-packaged-startup.log`、`nomifun-linux-packaged-evidence/linux-webkit-preflight.json` |

本轮涉及的主要测试命令：

```bash
cargo test --locked -p nomifun-js-runtime -p nomifun-js-host -p nomifun-js-authoring -p nomi-process-runtime -- --test-threads=1
cargo test --locked -p nomifun-miniapp-platform -p nomifun-plugin-platform -p nomifun-plugin-service -p nomifun-v4-root --tests -- --test-threads=1
cargo test --locked -p nomifun-runtime shell_env -- --test-threads=1
cargo test --locked -p nomifun-app --features browser-use,computer-use --lib --test nomi_core_route_gap --test startup_smoke -- --test-threads=1
cargo test --locked -p nomifun-app --test nomi_core_route_gap -- --test-threads=1
cargo test --locked -p nomifun-app --test startup_smoke -- --test-threads=1
bun test scripts/desktop-build-linux.test.mjs
```

带 features 的组合命令在 route-gap 失败后没有继续 startup_smoke；表中 startup_smoke
结果来自单独补跑，不能误记为组合命令全绿。App lib 忽略项为 live Browser、Computer
桌面输入、真实 provider；未执行 npm registry 网络集成的 ignored test。shell 原有 zsh
rc 用例在未安装 `/bin/zsh` 时会自行跳过 body，Rust test 的 `ok` 不代表已验证 zsh。

### 实际桌面观察

- Desktop 进程启动并完成 Nomi-core 路由组装；loopback health 200。
- 实际 WebView 请求 settings/system/agents/cron/companion/browser/terminals/conversations
  得到 200，WebSocket upgrade 101；inspector 读取到实际渲染的工作台正文。
- `/agent`、`/plugins`、`/mini-apps` hash 和内容渲染检查通过。
- UI-only：Create → Build Ready → Publish Active → Enable → Surface open → HTML
  读取 → close → 原 capability 再读 404。
- Service：同上，另执行正式 Ready Test passed、Service start/stop。
- 两类 MiniApp 随后从真实界面打开 Surface：检查 sandbox 为
  `allow-scripts allow-forms`、iframe load 完成且无 surface alert，再点击关闭并确认卸载。
- `--quit` 调用产品正常退出路径；确认 Desktop PID 消失、监听端口关闭，日志中
  channel queue、plugins、terminal cleanup 完成。首次开发启动曾以 SIGTERM 停止以便
  重建；只有后续正常退出才作为清理证据。`port.json` 实际仍保留历史 announcement，
  不代表服务存活：已检查其中的 PID 不存在、其端口不能再连接。现有写入实现未提供
  退出删除语义，因此不能把“删除 port.json”记为通过，也不把它当成进程锁。
- 同一隔离 root 重启成功。尚未把已发布对象的完整跨重启恢复、崩溃恢复、Upgrade /
  Rollback、完整 Plugin installed-app flow 等记录为人工桌面验收 PASS。

### 打包预检边界

已尝试本机 debug bundling，deb control 可读且声明了 GTK/WebKitGTK/AppIndicator
依赖。第一次对运行中的 debug executable bundling 遇到 Linux `ETXTBSY`，正常退出
Desktop 后重试；这属于对运行文件原地 patch 的开发操作约束，不应通过绕开 Host
完整性校验来“修复”。可选 debug RPM 压缩持续约 10 分钟仍未完成，已主动停止该
打包进程，没有记录 RPM 构建成功；release RPM 应在原生 lane 使用发行构建另验。

最后执行了完整 `tauri build --debug --bundles deb`，而不只是对普通 cargo dev binary
执行 bundle。前端重新构建、custom-protocol 嵌入和 deb 打包均成功。随后关闭 Vite，
将 deb 解包至 `/tmp/nomifun-linux-package-extract-20260911`，运行其中的
`usr/bin/nomifun-desktop`，沿用隔离数据目录。

结果为 `webview_origin: tauri://localhost`，`status: preflight-pass`，两类 MiniApp
再次通过 Build/Publish/Enable/Surface HTML 与撤权、真实 iframe 打开/关闭，Service
另通过 Test/start/stop。正常退出后该次 PID 101560 已消失，后端端口 41383 连接失败，
9232 inspector 和 5173 Vite 均无监听；OS 进程表无残留 Node/WebKit/NomiFun 进程。
这是 **WSLg 下解包后运行**，不是 `dpkg -i` 系统安装、desktop launcher 或真实 DE 验收。

本机 debug package：`target/debug/bundle/deb/NomiFun_0.7.6_amd64.deb`。
SHA-256：`c26d8ae43109532509d95c5c6940f53159f541139ac7b4aadd3304c35a4cf18e`。
该 hash 仅定位本轮预检包，不能替代 Host/package/legal release lock。

Debug 包只用于开发预检，没有生成 release PASS record，也不能复制到发行 cohort。
本轮未执行 root 安装、AppImage FUSE 启动、发行签名/升级或原生桌面安装验收。

## 5. 尚存问题与无法宣称通过的项目

### APP-TEST-01：Browser catalog 测试有固定 feature 假设

`nomifun-app/tests/nomi_core_route_gap.rs::nomi_core_catalog_exposes_native_nomi_capabilities`
固定断言 Browser unavailable。Desktop 本身启用 `browser-use`，相同测试在该 feature
下实际为 materialized，因此最初组合命令失败；默认 features 该用例通过。

这是测试与 host build 的假设不一致，不能据此把生产 Browser 一律改成 unavailable，
更不能将 materialized 解读为已经控制了系统 Chrome。该共享测试未在本轮修改，交由
集成 lane 按实际 owner/feature 条件补齐断言并复跑 feature matrix。

### APP-TEST-02：官方 Agent 默认路由复用的间歇性失败

`official_agent_direct_launch_reuses_configuration_and_creates_sessions` 在带 features 和
默认 features 下都曾出现两次 launch 返回不同 preset ID。随后连续复跑多次通过，
再追加复跑仍出现一次失败；最终整组默认测试 32/32 通过，不能消除已观察到的 flake。

临时、仅输出字段名的诊断显示差异在 `chat_route_records`，进一步一次失败显示两个
failover candidate 的 `model` 顺序不同。诊断代码已移除，没有记录凭据。
`AppServices` 启动时立即启动 managed-model catalog refresh；测试却假定两次默认
解析的模型图不变。此处需集成 owner 验证刷新与排序变化的因果关系，并使测试夹具
使用稳定目录/可控刷新；不同 resolved route 本来就不应绕过 exact payload 比较强行
复用。本轮不修改共享控制平面语义，也不以一次重试变绿关闭该问题。

证据：`nomifun-linux-app-tests.log`、`nomifun-linux-official-reuse.log`、
`nomifun-linux-reuse-repeats-2.log`、`nomifun-linux-routes-final.log`。

### WSL / 工具限制

- WSLg 出现 EGL/ZINK/dri2 与 GTK scale-factor 警告，仍成功显示/运行实际 WebView。
  不据此替真实硬件 GPU、DPI、输入法或 Wayland compositor 给出性能结论。
- 本机无系统 Chrome/Chromium，未执行 live Browser 用例；Browser linux64 下载、
  system executable 探测、连接/释放、profile isolation 仍需真实 desktop 验证。
- 无 WebKitWebDriver；软件源下载对应包未成功。已用内置 inspector 做本轮验证，
  不能写成 WebDriver 或完整点击/视觉回归已通过。
- 开发启动自动 updater 的 endpoint 请求报错；尚未验证发行 channel 的真实更新链。
- 没有真实模型凭据/显式 live provider 测试输入；StepFun 全链路待用户在安全环境验证。

### Computer Linux 是产品能力缺口，不是 WSL 测试豁免

`nomifun-agent-domain-wave2` 当前 bundled Computer provider 的平台合同不含 Linux。
底层 `nomi-a11y/src/linux/actor.rs` 虽有 AT-SPI 支持，但 Wayland 输入仍有 Unsupported
分支，窗口 focus 也未完整接通。不能只改 platform support flag 或提供静默 X11 fallback
就宣称支持 Linux Computer。

如 Linux RC 要求 Computer 完整可用，需另行明确并实现 Linux provider：AT-SPI、X11 /
Wayland portal 截屏/输入授权、资源 lease 与 generation、拒绝/撤权、坐标/DPI、清理和
真实桌面证据。当前应保持 typed unavailable；用户仍可使用不依赖 Computer 的工作流。

## 6. 真实 Linux Desktop 验收表

在同一待验 source/input cohort 下执行。优先 required Linux x64，至少分别覆盖真实
X11 与 Wayland；arm64 若进入支持矩阵，必须使用原生 arm64 工具链及系统库验证。
仅 `rustup target add` 不构成 GTK/WebKitGTK 跨架构环境。

| 范围 | 必须验证的点位 | 本轮状态 |
| --- | --- | --- |
| 安装/发行包 | release `.deb`/`.rpm`/`.AppImage` 与各自 Host/package/legal lock；发行版最低 glibc/WebKitGTK，依赖名称，非 root 用户，安装/卸载/升级数据保留 | debug 预检；native 待验 |
| AppImage | 有/无 FUSE2，`APPIMAGE_EXTRACT_AND_RUN=1` 构建路径，桌面集成/图标/资源定位，临时挂载目录，更新后可启动 | 未验 |
| 真实图形栈 | GNOME/KDE 的 X11/Wayland、硬件 GPU/软件回退、100%/150%/200% DPI、多屏/负坐标、IME/组合键/滚动/拖放 | WSLg 基础页面/iframe 通过；其余待验 |
| Shell 集成 | launcher 的最小 PATH，bash/zsh/fish rc，PATH alias 和中文/空格路径，shell 阻塞时仍启动，托盘关闭/恢复/退出、通知、autostart、keep-awake、deep link 单实例转发 | shell 定向测试通过；实际 DE 集成待验 |
| 桌面 IO | file open/save dialog、系统 opener、剪贴板、Secret Service/keyring 若使用、portal 授权/拒绝/撤销；主窗口和 MiniApp surface 权限边界 | 未人工验收 |
| Runtime | auto/manual/managed 精确探测，无 Node/不兼容 Node、推荐 LTS 官方下载、拒绝损坏 archive、手工选择、切换 quiescence/失败 abort、重启恢复，只有一个 committed Node | 定向测试和下载通过；完整 UI 状态矩阵待验 |
| 文件系统 | ext4/btrfs、只读/权限不足/noexec、空格/中文/大小写/反斜杠/非 UTF-8、symlink 替换、路径越界拒绝；如支持 Windows 挂载目录须另验 `/mnt/c` | identity 与平台测试覆盖部分；完整挂载矩阵待验 |
| Plugin N1 | 官方 SDK scaffold、Dependency、Build/Test/Apply、manual/compatible-idle、Mount generation、Config/Credential slots、KV/CAS/dataDir、Share/import、CLI、并发/异常/重启后恢复 | 后端平台测试通过；完整 installed-app candidate 未验 |
| MiniApp M1 | UI-only/Service 全生命周期、Service invoke/cancel、MessageChannel Bridge/KV/Files/private SQLite、Ready/Active/Previous 回滚、Share/Backup/Restore/trash/delete、运行中 Runtime 切换 | WSLg 主流程+iframe、后端平台测试通过；全量 installed-app candidate 未验 |
| Process / shutdown | Node/终端/Browser/Plugin/MiniApp 正常退出、强杀宿主后无存活子树、端口/锁恢复、suspend/resume、systemd logout | process-runtime 定向和正常 Desktop 退出通过；真实 DE 故障矩阵待验 |
| Browser | 系统 Chrome/managed Chromium 的 Linux arch/profile、launch/connect、真实页面交互、隔离与释放；未安装时 typed failure | 未做 live 验证 |
| Computer | 当前 bundled Linux unavailable；若要支持须先补 provider/portal/AT-SPI 实现再测，不能仅勾选桌面权限 | 明确能力缺口 |
| Nomi / Remote / CLI | 安全配置 StepFun、真实流式调用/取消、选模型创建 Session、工作台持久绑定、Remote 鉴权/重放；CLI share/import 与拒绝过期 revision | app 定向测试通过部分；live 和安装版完整闭环待验 |
| 发布提升 | required 三平台 candidate/signed_rc 六格 record、相同 source/input、真实 package locks、同 bytes 原样提升 Stable | 本轮不生成、不修改、不关闭 |

## 7. 可复现的开发预检

以下命令在仓库根目录运行。只对测试实例操作；先关闭自己的已有 NomiFun 实例，避免
single-instance 把启动请求转交给正常使用中的窗口。不要在有凭据的日常实例开放 inspector。

终端 A（使用 Vite 的开发构建）：

```bash
bun run --filter=./ui dev
```

终端 B：

```bash
cargo build --locked -p nomifun-desktop -j 4
export NOMIFUN_DATA_DIR="$(mktemp -d /tmp/nomifun-linux-acceptance.XXXXXX)"
export WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:9232
target/debug/nomifun-desktop
```

终端 C，把 `<同一个隔离目录>` 替换为终端 B 的实际目录：

```bash
node scripts/validation/linux-webkit-smoke.mjs \
  --inspector http://127.0.0.1:9232 \
  --data-root '<同一个隔离目录>' \
  --output /tmp/nomifun-linux-evidence \
  --quit
```

脚本需要支持全局 WebSocket/fetch 的 Node（本轮 Node 24）及英文或简体中文界面。
它不启动 Desktop、不负责安装 Node，也不会规避 Runtime Manager 的确认条件。
没有 selected Node 时，应从产品 Runtime Manager 合法选择/确认再运行。
部分发行版的 WebKitGTK 不提供 HTTP inspector，此时应报告工具不可用并人工验收，
不要把本脚本的连接失败误归因于产品不可启动。

结束后关闭自己的 Vite；确认 Desktop 进程、inspector 及后端监听端口均已退出。
`port.json` 可保留历史值，须检查进程/端口实际存活情况，不能仅看文件是否存在。
不要把 inspector 绑定到 LAN，不要将带 local-trust/credential 的数据库或原始
日志上传。`/tmp` 证据不随 Git 迁移，若需保留先审核脱敏并转存到受控证据目录。

真实 release package 应另用仓库受控构建入口，例如：

```bash
bun run build:linux x64 -- --bundles deb
```

这会执行实际 release 编译/包收集/lock 创建验证；应在冻结源码和合适的原生构建环境运行。
不要将开发 debug bundle、不同 commit 的包或本报告中的预检 JSON 喂给 Native RC 聚合器。

## 8. 与 macOS lane 的合并边界

- Linux 专属文件：`desktop-build-linux.*`、`shell_env_linux.rs`、Linux WebKit runner、本文。
- 少量共享边界：Runtime executable identity（保留 Windows 语义，修正 Unix 比较）、
  MiniApp Service factory（与 Build Host/probe 一致的 canonical admission）、
  shell_env Linux cfg 分发及 Linux-only 依赖边。
- 未改 macOS packaging、Browser release path、Node archive profile、Nomi 控制平面、
  canonical schema、PHASE 状态或三平台 record。macOS lane 可针对 shared factory
  跑同一个 symlink regression；shell 的 macOS 原函数保持原样。
- 集成前先比较远端最新提交与这几个共享文件，再正常 merge/cherry-pick；本轮没有为追赶
  远端而覆盖脏工作区，也没有擅自提交/推送。由集成 owner 统一冻结 source cohort。

## 9. Linux 专项续查（2026-09-11）

基线：`9c6800f7f` 加本次工作区改动。仅处理 Linux 专属执行路径、Linux 上才有意义的
安装器/内核实现差异；不处理 Browser feature catalog、Agent 默认路由复用或其他通用
业务问题。PHASE 与 RC 状态不变。没有操作远端 macOS 工作区、修改正式 latest.json、
签名密钥、release record、发布资产或生产用户数据。

### LNX-05：Linux deb/rpm 安装版会取得 AppImage 更新包

证据：锁定的 `tauri-plugin-updater 2.10.1` 按当前包类型先查
`linux-<arch>-<installer>`，再回退 `linux-<arch>`；deb/rpm 安装器分别验证输入包格式。
原 `make-latest-json.mjs` 只生成通用架构键，并优先选择 AppImage，因此同时打三种包时，
deb/rpm 安装版也会拿到 AppImage，不能由相应安装器安装。这不是 macOS/Windows 问题。

修复只增加 Linux 分支：

| 已安装包 / 更新键 | 生成的包格式 |
| --- | --- |
| `linux-<arch>-deb` | `.deb` |
| `linux-<arch>-rpm` | `.rpm` |
| `linux-<arch>-appimage` | `.AppImage` |
| 兼容键 `linux-<arch>` | 仅 `.AppImage`，不再用 deb/rpm 冒充 |

覆盖 x86_64/aarch64；只构建 deb/rpm 时，不新建不兼容的通用 AppImage 回退键。同版本
追加会保留其他平台和已经生成的 Linux 格式条目。Linux 发版 preflight 现在要求三种
安装器键齐全，并检查 URL 文件扩展名匹配，不能仅凭通用架构键宣布可发布。

测试使用隔离目录内的假包/假签名，证明的是实际生成脚本与 updater 查找顺序匹配，
**不是签名验真、实际系统安装或已发布更新通过**。真实 Linux 桌面仍需验证三种已安装
包的下载、授权、安装、重启；CrabNebula 主端点实际响应也需独立验证，本修复仅证明
GitHub 静态清单的生成结果，没有更改线上服务或 endpoint 顺序。

### LNX-06：Linux release 构建入口接受改变产物目录的参数

`desktop-build-linux.sh` 固定收集 `target/<selected-triple>/release`，却原样透传
`--debug`、`--profile` 和第二个 `--target`。实际构建可以成功，但脚本清理和收集了
错误目录，随后报没有 package；还会在这次 debug 构建前删除原 release 产物。

现在在清理/构建前明确拒绝这些覆盖参数。架构用入口支持的 x64/arm64 选择；debug 或
自定义 profile 直接使用 `bun x tauri build`，不进入 release-lock 入口。回归检查拒绝时
旧 Linux bundle 保留、dist 未创建；原有正常 release、多架构空产物检查仍通过。

### LNX-07：Linux 发版清理/上传选择会碰到其他平台的证据文件

原 `release-linux.sh` 在共享 `dist/desktop` 中按所有 `*.sig` / `*.release-lock.json`
进行清理和收集，会删除 macOS/Windows 签名/lock，或将其误纳入 Linux 上传列表。

现在仅匹配 `.deb`、`.rpm`、`.AppImage` 及其精确后缀 `.sig` / `.release-lock.json`。
Linux fixture 实际执行这两个 artifact 函数，证明 macOS/Windows 的文件被保留且不会
进入 Linux asset 列表。测试不执行登录、构建、tag、commit、push 或 GitHub 上传。

### LNX-08：Linux Browser 无条件关闭 Chromium sandbox

原 `launch.rs` 仅根据 `target_os = "linux"` 就加入 `--no-sandbox`；真实 Linux 桌面、
WSL、普通用户、具有 namespace/seccomp 能力的内核也全被降级，并非检测失败后的回退。
已移除这一 Linux-only 参数，保留 Windows/macOS 原有参数、CDP pipe、profile 隔离、
生命周期 owner、整树清理和 extra-args 安全限制。headless/headful 都默认保留 sandbox。

从官方 CfT Linux x64 URL 下载仓库固定版本 `149.0.7827.155` 到独立 `/tmp` 目录，实际
可执行路径为 `chrome-linux64/chrome`，zip 解出的可执行位有效；没有安装系统 Chrome。
本轮用系统 unzip 创建测试夹具，不将其记为产品 managed installer 完整验收。

新增显式 ignored 的 Linux 实机测试 `tests/linux_sandbox.rs`：通过产品 launcher 启动，
通过真实 CDP pipe 创建 `chrome://sandbox` 页面并读取状态，然后释放连接/进程 guard，
等待精确 runtime marker 清理。本机实际报告：Namespace sandbox、PID namespace、
Network namespace、Seccomp-BPF / TSYNC 启用，`You are adequately sandboxed.`。

**真实桌面待验：** Ubuntu AppArmor、禁用 unprivileged user namespace 的发行版、容器
seccomp/root 运行等可能不满足 Chrome 的 sandbox 前提；本次不会静默重试 `--no-sandbox`，
也没有修改内核参数/AppArmor 或安装 setuid helper。应在真实目标机确认受支持的系统浏览器
或受管理员管理的 sandbox 配置；不能把本机 WSL 的成功当成所有发行版可用。

### 续查验证与边界

所有日志仍位于本机 `/tmp`，不会随 Git 迁移；以下为开发预检，不形成 RC record。

| 定向验证 | 结果 / 证据 |
| --- | --- |
| Linux updater/build 回归，修复前 | 9 fail / 5 pass；`nomifun-linux-focused-before.log` |
| Linux artifact 隔离，修复前 | 2 fail；`nomifun-linux-release-before.log` |
| Linux sandbox 参数回归，修复前 | 1 fail；`nomifun-linux-sandbox-before.log` |
| Linux 构建、更新清单、发布证据隔离 | 23 passed；`nomifun-linux-focused-after.log` |
| Browser launcher 定向单测（含 Linux 参数回归） | 37 passed；`nomifun-linux-browser-launch-tests.log` |
| 真实 Linux Chromium sandbox 页面与 pipe/marker 清理 | 1 passed；`nomifun-linux-browser-sandbox.log` |
| 真实 Linux Chromium Host 正常 shutdown | 1 passed；`nomifun-linux-browser-shutdown.log` |
| 真实 Linux Chromium Host/standalone Drop 与稳定 profile 重启 | 3 passed；`nomifun-linux-browser-drop.log` |
| 真实 Linux Chromium headless 单受控页面 | 1 passed；`nomifun-linux-browser-headless.log` |

合计 66 项定向检查通过，其中 6 项使用真实 Linux Chromium；未把重复运行重复计数。
结束时 OS 进程表无残留 Chrome/WebKit/NomiFun 或本轮集成测试进程。两份 shell 脚本的
`bash -n`、更新清单脚本的 `node --check` 和 `git diff --check` 通过。

下载最初的 180 秒期限不足，断点续传后完成。直接对 Chrome 执行 `--dump-dom` 的一次
20 秒诊断超时，未记为通过；改用产品实际 CDP 路径完成上述验证。新增 sandbox fixture
最初在仅 `shutdown()` 而仍持有连接时等待 EOF，等待超时；修正为释放连接后再等进程退出，
按产品 guard 异步清理语义等待 marker，未为此修改共享 transport 实现。

可复现命令（`NOMIFUN_CHROME_BINARY` 必须指向专门测试用的真实 Linux Chrome）：

```bash
bun test scripts/desktop-build-linux.test.mjs scripts/make-latest-json-linux.test.mjs scripts/release-linux-artifacts.test.mjs scripts/release-native-locks.test.mjs
cargo test --locked -p nomi-browser-engine --lib launch::tests::
NOMIFUN_CHROME_BINARY=/absolute/test/chrome cargo test --locked -p nomi-browser-engine --test linux_sandbox -- --ignored --nocapture
NOMIFUN_CHROME_BINARY=/absolute/test/chrome cargo test --locked -p nomi-browser-engine --test integration_managed_host managed_host_shutdown_clears_only_exact_runtime_profile_artifacts -- --ignored --exact
NOMIFUN_CHROME_BINARY=/absolute/test/chrome cargo test --locked -p nomi-browser-engine --test integration_managed_host --test integration_single_tab drop_ -- --ignored --test-threads=1
NOMIFUN_CHROME_BINARY=/absolute/test/chrome cargo test --locked -p nomi-browser-engine --test integration_single_tab single_tab_headless_has_exactly_one_page_target -- --ignored --exact
```

本次不重复首轮已完成的整个 App/业务测试组，不启动模型请求。不关闭 `RC-LD-01`，不宣称
Computer Linux、桌面 portal、GPU/DPI/IME/托盘、原生安装/升级或 live provider 已通过。
验收结束时重新 fetch，远端仍为 `9c6800f7f`，未出现新的共享文件冲突。此处记录验收时的
工作区快照，后续提交与推送状态以 Git 历史为准；用户文件 `get-docker.sh` 未纳入本轮修改。
