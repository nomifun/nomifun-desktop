# NomiFun Mobile 全双工 / Desktop 协作执行 Prompt

2026-10-07最新Git授权：用户明确要求本轮commit+push，允许把Desktop与Mobile当前任务代码提交并推送，覆盖下文此前“验收前禁止commit/push”的等待条件。该授权不表示两云/设备/声学/完整工具宿主/Swift及用户体验验收通过，也不授权PR、合并远端PR、发布或部署。推送前保留无关工作、检查暂存范围与凭据/产物排除，不force-push或改写共享历史。

2026-10-07 用户新决定后的唯一执行指令，替代原 Desktop 全双工 UI 与全局文字链改造目标。当前方案：`2026-10-07-mobile-full-duplex-voice-implementation.zh.md`，设备实现另遵循 [SDK 重构补充](2026-10-07-mobile-voice-sdk-refactor.zh.md)。文件原日期仅保留既有链接，不表示旧目标有效。

## 新 /goal 正文

实施 Mobile 可选全双工语音前端与 Desktop 后端协作。Desktop 保留正常文字和一次性 ASR；撤回本任务加入的 Desktop 全双工入口，以及改变文字默认语义的共享改动。主文字会话任务链路的发送、排队、暂停/继续、停止/纠正、模型与Agent选择、工具/审批/权限、完成判定、历史和恢复不得因本功能改变。明确的voice工作输入必须经同一canonical authority按既有规则接纳，不能另起工作引擎。

更新 Desktop 最新远端并完整保留工作区和用户数据，以最新upstream加专属voice扩展重新集成，不能整包覆盖旧语音patch、恢复retired schema/reader或重写历史身份。Mobile 使用现有 React Native / Expo 项目 `C:\Users\rika0\code\nomifun\bak\refactor\nomifun-mobile`，不得改另一同名 clone 或另建产品前端；复用实际配对认证/会话同步，提供真实 Android/iOS/H5全双工端点、专业UI及权限/设备/前后台/网络生命周期。Desktop承担模型、工作桥与存储。

沿用并完成两个生产vendor-neutral adapter、独立VoiceProfile/版本route/credential lease、公开ports/core、voice-only工作和精确回执、初始typed source facts/来源修订撤回、已播保留、独立VoiceJournal、双端兼容与退役说明。缺凭据/设备/OS继续其余代码和本地验证，真实体验pending不能用fake/握手/编译补证。

用户明确完成验收且另行授权以前，禁止commit、push、创建/合并PR、发布或部署；验收完成也不自动提交。2026-10-07用户明确推进RN Web验收，允许隔离API-only服务和独立无头Chromium。仍禁止Tauri、可见浏览器、模拟器和原生移动应用，不触现有自动化、browser profile、端口或真实数据。

## 实施要求

读两个仓库适用AGENTS、最新Desktop docs/architecture/agent-session.zh.md、新M0–M8方案与状态/验收。最新用户指令优先旧W0–W8的Desktop UI/全局W5要求。先保存tracked/untracked与源码身份，选择性撤回和逐owner合并，不reset/clean、整文件ours/theirs或删除原草稿/队列。

正常文字API/DTO/SDK与领域调用者保持最新upstream。不得全局server FIFO、强制普通target/policy、改所有stream/ToolAdmission/steering markers、voice save gate、扩大默认ceiling/旧Source别名。新增能力必须严格opt-in且默认文字直接验证不变；不能隔离就撤回，不放宽底线。

Voice合同/profile/source能力独立于文字Preset/Snapshot及权限；保存不录音、不连接、不改工作模型和binding。voice失败/cleanup/journal不可用只影响voice，不改变文字原成功/失败语义。Voice/文字共唯一canonical owner，不复制任务、上下文或授权账本。

Mobile薄客户端不解析vendorJSON、不持长期key、不执行工具或判定成功。显式start/takeover/observer，导航保原绑定；mic、停播、结束voice和停止任务独立。前台政策明确，后台/来电/设备/权限先停采样sink，返回不自动开mic。Media/control/业务WS分离、有界，按真实格式/时长/年龄限制；shutdown/Drop等待abort+join。

Start/steer/cancel/approval由原writer精确原目标、真实receipt处理，晚A不操作B；附和/tentative/不明审批不执行工作。即时纠正只在voice策略与cleanup证明后applied，admitted工具先settlement，未知效果不重放，未应用纠正不变新任务。上下文和结果读typed canonical事实，不用UI projection fallback。初始活动assistant正文不能拼入不可撤销instructions；source撤回同时过滤恢复缓存与未播字幕音频，保已播事实，无字级对齐不报已听字数。

Supplier差异只在adapter，目录实际选型且Unknown不当Supported。保原batch ASR/TTS、创作与robot职责，Desktop最小880×600边界不变，移动布局仅独立Mobile产品。

按用户SDK-first要求，现有RN0.86.2/Expo57固定react-native-audio-api0.13.6（MIT），统一设备PCM capture/play/context/权限；撤回自写native device/DSP engines及Base64生产bridge，原八份源SHA副本保存在Desktop/build.noindex/voice-sdk-refactor-20261007/previous-voice-audio。TypeScript facade保原Identity/Generation/格式/duration-age/SDK ID与服务器disposition credit、实际sink-clock Played、前台人工恢复及close unknown。SDK必要补丁默认false、voice-only，不改变普通SDK caller；SDK不包含厂商连接、手机Agent或模型私钥。本次不改UI、主文字或后端任务链。

按方案M0–M8完成撤回、更新、后端、Mobile native/UI、来源/恢复、无影响证明和收口。Cargo/生成器串行，独立owner并行；最新baseline+有意义targeted验证，最终一次共享静态收口，不重复无关整仓检查。交付源代码、更新Prompt/方案、唯一状态/验收、撤回清单、可运行入口和明确外验条件，不能只停规划/接口/webfake。

