# StepFun 机器人语音链路补齐设计

- 日期：2026-08-08
- 状态：已获用户确认（架构=离散、聊天入口=侧边栏分组、失败=可见），进入实施
- 涉及仓库：`nomifun-tauri`
- 关联：`2026-08-06-robot-bridge-design.md`（机器人桥）

## 0. 背景

机器人已能连上 nomifun，但：①收不到用户语音（ASR）②不能语音回复（TTS）③聊天记录不在伙伴聊天列表里。经全链路考察（5 路并行探查），结论：**机器人语音链路是完整实现的真实代码，无桩、无未实现**。三个症状全部来自「配置 + 模型目录分类 + 一处刻意但未做完的可见性设计」，而非缺实现。

## 1. 根因（已交叉验证）

- **听不到**：companion `voice.asr` 槽与全局 `tools.speechToText` 都为空 → `RobotSpeech::transcribe` 硬报错 `no speech-recognition model configured`（`nomifun-robot/src/wiring/speech.rs:157`）→ 在 `session.rs:627-650` 被吞成空文本 → 只发一对空 `tts start/stop` → 纯静默。
- **不会说**：`voice.tts` 与全局 `tools.textToSpeech` 都为空 → `synthesize` 硬报错（`speech.rs:194`）。单句回复时该失败被记为“成功但无声”，仅 `tracing::warn`（`session.rs:161-169`）——正是“连上、显示文字、从不出声”的签名。
- **聊天不入列表**：机器人回合已持久化（用户+助手消息都写入 `type='nomi'`、`extra.robot_session=true, companion_session=true` 的会话），但该会话仅记录于 `{data_dir}/robot/threads.json`，未登记进 companion 线程注册表，且被前端 `conversationListFilter.ts:34` 双重排除 → 无任何 UI 入口可达（`ChatConversation.tsx:659` 其实已支持渲染该 `type='nomi'` 会话）。

**StepFun 现状**：`stepfun`/`stepfun-plan` 已是内置平台；OpenAI 兼容的 `/v1/audio/transcriptions`（ASR）与 `/v1/audio/speech`（TTS）本就走默认 OpenAI 路由（零适配器代码）。`response_format=pcm, sample_rate=24000` 正好吻合机器人下行 24k PCM 契约。真正缺口：`STEPFUN_FALLBACK_MODELS` 只列 chat，缺 ASR/TTS，导致目录里可能无语音模型可选；`VISION_INCLUDE` 漏 `step-1v` 命名。

## 2. 方案（离散 ASR+TTS+对话+视觉）

保留 nomifun「ASR→伙伴 agent 回合→TTS」管线，复用人格/记忆/会话线程/MCP 工具/打断。**不接 Realtime S2S**（会绕过整套伙伴架构）。

### 2.1 Stream A — StepFun 模型目录补齐（后端，小）
- `nomifun-system/src/model_fetcher/fetchers.rs`：`STEPFUN_FALLBACK_MODELS` 增加
  - ASR：`stepaudio-2.5-asr`、`step-asr`
  - TTS：`stepaudio-2.5-tts`、`step-tts-mini`
  - 多模态旗舰：`step-3.7-flash`
- `nomifun-api-types/src/model_capability.rs`：`VISION_INCLUDE` 增加 `step-1v`、`step-3.7`，使 StepFun 视觉模型自动带 `vision_input`。
- 分类由 `derive_tasks_and_traits` 的名字启发式驱动（`asr`→SpeechRecognition，`tts`→SpeechSynthesis，仅 `stepfun-plan` 被平台级强制为 image）——已验证命中。
- 测试：`model_task.rs`（asr/tts/vision 断言）、`fetchers.rs`（fallback 含 asr/tts）。

### 2.2 Stream B — 失败可见（后端 nomifun-robot + 少量 UI）
- `session.rs`：ASR/TTS「未配置/失败」时，不再纯静默：
  - 发一条设备可显示的 `sentence_start` 文本（如“语音识别未配置”/“语音合成未配置”）让 OLED 显示；
  - 把 `robot.status` 打成一个可诉求的诊断态（新 phase，如 `misconfigured`），经既有 `UserEventSink`/`robot.status` 上报。
- UI：伙伴「机器人连接」区 attention 在「已绑定机器人但伙伴缺 ASR/TTS 模型」时亮起并可跳转配置。

### 2.3 Stream C — 侧边栏「机器人」分组（前端为主）
- `useConversationListSync.ts`：新增 `robotConversations` 收集器（现被丢弃），仿 `sshConversations`。
- 新增 `RobotSessionGroup`（仿 `SshSessionGroup`），按机器人名分组（`extra.robot_id` → 名称，复用 `useRobotStatuses`）。
- `conversationListFilter.ts`：机器人行分流到该桶而非丢弃；不进普通工作列表。
- 点开 → 现有 `CompanionChatPanel` 渲染，可查看+续聊。

### 2.4 用户一次性配置（文档 + UI 引导）
1. 模型管理添加 StepFun API Key（base `https://api.stepfun.com/v1`）。
2. 伙伴「模型配置」选 ASR=`stepaudio-2.5-asr`、TTS=`step-tts-mini`（+音色）、对话/视觉=`step-3.7-flash`；或配全局 `tools.speechToText`/`textToSpeech`。

## 3. 测试策略
- 后端单元：分类（asr/tts/vision）、fallback 列表、路由到 `openai.audio_transcriptions`/`openai.audio_speech`、失败可见性。
- 前端：机器人行进入机器人桶的过滤测试 + `RobotSessionGroup` 结构测试。
- 复用既有 fake-device e2e（已覆盖 StepFun ASR multipart）。

