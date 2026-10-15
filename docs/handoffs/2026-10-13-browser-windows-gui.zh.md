# Windows 内嵌浏览器主界面验收交接

Windows Bilibili搜索子页持续加载已定位到opener controller过早隐藏，并完成最小修补和静态UI A/B/A。95E5/A166及更早历史保持原范围，当前实现与取证见[整改证据](../reports/2026-10-13-browser-remediation-evidence.zh.md)。Mac按用户指示暂缓，本项Windows通过不能代替WK现场或整个跨平台Goal。

**当前修补与实际界面证据（2026-10-15）**

真实popup成为活动tab后，旧呈现路径立即令opener的WebView2 controller不可见，child首次文档可能长期停在Provisional/None/0%。仅为准确popup opener暂时保活controller、HWND继续SW_HIDE，即可让child正常完成；撤回同一修补再次复现，因此本轮已定位应用边界原因。此结论限当前Windows/WebView2 155环境，不把SDK内部调度机制泛化，也不当作原始Mac百度现场根因。

最小修补只在NativeTab保存一个可选opener target；满足当前可见面板、活动child、同runtime、准确owner及有效surface lease时，首次child文档pending期间保持隐藏opener controller可见。提交、终态、Pause、后继尝试、关闭或lease失效即释放；UI线程再验精确owner/target与状态。child的44A原遮罩不改，无parent像素暴露或焦点变化，不新增snapshot副作用、ledger、CDP干预、UA/代理变更或重复导航。

| 相同静态UI、原S1 Profile对照 | 实际观察 |
| --- | --- |
| 77A46190旧行为 | 22.773s仍0%；37.829s切回parent后125.582ms首data、157.184msloadingFinished |
| C212F598修补 | 3次真实鼠标搜索均有结果；准确Document loadingFinished距click为1019.768/1294.166/1028.589ms，没有切回parent救援 |
| E429F4CD撤回修补 | 25.616s仍0%；49.358s切回parent后138.667ms首data、174.794msloadingFinished |

上述时间为输入与host callback到达关系，非线上TTFB；按exact view/rootTarget/requestID关联，不把iframe计成主文档。raw与receipt在c020根的gui-cu-static-baseline-77A46190、gui-cu-static-opener-C212F598、gui-cu-static-recheck-E429F4CD，准确SHA见整改证据。三组原DATA/WORK及S1 Profile均保持，没有Profile复制/清空；同静态UI0.8.8/API30/build-id1f7e0b07-b245-4f04-aaec-69c66d8d0fa4。使用现有Tauri/custom-protocol内嵌资源，不再依赖Vite5173，不改变内嵌产品形态。

C212主程序SHA为C212F5985127049691E9BB0D901090C0B7770E351DCA33B229365296D060BF82，main build EXIT0；独立review无阻断项，presentation定向7项通过。unit/main各4219来源及424静态资源前后差异0。host源码78424B212F7BC0B164D86579A43F0A0D57244DC4F3C98AE796271E4D57E28BE3、presentation源码00F43DAAEA93C0D627AE161A6652E207E466A3CBF88C5820A21071FC6743B4E3；反向对照后已恢复并核验此C212二进制与源码。PID28612最后一次真实鼠标搜索UTC03:31:11.489发起，准确popup/root/request的loadingFinished为UTC03:31:12.641514，距click1152.514ms（probe elapsed851ms）；搜索后169.084s的真实窗口仍显示同tab正常结果，期间没有parent切换、reload、Stop或新导航。证据gui-cu-static-final-C212F598/05-final-mouse-result.json SHA833676B423A485099E99911286EF840A13CA18280F746D9F0A620A1B5D661ABE；30行safe JSONL SHA5EA0E9DC3A068F11D2F4143F9326527E970EB620B8A996DD8D31803D9EBB7552，receipt SHA8A0C5D1D515E8CF20293FDE61D47550855BF1DB61BC7F29251319709ED45A5A2。最后app保留可视运行供用户查看，不声称PID28612已退出。

所有新启动、关闭与界面操作均由Root使用computer use自主完成，不再依赖用户启停。main中debug验收配置仅在编译指定独立data/work时启用，X沿既有QuitFlag/ExitCoordinator正常退出；普通debug/release产品行为保留。已观察准确PID消失和channel/plugin/terminal cleanup里程碑，不扩大为OS退出码或所有WebView/Profile/数据库逐项清理证明。更早localhost拒连来自Vite服务当时不可达；本轮静态UI对照排除了这一验收服务依赖。

UTC03:02 fresh fetch/ls-remote确认HEAD=origin/main=canonical remote为2d5aa62d3f7daf2869fe20be457174742431238a、0/0、无U冲突。原250路径232逐字节保全、18重叠整合；新增main debug验收配置后当前251本地路径，均未commit/push，备份/stash仍保留。下节旧同步与失败构建按历史保存，不能据此说当前仍无法启动或Bili未修。Mac完整Host/WK、两个smoke、A/B、正式store及queued/in-flight popup Pause继续交接。

**同步后早期诊断（历史，已由上述后续对照推进）**

