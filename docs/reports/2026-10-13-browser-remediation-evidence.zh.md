# 内置浏览器整改实施记录

整改覆盖共享 UI、Windows WebView2、macOS WKWebView、原生普通页与 popup、人工导航取消、Profile 与网站验证结果。任务保持进行中。用户随后明确要求当前环境先不继续 macOS 工作，保留已有改动并准备 [macOS 交接](../handoffs/2026-10-13-browser-macos.zh.md)；当前执行集中于 Windows 和共享层。Windows 本地原生 fixture 的导航、Profile 与 Agent 链已通过，9C6F 正式 GUI 及真实网站已取得下文限定范围的证据。9C6F 发现的同文档 History 状态缺陷现已源码修复并通过定向原生验证，后续95E5新主程序已构建并接续启动，对应同文档History已实机通过，本轮880×600原生输入、显隐与宽屏恢复也已实际通过；macOS 原生验收没有被视为通过。

当前已确认的产品约束：内嵌浏览器保持主要场景，用户与 Agent 使用同页；系统浏览器只保留部分场景功能，不作为主场景重构、自动回退或替代验收。防重、状态、兼容性、存储、诊断及恢复优化继续在此边界内推进。external_page 是内嵌 WebView 的外站创建选项，未改变产品窗口或浏览器入口。

**Windows Bilibili根因、修补与当前证据（2026-10-15）**

Windows搜索子页持续加载已定位到应用的原生呈现切换：真实popup激活后，宿主过早把opener的WebView2 controller设为不可见。相同原S1 Session/Profile的静态UI对照中，旧行为使child停在Provisional/None/0%，切回parent后才很快收到正文事件；只保留准确opener的controller活性即可正常完成，撤回该修补又重现。由此确认本次Windows/WebView2 155环境下的应用边界原因；不把未直接测得的SDK/Chromium内部调度细节写成普遍规范，也不据此归因原始macOS百度现场。

最小修补位于[Windows Host](../../apps/desktop/src/browser_surface/host.rs)与[原生呈现](../../apps/desktop/src/browser_surface/windows_presentation.rs)：NativeTab仅为真实popup保存一个可选的exact opener target；在当前可见面板、活动child、同runtime及双方未关闭的约束下，首次文档pending暂时保持opener controller可见，同时HWND继续SW_HIDE。child的原44A无内容遮罩保持不变，不能暴露parent像素或改变输入焦点。child提交、终态、Pause、后续尝试、关闭或surface lease失效会释放该hold；UI线程再次检查准确owner/target及当前状态。没有snapshot副作用、第二套ledger、CDP救援、UA/代理变化或重新导航。

| 静态UI对照 | 实际结果 | 安全证据 |
| --- | --- | --- |
| 77A46190旧行为 | 鼠标搜索后22.773s仍0%；37.829s切回parent后125.582ms收到首data、157.184ms收到loadingFinished | `gui-cu-static-baseline-77A46190/probe-safe-network-receipt.json` SHA `73C4939DB9D4B6E4E4C8E6C0E836CD7A46AC0AADCE64695C5F33E739B63A1771` |
| C212F598修补 | 3次真实鼠标搜索均显示结果，准确root Document的loadingFinished距点击分别1019.768/1294.166/1028.589ms，无切回parent救援 | `gui-cu-static-opener-C212F598/probe-safe-network-receipt.json` SHA `A77932F728793432DFD5DEAD5DCC2B3628F8B1F87B2EFC319D86CBBAF3B7FD6C`，62行safe样本 |
| E429F4CD撤回修补 | 再次25.616s仍0%；49.358s切回parent后138.667ms收到首data、174.794ms收到loadingFinished | `gui-cu-static-recheck-E429F4CD/probe-safe-network-receipt.json` SHA `E6E901EC0786DDBCE32F9A1538E0EDC7ADC1E189838F4DB1AB14EAA8A99AE23F`，30行safe样本 |

三组均使用原任务DATA/WORK与原S1 Profile，没有复制/清空Profile；同一静态UI manifest为0.8.8/API30/build-id `1f7e0b07-b245-4f04-aaec-69c66d8d0fa4`，通过现有Tauri/custom-protocol内嵌资源加载，不依赖Vite5173。上表是A/B/A原生对照，时间为host callback到达与实际输入时间之差，不是线上TTFB；按exact view/root frame/requestID关联，排除iframe。HTTP200响应头、Page读取限时未回、单次被动恢复均未被单独当作根因证明。C212 main完整SHA为 `C212F5985127049691E9BB0D901090C0B7770E351DCA33B229365296D060BF82`，构建EXIT0；presentation定向7项/0失败，unit与main各4219来源及424份静态资源前后差异0。独立review无阻断项；对应host源码SHA `78424B212F7BC0B164D86579A43F0A0D57244DC4F3C98AE796271E4D57E28BE3`、presentation SHA `00F43DAAEA93C0D627AE161A6652E207E466A3CBF88C5820A21071FC6743B4E3`。撤回对照后已恢复并核验此C212二进制与源码；恢复后PID28612的最后一次真实鼠标搜索同样通过：UTC03:31:11.489点击，准确popup/root/request的loadingFinished为UTC03:31:12.641514、距click1152.514ms（probe elapsed851ms）。Root在搜索后169.084s的真实窗口观察仍见同tab正常结果，期间没有切回parent、reload、Stop或新导航。最终安全30行JSONL SHA为5EA0E9DC3A068F11D2F4143F9326527E970EB620B8A996DD8D31803D9EBB7552，receipt SHA为8A0C5D1D515E8CF20293FDE61D47550855BF1DB61BC7F29251319709ED45A5A2，`gui-cu-static-final-C212F598/05-final-mouse-result.json` SHA为833676B423A485099E99911286EF840A13CA18280F746D9F0A620A1B5D661ABE。最后程序保留可视运行供用户查看，不宣称PID28612已退出。

程序启停已由Root通过computer use自行完成。为隔离验收加入的main debug配置仅在编译设置测试data/work时启用，窗口X沿既有QuitFlag/ExitCoordinator正常退出；普通debug与release仍保留原产品关闭行为。实际已观察X退出、准确PID消失以及channel/plugin/terminal的cleanup milestones，不扩大为OS exit status或所有WebView/Profile/数据库逐项清理证明。既有localhost拒绝连接属于当时Vite服务不可达，静态UI版本避免该验收依赖，内嵌浏览器产品形态不变。

Root在2026-10-15 UTC03:02 fresh fetch/ls-remote核对HEAD、origin/main、canonical remote均为 `2d5aa62d3f7daf2869fe20be457174742431238a`、0/0、无未解决冲突。同步时250路径的232逐字节保全/18重叠整合证明保留；随后明确授权的main debug验收配置新增1个tracked路径，当前251个本地路径尚未commit/push，备份与stash保留。后续Mac须携带最终portable源码包/manifest，不能只pull远程。Mac完整Host与两个smoke、本轮WK生产GUI、身份A/B、正式store和原始百度现场仍待真实Mac；queued/in-flight popup Pause缺口按用户指示仅记录交接。Windows本项修补不等于跨平台Goal完成或零风控。

**同步与早期Bili续查（历史，2026-10-15）**：本地已同步至 `HEAD=origin/main=2d5aa62d3f7daf2869fe20be457174742431238a`，Root现场核对ahead/behind为0/0，workspace现为0.8.8。同步前250个本地路径已备份并保留stash `3ba56082e18254650f88fe48a5999f29883d1dbb`；恢复时232个非重叠路径逐字节保全、18个重叠整合、unexpected=0，原staged为0并恢复为0，无未解决冲突。5个文件的文本冲突以审核过的7份Mac候选解决，保留远程快捷键/权限/AppKit/LaunchServices与本地浏览器合同，没有借同步实施Mac queued/in-flight popup修复。

本轮证据根为 `C:\Users\MINISFORUM\AppData\Local\Temp\nomifun-browser-bili-sync-c0200bd3e0d84e9f9c63a6a66df90927`。`restored-source-verification.json` SHA `848C9BAC0B79A2DE9ADBFA62957D2DB6A60D9A614E4F289666B42EEFE7D79839`；`applied-macos-candidates.json` SHA `B2C61ED7E8D9B38680BC51B399FE5A0A5416C5BE1F6F0D40F6F3368AA26B4FED`，7个已应用文件的磁盘hash已独立核对一致。备份、pre-sync patch/stash与融合审查记录继续保留；同步成功不等于未提交修复已进入远程，Mac接续仍须携带后续portable源码包及manifest。

| 同步后定向验证 | 实际结果与证据边界 | raw证据 |
| --- | --- | --- |
| Cargo metadata | EXIT0，locked/offline、Windows filter-platform；不是构建或运行 | `bili-post-sync-metadata` |
| Windows生产模块 | 125 passed/0 failed/121 filtered，最终frozen批次EXIT0；不构成Bili网站修复 | `bili-post-sync-windows-unit-frozen`，stdout SHA `D1D451D40730DE4C47A0BB213AEAD1F0CCD2156019535EB12C7B39E10F36D3B0` |
| Mac portable | 10 passed/0 failed，EXIT0；Windows上portable reducer范围 | `bili-post-sync-macos-portable-unit`，stdout SHA `C36CF843BDD038BB4AC2966F89E7EB57A9D0E316FF673F9B6408D2ACEC233222` |
| Mac crate Darwin all-targets | EXIT0，仅nomifun-browser-macos、aarch64-apple-darwin；不含完整desktop Host/App链接或本轮WK运行 | `bili-post-sync-macos-target-check`，stderr SHA `392603C0B53337053D093F1D6385998C96CD055DBF859DAE69C89138403A657C` |
| BrowserWorkspace | 34 passed/0 failed/59 filtered，EXIT0；Unix cfg的1项未在Windows编入 | `bili-post-sync-browser-workspace-unit`，stdout SHA `27F67CD83D2FB4577D008ECEE9F8AAB626EDA98BB4EC86A313B9B72A76F0898B` |

以上5个最终stage的launch/exit/raw日志与before/after均已读取，每批1870来源路径独立比较差异0。UI汇总 `ui-post-sync-validation-summary.json` SHA `32FFB49BEBAFC7CECF0E4DA6349C243AE96F5E31480B4E55DE6F6866A0D7739F` 的9份日志hash全部吻合：Browser三文件145项/923断言、Space6/56、Resize6/100；i18n7703 keys/33 modules、desktop2115 sources/min880×600、browser boundary与typecheck均实际通过。Root另外实际运行Agent Session boundary通过；这些检查分别证明合同/类型范围，不替代原生网站采样。

以下为修补前A166阶段的历史事实，当时Bili搜索子页**未查明、未修复**；启动拒绝随后已解除：用户后来手动启动A166 probe main，PID13864，真实UI已加载，没有换接口绕过自动审批。准确native SHA为 `A1667B61F06E11474B00C8CCFFECEB1FB018925985DD5A45DAFEBE1932E5F511`，native基于同步前3692，renderer基于同步后2d5；这是混合诊断基线，不是最终融合native的GUI验收。新事实位于本轮证据根的 `gui-baseline-A166/`，下文95E5及更早证明不倒改。

