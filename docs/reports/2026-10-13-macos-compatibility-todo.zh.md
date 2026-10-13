# macOS 兼容性核查与修复 TODO

核查日期：2026-10-13（Asia/Shanghai）。起始源码：`main` / `4f37253e3`，工作树干净。
本轮宿主：Apple Silicon arm64，macOS 26.6.2。本文是本轮施工与验收记录；当前架构仍以
`docs/architecture/agent-session.zh.md`、`docs/architecture/browser-platform.zh.md` 和源码为准。

## 范围与完成条件

覆盖桌面启动、原生 Browser、后台网页研究与渲染、Computer/Accessibility/TCC、进程与文件系统、
Terminal/SSH/JS 服务、renderer/桌面壳、音频/Robot/Office、网络代理、打包与更新。
不更改 desktop 880×600 最低视口合同，不引入手机布局，不迁移当前 WKWebView/WebView2 架构。
按用户后续范围说明，已评估无法实现并明确决定不支持的场景（尤其 WK）不进入补齐队列。
不为支持某功能移除权限、进程所有权或 Session 围栏。

每项以实际触发场景、源码证据、修复、针对性测试和必要实机验收分别记录。
源码/夹具通过、真实 App 通过、外部服务通过与签名发行通过是不同证据；未执行的验收保持未完成。
首次失败保留，后续修复结果另记。TODO 勾选只代表该项所写的验收范围完成。

证据可用性更新：恢复检查时，前轮 `/tmp/nomifun-*` audit 目录、对应进程及 exec handles 已不存在，原因尚未确认；源码、冻结 Dev bundle 和已安装的自有 Element Audit bundle 仍在。本文下方对旧临时报告“保留”的表述是当时的历史记录，目前不能通过那些路径重新复核原始文件。历史首 FAIL 与后续 PASS 不改写、不伪造恢复。后续报告、私有数据及自有窗口源码统一保存在仓库 Git 外持久私有目录 `.git/nomifun-macos-audit-20261013/`（0700），新验收另建数据目录并另记。恢复的 Swift authoring 源码不能充当恢复旧 raw logs 的证据。

当前队列共 45 条：43 条确认问题的修复实现与各自所列定向/原生检查已完成，2 条候选经生产调用与源码核查排除。MAC-45 正式Dev权限查询/设置详情与独立组件请求、拒绝、提交、公共投递及精确清理均已通过；横幅可见及正式Dev发送consumer另记边界。Computer观察/输入守卫的首FAIL和最终Store/Host/工具检查均已收齐；当前宿主的AppKit有资源退出与两代重启已通过，正式Dev的完整元素输入 attempt6 及正常退出双证明已通过。
下表的“源码/定向测试完成”不代表全部实机验收完成。原生 child 快捷键和独立重叠窗口的生产AX激活已通过；
独立原生Camera网站Deny/owner排空/隐藏/导航/Close已通过；formal Tauri Computer 当前实例重新检测 AX/Screen 均已授权，正式 cursor/launch 与窗口见证通过，完整元素输入首轮失败及修后 attempt6 PASS 分别保留。媒体首轮pending定位到网站delegate之前的系统授权；
MAC-34修后actual Dev的TCC归属、终端停止、watch重建和真实Command-Q双证明已通过，current媒体attempt2也有cleanup与exact exit0双证明。
当前Agent Session的canonical history、native reasoning字段、真实桌面冷加载及macOS Channel binding定向检查也已完成；该库存项仅在上述证据收齐后改为verified，模型与Channel夹具边界保留。
没有将明确决定不支持的 WK 场景转成新增功能要求。

## 并行分工

| 工作域 | Agent | 重点 |
| --- | --- | --- |
| 原生 Browser | `browser_native` | WK 页面/权限/生命周期/输入/文件、公开 API 边界 |
| 后台网页研究 | `headless_search` | Chrome 发现、版本与摘要、隔离渲染/搜索宿主 |
| Computer 与权限 | `computer_permissions` / `a11y_macos` | TCC、应用启动、窗口激活、坐标与 AX |
| 进程与文件系统 | `runtime_filesystem` / `file_path_audit` / `process_supervision` | shell 环境、进程代际、APFS 别名、SSH |
| 构建发布 | `packaging_release` | arm64/Intel、dylib、签名、公证、更新安装 |
| renderer 与桌面壳 | `renderer_shell` | Command、资源选择、Attached Chrome、更新入口 |
| 其他平台消费者 | `ancillary_platforms` | Office、音频/Robot、VSCode、系统代理 |
| 集成与验收 | 主 agent | 汇总 TODO、共享接线、串行 Cargo/真实 App 验证、审查 |
| 独立启动链复审 | `launch_review` | 两阶段身份、回调时序、控制消息异常、退出证明；与域实施Agent分开复核 |

## 确认的缺陷与实施队列

| ID | 优先级/状态 | 触发与证据 | 修复及验收要求 |
| --- | --- | --- | --- |
| MAC-01 | P1，源码/真实headless验收完成 | `apps/desktop/src/main.rs` 仅 Windows 调用 `headless_browser_runtime::prepare`；macOS 的本地网页研究与后台渲染没有宿主，即使已安装 Chrome | 接入 macOS 独立临时 Profile 的 headless 宿主，校验真实 Chrome.app 版本/完整引擎摘要，保留公网网络约束与进程 owner；定向测试及真实 macOS 渲染/搜索 |
| MAC-02 | P2，源码/实际原生焦点验收完成 | `ui/src/renderer/pages/conversation/Browser/keyboardShortcuts.ts` 拒绝 `metaKey`；WK child 焦点事件不进 renderer，Cmd+W 还可能被宿主菜单消费 | 支持 Command，公开原生本地事件转发精确 Session/target，保留编辑/IME/输入锁边界；renderer 行为回归与实际原生 child 焦点验证 |
| MAC-03 | P1，源码/定向及原生Deny退役验收完成；Grant/采集另验 | macOS WK 媒体授权直接返回 `Prompt`，pending permission 列表为空；隐藏/导航/Agent 接管不能通过现有 owner 排空请求；旧denial还残留到Reload/新授权之后 | 使用同一 typed permission 请求、精确目标、超时与 Grant/Deny completion；失效边界拒绝待处理请求；新doc清旧denial、当前Grant仅清对应kind，迟到旧回复不能清后继状态；定向30项及独立原生Camera五case通过。网站Grant、Mic采集和formal Agent Turn接管链未由该fixture覆盖 |
| MAC-04 | P1，源码/原生进程回归完成 | macOS shell 环境探测裸 spawn、无限 stdout、超时仅杀直接 shell，随后无界 reader join；startup 脚本的后代持有 stdout 可卡住启动 | 复用 Unix managed owner、非阻塞有界读取及后代清理；后台子进程与超量输出反例 |
| MAC-05 | P2，源码/定向/正式Agent启动与原生窗口验收完成 | `nomi-computer/launch.rs` 将 macOS bare app 名称按路径交给 `open`，不能正确按应用名称启动 | 显式 LaunchServices 应用名称调用，检查 OS handoff 结果；独立公开async入口及正式Agent Role都按裸名称启动唯一自有App，handoff、App自身确切bundle/PID marker及CUA窗口身份通过；不替代完整输入或发行态验收 |
| MAC-06 | P1，源码/定向测试及原生遮挡验收完成 | `nomi-computer/fallback_backend.rs` macOS focus 仅点击窗口中心，被遮挡时可作用于另一窗口或控件 | AX 精确窗口身份激活，歧义拒绝；独立同PID两个重叠窗口的实际生产focus入口及CUA前后见证通过；同名歧义/未授权另有定向测试 |
| MAC-07 | P1，源码/两类APFS验收完成 | macOS 路径别名键 NFD+lowercase 漏掉本机 APFS 已确认的 `Σ/ς`、`ß/SS`、`S/ſ` 等价别名，多文件 patch 预检可能遗漏重叠 | 使用系统 Unicode case fold，保持大小写敏感卷的区别，保留原始路径与权限校验；验证在任何发布前拒绝重复 |
| MAC-08 | P1，源码/原生recovery回归完成 | macOS orphan recovery 先读取 birth identity 后 `killpg`，检查与 signal 间可能发生代际复用；leader 消失也不能证明清理 | 逐个kernel generation冻结/信号/终止证明；root消失时仅只读观察原contained PGID，未证明清理返回typed Unproven，hosting保留identity/未知状态；不扩大到未记录的escaped descendants |
| MAC-09 | P1，源码/Intel装配验收完成 | Intel release build 的 rpath 只写 `RUSTFLAGS`；存在 `CARGO_ENCODED_RUSTFLAGS` 时 Cargo 忽略前者 | 保持两种 flags 的 Cargo 优先级并将 ONNX Frameworks rpath 注入实际生效字段；命令级测试通过，Intel 编译/产物验收单列 |
| MAC-10 | P2，源码/UI验收完成 | 后端已有 Attached Chrome connect/status/disconnect 和资源，renderer 固定 managed-browser，用户无法完成连接→选择→Session 绑定 | 在资源选择处显式连接与选择 provider，保留冻结授权、默认内嵌浏览器及 attach-only 所有权；UI/API/Session消费回归与隔离HTTP视觉已过，个人Chrome实际连接单列 |
| MAC-11 | 已排除 | 曾怀疑 updater 未消费安装上下文；`ui/src/common/adapter/tauriUpdater.ts` 实际通过 `installUpdateWithPreflight` 调用 `tauriGetUpdaterInstallContext` | 已有 DMG/AppTranslocation 安装前拒绝路径和定向测试，不作无根据修改 |
| MAC-12 | P2，源码/定向测试完成 | VSCode 检测只认 PATH 与 `/Applications`，漏掉用户级 `~/Applications/Visual Studio Code.app` | 增加同一发现路径并保留 PATH 优先级；实际程序参数回归通过 |
| MAC-13 | 已排除 | 曾怀疑 `scutil --proxy` 的嵌套代理覆盖全局值；生产调用不带 debug，Apple configd 的公开实现会移除嵌套代理字典 | 以生产调用和官方源码核查排除，不作无根据修改 |
| MAC-14 | P2，源码/定向及当前正式Dev通知详情GUI完成 | 通知设置 deep link 写死生产 bundle ID，Dev App 打开错误目标；`open` 仅 spawn，失败仍报成功 | 从实际 bundle 读取身份，无身份打开总页；等待 `/usr/bin/open` handoff 并报告失败；身份/错误定向测试 |
| MAC-15 | P1，源码/Intel装配及清单回归完成 | macOS release 固定 arm64，并在普通 arm append 使用 `--macos-arm64-only` 删除同版本 Intel updater entry | 增加显式 Intel 构建/发布参数，普通 append 保留真实其他架构资产；删除仅显式策略触发；脚本和manifest回归通过 |
| MAC-16 | P1，源码/真实headless验收完成 | macOS Chrome 主入口只是 launcher，仅对它计算 SHA 不覆盖实际 framework、libraries 和 native helpers | 指纹覆盖实际 versioned engine/helper 集合及版本指向，拒绝越界/丢失 bundle；变更 framework/helper 而 launcher 不变的反例与实际 headless 验证 |
| MAC-17 | P2，源码/定向测试完成；权限实机待验 | macOS 只读 cursor_position 构造 Enigo，导致 Computer observe 额外要求 Accessibility | 使用只读 CGEvent 坐标入口，保留 observe 的 Screen Recording 契约；坐标/授权分离回归 |
| MAC-18 | P1，源码/定向测试完成 | click 的按下后失败路径可能缺少 mouse-up，留下系统按键状态 | release guard 保证失败清理；注入失败反例验证按键释放，不以正常点击测试代替 |
| MAC-19 | P1，源码/定向测试完成；多显示器待验 | capture 几何解析失败时猜测物理像素及原点，可能将操作投射到错误显示器位置 | 无可信几何时明确失败，不制造默认坐标；多显示器/几何失败针对验证 |
| MAC-20 | P1，源码/定向及正常授权AX路径原生完成；撤销/密码/超时等另验 | macOS AX actor 的节点引用缓存、树遍历预算、密码值、目标窗口匹配、动作语义与超时分类有缺陷 | fresh walk/精确 live ref/有界树/secure 值过滤与真正窗口激活；LeftClick只AXPress，DoubleClick真实输入回落；mutating AX timeout保持Uncertain，经既有Effect链禁止像素补发/零效果结算；测试和独立原生 fixture 分别记录 |
| MAC-21 | P1，源码/原生回归完成 | 工作区 delete 校验后按 ambient 路径删除；父目录被替换可删除工作区外文件 | 在已授权父目录句柄下完成nofollow检查与删除；临时目录outside containment及write+search-only父权限回归过；不声称POSIX原子compare/unlink |
| MAC-22 | P1，源码/原生回归完成 | macOS 大小写别名可能让 `.NOMIFUN` 绕过 `.nomifun` 保留 owner 根检查 | 保留根ASCII大小写别名拒绝，现存uppercase目录与pre-I/O回归通过；不阻止普通相邻路径 |
| MAC-23 | P1，源码/原生PTY回归完成 | macOS PTY host 突然退出后仅杀 leader PGID，同 SID 的独立 job-control PGID 可泄漏 | launcher保有精确waitpid/SID anchor，session封存后发nonce receipt；实际SIGINT不依赖ISIG/可写stdin；完整fault/PTY回归及最新精确directChild反例通过 |
| MAC-24 | P2，源码/实际启动回归完成 | Finder/launchd 缺少 `SHELL` 时跳过真实用户 login shell，漏掉 zsh 的用户工具路径；Terminal 也降为 `/bin/sh` | 共用真实账户 shell resolver，显式非法路径拒绝；查账户放在隔离 helper，启动有界且不引入环境并发写 |
| MAC-25 | P1，源码/实际启动回归完成 | storage relocation 可能先创建 Tokio/SQLite worker，随后调用要求单线程的 PATH 环境 mutation 与新 fork helper | 三个 host 的 PATH enhancement 前移到 storage coordinator 之前，runtime cache 仍按最终数据根初始化；架构/实际启动验证 |
| MAC-26 | P1，源码/定向测试完成 | optional Chrome metadata 打开非普通 Info.plist（如 FIFO）时可能挂住后端启动 | 打开前验证普通文件、大小与bundle边界；FIFO/外部symlink回归，不运行候选Chrome |
| MAC-27 | P1，源码/原生回归完成 | Unix workspace write/atomic publication 保留环境路径而非已授权父目录句柄，目录替换可能改变实际写入位置 | PreparedParent capability完成相对stage/验证/发布；ACL/XATTR、readonly、immutable、no-replace、未知结果和原stage命名前缀均保持并已验；Linux预取可sync fd避免O_PATH回归 |
| MAC-28 | P1，源码/原生回归完成 | Unix workspace_read_dir/metadata丢弃已解析authority，环境路径替换可影响实际列举/观察 | 受限目录句柄列举，cursor持续验证namespace；metadata不引入文件字节read要求；四项原生替换/仅search权限回归过 |
| MAC-29 | P1，源码/原生PTY回归完成 | 新PTY完整回归揭示轮询器消费已认证Quiescing后，后台worker没有继承该回执，正常退出报清理未知 | 仅从认证frame保存receipt并跨handoff传递，不放松native cleanup守卫；quick exit与全部fault回归 |
| MAC-30 | P1，源码/离线请求回归完成 | 桌面fetch/XHR按字符串前缀/首字符判断backend，将协议相对外部地址、相邻端口等误附local-trust header；Request还被重写成URL字符串丢method/body | 按实际URL.origin匹配；只有native Request/URL参与自动附header，保留Request/显式init headers与body/signal；synthetic token离线矩阵验证，不发送真实secret |
| MAC-31 | P1，源码/原生回归完成 | 新PTY payload有自己的PID，可自身setsid离开launcherSID；仅seal原SID后waitpid会让forcekill超时 | 确认exact unreaped directChild后仅对该payload positivePID SIGKILL；随后seal/reap再发nonce receipt；同一native反例首FAIL，修后PASS，不扩大unknown descendants权限 |
| MAC-32 | P1，源码/签名装配验收完成 | 正式App启用Hardened Runtime却没有camera/audio-input entitlement；Info.plist用途说明不会自动补充资源声明 | plist只添加两项实际功能资源，Tauri与完整/增量App签名统一传递；库不带App权限，缓存跟随内容重签；实际Mach-O签名提取与装配53项通过，真实TCC/采集另验 |
| MAC-33 | P1，源码/实际binary及生产接线回归完成 | 实际Intel OfficeCLI 1.0.156的普通`--version`会更新已安装客户端skill/config；生产ToolDoctor、能力probe及预览均只禁resident，未禁更新，还可能影响批准的固定程序身份 | Office专属probe/render强制`OFFICECLI_SKIP_UPDATE=1`，既有Office资源声明同步；真实固定binary在两个等价privateHOME下before改三哨兵/after全部保留；生产入口反例通过。只关闭该更新路径，不声称全HOME隔离或任意手工Agent shell强制注入 |
| MAC-34 | P1，源码/实际LS、终端、watch与Command-Q退出完成 | 直接spawn已签名`.app/Contents/MacOS/nomifun-desktop`不保证TCC主体为该App；实际media PID25031 Camera申请/提示subject为Codex；同binary/signature仅LS启动的PID25529/PPID1后主app/GPU/coreaudiod subject均为auditID | 公开NSWorkspace两阶段新实例启动，保持nonce+kernelpeer/PID/path/birth与preworker knote、late callback身份确认、actualApp正常exit0及verifiedcleanup共同证明；修Darwin对端退出后的timeout修改EINVAL及CLI提前断lease，FIFO/sticky未知态/no force保留，不放宽旧directchild PPID，保留cwd/DATA_DIR/私密env |
| MAC-35 | P2，源码/定向真实管道回归完成 | Mac构建helper runCommand(capture=true)在child exit时返回stdout，但stdio可能尚未关闭；fast-build随即JSON.parse Cargo metadata，有截尾/分块Unicode隐患 | 等待close/stdio完成并连续UTF-8解码；实际parent exit0而writer后代仍持管道反例，修前提前settled FAIL、修后完整中文JSON PASS；artifact collector也改直接chunk+StringDecoder及EOF尾行，原tinyreport首次失因仍未知，不以此冒称复现 |
| MAC-36 | P1，源码/Store与Host定向完成 | ComputerRoleHost输入后把观察counter重置0；再次观察复用G1，过期expected_generation可被接受。fetch_add后saturating也不能阻止Atomic在MAX回绕 | 生产改用精确committed tool/call-started.seq，不再维护可回绕counter；失效撤销授权，保留pre-native失效、surface kind和新观察正控。Store因果查询11项及最终Host21项通过；首FAIL和两次夹具失败保留。MAX仅test边界，不声称SQLite可产生u64MAX |
| MAC-37 | P1，源码/真实Store与Fake port集成完成 | 生产只创建一个ComputerTool，却按Session/snapshot/resource建立独立Host lock/counter/kind；B刷新共享ref/geometry后，A旧输入可被解释成B当前cache，canonical未结算Effect锁不能挡顺序观察改绑 | 同一物理tool共享串行及观察authority，精确绑定Session/Turn/native lease/Snapshot/provider/principal/resource；拒绝A旧token与猜填B token但保留B合法lease。新观察先撤旧cache，input保留已准入几何至设备调用返回。实际Host/Store集成及序列化检查通过，不增加Session账本 |
| MAC-38 | P1，源码/工具与Host定向及当前正式Dev元素输入完成 | right/double与ordinary click/setValue回退使用旧center；Backend后重复效果；pointer→type焦点漂移；SetValue只插入却报整值成功；整数point越界；Uncertain hint中的stale短语误判零效果 | click/right/double实时ref/gen/bounds及遮挡验证，Backend/Uncertain不补发。保留原生setter，退役自动SetValue pointer/typing及无consumer接线；Unsupported在pointer前拒绝，以精确专属类型结算failed而非stale/unknown。分类严格grammar；整数point在范围内。六组首FAIL与20/111/21项最终检查分开，Fake不替代真实输入 |
| MAC-39 | P1，源码/真实Store冷重开与factory集成完成 | 正常冷启动重建Host/tool/actor counter为0；同Session新Turn观察又G1，canonical历史保留旧G1/ref1，新合法调用可能误用旧数字。现有fence保护调用准入，没有进入观察来源 | 复用精确committed tool/call-started.seq及current execution scope；Store旧epoch拒绝通过。Host升级反例实际保存旧tool/result、关闭Turn并在后继Turn重建factory，拒旧token且保留fresh正控，设备明确Fake；不用clock/random/head.last_seq或新Session账本 |
| MAC-40 | P1，源码/目标测试/当前宿主idle及liveWK原生退出完成 | 普通NSApplication.terminate/System Quit不进入项目ExitRequested协调器；Tao0.35.3的applicationWillTerminate直接Exit，AppKit也不执行main末尾。旧cleanup与认证receipt仅在受控app.exit/run_return后完成 | 公开applicationShouldTerminate接已有cleanup，在允许退出前发送one-shot认证receipt；保exactkernel与cleanup双证明及原delegate。首public terminate kernel0/noReceipt FAIL保留，修后idle及最终liveWK公开退出双证明PASS；不把OS logout/Release/Intel分面冒称通过 |
| MAC-41 | P1，源码/163项Rust及72项JS/当前宿主livePTY+WK两代Dev交接完成 | 现役application.restart/updater relaunch走process plugin；不可取消的RESTART_EXIT_CODE callback在main同步等backend，已初始化WK清理又等main GCD。macOS updater未调用现配置on_before_exit；Tauri relaunch也不返回main末尾或经Dev LS admission | macOS正式restart先由保留owner异步cleanup，真verified才handoff；同官方JS入口及正常原生菜单，无raw旁路。Dev旧receipt+exactkernel0后同lease新descriptor/nonce/publicLS，两代真实App通过；mac安装前preflight保F32 false不是cleanuptrue。实际installer替换/Release successor另验，不属于WK明确不支持范围 |
| MAC-42 | P1，窄修/确定性原生组件及正式WK正常创建退出验收完成 | DeferredEngine首次初始化提交TauriTask；若原event callback尚未返回且进入真实native modal，Tauri guard不消费该Task，shutdown又等待同一初始化结果，modal内清理无法完成 | 首次初始化改用同一公开CF主循环helper，保sameowner/gate/closing/watch结算及真实Engine mainthread要求；standalone smoke同helper接线。首5s FAIL保留，修后同断言实际在原callback仍active时结算且modal completion1/deadline0；最终两代Dev实际创建liveWK后重启/公开退出另有记录。不能用概率HTTP竞态或fake settled代替，不宣称所有外部AppKit Later天然死锁 |
| MAC-43 | P2，窄修与29项库回归及当前正式Dev原生点击完成 | macOS copy_actions把SDK读取失败/null/错误CF类型当empty list；正式ordinary click_element因此误判confirmed Unsupported，进入guardedpointer。此时位置/前台守卫仍存在，不宣称错目标或已发生真实输入事故 | 严格动作列表读取返回Backend未知，只有成功且完整的合法list确知无press才Unsupported；非string/null元素与截断亦不可推absence，copy-rule均衡释放。保AXPress/knownempty正控、不改变派发后Uncertain/Host分类；首2PASS/3FAIL与修后29PASS分开，CF/Recorder不代native点击 |
| MAC-44 | P1，123项库回归及当前正式Dev完整元素输入完成；首FAIL保留 | 已守卫坐标的绝对move交给Enigo后，按钮事件重新读当前系统位置；异步move尚未生效或cursor漂移时，按下/释放不再携带同一次被授权的目标。macOS Enigo位置读取还混用AppKit坐标与pixelsHigh。本轮owned窗口仅收到mouseMoved，ordinary工具却returned，接收端断言失败；首事件的button真实位置没有采集，不能冒称已归因到Retina比例 | macOS pointer事件直接携带该次已验证Quartz坐标与明确clickCount，保main-thread/TCC前提、press前release登记、故障释放与已开始worker保留；不以sleep/重试补效果。当前只读宿主bounds和pixels均1710×1112、两种cursor读取一致，单位风险与实际首失败分开。12项纯Recorder/故障测试及全库123 PASS，Intel/Windows crosscheck；新私有scope正式原生左/右/双击、整值替换及readonly前置拒绝 attempt6 PASS，receiver与canonical Effect分别证明 |
| MAC-45 | P1，Rust8项/UI26项、正式Dev query/settings及独立通知组件原生验收完成 | 锁定 notification SDK 的所有desktop权限查询/请求恒Granted；renderer读取缓存，已授权行隐藏系统设置；Dev发送借用Terminal身份，NSBundle.bundleIdentifier全局swizzle并丢弃发送错误。旧UI“已授权”不能当真实OS权限证据 | macOS同名plugin使用公开UNUserNotificationCenter、真实App bundle和原ACL；真实query/request、一次callback及有界等待，submit返回OS接受或实际错误，不能当banner已见。Win/Linux保留原plugin；前端直接等待IPC、通知ready仍可打开实际bundle设置；旧异步结果不覆盖新状态；不补notification-click DEGRADE_STUB |

