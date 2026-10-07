# Mobile 全双工 / Desktop 协作实施状态

2026-10-07用户另行明确授权本轮commit+push；此前禁止自动Git动作的记录属于授权前状态，当前可提交/推送Desktop与Mobile任务代码。真实两云、工具宿主、设备/声学/Swift及用户验收仍pending，PR/发布/部署未授权。推送前fetch发现Desktop远端在02f7之后新增14提交至81a8e035b，将保留语音提交并合并该远端，不force-push；02f7的前批验证保留其源码时点，合并后的检查单独记录。

2026-10-07。当前目标以同日用户决定、执行Prompt、M0–M8方案与 [SDK 重构补充](2026-10-07-mobile-voice-sdk-refactor.zh.md) 为准：现有RN Mobile可选全双工，Desktop保留文字与一次性ASR；主文字任务链路零功能影响。设备层已完成react-native-audio-api0.13.6接线与持久voice-only补丁，旧八份bridge/Base64已退役。当前SDK定向行为、MSVC header、Android Release及静态产物核对通过；Swift、真设备生命周期、两云、LAN/TLS、视觉/声学、负载及用户验收仍pending，整个Goal未完成。

## 源码与保留

Desktop从b25ca352559fe99b79c6938237c94f32a77f0a1e fast-forward到 **02f7b677d5a1efb4456d425a44f037faa4ee3c31**，吸收82个远端提交。采用最新upstream加专属voice扩展，没有整包覆盖旧patch。

完整快照在忽略目录build.noindex/mobile-voice-reframe-20261007-d655ab23：manifest记录277项tracked/untracked变更，其中272份当时存在的文件及5项删除状态；当前再次核对272份保存内容的SHA256，全部匹配。manifest.json/working-tree/base-tree/patches保留原文及删除记录；withdrawal.json记录撤回174项tracked及63项旧Desktop/共享路径untracked。先保留再撤回，没有用户数据删除、reset/clean或历史身份改写。先前stash d6b552dcb4024da63214348e02ab0fdab4cb0c25当前Git对象仍存在，不可丢弃。

Mobile采用同级refactor/nomifun-mobile，原main/c4ecae6740f01d27529b6f2a0f7c9017abffc2ba；另一bak/mobile clone未改。没有commit/push/PR/发布/部署。用户明确推进RN Web后，仅运行独立无头Chromium及全新data/work目录、127.0.0.1:0的真实API-only服务，全部已关闭。未启动Tauri、可见浏览器、模拟器或原生移动app，未碰现有自动化/profile/端口或真实用户数据。

## 当前范围

| 范围 | 代码/本地证据 | 真实验收 |
| --- | --- | --- |
| M0–M1 | 新Prompt/方案、全量快照、更新02f7；Desktop全双工UI、Agent voice tab/save gate及全局W5改动撤出 | 源码交付 |
| M2 | 独立合同/profile/schema/TS；只读目录/role/注册表；专属认证API/票据；真实storage-generation/root、profile/config/credential与前台租约 | 两供应商权限、配对/TLS待验 |
| M3 | 原writer上的opt-in start/binding/floor、cancel/steer/native generation fence与原审批CAS；voice-started可选owned model-step关闭证明；原operation回执 | 并发纠正、工具结算、审批待真实运行 |
| M4 | 当前共享UI Android Release/6checks及iOS JS/Hermes通过；本批Web预算/clock/flush/close和voice主题修复，source-bound导出及15项真实无头浏览器检查通过 | Swift/iOS native包、真机/完整交谈视觉与声学待验 |
| M5 | 初始typed source与静默ACK；撤回覆盖恢复缓存/未播；同namespace/binding/floor/lease恢复committed用户与消费timing；撤正文留timing | 实听、回声、撤回待验 |
| M6 | 主Preset/Snapshot/DTO/defaultmodel/SDK/DB/普通Mobile hooks保持upstream；None原payload回归；voice导航/故障后草稿保留通过，UI显式palette不改变普通默认调用 | 完整宿主/并发负载/响应时间待验 |
| M7 | 实际观测入口、source-bound RN Web与分层pending证据规则 | 两云/完整工具宿主/设备/OS/可见GUI仍待外验 |
| M8 | 独立SQLite snapshot/owned delete意图/确认tombstone后清理，不挂Main备份/删除结果链；状态/验收/脚本 | 用户验收未完成 |

