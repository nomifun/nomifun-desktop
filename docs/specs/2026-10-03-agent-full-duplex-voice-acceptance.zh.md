# Mobile全双工 / Desktop协作验收

2026-10-07用户已明确授权本轮commit+push，覆盖此前等待验收再提交的Git限制；不代表本清单真实pending项已通过。PR、发布、部署与可见GUI/原生app安装运行仍未授权。代码提交/推送与产品用户验收分开记录。

2026-10-07；代码方向以同日Mobile方案、执行Prompt与 [SDK 重构补充](2026-10-07-mobile-voice-sdk-refactor.zh.md) 为准。**当前SDK重构源码、定向行为和Android构建/静态产物已通过；Swift、真设备、云模型与真实体验验收未完成，未授权commit/push/PR/发布。** 本文是当前唯一验收清单，原Desktop W0–W8 UI与全局文字改造不再作为交付目标。

## 底线与环境

主文字发送、排队、暂停/恢复、停止/纠正、模型/Agent选择、工具/审批/权限、完成、历史与恢复必须保持最新upstream行为。voice显式工作指令进入原canonical owner，按原规则影响指定工作；mic暂停、停播、结束voice、后台/断网绝不自动停止Desktop任务。

Desktop源码02f7b677d；Mobile源码c4ecae674加本工作区改动；完整旧修改在build.noindex/mobile-voice-reframe-20261007-d655ab23，旧stash继续保留。本次没有执行真实用户数据升级或reset。

注意：最新upstream自身的nomifun-db/src/agent_store_clean_cut.rs会在识别到旧Agent代际时clean cut，旧Agent数据不迁移。它是本次远端更新带来的既有行为，voice没有修改Main DB/bootstrap。禁止直接用现用旧data_dir验收；先完整备份，再使用隔离副本或新临时目录审核升级。未知/损坏lineage须fail closed，不恢复旧reader、不改写身份、不拿编译测试声称旧历史已保留升级。

当前机器另有GUI自动化。用户2026-10-07明确推进RN Web，允许独立无头Chromium及API-only服务；仍禁止Tauri、可见浏览器、模拟器或原生移动app。使用新data root/已存在独立work_dir、127.0.0.1:0及独立browser context，不调用dev/free-ports/kill-stale或附着现有浏览器/进程。当前本地证据未使用真云模型或物理音频设备。

## 本地可运行验证

Desktop仓库：

    bun run typecheck
    bun run check:desktop-ui-boundary
    bun run check:i18n
    bun test ./ui/src/renderer/services/SpeechToTextService.test.ts ./ui/src/renderer/hooks/system/useSpeechInput.lifecycle.test.tsx
    bun run check:agent-session-boundary
    bun run check:voice-boundary
    bun run check:voice-contracts
    bun run test:voice-smoke-runner
    cargo test -p nomifun-agent-session --lib
    cargo test -p nomifun-agent-runtime -p nomifun-chat-model-broker --lib
    cargo test -p nomifun-agent-execution --lib
    cargo test -p nomifun-conversation --lib
    cargo test -p nomifun-voice -p nomifun-voice-core -p nomifun-audio
    cargo test -p nomifun-model-invoke --lib voice
    cargo test -p nomifun-app --no-default-features --lib voice
    cargo test --manifest-path tests/fixtures/voice-external-adapter/Cargo.toml

Cargo/合同生成器串行，最后契约使用独立voice-contracts，不要求在主Agent schema中生成voice。Mobile仓库按docs/VOICE.md运行typecheck、i18n、纯行为tests、Android/Apple autolinking及隔离export；当前原生build必须包含固定AudioAPI SDK与其必要voice-only保证，不能再以已退役的本地module证明。

无头服务需要时，使用真实web CLI的 --api-only --host 127.0.0.1 --port 0 --data-dir <隔离临时目录>，实际端口从该目录port.json读取；认证方式保原产品策略。不要启动普通dev脚本，不共享现用数据。本轮以独立新编译binary执行过真实认证API-only验证并已关闭，报告在build.noindex/voice-api-smoke-mux8ezp8-v8hd97c；这是HTTP认证/目录/off惰性证据，不是声音、真模型或视觉验收。

纯端口harness、本地WS和mock fixtures只证明代码合同/生命周期/来源/ACK。静态导出与autolink只证明打包发现。指标没有样本时为null/pending，不能替代真实设备p50/p95。无头runner的correction_applied_ms使用应用回执request_kind=steer，而非供应商工具args；测量区间是work_trigger_received_to_canonical_applied，不是语音结束或实际听感。恢复时停止上传并保心跳，InputReleased归还额度而不算模型准入，只有当前epoch/generation实际Ready+Capturing ACK允许继续；不重发被丢弃尾帧。

## 真实入口

