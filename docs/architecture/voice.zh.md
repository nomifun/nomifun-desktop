# 实时语音架构

语音是独立的媒体与交互入口，工作仍通过唯一 canonical AgentSession 执行。Mobile 是独立客户端，负责设备媒体；供应商连接、长期凭据和工作授权由 Desktop 后端持有，不影响桌面 renderer 的 880×600 最低窗口合同。

## 所有权

| 层 | 责任与源码 |
| --- | --- |
| 公开合同 | [nomifun-voice-contracts](../../crates/backend/nomifun-voice-contracts/src/lib.rs)：媒体、profile、工作交互与 API 类型 |
| 媒体核心 | [nomifun-voice-core](../../crates/backend/nomifun-voice-core/src/lib.rs)：供应商无关的端口、会话与媒体生命周期 |
| 产品 owner | [nomifun-voice](../../crates/backend/nomifun-voice/src/lib.rs)：端点、VoiceJournal、来源、待处理输入与工作桥接 |
| 供应商 wire | [model-invoke/voice](../../crates/backend/nomifun-model-invoke/src/voice/mod.rs)：已注册协议、鉴权、连接与供应商事件 |
| 应用装配 | [mobile_voice_host.rs](../../crates/backend/nomifun-app/src/router/mobile_voice_host.rs)、[voice_work_host.rs](../../crates/backend/nomifun-app/src/router/voice_work_host.rs)：认证、registry 与 canonical 工作准入 |

生产适配器身份由 registry 明确注册，当前包含 `stepfun.realtime_s2s` 和 `openai.gpt_live`。模型、协议与连接必须显式选择；adapter 存在、音频 I/O 或握手成功均不证明账号权限、语义双工或实际听感。新增 adapter 依赖公开 ports/contracts，不能取得工作 authority。

## 媒体生命周期

输入与输出使用精确 voice/session、activation epoch、generation、segment revision 和实际音频格式。跨设备、JS 与服务器队列按音频时长及 wall age 有界；同一输入帧只在精确 `InputAdmitted` 或 `InputReleased` 后归还额度，released 不证明模型准入。迟到 ACK、旧 generation 和失效 lease 不能恢复采集或播放。

停播必须真实清除旧 generation；`played` 来自实际 sink clock，并保留 estimated、unknown 和 uncertain tail。scheduled、软件出队和普通完成 callback 不证明用户已听。不同设备 context 的时钟不拼成连续消费。暂停采集、停播、关闭 voice 与取消工作是独立操作。

后台、中断、路由和权限变化撤销媒体 lease，恢复前台不自动打开麦克风。关闭保留 owner 直到真实清理及 join；无法确认时报告未知或失败，不把删除 listener 当作退出证明。Mobile 设备 SDK 的实现与构建证据由 Mobile 仓库维护，本仓库不继续保存其施工 Prompt、旧 APK 摘要或过时验收次数。

## 来源与 canonical 工作

VoiceJournal 保存转写、来源 revision、媒体消费和待处理输入，只引用 canonical 工作事实，不成为第二份 Agent 日志、授权或完成账本。任务状态和可播内容读取 typed Event/Payload 与精确 Turn receipt；推理和原始工具内容不直接播报。

显式 voice-started Turn 才能使用即时纠正。替换模型步骤必须确认原模型操作取消及 join；已准入工具先结算，未知效果禁止重放。失效的未准入生成可被 typed history 排除，审计事实仍保留。普通文字请求不启用这条 voice 策略。

相对取消和纠正必须证明转写时钟、来源 revision 与当时工作目标；无法证明时等待 authenticated 用户确认，不使用收到消息时的最新任务补造关联。模型重试的新调用 ID 不得重复准入已提交来源和意图。语音关闭、媒体失败或 journal 不可用不改变已有文字工作结果。

重启后由用户明确启动语音并完成 lazy reconciliation。已确认未准入的 Queued 输入保留原 operation key、策略及 binding/context-floor 围栏；未知 Dispatched 只查询原 canonical receipt，不重新派发。详见 [Agent Session](agent-session.zh.md)。

## 物理机器人入口

实体机器人使用伙伴的主要 Agent 与独立设备 Session，由 [robot_wiring.rs](../../crates/backend/nomifun-app/src/robot_wiring.rs) 和 [robot session](../../crates/backend/nomifun-robot/src/session.rs) 装配。该链独立于 Mobile 的媒体实现，不能以另一产品的双工或播放能力代替设备验收。