| A166本轮观察 | 实际事实与限制 | 证据 |
| --- | --- | --- |
| Enter搜索popup及同URL普通tab | 02/03/04快照中Enter生成子页、后续约9秒观察已有真实搜索结果；05原生trace同次seq1/docgen1、200、commit885ms/finish1549ms，user0/page1/drop0/routing_lost=false。06/07普通tab访问相同URL也显示结果 | `05-popup-navigation-diagnostics.json` SHA `7F2122193B8B9AE6EF06DE9BBC25A7A21C79840671877FD00DABCEE4D0B3F08A`；观察间隔不是加载耗时 |
| 鼠标搜索popup长时间pending后同次完成 | 09/10为None/0%；11于UTC16:46:53.209、搜索后113.98s仍Provisional/None/0%，12保留同阶段截图。15确认准确同child/runtime1/control1，docgen0→1，trace commit121462ms/finish122110ms、200，仍user0/page1/drop0/lossfalse。14约250秒时看到真实搜索结果，但实际完成时间按trace，不称加载250秒 | pending SHA `41D81DC162439B08501A207FBF533599E931041A3BB064A2B7EF007FF0F1BB97`；recovered SHA `1C3787FD452680EFFBD01FC87F7C7F66B897B7EED46A6295A41C1ACAC26923D8` |

此次没有以reload、Stop、runIfWaiting、改UA或代理救援该pending child。`probe-safe-network.jsonl` SHA `E16FE90029CB26D2F154D19AA3D7000B322CF1C5ACA499273FD18DEE2C8ABA34` 记录该view的Network.responseReceived为200/632ms；样本自身仍声明navigation association unknown，未取得body完成证据。因此不能称网络已正常、body已完成或问题已归因renderer，也不能把被动等到结果称为修复。Page读取未在500ms内回复只记限时观察，原因未知；TargetInfo回复与Page未回复不能推出renderer hang或Debugger waiting。当时下一probe仅计划补body完成取证，probe定向unit通过本身不构成站点修复；后续根因与修补以上方A/B/A为准。

早期同步后main构建 `bili-post-sync-probe-main` 实际EXIT101，stderr失败点为无法删除被PID13864占用的nomifun-desktop.exe（OS error5），未出现Rust源码编译诊断错误，不计成功build；`bili-post-sync-probe-main-full-source-exit.json`记录4215来源差异0。该阶段Root曾请求用户正常托盘退出；后续已改为Root通过computer use自主启停并取得上方成功build和静态UI采样，不能以旧125项unit替代。fresh fetch后Root仍核对HEAD=origin/main为2d5、0/0、无U冲突。Mac完整Host/WK、A/B、正式存储与原始百度现场仍交接；本段保留早期诊断边界，当前Bili结论以上方修补与实际对照为准。

| Goal 要求 | 当前实现 | 当前证据 | 证据边界与交接 |
| --- | --- | --- | --- |
| 共享 UI 同批防重与旧操作隔离 | 同步 operation token，Session/client/host/unmount 失效；Stop/Pause 独立中断 token，typed 页面恢复 | BrowserPanel 141 项与 client 2 项通过；9C6F 实际两 Session 基础切换、独立页面与输入通过 | GUI 强 old-finally/new-busy-gate 重叠仍未证明，约70秒的两截图间隔不能替代该用例 |
| Windows 导航事实与精确取消 | 原生 NavigationId owner、协议失败/下载、旧回调与 Stop/Reload guard；准确 Stop/Pause 取消终态与独立迟到事实；在途 popup owner 等待真实关闭证明；History 控制 fence 与真实文档尝试分开 | 同步后Windows模块125项通过；历史64CB/34DDD的完整导航与准确popup阶段、60C/72E History与BeforeUnload Stay原生通过；95E5对应History GUI及最低窗口/输入通过 | 同步后125项不扩大历史native范围；Bili另由上方静态UI A/B/A定位并修补；DNS本机结果不是DNS失败证明 |
| 两平台诊断 | 有界 trace、独立 dispatch/native 序号、累计 counters、安全 action 样本与控制状态 | 同步前共享合同93项通过；DF6539 的安全有界 trace、87 次独立 HTTP 主文档 witness 及最终页面断言通过 | WK 原生轨迹；关联丢失故障注入属于 unit owner 范围 |
| 循环提示及明确暂停 | shared repeat detector、renderer 提示；native policy hold、显式 resume 无 replay | DF6539 完整导航通过 HTTP 冻结、A↔B、原 nonce 保留、Resume 仅放行未来 POST、首次加载 Pause 与真实 coordinator join | 正式主 GUI 提示/操作；WK 原生验收后续交接 |
| Windows 外部页脚本隔离 | explicit external_page 创建模式，保持 native COM/lifecycle | DF6539/64CB 主文档、subframe、popup、Worker 与原生 Agent 链；34DDD 首次原生 prompt 回复后同槽/CurrentDocument、双层可见及可信输入实际通过 | 更新正式主 GUI与真实站点兼容性；vendor mock tests 未运行 |
| 身份及 Profile | Windows 原生身份/effective environment/helper；macOS 不可变 native 对照 policy，默认仍 Safari17 | DF6539 生产 Profile 5 phases 重开、应用重启及不同 Session 隔离通过；native macOS crate 全目标 cross-check 通过 | macOS 同机同网实际身份 A/B 与原生存储；Windows 路径限制见下文 |
| 百度 smoke 与真实发起方 | 主页、搜索、Bilibili独立outcome，验证不算正常；主文档/SPA/实际provisional分别记录 | 历史百度/Bili主页与95E5 History证明保留；A166混合基线Enter/普通tab出现结果，鼠标popup同次长期pending后完成 | A166当时body完成未知且为混合基线；后续C212静态UI A/B/A证明Windows本项修补，Mac现场仍未复现 |
| Windows Worker 启动 | 保留 OOPIF 首脚本安全及 consumer 辅助预算，准确 auxiliary session 恢复后解除附着 | DF6539 完整原生导航中 parent/nested Worker、两层 frame、HTTP 执行与身份全部通过；107 项 production unit 包含 Worker/OOPIF | 真实站点范围；历史 production 基线失败仍按当次范围保留 |
| Windows Cookie 持久性 | native UDF SDK 输入采用同目录 ordinary 表示；检查实际 raw UDF getter，拒绝同目录 verbatim override | DF6539 的生产 Profile 5 phases 全 PASS：HttpOnly Cookie/LS/IDB 重开与应用重启保留，不同 Session 隔离；错误和 verbatim override 均 0 GET，来源数据保留 | UNC/过长/特殊名称无法安全转换时创建前拒绝，不宣告全路径/全 Runtime 支持 |
| 用户原生文件选择 lifecycle | broker 保持 chooser、文档与 picker process 的 ownership；fixture 单次输入与被动 witness 保持严格 | 0AD3 artifact 的 Dialog/user-files/user-download cancellation 回归 EXIT0，包含 OOPIF/目标 frame、Agent admission 与 close；DF6539 site-data 维护 view EXIT0 | 旧 chooser witness 失败仍保留；后继通过不证明间歇问题已消除；用户下载传输范围见下文 |
| TLS 失败 fixture | PFX DefaultKeySet、严格 preflight 与 managed cleanup 已补充 | DF6539 原生明确 Tls/WebView2 code 5，ClientHello=1、HTTP=0、证书 key 与 owned root 正常清理 | 保留旧 fixture 失败范围；本次未修改信任库或浏览器证书策略 |
| Agent 原生链与 Canonical 完成 | fixture 在同一 step 10 按实际广告工具选择报告或普通 assistant-stop | DF6539 EXIT0；本地脚本模型 11 次调用、精确 turn/completed seq 142、canonical history、原生上传字节/下载 SHA/输入恢复/cleanup 均通过 | 正式主 GUI 同页交互；本次脚本 fixture 不构成真实模型验收 |
| 正式主 GUI | 两个无Browsergrant的Session；9C6F+当时Vite renderer 的已完成批次；95E5新main已构建并接续启动，History对应GUI已通过 | 9C6F 的popup Pause/Resume、无内容失败恢复、基础Session切换、880×600输入/菜单/重开/宽屏；百度主页/搜索与Bili主页各自静置正常，49事件完整诊断已取得；95E5 History、880×600原生输入/显隐/宽屏恢复通过 | Bili搜索pending仅记录限制，其他未覆盖范围不设新增发布gate；Mac交接 |

**已完成原生批次的状态边界**：example artifact SHA256 为 `DF6539DE01127A9C61C913008EDA6E0E3AE839E3140A2C7FDF2E30FF3861CF1B`，对应生产 Windows 模块 107 passed/0 failed/123 filtered。该 artifact 的完整导航、生产 Profile 5 phases、Agent 原生链与 canonical 完成、site-data 维护 view 回归均 EXIT0。首次加载 Pause 的旧取消覆盖已修复并通过完整导航；Profile 重开、应用重启与不同 Session 隔离也已通过生产 Host，未使用 Cookie 复制、Keeper 或人为延时。DNS 分支明确输出 dns_failure_observed=false（committed/other/WebView2 code 0），属于本机环境限制，不用整套绿灯替代 DNS 失败证明。用户下载 fixture 仅证明 chooser/cancellation lifecycle，实际文件字节由独立 Agent 链证明，未宣告所有用户传输生命周期通过。后续 Popup/presentation 修改不借用此 artifact 的原生通过状态。后续正式主 GUI 与真实网站的已测范围见下文；当前Windows/shared本轮定向收口已完成，macOS按用户要求暂缓且未计原生通过，不能宣告整个 Goal 完成。

最终运行证据集中在 `nomifun-browser-conformance-final-5f70e92f-8fc7-494b-80a4-e7c9e27213a9/`，来源记录为 `artifact-closeout-final-profile-guard.json`。以下五次执行各自的前后源码快照均包含 253 个路径，现场独立比较差异数为 0；结果与 exit JSON 同时核对，未依据消息中的预计结果记 PASS。

| 最终运行 | 实际结果与范围 | stdout 日志 SHA256 |
| --- | --- | --- |
| `closeout-final-native-navigation` | EXIT0；87 次独立 HTTP 主文档请求、完整导航/暂停/恢复/竞态/有界脱敏诊断及最终原生截图 | `3A61B5CE0615A944B0C9597B6E0D4824D62F58C10115FF4E7F4D8F86DCFB9D79` |
| `closeout-final-profile-persistence` | EXIT0；生产 ProfileStore/Desktop Host、5 次独立应用阶段、重开/重启/Session 隔离及 override 拒绝 | `85FC3CE6218B07D1D1261E621ABA459EB7AA3D75CC1E9B424D6041989462C276` |
| `closeout-final-agent-terminal` | EXIT0；精确 Session/Operation 的 completed seq 142、11 次本地脚本模型调用、输入/上传/下载与 cleanup | `6DEC8EADA08EC816F9EF35E585E627BEB0EA08E6B9B21E29ACCDE07296C49EC7` |
| `closeout-final-site-data-regression` | EXIT0；生产用户 ClearSiteData 命令、维护 view 原生 clear/close、双 origin 与 active writer、其他 Session 不变 | `826B5909A2522ADE48549C08C885FD79762B77F0325828329938364803F427E1` |
| `closeout-final-windows-unit` | EXIT0；production binary Windows 模块 107 passed/0 failed/123 filtered | `5025C3EDD0B62A0290A8F0FA781F63F5F2A072547298657C57734560DED297E5` |