## 4. 非目标
StepFun Realtime S2S、流式 ASR、`wait_for_runtime_subscription` 竞态等既有健壮性问题（列为后续）。

## 5. 实施状态（2026-08-08）

已实现并通过测试：

- **Stream A（StepFun 目录）**：`STEPFUN_FALLBACK_MODELS` 增加 `stepaudio-2.5-asr`/`step-asr`/`stepaudio-2.5-tts`/`step-tts-mini`/`step-3.7-flash`；`VISION_INCLUDE` 增加 `step-1v`/`step-3.7`。测试：`nomifun-api-types` 分类 13/13 通过（新增 ASR/TTS/vision 断言）；`nomifun-system` StepFun fallback 3/3 通过。
- **Stream B（失败可见）**：`session.rs` ASR 失败不再静默——区分「ASR 报错」与「本轮无声」，报错时经既有帧在设备 OLED 显示「未配置语音识别模型…」并 sad 表情；`speak_sentence` TTS 首次失败时在屏上补一行「（未配置语音合成模型，暂时只显示文字）」。新增 `MockSpeech::fail_next_synthesize` + 两个单测（`asr_failure_shows_a_notice_instead_of_silence`、`tts_failure_shows_a_one_time_notice_beside_the_reply`）通过；`empty_transcript` 静默路径保持不变。
- **Stream C（侧边栏机器人分组）**：`useConversationListSync` 新增 `robotConversations`；`useWorkpathUiState` 新增 `robotGroupExpanded`/`toggleRobotGroup`（持久化键 `nomifun:robot-group-expanded`）；新增 `RobotSessionGroup`（仿 `SshSessionGroup`，按 `robot_id` 二级聚合、名字取自 `/api/robots`）；在 `SessionList/index.tsx` 折叠/展开两处挂载；i18n `nomi.robot.sessionGroup`/`nomi.robot.group.*`（zh+en）。测试：SessionList 107/107 通过（含新增 `RobotSessionGroup.structure.test.ts` 8 项）；`ui` typecheck 通过。

**本期未做（明确后续）**：把「已绑定机器人但伙伴缺 ASR/TTS 模型」做成 `robot.status` 的诊断态 + 伙伴「机器人连接」区 attention 常亮。设备侧已可见（OLED 提示）已达成「变可见」目标；此项为额外 UI 提示，需引入新的 status phase 枚举 + DTO/UI/线协议测试联动，风险与收益不匹配，单列后续。

## 6. 启用步骤（用户侧，代码已就绪）

1. 模型管理 → 添加 **StepFun**（预置平台，base `https://api.stepfun.com/v1`）→ 填 API Key。
2. 拉取模型列表（或用内置 fallback）后，在伙伴「总览 → 模型配置」里：
   - **语音识别 ASR**：选 `stepaudio-2.5-asr`
   - **语音模型 TTS**：选 `step-tts-mini`（并填音色，如 `cixingnansheng`）
   - **对话/视觉**：选 `step-3.7-flash`（多模态）或 `step-3.5-flash`（纯文本）
   - 或改配全局 `tools.speechToText` / `tools.textToSpeech`（模型管理 → 语音）。
3. 机器人绑定该伙伴、局域网访问开启后即可听说；聊天记录出现在会话侧边栏「机器人」分组。

**验证**：说话 → 设备屏显 `stt` 文本 → 伙伴回复分句播报；未配模型时设备明确屏显诊断（不再静默）。

## 7. 健康检查修复（2026-08-08，用户实测反馈）

**现象**：录入 TTS 模型 `step-tts-mini` 做健康检查报 `InvalidParams: 400 ... The voice_id (alloy) does not exist`。

**根因**：TTS 健康探针（`ModelInvokeService::probe`）不带 voice → `openai.audio_speech` 适配器回落到 OpenAI 默认音色 `alloy`，而 StepFun 没有该音色 → 400 `voice_id_invalid`。探针的「可达即健康」容忍规则原本只覆盖 `ImageEdit`/`SpeechRecognition`（占位文件不可用但已到达线端），未包含 TTS。

**修复**：
1. `nomifun-model-invoke/src/service.rs`：把 `SpeechSynthesis` 纳入「可达即健康」容忍——探针无法通用地知道某供应商的音色枚举，`alloy` 只是占位，收到上游 voice 相关 400（`InvalidParams` 且有 `http_status`）即证明端点+鉴权+模型可达，判健康；本地 400（无 `http_status`）仍判不健康。新增回归测试 `probe_tts_voice_400_is_healthy_and_reaches_the_wire`，全部 13 项 probe 测试通过。
2. `ui/.../ttsVoiceOptions.ts`：新增 `stepfun` 音色建议（经 `/v1/audio/system_voices` 实测校验：`cixingnansheng` 等 24 个），伙伴 TTS 槽/全局 TTS 的音色选择器即会建议这些音色（仍为自由文本，可填克隆/新音色）。测试 + typecheck 通过。

**重要**：健康检查「失败」是提示性的，**不阻断**模型使用。实际运行时音色取自伙伴 TTS 槽（已有音色选择器），只要选了真实音色（如 `cixingnansheng`）就能正常合成——无需在模型行单独加音色字段（会与伙伴槽重复）。此修复让健康检查正确通过并补上音色建议；生效需重新构建/重启应用。