HEAD与origin/main已同为 `2d5aa62d3f7daf2869fe20be457174742431238a`（Root核对0/0，workspace0.8.8）。同步恢复250个本地路径：232个逐字节保全、18个重叠整合；5个文件的文本冲突以审核7Mac候选解决，无未解决冲突，原staged0恢复，备份和stash `3ba56082e18254650f88fe48a5999f29883d1dbb` 保留。`restored-source-verification.json` SHA `848C9BAC0B79A2DE9ADBFA62957D2DB6A60D9A614E4F289666B42EEFE7D79839`，位于新的 `nomifun-browser-bili-sync-c0200bd3e0d84e9f9c63a6a66df90927` 临时证据根。

同步后Windows125/0/121 filtered、Mac portable10、Mac crate Darwin all-targets、Workspace34项各EXIT0/1870来源0差异；UI145/923、Space6/56、Resize6/100及typecheck/browser/desktop2115/min880×600/i18n通过，UI汇总SHA `32FFB49BEBAFC7CECF0E4DA6349C243AE96F5E31480B4E55DE6F6866A0D7739F`，Root另有实际Agent Session boundary通过。准确范围和raw标签见整改证据；这些不等于同步后原生网站或GUI通过。

A166阶段Bili尚未查明、未修复，当时已实际启动。自动审批拒绝已留作历史；用户已手动启动native SHA `A1667B61F06E11474B00C8CCFFECEB1FB018925985DD5A45DAFEBE1932E5F511`、PID13864，真实UI已加载。native基准3692、renderer基准2d5，证据在c020根 `gui-baseline-A166/`，明确混合诊断基线，不能作最终融合native GUI通过。

- Enter搜索popup：02/03/04观察实际结果；05为seq1/docgen1/200、commit885ms/finish1549ms、user0/page1/drop0/lossfalse，diag SHA `7F2122193B8B9AE6EF06DE9BBC25A7A21C79840671877FD00DABCEE4D0B3F08A`。06/07同URL普通tab也出现结果。
- 鼠标搜索popup：09/10和11/12持续None/0%；11在UTC16:46:53.209、搜索后113.98s仍Provisional。15是准确同child/runtime1/control1恢复、docgen1，commit121462ms/finish122110ms/200；14约250秒时见实际结果。恢复diag SHA `1C3787FD452680EFFBD01FC87F7C7F66B897B7EED46A6295A41C1ACAC26923D8`，不能将截图时间当加载耗时。
- 未以reload、Stop、runIfWaiting、UA或代理干预恢复。`probe-safe-network.jsonl` SHA `E16FE90029CB26D2F154D19AA3D7000B322CF1C5ACA499273FD18DEE2C8ABA34` 有该view的200/632ms响应头样本，但样本导航关联unknown、body完成未知；不称网络正常或renderer已归因。Page读取未在500ms回复只记限时观察，原因未知，不能凭TargetInfo回/Page未回推出renderer hang或Debugger waiting。当时下一候选及其unit仅计划补body取证，不是根因修复；当前修补以上方对照为准。
- `bili-post-sync-probe-main` EXIT101：覆盖运行中13864的exe时报OS error5，未见Rust源码编译诊断错误；full-source回执4215项漂移0。当时已请求用户正常托盘退出，该失败记录不预记后续build/采样；此后Root已自行CUA启停并取得上方实际结果；fresh fetch由Root核对2d5/0对0/无U。Mac queued/in-flight gap与完整Host/WK仍未闭合，portable源码包及最终prompt另行交付；本节不是当前Windows Bili状态。

**同步前95E5及先前GUI状态**

旧 AA94 已由用户正常 tray Quit，原 PID44036/backend49285关闭与owned shutdown已核对，没有force kill。新main构建EXIT0，SHA256为 `9C6F261A219E0BA81910F425AC25168DC2D42F33EF361862AF278BF0546F1431`，前后255路径零差异；构建汇总SHA256为 `EBEE4923E4A61A32C0B3C85777B4660EC47814BD0352F3AA224B2F4D49043244`。工具启动历史拒绝后，用户亲自隔离启动成功，fresh PID34236/exact executable、owned port59127与health200/ok/0.8.6已核对；唯一真实窗口和Browser入口及后续功能实机范围已取得；这组9C6F已由用户正常Quit，原PID34236与backend59127关闭已核对。后续E584实际启动并正常teardown；95E5已构建并接续启动，对应History实际GUI已通过，本轮880×600输入、显隐与宽屏恢复也已实际通过，未借旧窗口计新GUI。测试继续使用独立data/work、唯一UI namespace `com.nomifun.desktop.browseracceptance.1a9eb67ef82c44acb7cd6131b337a431`与marker，overlay不注册deep-link，默认发布配置保持原值。