每项均保留同名 `.stdout.log`、`.stderr.log`、`-exit.json`、`-launch.json` 与源码快照；四个 native 场景另有 `-result.json`。生产 unit 的 stderr SHA256 为 `BC5448C203BA0E5E631B14D5F78D4387F899994F9CC6A30BEBA45D7483F44C35`，导航 stderr 为 `00CC1163B4A2CF5B0D416CAA5D24BFEEE80969B006458D4F963783EE11F047B5`；原生 stderr 记录具体 TLS/状态过程，stdout 与退出码不能单独替代它。

**后续 Popup/presentation 纯代码验证**：用户物理 Escape 终止界面操作的当回合，未再启动 native/window/input fixture，也未构建或覆盖运行中的 main。更新后的 actual production binary Windows 模块为 **112 passed/0 failed/123 filtered，EXIT0**；`closeout-popup-and-no-content-windows-unit-retry.stdout.log` SHA256 为 `F177DF1D9753B4EAC261C132A9624B51221C72061E441F3173C8B6E8E46E27B3`。随后只编译 example，EXIT0、artifact SHA256 为 `64CB7E6F0B426A16FC7C6A7BA32AD8B55A21A6E2010FAD244B95FC567DD9397C`，当时没有执行。两次前后源码快照各 254 个路径、差异均为 0；汇总 `popup-no-content-pure-validation-summary.json` SHA256 为 `73FB58D88CF4864FF53F4925553A46A2982D5CC3F8B09724976852ECB4D745E2`。后续执行仅在用户明确恢复授权后进行，不改写当回合停止工具的范围。

新 popup Action 没有 NavigationId/DocumentId，保持 `navigation_sequence=None`、`document_generation=None`。新增 unit 首轮 111 passed/1 failed 的失败来自误将此未关联样本期待为父页最新文档代际；修正采样和断言后才取得上述 112 项通过，没有放宽共享 trace 规则或 Pause/请求计数要求。准确父 Core、当前目标、Pause ACK、文档 nonce 和 0 GET 由独立原生 fixture witness 核对；这些 witness 在下述用户授权恢复后的原生批次取得实际证明。

**用户授权恢复后的原生回归**：用户随后明确答复“继续原生与界面验收”。64CB7E6F artifact 的完整 navigation-only、managed-popup-only、agent-only 均实际 EXIT0；root 独立读取 result/exit，并比较每次 254 个来源路径的前后快照，差异均为 0。来源和证据集中在同一根的 `resumed-popup-mask-native-summary.json`。

| 恢复后原生运行 | 实际结果及范围 | result SHA256 |
| --- | --- | --- |
| `resumed-popup-mask-navigation` | Pause ACK 后可信 named/blank popup 请求被拒绝，GET 增量 0、tab/view/父 nonce 不变；Resume 与明确导航的正向 popup 通过；生产 writable about:blank 实际可见且可输入；首次附件 None 双隐藏但真实 Save chooser 出现并取消退出；恢复 pending None 双隐藏，真实 commit 后同槽可见且可信输入通过；保留文档 pending 可见 | `00FB312F262E7CD09069DB388FF48CB2D5F157F2B8E4C803D5D1C033E84F0906` |
| `resumed-managed-popup` | 正式 Host managed popup、命名复用、配额、用户/Agent 输入锁、close 与 cleanup 回归 EXIT0 | `77BA4D241D69ECC0E1A395D0EAE573AA4E73662930FBA44D6D602F68C7FC5329` |
| `resumed-agent-download` | 实际原生下载字节与 publication、target-blank cleanup、上传/输入恢复及精确 canonical completed seq 142；11 次脚本模型调用 | `F6534E0F169320396C66610E6A7C4A5162777FE69B3174F65C9A2CE92463F3E1` |

首个附件用例只证明选择与取消生命周期，不称保存字节通过；实际下载字节由独立 Agent 链证明。DNS 仍未取得预期 DNS 失败，不借整套通过补出该分支。64CB 中“先 Pause 完成、再点击”的证明没有被扩大为在途事务结算，后续以准确 claimed/published stage 单独取得下面的证据。

**在途 popup 与首次原生 prompt 的补充证明**：example artifact 为 `34DDD55E8B3D95B7524D6708F24CAB3094C4AB82516A06C876FD484EF6D2CE9D`。`claimed-popup-pause-native`、`initial-dialog-visible-input-native` 和 `claimed-popup-initial-dialog-windows-unit` 均实际 EXIT0；各自前后 255 个来源路径已独立比较，差异为 0。两个 native result、exit 与实际 SHA 均现场核对，汇总 `claimed-popup-initial-dialog-native-summary.json` SHA256 为 `694D2D404F366CF864EC68043F9ED3CD672B940837B0973D8F8A8A60F38820E9`。

| 补充运行 | 实际结果与准确证明范围 | 证据 SHA256 |
| --- | --- | --- |
| `claimed-popup-pause-native` | 正式 Host 已 claimed/published 候选、binding 前，由 exact opener 单次 debug oneshot 固定时序；无 sleep。native hold 先于释放，deferral 在仍持有 candidate 时结算，准确 native Close ACK 和两 registry 清除先于 Pause ACK。可信 window.open 返回 null；Resume 后旧 GET 增量 0、新 popup GET 增量 1，父 nonce 保留 | result `D0E3E0C7CF13215F28A3750C6650D27DDFC752863B7656E3C91CC4589938D4FB` |
| `initial-dialog-visible-input-native` | 首次脚本真实原生 prompt 回复后，同 native slot/document 且 CurrentDocument，controller/HWND 双可见、原生用户输入 enabled、实际输入值与 focus/可信输入通过；layout_or_show_after_reply=false，没有用后继布局补可见性。完整 dialog/navigation/close 矩阵同时通过 | result `334DF746EBEDAC6E255744D954723EDFFDC9EF2B4F9E5DDBD4CFB9B97D92F36E` |
| `claimed-popup-initial-dialog-windows-unit` | actual production binary Windows 模块 112 passed/0 failed/123 filtered，源码前后稳定 | stdout `3E52E0280C0236F729DA0602EBF2C190EA905F58FC6608215BAF64456F433203` |

该 popup proof 绑定 claimed/published、binding 前的准确候选阶段，不是全部调度交错的穷举。原生 NewWindowRequested 的文档关联仍 unavailable，不伪造 NavigationId/document generation。stderr 保留 stale request 与 controller 结束后的 policy restoration warning；通过不表示没有 warning。后续 9C6F 的用户启动、功能 GUI、最低窗口、Session 切换与百度/Bilibili 的实际范围见下文；该原生批次本身不替代 GUI 或 macOS 验收。

**9C6F 历史 main 构建、第二 Session 准备与用户启动**：该批源码的 `nomifun-desktop` main 已用隔离 GUI 测试 overlay 完成实际 `cargo build --locked --offline -p nomifun-desktop --bin nomifun-desktop --message-format=json-render-diagnostics`，EXIT0。磁盘 executable SHA256 为 `9C6F261A219E0BA81910F425AC25168DC2D42F33EF361862AF278BF0546F1431`，来源前后 255 个路径独立比较差异 0；构建和后续 app 启动分别记录。`final-native-and-gui-build-summary.json` SHA256 为 `EBEE4923E4A61A32C0B3C85777B4660EC47814BD0352F3AA224B2F4D49043244`，同证据根保留 `browser-remediation-final-gui-main` 的 launch/exit、stdout/stderr 与来源快照。

GUI 隔离根的 `gui-evidence/secondary-session-preparation.json`（SHA256 `0A0DCB46D257F1C42317E8FC910CE808B72D52D4A523DB226E2EB62BB3B12E9B`）及实际 READY stdout、exit 文件均核对。正式 API 创建第二 Session `01a13543-dbf4-7150-9d1e-fc42706eb8cc` 成功、EXIT0、stderr 为空，准备进程已正常清理。第一 Session 不变；第二 Session 的 Browser resource bindings/grants 均为 0、Turns 为 0，唯一启用的 capability 为合法非 Browser `agent.tool-discovery`，未启动脚本模型服务。此前 fixture 的校验/编辑路由假设已修正，失败尝试不计为准备通过，也没有据此推导生产 Session 缺陷。准备不是 GUI Session 切换证明。

此前隔离启动工具请求在执行前被自动审批拒绝：`rejected: blocked by policy`，没有更细原因。`gui-evidence/launch-policy-rejection.json`（SHA256 `0FF86319F3D95681C014285F3FF22A3FFDA333DC1F12B6FDE9B2AE0DD2676155`）中的 action_executed=false/app_process_absent=true 是当次历史状态；没有换工具接口绕过。用户亲自在新的 PowerShell 设置 owned DATA/WORK 后启动，解除启动等待。此前 AmpersandNotAllowed 来自多行粘贴合并缺分号；修正为分号分隔单行后窗口出现，不归因于产品 startup/DLL。`NOMIFUN_LOG_DIR` 输入仍无效，实际日志根为 DATA/logs；CLI 固定 argv 和 Sky 缺少隔离参数的边界没有被修改。

用户启动后的 fresh 只读核验：准确工作区 executable、SHA9C6F、PID34236（本地22:25:07启动）；owned data 的 port.json 为59127/PID34236；实际 /health 为200、status=ok、version=0.8.6。`gui-evidence/main-9C6F-launch.json` SHA256 为 `05B156643104D503EDF2AB2733F5804CDA4D7DAD84B32794D62707C5BE929077`，记录 user_launch=true、agent_launch=false、launch_policy_bypassed=false，Vite HTTP200 与 owned DATA/WORK/logs。GUI owner 随后取得唯一真实 NomiFun 窗口25629038，外框1282×834，选择第一 Session 并通过无 Browser grant 的入口打开 owned51378/popup、main document请求1。fresh `gui-evidence/9C6F/popup-parent-baseline.json` SHA256 为 `5267F6AE016570BF6010CF9DA0BE0FDEBF0C0133C93C5ED92A6E1BC6EFA0E603`；该文件的窗口/Sky观察与截图元数据已核对。这证明启动及入口基线，不将外框尺寸当成最低内容视口证明。

**9C6F 正式主程序的功能 GUI 证据**：已只读核对 `gui-evidence/9C6F/` 的观察时间、同一窗口25629038、截图元数据、可访问性与独立 fixture 计数。以下是实际结果，不借原生 fixture 或旧 AA94 替代；截图文件名本身不构成通过条件。