## 验收空白

- [ ] 最低 macOS 14 真实 App、Intel Mac 真实 App/硬件。
- [x] `agent-architecture-inventory.json` 的 `macos.current_agent_session`：当前源码在本轮macOS宿主完成真实Tauri Session输入→工具回执→持久终态→正常退出→冷加载历史/High字段/产物，以及两个macOS Channel owner/binding定向检查。四分面证据收齐后改为verified；localhost模型与RecordingPlugin不代表外部服务商、reasoning wire或IM投递。
- [x] 临时 case-sensitive APFS 磁盘映像挂载后执行真实文件服务的大小写/Unicode distinct及NFC/NFD alias验收；通过后正常卸载自己创建的映像。
- [x] 本轮native arm64 Dev App实际启动、UI API加载及supervisor退出；实际安装Chrome独立headless搜索/渲染通过（开发态与后台入口证据）。
- [ ] Finder直接冷启动；不能由CLI启动/退出或API测试替代。
- [x] current独立原生App的Camera网站权限Deny/owner排空/隐藏/导航/Close五case及可信Start/HTTP见证；system Camera公开Authorized前提成立、Mic未授权，不代表网站Grant、实际采集或formal Agent Turn接管链。
- [x] current source Dev实际GUI加载后真实Command-Q退出：App cleanup回执true、exact kernel exit0、controller final_verified、整条开发命令exit0均通过。
- [x] 独立同PID重叠窗口的生产Computer AX激活；CUA确认遮挡窗口→目标窗口及截图。仅现有shell责任身份，不代表formal Dev系统授权已完成。
- [x] formal Tauri Computer 的系统权限：用户明确授权 Dev 的 AX/Screen 后，当前受控 LS 实例重新检测均已授权；未扩展到 Dev Mic/Camera 或其它 App。
- [x] formal Tauri Computer 完整元素输入：普通点击首FAIL保留；MAC-44 修后 fresh attempt6 使用正式Dev与独立owned窗口，真实左/右/双击、标准NSTextField整值替换及readonly前置拒绝全通过，11工具/6Effect最终无unknown；整值写入区间全窗口pointer/key计数前后相同。未采集用户屏幕；当前Apple Silicon/macOS26宿主，不代其它硬件。
- [x] MAC-45 正式Dev真实OS权限及MAC-14通知详情落点：native denied与OS开关off一致；用户授权开启后fresh query成为granted，ready仍有设置入口；修正id参数后从通用页直接到Dev通知详情。
- [x] MAC-45 独立组件真实权限请求回调、未请求时发送拒绝、授权后系统提交与公共投递查询以及精确清理；正常GUI Finish有cleanup receipt及exact kernel0双证明。正式renderer当前无实际发送consumer，该组件不能冒称正式Dev发送链。
- [ ] MAC-45 横幅/通知中心可见展示、真实OS NSError拒绝分面；不能由addRequest callback或getDelivered记录代替。没有为验收添加正式产品发送入口。
- [x] 当前源码原生WK child焦点下Cmd+L/R/T/W真实GUI及独立HTTP见证，当前Apple Silicon/macOS26开发态完成；不是Intel/macOS14实机证明。
- [ ] 真实外部模型/服务及用户授权个人 Chrome 的完整产品流程。
- [ ] Developer ID 签名、公证/Gatekeeper、安装后更新、跨版本升级与 Windows GUI 回归。

签名环境、用户权限或外部设备不可用时记录具体原因；这类空白不能被 mock、类型检查或 ad-hoc 包填平。

## 已核查且未新增修复项的范围

以下结论来自本轮当前源码及静态调用链，而非“另一平台能跑所以 macOS 也能跑”。Office在后续实物验证发现MAC-33，已从本表移入修复队列。
这些范围未发现可证实的新增 Darwin 实现缺口；表中存在实现不代表列出的实机条件已经验收。

| 范围 | 当前实现证据 | 仍需区分的验收 |
| --- | --- | --- |
| Robot Opus/容器 | vendored opusic-sys静态libopus与共享Symphonia解码，无Windows DLL依赖 | 真实机器人上下行音频 |
| Robot Silero/ONNX | `nomifun-robot/Cargo.toml` 的Intel Darwin API23/ORT1.23.2分支；模型内嵌、Energy VAD失败回落；`scripts/lib/macos-onnx-runtime.mjs`校验供应SHA与x86_64库 | Intel发行态加载/真实推理；fallback成功不能证明Silero成功 |
| renderer语音录制 | `useSpeechInput.ts` 按MediaRecorder.isTypeSupported选择包含audio/mp4的编码，并支持webkitAudioContext | 真正原生getUserMedia/TCC、录音、ASR服务商请求 |
| 系统代理/Fake-IP | `nomifun-net/src/proxy.rs` 有界scutil查询与HTTP/HTTPS/SOCKS/bypass；socks5h远端DNS，共享HTTP client接入；Fake-IP有接口所有权信号 | PAC、企业代理、VPN/TUN与第三方SDK完整路径 |
| SSH/ssh-agent | `nomi-ssh/src/connection.rs` 共享Russh连接/远端shell/SFTP；Unix socket agent签名及缺SSH_AUTH_SOCK错误明确 | 真实远端、Keychain agent、特殊网络环境 |
| 本地WebUI/Robot LAN | `nomifun-app/src/desktop.rs` 分离永久loopback与可切换LAN listener；lan_endpoint共享接口/route来源IPv4或显式地址 | macOS Local Network权限、防火墙与外部设备可达 |
| Webhook/Channel | `nomifun-webhook/src/sender.rs` 生产sender接动态nomifun_net::http_client；Channel共享Reqwest/协议，无本轮证实的Darwin缺口 | 服务商登录、长连接、真实回调、每个SDK的代理行为 |