旧AA94的popup/错误页反例仍保留，64CB/34DDD原生通过未改写旧结果。9C6F当前真实GUI批次已收口：popup Pause/Resume、首次无内容失败恢复、两Session基础切换/输入、修复后880×600和宽屏恢复通过；百度主页/搜索与Bili主页各自静置正常。同文档history状态缺陷已由D74源码修复并通过60C/72E定向native，95E5新main已构建并接续启动，对应History实际GUI已通过，880×600输入、显隐与宽屏恢复也已在本轮实际通过；Bili搜索只有provisional后人工Stop/close，业务结果不PASS，不能估计风控率或扩大原因。

上轮最后核验的 owned 服务 epoch 为 Vite `13076/5173`、页面 `43740/65409`；此前 `58674`、`56868`、`49563` 属于已结束的旧页面 epoch。旧 canonical fixture server 已结束，准备时关闭过数据库，只留下可由真实主程序读取的 Session；人工浏览不需要恢复旧脚本模型 server。接续时仍须现场检查这些服务，不能把记录视为当前存活证明。准备与逐步截图、计数、菜单诊断 prefix/tail 在上述 GUI 临时根归档；AA94早期工具截断只保留prefix/tail；后续9C6F完整49事件另由selected_text取得并parse，见下文。

普通文件汇总已保存到该临时根的 `gui-evidence/gui-main-AA94-summary.json` 和 `gui-main-AA94-handoff.zh.md`，包含 27 个用例状态与 17 次截图观察；暂停 popup 的反例明确标为 FAIL，没有将它改写为“blocked”。这些汇总保留停止工具时的最后已知状态，不是终止后的新窗口观察。

**上轮用户终止与后续授权恢复**

正常顶部边缘拖拽时，Computer Use 明确返回用户按物理 Escape 停止，并要求当回合不再调用 Computer Use。已立即停止所有界面输入、观察及替代接口操作，没有 reset/retry，没有强制退出主程序。最后已观察外框为 `881×834`；没有取得 600 高度、第二 Session 切换、百度主页/搜索或 Bilibili 60 秒验收。终止时 AA94 与自有服务留下运行，该记录不是当前存活状态；接续先重新核验归属，不以旧 PID 结束未知进程。用户随后明确授权继续原生与界面验收，才执行下面的新原生批次；最低窗口和真实站点的后续实际9C6F范围另列于下文。

终止当回合只进行普通文件记录、独立 example 编译和 production 模块 unit，没有运行新的原生窗口/input fixture，也没有覆盖正在运行的主程序。授权恢复后才取得 writable about:blank、首附件 chooser 与下述 native 范围，受影响真实GUI当时仍须在9C6F上重测，后续实际范围见下文。第二个零 Turn Session 后续已通过正式 API 在自有数据根准备，没有扫描凭据、hidden JS 或直接数据库写入。

终止后的纯验证为 actual Windows production 模块 112 项通过/0 失败；example 编译 EXIT0，SHA256 `64CB7E6F0B426A16FC7C6A7BA32AD8B55A21A6E2010FAD244B95FC567DD9397C`，当时没有执行。两次源码快照各 254 个路径、前后差异 0；pure manifest 为 `nomifun-browser-conformance-final-5f70e92f-8fc7-494b-80a4-e7c9e27213a9/popup-no-content-pure-validation-summary.json`。下面的实际执行是在用户恢复授权之后，不能改写此前停止工具的范围。

**恢复后的实际原生结果与剩余证明**

64CB artifact 已实际运行三个 native 批次，result、exit、来源及 hash 均现场核对。每次前后源码快照为 254 个路径、独立比较差异 0；汇总为同证据根的 `resumed-popup-mask-native-summary.json`，SHA256 `0C81D89D5B5AB15B9888FA843E9DB3A37D86B66BE4F9B08235ECDCA95052D396`。其中 native_ownership_released=true、main_modified_or_terminated=false 只描述当次批次收束，不能代表后续服务或窗口状态。

| 原生批次 | 实际通过范围 | result SHA256 |
| --- | --- | --- |
| `resumed-popup-mask-navigation` | EXIT0；Pause ACK 后可信 named/blank popup 的 GET 增量 0、tab/view 与父 nonce 保留；Resume/明确导航正向 popup；writable about:blank 实际可见与可信输入；首附件 None 双隐藏但真实 Save chooser 出现并取消退出；恢复 pending 双隐藏、commit 后同槽可见/可信输入；保留文档 pending 可见 | `00FB312F262E7CD09069DB388FF48CB2D5F157F2B8E4C803D5D1C033E84F0906` |
| `resumed-managed-popup` | EXIT0；正式 managed popup、命名复用、配额、用户/Agent 输入锁、close 与 cleanup | `77BA4D241D69ECC0E1A395D0EAE573AA4E73662930FBA44D6D602F68C7FC5329` |
| `resumed-agent-download` | EXIT0；原生上传/下载字节与 publication、target-blank cleanup、输入恢复及精确 canonical completed seq 142；11 次本地脚本模型调用 | `F6534E0F169320396C66610E6A7C4A5162777FE69B3174F65C9A2CE92463F3E1` |

首附件用例不声称保存字节，实际字节由独立 Agent 链证明；DNS 仍为 dns_failure_observed=false，不借导航整套通过补出 DNS 失败。三个 batch 的 launch/exit/result、stdout/stderr 与源码快照都保留在同一根。

