# Mobile 全双工与 Desktop 后端协作

2026-10-07最新Git授权：用户要求本轮commit+push，允许Desktop与Mobile任务代码入库，覆盖本方案此前验收前Git等待限制。真实pending验收项未判通过，PR/发布/部署未授权。

2026-10-07 用户最新产品决定。本方案替代原 Desktop 全双工 UI/全局文字控制迁移要求。Desktop 根为 `C:\Users\rika0\code\nomifun\bak\refactor\nomifun-desktop`；Mobile 使用现有 React Native / Expo 项目 `C:\Users\rika0\code\nomifun\bak\refactor\nomifun-mobile`，不另建产品前端、不修改另一份同名 clone。Android 验证目录只保存该项目的隔离构建副本。当前设备实现按 [SDK 重构补充](2026-10-07-mobile-voice-sdk-refactor.zh.md) 执行，旧本地 bridge 已退役，新 SDK 接线、定向验证和 Android 构建/静态产物核对已完成，真实外验仍待完成。

## 目标与硬边界

Desktop 保留现有正常文字输入和一次性 ASR。全双工是 Mobile 中默认关闭、显式启动的另一个交互方式；Mobile 是薄客户端，工作由 Desktop 唯一 canonical AgentSession/Turn/Event/Payload/Effect 和 Runtime 执行。

文字主链路不得因语音改造改变默认发送、排队、编辑、暂停/继续、停止、纠正策略、模型/Agent选择、工具授权、审批、完成判定、历史和恢复。用户通过 Mobile **明确提交**工作/纠正/取消/审批，是原 authority 接纳的正常用户操作；自由语音、附和、未提交字幕、开启/结束语音和语音故障不能隐式修改工作。必要通用扩展必须 additive、严格 opt-in，不能用 `safe_boundary` 默认值等替代直接兼容证明，无法隔离则撤回。

媒体连接、队列、限额、取消和故障树独立；验证语音错误/过载不传播到文字请求，关闭无残留。物理 CPU/网络和共享云账户资源竞争需要实际测量，不能以“默认关闭”声称性能已经无影响。

用户明确验收完成并另行授权对应动作以前，禁止commit、push、PR创建/合并、发布和部署；验收完成也不触发提交。允许fetch、完整快照及无提交源码更新。2026-10-07用户明确推进RN Web后允许独立无头Chromium和API-only服务；仍禁止Tauri、可见浏览器、模拟器和原生移动应用，不操作现有进程/profile或真实数据。

## 基线与撤回

Desktop 工作树为 b25ca3525；2026-10-07 fetch 远端为 02f7b677d，相差82commits。两份 Mobile 均 clean main/c4ecae674/同远端，选择 refactor clone。更新前保存全部 tracked/untracked 修改，采用“最新 upstream + 明确允许的 voice 扩展”，不把旧 patch 整包覆盖，不恢复上游已退役的 schema、reader、checkpoint、草稿导入、兼容别名或设计输入。

先读最新 AGENTS 与 `docs/architecture/agent-session.zh.md`。更新源码不授权运行真实用户数据库的 clean-cut/reset/import；有升级 lineage 限制只在隔离 fixture 证明和记录，不以本语音任务清理用户数据。

| 原改动 | 新处理 |
| --- | --- |
| Desktop VoiceDock/VoiceStartButton/字幕、实时设备、DOM RTC/worklet | 撤出 Desktop 产品路径；按平台媒体能力重组到 Mobile，不能直接把 DOM controller 当 RN 实现 |
| Agent voice tab/save gate/model detour、普通 Preset/Snapshot voice_plan | 撤出文字配置/binding；新独立 VoiceProfile overlay 引用现有 Agent/Revision，不让语音配置/凭据阻断文字编辑 |
| server FIFO 替换文字本地队列、强制 target cache/普通 DTO、停止后排队语义 | 恢复最新文字源码/API/测试；现有草稿/队列原文不自动迁移或删除 |
| 全局 exact cancel/steer、所有 stream/ToolAdmission/steering marker/SDK TextReplaced | 撤回默认文字路径；voice-only 显式入口和原 writer 的附加 fence，不另起工作 authority |
| 主 descriptor 加 voice 源码、所有文字 Source aliases/readmission、digest/ceiling改造 | Voice 能力/source单独隔离，不改写旧身份，不恢复最新上游已经退役的数据兼容路线 |
| voice journal/provider close失败使普通 bind/delete失败 | 清理只归 voice owner，及时撤 lease，剩余清理走专属监督/可恢复intent，不阻断文字结果 |
| 一次性 ASR AbortSignal/XHR清理、cancel/unmount/new-record epoch/late filter | 保留并定向合并，仍转入原可编辑输入，不能自动启动任务 |
| audio/core/两生产adapter/VoiceJournal | 复用后端；恢复不相关 batch/robot/创作普通路径，避免附带改变原产品 |