## 后续原生验收执行点

- Cmd+L/R/T/W已在本轮current source隔离Dev App执行并通过；后续只有相关生产代码变化或新失败时才重验。driver要求实际App PID、Session及原生操作证据，不能用HTTP或DOM直接调用代替。
- 媒体拒绝/隐藏/导航/owner切换使用独立`NomiFun WK Media Audit.app`，网页层全部Deny，不录制或上传设备内容。系统相机NotDetermined时getUserMedia会先触发OS提示，不能将“网站Deny”写成“不触发系统请求”。用户后来明确仅允许该audit App的系统相机访问；attempt2 before_start公开Camera已Authorized、Mic仍NotDetermined，root没有点击新的OSGrant。上述网站五case已通过；需要扩大到其它App或新的设备授权时仍须在操作时确认。
- 本轮“权限拒绝/退役”仅指网站UIDelegate已经交给typed owner的请求。系统Camera/Mic提示由OS管理，公开WK owner不能取消该提示；该限制不新增为WK补齐任务。Camera-only夹具不要求Microphone系统授权。
- 当时的媒体包为`/tmp/nomifun-macos-launchservices-final-audit.i2A4Nj/media-system-prerequisite/`，当时验签通过。attempt1系统前提未满足而退出1；用户确认该audit App相机访问后，attempt2在Camera公开Authorized前提下通过五项网站Deny/retire与整体清理/exit0。首次失败与修后结果不改写，但包及原始日志当前均已缺失；这些旧路径不是可执行入口。后续仅相关源码变化、新失败或新scope需要重验，须从当前 `scripts/validation/package-macos-browser-media-fixture.mjs` 重新打包到新的私有目录；共享控制器仍为唯一启动入口，package helper拒绝覆盖已有目录。各case实际可信Start/可见/聚焦/HTTP证据不代替自动后台触发或网站Grant。
- 媒体fixture只读检查public AV Camera状态。整个流程共用120s NotDetermined等待预算，系统Authorized后才启动原网站8s限时；真实当前网站请求已到达时优先处理并记录OS查询不一致。预算耗尽后最多再查询3s，仅仍NotDetermined时拒绝；Denied/Restricted/Unavailable明确为前提未满足。只给media的probe/watchdog加120s，ordinary非media不变，production网站owner30s不变。这是验收前提分离，不是新的production缺陷或OS Grant实现。
- 遮挡窗口fixture及`macos_fixture_probe`仅针对带本轮audit标题的明确PID；要求已有AX/Screen权限，未授权时记录失败，不自动授权。
- 本轮临时数据位于Git外`/tmp/nomifun-macos-product-audit.dwZcaH/`；首FAIL媒体包目录与成功`media-bundle-final`目录分开保留。临时APFS已卸载；Dev App/Vite已经supervisor正常关闭，独立fixture服务已停。恢复GUI验收时用新fixture数据根、重新确认PID/端口，不能沿用历史快照；旧数据及失败证据保留。

## 验证日志

运行日志、临时 Profile、数据库与凭据均放在 Git 外。以下按阶段保留首次失败和后续结果；旧阶段的“当时待验/未知”不覆盖摘要与执行点的当前状态，不能以新的PASS改写旧失败。代码后续变化时继续补最终集成结果。

| 命令/范围 | 当前结果 | 证据限制 |
| --- | --- | --- |
| `cargo test -p nomifun-browser-macos` | 首轮25 PASS；第二轮27 PASS；权限恢复补充后最终30 PASS | 包含native shortcut映射、权限代际和拒绝→Reload/Grant状态恢复；实际网页/TCC验收另记 |
| `cargo test -p nomifun-shell --lib` | 首轮69 PASS；最终71 PASS | 包含系统真实不存在应用的handoff失败；不表示真实VSCode/Terminal GUI已验证 |
| `cargo test -p nomi-browser-engine --lib headless_page::macos_release::tests` | 首轮4 PASS；最终6 PASS | native helpers/Resources变更反例通过，实际Chrome消费者另记 |
| `NOMIFUN_SEARCH_CHROME=... cargo test -p nomi-browser-engine --lib headless_page::tests -- --ignored --test-threads=1` | 6 PASS，实际安装 Chrome 独立 headless | 包含真实公网 render、DNS/loopback限制、跨来源渲染、取消/清理；不是可见 WK 浏览器或个人 Chrome attach |
| desktop-ui / browser-platform / process-runtime / agent-session boundary | 首轮及最终集成全部 PASS | 不替代真实原生交互；renderer仍遵守最低880×600 |
| renderer 定向回归、i18n、typecheck | agent 185 PASS；root重跑关键4文件53 PASS；i18n/typecheck PASS | 实际 renderer+Guid hook的隔离HTTPfixture在880×600/1280×800完成显式attached Session创建及断开阻断；不是个人Chrome实际连接 |
| release shell rpath | 8 PASS | 首次 Bun filter 路径运行的 4 FAIL 保留；明确 `./scripts/...` 文件运行通过 |
| release architecture/native-lock/upgrade-gate/updater merge | 最终与构建flags共30 PASS；App/DMG/dev合约46 PASS | 包含真实 hdiutil ULMO/UDZO verify+read-only mount；不表示Intel实际产物、DeveloperID签名或公证通过 |
| desktop headless unit 编译首轮 | FAIL，`Shortcut: Clone` 约束遗漏 | Tauri `emit_to` 要求 Clone，已修 derive；后续结果另记 |
| `cargo test -p nomifun-desktop headless_browser_runtime::tests` | 修复编译后4 PASS，3个native测试默认ignored | XML/binary plist、版本/身份、FIFO/symlink拒绝；实际Chrome协议与搜索需显式执行 |
| `cargo test -p nomi-process-runtime --test startup_probe` | 首轮及完整最终轮1 PASS | standalone父进程线程/超时owner和完整completion frame验证 |
| `cargo test -p nomi-process-runtime --test parent_death macos_parent_death -- --include-ignored` | 首輪3 PASS，实际独立SID/PGID反例 | 首次执行时已含修复，不能编造为修复前FAIL；后续interrupt/partial-frame修复后需最终重跑 |
| Computer/A11y 首轮编译 | 2次FAIL：新增RawFd import遗漏、并行施工的recovery模块尚未生成 | 测试未开始；完成共享runtime施工后统一重跑，不将此归因于上游或权限 |
| `cargo test -p nomi-a11y --lib` | 最终13 PASS | 匹配/缓存/密码值/动作语义/error分类；原生TCC与遮挡窗口另验 |
| `cargo test -p nomifun-runtime --lib shell_env` | 20 PASS | 包含真实有界shell执行、后代持stdout、超时/噪音/大PATH排空 |
| `cargo test -p nomi-process-runtime` | 首轮编译FAIL：libc未暴露PROC_PGRP_ONLY；补本机SDK常量后完整lib首轮174 PASS/7 FAIL | 6项launcher/payload PID角色夹具需精确更新，1项真实Quiescing receipt丢失；都已修，最终结果另记；integration首轮未执行 |
| process完整lib第二轮 | 180 PASS/1 FAIL | 原7项已过；新launcher与payload身份尚有一处supervisor fixture断言不符，现已按sameSID/实际PPID及双进程cleanup精确更新，不放松publication顺序 |
| process第三完整轮 | 304 PASS（181 lib + 123 integration），0 FAIL/ignored | 原生PTY/parent-death/startup/Seatbelt/精确代际等全过；随后preACK不fork用户代码的权限收口单独定向复验 |
| startup实际链 | 1 PASS，architecture 2 PASS | 私有HOME、缺SHELL、minimalPATH、真实账户profile及后代stdout，实际enhance_process_path保持有界 |
| 初始化请求信任 | 9 PASS，72 origin断言及Request/URL语义 | actual source离线VM+synthetic token；没有发送网络或真实secret，原误附header证据与修复后结果分开 |
| file首轮完整检查 | lib364 PASS/2ignored；多数integration PASS；macos_workspace 7 PASS/3 FAIL/1ignored | 三失败是新stage无必要更名使现役“.nomifun-patch-”辨认失效，已恢复原前缀，保留readonly/ACL/immutable成功条件不变 |
| case-sensitive APFS opt-in生产链 | 1 PASS | 实际256MiB sparsebundle，八种case/Unicode名称独立，NFC/NFD别名发布前拒绝；已正常detach自建disk4 |
| 文件后续验收 | regular合计541 PASS/3ignored，case-sensitive opt-in另1 PASS | stage前缀恢复后Mac10项全部过；其余snapshot/ZIP73项过；两项未跑为512MiB压力与fixture子进程入口，case-sensitive已显式跑 |
| Computer定向lib | 首轮91 PASS/1 FAIL/7ignored；修测试文字后92 PASS/7ignored | 新Uncertain断言文字不一致，实际未知结果/禁止重试逻辑未改；A11y13项另已过，真实设备/窗口操作单列 |
| App Computer/Session消费者 | 首轮18 PASS/1 FAIL；修基线陈旧测试后20 PASS | HEAD `7317bd993`已将Computer设可选，旧test未同步；production无改，新增无绑定Observe/Input/Launch→ResourceCardinality且0executor |
| 原生recovery领域消费者 | boot reaper10 PASS、AppHosting restart3 PASS | root消失/live group与错误birth不可被清为Stopped，保留原identity和Unknown；无新增ledger/legacyAgent导入 |
| 最后PTY权限/cleanup收口 | preACK后Unix60 PASS；known direct escapedSID首次0 PASS/1 FAIL，修后PTY14+parentdeath3 PASS；最终Unix60再次PASS | SIGCHLD default/flags0保持exactwaitid lease；同一native反例前后分记，positivePID只对未reap直接child |
| Mac Chrome供应指纹最终 | 6 PASS | helper/framework/resources单项变更、越界alias、缺失文件与standalone兼容；实际协议/搜索验收随后补 |
| Desktop实际Chrome供给消费者 | 3 PASS，36.89s（测试执行，不含编译） | 真正安装release协议/product一致、匿名公网搜索、错误product在搜索前拒绝；完整source已编译并真实使用，不连接个人Chrome |
| Intel核心交叉检查 | `nomifun-browser-macos` / `nomi-a11y` / `nomi-process-runtime` / `nomifun-file` / `nomi-computer` PASS；最后权限恢复改动browser再次PASS | x86_64-apple-darwin source；不是Intel机器运行或完整发行产物 |
| 真实Dev App启动/开发态退出 | native arm64完整App PASS，/health HTTP200，实际UI API200；watch重建后再次启动并经supervisor正常退出 | 本轮独立desktop-data；60758/63107及60982/70623为阶段快照，当前已停；签名adhoc、Vite开发，不是正式发行或最终原生GUI验收 |
| 原生媒体bare首轮 | FAIL：真实WK delegate没有产生权限请求 | 设备名只在硬件核查摘要确认MacBook Air相机；后续safe诊断secure/API均true、视频/音频各1，无拒绝error；bundleIdentifier为空，用途说明却存在，不能直接归因缺说明/设备/TCC |
| 完整media fixture首次包装 | FAIL（launch前）：整文件SHA在合法codesign后变化 | 需证明签名前供应与签后签名/代码区分别一致，不降低签名验证；后续包使用新目录，不覆盖首次失败 |
| 完整media fixture后续包装 | 3 PASS/17 assertions；真实current example包装PASS，codesign deep/strict PASS | source=pre-sign-copy、signed-cache=final-copy分别证明；独立audit身份/新数据根，未launch、无Grant/采集；首FAIL目录保留 |
| 原生窗口/快捷键验收 | 锁屏阶段保持未验证；本轮恢复UI后Command四项实际PASS，窗口激活仍待验 | 同一App PID23156/确切Session，CUA每次先聚焦WK验收输入，再发真实Cmd+L/R/T/W；HTTP初页1/导航+重载2/新页1，加AX/视觉两标签→一标签且主窗口保留；最终driver PASS |
| Camera/audio signed资源装配 | 53 PASS，264 assertions，plutil lint PASS | 实际synthetic Mach-O adhoc签名恰好两true keys，incremental reseal保留；两次抽取显示格式的31 PASS/1 FAIL独立保留，`--xml`后通过 |
| 实际Dev App签名声明 | PASS：codesign提取恰好camera/audio-input两项true | 验证已构建真实App的资源声明，不表示TCC授权或实际采集通过 |
| Windows AX交叉检查 | `nomi-a11y` x86_64-pc-windows-msvc PASS | 共享Uncertain分类改动通过编译；不是Windows GUI回归 |
| 登录shell与Terminal消费者 | login_shell相关4 PASS；Terminal types14 PASS | 真实账户查询、显式shell优先/非法值、shell sentinel和spawn消费者；不表示所有用户profile组合已验 |
| 实际只读cursor查询 | `nomi-computer` ignored cursor_position_real显式执行1 PASS | Quartz只读入口实际返回有效坐标；未移动鼠标、未新增系统授权或绕过锁屏 |
| 完整Intel release编译/装配首轮 | release编译13m13s PASS；App+ULMO DMG生成；整条`MACOSX_DEPLOYMENT_TARGET=14.0 bun run build:mac intel`退出3/FAIL | 后置release-lock明确拒绝dirty tracked worktree，未commit或绕过guard；这是MAC33修复前的阶段产物，不是最终修后发行包 |
| Intel App/DMG实物验收 | PASS：x86_64、minOS14.0、Frameworks rpath、ORT API23实际dlopen、两项entitlement、strict签名；DMG verify/只读挂载比对21项bytes/mode/link | ORT1.23.2仅x86_64/minOS13.4，所有非系统依赖随包；挂载App也strict签名通过，已正常detach。Rosetta加载不等于Intel/macOS14硬件运行。原ARM bundle已备份并字节恢复，dist旧版本保留；新DMG已移到临时目录并标记pre-MAC33.audit，避免当成最终包 |
| Intel OfficeCLI嵌入实物 | 34,832,096bytes与批准SHA一致，x86_64/minOS12.0，实际`--version`=1.0.156 | 首次探测未隔离HOME，意外刷新真实两skill及config；后续修复验证全部privateHOME，原始旧bytes没有预先备份，不能猜测恢复 |
| Office普通命令副作用before/after | before“不改HOME”合同FAIL：两skill+config均被改；after相同binary且SKIP=1，三者bytes/mtime全保留，version仍PASS | 两个等价独立HOME、NO_AUTO_RESIDENT=1；sandbox拒绝真实三用户路径write及network。vendor原始SHA执行后仍一致；这个反例本身不证明production入口注入，另有两项入口回归 |
| MAC33生产接线 | managed verify反例1 PASS，render_html integration反例1 PASS，Agent Office资源契约1 PASS | 独立privateHOME test进程继承SKIP=0，真实生产代码必须覆盖为1；fixture缺flag则改哨兵，不降低生命周期cleanup/能力验证要求 |
| MAC33后的架构边界 | Agent Session及UARC boundary PASS，diff check PASS | UARC仍报告1项macos.current_agent_session未验证；未降低completion判定或修改已冻结Session事实 |
| MAC33后Intel源码交叉检查 | nomifun-runtime/nomifun-office/nomifun-app（browser-use/computer-use）x86_64-apple-darwin PASS，1m56s | 覆盖最后生产flag与资源契约；不是MAC33后完整App重建、Intel/macOS14实机或正式发行证明 |
| 最新native arm64 Dev App与Command实物 | 完整current source App重新build/run PASS（38.63s编译）；CUA实际操作+driver verify PASS | 新隔离desktop-data，--no-watch，未触发外部模型/个人Chrome。证据与plan/receipt/report在`/tmp/nomifun-macos-native-final-audit.4afWjE/`；AX文件为实际CUA返回的相关摘录，截图在会话内供视觉核查 |
| current媒体独立App首轮与第二轮 | 两次FAIL：delegate仍没有产生request；第二轮CUA激活又被Mac锁定拒绝 | current example重建+新audit包/签名前后供给校验PASS；实际main_bundle=true/auditID/usage2=true，secure/APItrue/设备1+1/promise pending；没有完成可见且聚焦的对照，不能确认根因，也不能标MAC03原生完成 |
| trusted可见/聚焦媒体实物对照 | 真实CUA Start及HTTP trusted/visible/hasFocus均true，8s仍FAIL；TCC请求实际已到native | 同轮PID25031 Camera非preflight进入AUTHREQ_PROMPTING，responsible/subject=Codex、accessing/requesting=audit，至04:34:30无result/reply；不能再归因这轮未聚焦；未Grant、未读取TCC数据库 |
| LS启动归属对照 | PASS：同已验证binary/signature仅open新实例；PID25529/PPID1，mic主进程/GPU/coreaudiod preflight subject均auditID | 尚未Start/无Camera请求。open-W exit0仅launcher等待结果，fixture因无人Start前提timeout FAIL，两种范围独立记录；不是media lifecycle PASS |
| public native媒体前提 | camera/mic均AVAuthorizationStatusNotDetermined/0；诊断example编译及新包签名/供给PASS | own NSWindow bounded tree与publicUIDelegate selector/AV状态读取，无requestAccess/Grant/privateWK；beforeStart window inactive/key=false且无visible matching view，仍需真正激活后验证动态selector |
| 媒体前提诊断增强 | example编译PASS（2.87s）；新独立package strict签名/供给PASS；缺--media-permissions时明确exit2 | 仅fixture增加safe visibility/focus与可选trusted Start；每case60s gate+8s delegate、总420s/watchdog450s有界，不改变production、私有WK设置、默认ordinary smoke；完整native运行仍因Mac锁定未验 |
| MAC34原生nonchild observer首轮 | 5 PASS/1 FAIL，未当作全通过 | FAIL在夹具APP_PID输出解析，libtest同一行前缀使find_map为空；尚未进入该项observer注册/退出验证。生产身份/退出条件不放宽，后续修复结果另记 |
| MAC34 controller首轮编译 | FAIL，exit101 | 新macOS模块9处libc依赖未声明，尚未进入真实LaunchServices；后续显式target依赖与编译另记 |
| MAC34 nonchild observer第二轮 | 5 PASS/1 FAIL，真实nonchild exit7通过 | 信号夹具selfkill后立即exit1竞争，实际收到code1而非signal15；保留失败，修夹具保证信号路径可观测，不改变production wait-status解码 |
| MAC34 nonchild observer修夹具后 | 6 PASS；raw wait-status lib另1 PASS | 实际launcher退出后App PPID非controller，正常exit7与sticky终态、signal15、exec后即使replacement exit0也保持Unknown、错误callback/exe拒绝、Drop不杀均过。两次5/1失败仍保留，未放宽production条件；process/browser/Agent Session boundaries再通过 |
| MAC34 controller修依赖后 | 2 PASS，编译2m26s | missing-cleanup与absolute partial-frame deadline实际测试过；公开NSWorkspace binding编译通过。不能代替App启动验收 |
| MAC34 current Dev App首次LS验收 | FAIL，开发命令exit1；编译2m43s PASS | 真实App PID28956/PPID1与controller28955创建，但30s握手超时，数据初始化未进入。私密stderr仅记admission failed；cwd/DATA_DIR校验已先于重定向完成。两进程已退出，无force。completion与pre-start ACK时序待进一步诊断，未将怀疑根因当结论 |
| MAC34 categorical时序诊断 | FAIL，编译4.18s PASS | 真实launch_submitted→peer_connected→hello_received，30s时callback_received=false/peer_connected=true/hello_received=true；精确阶段证据已补，未输出env/nonce。将核查两阶段admission，保留preworker knote与最终callback/actualexit/cleanup证明 |
| MAC34 Node protocol/旧watch接线 | 首轮新protocol夹具4 FAIL（stat(file/..)为ENOTDIR），修fixture后组合27 PASS | private 0700/0600、argv不带nonce/env、helperexit0不等App成功、symlinkcwd、null-envelope与超量stdout丢弃、queue cleanup；不是实际LS证明 |
| MAC34独立Node复核 | fullrunner首轮20 PASS/1 FAIL；新增controller名字反例0 PASS/1 FAIL | Cargo tinyproducer回报缺失首次待诊断，不反复全跑掩盖；ownedcache内unrelated_macos_application_controller被接受，需exactbasename。watch5项原有反例保留并增强，其scope与FIFO/stdout/controller-exit事实独立记 |
| MAC34 Node最终定向收口 | watch5 PASS/39 assertions；protocol6 PASS/15 assertions | exactbasename修后wrongname被拒绝；scope/controller透传、FIFO、EOF及controller裸exit0不构成App证明、sticky/no-force保留；null与超量discard/privatecwd均过。Cargo tinyreport首次仍单独待诊断 |
| 原生Computer遮挡窗口 | PASS，生产focus入口exit0+CUA AX/视觉证据 | 用户已明确手动解锁；独立Swift fixture PID30568两个同frame窗口，occluder起始在前；真实nomi-computer focus_window定位window10965 target并AX激活。CUA随后焦点和截图均为target，未中心点击/键入/调用Computer屏幕采集或新增TCC授权；实际probe为现有shell责任身份，不代替formal Dev权限、所有多屏/密码/TCC负例。两fixture窗口随后CUA关闭，App自身180s正常terminate已确认PID消失 |
| MAC34两阶段current Dev启动 | 启动PASS：编译42.77s、actual UI已加载、health200；退出FIRSTFAIL另记 | 原knote注册→ACK→callback_received→completed_launch_identity_confirmed，AppPID30696/PPID1。实际主进程/GPU/coreaudiod mic preflight subject=Dev，Accessibility/ScreenCapture也为Dev，未Camera请求/Grant。Dev UI仍Computer未就绪，独立shell probe AX通过不能替代formal Dev权限 |
| MAC34两阶段正常stop首次 | FAIL，wrapper exit1，未当完成 | 对准确自有supervisor30379发SIGINT，经既有lifetime q，App记录request verified shutdown/terminal cleanup0；controller仍报cleanup unconfirmed。相关PID已退出但不能替代双证明，追加安全receipt/kernel诊断，不放宽成功条件 |
| MAC35管道与artifact collector | 3 PASS/7 assertions；原POSIX/missing Cargo相关2 PASS/3 assertions | 新真实writer后代在parent退出后持stdout，显式释放后完整中文JSON才settled；temporary仅revert close→exit的前态反例0 PASS/1 FAIL。split-UTF8/multiple artifacts/EOF tail/streamerror/exit101/ENOENT均保留；原20/1首次另存、独立新producer180/180不能补造其原stdout |
| MAC34 actual Native Command-Q诊断 | FAIL，编译6.41s PASS；真实App退出0、cleanup=true、receipt sent都确认 | App安全日志preparing/sent code0/cleanuptrue；controller actual kernel codeSome0/signalNone，terminal receipt读取报InvalidInput，commandexit1。排除这轮Tauri bypass/实际清理非0，不将收到前的sent冒充controller已读到 |
| MAC34 Darwin queued-receipt最小反例 | FAIL机制已实际复现 | real socketpair与accepted AF_UNIX，writer写31bytes换行后关闭；NONBLOCK receiver修改SO_RCVTIMEO均errno22，随后普通recv仍取到全部31bytes。与actual Native terminal读回执失败吻合；将区分blocking ACK与nonblocking controller读，不忽略EINVAL、不降低双证明 |
| MAC34同一read_frame前后 | 生产前态0 PASS/1 FAIL（errno22）；修后module6 PASS，1.47s编译 | real queued-receipt peer-close反例、private0700/0600/symlink/FIFO/oversize、one-shot receipt及failure不可重试、eligibility/lifetime一致、absolute慢帧限时均过。controller所有读取明确Nonblocking，App ACK维持Blocking时限，未吞EINVAL |
| MAC34 CLI stop lease首次反例 | 0 PASS/1 FAIL，2 assertions | 真实IPC测试在q已收到后，旧supervisor立即destroy使runner提前settled，actualApp-exit fixture尚未交付；实施独立stop-request+等待双证明后交付exit，保持真正runner死亡的EOF清理与未知态关闭准入 |
| MAC34 CLI stop lease修后 | watch targeted10 PASS/70 assertions | active/launching保留control等待App双证明与lifetime完成；未launch lease可destroy，真实allowHalfOpen peer不发FIN也能stop。queued/beforelaunch/not-acquired/timeoutsticky/EOF/noForce覆盖；测试夹具connect订阅及重复rm两FAIL、误等待halfopen close两timeout独立保留，不更改production成功条件 |
| current media最终包 | 当前smoke/controller combinedbuild PASS（21.79s）；新package签名/供给PASS，仅包装 | fresh `/tmp/nomifun-macos-launchservices-final-audit.i2A4Nj/media-final/`，实际auditID与用途说明确认，source d8bb62…16788 / signed fb7d4a…12df1；尚未启动/授予系统相机或网页媒体权限 |
| MAC34修后actual CLI stop | PASS，整条开发命令exit0；编译27.75s PASS | current App33179/PPID1、controller33178/独立PGID，normalSIGINT仅自有supervisor32844；controller exactkernel codeSome0/signalNone→terminal receipt code0/cleanuptrue→final_verified=true。lease未提前拒绝，所有自有PID消失；无force。Mac再次锁定使修后Command-Q待验，已异步请求手动解锁 |
| MAC34 current实际watch重建 | PASS，watch命令最终exit0；重编4.40s PASS | 仅touch自有Rust文件mtime，前后SHA均a10ccae8…bec6ab，未改变源码bytes。旧App33733/controller33728在watcher丢runner lease后先exactkernel0+cleanupreceipttrue+final_verified，再启动新App34108/controller34107；最终normal CLI stop也双证明PASS，旧/新PID均清除、无force。完整console在`/tmp/nomifun-macos-launchservices-final-audit.i2A4Nj/watch.console.log` |
| 最新controller Intel裸crosscheck | FAIL（环境），未当平台源码失败 | 未接项目Intel构建环境时ort-sys缺x86预建包；需共用prepareMacosBuildEnvironment提供已经批准的ONNX动态库/rpath，不绕过供应或改依赖。后续正确环境结果另记 |
| 最新controller Intel正确环境crosscheck | PASS，2m26s | 共用项目Intel环境，ONNX1.23.2已批准动态库及实际rpath，x86_64-apple-darwin新NSWorkspace/peer/knote/FrameMode/receipt源均通过；不是Intel硬件执行或MAC34后完整正式发行包。最终process/browser/Agent Session/desktop880边界均再通过 |