原来两项待补证已由后继 34DDD artifact 实际完成：准确 claimed/published、binding 前候选的暂停结算，以及首次脚本真实原生 prompt 回复后的可见/可信输入。artifact SHA256 为 `34DDD55E8B3D95B7524D6708F24CAB3094C4AB82516A06C876FD484EF6D2CE9D`；两 native 批次与 production unit 均 EXIT0，各自前后 255 个来源路径独立比较差异 0。汇总 `claimed-popup-initial-dialog-native-summary.json` SHA256 为 `694D2D404F366CF864EC68043F9ED3CD672B940837B0973D8F8A8A60F38820E9`。

| 补充原生批次 | 实际证明与准确范围 | result SHA256 |
| --- | --- | --- |
| `claimed-popup-pause-native` | exact opener 单次 debug oneshot 固定候选已 claimed/published、binding 前的时序，无 sleep；deferral 在 owner 持有期间结算，native Close ACK 与 native/runtime registry 清除先于 Pause ACK；可信 window.open 返回 null、父 nonce 保留；Resume 后旧 GET 增量 0、新 GET 增量 1 | `D0E3E0C7CF13215F28A3750C6650D27DDFC752863B7656E3C91CC4589938D4FB` |
| `initial-dialog-visible-input-native` | 首次真实 prompt 回复后同 native slot/document、CurrentDocument、controller/HWND 双可见、输入 enabled，实际值/focus/可信输入通过；layout_or_show_after_reply=false；完整 dialog/navigation/close 也通过 | `334DF746EBEDAC6E255744D954723EDFFDC9EF2B4F9E5DDBD4CFB9B97D92F36E` |

该claimed-popup批次的 production binary Windows 模块为112 passed/0 failed/123 filtered；该History批次为120项通过，后续UA review为121项，见下文；`claimed-popup-initial-dialog-windows-unit.stdout.log` SHA256 为 `3E52E0280C0236F729DA0602EBF2C190EA905F58FC6608215BAF64456F433203`，exit 与前后快照同样核对。候选 proof 不穷举全部调度交错，native request 的文档关联仍 unavailable，不补造 NavigationId/generation；具体 stage 不能被泛化为任何在途状态。

旧 AA94 正常退出证明为 GUI 根的 `gui-evidence/AA94-normal-exit-verification.json`，SHA256 `E6D1BA4E34CD454053D428C47D8A83E45C6AFED89740955A948BD81207167E79`。已读取真实 main-final.stdout 中 ChannelMessageLoop stopped、all plugins shut down、owned_runtime_teardown 和 terminal shutdown cleanup，旧 PID/backend 关闭已核对，不声称全部 WebView broker 退出；没有通过 GUI 工具代替用户退出。后继 9C6F 的 build、Session 准备不计作受影响 GUI、最低尺寸、Session 切换或真实网站通过。

**9C6F 历史启动拒绝、用户启动与现场接续**

此前工具隔离启动在执行前返回 `rejected: blocked by policy`，没有更细原因。拒绝记录SHA256为 `0FF86319F3D95681C014285F3FF22A3FFDA333DC1F12B6FDE9B2AE0DD2676155`，其中app_process_absent/inputs0是当次状态。没有换shell、wrapper、隐藏接口或无隔离Sky绕过；随后是用户亲自启动，原拒绝没有被重写为工具启动成功。

用户已在新PowerShell设置owned DATA/WORK并启动准确9C6F；此前AmpersandNotAllowed来自多行粘贴合并缺分号，修正单行分号后测试窗口出现，不推导产品startup或DLL缺陷。DATA/WORK仍为下面隔离根下的data/work，没有默认启动用户现有数据集。CLI固定argv、Sky无隔离参数与bootstrap忽略LOG输入的边界未变，实际日志根为DATA/logs。

fresh只读已核验PID34236、准确工作区exe、SHA9C6F、owned port.json59127/PID34236与实际health HTTP200/statusok/version0.8.6。`gui-evidence/main-9C6F-launch.json` SHA256为 `05B156643104D503EDF2AB2733F5804CDA4D7DAD84B32794D62707C5BE929077`，记录human launch且无policy bypass，Vite HTTP200。GUI owner已选唯一真实NomiFun窗口25629038（外框1282×834），在第一Session通过无Browsergrant入口打开owned51378/popup，mainDoc请求1；fresh `gui-evidence/9C6F/popup-parent-baseline.json` SHA256为 `5267F6AE016570BF6010CF9DA0BE0FDEBF0C0133C93C5ED92A6E1BC6EFA0E603`。已核对该文件的窗口与截图元数据；外框不充当最低内容视口证据。

**9C6F实机功能与网站范围收口**：最新 `root-closeout-9C6F/final-9C-main-gui-acceptance-summary.json` SHA256为 `F7DC2177849386B076A6977459ABF9494A6140079ED19191C84544AD4EA71441`，记录于2026-10-13T16:15:19.756Z（本地10/14）。汇总明确9C debugexe+localhost5173当前renderer，九个冻结production文件source_drift=[]，不代表bundle/installer；原报告文件名保留10/13。