| GUI 场景 | 实际结果与范围 | 具体证据与 SHA256 |
| --- | --- | --- |
| 原生人工输入 | 第一 Session 的网页输入回显为 `GUI9C_NATIVE_TRUSTED`，独立 HTTP manual witness 为 trusted=true，父文档保留 | `9C6F/native-input-trusted.json` / `native-trusted-witness.json`；witness SHA `84A4E2CC45086145DD50B08052A95DBD9424ECB03463DB7CC46878052CCC8027` |
| 父页 Pause 后 named/blank popup | 实际 UI 显示暂停并保留父页输入；关闭已有 named 子页后再次点击及新 blank 点击均无新子文档。三个 count（两种 Pause、Resume 后无新点击）均 stable:document=1 | `pause-blocks-closed-name-popup.json`、`pause-blocks-new-blank-popup.json` 与各 count；两 Pause count 和 `resume-does-not-replay-popup-count.json` 均 SHA `4B1F2352BCDBB5B70E2A0633E0EFCC5EFD45349EB3B01F833973A1F605953ED8` |
| Resume 不重放、新点击正常 | Resume 不增加旧请求；之后一次新 blank 点击出现真实子页，stable:document 从1到2、delta1，关闭子页仍保留父页 | `resume-fresh-blank-child-created.json`、`resume-child-closed-parent-retained.json`；fresh count SHA `12D164A68655AD7B8B83FA633988ACF7911434FCF36FBADF872351396F84E4CC` |
| 首次 Failed/None 后恢复 mask | 首次连接拒绝显示宿主失败提示、无可访问的原生 fixture 文档；120000ms恢复 pending 显示加载提示且仍无原生文档，fixture slow请求1；同槽结束后出现真实/slow文档和可写输入 | `refused-first-content-none.json` SHA `2D84479B504C3850B050EA8921E1189C579145A2081F361862E1D50B86C402AD`；pending SHA `4483038CF4E58900F505D131651D501A74E496E9C0FC5DFFDEC97884FE46F965`；finished SHA `5F46BAADC1144D9572CE2C0E0AE64D02DDCD9F8EFE3B60FFED20AA48F478CFCF` |
| 两 Session 基础切换 | 实际面板从S1切到S2，S2最初为空浏览器、再打开独立/stable，地址栏可写；后续最低窗口的S2原生输入取得单独trusted witness。没有把基本切换记作强竞态 | `second-session-empty-browser.json` SHA `183385EEF98562BF7071C0B46719A152DAF0BE3EB2357937452C521DD78B7A51`；stable SHA `2C23EB0738695DC718CB70D55BE1A1825FF8919572B20C249FBD9D873B13ADE3` |

双 Session 的证明边界必须保留：S1 地址/加载场景为60000ms，`session1-command-pending-before-switch` 捕获在 `2026-10-13T14:45:43.642Z`；S2 `second-stable-while-first-pending` 捕获在 `14:46:53.487Z`，相隔约70秒。不能仅凭后一个文件名宣告旧请求仍在途、old finally 不解锁新busy gate 的强GUI竞态已证明。两个文件 SHA 分别为 `1D9895391A5E79B121BEBEA53EA8AEF18AFE55DF58B34643D8462DB7EF5FA0BA`、`B328D3905285828C187F119972C5FE6136427BAB25941E2238DF34AF05B251BD`；对应同步token/生命周期合同仍由定向UI测试证明。

**最低窗口实际失败与修复后回归**：第一次外框882×602场景中，root现场发现Browser完全被挤出，`root-closeout-9C6F/minimum-outer-882x602-after-layout.json`（SHA `E15012024D359EBB82A90DBC833E9C778D788516A79BFDCB15A95AC5824CA263`）保留为历史失败。共享renderer随后修复，以下当前Vite实机批次已核对；源/客户区880×600与外框882×602分别表达，不以外框值自行推导viewport。

- `minimum-outer-882x602-final-layout-settled.json`：Browser文档、地址栏/菜单实际可见，截图882×602；SHA `5D9A4758394471301C4931C62FC191D3D34E30788729DB875FC8FF187B452EF8`。
- `minimum-880x600-trusted-native-input.json` 与 `minimum-browser-menu-controls-visible.json`：S2回显 `GUI9C_MIN_NATIVE`，actual manual trusted=true；菜单缩放/复制/暂停等入口可见。menu SHA `0A663E60CAB05F8270B6B00CD630B20FF1C7B7A059C70E0D2572D8431B4C07BB`，after-input count SHA `C2F480238816468EECFEABEDDA9F0A7738EE062FAFE19B4134CF06BD8FD8B6F8`。
- 显式展开主侧栏时Browser hide/detach，globe重开后原/stable与输入保留；`minimum-reopened-preserves-native-input-document.json` SHA `8699739ADF48BF1599D6BA288950A94D79A317B4902A819EFB28CEF0878790F1`。
- `wide-restored-sidebars-input-and-document.json` 记录最大化后1920×1032、主/Session侧栏恢复184/300及原divider约1000，文档/输入保持；SHA `EF3509C10AC4D0ECCE556C1CD233327279B9F48F7D34EEF16B55697DBEA5408C`。before/after-input/reopen/final 的 stable:document 均3、delta0，S2 nonce始终相同；final count与after-input SHA相同。

该最低实机批次证明可用浏览与基本布局恢复，不把聊天输入可达写成active Agent Stop实际点击：Session零Turn。额外916拖拽在执行前被Sky以outside bounds拒绝，未计916通过；这些是未覆盖范围，不增加发布gate。Host9C6F与当前Vite renderer/最终9个冻结production文件的source_drift=[]在完整GUI汇总中分别记录，不代表bundle/installer。最低窗口本批不含网站，后续网站结果见下文；macOS按用户要求交接。

**9C6F历史网站静置与交互结果**：网站证据位于 `gui-evidence/root-closeout-9C6F/`，导航报告经产品Copy菜单、本地地址栏UI读取，没有直接读取clipboard或认证数据。开始/结束观察和完整安全trace已核对，各站点/操作分别记录；本节不代表后续C212的当前结论。

| 场景 | 实际结果 | 证据与SHA256 |
| --- | --- | --- |
| 百度主页静置 | 95.158s，15:22:38.836Z至15:24:13.994Z，窗口内输入命令0；两端正常主页，无可见challenge/版本拒绝/原生错误/loop提示；seq4/docgen2、Finished/200/CurrentDocument | `baidu-home-static-window.json` SHA `FD70634A3BA292F9BFB1DC4BF8682E1807DF08545E71EDA3F99F534454BC425C`；start/end与`baidu-home-static-navigation-diagnostics.json` SHA `6EC3916858780210441ACE7A5F9EBADBC27E6C724A0FAE4AB43FEC05C11EAE5F` |
| 百度搜索静置 | 真实native输入通用“浏览器”并点击搜索后，普通结果114.721s静置（15:27:49.628Z至15:29:44.349Z），输入0。搜索使用同文档SPA，native content_url=/s而仍seq4/docgen2，无新主文档事件；原200是主页尝试，不证明搜索全部HTTP状态 | `baidu-search-static-window.json` SHA `0FC49D996512FB04993BB0824E1E49A1D45BBD5601F5876DFA8B0280998369BD`；search start/end与诊断SHA `5DDA6931163D4A91E97FDC0AB8127196AA86EBB68741FA50E7BBD39E535B543F` |
| 百度首次Back | **FAIL**：页面实际退到前一/s历史项，summary seq5 Requested/Retained/0%，新增只有event13/dispatch3/back_forward，缺少native Starting/Completed，UI长期loading。生产owner复核指向synthetic Requested后navigateToHistoryEntry ACK及SourceChanged(false)只更新URL、不结算；此为9C6F历史FAIL；后续D74源码修复与60C/72E定向native已通过，95E5对应同文档Back/Forward已实机回归通过，保留9C6F历史FAIL | `baidu-back-persistent-loading-diagnostics.json` SHA `BD6B2005052D3FD4429996114B1A77564EF44E69BE1AD6B1719F296838F49670`；旧`spa_url_and_history_events`只验URL/历史，不覆盖load phase |
| 百度后续full Back/Reload/显隐 | 第二次Back走full native回主页，明确Reload、hide/reopen正常；不抹去首次SPA Back失败。同Tab最终完整49事件及计数已取得，见下文 | `baidu-native-back-second-home-finished.json` SHA `F9809541A28A8B456EE9383CC273CF9E7271FE233115181B41720ED173DF5C16`；reload SHA `BCC0AF43B75A188B1610B7A2E6BDE2A5C66F8E6425F846A2B8A3BD6FA6E793BC`；hidden/restored文件分别保留 |
| Bilibili主页 | 136.237s，15:41:55.144Z至15:44:11.381Z，输入0；两端公共主页正常，仅login promotion，无可见challenge/版本拒绝/原生error/loop | `bilibili-home-static-window.json` SHA `3EE34388976027D5073D0B346BA6F9F102DB19EF433D5503D1C2BE962DEF0134`；start/end另存 |
| Bilibili搜索子标签 | 真实输入/物理magnify生成第二tab；seq1 Provisional/None/0%、Requested+Started/page_navigation/user_initiated=true，未观察content/commit/completion；原因未知，不判验证码/风控，不与SPA Back同因。Response只在Completed采样，缺字段不证明HTTP未返回；人工Stop/close，无auto reload | `bilibili-search-popup-pending-navigation-diagnostics.json` SHA `C1E753DEB7B649A7F98274765C0352CC8D2E7F79DF65D10C08C49419216C7084`；supported/lossfalse/drop0、user0/page1 |

百度主页/搜索trace均degraded、routing_lost=true、drop0，累计user2（先stable再百度）、agent0/page0/unknown2/redirect0；一条百度host Requested及一条seq4 native Requested，随后commit/response/finish，无后继主文档导航。unknown保持unknown，静置只证明主文档不循环，不称网站全HTTP无请求或完整HAR；原生身份Chromium155/Edge155。读diag产生dirty draft，原生/s与草稿homepage不同不作为URL同步缺陷。9C6F首次SPA Back失败保留，不计完整网站交互PASS；D74源码修复及后续定向native通过，不补写9C6F为PASS，95E5对应同文档Back/Forward已实际通过，本轮880×600原生输入、显隐与宽屏恢复也已实际通过。Bili搜索只观察到Requested/Started及Provisional/None，Response事件本来在NavigationCompleted才采样，缺少事件不能证明HTTP未返回；原因未知，不混同无NativeStart的SPA Back，也不归于page-filter排除/worker等待。人工Stop后标准Cancelled/None，再close子标签返回parent，输入仍保留，无自动reload。