最终actual Command-Q：current Dev编译25.16s，GUI实际加载，CUA真实super+q后App51325/controller51322均退出；App code0/cleanuptrue/sent、controller receipt_received code0/cleanuptrue、exactkernel codeSome0/signalNone、final_verified=true，整条开发命令exit0。首次Command-Q FAIL保留，修后结果不覆盖；console为`command-q-final.console.log`，私密App日志为`/tmp/nomifun-app-launch-c493sK/`。

补充媒体前提验证：完整Cargo定向4 PASS，编译3.84s；current example build PASS22.11s。
覆盖OS等待超过8s不启动网站deadline、明确拒绝、五case共享120s、预算0时Authorized仍可进入、真实request优先。
仅fixture改动，公开只读无requestAccess/Grant/采集，旧8sFAIL未改为PASS。
新目录`media-system-prerequisite/NomiFun WK Media Audit.app` strict签名/供给PASS，仅包装；source6c9119de…11975c与签名前供给一致、signed18f994a4…b233ac与最终copy一致。
该包包装完成时未launch/未Camera Grant；当轮开始及包完成后CUA inventory均明确Maclocked。后续UI状态恢复时继续actual运行，下述结果单独保留。
current媒体attempt1：经共享NSWorkspace控制器启动App51608/PPID1，实际CUA Start+HTTP trusted/visible/focus均true；own WK UIDelegate实际存在且响应精确公开media selector。Camera51608.6非preflight请求的AUTHREQ_SUBJECT与PROMPTING Sub/Resp均为`com.nomifun.desktop.audit.media`，已排除当前归属Codex问题。
用户相机Grant确认已在具体操作时提出并保持pending，未授予相机/麦克风访问。系统仍public NotDetermined0，累计120s前提预算耗尽，五项网站delegate生命周期未进入PASS；App cleanup_verified=true/actualkernel exit1、controller据非0拒绝成功，全部自有PID已退出。记录为系统前提未满足，不冒称production缺陷或通过。证据`media-system-prerequisite/launch-attempt-1.console.log`及`/tmp/nomifun-app-launch-WKDQL6/`，原始日志不覆盖。

本轮中途原生UI恢复可读后完成Command验收，随后Mac再次锁定。Dev App/Vite经supervisor正常关闭，fixture通过自己的`/shutdown`正常退出；没有为解锁而调用其它系统自动化。
待执行的current媒体包位于`/tmp/nomifun-macos-launchservices-final-audit.i2A4Nj/media-system-prerequisite/`；`media-final`是系统前提分离之前的包，`4afWjE/media-user-ready`是MAC-34之前的旧轮次与失败证据，未覆盖，不作为当前启动入口。

Agent Session原生验收准备阶段：既有`conversation_gui_fixture`在全新`/tmp/nomifun-macos-native-session-audit.UmNoGI/desktop-data`中通过生产DesktopServer API创建本地协议模型、Agent revision及Session，然后正常关闭准备阶段后端，保留loopback模型服务。fixture编译3m05s PASS；current Dev App编译1.10s PASS，公开LS两阶段握手与独立后端启动通过，AppPID52502。
Session为`01a13254-7872-7e22-801f-a217800be90c`；只读SQLite基线确认Turn为0、唯一`reasoning_effort`为null，typed workspace绑定为managed-session。直接GET未带local-trust时403，未绕过认证或改写Session。实际CUA首次选取这个正在运行的App时报告Mac locked；已请求手动解锁并保持解锁，当时尚未进入真实UI、High设置、Turn执行或冷启动恢复验收，库存保持`implemented_unverified`。准备不计为GUI PASS；本地mock模型也不代表真实服务商。