- Popup：`9C6F/pause-closed-name-count.json`、`pause-new-blank-count.json`、`resume-does-not-replay-popup-count.json`均stable:document=1，同SHA `4B1F2352BCDBB5B70E2A0633E0EFCC5EFD45349EB3B01F833973A1F605953ED8`；新点击后为2/delta1，fresh count SHA `12D164A68655AD7B8B83FA633988ACF7911434FCF36FBADF872351396F84E4CC`，真实子页出现，父页输入保留。S1 `native-trusted-witness.json`另证trusted=true。
- 首次失败恢复：`refused-first-content-none`有宿主失败提示且无native文档；`failed-recovery-pending-none-masked`为120000ms恢复pending、加载提示、仍无native文档；`same-slot-recovery-finished`出现真实/slow与可写输入。pending/finished SHA分别 `4483038CF4E58900F505D131651D501A74E496E9C0FC5DFFDEC97884FE46F965` / `5F46BAADC1144D9572CE2C0E0AE64D02DDCD9F8EFE3B60FFED20AA48F478CFCF`，未将GUI树当COM visibility getter。
- Session：实际browser面板ID从S1切到S2，S2空入口→独立/stable→可写地址栏。S1 60000ms场景捕获14:45:43.642Z，S2捕获14:46:53.487Z（约70秒），文件 `session1-command-pending-before-switch` / `second-stable-while-first-pending` 只证明基础切换，不能由名字推强oldfinally/newBusyGate仍在途GUI竞态。
- 最低：历史 `minimum-outer-882x602-after-layout` 的Browser被挤出FAIL保留；修复后配置客户区880×600/外框882×602实际Browser/地址栏/菜单可用，S2回显 `GUI9C_MIN_NATIVE`且trusted。显式主侧栏展开hide/detach后globe重开，doc/echo保留，最大化恢复184/300侧栏与原divider约1000；before/after-input/reopen/final stable均3/delta0。final layout SHA `5D9A4758394471301C4931C62FC191D3D34E30788729DB875FC8FF187B452EF8`；after-input/final count SHA `C2F480238816468EECFEABEDDA9F0A7738EE062FAFE19B4134CF06BD8FD8B6F8`。零Turn没有active Agent Stop点击；916拖拽执行前outside bounds拒绝，未计916通过，不增设发布gate。

网站分项：百度主页95.158s、搜索114.721s、Bili主页136.237s各窗口无输入，两端normal，验证码没有当normal。百度搜索是same-document SPA，仍seq4/docgen2，native content=/s，无新mainDocument事件，原200不是搜索所有HTTP状态。首次Back **FAIL**：实际/s历史已退，seq5 Requested/Retained/0%，只有dispatch3/back_forward而无nativeStart/Complete；诊断SHA `BD6B2005052D3FD4429996114B1A77564EF44E69BE1AD6B1719F296838F49670`。Owner源码复核指向synthetic Requested+history ACK/SourceChanged(false)未结算；这是9C6F历史FAIL，后续D74修复、60C/72E定向native及E584构建已通过，当前95E5对应History实际GUI已通过，880×600输入、显隐与宽屏恢复也已在本轮实际通过；旧SPA conformance只验URL/history，不覆盖load phase。

第二次full Back、Reload与显隐正常不抹去首次SPA FAIL。Bili搜索物理点击生成新子tab，seq1 Requested+Started/page_navigation/user_initiatedtrue、Provisional/None，尚未观察content/commit/completion；Response只在NavigationCompleted采样，不能据缺字段判HTTP未返回，更不判captcha/风控或pagefilter等待。pending诊断SHA `C1E753DEB7B649A7F98274765C0352CC8D2E7F79DF65D10C08C49419216C7084`；人工Stop后Cancelled/None，close回parent输入仍在，无自动retry。Parent跨文档Back/Forward、Reload和hide/show均记录finished/正常，搜索业务仍不PASS。

最终parent完整诊断SHA `6B0B683BD4CB10778324D1FE36DFD453B26B78AC8BFE473A89E031EC27E66CC0`：通过产品Copy→本地地址栏UI selected_text取得17690chars并parse，tree Value7172仅前缀。完整49events/drop0，最后seq17/docgen8 Finished/200/CurrentDocument，user9、agent0/page0/unknown8/redirect0。九个显式命令含一次同文档Back和八次full，nativeRequested8数量/顺序一致，显隐无额外Requested、无重复Host派发；unknown来源/degraded/routing_losttrue保留，不称具名绑定、完整HAR或全HTTP零请求。GUI输入随后释放给history native补证，最终9C界面留正常；后续60C/72E History原生与E584构建已取得，单独记录如下，不修改本批9C6F历史证明。

**History补证与E584历史构建**