## 已通过的本地批次

### 当前共享UI打包（2026-10-07）

Android实际assembleRelease（48528）exit0，1m9s、576tasks/27executed/549cached；256src与隔离副本及全输入before/after一致，SDK69/patch未变。新包在Mobile/build.noindex/voice-sdk-ui-20261007092140/nomifun-mobile-sdk-ui-release-test-arm64.apk，59976185B、SHA256 c2e51872ff5e4cb6e7feb3cac45c71db436aebdf001d34cf525afbd080b4afa6。current-ui-packaging.json与6项真实产物检查确认DEX、SDK注册、AArch64库及本轮生成/内嵌Hermes；Hermes4944048B/SHA36304c75544fd263a4a9dbfb66cc31ff1c5495ece6fb8a5b22c8b5c8dffb80b2，已区别旧包。另实际aapt2/ZIP16KiB检查通过，两音频权限存在、无FGService/debuggable=true；TEST签名、未装/未运行。旧previous-*保留，owned daemon24596已不存在，build-owner-terminal.json记录终态。

当前iOS JS/Hermes报告在Mobile/build.noindex/voice-ios-js-current-20261007-7f21c9/report.json：标准Expo export exit0、2137modules/46assets/Hermes5794000B，SHA57a3c77d24f4d0d246014fd7c717a9f74888a3c17bba906a89594e5d0b8ce864；266份输入与1394SDK文件前后一致，进程/子进程已消失。它不编译Swift或运行iOS设备。当前共享UI打包以这两个新目录为准，旧证据保原source时点，Web15项/source264/ui9不变。真设备、Swift/iOS native包、两云、完整工具宿主、LAN-TLS/负载与用户验收仍pending。

### 最新RN Web批次

Mobile/build.noindex/voice-web-export-20261007-ui8有50routes/1768Web modules及264份源码输入一致的manifest。voice-web-evidence-20261007-ui9/report.json记录15项通过、0page errors、实际Chromium149.0.7827.55；报告SHA256 860bde0a8e6ceceabc6144727a61eb7af8c0faf38f06796b2793f8d2f5cfd2a8。390×844浅/深色与1024×768仅是独立Mobile产品的截图，已实际查看；没有Desktop renderer手机模拟。发现动态主题不一致后，仅voice稳定Appearance订阅及显式palette修复，重新导出/执行通过；普通Screen/Button未传可选themeColors时保原默认theme。

7项真实RN页面/生产API检查含首次设置/JWT、真实canonical测试Session、配置不获取mic/媒体或打开voice.sqlite3、动态theme、voice导航及目录404故障后文字草稿保留、刷新绑定。8项真实AudioWorklet检查用确定合成WAV与模拟server disposition，覆盖非零PCM、全局sequence、精确credit/expiry、350ms JS阻塞后0旧尾帧、generation flush/actualclock/unknown，以及close后track ended/0PCM。Web新增worklet→JS→ACK同一duration/age预算、输出MessagePort预算/flush ACK/clock及close unknown。worklet不能直接stop浏览器track，阻塞时只证明编码/转发停止与旧尾帧过滤；物理track stop等待JS处理。cloud_calls=false、声学pending、p50/p95=null。

实际后端93份源与旧构建manifest匹配，binary SHA256 2dee537ba7b3c0054f8c5ef394a2ccda8920191a196141aa2bebf32b3fadd669。API-only缺真正system.browser_use宿主，普通General创建422；没有改默认或伪造nativeRole，仅显式发布的chat.minimal测试Session用于UI/草稿。完整General工具工作待真实Desktop nativeBrowser/Computer owner。受影响31tests/173assert及最新typecheck通过，Node代理8项真实HTTP/WS检查通过；Bun该WS代理不兼容，listen前fail closed。

全部运行句柄已终态。backend-close.json保存SIGINT/application/terminal cleanup，PID36056/端口60007均不存在，PTYexit1如实保留；浏览器/代理close已确认。前批SDK native patch未改，先前APK仍对应其构建时点，不补证这批后续共享UI源码的native包。真实两云、完整宿主、设备/声学/LAN-TLS/负载、native/Mac及用户验收仍pending。