后续UI恢复后完成当前Session验收：真实CUA打开上述确切Session并选择“高”，唯一`agent_sessions.reasoning_effort`持久值为`high`。实际输入请求启动生产Turn（operation suffix `01a1325b-b7c6-7242-a0d9-3b834b00eee6`）；UI按顺序显示说明及`write_file`，展开回执为written/created true、73bytes，真实managed Workspace的`journal-check.md`SHA为`32a55227226aa5e4f46ba2eb4662e8cda2be2e06956405c06848af5c76e40af2`，与回执一致。loopback模型`/finish`仅释放夹具响应；生产Store持久Turn completed与确切terminal_event_id。模型固定回复只写文件，没有按请求文字额外读文件，不将其记录为read工具通过。
真实Command-Q后整条命令exit0，App cleanup true、原kernel exit0、controller final_verified true。随后同数据根冷启动真实App，CUA同Session恢复“高”、用户请求/说明/最终正文、产物卡及可展开的相同工具输入/回执；第二次真实Command-Q也有相同双证明。当前宿主与正式Tauri开发路径已验；未验证Intel/macOS14、外部模型或reasoning请求字段wire。
补充Channel检查：`cargo test --locked -p nomifun-app --lib router::nomi_core_wave4::tests::channel_send_uses_the_live_plugin_once_for_an_idempotent_replay -- --exact`为1 PASS（编译3m50s、执行0.54s），使用真实SQLite repository/ChannelManager与RecordingPlugin、受控ChannelSessionPort，覆盖exact Session ingress、一次幂等发送、未知结果不重发、owner/domain及解绑边界。`router::nomi_core_session::agent_switch_resource_tests::companion_refresh_only_retains_channels_owned_by_the_same_companion -- --exact`另1 PASS（编译0.93s），覆盖同owner/Companion保留及拒绝混入客服身份；不是外部IM真实发送。
四分面证据由独立renderer agent按inventory原文及现行架构复核后，`macos.current_agent_session`改为verified；没有降低required_evidence或以boundary脚本代替实机。证据为`canonical-before.json`、`canonical-active.json`、`canonical-terminal.json`、`canonical-after-cold-restart.json`及上述目录中的两个native console/两个channel log；实际CUA AX与截图保留在本会话。两次App均已退出，fixture经自身`/shutdown`正常exit0。
更新后`bun run check:uarc-boundary` PASS：2853个production文件、全部retired matcher零引用、baseline anomalies open=0、macOS gaps pending=0；`git diff --check` PASS。这个库存数字仅对应其架构任务，不覆盖总TODO仍列明的媒体系统授权、formal Computer权限、硬件、服务商及发行验收空白。
退出后的额外Python只读复查曾两次报`unable to open database file`，数据库stat/header均正常；首因未确认，不推断为生产冷加载故障。确认App和写入进程已结束后以`mode=ro&immutable=1`读取静态DB成功，得到high/同completed Turn/73 events；该参数仅用于这次quiescent取证，不进入生产实现或活跃数据库读取。

MAC-14真实UI补验：同一隔离数据的current Dev正常LS启动，CUA「设置→能力与权限→通知」显示「已授权」。当前恢复用「打开系统设置」按既有ready规则隐藏；未撤销已授权状态、点击请求授权、切换通知开关或修改系统设置。故这次只证明当前UI状态及导航，notification deep-link实际GUI落点仍未验。App随后真实Command-Q双证明通过，整条命令exit0；日志`native-notification-settings.console.log`，私密App日志`/tmp/nomifun-app-launch-Ko5air/`。

MAC-05真实裸名称启动补验：新增仅用于独立自有App的`macos_launch_audit` example，接受唯一`NomiFun Launch Audit `前缀及1–64ASCII字母/数字/hyphen后缀；普通App名被拒绝，不查询或请求AX/Screen/media。公开`nomi_computer::launch::launch(name,None)`编译PASS3.37s。Git外tiny App只写本轮独立marker并创建标明用途的无敏感内容窗口，自带180s结束计时；不读取用户内容或申请权限。
首轮该App位于`/tmp`，`lsregister -f`和只读registry均确认其记录，但生产裸名称入口返回「Unable to find application named…」/exit1，marker不存在。该失败保留，不归为生产成功或已确认源码缺陷。随后仅移动同一自有App到已存在的`~/Applications`并注册，名称、binary和生产入口均不变，handoff PASS/exit0；App自身`didFinishLaunching` marker确认`com.nomifun.desktop.audit.launch.20261013-12e61f25`、确切installed bundle路径、PID55036/PPID1。位置与名称解析行为有实际对照，但未进一步证明LaunchServices内部拒绝临时路径的原因。
CUA选取这个已确认live的App进行窗口核对时再次报告Mac locked，因此实际App启动与窗口视觉分开记录；handoff JSON本身明确`actual_application_start_verified=false`，成功启动由独立marker另证，不偷换两者。证据在`/tmp/nomifun-macos-launch-name-audit.5k5lh1tb/`的`handoff-first.json`、`handoff-installed.json`、`launch-marker.json`和metadata/build日志；普通`Calculator`名称反例exit1发生在生产调用前，没有启动该普通App。该函数级验收不代表已授权formal Tauri Computer Agent Role。
确认live后按确切program只读轮询，fixture自身计时结束后PID55036消失，没有强制结束；这次未收集kernel exit status，不能冒称其exit0。随后只注销本轮installed App并将其移回Git外原目录保留，既有`~/Applications`目录和其它App未改。`cleanup-report.json`记录位置及binary SHA；测试不留常驻App或安装副本。

current媒体attempt2：用户明确答复仅允许`com.nomifun.desktop.audit.media`相机访问，并手动解锁、保持解锁。已重验同一冻结App的strict签名与binary SHA18f994a4…b233ac，使用新的`attempt-2-data`及`launch-attempt-2.console.log`，不覆盖attempt1。公开NSWorkspace控制器启动App55445，首case before_start的公开Camera状态已为Authorized3、Mic为NotDetermined0；root没有看到或点击新的OS授权提示，不能冒称此次执行了OSGrant。系统NotDetermined累计等待为0ms。
CUA依次真实点击`explicit_deny`、`input_owner`、`hide`、`navigation`、`close`五个Start，独立HTTP trusted/visible/hasFocus gate均成立。每个实际Camera pending都由精确origin/doc/kind typed owner接管；explicit旧doc回复严格拒绝且completion只消费一次，Deny/owner/hide有网页拒绝见证，navigation排空且新doc无旧denial，Close实际成功。当前公开UIDelegate类及selector、请求真实到达均有每case记录。独立browser agent按生产/fixture断言复核了报告，足够关闭原生网站Camera Deny/退役分面。
最终`MACOS_BROWSER_COMPATIBILITY_SMOKE_PASS`、core_checks_passed、native/disposable cleanup均true；controller exact kernel codeSome0/signalNone、terminal receipt code0/cleanuptrue、final_verifiedtrue，Node命令exit0。完整证据为`attempt-2-report.json`、上述console及`/tmp/nomifun-app-launch-zdervu/`；App55445和controller均已结束。
报告仍明确website Grant/system Grant过程`not_covered`、actual_sensor_capture=false，只有video:true；Mic、自动后台触发、正式renderer permission按钮及canonical Agent Turn接管链未由这个独立fixture覆盖。下载picker由fixture自身实际取消、未选目的地、未使用用户Downloads，没有向个人Documents写入；root看到panel后尝试点击Cancel时App已结束，工具返回inactive，不将这个点击记录为GUI成功。首轮系统预算失败不被改写。
formal Dev Computer权限准备：在同一隔离数据根正常LS启动current Dev后，CUA从首页「去配置」到`settings/permissions?tab=computer-use`，真实显示辅助功能及屏幕录制待授权。当前已在两个请求按钮处向用户一并询问仅`com.nomifun.desktop.dev`的这两项访问，尚未授权；audit Camera确认不扩大为Dev的AX/Screen/Mic授权。正式设备Action验收保持未完成。
用户随后明确允许NomiFun Dev的辅助功能和屏幕录制；此确认不包括其它App或Mic。root在实际页面点击辅助功能请求，System Settings中已有NomiFun Dev开关on，但当前Dev重新检测仍待授权，不把既有开关作为effective权限PASS。为避免反复绑定中间binary，当前App正常Command-Q双证明/整条命令exit0已关闭；待新守卫修复与最终App定版后按现有公开UI恢复流程绑定确切App。未操作其它App权限、移除条目、开启Screen开关或执行真实Computer输入。
MAC36首次命令漏`computer-use` feature，编译9.77s但实际0 tests，不记PASS；补全`--features browser-use,computer-use`后同一精确反例编译3m12s、实际0 PASS/1 FAIL/exit101，旧token replay=Ok、Fake executor3→4。两个日志独立保存在`mac36/first-counterexample.log`及`first-counterexample-with-features.log`。
MAC38首反例`cargo test --locked -p nomi-computer --lib element_gesture_ -- --nocapture`为1 PASS/4 FAIL/exit101：frontmost/window/bounds/dirty四用例，right与double都resolver_calls=0且native recorder被调用；buttons/counts/AX坐标正控PASS。只为测试引入leaf gesture port与默认Unsupported非mutating resolver接口，旧production路径尚未使用resolver；全部Fake不创建AX/Enigo、不读屏幕或执行鼠标输入。证据`mac38/first-counterexamples.log`，生产repair另记。
MAC36/37首次Computer组（完整features）编译2m14s，实际6 PASS/5 FAIL/exit101。overflow返回MAX同时Atomic变0，下一次返回1；跨Session ref旧A文本实际到Fake fieldB/nativegen3，旧A坐标实际消费B几何成为[1521,-259]；failed观察后旧token还允许identity[10,20]输入。全部仅记录型leaf port，无真实设备。证据`mac36/first-shared-authority-counterexamples.log`，既有守卫正控6 PASS不覆盖新5 FAIL。
MAC39精确factory重建反例为0 PASS/1 FAIL/exit101：保留旧envelope、drop旧Invoker/所有Host/port后，同canonical Store/Session新factory又mint1，历史样值replay实际进入Fake fieldB/nativegen2，随后current token也失效。限制：旧envelope没有通过closed Turn persistence/hydration，该test证明组件重建别名，不冒称真实native事故；独立只读追踪确认真实closed history保留token文本，修后仍需要canonical因果来源集成。证据`mac36/first-cold-factory-counterexample.log`。
MAC38 ordinary fallback首反例为2 PASS/5 FAIL/exit101：frontmost/window/bounds/overlay四漂移中，click_element与set_element_value都不调用resolver而向Recorder补发pointer/type；BackendAfterEffect另验证semantic_effect已经记录1仍补发。stable click/type及既有Uncertain不重发两正控PASS。只将leaf输入转发默认native port以便Fake recorder接收，旧fallback决策保持到首FAIL结束，没有真实点击或键入。证据`mac38/first-element-fallback-counterexamples.log`。scope扩至所有AX-derived pixel fallback，修后另验，不以right/double分支修复覆盖普通回退。

MAC38 pointer→type首反例为1 PASS/4 FAIL/exit101：Fake pointer Future完成时切换frontmost/window/overlay/另一field，实际set_element_value均pointer=1、type=1、focus_checks=0。稳定受控focus正控通过。正确边界是已发生一次pointer后拒绝type并返回Uncertain，不能标成零效果Stale。证据`mac38/first-pointer-to-type-counterexamples.log`；生产第二段焦点证明尚在实施。

MAC38整值替换首反例`element_value_fallback_replaces_nonempty_value_or_rejects_before_pointer`为0 PASS/1 FAIL/exit101，编译1.91s。实际set_element_value在原生Unsupported后pointer1/type1，Fake原值`existing-field-value`和末尾caret得到`existing-field-valuereplacement-value`，却返回“Set element…”。证据`mac38/first-value-replacement-counterexample.log`。它与readonly/未知值/空白被display normalization折叠的源码风险共同证明，仅补焦点身份不足以保证whole-value contract。选择保留原生setter，退役自动keyboard fallback及不再有生产consumer的focus-proof接线；不会以通用Cmd/Ctrl+A猜测全文selection，也不只改成功文案。

MAC38整数anchor首反例`cargo test --locked -p nomi-a11y --lib pixel_center_ -- --nocapture`为1 PASS/2 FAIL/exit101，编译0.68s。`[-1,0)`的center -0.5被cast为0，subpixel `Rect(0.2,0.2,0.2,0.2)`没有整数点却得到(0,0)；既有负原点/无效geometry正控通过。证据`mac38/first-integer-anchor-counterexamples.log`。修复必须选half-open bounds内且finite/i32可表示的整数point，没有合法整数point则拒绝，native hit/proof与实际pointer统一使用同helper。

MAC38零效果分类首反例`uncertain_semantic_effect_hint_cannot_forge_stale_zero_effect_rejection`为0 PASS/1 FAIL/exit101，编译1.54s。实际Tool的Fake engine先记录semantic_effect1，再返回Uncertain，hint包含“failed: stale reference:”和“No pixel fallback was performed;”；旧contains classifier将整个实际失败结果误判为Stale零效果。没有native pointer或真实设备调用。证据`mac38/first-uncertain-hint-classification-counterexample.log`。新分类必须核验数值ref之后立即出现的typed category及固定tail，而不是读取嵌套hint中的短语；已发生效果继续Unknown，不因无pixel fallback而变零效果。

MAC36/37/39的真实canonical Store因果检查：`cargo test --locked -p nomifun-agent-session --lib committed_native_tool_causation`为11 PASS，编译29.55s、执行1.62s。覆盖首次fence0、精确per-call identity、typed header/claim、active lease、暂停/恢复及后继Turn/冷重开旧epoch、已完成tool/result与host_tool_settled提前结算拒绝。基础Effect API兼容和不改变cursor亦有正控。证据`mac36/fixed-canonical-causation.log`；不证明native设备或TCC。

Host修后首次检查`cargo test --locked -p nomifun-app --lib --features browser-use,computer-use router::agent_role_host::tests`编译FAIL/exit101，未执行测试：新fixture的`format!(...).into()`传给`impl Into<OperationId>`引发E0283。证据`mac36/fixed-role-host.log`。已仅删fixture参数多余into，生产guard和测试期望未改，重跑结果另记。

Host修夹具类型后重跑，编译2m12s成功，实际17 PASS/4 FAIL/exit101，执行6.47s；四个新realStore tests均在fixture Session创建处被`agent_session_id must be UUIDv7`拒绝，尚未进入预期的causation/Effect/冷恢复检查。证据`mac36/fixed-role-host-rerun.log`。正在仅修fixture ID生成与对应context/source引用，不弱化Store准入或test expectations，后续结果另记。

Host仅修cfgtest的真实UUIDv7身份后，最终21 PASS/0 FAIL，编译18.16s、执行7.29s，证据`mac36/fixed-role-host-uuidv7-rerun.log`。包含上述四项真实Store场景、same-value token跨Session拒绝、B合法authority保留、精确未结算call、old closedTurn保存后factory重建及Unsupported零效果failed/Backend-after-effect unknown。所有device ports明确Fake，无原生输入。生产UUID校验、授权、lease/Effect guard及期望均未弱化。

MAC38最终库检查：`nomi-a11y --lib`20 PASS/0 FAIL（编译0.57s）；`nomi-computer --lib`111 PASS/0 FAIL/7 ignored（编译1.44s、执行3.53s）。root先核对7项真实截图/窗口/鼠标测试默认ignored，没有执行个人屏幕或真实输入。结果覆盖whole-value拒绝/支持原生setter正控、Backend/Uncertain不补发、嵌套hint不分类为零效果、integerpoint及guarded pointer；首次六组FAIL不覆盖。日志`mac38/fixed-a11y-lib.log`及`fixed-computer-lib.log`。最终`nomi-a11y x86_64-pc-windows-msvc --tests`再次交叉编译PASS（0.73s，`fixed-final-windows-check.log`），包含最终退役接线，不代表Windows test执行或GUI。

最终Intel交叉检查`cargo check --locked -p nomi-a11y -p nomi-computer --target x86_64-apple-darwin --tests`PASS（5.13s，`mac38/fixed-final-intel-check.log`），验证最终shared helper、macOS actor及Computer source/tests可编译；不是Intel实机运行或最终发行包验收。