conformance根为 `C:\Users\MINISFORUM\AppData\Local\Temp\nomifun-browser-conformance-final-5f70e92f-8fc7-494b-80a4-e7c9e27213a9`。冻结生产 `windows_navigation.rs` SHA `D74E74C22BCC3067371ECEDA13AB16B206D15DF72D2D469ABE111124FFEF354A`，production Windows实际120 passed/0 failed/123 filtered；`same-document-history-native-summary.json` SHA `AA62D42A8C64A3B01F1FF5A64FDA9BD1CA08CDA8B616018BA7C764D289AEBEA8`。三个native raw result/exit与来源快照已独立核对，各256路径0差异、EXIT0：

- `same-document-history-native`，artifact `60C528473215AAECC2E0E727A78390E27B186ADC6FED8B720DF68C9A300974F3`：四次同文档Back/Back/Forward/Forward保持Finished/Ready/100%、docgen/nonce不变、主文档HTTPdelta0，control fence推进；run-lock、stale/current Reload和跨文档真实NativeStarting/Finished另有证据。raw result SHA `C6D561860A6B673FDB57885A698E048F4E39402275C269813258F0AA658363E7`。
- `history-beforeunload-stay-native`：generic Navigate→BeforeUnload→Stay，不能当History Back→Stay。raw result SHA `334DF746EBEDAC6E255744D954723EDFFDC9EF2B4F9E5DDBD4CFB9B97D92F36E`。
- `history-settled-command-stay-native`，artifact `72E65F25EA0CCB8FF16E08793908FC098D58049F5763111B456BFA8284CEFD06`：真实History Back已回复后的BeforeUnload false deferral/Stay，原文档Finished/Ready、docgen/nonce保持、目标主文档HTTPdelta0，无NativeStarting或伪造Finished；cancel audit为空，不造已结算命令的取消关联。raw result SHA `B3A4CA6CD9CDD0BEC00A871E823C7F7DE1B38AF12BF80DBFF4409A7B8791BAFC`。

控制fence不是主文档attempt序号。Back/Forward保留原文档事实；只有真实NativeStarting创建Loading，空History ACK与SourceChanged(false)不代表Finished，不用JS、URL猜测或另一份ledger补全。真实NavigationId/attempt及Unknown迟到来源保留，跨文档请求数可能受缓存影响，不以HTTPdelta替代原生生命周期。

用户正常Quit9C6F后，旧34236/59127已关闭，不声称所有WebView broker退出。新main实际build EXIT0/19.73s，前后2577来源路径0差异（含2320 renderer）；exe SHA `E58484E937E40E6444A4F02FF236426140F94F438721F639BEFEEB2A28F63E4A`，`same-document-history-final-main-summary.json` SHA `1ABD55F9919341CF169553D3F9E2D1F7CA2A0F0B1A16D6B5FAF0B9625E4DCCF3`。该构建回执没有启动主程序或执行GUI；后续E584已实际启动并正常teardown，当前95E5接续范围见下文。不得默认打开用户现有data，也不借旧9C最低窗口实机证明宣告E584已验收。

**95E5 review及本轮定向GUI已收口**

用户已认可保留tauri/tauri-runtime-wry两份vendor。本轮两项P2均已修：仅Browser注册布局预留及侧栏展开dismiss，Browser只隐藏/detach并保留页面/输入所有权；Preview保持原有非破坏性侧栏折叠策略，未保存编辑不经closePreview被清空，Browser/Preview切换时交接管理。UA是可选诊断，属性不可用/读取失败/无效值返回None和固定reason；实际UDF/Profile/持久性/InPrivate校验仍必须成功，失败拒绝目标导航。

`closeout-ua-diagnostic-windows-unit-20261014`实际121 passed/0 failed/123 filtered，EXIT0、256来源路径0差异；stdout SHA `2CFEBF140EC26EBDEE5CE6E8C0B74993EB151A9839F5CE5A6D73C99D51E743A6`。`closeout-review-final-gui-main-20261014`实际build EXIT0/43.48s，2577来源路径0差异（2320 renderer）；新exe SHA `95E5B839FA12F0D03316215EBDFE649D4DAEB46D80E1E30CD8635E2C42524EC7`，artifact回执SHA `83AC5DD085F90BE5BDDDE757C29DEDA430E4D78DEE260852B4EEAAFDCAF14DE4`，exit SHA `6BA71067AECCE06B33C08F4DAA07DA0CF49F51F1BB7B5595297ACCB0EC258D5B`，均在上述conformance根，已独立读取与比较。构建仍有warning，不算新GUI证明。

本轮GUI测试根的 `review-closeout-validation.json` SHA `AF3820116AC0642D0DE37077AFF9511AF7C55E2C7D0C9914E58234F074BC6052` 已读：Root实际typecheck EXIT0、browser boundary PASS；UI owner提供Space6/56、Resize6/100与desktop boundary2095 sources/min880×600 PASS，来源明确，不称UI raw log。汇总记录95E5 PID38196/backend59312与frontend均200、该时点GUI in_progress；后续最终manifest另记实测结果，旧准备服务表不代替当前进程证明。