以下是对应执行时源码的前批证据。该批Mobile媒体/autolinking/Android编译与APK条目属于SDK重构前，不能用于认证当前SDK；新SDK证据独立记录在“当前SDK重构”中。

- 最新Agent Session102、Execution102、Conversation owner26、Runtime342、Broker20项通过；含普通start/cancel/steer的原payload字节、None原模型请求、工具结算、历史/恢复及实际producer清理。Runtime一项忽略是需要仓库外caller-owned closed journal的既有用例。
- model-invoke voice filter73项通过：含原batch、本地WS/signaling、初始fact/local replay真实ACK、32KiB来源与4KiB回执控制burst、独立256KiB wire-byte预算，未调用云服务。
- 独立voice合同6、core公开port5、audio1通过；voice末批60项通过，含未知clock人工确认、晚A不操作B、新call ID去重、ASR revision原目标/queued CAS、非零generation不续租、queued缺first-claim证明保持未解析、持久删除CAS和实际来源撤播。
- Mobile34项177 assertions及最后UI修复后的typecheck/i18n通过；当前UI再次隔离H5 export50静态routes通过（build.noindex/voice-source-ui-20261007），仅打包证据。Desktop ASR6项20 assertions/typecheck/i18n/880x600边界的前批证据保留，Android/Apple autolinking实际发现本地module仍仅打包发现证据。
- 无头smoke runner最新12项通过，新增恢复暂停/实际ACK/心跳、InputReleased额度和Live无工具args的应用request_kind指标；只有脚本与脱敏/有界媒体证据，无声学验收结论。

计数以当前02f7代码的真实执行批次为准，不混用旧299/b25证据。

前轮App完整voice filter20项通过：actual assembler/source验证、同Turn first-native-claim证明、确切owned supersede来源过滤、实际broker producer/opening退出、原审批CAS/voice-off原payload及旧namespace不准入。本轮新增lazy recovery两项actual Host测试通过：knownQueued原key只准入一次、UnknownDispatched无重发，namespace/binding/context-floor不符durable defer；Service19项及最终neutral metadata回执/因果顺序用例通过。独立合同check、voice边界、Mobile类型与双仓diff check通过。UARC前批0 anomalies，原macOS gap1；其他未变更的前批主链/外部port/ASR证据保留，未重复运行替代体验证明。

本轮App测试编译曾遇Windows rustc stack overflow；只在验证进程设置RUST_MIN_STACK=536870912后同命令通过，未修改仓库全局编译设置、运行时栈或模型限额。新Web binary在build.noindex/voice-api-bin-20261007构建通过。实际API验证报告及源码hash manifest在build.noindex/voice-api-smoke-mux8ezp8-v8hd97c：9项检查通过，涵盖真实auth拒绝/产品setup bearer/voice API v1及双factory目录/普通auth身份/错误Session拒绝/off无voice.sqlite3。隔离目录没有配置模型，catalog的role行验证是空集，非空shape另由注册表/Mobile测试证明；不把本HTTP验证算云权限或音频验收。关闭时观察到SIGINT、backend/terminal清理日志，随后PID和监听均不存在；PTY shell exit为1，未以该退出码伪报终态成功。

Rust输出保留上游既有dead-code/unused warnings；Mobile i18n检查通过，存在原动态调用未静态引用warnings。前批构建结果保留；当前SDK facade/voice-only补丁的新通过证据独立记录如下。两个仓库无本任务commit/push。

## 调整与限制

保护文字底线，撤出全局FIFO、全局工具围栏和SDK文本替换。新增模型流分支仅由明确选择SupersedeModelStep的voice-started Turn安装；普通文字None继续原模型流/策略。Broker真实producer及host opening task均有cancel、abort与join身份回执，Runtime只在证明完成后记typed VoiceModelStepSuperseded并继续同Turn；未准入旧工具不会执行，已准入工具先结算。无法确认清理时不启动后继步骤、不称applied、不取消后另起纠正任务。语音停播/抢话与任务纠正保持独立。

