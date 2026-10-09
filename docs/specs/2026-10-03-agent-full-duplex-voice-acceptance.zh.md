# Mobile全双工 / Desktop协作验收

本文是 Mobile 全双工语音与 Desktop 协作的验收清单。产品方案以
[Mobile 实施方案](2026-10-07-mobile-full-duplex-voice-implementation.zh.md) 与
[SDK 重构补充](2026-10-07-mobile-voice-sdk-refactor.zh.md) 为准；
供应商协议依据见 [全双工生产协议参考](2026-10-03-agent-full-duplex-voice-research.zh.md)。
已完成批次的构建与测试证据记录见 Git 历史与各自 `build.noindex/` 报告，不在本文保留。

## 底线与环境

主文字发送、排队、暂停/恢复、停止/纠正、模型/Agent选择、工具/审批/权限、完成、历史与恢复必须保持最新upstream行为。voice显式工作指令进入原canonical owner，按原规则影响指定工作；mic暂停、停播、结束voice、后台/断网绝不自动停止Desktop任务。

最新upstream的 `nomifun-db/src/agent_store_clean_cut.rs` 会在识别到旧Agent代际时clean cut，旧Agent数据不迁移。禁止直接用现用旧data_dir验收；先完整备份，再使用隔离副本或新临时目录审核升级。未知/损坏lineage须fail closed，不恢复旧reader、不改写身份、不拿编译测试声称旧历史已保留升级。

验收环境限制：允许独立无头Chromium及API-only服务；禁止Tauri、可见浏览器、模拟器或原生移动app。使用新data root/独立work_dir、127.0.0.1:0及独立browser context，不调用dev/free-ports/kill-stale或附着现有浏览器/进程。

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
    cargo test -p nomifun-voice -p nomifun-voice-core
    cargo test -p nomifun-model-invoke --lib voice
    cargo test -p nomifun-app --no-default-features --lib voice
    cargo test --manifest-path tests/fixtures/voice-external-adapter/Cargo.toml

Cargo/合同生成器串行，最后契约使用独立voice-contracts，不要求在主Agent schema中生成voice。Mobile仓库按其docs/VOICE.md运行typecheck、i18n、纯行为tests、Android/Apple autolinking及隔离export；当前原生build必须包含固定AudioAPI SDK与其必要voice-only保证，不能再以已退役的本地module证明。

无头服务需要时，使用真实web CLI的 `--api-only --host 127.0.0.1 --port 0 --data-dir <隔离临时目录>`，实际端口从该目录port.json读取；认证方式保原产品策略。不要启动普通dev脚本，不共享现用数据。这是HTTP认证/目录/off惰性证据，不是声音、真模型或视觉验收。

纯端口harness、本地WS和mock fixtures只证明代码合同/生命周期/来源/ACK。静态导出与autolink只证明打包发现。指标没有样本时为null/pending，不能替代真实设备p50/p95。无头runner的correction_applied_ms使用应用回执request_kind=steer，而非供应商工具args；测量区间是work_trigger_received_to_canonical_applied，不是语音结束或实际听感。恢复时停止上传并保心跳，InputReleased归还额度而不算模型准入，只有当前epoch/generation实际Ready+Capturing ACK允许继续；不重发被丢弃尾帧。

## 真实入口

Mobile执行bun run voice:web:export --out-dir build.noindex/<全新导出目录>；node scripts/voice-web-server.mjs --export-dir <该目录> --backend-url <显式origin>仅监听127.0.0.1随机端口，不自动打开浏览器。自动化用bun run e2e:voice-web --export-dir <目录> --backend-url <隔离backendorigin> --data-dir <新backenddata> --report-dir build.noindex/<全新报告目录>；--media-only可仅重验媒体。Playwright通过NOMI_E2E_PLAYWRIGHT借用外部runtime，NOMI_E2E_EXECUTABLE_PATH指定实际测试浏览器，始终headless。独立数据证明与完整复现见Mobile docs/VOICE.md、docs/TESTING.md，不用dev默认端口或现用数据。

API-only缺真实Browser/Computer roles，General创建422不算成功；只显式创建发布的chat.minimal测试Session，不改默认、不伪造绑定。媒体合成WAV/模拟disposition不证明模型或听感，cloud_calls=false、p50/p95=null。JS阻塞时worklet可停编码/发送，不能直接stopMediaStreamTrack，不能补证native前台政策。