**9C6F历史GUI收口**：Bili parent跨文档Back回百度、Forward回Bili主页、明确Reload与hide/show均有实际finished/正常观察；文件为 `bilibili-parent-native-back-baidu-finished`、`bilibili-parent-native-forward-home-finished`、`bilibili-parent-native-reload-finished`、`bilibili-browser-restored-native-home`。`final-9C-main-ui-left-normal`记录最后正常界面，随后GUI输入通道释放给native补证。该批次Bili搜索非正常结果PASS，本地同origin popup范围不覆盖cross-site不等于另一个已证实缺陷；后续Windows修补见文首A/B/A。

同Tab最终诊断 `baidu-bilibili-parent-final-navigation-diagnostics.json` SHA256为 `6B0B683BD4CB10778324D1FE36DFD453B26B78AC8BFE473A89E031EC27E66CC0`。树Value仅7172chars前缀；通过产品Copy菜单与本地地址栏UI的selected_text取得17690chars完整JSON，parse成功，捕获于 `2026-10-13T16:02:23.698Z`（本地10/14）。完整49events/drop0，load seq17/docgen8 Finished/CurrentDocument/200，user9、agent0/page0/unknownNative8/redirect0。九个显式Host命令依次为stable、百度、SPA Back、full Back、Reload、Bili、Back、Forward、Reload；八个native Requested数量/顺序与八次full操作一致，SPA Back只有dispatch。显隐后没有额外Requested，没有重复Host派发；sourceunknown8与degraded/routing_lost=true仍保留，不能写成具名host-native绑定或全HTTP零请求。

完整范围汇总 `final-9C-main-gui-acceptance-summary.json` SHA256为 `F7DC2177849386B076A6977459ABF9494A6140079ED19191C84544AD4EA71441`，记录于 `2026-10-13T16:15:19.756Z`，包含9个冻结renderer文件一致、各项实际PASS/FAIL/未覆盖范围。任务报告保留10/13文件名。没有用单次访问估计风控/故障率、推导用户Mac现场根因或承诺零风控；后续History修复、native及E584构建证据单独记录如下，不改写9C6F本批实际FAIL或最低窗口的冻结renderer范围。

本轮修正中英循环提示，主要恢复留在内嵌页面内的暂停/检查/恢复，Pause 只作用当前标签、其他标签继续运行。随后 `navigationPausedHint` 改为“暂停会保留可用页面内容” / “Pausing retains usable page content”，避免首次加载 None 也声称当前页面可见；仅改字串，未改 renderer/Host 逻辑。9C6F 的 255 路径构建快照不包含 locale，Vite dev renderer 使用当前字串，不借 main hash 或字串修改记作新 GUI 通过。既有系统浏览器显式入口保留。locale JSON/计数占位符检查、最新桌面 UI 边界 2094 sources/min880×600 与当前 browser boundary 均实际 PASS。

**同文档 History 修复、定向原生验证与 E584 构建**：以下结果在 `2026-10-13T16:57:17.549Z` 前已取得（本地10/14），与9C6F真实GUI批次分开。冻结生产 `windows_navigation.rs` SHA256为 `D74E74C22BCC3067371ECEDA13AB16B206D15DF72D2D469ABE111124FFEF354A`；准确修改范围已审阅。`same-document-history-native-summary.json` SHA256为 `AA62D42A8C64A3B01F1FF5A64FDA9BD1CA08CDA8B616018BA7C764D289AEBEA8`。已独立读取各raw result、exit、日志及前后来源快照，不以汇总预计结果记为通过。

Back/Forward 现在推进当前控制 fence 并记录宿主 dispatch，保留真实文档的阶段、进度、问题与原生绑定。同文档历史切换没有新主文档尝试；只有真实 NativeStarting 才创建新的 Loading。`Page.navigateToHistoryEntry` 的空回复不证明完成，`SourceChanged(false)` 只更新元数据，不通过URL、JS或空响应补造Finished；没有增加第二份命令/文档ledger。公开 `load.navigation_sequence` 可在无新原生尝试时推进，精确取消与trace仍关联原NavigationId及其attempt序号，迟到来源不能确认时保留Unknown。

| 实际批次 | 结果与准确范围 | raw result SHA256 |
| --- | --- | --- |
| `same-document-history-production-unit` | production binary Windows模块120 passed/0 failed/123 filtered，EXIT0；前后256路径0差异 | stdout SHA `0B8AA5835CB427B452CB0AB935C524ADFB393A6FF48E34793B4E4638ACDFF563`；exit与同名来源快照保留在conformance根 |
| `same-document-history-native`，artifact60C | history-only，EXIT0；Back、Back、Forward、Forward四次均Finished/Ready/CurrentDocument/100%，docgen与nonce保持、HTTP主文档delta0、无新增native Start/Commit/Finish、每次宿主command delta1且control fence推进。Agent run-lock拒绝人工历史操作、历史前stale Reload被拒、当前fence显式Reload、跨文档Back/Forward真实NativeStarted/Finished均通过 | `C6D561860A6B673FDB57885A698E048F4E39402275C269813258F0AA658363E7` |
| `history-beforeunload-stay-native`，artifact60C | dialog-probe-only，EXIT0；这是generic Navigate→BeforeUnload→Stay，不能替代History Back→Stay证据 | `334DF746EBEDAC6E255744D954723EDFFDC9EF2B4F9E5DDBD4CFB9B97D92F36E` |
| `history-settled-command-stay-native`，artifact72E | history-only，EXIT0；真实History Back命令已回复后用户选择Stay，原生BeforeUnload false deferral完成；原文档Finished/Ready、docgen和nonce保持、目标主文档HTTPdelta0、无NativeStarting或伪造Finished。`control_dispatch_rejections_observed=[]`，不补造已结算命令的取消关联 | `B3A4CA6CD9CDD0BEC00A871E823C7F7DE1B38AF12BF80DBFF4409A7B8791BAFC` |

三个native批次前后各256路径，独立比较均0差异。60C artifact SHA256为 `60C528473215AAECC2E0E727A78390E27B186ADC6FED8B720DF68C9A300974F3`；独立History Stay增补的72E为 `72E65F25EA0CCB8FF16E08793908FC098D58049F5763111B456BFA8284CEFD06`。跨文档场景本次HTTPdelta1是观察值，缓存请求数不作为真实NativeStarting/Finished的替代。三次exit/result和对应stdout/stderr、来源before/after均位于上述conformance根，不扩大为完整导航矩阵或真实网站GUI。

用户正常Quit旧9C6F后，原PID34236不存在、59127无listener已现场核对；不声称全部WebView broker退出。新main实际build EXIT0，耗时19.73s，磁盘executable SHA256为 `E58484E937E40E6444A4F02FF236426140F94F438721F639BEFEEB2A28F63E4A`。`same-document-history-final-main-summary.json` SHA256为 `1ABD55F9919341CF169553D3F9E2D1F7CA2A0F0B1A16D6B5FAF0B9625E4DCCF3`，其 `same-document-history-final-gui-main` 来源前后2577路径、包含2320 renderer路径，独立比较0差异。构建stderr仍有warning，不描述为无warning。

该E584回执仅证明当时构建，conformance本身没有启动app或执行GUI输入。后续E584曾实际启动并正常teardown，见下述review收口；当前接续版本为95E5，已取得对应History实际GUI通过，880×600原生输入、显隐与宽屏恢复已在本轮实际通过。新编译不改写9C6F历史FAIL，也不把9C6F+其冻结9文件Vite的最低窗口PASS倒算成E584GUI或bundle/installer证明。Bili搜索子页原因未知及Mac暂缓保持原范围。

**Review收口与95E5历史接续**：用户已确认保留 `tauri`、`tauri-runtime-wry` 两份局部vendor；external_page的创建前隔离边界不改，精确补丁与许可证仍见 [vendor说明](../../vendor/TAURI-PATCHES.md)。该轮review提出的两项P2已修复并冻结：

- 只有Browser注册布局空间预留及侧栏展开后的dismiss，回调只隐藏/detach Browser，保留其页面与输入所有权。Preview沿用原有非破坏性的侧栏折叠策略，不将清空tabs/未保存编辑的 `closePreview` 交给布局；Browser/Preview切换时交接侧栏管理。组件测试的几何来自Happy DOM fixture，不当作新原生像素证明。
- UA是可选诊断：原生属性接口不可用、读取失败、空值/无效值时identity为None，只记录固定reason，不阻断普通页或popup，不猜UA、不截断成测量成功。UDF实际目录、Profile持久性及InPrivate匹配仍经必须成功的inspect/verify路径，失败仍拒绝目标导航；没有将存储身份校验降级为可选。

已独立核对conformance根的 `closeout-ua-diagnostic-windows-unit-20261014`：实际production Windows模块 **121 passed/0 failed/123 filtered，EXIT0**，前后256路径独立比较0差异；stdout SHA `2CFEBF140EC26EBDEE5CE6E8C0B74993EB151A9839F5CE5A6D73C99D51E743A6`，exit SHA `DC3DEFAC53A119715FD6E799DAD20DB6707A60E38A3D914847DD367B5840A913`。此121项是后续UA诊断补证，不改写120项History或更早fixture的artifact范围。

`closeout-review-final-gui-main-20261014` 实际build EXIT0、43.48s，前后2577来源路径独立比较0差异，含2320 renderer路径；磁盘exe SHA为 `95E5B839FA12F0D03316215EBDFE649D4DAEB46D80E1E30CD8635E2C42524EC7`。artifact回执SHA `83AC5DD085F90BE5BDDDE757C29DEDA430E4D78DEE260852B4EEAAFDCAF14DE4`，exit SHA `6BA71067AECCE06B33C08F4DAA07DA0CF49F51F1BB7B5595297ACCB0EC258D5B`，stdout SHA `3BA473D073AE33AE245FAF79FE70F86282D5F91B3BA60DF065C35F6AFDE8D87E`。stderr保留12项warning，构建通过不表示无warning、GUI或bundle/installer已验收。

E584旧main后续曾实际启动，不再沿用“待用户人工启动”的状态。其退出与Vite恢复及95E5真实GUI结果见下述实际回执，构建和启动事实本身不计功能GUI通过，不新增机制、矩阵或用户退出/启动要求。9C6F历史Back失败、其冻结renderer最低窗口证明、Bili搜索未知原因和用户暂缓Mac的范围保持。

本轮focused检查汇总为GUI测试根的 `review-closeout-validation.json`，SHA `AF3820116AC0642D0DE37077AFF9511AF7C55E2C7D0C9914E58234F074BC6052`，记录于2026-10-14T01:19:36.1635809Z。该汇总明确区分来源：Root实际exec的typecheck EXIT0与browser boundary PASS；UI owner报告的Space6项/56断言、Resize6项/100断言及desktop boundary2095 sources/min880×600 PASS。已读汇总，不将它描述为这些UI命令的raw log。汇总还记录95E5准确PID38196/backend59312与frontend均HTTP200、gui_status=in_progress；这只证明进程/backend接续，后续真实窗口功能已在下述最终manifest按本轮范围收口。