主Preset/Snapshot、digest、权限ceiling、历史schema/旧reader没有voice字段。独立profile、provider、journal和cleanup错误只影响voice。最新Agent规范不允许重新引入retired数据，旧兼容设计输入一并撤出。

前台明确启动/接管。后台/锁屏/来电/权限/设备先停媒体；返回不自动开mic，RTC重建保原暂停状态。Native仅广告实际relay，H5按真实API选择，无ExpoGo/native RTC假支持或静默降级。

M6零影响是功能/协议/权限/生命周期的底线；共机CPU、网络、云配额竞争仍需并发实测，静态对比不保证零物理开销。

/goal工具不支持编辑未完成目标正文；若卡片仍显示旧文，同日用户指令与当前Prompt/方案是实际执行目标，没有虚标旧目标完成绕过限制。

用户已明确授权接受Android SDK许可，隔离工具链与当前构建进展见下。macOS/Xcode与真设备/模型权限及GUI许可仍未提供；autolink、导出、编译或mock不能代替真实体验。

## 本轮继续核对

前批曾审阅并修复旧iOS bridge的资源所有权，但该bridge现已归档退役，不能用它认证当前SDK；对应Android Kotlin与APK证据保留如下。语音首次healthy runtime status=None、knownQueued显式lazy恢复及runner恢复/credit/Live纠正指标已有前批代码证据，本次SDK重构不改变这些后端工作规则。恢复仍不在voice-off启动或普通文字路径运行；原writer围栏与实际journal lookup-only reconciliation保留。

前批M0保留/更新、M1退出路径、M2真实HTTP认证/目录/off惰性、M3/M5/M6原caller与围栏均有相应证据，本次不改UI、主文字或后端任务链。M4当前SDK接线及必要原生策略源码、本地行为和Android构建/产物已通过，Swift原生与真机尚待验证；M7两云权限、三平台、LAN/TLS、设备/声学/负载p50/p95及用户验收未完成。M8当前入口/退役/状态验收记录保留，整个Goal继续未完成，不虚标complete或缩减成功条件。GUI与提交/推送禁令继续。正式生产签名与分发不属于本任务完成条件，未来如需要须单独授权。

## 当前SDK重构

现有RN0.86.2/Expo57固定react-native-audio-api0.13.6（MIT），由设备SDK统一PCM capture/play/context与权限，facade保原Identity/Generation/实际格式/duration-age、SDK sample ID与服务器disposition额度、实际sink-clock Played、前台人工恢复和close unknown。它不承载vendor协议、手机Agent或模型私钥；目标是删除自写device/DSP engines，不增加第二套设备owner或全局改变普通SDK caller。

旧八份native bridge与Base64生产路径已撤，原八份SHA副本在Desktop/build.noindex/voice-sdk-refactor-20261007/previous-voice-audio。没有新增device/DSP engine。默认false的voice-only SDK补丁已接入原生capture→conversion→CallInvoker/JS→server ACK duration-age预算、原生独立age monitor/过期锁存、语音处理请求、后台/interrupt/route停设备且禁止自动mic复活、actual sink timestamp/flush和close unknown。普通SDK caller默认不变；不同SDK sink timeline不伪合并为连续Played。

补丁通过Mobile `patchedDependencies` 与 `patches/react-native-audio-api@0.13.6.patch` 持久化，SHA256 `d4f2119bead586f8371f2c329b473a4b9a87c7da82e5d43163bfae0af939b1ef`。当前6项产物核对比对253份Mobile src、相对官方发布包排除CRLF-only后的69份SDK修改（57非iOS、12 iOS）、source freeze、补丁和锁文件与短构建副本一致。此处源码身份不证明iOS native编译。

本批endpoint定向17项/89 assertions，Mobile合计50项/259 assertions、typecheck与i18n通过。MSVC实际编译运行SDK header，13组/71 checks通过，覆盖硬件额度贯穿conversion/JS/server ACK、未知/重复/旧session ACK、无新hardware/JS情况下耗龄、超龄事实不被late ACK清除及native monitor；无设备或RN runtime证明。