旧源码/原文档先保存到不参与构建的任务快照，再删除退出生产入口、DTO、schema、测试及 current-input 设计。主链路改动逐项审查和直接回归；撤回以最新远端为准，不用旧文件恢复正常文字代码。

## 独立合同、配置与协作

Voice 类型、JSON schemas、TS生成和能力版本独立于主 Agent 合同。VoiceProfile/route 保存独立语音模型、host校验配置、transport和偏好，只引用owner、Agent/Revision、Session/binding、Desktop实例/数据代际；不复制权限、工作状态或Runtime。保存不录音、不连接云模型、不改文字模型/Snapshot。Profile/store/凭据失败仅语音不可用。

Activation 冻结profile/route/config/adapter identity、credential lease、Desktop实例和数据代际、会话及输入端点。权限/Agent变化撤 lease，明确重新绑定；不得偷偷采用最新默认。单个VoiceAdapterRegistration供目录、validator、probe和factory消费；Supported/Unsupported/Unknown分清，typed tools与metadata delegation分开，两生产adapter共core。

| Owner | 职责 |
| --- | --- |
| Mobile controller | 用户手势、设备/前后台/权限、音频/RTC、字幕、消费cursor、即时本地停播；不解析vendor JSON、持长期key、决定权限或执行工具 |
| Desktop voice service | 独立认证API/媒体、profile/route、lease、供应商连接、工作桥、来源/played facts和VoiceJournal |
| Desktop canonical owner/Runtime | 唯一输入准入、工具/效果、审批、取消、完成、历史和恢复；文字默认行为不变 |

复用实际 Mobile 配对、JWT/Desktop installation token、REST baseURL和业务WS；Voice HTTP必须接受已有配对身份，不能假定只有cookie/local-trust。新增专属版本namespace及能力发现；旧Desktop不支持voice时Mobile文字继续可用。不修改全局auth/CORS/业务WS去迁就voice。

Media走专属有界连接，attachment为owner/session/binding/epoch/device限定的短期一次性能力；长期vendor key只在Desktop。票据优先subprotocol/header，避免URI/反代日志；app现有trace只记录path，不假称已泄漏。H5/native Origin/CSRF/反代信任采用实际现有owner原则。

Relay：Mobile流式明确规格音频，Desktop连接vendor，下行产品帧/控制分离。Native RTC：Mobile只处理SDP附件、tracks/sink，Desktop做信令/sideband，不重复relay采集播放。降级需明示且保持同模型语义，不能将ASR+TTS冒充全双工。

## Mobile UI 与媒体

复用session页、theme tokens、中文优先i18n及配对/Agent导航。新增语音入口和专业、克制的通话界面：绑定Desktop/Agent/Session、独立连接/采集/播放/工作状态、tentative字幕、具体工作回执/审批、mic暂停/停播/结束voice/独立停止任务。离开会话页保留原绑定入口，不随导航转移；observer不拿mic/model lease，接管明确撤旧端点。

当前RN0.86.2/Expo57.0.11设备实现固定react-native-audio-api0.13.6（MIT），由SDK统一PCM capture/play/context与权限，TypeScript facade保留既有媒体端口。旧八份Swift/Kotlin bridge及Base64生产路径已撤，原SHA副本保存在Desktop/build.noindex/voice-sdk-refactor-20261007/previous-voice-audio。删除自写device/DSP engines，只在SDK默认false的voice-only选项补必要媒体保证，不能全局改变普通SDK caller；手机仍不持模型私钥或运行Agent。

H5既有gUM/AudioWorklet/RTC端点与UI不在本次重写范围。Native仍为relay；SDK原生capture→JS→server disposition的duration-age预算、原生age monitor及过期锁存、语音处理请求、后台/interrupt/route停设备且禁止自动mic恢复、实际sink clock/flush和close unknown已接入。身份、epoch/generation、格式、credit与Played合同保留，不同SDK sink timeline不伪合并消费。持久补丁SHA256为 `d4f2119bead586f8371f2c329b473a4b9a87c7da82e5d43163bfae0af939b1ef`。当前SDK endpoint17项/89 assertions、Mobile50项/259 assertions、type/i18n与MSVC header13组/71 checks通过；实际Android Release构建和6项产物证据已通过，最新APK与日志在Mobile/build.noindex/voice-sdk-verified-20261007。H5新export50routes、iOS JS2135modules/Hermes也通过。以上对应当前SDK源码，旧APK保留为历史基线；不能用本地静态证据声明Swift原生、真设备/声学或两厂商体验完成。