**95E5同文档History的真实主GUI回归**：在canonical第一Session、PID38196/window26021452、准确95E5主程序中，真实百度主页输入“浏览器”并Return得到普通搜索结果。首次Back从搜索历史项tb_enter回到tb_pre，随后Forward恢复tb_enter；两次fresh窗口记录均可手动操作、无持续loading。分别为 `gui-evidence/review-closeout-95E5/baidu-same-document-back-settled.json`（2026-10-14T01:23:02.172Z，SHA `EEDB3F46B5CB1B8D59EBD86DE977F9631B6817F793B52874229962377F6C3D2D`）与 `baidu-same-document-forward-settled.json`（01:25:49.066Z，SHA `DEB43836DA96AA5549FEC214C0F82B75C07EE88E04C4937893BEBC7A1D1DEBD0`）。这闭合9C6F首次Back长期Requested的对应实机反例，不改写9C6F历史FAIL。

Back后的完整 `baidu-same-document-back-diagnostics.json` 经产品Copy→未提交本地地址草稿→UIA selected_text取得3842chars并成功JSON.parse，已独立读取，SHA `007AC4C32571526E520A47AE58D85DC8F4EEE2ADCA93D3DECD279D3F94BCEED7`。load控制序号3/Finished/CurrentDocument/100%；trace7events、唯一原native attempt2/docgen1，Back仅host dispatch2/back_forward，无新增native attempt。user_commands2为初始Navigate及Back，agent/page0、unknown_native1/redirect0/drop0、routing_lost=true保持原义；保留200来自原native主页，不当作Back产生新HTTP响应。没有提交诊断草稿、不将主文档trace称为全HTTP或完整HAR。本轮最低窗口与原生输入的实际结果及来源见下文最终汇总。

**95E5本轮Windows/shared定向GUI最终收口**：`gui-evidence/review-closeout-95E5/final-95E5-gui-closeout-summary.json` 记录于 `2026-10-14T01:41:00.9640397Z`，SHA `31262E2825F2313B6B699E1234F6B2B702E0418ADBE87E87D094E82D382E4BB5`。已实际读取并核对其52个附件hash，无不符；下列范围为本轮真实主程序操作，不是编译或Happy DOM结果。9C6F历史FAIL及其原renderer证明保持原样。

| 本轮实机范围 | 实际结果 | 证据 |
| --- | --- | --- |
| 百度主页/普通搜索静置 | 主页86.241s、真实原生输入及Return后的搜索88.385s，两端正常、无观察到验证码；时间差已由四个快照UTC独立计算 | `baidu-home-initial` / `baidu-home-before-search`，`baidu-search-initial` / `baidu-search-after-static`；搜索结束JSON SHA `C0A0D245DF8367B2EFCEA99824102B06BEE853D05967695F9007E47E2E66F271` |
| 同文档Back/Forward | 上述Back完整diagnostic与Forward fresh窗口，均可手动操作、无永久loading；控制fence与原native尝试区分保持 | Back诊断SHA `007AC4C32571526E520A47AE58D85DC8F4EEE2ADCA93D3DECD279D3F94BCEED7`；不是全HTTP/HAR或风控成功率证明 |
| 880×600与可信原生输入 | 准确外框882×602，客户区合同880×600；Browser/地址栏/聊天输入可见，页面输入 `GUI95E5_MIN_NATIVE` 有实际截图及HTTP trusted=true witness | `fixture-minimum-880x600.json` SHA `12432CD02D3A2515E54C5786FF76FF89454C2788A65F1664E9234A38FD05BF1C`；`fixture-minimum-native-input.json` SHA `EAAB485F3F9C99147A288141B7E13C9C8940EAB9500605DE71C991BD2EC1F66F`；已查看对应原生截图 |
| 收起/重开/宽屏恢复 | 重开settled后同nonce及输入保持；恢复1920×1032与184/300侧栏偏好；`/stable:document`累计始终1、delta0，无显隐额外GET | `fixture-minimum-reopened-settled.json` SHA `D8B6C1ACF95611A70736061651BC91635C3DE9E918AB813A2AA5C8DFC86FC0B5`，`fixture-wide-restored.json` SHA `33686DDF6925C729B04B338B6C13282BD450652EEEFC944700FE105CB4BAADA9` |

before-hide、after-reopen和final三个独立计数JSON内容/hash相同，SHA均为 `84F288BCF80918D8E418615CA8C16A629881408B360C26777F5B73A3BF800B63`：loopback owner18544，stable文档1，manual witness的nonce为 `e0a6d0f4-8f6e-45c2-82da-8771046aedba`、trusted=true。计数和截图分别证明请求与可见输入，不用AX是否暴露每个原生value替代HTTP可信witness。GUI后 `gui-source-verification.json` SHA `ED605A2EC43246AD05C8968CC6245DF5E21F1E4CF2DA9A779F7968E5C159515B`，记录2577来源相对build-after漂移0。最后窗口1920×1032、Browser真实页保留运行，manifest核对main38196/backend59312及frontend200；这些是最后观察状态，不保证后续PID一直存活。

E584的localhost拒绝连接属于独立历史开发服务事件：GUI测试根 `service-recovery-20261014.json` SHA `9F588FB2A0016130F1779A980F17C4451B49146472068958DBA4A066828D5224` 记录E584/PID14512/backend60728仍存活，5173无listener，恢复Vite后frontend/backend均200，恢复未改源码；原Vite退出原因没有保留exit/stderr，仍unknown。随后owned `data/logs/2026-10-14.nomicore.log` 在UTC01:01:10记录ChannelMessageLoop停止、channel queue drain停止、all plugins shut down和terminal cleanup四条正常teardown。该localhost现象不被归为已证实的内嵌引擎崩溃，也不混同新95E5GUI；当前launch回执位于测试根 `main-review-20261014.launch.json`，stdout/stderr独立归档。

该95E5批次的Windows/shared两项review修复、121项Windows unit、定向组件检查、95E5构建及上述实际GUI范围已收口；当时为debug主程序+Vite，不是bundle/installer或全部网站证明。当时Bili搜索子页pending原因未知，后续已由文首静态UI对照定位并修补；Mac按用户要求暂缓。强Session重叠、零Turn中的active Agent Stop未覆盖仅说明该批边界，不新增发布gate，不宣告跨平台Goal完成，也不推导零风控。

已执行的定向命令：

- `cargo test -p nomifun-browser-platform --lib --locked --offline`：最新 93 项通过，包含同文档 provisional 序号、优先 Stop/Pause、创建取消、必填控制序号拒绝、Unknown repeat 与 dispatch independence。
- `cargo test -p nomifun-app --features browser-use --test browser_workspace --locked --offline -- --test-threads=1`：13 项通过；缺少控制序号的 HTTP 请求为 422 且不下发，合法 owner 的原序号保留到 native port，其他 owner 为 403，canonical Turn 准备和已持有的 Agent 输入锁拒绝用户控制。Agent 的 Pause/Resume 为 UnsupportedAction，用户浏览不修改 canonical binding 或 grants。使用 native port fixture，不代替 WebView2 实机。
- `cargo test -p nomifun-browser-platform --lib navigation::tests --locked --offline`：13 项通过，包含宿主 dispatch 与原生 attempt 独立计数。
- `cargo test -p nomifun-browser-macos --lib --locked --offline`：同步前9项实际reducer测试通过；同步后portable10项见上文，不是原生WK实机运行。
- `cargo check -p nomifun-browser-macos --all-targets --target aarch64-apple-darwin --locked --offline`：通过，覆盖 native crate 生产与 unit API；desktop macOS host 与完整 App 未在该命令范围内。
- `cargo check -p tauri -p tauri-runtime-wry --offline --locked`：通过；vendor 内 mock unit tests 尚未执行。
- `cargo test -p nomifun-desktop --bin nomifun-desktop browser_surface::windows:: --locked --offline`：95E5 review批次实际production binary模块 **121 passed，0 failed，123 filtered，EXIT0**，日志与256路径来源稳定证明见历史review收口；同步后125项见本节开头；History批次为120项，保留其原回执。较早claimed-popup/initial-dialog批次为112项通过、255路径稳定。相对原生批次的 107 项增加 3 项 popup 准入/清理与 2 项无内容 presentation 测试；107 项相对历史 95 项增加 5 个准确取消/迟到事实、5 个 UDF 输入准备与 2 个实际 getter 表示校验测试。历史 95 项对应 Worker 源 `frame_sessions.rs` SHA256 `6EA8E875AC1F688512021886BD77161AD427CA142FBB67DBF26B1FF8DF444068`，日志为 `nomifun-browser-windows-20261013-navigation/production-windows-worker-module-tests-final.log`（SHA256 `245430C60163D800B6B8F74959D35E4049F6A5BDD0D81BEBA2FAC117A1B3D2E2`）；更早 83/89 项仍保留其原日志。unit 与真实 WebView2 验收分别证明各自范围。
- `bun test --cwd ui src/renderer/pages/conversation/Browser/BrowserPanel.test.tsx src/renderer/pages/conversation/Browser/client.contract.test.ts`：同步前143项、862次断言通过，包含typed页面失败恢复和权限失效；同步后145项/923断言另列于上文；其中 BrowserPanel 为 141 项。
- `bun run --filter=./ui typecheck`：通过。
- `bun run check:desktop-ui-boundary`：通过，最低 880x600。
- `bun run check:browser-platform-boundary`：Popup/presentation 最终源码与新增 fixture 合并后，root 实际复验 EXIT0。该源码检查不代替新 fixture 的原生执行。

控制命令 Reload、StopLoading、PauseNavigation、ResumeNavigation 必须携带发出时看到的 `expected_navigation_sequence`；缺失值不默认为零。Windows 公开 `load.navigation_sequence` 是当前控制 fence，Back/Forward 可在没有新原生文档尝试时推进；准确 NavigationId、其原 attempt 序号及真实提交的 document generation 保留各自含义，取消/轨迹关联不改绑到新 fence。UI、native shortcut、后端取消入口和原生线程均核对捕获的控制序号，迟到控制不重新取最新序号操作后继。

当前本机是 Windows。已安装 Apple Rust target 支持 focused cross-check；仓库内 macOS runner 是本地 Darwin runner。用户明确暂缓该平台后不再尝试取得环境或继续其实现，真实 A/B、输入、Profile 重启、native pause/race 与网站结果在交接文件中保留为未完成；当前优先完成 Windows 的本地交付。

**Windows 原生首轮结果与已确认的后续修复**

`browser_workspace_smoke --navigation-only` 的第二次执行实际启动 WebView2，在 Runtime `155.0.4283.45`、Windows build `26200`、无检测到的环境/策略覆盖下，初始导航/被动更新、显式单次 Reload、两跳 redirect 单 native attempt、页面 JS reload/replace/meta 场景通过。安全诊断投影也通过。

