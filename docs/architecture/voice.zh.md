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

## 验证边界

定向检查覆盖公开端口、媒体预算、迟到事件、来源撤回、journal 恢复与 canonical 纠正。实际账号权限、双供应商交互、LAN/TLS、Android/iOS/H5 设备生命周期、声学效果和清理证明需要对应环境执行；类型检查、假设备、静态安装包和源码实现不能补证这些结果。历史本地执行记录留在 Git，当前页不声明未经重验的设备验收。

可从 [core port tests](../../crates/backend/nomifun-voice-core/tests/port_contract.rs)、[service tests](../../crates/backend/nomifun-voice/src/service/tests.rs)、[provider contract tests](../../crates/backend/nomifun-model-invoke/src/voice/contract_tests.rs) 和 [runtime correction tests](../../crates/backend/nomifun-app/src/router/voice_runtime_correction_tests.rs) 定位受影响检查。