### 当前共享UI原生产物

新Android包为Mobile/build.noindex/voice-sdk-ui-20261007092140/nomifun-mobile-sdk-ui-release-test-arm64.apk，59976185B/SHA256 c2e51872ff5e4cb6e7feb3cac45c71db436aebdf001d34cf525afbd080b4afa6。实际Release48528 exit0/1m9s、256src对应构建副本与前后SHA，6项真实产物核对及aapt2/ZIP16KiB通过，当前Hermes已嵌入。current-ui-packaging.json/build-owner-terminal.json保留来源、终态和旧previous-*；TEST签名、未安装或运行，不算设备/声学/云验收。

当前iOS JS/Hermes报告在Mobile/build.noindex/voice-ios-js-current-20261007-7f21c9/report.json：exit0/2137modules/46assets/5794000B，SHA57a3c77d24f4d0d246014fd7c717a9f74888a3c17bba906a89594e5d0b8ce864；source/SDK前后一致，进程终态已确认。它不替代Swift/iOS native包或设备；这里只完成当前共享UI打包待办，下方真实矩阵仍需验收。

### RN Web本地验收

Mobile执行bun run voice:web:export --out-dir build.noindex/<全新导出目录>；node scripts/voice-web-server.mjs --export-dir <该目录> --backend-url <显式origin>仅监听127.0.0.1随机端口，不自动打开浏览器。自动化用bun run e2e:voice-web --export-dir <目录> --backend-url <隔离backendorigin> --data-dir <新backenddata> --report-dir build.noindex/<全新报告目录>；--media-only可仅重验媒体。Playwright通过NOMI_E2E_PLAYWRIGHT借用外部runtime，NOMI_E2E_EXECUTABLE_PATH指定实际测试浏览器，始终headless。独立数据证明与完整复现见Mobile docs/VOICE.md、docs/TESTING.md，不用dev默认端口或现用数据。

当前voice-web-evidence-20261007-ui9/report.json有15项通过、264份源输入一致、0page errors、Chromium149；SHA256 860bde0a8e6ceceabc6144727a61eb7af8c0faf38f06796b2793f8d2f5cfd2a8。7项真实RN页面/生产API和8项真实AudioWorklet合成WAV合同分别记录；phone/wide/dark截图已查看，用户视觉/完整交谈验收仍需确认。动态theme、未启动不申请mic/媒体/store、voice导航/404后草稿保留通过。

API-only缺真实Browser/Computer roles，General创建422不算成功；只显式创建发布的chat.minimal测试Session，不改默认、不伪造绑定。媒体合成WAV/模拟disposition不证明模型或听感，cloud_calls=false、p50/p95=null。JS阻塞时worklet可停编码/发送，不能直接stopMediaStreamTrack，不能补证native前台政策。backend-close.json确认SIGINT/application/terminal cleanup及PID36056/端口60007不存在；proxy/browser close确认。旧SDK APK属于之前源码时点，新共享UI的native包需重新打包；两云、完整工具宿主、物理设备/声学/LAN-TLS/负载、native/Mac与用户验收继续pending。

Mobile配对原Desktop实例后，在会话标题栏进入Voice页，选择具体Session与独立语音模型/adapter/connection role/精确transport，保存VoiceProfile，然后明确开始。保存不录音，不改工作模型或Agent binding。只显示实际端点支持的transport：native目前relay，H5可relay/native RTC；不能安装了Expo Go就当原生模块可用，不能悄悄改用ASR+TTS。

业务WS、voice媒体与控制分开。API为 /api/mobile-voice/v1；GET capabilities/catalog/availability，PUT profiles/{id}，POST profiles/{id}/probe与sessions、sessions/{id}/work、control、attachment、source-context。连接probe仅由用户手势检查已保存profile，真实握手/ACK/关闭不代表声音、设备或工作体验通过。媒体票据在短期子协议，禁止URL/长期storage存token。每秒真实前台心跳，3秒租约；mic暂停仍有心跳，后台停止心跳/设备而Desktop工作继续。

工作纠正默认safe_boundary；显式选择SupersedeModelStep只对随后voice启动的任务安装owned操作端口，不升级已有普通文字Turn。中途纠正须真实producer/opening退出及join证明；已准入工具先结算，关闭失败不能开启后继步骤。目标曾变化且供应商无法证明转写clock时，相对纠正/取消会显示人工来源目标确认卡片；只可确认当前前台端点中完整匹配的来源revision和精确任务呈现，旧呈现、观察端、无目标或来源缺失都不可确认。被阻止的原请求尚无reserve，确认重prepare它；有未知准入结果的旧operation仍lookup-only。能够证明来源原目标A的相对指令不能改绑B。