canonical回归`cargo test --locked -p nomifun-agent-session --lib`为159 PASS/0 FAIL，执行24.43s，覆盖history/projection/context、native lease/checkpoint/recovery、Effect精确因果和未知效果、dataset relocation、schema/reset范围。证据`mac36/fixed-canonical-session-lib.log`。`check:agent-session-boundary`及`check:uarc-boundary`同时通过；库存pending=0只表示该库存已列项的证据满足，不能覆盖本TODO仍在施工的MAC36–39或formal Computer native验收。

受影响reset集成`cargo test --locked -p nomifun-db --test agent_store_reset`为2 PASS/0 FAIL，编译20.19s、执行0.91s，验证主库clean baseline及同代Agent reset保留非Agent users。证据`mac36/fixed-agent-store-reset.log`；本轮没有新增schema或持久观察账本。

收尾静态检查`check:desktop-ui-boundary`（2091 renderer sources、880×600）、`check:process-runtime-boundary`、`check:browser-platform-boundary`及`git diff --check`均通过。新的焦点证明实现仍在施工，不能以这些静态检查替代其针对性或原生验证。

MAC38 Windows focused-candidate窄修后，`cargo check --locked -p nomi-a11y --target x86_64-pc-windows-msvc --tests`交叉检查PASS（0.65s），证据`mac38/fixed-focused-windows-check.log`。这是包括Windows-only tests的编译检查，没有执行这些tests或Windows GUI动作。独立只读复核已核查Windows/Linux编辑候选语义、整数point覆盖及父链；macOS/tool及整值setter回退仍在收尾，不以其它平台编译替代本机行为证据。

用户授权后的公开System Settings只读复查：NomiFun Dev在辅助功能和录屏与系统录音两个列表均显示on；没有切换、删除/重加条目或改动其它App。与此前current Dev实际pending不同，这个UI值仍不足以证明当前binary的effective权限。最终App定版后继续实际recheck及必要的精确Dev绑定恢复。

正式Computer最小验收的独立启动目标已准备，尚未启动：`/tmp/nomifun-macos-canonical-computer-audit.pvq6dj7r/metadata.json`记录唯一name/identifier/marker和installed bundle；源为自有Swift NSWindow，仅写本轮private marker、180s自结束、不请求设备权限或读取用户内容。strict签名及公开LaunchServices注册通过；本轮结束须正常退出、注销并移回private原目录保留，既有其它App不动。

该正式Computer fixture首次构建PASS（2m40s，`fixture-build-first.log`）；首次准备运行exit1/FAIL（`fixture-first.console.log`），错误为“error decoding response body / expected value at line 1 column 1”。未出现READY，尚未正式Dev、模型请求或owned App启动；首次`data`和日志保留。当前diagnostic未指出HTTP路径/方法/status，首因未确认，正在仅修新fixture阶段诊断与公开API对照，不修改生产API/auth/Store或将失败当native通过。后续使用新数据目录另记。

fixture根因修后：四POST路径/method都由生产router确认，首privateDB只读查到provider/preset/revision1已创建、Session/Event0；新fixture误写退役字段`skills`，实际AgentPresetDocumentDto字段为`skill_bindings`。仅修example并typed preflight，添加stage/status/type/有界响应字节数分类，日志不输出body/trust/env/credentials。首次HTTPstatus未采集，不冒称实测422。修后build3.78s PASS，新的`data-attempt2`四stage依次201/200/200/200，READY/prepare backend stopped=true；Session为`01a13342-3f93-7e31-b514-369e775a60fe`，mock control为127.0.0.1:53755，`fixture-attempt2.console.log`独立保留。

最终Dev的该数据根首次build2m45s PASS，公开LS完整两阶段identity confirmed，App81545/PPID1、controller81544、冻结包`target/macos-dev-app/38f412e54378bd28/generation-0p13Id/NomiFun Dev.app`，日志`native-final-first.console.log`及`/tmp/nomifun-app-launch-9Z3yjL/`。实际Computer页仍AX/Screen待授权。用户再次完成必要系统账户认证；root只移除并重加这两项NomiFun Dev旧条目，公开FilePicker选取上述精确包，两个system开关on，未操作其它App/Mic/Camera或私有TCC数据。

Screen绑定后出现OS“须退出才生效”提示，root未点击“退出并重新打开”。后续App AX HTML消失，第一次CmdQ被native工具的userchanged guard阻止，刷新后第二次提示App quit；具体发起者未由日志确认。该Dev命令最终FAIL/exit1：controller exactkernel codeSome0/signalNone，receipt_received=false、terminal_receipt_read_failed UnexpectedEof。Appstderr只有admission分类，无preparing/sent/send_failed；这是与既有queued-receipt已sent首FAIL不同的证据。没有发送模型/输入或启动owned App，不以normal0补证cleanup，也未立即重启旧实例。

独立旧资源审计证明没有遗留受管资源：精确81545/81544均失活；该私有root的Hosted registry/apps、Robot及全部Agent Execution/Terminal/SSH/Channel/Cron/Remote/Knowledge任务表0，Turns/Effects/Payloads0，native recovery候选0。仅一个ready/live Session及3配置events/Computer binding，没有tool/process/nativeclaim/unknown/pending事实；API活动无任务或启动，仅GET/设置/permission request。既有resource容器初始化为空，外部进程需实际工具/render才创建，in-process listeners随精确main退出结束。server.lock.info/port.json是breadcrumb不作活锁证明，WAL/SHM不删除。oldreceipt仍FAIL；只有在这次独立审计后允许新私有目录的独立counter，不复用/解除旧failed lease或追认成功。

独立异步生命周期复核无新增缺陷：`join_input_task` deadline后仍等待exact worker；macOS main-queue未开始closure会被取消，已开始则等待回执；mutating AX actor使用recv等待，而100ms recv_timeout只是actor空闲取命令轮询。正式Engine保留实际工具owner task直至invoke/settlement，caller取消不丢弃其future，shared physical lock不提前释放。Computer变更的existing exclusive Effect resource key不含Session/Turn/binding ID，DB对同owner/local-desktop的pending/unknown保留跨Session唯一围栏；Unknown仍可只读观察，不能新增物理变更。该源码证据限同一正式App/Store/owner，不冒称跨所有App/数据库的OS锁或远端AX原子事务，未执行native动作或新增账本。

MAC40独立原生旧源码反例：全新private scope `/tmp/nomifun-macos-appkit-exit-audit.8MdF3lXi/old-counter-data` 的Dev build24.99s PASS，公开LS两阶段confirmed，App86850/PPID1、controller86848，后端55027。冻结包为`target/macos-dev-app/43d576c025ffcdde/generation-cnNW9o/NomiFun Dev.app`。只读prepare记录exact PID/start key/UID/程序与bundle；请求时再次验证同一generation与公开NSRunningApplication身份，公开`terminate()`返回accepted=true。随后controller确切kernel codeSome0/signalNone、receipt_received=false、UnexpectedEof，整条native command exit1；无强制终止。stderr仍仅admission，无receipt preparing/sent/send_failed。证据为该scope的`native-appkit-first.console.log`、`old-public-terminate-request.console.log`及`/tmp/nomifun-app-launch-g3PSnM/`。probe JSON明示request成功不等于kernel/cleanup通过。CUA当时锁屏，因此不把它计为窗口视觉、真实OS logout或权限UI验收；公开native生命周期反例与该分面分开。

独立probe首次build因Retained NSURL借用类型不匹配产生两处E0308，未执行反例；日志保存在`/tmp/nomifun-macos-canonical-computer-audit.pvq6dj7r/appkit-probe-build-first.log`。仅修局部借用后build1.47s PASS，`probe-build-fixed.log`另存于上述MAC40 scope；未弱化PID/start key/UID/签名bundle路径准入。上述首原生FAIL发生在修复前且由可归因的正常公开terminate请求触发，不能被后续idle或run_return正控覆盖。

MAC40第二旧scope独立资源审计：exact86850/86848失活，private根非symlink且work/data绑定一致；Hosted registry/apps、Robot为空，旧process registry不存在。所有Session/heads/bindings/events/Turns/Effects/Agent Execution和native recovery candidates均0，Terminal/SSH/Channel/Cron/Remote/Knowledge任务表全0。HTTP分类仅256 GET及3次client settings PUT，无POST或resource acquisition。启动listeners及后台循环属于已退出的确切main，无旧受管对象可移交恢复。保留breadcrumb、WAL/SHM和receipt FAIL；允许在新私有scope进行修后native case，不重开旧生命周期补证。用户再次确认解锁后，CUA实际读取现有System Settings权限页成功；尚未启动修后Dev或改变新的权限项。

MAC40施工版本独立源码复核发现并修正restart及FATAL边界：迟到callback在native token取消后不得补发normal0覆盖Tauri restart；fatal原始normal code即使0，也须复用`process_exit_code`保留最终1，不能绕过run_return将其变NativeYES。公开delegate同对象重告知保留原identity和Tao Dock行为，CF主队列helper抽取供真实modal组件验收复用。这些是未发布施工版本的源码审查结果，没有冒充原生首FAIL；one-shot receipt与pending panic/deadline边界仍待最终定向与原生检查。

MAC40修后首次定向编译FAIL/exit101，未执行测试：`fixed-appkit-tests-first.log`记录三处E0308，都是`std::ptr::eq::<NSApplication>`接受raw pointer时无法自动把`&Retained<NSApplication>`转为目标引用。仅修显式借用，保留same-object判定与所有准入守卫；后续结果另记。独立最终源码审查已确认panic前保token、公开reply后consume、迟到/restart/fatal与FFI诊断二次catch边界，源码审查不替代编译/原生通过。

MAC40修显式借用后目标检查实际10 PASS/0 FAIL（编译8.52s，`fixed-appkit-tests-borrow-rerun.log`），包括真实ExitCoordinator的forced/fatal/restart语义、token耗尽/重复/迟到/dispatch失败与准备panic取消责任。panic测试日志保留预期fixture panic，测试自身通过；没有执行native动作。真实Unix socket writer组9 PASS（执行0.05s，含新增3项并发单frame/不可升级/破损transport锁死）；exit_coordinator组4 PASS，process_exit组2 PASS，unverified_normal_exit另1 PASS。分别保留`fixed-receipt-tests.log`、`fixed-exit-coordinator-tests.log`、`fixed-process-exit-tests.log`、`fixed-unverified-exit-test.log`，未用0tests或源码review代替执行。

MAC40 current中间冻结版首次idle正式原生PASS：build26.33s，包`target/macos-dev-app/9bd8734c6cee4e1a/generation-dz4Cyx/NomiFun Dev.app`，App91580/后端55832；公开LS完整两阶段confirmed，CUA实际看到同App空会话首页。独立exact PID/start key/UID publicterminate accepted后，实际callback entered/token1/Later、真实terminal cleanup deleted0、单次authenticated receipt preparing/sent0true，以及NativeYES先后证据齐全。controller先观察到exactkernelSome0/signalNone，再读取已排队terminalreceipt0true，final_verifiedtrue，完整命令exit0。证据为`fixed-idle.console.log`、`fixed-idle-public-terminate-request.console.log`及`/tmp/nomifun-app-launch-i52HiS/`。这个PASS只属于MAC40 idle，没有活PTY/WK、OS logout或formal Computer权限证明；新增MAC41最终源码仍在实施，不能把中间版当最终发行验收。

公开CF组件first build22.91s PASS，binary SHA`dff047d67fc90f0f08b7a3f2a806be8ad7a0f3735ae579489d7fcd15c685d3fa`。main-loop原生first PASS/exit0：PID93148，default callback的CF current mode=kCFRunLoopDefaultMode，真实NSApplication.runModalForWindow中worker回执mode=NSModalPanelRunLoopMode，MainThread/确切自有modal/original callback active全部成立，completion只1次、deadline0，关闭自有modal并正常退出。报告`main-loop-first.json`及console均保留；没有DesktopServer/authenticated receipt/个人屏幕或TCC操作，不把组件PASS当完整Dev退出。

MAC42首次确定性原生counter FAIL/exit1：PID93214，原TauriTask回调不返回期间，正式DeferredEngine.get和同owner.shutdown各首次实际PollPending后握手，再进入真实native modal。5s无completion，公开CF deadline在相同NSModal mode及exact自有modal中成立并只调用1次，首FAIL保持。原TauriTask返回后，同一get/shutdown futures才实际settled，closed initialization拒绝、shutdown成功、actual_cleanup_proven=true，组件正常结束；没有丢future或强制终止。`deferred-engine-first.json`明确passed=false及settled_before_original_event_callback_return=false，counter console另存。组件没有Backend/DB/网络/权限，源码因果为Tauri callback guard阻止其排队init；不冒称原生正式App systemquit或所有Later都会卡死。窄修只改首次init派发同CFhelper，保closing/watch/owner；example只补同helper模块alias，不改原断言或首报告。

MAC42修后component与existing Browser smoke两example build6.09s PASS（`component-build-fixed.log`）；相同component断言原生all PASS/exit0，PID94031。`main-loop-deferred-fixed.json`证明正式get/shutdown仍firstPollPending，原TauriTask尚active且真实NSModal mode内即结算，closedinit拒绝/shutdown成功；exactowned modal/MainThread/回执1次/deadline0均成立，default与ordinarymodal正控也PASS。独立只读review核实sameowner、原Futures retained、gate/watch未伪造，panic/poison明确Err而非Ok(None)。此反例在首次init前closing，覆盖取消初始化与modal结算；不代closing=false真实Engine/WK创建与正式App退出，后者继续单列验收。

MAC41正式调用链独立源码确认：application.restart与mac updater postinstall都调用plugin-process2.3.1的request_restart；不可取消sentinel在main同步等backend，而已初始化WK关闭/monitor移除等main GCD。updater2.10.1 mac install不执行配置on_before_exit，不能以该hook当mac precleanup证据；Tauri2.11.2实际relaunch还从Exit中直接spawn bundle executable、exit0，不返回main末尾，需补Dev旧receipt/新LS admission。plugin exit本身另查确是app.exit普通协调器，不误记直接process::exit。现有FactoryReset/StorageRelocation UI会触发restart，不能用破坏配置制造验收；后续专用自有按钮只触发正式process restart。上述源码事实尚不是native首FAIL，不属于已决定不支持的WK上传/evaluate/CDP边界。

后续有资源正式验收的全新preparer build已PASS，`active-owned-data`正常READY，控制server127.0.0.1:56062，Session `01a133b4-6a5f-7dc2-90ce-f61f79a072ed`。只准备canonical隔离Session与localhost页、正常测试登录凭据，prepare后端已关闭，mock页面server继续；未启动正式Dev、PTY或WK runtime，准备不计活资源PASS。

MAC43首单位counter实际2 PASS/3 FAIL/exit101（编译1.26s，`action-metadata-first.log`）：生产decoder/invoke_semantic_press将SDK error-25204、成功却null和错误CFtype都转为Unsupported；native leaf Recorder保持0，没有真实AX或输入。validempty在native前Unsupported和合法AXPress单次派发两正控PASS。FalseUnsupported会让正式ordinaryclick进入位置守卫后的pixel fallback，metadata未知没有等于确认不支持。未把library AXShowMenu分支误记formalright缺陷：正式right/double当前直接do_element_gesture，独立复核已纠正调用因果。本轮只修正式可达元数据读取合同，不扩该unused分支或新增PID API。