当前SDK短副本 `C:\nomi-v-dc8670c5` 实际 `:app:assembleRelease`（59455）exit0，4m49s、576tasks（510执行、66up-to-date）。6项只读产物检查全部通过，真实DEX class_defs包含AudioAPIModule而无旧NomiFunVoiceAudioModule；生成RN PackageList注册AudioAPIPackage；APK包含实际AArch64 `libreact-native-audio-api.so`，Hermes bundle与该构建生成文件SHA一致。当前SDK APK保存在Mobile/build.noindex/voice-sdk-verified-20261007/nomifun-mobile-sdk-release-test-arm64.apk，59974745字节，SHA256 `c75166268c7b287cc6a4bb0bab1dbf20a1fc980e6d29a4e7d8e06c4237bfd08d`；同目录保留sdk-artifact-evidence.json、Gradle日志、Manifest、zip/ELF alignment与voice-policy-result.json。

aapt2最终包确认RECORD_AUDIO、MODIFY_AUDIO_SETTINGS、既有usesCleartextTraffic=true，无foreground service权限/服务或debuggable=true。zipalign16KiB与29个ELF PT_LOAD16KiB静态检查通过，不声称真机16KiB页或音频结果。仍用Expo模板TEST签名，未读取生产签名key，不是发布包。当前H5导出50routes（短副本sdk-web-export），iOS JS导出2135modules/5792310字节Hermes（sdk-ios-export）通过；后者不证明Swift/macOS/Xcode构建。

实际构建兼容修复包括RN CXX flags的语种适用与ctor次序、未启用FFmpeg的guard、vendor release assertion的unused值显式 `(void)`；Git Bash仅加入本构建进程PATH。未关闭-Werror、未降级RN/Expo/SDK、未修改系统PATH。新本地证据不替代M7真实体验矩阵；真设备生命周期、两厂商权限、LAN/TLS、视觉/声学、性能与用户验收继续pending。没有启动GUI或安装app，没有commit/push/PR/发布。

## 重构前Android原生编译与打包证据

以下旧源码/产物/日志继续保留；八份源SHA对应当时自写bridge，不能当作当前SDK通过。

当前工程为refactor/nomifun-mobile的RN0.86.2/Expo57.0.11，另一clone未改。Mobile/build.noindex/voice-android-validation 使用实际离线template prebuild成功，285份源/资源复制SHA一致，主7份config/package/lock保持原hash；原项目没有新增android/ios目录。生成输入：SDK36/BuildTools36.0.0/NDK27.1.12297006/AGP8.12.0/Kotlin2.1.20/Gradle9.3.1/JVM17，CMake3.30.5来自实际ReactAndroid配置。autolink已确认真实本地音频模块和麦克风/音频设置权限。

2026-10-07用户已明确授权“接受SDK许可”。Desktop/build.noindex/voice-native-tools 已对官方包完成SHA校验并隔离安装Microsoft JDK17.0.20.1、Android SDK36、Build Tools36.0.0、platform-tools、CMake3.30.5及NDK27.1.12297006；AGP另自动安装所需Build Tools35.0.0，许可记录已落盘。Java/Javac及Gradle9.3.1版本命令通过，没有全局安装或读取私密properties。

Mobile/build.noindex/voice-android-compile复制源与依赖，22个native项目均autolink到新目录，避免Gradle写原依赖。真实构建曾因官方React debug AAR下载截断失败；现已分段恢复278992434字节，官方SHA256 401033773b3f08057851265ac52667ce74029f30a1d782d13fcf4949f7c2eda3与SHA1均匹配；原.module/.pom也保留官方校验，隔离Maven树只供本构建使用。`nomifun-voice-audio:compileDebugKotlin`真实通过（2m2s、59tasks）。完整APK首次在长路径Worklets CMake/Ninja重复生成失败，日志保留；43个再生成输入存在、非futuremtime且GLOB一致，不能只凭对象路径warning断言直接根因。采用独立短物理副本C:/nomi-v-26c74720及短缓存后，原依赖源码/版本不变，实际`:app:assembleDebug`通过（7m46s、359tasks全执行）。原仓库未移动/清理，8个音频源文件与编译副本SHA一致。仅构建、无安装或运行app、无emulator/GUI、无commit/push。