首期明确前台政策：后台/锁屏/来电/audio focus/设备或权限变化先停采样和sink，后台工作继续；前台返回不自动开mic。锁屏继续通话需单独产品决定及OS实测，不静默加入无限后台录音。Native停播持续detach/mute，只有安全边界或新attachment证明才能恢复。Played如实precision/uncertain tail，不报未经对齐的已听字数。真实外放AEC及可听延迟需设备测试。

## 工作、来源与恢复

Voice start/steer/observe/cancel/approval共用原canonical owner。专属入口在同writer验证原session/binding/operation/generation或已有同等fence，晚到A不操作B；普通文字API/调用者不强制新target/policy。最新上游已有同等正确能力直接复用，不维护第二实现。

即时纠正只在显式voice策略下使用可证明owned attempt/safe model-step，普通文字保留upstream。工具已准入先settlement；无cleanup证明如实pending-boundary，不假装applied。Voice待处理意图与canonical started分开，未确认查原receipt，不盲重发；未应用纠正不变新任务。

状态、结果、上下文与可播来源读typed canonical events/payload/真实Turn receipt；不把Message UI projection反向作为工作Runtime上下文，不读退役字段或私有transcript、不推断完成，reasoning/原始工具内容不直接播报。

初始canonical对话用带来源的结构化facts，与冻结Agent指令分离，退役把活动assistant正文拼入不可撤销instructions的做法。撤回覆盖初始facts、private恢复缓存、未播媒体/字幕；保已播cursor和审计。VoiceJournal只存voice事实和canonical引用，不逐帧存音频，不是第二权限/任务账本；故障、备份、删除和恢复不能阻断文字操作或自动重播/重做。

审批绑定真实request/action/presentation/版本和现有权限；孤立“是”、附和和不明来源不扩权，原强制点击规则保留，Mobile呈现同一canonical决策。

## M0–M8 实施与验收

当前共享UI已补Android实际Release/6项产物核对及iOS JS/Hermes，新目录与来源SHA见唯一status/acceptance。SDK native未改，旧包保留其源时点。此项完成Android当前UI打包，不证明Swift/iOS native包、设备/模型/工具/声学或负载矩阵。

RN Web门槛使用source-bound导出、Node同源代理、新data root/工作目录及随机端口。实际RN页面/真实API验证登录、canonical测试Session、配置禁用、草稿保护和故障隔离；真实AudioWorklet用确定合成WAV验证PCM、duration/age精确ACK、JS阻塞、flush/clock/close。fixture不补证云理解/声学，p50/p95无真实样本保持null。仅voice稳定Appearance和显式Palette修复深浅色，普通Screen/Button调用者不传themeColors，默认行为保持原代码。

API-only无真实native Browser/Computer roles，General创建422不能绕过。仅显式官方chat.minimal测试Session可做结构UI/草稿检查，不能算完整默认Agent工具验收。当前Web15项通过，复现与报告见唯一验收清单；两厂商、完整工具宿主、native/声学/负载及用户验收仍未完成。

| 包 | 必交付/门槛 |
| --- | --- |
| M0 | 新Goal/Prompt/方案、完整快照、最新Desktop与单一Mobile根、禁止commit/push门槛 |
| M1 | 更新源码、恢复文字/ASR、删除Desktop全双工UI与全局耦合，不恢复retired数据实现 |
| M2 | 独立voice合同/profile/registry/auth/API/lease；off惰性无network/mic/store依赖，不改主DTO/digest/ceiling |
| M3 | 原writer上的voice-only输入/fence/可证明纠正/回执/审批，普通文字直接回归 |
| M4 | Mobile H5+Android+iOS真实媒体/UI、配对/能力协商、设备/前后台/observer/takeover |
| M5 | 初始typed source、转写修订/重复/撤播、已播保留、journal repair/无重放，双adapter同桥 |
| M6 | 与最新baseline比较send/queue/stop/steer/工具/审批/历史/恢复/模型/Agent切换；voice故障不改文字结果或owner |
| M7 | 两厂商真实权限、三Mobile平台、Desktop LAN/远程TLS、耳机/外放、设备/来电/后台/网络/多客户端，p50/p95及真实receipt |
| M8 | 删退出实现/旧current-input设计，唯一状态/验收、复现脚本与外验条件，用户验收不等于提交授权 |

Cargo/生成器串行，独立owner并行。使用最新baseline，不把299/b25旧证据当02f7新实现证明。运行受影响canonical/voice边界、生成/TS、Mobile type/i18n/行为及native build；最终共享收口一次聚合静态检查，仅新改动、失败或风险再扩。缺凭据/设备/OS/GUI授权则继续其余可实施工作，明确条件/入口/复现/证据，不以mock、握手或编译代替真实体验。实现、本地验证、用户验收分别标记。