Mobile配对原Desktop实例后，在会话标题栏进入Voice页，选择具体Session与独立语音模型/adapter/connection role/精确transport，保存VoiceProfile，然后明确开始。保存不录音，不改工作模型或Agent binding。只显示实际端点支持的transport：native目前relay，H5可relay/native RTC；不能安装了Expo Go就当原生模块可用，不能悄悄改用ASR+TTS。

业务WS、voice媒体与控制分开。API为 /api/mobile-voice/v1；GET capabilities/catalog/availability，PUT profiles/{id}，POST profiles/{id}/probe与sessions、sessions/{id}/work、control、attachment、source-context。连接probe仅由用户手势检查已保存profile，真实握手/ACK/关闭不代表声音、设备或工作体验通过。媒体票据在短期子协议，禁止URL/长期storage存token。每秒真实前台心跳，3秒租约；mic暂停仍有心跳，后台停止心跳/设备而Desktop工作继续。

工作纠正默认safe_boundary；显式选择SupersedeModelStep只对随后voice启动的任务安装owned操作端口，不升级已有普通文字Turn。中途纠正须真实producer/opening退出及join证明；已准入工具先结算，关闭失败不能开启后继步骤。目标曾变化且供应商无法证明转写clock时，相对纠正/取消会显示人工来源目标确认卡片；只可确认当前前台端点中完整匹配的来源revision和精确任务呈现，旧呈现、观察端、无目标或来源缺失都不可确认。被阻止的原请求尚无reserve，确认重prepare它；有未知准入结果的旧operation仍lookup-only。能够证明来源原目标A的相对指令不能改绑B。

独立数据入口：GET /api/mobile-voice/v1/data/export返回32MiB以内自一致SQLite snapshot及schema/voice-sequence头；DELETE /api/mobile-voice/v1/data/sessions/{id}显式删除自身voice事实与profile。普通主Session删除不等待voice，专属监督只确认真实tombstone后删除voice事实、保Non-Agent profile。持久删除意图保owner和profile模式，重启不改变意图。恢复snapshot不复活mic/输入lease、不盲重发工作；原activation namespace与当前root/generation不符或未知的未准入意图deferred，不自动提交；scope/namespace不匹配记录保留为历史而不自动入模。

有厂商凭据、隔离已配置Session及事先准备的PCM/WAV时，可显式运行：

    bun run test:mobile-voice-live --run-live --base-url <认证实例> --agent-session-id <原Session> --binding-version <版本> --provider-id <厂商ID> --model <精确型号> --audio <录音路径> --source-identity <当前40位源码SHA>

产品认证token从NOMIFUN_VOICE_SMOKE_AUTH_TOKEN读取，不写命令行或报告。脚本只观测媒体和真实工作回执；不伪造played，不自动cancel工作，结束仅关闭voice。真实听感由实际Mobile端记录。

## 双供应商 × Android/iOS/H5矩阵

当前设备层固定react-native-audio-api0.13.6（MIT），旧八份native bridge/Base64生产路径已撤。新facade保原媒体身份/格式/预算/额度与工作边界；SDK原生voice-only策略默认false，已接入跨JS/server ACK duration-age预算、原生age monitor与过期锁存、语音处理、后台/interrupt/route禁止自动mic恢复及真实sink clock/flush。没有新增设备engine，普通SDK caller默认不变，不同SDK sink timeline不伪合并消费。当前源码、本地行为及Android构建/产物已有独立证据，不把SDK安装或旧APK记为当前通过。

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

构建环境：Mobile 为 RN0.86.2/Expo57.0.11，隔离 JDK17、SDK36、Build Tools36.0.0（AGP另需35.0.0）、platform-tools、CMake3.30.5 及 NDK27.1.12297006。

iOS Hermes导出不代表Swift/Xcode原生编译，仍需macOS/Xcode/CocoaPods及真机；Android真机、两云模型权限、LAN/TLS、视觉、声学及并发p50/p95继续pending。H5还需安全上下文和实际麦克风/播放权限。正式生产签名及分发不属于本任务完成条件，未来如需要须单独授权。

用户需明确完成本矩阵验收；验收后提交/推送仍必须另行明确授权，没有默认commit+push。