Android Debug arm64 APK保存到Mobile/build.noindex/voice-android-verified-20261007/nomifun-mobile-debug-arm64.apk，91371370字节，SHA256 939fabf362addefdc586d180002583b3b98873a69dd75b667fac622589db3eb3。同目录Gradle日志、android-evidence.json与aapt2最终Manifest记录可复核；7项只读产物检查通过，真实DEX含NomiFunVoiceAudioModule且Expo modulesMap注册该类，最终包包含两音频权限及既有HTTP策略。CMake3.22.1由Worklets实际需要额外安装，与3.30.5并存。Gradle命令exit0，单用daemon PID43228已不存在；daemon关闭时另有cache contention handler已关闭的工具日志，未将它当成应用生命周期验证。

重构前实际assembleRelease（83661）exit0，耗时7m51s、569tasks（523执行、46up-to-date）。2138模块以dev=false打包并经Hermes -O嵌入APK，assets/index.android.bundle为4812488字节；8个voice源文件SHA与当时产品一致，该构建步骤没有新增业务源码。离线Release arm64 APK保存到Mobile/build.noindex/voice-android-release-verified-20261007/nomifun-mobile-release-test-arm64.apk，56630782字节，SHA256 eaf5c9fda34a478a1a091907811e0a56b13e6c61ff0186a1181f19b769b354fb。同目录保留日志、android-release-evidence.json、root-supplement.json、最终Manifest与alignment记录；7项只读release产物检查全部通过，证明当时真实DEX/Expo注册、嵌入JS及27个arm64 .so。

aapt2确认Release包内RECORD_AUDIO、MODIFY_AUDIO_SETTINGS与既有usesCleartextTraffic=true，未含debuggable=true；zipalign -c -v -P16 4成功，27个ELF64的PT_LOAD alignment均不小于16KiB且offset/vaddr同余静态检查通过。这些是包内静态证据，不能当作真机16KiB页或声音验证。Release variant仍使用原Expo模板TEST签名，不是正式生产签名或发布；未读取生产签名key，正式签名须另获授权。没有安装或运行app、GUI、commit/push，实际设备、两云权限、声学、视觉、性能和用户验收仍pending。

实际Android JS打包发现上游markdown-it10.0.0调用却未声明punycode，导致Native解析失败；已只加固定runtime依赖punycode2.3.1与锁文件，无其他版本升级、不改Markdown业务。真实Android export通过：2138modules/46assets/约5.8MB Hermes；短副本iOS export也通过：2059modules/42assets/约5.6MB Hermes。7项产物报告记录各HBC的SHA与大小；不代替Swift编译。另将HEAD已有但Expo57未消费的HTTP/LAN配置迁至正式withAndroidManifest插件；离线prebuild和APK最终Manifest实际含usesCleartextTraffic=true与两音频权限。普通hooks/API及音频源不因此改变；这是必要构建/配置修复，不是LAN/TLS、正式生产签名或声音体验验收。

## 本轮补齐与剩余验收

上一轮绿色测试不能证明目录→Mobile字符串角色合同、sources失效→实际撤播、observer完整快照、长通话来源窗口、ACK和Relay重连状态全部接通。本轮针对这些实际缺口补 production code，并完成上述定向整合。测试期间命名指令正例暴露英文动词前缀大小写问题，已仅对动作措辞作ASCII大小写兼容，opaque OperationId完整且case严格保留；没有增加虚构的ID长度合同。

明确可选的SupersedeModelStep已接自有producer cancel+join、同canonical Turn、typed voice-only discard及工具准入/settlement保护；普通文字None、实际broker opening取消与错step/op的来源过滤已有定向证明。来源clock未知且目标曾变化时不采用到达时的latest task；Mobile展示确切committed source revision与任务呈现，人工确认只重prepare原未reserve请求。新tool call ID按来源/归一化意图关联原持久operation；ASR revision纠正原canonical/queued目标，不启动第二任务。源bound_target不随current receipt续租至新恢复代际；opening0仅从同Turn/fence0/causation正确的首次native claim证明升级，缺证明保持未解析。metadata委派没有未知意图默认Start，有活动任务的模糊说法先澄清，明确新任务和原目标纠正分开。退役新policy端口取代的两个未调用voice wrapper及仅声明、所有调用者均None的activation_fence接口，保留上游既有未使用接口；实际工作围栏仍归canonical writer。