已真实准入工具后的早期公开进度，消费正式 `PublicOutputCommitted` 来源证明绑定的不可撤回前缀；原公开 part、模型步骤、accepted root、发布范围与摘要必须配对，每次前缀以其 commit event 身份去重。最终答案则使用原 `message/completed` 的 part 数及摘要，并要求 canonical 公开结果证明实际 Host 成功 `turn/completed` 与 `Answered` 的精确消息引用；`through_seq` 包含该终态许可。`CompletionDelivered` 仍是可被 Host 收窄为暂停的提案，不能提前据此播报完成。不将原始 token、推理、工具正文、可撤回草稿或已丢弃内容送入 TTS，也不提前封口仍可增长的聚合消息。

直接设备请求沿同一 typed `request_user_input` 交接。问题、用户回答、状态、结果和取消引用原 Session、Turn 与 question identity；录音起始冻结已呈现的问题，新问题、慢 ASR、断连或新连接不补造关联。语音不替代要求点击的审批。

后台子 Turn 已提交的真实缺信息请求不受 `automatic` 决策策略阻挡；该策略不提供缺失的真人答案。旧审批／StopTurn 路径仍要求 `AskUser`。问题呈现事实在原文字实际进入 writer 后记录，排队 ACK 不产生答案授权；每帧继续核验原问题的待答状态。普通请求的媒体同样核验原 accepted input 与精确 Turn，UI 取消后停止发送新帧，正常 Completed 仍可交付原最终答案。

普通请求在设备 Session 直接执行已授权工具。按需后台任务的真实报告沿原请求来源直接交付，不固定开启隐藏总结 Turn。成功由正式事件推进；持久事实重读只修复漏帧或重启，不以退避巡检发现正常结果。

供应商通过 SpeechServices 明确声明增量 ASR 与流式 TTS。支持的输入流协议在录音期间发送音频；流式合成在供应商响应完成前交付实际音频。批处理只声明 Batch，完整合成后切帧不成为 Streaming。生产播放保留音频节拍、有界队列、背压和 generation；停播、改口、补充、取消工作和新请求分别作用于确切对象。

| 已实现的独立语音协议 | 交付方式 |
| --- | --- |
| `deepgram.listen_stream` | 显式选择的 WebSocket 增量输入；只将供应商最终片段组成正式转写，`CloseStream` 后验证终结与原 worker 退出 |
| `openai.audio_speech` | 真实 HTTP PCM 字节流拉取，24 kHz、16 位小端单声道；音频到达后按节拍编码下发，不等待完整文件 |
| 其他现役批量 ASR/TTS | 明确 Batch；ASR 的 SSE 文本响应不等于增量音频输入 |

实现依据供应商公开合同：[Deepgram Listen Streaming](https://developers.deepgram.com/reference/speech-to-text/listen-streaming)、
[Deepgram CloseStream](https://developers.deepgram.com/docs/close-stream) 和
[OpenAI Text to Speech](https://developers.openai.com/api/docs/guides/text-to-speech)。
协议测试使用本地受控 HTTP/WS，账号权限、网络缓冲和实际首音另行测量。
Pacer 自身的队列为 32 个 60 毫秒音频帧；后续 writer 受理不证明设备已播放，不能把这一个队列的容量当作端到端播放延迟。

设备断连、连接替换、改绑和 context clear 撤销原交付范围。迟到 ASR、模型、音频或后台结果不得进入后继请求或其他连接。媒体、模型、设备动作与后台执行各自有预算，后台总期限不替代首个公开答复或首音要求。

具体阶段、实测范围与待解锁环境见[统一伙伴专项验收](../validation/companion-unification-2026-10-18.zh.md)。本地受控 HTTP、协议和模拟音频只证明代码路径，不证明真实账户可用、实体设备播放或机械完成。

## 验证边界

定向检查覆盖公开端口、媒体预算、迟到事件、来源撤回、journal 恢复与 canonical 纠正。实际账号权限、双供应商交互、LAN/TLS、Android/iOS/H5 设备生命周期、声学效果和清理证明需要对应环境执行；类型检查、假设备、静态安装包和源码实现不能补证这些结果。历史本地执行记录留在 Git，当前页不声明未经重验的设备验收。

可从 [core port tests](../../crates/backend/nomifun-voice-core/tests/port_contract.rs)、[service tests](../../crates/backend/nomifun-voice/src/service/tests.rs)、[provider contract tests](../../crates/backend/nomifun-model-invoke/src/voice/contract_tests.rs) 和 [runtime correction tests](../../crates/backend/nomifun-app/src/router/voice_runtime_correction_tests.rs) 定位受影响检查。