E584旧main后续已实际启动，其退出/Vite恢复及95E5实际GUI均按下述实际证据记录。Root为当前唯一GUI owner，不再要求用户重复退出/启动；不借组件fixture或9C窗口替代95E5的实际验收。既有9C minimum、History native及Bili未知原因的准确范围继续保留。

95E5本轮实机已取得百度History对应反例回归：真实主页输入“浏览器”并Return后普通搜索，首次Back从tb_enter到tb_pre、Forward恢复，fresh窗口同PID38196/window26021452，均可手动操作、无持续loading。`review-closeout-95E5/baidu-same-document-back-settled.json` SHA `EEDB3F46B5CB1B8D59EBD86DE977F9631B6817F793B52874229962377F6C3D2D`，Forward SHA `DEB43836DA96AA5549FEC214C0F82B75C07EE88E04C4937893BEBC7A1D1DEBD0`。

Back完整诊断由产品Copy→未提交本地地址草稿→UIA selected_text取得3842chars、JSON.parse，文件 `review-closeout-95E5/baidu-same-document-back-diagnostics.json` SHA `007AC4C32571526E520A47AE58D85DC8F4EEE2ADCA93D3DECD279D3F94BCEED7`，已独立读取。控制序号3/Finished/CurrentDocument/100%，7events仅原native attempt2/docgen1与Back host dispatch2；user2/agent0/page0/unknown_native1/redirect0/drop0/routing_lost=true。原200不是Back新HTTP，unknown仍unknown，不称完整HAR/全HTTP无请求；诊断草稿未提交。9C6F原FAIL保留，本轮最低窗口/输入实际结果见下文最终汇总。

**95E5本轮实际GUI最终结果**

`gui-evidence/review-closeout-95E5/final-95E5-gui-closeout-summary.json` SHA `31262E2825F2313B6B699E1234F6B2B702E0418ADBE87E87D094E82D382E4BB5`，记录于2026-10-14T01:41:00.9640397Z，52个附件hash已现场核对无不符。百度home86.241s、原生输入/Return后的search88.385s静置正常，同文档Back/Forward已实际闭合旧反例。新main与Vite是本轮准确组合，不借9C结果补新通过，也不倒改9C历史FAIL。

最低窗口以 `fixture-minimum-880x600.json`（SHA `12432CD02D3A2515E54C5786FF76FF89454C2788A65F1664E9234A38FD05BF1C`）的实际外框882×602对应客户区880×600；Browser/地址栏/聊天输入可见。`fixture-minimum-native-input.json` SHA `EAAB485F3F9C99147A288141B7E13C9C8940EAB9500605DE71C991BD2EC1F66F`及实际截图呈现 `GUI95E5_MIN_NATIVE`，独立HTTP witness trusted=true。收起后 `fixture-minimum-reopened-settled.json` SHA `D8B6C1ACF95611A70736061651BC91635C3DE9E918AB813A2AA5C8DFC86FC0B5`、宽屏 `fixture-wide-restored.json` SHA `33686DDF6925C729B04B338B6C13282BD450652EEEFC944700FE105CB4BAADA9` 均保留nonce和输入，1920×1032下184/300侧栏偏好恢复。三份before-hide/after-reopen/final计数同SHA `84F288BCF80918D8E418615CA8C16A629881408B360C26777F5B73A3BF800B63`，stable主文档一直1/delta0，nonce `e0a6d0f4-8f6e-45c2-82da-8771046aedba`、trusted=true。

GUI后 `gui-source-verification.json` SHA `ED605A2EC43246AD05C8968CC6245DF5E21F1E4CF2DA9A779F7968E5C159515B`，2577来源相对build-after漂移0。最后窗口1920×1032、Browser页保留运行，main38196/backend59312及frontend200；记录最后观察，不把旧PID视为未来仍存活。准确launch为测试根 `main-review-20261014.launch.json`，其stdout/stderr与实际DATA/logs分别归档。

历史 `service-recovery-20261014.json` SHA `9F588FB2A0016130F1779A980F17C4451B49146472068958DBA4A066828D5224` 对应E584/PID14512/backend60728：native/backend仍存活，5173无listener，恢复Vite后frontend/backend200且未改源码；原Vite退出原因unknown。owned `data/logs/2026-10-14.nomicore.log` UTC01:01:10四条正常teardown已读，不把localhost拒绝连接命名为已证实内嵌引擎崩溃。

该95E5历史批次的Windows/shared两P2及定向GUI可收口，不再要求用户退出/启动或新增验收。当时为debug+Vite，非bundle/installer；Bili子页原因当时未知，后续当前修补见文首静态UI A/B/A。Mac暂缓、强Session重叠与active Agent Stop未覆盖保持原范围边界，不宣告跨平台Goal完成或零风控。

**已确认的产品约束**

保持原生内嵌浏览器为主场景，用户与 Agent 使用同一个页面。兼容性、导航、暂停、存储和失败恢复继续在内嵌面板内优化。系统浏览器仅保留用户明确选择的部分场景功能及对照，不能成为主场景重构、自动回退或内嵌失败的验收替代。

**工具终止与窗口归属**