独立数据入口：GET /api/mobile-voice/v1/data/export返回32MiB以内自一致SQLite snapshot及schema/voice-sequence头；DELETE /api/mobile-voice/v1/data/sessions/{id}显式删除自身voice事实与profile。普通主Session删除不等待voice，专属监督只确认真实tombstone后删除voice事实、保Non-Agent profile。持久删除意图保owner和profile模式，重启不改变意图。恢复snapshot不复活mic/输入lease、不盲重发工作；原activation namespace与当前root/generation不符或未知的未准入意图deferred，不自动提交；scope/namespace不匹配记录保留为历史而不自动入模。

有厂商凭据、隔离已配置Session及事先准备的PCM/WAV时，可显式运行：

    bun run test:mobile-voice-live --run-live --base-url <认证实例> --agent-session-id <原Session> --binding-version <版本> --provider-id <厂商ID> --model <精确型号> --audio <录音路径> --source-identity <当前40位源码SHA>

产品认证token从NOMIFUN_VOICE_SMOKE_AUTH_TOKEN读取，不写命令行或报告。脚本只观测媒体和真实工作回执；不伪造played，不自动cancel工作，结束仅关闭voice。真实听感由实际Mobile端记录。

## 双供应商 × Android/iOS/H5矩阵

当前设备层固定react-native-audio-api0.13.6（MIT），旧八份native bridge/Base64生产路径已撤，原SHA副本在Desktop/build.noindex/voice-sdk-refactor-20261007/previous-voice-audio。新facade保原媒体身份/格式/预算/额度与工作边界；SDK原生voice-only策略默认false，已接入跨JS/server ACK duration-age预算、原生age monitor与过期锁存、语音处理、后台/interrupt/route禁止自动mic恢复及真实sink clock/flush。没有新增设备engine，普通SDK caller默认不变，不同SDK sink timeline不伪合并消费。当前源码、本地行为及Android构建/产物已有独立证据，不把SDK安装或旧APK记为当前通过。

本地行为证据覆盖SDK ID与既有InputAdmitted/InputReleased精确对应、旧generation清除、sink-clock/unknown Played与close unknown合同。真实设备仍须验证后台原生停设备、人工恢复、实际消费和清理结果；普通SDK caller默认false保持原行为。本次不改UI、主文字或后端任务链，SDK不是vendor SDK，手机不增加Agent/模型私钥。以下真实体验矩阵继续适用于新SDK。

StepAudio 3原生typed-tools与GPT-Live原生metadata delegation分别验收，不把metadata伪造name/arguments。每个供应商分别需要合法凭据、模型权限与可用voice connection role；本地协议Supported不是云账号权限证明。

| 场景 | 必须得到的结果 | 当前 |
| --- | --- | --- |
| 开/关与旧Desktop | 未启用无mic/network/store依赖；旧服务无能力时文字仍可用 | 本地证据/实际待验 |
| 文字并发 | 同Session正常发送/排队/取消/暂停恢复/工具审批/历史模型切换按原行为；记录性能与资源竞争 | 真负载pending |
| 原生双工 | 正在听/说仍理解插话与附和，噪声/回声不变工作 | pending |
| 延迟与音频 | 真实外放/有线/蓝牙AEC、抢话停播、帧时长/年龄/backlog/underflow；实际采集p50/p95 | pending |
| 原任务纠正 | 已听输入对应原operation/generation；晚A不操作B；工具已准入先settlement；未知效果不重放 | 本地fence证明/真运行pending |
| 来源消歧与去重 | 未知时钟不绑定latest；明确人工目标确认可继续原未reserve输入；新call ID不重启同一任务；更高ASR revision只纠正原任务/queued CAS | 本地证明/真运行pending |
| 纠正完成判定 | received/queued/pending_boundary/applied/terminal清楚；无abort+join证明不报即时替换已完成 | pending |
| 审批 | 具体当前question/action/版本/来源/呈现一致；附和/tentative/模糊yes/forced-click不授权；重放查原CAS回执 | 本地证明/真运行pending |
| 媒体生命周期 | 暂停mic保播放，停播保mic，结束voice保任务；后台/锁屏/来电/权限/路由切换立即停设备，返回不自动录音 | pending |
| 断网/进程死亡 | 前台lease到期释放，dispatcher未准入查原key，不变另一binding；关闭abort+join/未知finalization诚实报告 | pending |
| 多端 | observer不拿mic/model；接管明确撤旧epoch；导航不迁移已绑定Session | pending |
| 来源/恢复 | 初始正文非instructions；撤回清恢复缓存和未播；已播只留真实timing，未知wordalignment不保整句为已听 | 本地证明/实际pending |
| profile/认证/数据代际 | disabled provider也能关profile；secret/profile/auth/root/generation变化撤lease；新scope不恢复旧数据 | 本地证明/实际pending |
| 删除/导出 | Main成功不因voice失败改变；owned voice独立retry，snapshot自一致；恢复不自动执行或录音 | 本地证明/真实操作pending |