现有 /goal 工具不能改未完成目标正文，若卡片暂显示旧文，此Prompt与2026-10-07用户指令是实际执行目标；不得虚标旧目标complete绕过工具，也不得因此停止可实施工作。

## 当前继续条件

最新共享UI打包已完成Android实际Release/6项产物核对及iOS JS/Hermes，报告分别在Mobile/build.noindex/voice-sdk-ui-20261007092140和voice-ios-js-current-20261007-7f21c9，具体SHA/源码与copy前后一致/进程终态见唯一status与acceptance。新Android包含本轮UI，旧包不补新源码。当前仍未完成Swift/iOS native包、真设备、完整工具宿主、两云、LAN-TLS/负载与用户验收，不能以打包代替体验。

用户推进RN Web后的本批交付：Web补齐worklet admission→转换→MessagePort→JS→精确server disposition的duration/age预算、expiry锁存、旧epoch/sequence、输出MessagePort预算/flush ACK、actual output timestamp及close unknown。仅voice改为稳定Appearance订阅、显式Palette；Screen/Button新增可选themeColors，普通调用者仍原默认theme。此前“SDK重构不改UI”仅描述SDK那一批，不禁止本轮验收发现的语音UI问题修复；主文字hooks/API、默认选型、后端任务链未改。

当前264份源码输入一致的50routes导出，真实RN Web/生产API-only与真实AudioWorklet执行15项通过，目录Mobile/build.noindex/voice-web-evidence-20261007-ui9，含报告、深浅色/宽屏截图和backend-close.json。受影响31tests/173assert及最新typecheck通过，Node代理8项真实HTTP/WS检查通过。合成WAV与模拟disposition只证明浏览器合同，cloud_calls=false、p50/p95=null。JS阻塞时worklet可停止编码/发送，不能直接stopMediaStreamTrack，物理停track等待JS处理。API-only无真实native Browser/Computer roles，General创建实际422；只显式创建官方chat.minimal测试Session，不改默认或伪造host binding。真实两云、完整工具宿主、物理设备/声学、LAN/TLS/负载、native/Mac及用户验收仍pending。旧SDK APK绑定此前构建时点，不能证明后来共享UI源码的原生包。

2026-10-07本轮已完成审计发现的真实跨层缺口，具体代码与末批证据见status，不以此前绿色测试代替此次接线证明。目录/来源撤回/完整字幕快照/静默ACK/Relay重建/精确来源关联和明确可选的owned model cancel+join已有production接线及定向验证。只有清理证明完成且没有已准入工具效果时才能替换原模型步骤；工具已准入先结算，普通文字None路径保持原行为。

用户接受SDK许可后，重构前的自写Android音频module、Debug及Release arm64 APK编译均已通过，Android/iOS Hermes分别打包通过。实际assembleRelease（83661）exit0，7m51s、569tasks（523执行、46up-to-date）；2138模块dev=false经Hermes -O嵌入APK，8个voice源SHA与当时产品一致。离线Release及7项只读包内证据保存在Mobile/build.noindex/voice-android-release-verified-20261007，包含真实DEX/Expo注册、4812488字节JS、27个arm64 .so、最终音频权限与既有HTTP设置、无debuggable=true、zipalign及16KiB ELF alignment静态结果。这些历史证据保留，不能算当前SDK重构通过。

当前 SDK 已 pin 为 AudioAPI0.13.6，统一 facade 与默认false的 voice-only 原生策略源码已完成：跨JS/server ACK的 duration-age permit、独立原生 age monitor/过期锁存、语音处理请求、后台/interrupt/route 停设备且不自动恢复、actual sink timestamp/flush 与 close unknown。不同 SDK sink timeline 不伪合并消费。旧八份 bridge/Base64 已撤并保留 SHA 副本，没有新设备 engine；补丁通过 patchedDependencies 持久化，SHA256 `d4f2119bead586f8371f2c329b473a4b9a87c7da82e5d43163bfae0af939b1ef`，普通 SDK caller 的默认行为保留。

本批实际本地证据：endpoint17项/89 assertions，Mobile50项/259 assertions、typecheck/i18n；MSVC native header13组/71 checks；当前 SDK `:app:assembleRelease`（59455）exit0，4m49s、576tasks（510执行、66up-to-date）；6项只读产物核对通过，含 source/patch/SHA、真实 DEX class_defs、新 RN PackageList 注册、AArch64 SDK .so 和 APK/生成 Hermes bundle 一致。当前 SDK APK 在 Mobile/build.noindex/voice-sdk-verified-20261007/nomifun-mobile-sdk-release-test-arm64.apk，59974745字节，SHA256 `c75166268c7b287cc6a4bb0bab1dbf20a1fc980e6d29a4e7d8e06c4237bfd08d`；同目录保留日志、报告、Manifest、alignment 与原生 header 结果。最终两音频权限/既有HTTP设置、无 foreground service/debuggable=true、ZIP16KiB和29个ELF PT_LOAD16KiB静态检查通过。H5当前导出50routes，iOS JS2135modules/5792310字节Hermes通过；这些不证明设备、云模型或Swift已通过。

Release variant仍为原Expo模板TEST签名，非正式生产签名/发布；未读取生产签名key。正式签名/分发不属于本任务完成条件，未来如需要另行授权。Swift/macOS编译、真实设备生命周期、两云模型权限、LAN/TLS、视觉/声学、并发负载与用户验收仍pending，整个Goal未完成。后续应推进这些对应外验或具体新失败，不重复无关静态检查填补体验证据。未安装或运行app，仍不自动启动GUI、提交、推送、发布，不直接运行旧用户data_dir升级。未完成真实验收不得称其通过。