早期 Computer Use 在读取新建 Edge 匿名窗口时返回：`could not determine the current browser URL on Windows with enough confidence to enforce policy`。该回合停止了界面输入，没有绕过限制。后续回合重新初始化工具，已能选择真实 NomiFun 主窗口；它与外部 Edge 对照是不同证据范围，不把早期中止描述为当前内嵌主应用仍未启动。

Cua 的 Edge 创建入口为 unavailable，listBrowsers 为空。Sky 的 documented LaunchAppInput 只有 app，没有 Profile/env/启动参数；不能拼接 undocumented command line。有关 API 见本机 computer-use 插件的 `docs/api.md`，开始后续允许原生操作的回合时重新依照当前 skill 和工具状态执行。

新匿名窗口 HWND `28903512` 没有访问测试 URL；原 Edge 普通窗口 HWND `6619560` 的标题及标签数量未改变。两个窗口共享 PID `2244`，不得终止该进程来清理本次窗口。需要清理时，仅在允许操作且重新识别归属后关闭新窗口，或由用户手动关闭。

**本次隔离准备**

临时根：`C:\Users\MINISFORUM\AppData\Local\Temp\nomifun-browser-main-gui-1a9eb67ef82c44acb7cd6131b337a431`。data/work 位于该根下。canonical Session 为 `01a13383-5c6d-7011-8e73-f25e60e7a345`，通过正式 `browser_gui_fixture --user-browser-only` 准备；准备阶段数据库已关闭，没有 Agent Browser grant，model_calls 为 0。

第二 canonical Session `01a13543-dbf4-7150-9d1e-fc42706eb8cc` 已通过正式 API 追加。实际 READY stdout、exit 0 与空 stderr 核对，准备进程正常清理、第一 Session 不变。Browser resource bindings/grants=0、Turns=0，唯一 enabled capability 为合法非 Browser `agent.tool-discovery`，未启动脚本模型服务。记录 `gui-evidence/secondary-session-preparation.json` SHA256 为 `0A0DCB46D257F1C42317E8FC910CE808B72D52D4A523DB226E2EB62BB3B12E9B`；前序 fixture 校验/路由失败保留其范围，最终准备通过没有证明主 GUI Session 切换。

随后只修正中英暂停提示为“暂停会保留可用页面内容” / “Pausing retains usable page content”，保持当前标签/其他标签的边界，未改 renderer/Host 行为。9C6F 的 255 路径构建快照不包含 locale，当前 Vite dev renderer 读取最新字串；桌面 UI 边界 2094 sources/min880×600 与 browser boundary 均实际 PASS。字串和边界检查仍不是 GUI 操作证明。

| 已创建的自有准备服务 | 准备时 PID | 端口 | 原 owner 的 exec handle |
| --- | --- | --- | --- |
| 当前 renderer 的 Vite | 43976 | 5173 | 81497 |
| browser-main-gui-fixture.mjs | 44692 | 49563 | 55438 |
| canonical user-browser-only fixture | 12684 | 65034 | 58896 |

这些是历史准备事实。后续必须重新验证 PID 的 exe/command、监听和 HTTP 状态，不能仅凭 PID 或旧 port.json 操作进程。只收束本任务仍匹配的服务；服务死亡时正常重启，不结束占用端口的未知进程。

**后续交接与范围边界**

1. 9C GUI范围已收口，其同文档History缺陷现有D74源码修复、60C/72E定向native和E584新main构建证明。当前95E5已构建并接续启动，对应History、880×600输入和显隐/宽屏恢复均已实机通过，本轮Windows/shared定向范围收口，不再要求用户重复退出/启动；不以旧URL/history测试、full Back成功或build替代新GUI。
2. 每次接续核对实际95E5 executable、新PID、动态backend、Vite、owned data/work和准确窗口，原9C PID34236/59127已结束，不把旧PID或port.json当当前事实。既有MODNet预热/回退单独记录，不伪造缓存或改网络/资源；未知实例不操作。
3. 保留已取得的最低窗口、popup/恢复和基础Session范围；强busy-gate重叠、零Turn下未exercise的active Agent Stop及未执行916仅说明边界，不引入无限额外矩阵或发布gate。源码变化时做最小充分回归，不用phone/mock toolbar代替实机。
4. 两站正常静置和已观察交互分别保留；Baidu首次SPA Back与Bili搜索pending分开，不把pending/登录promotion当验证码，亦不承诺零风控。95E5后续History实机回归已按同一tab/doc安全trace收口，不自动刷新搜索或重放点击。
5. 外部浏览器仅作可证明条件的对照。普通、managed、private 环境分别标注；无法固定 Profile/操作条件时如实表达限制，不读取或复制用户 Cookie，不为通过而解验证码或重复刷新。

macOS 已由用户明确暂缓，见[独立 macOS 交接](2026-10-13-browser-macos.zh.md)。本文件不要求当前 Windows 继续该平台工作。后续macOS可按实际WK契约补验证同文档Back/Forward保留文档状态与跨文档真实生命周期；当前只记交接建议，不修改Mac源码或其交接验收状态，不自行提交、推送、部署或新增 CI。