2026-10-07用户已批准接受Android SDK许可。当前refactor/nomifun-mobile为RN0.86.2/Expo57.0.11，隔离JDK17、SDK36、Build Tools36.0.0（AGP另需35.0.0）、platform-tools、CMake3.30.5及NDK27.1.12297006已由官方包校验并真实安装；另一clone未改。

当前SDK补丁已通过patchedDependencies持久化，SHA256 `d4f2119bead586f8371f2c329b473a4b9a87c7da82e5d43163bfae0af939b1ef`。endpoint17项/89 assertions，Mobile50项/259 assertions、typecheck/i18n通过；MSVC真实SDK header13组/71 checks通过，仅证明原生预算/年龄/ACK/monitor逻辑，不证明设备或RN runtime。

当前SDK `:app:assembleRelease`（59455）exit0，4m49s、576tasks（510执行、66up-to-date）；6项只读证据通过，绑定253份Mobile src、69份SDK实际修改（57非iOS、12 iOS）、persisted patch/lock和隔离副本。真实DEX class_defs有AudioAPIModule且无旧NomiFunVoiceAudioModule，生成RN PackageList注册AudioAPIPackage，实际AArch64 SDK shared library在包，APK Hermes与生成bundle SHA一致。当前APK在Mobile/build.noindex/voice-sdk-verified-20261007/nomifun-mobile-sdk-release-test-arm64.apk，59974745字节，SHA256 `c75166268c7b287cc6a4bb0bab1dbf20a1fc980e6d29a4e7d8e06c4237bfd08d`；同目录报告、日志、Manifest、alignment与native-policy结果可复核。

当前包aapt2确认RECORD_AUDIO、MODIFY_AUDIO_SETTINGS与既有usesCleartextTraffic=true，无foreground service权限/服务或debuggable=true。ZIP16KiB和29个ELF PT_LOAD16KiB静态检查通过。Release仍为Expo模板TEST签名，不是发布。新H5 export50routes；iOS JS2135modules/5792310字节Hermes通过，后者不替代Swift/macOS/Xcode或真机。真实SDK兼容修复及过程PATH限定见SDK补充；没有关-Werror或降版本。以下原bridge记录属于历史基线，当前包必须使用本段独立证据。

以下为SDK重构前证据，全部保留但不认证当前实现。HEAD已有的HTTP意图由正式config plugin写入Manifest；当时自写音频module真实compileDebugKotlin通过，补固定punycode2.3.1依赖后Android/iOS导出2138/2059模块Hermes。独立短副本完成Android Debug arm64 APK（359tasks实际执行）；APK/日志/7项只读报告在Mobile/build.noindex/voice-android-verified-20261007，证明当时DEX、Expo注册及包内usesCleartextTraffic=true、RECORD_AUDIO、MODIFY_AUDIO_SETTINGS。未安装或启动app。

重构前assembleRelease（83661）exit0：7m51s、569tasks（523执行、46up-to-date），2138模块以dev=false经Hermes -O嵌入APK，assets/index.android.bundle为4812488字节。8个voice源SHA与当时产品一致。Release arm64 APK在Mobile/build.noindex/voice-android-release-verified-20261007/nomifun-mobile-release-test-arm64.apk，56630782字节，SHA256 eaf5c9fda34a478a1a091907811e0a56b13e6c61ff0186a1181f19b769b354fb；同目录日志、android-release-evidence.json、root-supplement.json、最终Manifest与alignment记录保留。7项只读release证据检查全部通过，证明该旧包真实DEX/Expo注册、嵌入JS及27个arm64 .so；新SDK必须另有源码身份、构建与产物证据。

aapt2确认Release包内两音频权限与usesCleartextTraffic=true，无debuggable=true；zipalign -c -v -P16 4成功，27个ELF64 PT_LOAD alignment均不小于16KiB且offset/vaddr同余静态检查通过，不能据此声称真机16KiB页兼容。Release variant仍使用原Expo模板TEST签名，非正式生产签名/发布，未读取生产签名key；正式签名须另获授权。

iOS Hermes导出不代表Swift/Xcode原生编译，仍需macOS/Xcode/CocoaPods及真机；Android真机、两云模型权限、LAN/TLS、视觉、声学及并发p50/p95继续pending。H5还需安全上下文和实际麦克风/播放权限。正式生产签名及分发不属于本任务完成条件，未来如需要须单独授权。未启动或安装app、emulator/GUI，没有commit/push；GUI及提交/推送禁令继续，编译与包内静态检查不替代真实验收，整个Goal未complete。

用户需明确完成本矩阵验收；验收后提交/推送仍必须另行明确授权，没有默认commit+push。