MAC43最终库回归实际29 PASS/0 FAIL（编译1.23s，`action-metadata-fixed-lib.log`），含原20项及9动作元数据项；修后SDKerr/null/错误CFtype、array中的null/nonstring、越过完整4096预算均Backend，只有完整合法array才确认press/absence。新增预算正控和成功/错误CF retain平衡检查，所有preflight错误在native leaf前，派发后仍Uncertain。纯CF/Recorder无真实AX/权限/输入，正常native guardedclick/整值setter由独立owned窗口后续验收。

MAC41 JS首运行器误用Node而文件依赖bun:test，ESM拒bun:、没有真实cases执行（`restart-node-tests-first.log`），不归为生产故障。改正确Bun后首次实际72项/70 PASS/2 FAIL、288expects（7.47s，`restart-bun-tests-first.log`）：POSIX Cargo report测试缺nomifun-desktop报告，以及empty-array invalid restart flag的test.each回调5s超时。两个失败单独保留，正在准确诊断/最小修；strictbool和cleanup/kernel守卫不得因失败放宽，尚不能记JS全部通过。

MAC41 Rust全bin回归首次实际162 PASS/1 FAIL/3 ignored（执行1.37s，`restart-desktop-tests-first.log`），新生命周期/IPC/receipt/gate检查通过；旧profile relocation夹具在`/var`符号链接被正确拒绝。仅canonicalize后该单项仍FAIL（编译4.13s，`restart-relocation-fixture-fixed.log`），错误是matching finalized receipt；独立inspect证实缺regular `nomifun-backend.db`，未弱生产校验。按common relocation fixture补opaque数据库占位（这个test不验证SQL启动）后原断言实际1 PASS（编译3.68s、执行0.45s，`restart-relocation-fixture-dataset-fixed.log`）。合计163项已验证、3项声明的Chrome/公网ignored未执行，不以ignored代PASS。

MAC41 JS第二次71 PASS/1 FAIL仍保留，`[]` tuple只修Bun.each展开而不弱strictbool。私密准确fake producer诊断显示Node/Bun都收齐111字节，生产parser未改；同cwd/受限PATH的Bun相对文件参数复现BadFD/报告缺失，绝对与`./`路径对照通过，内层FD原因仍未证明。最终`bun test ./scripts/lib/macos-application-launch.test.mjs ./scripts/run-macos-dev-runner.test.mjs`实际72 PASS/0 FAIL、290expects（834ms，`restart-bun-tests-dot-prefix.log`），前失败不覆写；不同shim与路径对照未偷换为原首次PASS。

MAC41最终正式App首次build49.32s PASS，冻结包`target/macos-dev-app/42fc14e376eb0eee/generation-oA5H0u/NomiFun Dev.app`，旧App7545/后端59022，公开LS两阶段confirmed。normal JWT客户端browser GET实际403（`active-resources-first.json`），发生在任何mutation前；没有放宽auth或导出trustsecret。随后CUA确切Session `01a133b4-6a5f-7dc2-90ce-f61f79a072ed`打开正式Browser并加载唯一loopback页，真实native child AX包含验收heading/字段及profile HTTP witness。背景窗口时child不可见；Raise后实际AX可读，未把初次空白截图当真实页面通过。

自有PTY通过正常login后的正式POST201/GET创建，唯一terminal `01a1343c-2b64-7021-ab4c-784ab9e7061b`，exact leader8084/startkey1791877720941483、sleep8085/startkey1791877720944954、UID501/PGID8083。native only-read libproc helper固定birth before/after，未读env/参数或扫描其它PID。07:48:40 UTC创建、07:50:53 UTC真实cleanup deleted1，相隔132s，小于180s自然结束；两个exact generations随后gone，因此不把自然计时结束算清理。

CUA实际原生菜单“重新启动应用”触发同process service，旧App cleanup receipt0true+exactkernelSome0/noSignal+final_verified；同仍存活runner再创建`/tmp/nomifun-app-launch-ppQB4C/`新LS两阶段confirmed，新App8147/后端59236。旧私密log为`/tmp/nomifun-app-launch-0CPGKw/`，完整`native-active-restart-first.console.log`。新CUA恢复同Session/原seed历史，终端项消失；第二次真正创建WK唯一loopback页面，Raise后native child AX完整可读。随后独立public terminate probe精确8147请求accepted，callbackEntered/token1、真实cleanup、单receipt及NativeYES，controller exactkernel0/terminalreceipt0true/final_verified，整条runner命令exit0，无第三代。当前Apple Silicon/macOS26的两代Dev重启及liveWK正常AppKit退出通过；不是实际bundle replacement、Release successor、Intel/macOS14或OS logout。

退出后精确7545/8147/8084/8085均gone。私有quiescent DB的普通Python mode=ro再次出现unable-to-open，首因未确认；只在确认写入/App结束后用immutable=1取证，terminal_sessions=0、Session=2，未进入生产/活跃DB读取，也未删除WAL/SHM。旧错误表名`terminals`查询失败只属取证命令错误，不推断生产损坏。