该次执行以失败退出：Pause 已阻止文档替换并产生 Paused policy 记录，但测试服务端仍收到 `/nav/loop` 后继主文档 GET。严格断言保留；没有把该运行计为通过。WebView2 [get_Cancel 合同](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2navigationstartingeventargs) 明确允许 host 处理取消期间 GET 已经发生。随后已补充精确 root Document 的请求阶段 gate，取得部分暂停/继续原生用例证明，范围见下文；报告不以文档 hold 等同于网络停止。

空内容失败后的恢复显示已由单一 native presentation owner 实现：同时核对 controller 和 child HWND，按仍有效的面板请求在每次状态变化施加 mask。5FE4 原生运行已通过无 resize 的同位置失败恢复、实际 crash 后恢复、controller/HWND 可见性与原生截图断言；隐藏后的迟到完成与后续race在该5FE4阶段仍待补；后续DF6539及准确popup stage的实际证明另见相应记录，不由5FE4局部通过推导。

首轮可追溯日志位于本机临时目录 `C:\Users\MINISFORUM\AppData\Local\Temp\nomifun-browser-navigation-20261013-5080\` 的 `attempt1.*` 和 `attempt2.*`。attempt1 为 compile failure；attempt2 已实际运行但失败，其日志存在工具输出截断，后续运行采用完整文件捕获。最终交付只引用取得完整证据的运行。

**请求阶段暂停的合同与能力范围**

Windows helper 使用主文档 Fetch 请求阶段拦截，并保持拦截器到 controller 关闭，避免 Fetch.disable 自动继续尚未收到回调的旧请求。继续的边界使用原生 QPC 与 Network 的最早创建时间；Fetch.networkId 只与同一原生 root frame 的 exact Network ID 关联，不通过 URL 猜测，不修改 headers、HTTP method 或 body。

该时钟证明来自固定 Chromium source：CPU [invariant TSC 判定](https://github.com/chromium/chromium/blob/61998d54c209eeb9a451915bda78764a542d36c8/base/cpu.cc#L248-L254)、Windows [QPC 整数微秒换算和时钟选择](https://github.com/chromium/chromium/blob/61998d54c209eeb9a451915bda78764a542d36c8/base/time/time_win.cc#L462-L552)，以及 native navigation [Network 时间来源](https://github.com/chromium/chromium/blob/61998d54c209eeb9a451915bda78764a542d36c8/content/browser/devtools/protocol/network_handler.cc#L3080-L3111)。ID 关联遵循 [Fetch.networkId 合同](https://github.com/ChromeDevTools/devtools-protocol/blob/c3ed21b573304f25851899056f38470f287abf16/pdl/domains/Fetch.pdl#L212-L214)；微软的 [QPC 跨线程一 tick 歧义](https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps#guidance-for-acquiring-time-stamps) 被保守地纳入阻止边界。这些是实现合同依据，不代替实际 WebView2 验收。

当前代码只向具备上述 CPU/clock 能力的 x86/x64 提供 navigation_control；其他架构或无法建立时钟证明时省略该控制并拒绝 Pause，普通导航与 Stop 仍可使用。初始化和运行时拦截失败必须如实表达，不能把仍存在的 native policy hold 当作已经证明网络暂停成功。

**历史原生运行及其当前证据边界**

历史严格 native 运行已经通过 Pause 后 HTTP 主文档计数冻结、A↔B、原文档 nonce 保留以及 Resume 不释放旧 POST；整套因 resource 场景缺少 frame/worker witness 而失败。完整日志为 `nomifun-browser-final-c3826bcc-91a3-49ad-b53d-7fd4ae1ae511/navigation-only.log`。后续 89113 越过 resource/Worker 后因 TLS fixture 未送出证书失败；5FE4 越过修正后的 TLS 后因首次加载 Pause 失败。三个历史整套失败仍保留；当前 DF6539 完整导航通过，证明的是后续已修正的源码和本次矩阵，未改写早期结果。

Profile fixture 已确认首次 `/profile-seed` 的 Cookie HTTP witness 成功，同一 Session 的 CloseAll 后 `/profile-reopen` Cookie 和原生 jar 均缺失，localStorage/IndexedDB 仍保留。阶段日志与安全请求 witness 位于 `nomifun-browser-profile-evidence-5bead193a16a45f18db66bf895a9708e/`。

微软的 [用户数据目录合同](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/user-data-folder) 明确 UDF 保存 Cookie 等浏览器数据，并建议需要复用用户数据的应用保留 UDF；Win32 不自动删除 UDF。因此，持久 Cookie 随保留的同一 UDF 重开可用是本轮对照的合同预期，仍须由实际原生 jar 与服务端请求证明，不能以文档或目录相同把当次失败计为通过。

后续被动诊断确认该 Cookie 的 `session=false`、HttpOnly=true、未来过期时间；重开前后 browser PID、ProfileName、ProfilePath 相同，失败断言前也已实际测得进程内 Profile token 相同。保留 Environment5 的对照仍失败且没有观察到 broker 退出，不采用基于该假说的进程等待修复。早期“原生 jar”仅指 CDP Network.getCookies；后续已增加 CookieManager(null URI/all) 独立对照，也确认控制 Cookie 缺失。保留的带 ownership marker 的合成测试根中未找到 Cookies/WAL/SHM 文件，因此 Cookie 行及加密标志为 unknown，不把数据库缺失解释为解密失败或已知行删除。

实际 NativeProfilePath 已证明包含在 intended owned UDF 内。首次 view 显式等待 35 秒、普通 Tauri/external_page 最小构造、旧 SDK Profile/CookieManager keeper 及 raw public SDK 同前缀基线均未解决重开 Cookie 丢失，localStorage/IndexedDB 保留。因此不采用生产延时、隐藏 view 或 keeper，不把 Host/CDP 初始化隔离认定为必要原因。已知 Cookie 偏好与策略项未发现显式配置，未存储的 Runtime defaults 仍未证明。早期 raw SDK 完整结果为 `raw-sdk-default-only.result.json`，其中 EXIT0/diagnostic_completed 只表示诊断完成，storage_acceptance_passed=false。

历史 `raw-sdk-path-ab.result.json` 使用同一 5FE4 artifact、相同 HTTP origin 与 Set-Cookie 响应，在两个独立新建 owned UDF 中只对照传给公开 SDK 的路径表示。两边首次 HTTP 与 SDK Cookie 均存在、session=false、HttpOnly=true、未来过期，实际 UDF 目录对象与各自所选目录一致、非 InPrivate、ProfilePath 包含在 owned UDF 内；重开仍使用各自同一 Profile。安全结果如下，不导出路径或 Cookie 值：

| SDK 用户数据目录输入 | 输入长度（UTF-16） | Close 前 / Reopen 后 Cookies DB | Reopen 后 SDK / HTTP Cookie | localStorage / IndexedDB |
| --- | --- | --- | --- | --- |
| Windows verbatim 表示 | 164 | 均不存在 | 均缺失 | 均保留 |
| 同一所选目录的 ordinary 表示 | 160 | 均存在 | 均存在 | 均保留 |

该对照不使用 Wry/core IPC/CDP、不复制 Cookie、不改变 Session namespace，Controller 每实例只 Close 一次，两个临时目录均完成 owned cleanup。结果支持修正 native UDF 路径表示这一方向，不能推广为所有 Runtime/路径的结论。A/B 结果中的 `production_storage_acceptance_passed=false`、`no_production_change=true` 指此次诊断运行；结果 SHA256 为 `C1B909859FE0993F7CEB87975F646AEC270FEA7984AB5E16E460506B78A63B34`。

后续生产源码增加 `prepare_native_data_directory`，在 lazy native 创建时仅转换 SDK 输入表示；原 ProfileStore 路径继续负责身份及清理，并用同时持有的原始/转换目录 handle 的 volume/file identity 证明同一目录对象。普通页与站点维护 view 共用 helper。独立 `closeout-udf-property-proof-result.json` 随后再次复现 169 字符 verbatim 与 165 字符 ordinary 对照，并证明原生 UserDataFolder getter 在 canonicalize 前保留了各自表示；verbatim 即使指向正确目录对象仍丢 Cookie。因此生产 `verify_effective_profile` 先检查实际 getter 为安全 ordinary 本地路径，再核对目录对象和原 owner，不能 canonicalize 掩盖 override 的有效表示或重写企业/环境覆盖。该独立测量结果 SHA256 为 `AF04D1DD0D26B64F8D2C8A02F84DCD0DFCC5154539E24DECB0D7EE863F4AE971`。

当前 DF6539 的严格生产 Profile fixture 实际运行 5 次独立应用阶段：seed、restart、错误目录 override、同目录 verbatim override、匹配 ordinary override 后 preserved。seed 内包含同 Session CloseAll/Reopen 与另一 Session 隔离，restart 证明应用退出后重启保留及持续隔离；HTTP HttpOnly Cookie、localStorage、IndexedDB 均按严格断言通过。匹配 ordinary override 实际测量通过，另外两种 override 均在目标 GET 前拒绝，两个请求计数均为 0，来源目录的数据保留。没有受控 Environment keeper 或人为等待介入，输入准备及 getter 拒绝也纳入 107 项 production unit。站点维护 view 的 ClearSiteData/Close 与其他 Session 不变在同 artifact 独立复验通过。

不能安全转换为 ordinary 本地磁盘路径的 UNC、过长或特殊名称仍在创建 view 前以既有 ProfileCleanupInvalid 拒绝，不盲删前缀或另选目录；全路径、其他 Runtime 或其他平台的存储兼容性没有由此次证明覆盖。

历史 `--dialog-probe-only` 运行 51909 已通过完整 dialog/runtime/navigation/close 矩阵；Stay 为 Cancelled/Stopped/NavigationRejected，保留实际旧 nonce 与 generation，Accept 实际导航。aligned artifact 5286 的独立复验中，Dialog、site-data、user-downloads 均 EXIT0；site-data 的 JS Cookie/cache/SW 路径不代替 HttpOnly 持久性，chooser/cancel/lifecycle 不代替实际下载完成。当次 user-files 为 EXIT1：日志依次出现 hide、navigate、frame_navigate 后的 `ActionInterrupted`，来源是 `FileChooser.next` 的 chooser witness 等待，实际目标 iframe 替换尚未执行；当时仅 hide/navigate 的退出、None 与零文件断言已过，不能把错误吞掉视为 frame 导航成功。历史日志与结果为 `legacy-native-summary.json`、`legacy-user-files-only.log` 及其 result/exit 文件。

随后构建 87691 产出 SHA256 `5FE4CD090060B7080E67FC587DBF5959DE8540BDD76D420D2B6C51FEBA581106` 的 artifact，user-files 完整原生复验 EXIT0。有界被动诊断记录恰好 1 个 OOPIF chooser、非空 session、有效 frame/backend input/mode，随后 picker 打开、目标 frame 替换 ready witness、精确 picker process 退出及未交付文件；hide、navigate、Agent admission 和 close 全部通过。完整历史证据为 `tls-path-final-userfiles.log`（SHA256 `455FC7D1B1A67B04937C423E03BAFE5BB24E3886F5EF961EE7A9A91A45475AAE`）及对应 result/exit、`artifact-tls-path-final.json`。

后续包含取消与 UDF 输入修正的 `0AD3FBCD54C194BDAB89F18F80B6D08B793AE21B8832D3369FA81B4B85F2D3F1` artifact 又通过 `closeout-dialog-regression`、`closeout-user-files-regression`、`closeout-user-downloads-regression`，各有完整 result/exit 和前后源码快照。dialog 覆盖 BeforeUnload Stay/Accept 与 lifecycle；user-files 覆盖 OOPIF 替换、无关 iframe、hide、导航、Agent admission/close 等待 picker 退出；user-downloads 仅 cancellation_matrix=true，saved_http_bytes=false、active_transfer_survives_agent_then_closes=false，不能扩大为传输完成证明。输入仍是单次 press/release，未吞错或重放 click；多个后继通过未补出旧 chooser 失败的详细原因，亦不证明间歇问题已消除。该三项不借用最终 DF6539 artifact 身份；DF6539 实际字节/下载 publication 的证明来自独立 Agent 链。

Worker 历史 production 基线中，worker HTTP 已加载但无执行消息；强引用和只 attach 均未解决，准确 session 的 runIfWaitingForDebugger 后执行恢复。普通内嵌 constructor 正常。手动 attach 的 `waitingForDebugger=false` 不证明启动未等待。结合 M155 中等待设置先于类型 filter 的源码合同，生产修复保留 OOPIF 首脚本策略，将 worker/worklet 辅助 session 与 DOM frame 路由分开，并在准确归属与预算内恢复。冻结源纳入 aligned artifact 5286；89113 及后续 5FE4 严格原生运行均通过 parent/nested Worker、两层 frame、资源身份与 HTTP execution、popup marker 和 403，未使用诊断干预放宽断言。历史 89113 整套 EXIT1 发生在 TLS fixture，日志为 `navigation-final-production-worker.log` 及其 result/exit 文件。

历史 TLS fixture 的服务端 native error 为 `0x8009030E`，严格客户端 certificate_delivered=false，未形成证书拒绝场景；不归因于生产 classifier。随后 PFX DefaultKeySet、严格 preflight 与 managed cleanup 的独立验证证明证书送达、untrusted root=true、hostname invalid=false、无 trust override、临时 native key 清理。独立证据为 `tls-handshake-diagnostic/original-handshake-result.json`、`final-fixture-handshake-result.json`、`final-cleanup.json`。没有修改信任库或浏览器证书策略，preflight 不计入浏览器 HTTP witness。

历史 5FE4 的实际原生导航在修正后的 TLS 中取得 ClientHello=1、server_no_credentials_failures=0、HTTP 请求及完成文档均为 0，并通过严格断言后继续。空内容失败的 controller/HWND mask、无 resize 同位置恢复及截图、实际 Page.crash 后 authority 撤销和恢复、首次加载 Stop 也通过前置断言；当次 DNS 最终字段未输出。最终 DF6539 完整运行已输出真实 Tls/WebView2 code 5 及正常 managed/owned cleanup；DNS 明确为 false，实际为 committed/other/WebView2 code 0，仅证明该原生结果被如实分类，未证明 DNS 失败。

历史 5FE4 整套 EXIT1：`/nav/slow-pause` 的 navigation_sequence=2 先 Cancelled/automatic_navigation_paused，随后追加 dispatch Failed 与原生 Failed/network_connection/native_code=9，最终 paused=true 但 phase=Failed。日志为 `tls-path-final-navigation.log`（SHA256 `52C3FB481DF15190271EE882D17AC0D24BC48445077E423857096458E80C2B8B`）及 result/exit。生产修复只以准确尝试的已确认 native Stop/Pause 保留取消终态，独立原生迟到完成继续记录；Stop 意图本身不能掩盖失败，真正 commit 仍更新文档事实，旧回调不影响后继。5 个新增 production tests 与 DF6539 完整原生导航均通过：首次加载 Pause/Stop、保留旧文档失败、晚前驱、provisional 控制序号、迟到 receipt、旧 Stop/Reload、隐藏后迟到 commit，以及真实用户 coordinator 的原操作 join。原生导航 fixture 未覆盖的下载传输与存储分别由独立场景记录，不能自动从其 EXIT0 推出。

Agent 历史 canonical-warmup 运行已证明同页 trusted input、上传 owner 完成、实际 HTTP 204/文件字节与服务端 body、下载实际长度/SHA，以及 UserReady、native unlock 和 projection.finished。历史诊断运行 91343 的精确分页结果为：1 页、146 个事件，目标 Operation 的 `turn/started` 为 seq 5、`turn/workspace-files-observed` 为 seq 142、`turn/failed` 为 seq 146；错误为 `USER_LLM_PROVIDER_INVALID_REQUEST`，脚本模型调用 12 次，fixture 记录 `Unexpected extra model request 11`。因此不是分页遗漏或晚 append，当次确实失败；源审查及下述新运行的实际工具表确认，旧 fixture 在 Browser-only 合同下硬调未广告的 `report_completion` 是 fixture 错误，不能通过原生 unlock 或 projection.finished 宣告 canonical 完成。

当前 fixture 在同一 step 10 根据实际工具表选择 `report_completion` 或普通 assistant-stop，不增加最终模型轮次，生产 Agent 完成合同保持不变；没有 mandatory Plan Gate、Workspace grant 或旧 transcript fallback。首轮通过的 97535 使用 aligned artifact 5286（SHA256 `75C82FB8429D98E52AB8177AA99BC2538813216213839199D793131E9A317D9E`）；最终 DF6539 的 `closeout-final-agent-terminal` 再次 EXIT0。实际广告 10 个工具、report_completion_advertised=false，同 step 10 普通 assistant-stop，总计 11 次本地脚本模型调用。最终精确同一 Session/Operation 的 started seq 5、workspace-files-observed seq 136、completed seq 142，1 页/142 事件与 canonical history marker 均通过。trusted input、上传实际字节、下载长度/SHA、target-blank cleanup、终态前 native lock、UserReady/unlock、同页保留及 teardown/隔离后端清理全部通过。这是内嵌原生链与 canonical 合同验收，不构成真实模型服务或正式主 GUI toolbar 验收。

上述 Agent、Worker 和阶段矩阵的完整日志集中在 `nomifun-browser-final-*` 与 `nomifun-browser-conformance-final-5f70e92f-8fc7-494b-80a4-e7c9e27213a9/`。历史 91343 的精确终态日志为 `agent-exact-canonical-pages.log`；首轮通过证据为 `agent-final-exposure.log`（SHA256 `F47552C983655D50759FC171C2F9277FD226844DCECC7CDB6E8BEC9A60239BCF`）及 result/exit、`artifact-5286.json`；当前 DF6539 来源及最终 Agent 日志见上表。

本机当前证据根分别为 `C:\Users\MINISFORUM\AppData\Local\Temp\nomifun-browser-conformance-final-5f70e92f-8fc7-494b-80a4-e7c9e27213a9`、`C:\Users\MINISFORUM\AppData\Local\Temp\nomifun-browser-profile-evidence-5bead193a16a45f18db66bf895a9708e` 与 `C:\Users\MINISFORUM\AppData\Local\Temp\nomifun-browser-windows-20261013-navigation`。这些临时本地记录用于追溯本次结果；交接前须按实际保留状态确认存在，不把目录名或旧 PID 当成仍在运行的证明。

历史AA94正式主 GUI 曾在独立测试 namespace 中启动，主程序 SHA256 为 `AA94E75A7015D9EDA4BDE22E1492FB6F55ACB0E79932E4960000AD0D1D62814A`。实际 PID `44036`、动态 backend `49285` 与 HTTP health 已核对；使用当前 Vite renderer、独立 data/work 与第一方 UI 缓存，构建禁用了本次 deep-link 协议注册。发布配置没有改动。真实输入、菜单/复制诊断/收起恢复/分栏的 0 额外主文档请求，明确 Reload、地址栏 F5、网页 Ctrl+R 的各一次请求，以及 pending Stop、首次无内容 Stop、同槽恢复、页面发起加载的 Pause/Resume 已取得实际界面证据。循环 Pause 后约 47 秒及 A↔B Pause 后约 37 秒 HTTP 主文档计数保持不变。GUI 的诊断读取使用产品菜单与自有未提交草稿；工具对长文本截断时仅归档 prefix/tail，不称完整事件数组已读取。

这次旧 AA94 主界面验收发现的 popup/Pause 和无内容恢复反例已有源码整改、64CB 回归与34DDD补证。9C6F 后续用户启动、真实功能、基础 Session、最低尺寸和两站分项证据已按上文范围收口；其同文档 Back 历史失败不被 D74 修复/native 或 E584 构建改写，当前95E5的History已取得主GUI通过，880×600原生输入、显隐与宽屏恢复已在本轮实际通过。

上一轮随后由用户物理 Escape 明确终止 Computer Use，当轮停止全部界面操作与观察，不通过其他接口接替输入。停止时外框 `881×834`，尚未达到 600 高度；Session 切换与三个真实网站未完成。当轮只完成 example 编译与 production unit；授权恢复后才执行新原生回归。用户随后正常 tray Quit 旧 AA94，原 PID 44036 已不存在、49285 无 listener/health 不可用，owned shutdown 四条日志已核对；没有 force kill。退出证明 SHA256 为 `E6D1BA4E34CD454053D428C47D8A83E45C6AFED89740955A948BD81207167E79`，只关旧 main/backend，不称所有 WebView broker 已退出。旧进程退出后才构建 9C6F；其 build 和第二 Session 准备不计作新 GUI 通过。具体启动限制和后续步骤见 [Windows GUI 交接](../handoffs/2026-10-13-browser-windows-gui.zh.md)。

早期 Computer Use 因无法可靠识别新建 Edge 匿名窗口 URL 自动中止，Cua 未取得可用 Edge，未访问测试 URL。这不构成内嵌验收。新增 HWND 当时为 `28903512`，与用户原窗口共享 PID `2244`，不得据此结束共享进程。历史服务 PID 只作为线索；当前准备记录与服务 epoch 见自有 GUI evidence，必须现场核验，不把旧 PID 或 fixture 截图计为当前正式验收。

本地原生验收入口：

- Windows：`cargo run -p nomifun-desktop --example browser_workspace_smoke -- --navigation-only`，使用临时 Profile 和独立 HTTP witness。
- macOS：`cargo run -p nomifun-desktop --example macos_browser_compatibility_smoke -- --identity-ab --live-sites --evidence-dir <absolute-temporary-directory>`；真实手动证据使用 `--manual-sites`。以实际 runner 支持的选项为准，不把脚本内物理操作声明当作生产 toolbar 已验收。

完成前逐项核对调查报告中的矩阵，将新增 fixture/test 输出和 native 截图/安全时间线加入本记录；不以最终停在验证码页或一个平台的绿灯宣告跨平台完成。