MAC-33根因核查使用固定版本的[OfficeCLI UpdateChecker](https://raw.githubusercontent.com/iOfficeAI/OfficeCLI/v1.0.156/src/officecli/Core/UpdateChecker.cs)：
更新入口的SKIP判定早于配置与skill刷新；该flag不覆盖先执行的可选[CLI日志](https://raw.githubusercontent.com/iOfficeAI/OfficeCLI/v1.0.156/src/officecli/Core/CliLogger.cs)。
本轮实际受影响的是`/Users/muri/.claude/skills/officecli/SKILL.md`、`/Users/muri/.cursor/skills/officecli/SKILL.md`及`/Users/muri/.officecli/config.json`；
确认修改时间分别为03:30:57和03:31:00。仅保存了观察到的新skill副本，没有将新副本冒称旧备份，也没有覆盖用户文件作推测回滚。
Intel阶段验收与before/after报告保留在Git外`/tmp/nomifun-macos-intel-build-audit.SuFcMN/`。

MAC-32依据：当前signer源码与本地Tauri2.11.2 schema/官方signer均未自动补资源声明；见
[Apple Hardened Runtime](https://developer.apple.com/documentation/xcode/configuring-the-hardened-runtime)及
[Tauri entitlements配置](https://v2.tauri.app/distribute/macos-application-bundle/#entitlements)。用途说明、签名entitlement和系统用户授权是不同条件。

MAC-34/TCC请求顺序依据还包括[WebKit官方说明](https://bugs.webkit.org/show_bug.cgi?id=229590#c18)：系统及缓存检查先于网站delegate；当前实际tccd因果记录与该顺序吻合，不能用签名或页面焦点替代系统授权回执。Codex窗口只读检查被原生UI工具安全策略拒绝，没有改用其它方式绕过，未给予Codex相机权限。

隔离 renderer 视觉证据保存在 Git 外 `/tmp/nomifun-macos-renderer-audit/`。其中缺少fixture数据代初始化的首次、第二次失败独立保留，成功截图另记；产品generation保护未改。

恢复后的当前源码复验：持久目录 `logs/current-desktop-lib.log` 实际 163 PASS/0 FAIL/3 ignored；`current-macos-runner-tests.log` 实际 72 PASS/0 FAIL/290 expects；`current-a11y-tests.log` 实际 29 PASS/0 FAIL。上述旧首失败仍按历史保留，不将新重复运行当原首 PASS。正式 owned-element fixture 重新构建 3.11s PASS，恢复 Swift 编译、ad-hoc 严格签名和公开 LS 注册通过；原 installed 自有 bundle 移至私有目录保留，没有操作其它 App。

正式 Computer 最小 cursor/launch 旅程的后续历史结果：用户授权 Dev AX/Screen 并完成系统认证后，受控 LS 重启的新实例实际“资源已就绪”；Session `01a13342-3f93-7e31-b514-369e775a60fe` 单次请求仅查询光标、按裸名启动自有 Launch Audit。固定 localhost 模型 calls3，无失败、finalresponse true、report passed=true；两个精确 canonical tools、completed Turn 与 succeeded Effect 收齐，独立 marker/PID10626/PPID1及 CUA 原生窗口标题一致。正常退出该 tiny App 后确切 PID gone，未采集 tiny App kernel exit code，不冒称 code0。该轮原始 `/tmp` 报告当前缺失；这里只保留当时实际工具输出的历史结果，不代本轮完整元素输入。

持久正式元素 attempt1：新数据根为 `elements/data-attempt1`，Session `01a13476-d28b-7ff2-9510-e00a774b8271`，preparer 四个公开 API stage 201/200/200/200，prepare backend stopped=true，控制端口50205。受控 Dev 冻结包 `target/macos-dev-app/a39eb2fd7a00702c/generation-LPSCpe/NomiFun Dev.app`，App4018/PPID1、controller4017，公开 LS 两阶段身份通过。CUA 在正式“能力与权限”页点击重新检测，AX/Screen 均为已授权；返回确切 Session 后只提交一次固定请求。正式 launch 启动自有 Element Audit PID4469/PPID1、精确 bundle/path、nonce 和 window196，CUA 实际窗口标题、五个控件及前台焦点一致，root gate POST200 后才执行 AX 观察。

该 attempt1 首 native FAIL 保留在 `elements/attempt1-status.json`、`attempt1-report.json`、`attempt1-widget-witness.json`、`attempt1-canonical-classification.json`。calls4 后接收端只有左 target 一次 mouseMoved/type5，没有 down/up 或 receiver callback，普通点击工具却返回 guarded pixel fallback；fixture 明确失败并停止，未继续 right/double/setter。canonical 只有三个 tool calls、launch/input 两个 returned Effects，随后 turn/failed 为 fixture400，retryable=false；没有把 returned 当接收端成功，也没有重试物理输入。首 button 的真实落点未采集。后续只读当前宿主 `attempt1-current-display-readonly.json` 的 bounds/pixels 均1710×1112，AppKit→Enigo readback 与 Quartz cursor 当前一致、native_events_posted=0，因此不能将本次首 FAIL 宣称已由 Retina 倍数误差精确归因；源码中 move 后重读可变 cursor 仍丢失同一次 guarded target 合同，MAC-44 据此窄修。

首轮失败后 Dev 正常 Command-Q，完整 runner exit0；exact kernel Some(0)/no signal、terminal receipt code0/cleanup_verified=true、final_verified=true 收齐。App stdout/stderr 已复制到 `elements/native-attempt1/`，没有复制或输出秘密环境与控制描述符；本轮首 canonical 数据和首 FAIL 文件不删除。恢复后的 tiny fixture 后续窗口状态读取出现另一个自有 PID5084/window211，不能当4469首事件证据；清理动作不并入首输入 PASS。preparer 通过 /shutdown 正常结束，未强制终止或修改旧 failed 事实。

MAC-44 窄修已经实现：仅 `nomi-computer/src/input.rs` 和新 `macos_pointer.rs`，没有修改 Host/Store/夹具断言、添加依赖或改 Cargo.lock。每个操作持有 private Quartz source；move、down/up、dragged 和 scroll 都显式携带同一次已准入 point，本地 held 状态代替异步 OS pressed 状态读取，单/右点击 count1、双击1→2。按下前登记匹配释放义务，释放只重试同一 button/point/count，移动失败保留最后尝试坐标，panic/timeout 保留原 cleanup 和 worker 合同。公开 CGEventPost 返回 void，只证明 publication；工具仍要求观察结果，没有新增接收 ACK 或假完成证明。

MAC-44 pure 首 targeted 17 PASS/2 device ignored，追加 panic RAII 与 targeted-scroll 初始移动失败边界后 19 PASS/2 ignored；全库最终实际 123 PASS/0 FAIL/7 ignored（`logs/mac44-computer-lib-final.log`）。12 项新增 pure Recorder 直接检查事件坐标、类型、click count、故障释放与无重复 press；没有触发 OS 输入。独立 readonly review 对照当前 SDK 的 public CG ABI/constants 和 retain/release，未发现 must-fix。Intel `--tests` check 1.67s PASS、Windows MSVC `--tests` check 15.10s PASS，均不代硬件运行。Agent Session、UARC、桌面 UI、Browser 与 process boundary 当前全部 PASS，diff check PASS。workspace rustfmt 配置明确 disable_all_formatting，未更改该政策或格式化其它文件。

新原生 attempt2 的自有字段先窄修 authoring 角色：attempt1 的真实 AX 返回 value/readonly 为 unknown，未弱 fixture 的 textfield 断言，而给实际 NSTextField 的显式 accessibility element 声明 `.textField`。旧 Swift bytes 保存于 `elements/OwnedElementAudit.attempt1-before-explicit-textfield-role.swift`，原生 super setter、readonly selector guard及事件断言不变；不由角色声明强制 settable=true。新 Swift 编译及 strict 签名 PASS，attempt1 bundle 移入 private 原样保留。新 fixture build2m28s PASS，`data-attempt2` setup READY、Session `01a1348f-91d6-7270-8eef-c0a8951667b6`，控制端口50920；新 Dev build2m45s PASS、公开 LS 两阶段 confirmed，App7341/PPID1、controller7340、冻结包 `target/macos-dev-app/06199f35fa1ef074/generation-e7UBLX/NomiFun Dev.app`。新的 ad-hoc 实例当前权限未生效，尚未发送模型或输入请求；系统设置恢复用户既已授权的 Dev 条目时要求账户认证，已请求用户亲自完成。源码 fingerprint 保存为 `elements/attempt2-source-fingerprint.json`，正式原生 PASS 仍待收齐。

用户完成实际账户认证后，root 通过 public System Settings 仅重加 Dev AX/Screen 到上述精确 bundle，两个系统开关 on。OS 要求重启时选择“以后”，随后正式原生菜单重启；旧7341 cleanup receipt true/kernel0/final_verified，新8547/PPID1 public LS 两阶段 confirmed。新实例在正式权限页重新检测，AX/Screen 均已授权。本轮同一 Session 单次提交、owned PID8821/window442/nonce与 CUA 五字段/前台一致，root gate200后执行。修后真实 ordinary/right/double 接收端分别 mouse_downs1/right_downs1/mouse_downs2，buttons[0]/[1]/[0,0]，clickCounts[1]/[1]/[1,2]，精确 owned PID/窗口/labels一致；MAC-44 三类点击真实通过。

attempt2 整体仍 FAIL，未继续 readonly：calls10 时可编辑字段收到一次自定义 `setAccessibilityValue`、一次 super 调用并返回，但实际 stringValue 与 getter仍 seed，完整替换串未到控件内容。`attempt2-status.json`、`attempt2-report.json`、`attempt2-widget-witness.json` 和 `attempt2-canonical-classification.json` 原样保留；canonical 九工具、五个 returned Effects，终态 turn/failed 为 fixture400，没有 unresolved/unknown。这是自定义 authoring 改写 AX adapter 的真实失败，尚不足以归为生产 setter 缺陷；“一次 super.setAccessibilityValue”不是标准 NSTextField 内容替换合同，不把这项指标当原生 PASS。

独立只读审查依据 Apple 标准控件 AX、method-based setter 优先级及 NSControl/NSCell 文档，决定恢复未重写 AX 的标准 editable NSTextField，并用主队列只读控件/cell 模型值见证；不手动写入期望串，不用 AX 属性镜像代实际内容，不弱化非空 seed→完整 Unicode 值及零 pointer/key 断言。旧自定义源码单独保留，下一轮使用新数据根。readonly 继续是 instrumented selector 拒绝，不冒称 stock readonly 行为。失败后 tiny 的精确8821已退出，当前 witness移至 `attempt2-cleanup-widget-witness.json`；正式 Dev 正常 Command-Q，完整 runner exit0及 cleanup receipt/kernel0/final_verified双证明收齐，两个代际 App stdout/stderr已持久复制。模型服务通过 /shutdown 正常停止，不覆盖前轮失败。

标准字段 attempt3 使用 plain `NSTextField(string: seed)`，没有替换 cell、AX element/role/selector/写入 override。只读 main-loop sampler 在有 active editor 时不读取可能 validateEditing 的 Control getter；仅真实 snapshot 变化落盘，不写入期望串、不制造 setter/SDK 次数。旧 attempt2 source/meta 保留，legacy 未观测 AX 指标为 null。fixture 改用 actual control/cell 完整值、非空 before、无 editor、一次 sampled 变化与零 pointer/key，唯一 canonical setValue 另由事件链证明；不是内部SDK调用计数。fixture build6.73s及 Swift/strict签名通过，新 `data-attempt3` Session `01a134b3-1f3f-7720-a61d-c5461aadd32b`、control51559。Dev10253/PPID1 public LS confirmed，冻结包 `target/macos-dev-app/1d8bdf7988a9b730/generation-2U73QQ/NomiFun Dev.app` 的 binary SHA 与 attempt2 同为 `effface4d2d262142456fc1788255e05eb7d0e8365d4779461bae33bb8279941`，当前授权保持有效。

attempt3 native 五项行为实际收齐：owned10519/window492前台gate200后，三种点击再次通过；标准 NSTextField/NSTextFieldCell 的 actual control/cell 均从 `seed 非空 α😀` 完整变为 `完整替换 中文Ω😀`，editing=false、editor=null，sampled变化1，字段pointer0/keys空。readonly的实际setter/键鼠均0、值保留，公开结果确是 `ROLE_HOST_UNSUPPORTED_INPUT`，Effect为 rejected、内部code相同并保留精确 `COMPUTER_INPUT_UNSUPPORTED_NO_EFFECT` 原因，五个其它Effects returned、没有Unknown。整体 attempt3 仍是 FAIL/turn-failed：夹具误要求公开工具错误也包含内部说明，内核实际上有意只暴露code。`attempt3-status/report/widget-witness/readonly-canonical/effect-classification.json` 首证据不改写。只修夹具匹配确切公开错误，同时在 bounded owner Effect 核对内部零效果原因；没有修改生产消息或放宽型别/效果断言。fixture修后build6.06s PASS，下一轮 fresh scope 收齐正常完成终态。该轮模型服务正常停，tiny确切PIDgone，正式Dev Command-Q取得 receipt/kernel0/final_verified及runner exit0；App日志已持久复制。

attempt4 使用相同生产 binary/标准 Swift，fresh `data-attempt4`、Session `01a134be-4124-72a2-9342-365af3514671`、control51719；Dev10905/PPID1 public LS confirmed，包 `target/macos-dev-app/eb3dedbe9c4a4a2f/generation-FgNb0Z/NomiFun Dev.app`。owned11163/window540的五行为再次通过，仍整体 FAIL：root 上次把 canonical `EngineToolError` 的公开 Display 当成模型侧 `AgentEngineError` 的 Display，后者实际含 `Agent Runtime Tool` 层。该可避免的夹具错误保留为 `attempt4-status/report/widget-witness/readonly-layer-results.json`；两种实际文本分别由144 `tool/result-recorded` 和145 `runtime/progress-recorded` 原样证明，不凭猜测删内部限制。只将夹具预期改为真实 typed KernelError → 两种 wrapper 构造，模型 `is_error=true` 与确切 Runtime文本、canonical output=null与确切Core文本、bounded Effect完整零效果原因分别严格核对。

最终 layered-error fixture build3.71s PASS；独立 readonly review 通读报告全部断言并对照生产映射，确认12次普通Stop可完成、11工具不自动启用task ledger、6Effect状态/因果/结果、标准模型值与稳定witness匹配均一致，未发现剩余 wrongassert。没有修改生产代码或重复广泛测试。attempt4模型服务已正常 /shutdown，tiny确切PID已退出、cleanup marker移入private保留；root 请求正式Dev正常退出时原生工具报告 Mac locked，未操作锁屏或强制终止。当前受控Dev与runner继续存活，尚没有本轮最终exit0双证明；已请求用户手动解锁，最终fresh scope及正常退出验证待继续。


## Computer 最终原生 attempt5 / attempt6 与退出记录

上述 attempt4 锁屏前提随后由用户解锁解除，原 Dev 正常 Command-Q、cleanup receipt 与 exact kernel exit0 双证明完成；模型服务已正常 /shutdown，没有强制终止或锁屏操作。

attempt5 使用相同生产 binary、fresh data-attempt5、Session 01a134d9 前缀、Dev12216/PPID1、owned12497/window626，整值 control/cell 确实变为“完整替换 中文Ω😀”，但整体 FAIL 保留：其 lifetime field pointer 计数有4次mouseMoved。全窗口271个事件含261次move，事件未采集source PID，不能归因于人工或SDK。证据 elements/attempt5-event-summary.json、attempt5报告及console。夹具原来的整个窗口生命周期零输入约束不精确覆盖那次setter；修成提交setter前保留真实seed/nonediting和全窗口InputTotals基线，setter回执后严格比较全窗口pointer/key增量及field增量。没有清零、丢弃事件或弱化readonly拒绝要求。

用户确认短暂独占鼠标键盘后，fresh attempt6 的生产 binary SHA256 effface4d2d262142456fc1788255e05eb7d0e8365d4779461bae33bb8279941，正式Dev target/macos-dev-app/5fd2d901fedfd1d6/generation-7FmROR/NomiFun Dev.app，actual12956/PPID1，两阶段public LS confirmed；fixture独立localhost模型不是外部服务商。Session 01a134ed-efa5-7800-914e-87aa8f5998bd，owned PID13213/window708/nonceUTQSha，真实CUA前台/身份确认后root gate200。工具观察frontmost并校验实际nonce/窗口；operator隔离不冒称硬PID API边界。最终12模型calls、11工具、1预期readonly拒绝，整体passed=true/pending=false，canonical Turn completed/ready，6Effect=5returned/succeeded+1rejected/failed，unknown与unsettled均0。

三类真实receiver计数为左1、右1、双击2，按钮0/1与clickCounts1/[1,2]、实际owned PID/窗口一致。标准NSTextField/NSTextFieldCell 的非空“seed 非空 α😀”都整体替换为“完整替换 中文Ω😀”，非editing/editor=nil，仅1次实际transition；写入区间全窗口InputTotals前后同为 keys={}、pointer_total=11，field计数也前后相同。readonly保持“readonly 非空 β😀”，setter_requests=0、pointer=0、keys={}，public ROLE_HOST_UNSUPPORTED_INPUT 与 bounded Effect精确前置零效果原因一致。CUA实际读取了完整新值。没有依赖SDK内部call计数、截图、task ledger或CompletionReported假完成。

完整PASS报告 elements/attempt6-report.json 在cleanup前保存，summary/status/source-fingerprint及 logs/elements-native-attempt6.console.log 分别保留。正常关闭owned App后，CUA getAXState 曾自动重启该工具App到13229/window720；这是工具query行为，不能当原本13213的PASS witness。另关闭重启实例并切回Dev查询以避免再启动，相关marker另存 attempt6-cleanup-query-relaunched-widget.json。旧/新owned PID均退出，实际installed bundle仅unregister并保留到 elements/preserved-final-installed.app。Dev随后真实Command-Q，controller actual_status code0及receipt cleanup_verified=true、final_verified=true，native exec1116 exit0；actual12956/controller12955已退出，fixture /shutdown后preparer36013 exit0。已通知用户可恢复正常键鼠操作。该二进制证明止于MAC44，不覆盖之后新增MAC45通知代码。

## MAC-45 通知首反例与实施记录

源码确认锁定 tauri-plugin-notification 2.3.3 desktop permission_state/request_permission 都恒Granted，Dev mac sender指定 com.apple.Terminal，异步忽略show错误；mac-notification-sys 0.6.13 的setApplication实际交换全局NSBundle.bundleIdentifier实现。根因是生产依赖与调用链，未执行旧sender去制造系统通知，也不宣称已有CF身份读取已受污染。之前 MAC14 UI“已授权”记录仅证明当时页面文本；本次确认其不能证明真实UN权限。

新MAC-only notification plugin保留同namespace/三命令/官方ACL，在真App.bundle内用公开UN查询权限、显式请求Alert/Sound、等待addRequest回调；CLI/未知OS枚举拒绝，NotDetermined=default、Denied=denied、Authorized/Provisional=允许提交（不等同banner）。回调单发、caller退休与timeout晚回调无重试、Rust panic/ObjC异常围栏，10/300/30s各操作有界；foreground公开delegate由main TLS保留，只接自有弱delegate，不替换已有第三方owner，OS最终决定presentation。旧sender没有显式shouldPresent:YES证据，不能把新foreground支持写成旧实现已有的能力；notification click导航仍不补。

前端改为直接等待permission/query/send IPC，通知已授权仍显示打开实际系统设置，失败撤过期状态；query/request共用revision与cleanup防旧async成功/错误覆盖新状态。首DOM红测11PASS/7FAIL和独立review新增race红测22PASS/4FAIL分别保留，最终26PASS/0FAIL/82expects，typecheck和desktop boundary2091sources/min880×600通过，独立review重跑26项通过。对应 logs/mac45-ui-first-failure.log、mac45-ui-race-first-failure.log、mac45-ui-final-26.log 等；纯UI/mock不代真实OS授权/通知。统一Rust首轮编译确认为新delegate的PartialInit错误使用直接init（E0277），exit101，日志 logs/mac45-rust-first.log 保留；尚未运行Rust tests。只修正superclass init后重跑，不将此归因于OS或上游。


MAC45统一修后Rust 8 PASS/0 FAIL（9.37s，logs/mac45-rust-fixed.log），原first-party初始化/ACL定向2 PASS（mac45-plugin-acl.log）；对应主线程delegate init首次E0277/exit101日志另保留。正式Dev本轮 actual17036/PPID1、controller16968，包generation-QbdWUj public LS完成；CUA通知页真实显示“已拒绝”，系统设置 NomiFun Dev allow-notifications off一致。用户明确允许 Dev 与独立 audit.notifications 的通知验收后，仅Dev开关改on，回Dev焦点自动fresh query成为“已授权”，ready仍有系统设置按钮。没有发送生产Dev通知（当前没有正常renderer消费入口）；正式query/实际OS前后及scope记录notifications/dev-attempt1-ui.json。真实Command-Q、receipt cleanup_verified、exact kernel0、controller final_verified与exec34251 exit0通过。

MAC14首次实际deep-link还没有完成：旧bundleIdentifier参数仅打开通知总页，root随后手动点选实际Dev才读到详情，不能把该手动步骤算deep-link成功。当前小修URL参数为id并同步既有identity/reject-invalid测试；原native总页FAIL及修后native落点结果分别保留。该系统设置URL不是Apple稳定SDK契约，当前宿主实机验收不能代所有macOS版本。


MAC14修后既有identity/invalid/no-bundle定向1 PASS（compile2m53s），日志mac14-settings-param-fixed.log。fresh正式Dev actual18133/PPID1、controller18132、generation-uAdeAy，publicLS confirmed；在已授权通知页仍见“打开系统设置”。root先把系统设置切到通用，随后只点击产品入口，实际系统设置直接到“NomiFun Dev”通知详情，开关on，无后续手动选择App。这证明当前macOS26宿主 id 参数正确；原bundleIdentifier总页FAIL保留，非所有系统版本契约。证据notifications/dev-attempt2-settings-ui.json及mac14-settings-native-attempt2.console.log。真实Command-Q actual0、receipt cleanup_verified=true、controller final_verified=true、exec23725 exit0通过。未发送正式Dev通知。最终全项目bun run check exit0（typecheck、i18n/theme/icons、desktop880、Browser/process/Agent Session/UARC等标准完整链），logs/final-project-check.log。


MAC45 Intel生产bin正确环境crosscheck2m05s PASS（mac45-intel-check.log），不代Intel硬件。独立组件first build4m44s PASS，但first native42011/PPID1虽真实auditID/LS confirmed，页面blank且report operations=[]/pending=null：夹具错误拒绝Tauri真实tauri://localhost空path，不是生产通知缺陷。CUA正常Close触发run_return0、main实际exit1、receipt code1/cleanupfalse、controller final_verified=false，exec39222 exit1。完整首次report/App/log/package分别保留，first失败未升级PASS，确切42011已退出。该失败scope未请求授权/发送或清理通知，不能由UI截图宣称模块已验；后继测试用修后source与新bundle/data/report，保留前次失败身份，未改通用launch guard或强杀。修正仅fixture_document接同scheme/host合法空根路径并拒userinfo/port/query/fragment，boundedtrace不打印URL数据；source SHA04d3ab3ce20b7440f7d2228a12b07315010f92e24d77f059b25f20fbd4a45a5b，下一scope实际trace再核。


MAC45独立组件修后build4.37s PASS（logs/mac45-fixture-build-fixed.log），新source/packaging指纹与firstFAIL分开保留。fresh attempt2 actual42424/PPID1、bundle com.nomifun.desktop.audit.notifications、独立app/data/report，publicLS completed。实际trace NAV scheme=tauri/host=localhost/path=empty allowed=true、ASSET index present=true，CUA页面真实载入，与白页源码根因一致。共有7操作：自动freshQuery null/NotDetermined；首次可信Send明确reject“macOS notification permission has not been requested”，无暗中Grant；用户已授权范围内实际Notification Center系统弹窗显示NomiFun Notification Audit，CUA执行其公开“允许”动作，请求IPC granted；freshQuery true；随后仅一次OS发送callback accepted，public delivered返回实际request ID 01a13529-567c-7951-bf82-1ce3b867b826，完整title/body nonce 01a13528-5668-78c1-8f79-e44d9ac1d47e及中文Ω😀一致。

实际发送被系统接受和公共投递记录是不同分面。旧NotificationCenter binding已退休，读取timedout；ControlCenter无窗口binding也timedout；Apple官方Fn+N快捷键在CUA返回keyNotFound(fn)，未执行。没有再次发送来追横幅，也未改全局Focus/共享屏幕通知偏好。故visible_banner_verified=false，不能写用户已看到；真实OS NSError submission拒绝未自然触发，纯callback/生命周期检查不冒称该native分面。

可信Clear仅移除上述实际delivered request ID，pending_before=[]，随后 delivered_after/pending_after/final_delivered_after全=[]，current_inventory_clear=true、notification_cleanup_verified=true，不以void移除当ACK，不删其它通知。可信Finish正常app.exit0/run_return0，final NSBundle/PID/path与初始一致，7操作均settled且pendingnull；component_cleanup_verified=true。controller receipt code0/cleanuptrue、exact kernel exit0、final_verified=true，exec69711 exit0，actual42424已退出。报告scope始终standalone_component_notification_fixture，formal_dev_sender_verified=false、native_system_quit_verified=false；生产正式Dev CmdQ另有前两轮真双证明。component-attempt2-report/before-finish-report/summary/package/controller-result及App stdout/stderr均保存persistent notifications目录。旧attempt1仍是正常abort1/cleanupfalse/finalfalse，不会被新PASS覆盖或当共享cleanup成功。
