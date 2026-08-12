# 机器人桥（Robot Bridge）：ESP32 物理机器人 ↔ nomifun 桌面伙伴对接设计

> [!CAUTION]
> **本文的模型管理章节已被 2026-08-11 现行规范替代。** 机器人桥的设备、配对和传输设计仍作为历史设计记录保留，但下文关于旧模型表投影、模态分区和语音兼容迁移的描述不再约束当前模型体系。现行单一能力源、九模态、后端协议 manifest、自由模型 ID 与每模态 transport 配置以[《模型供应商 × 模态官方接口核验矩阵（2026-08-11）》](../../specs/2026-08-11-provider-modality-official-matrix.zh.md)为准。

- 日期：2026-08-06
- 状态：已获用户确认（方案与十二节设计整体评审通过）
- 取代：`2026-08-03-xiaozhi-robot-integration-design.md`（用户决定推倒重来，本文为唯一有效版本）
- 涉及仓库：`nomifun-tauri`（主要实现）、`xiaozhi-yuntai`（固件，云台 MCP 工具 + 配置）

## 0. 背景与目标

用户有一台基于 xiaozhi-yuntai 固件的 ESP32-S3 物理机器人（板型 `esp32-s3n16r8-emoji`：OLED 表情屏、双舵机云台 pan/tilt、麦克风与扬声器，暂无摄像头），需要以 nomifun-desktop 作为其唯一后台，打通：

1. **人格对接**：机器人是某个桌面伙伴的"物理化身"，共享其人格、模型配置与记忆；激活码配对绑定，支持换绑。
2. **多模型对接**：伙伴总览页扩展为五类模型槽位（对话主/备、VAD、ASR、视觉、TTS+音色），并与重构后的"模型管理"打通。
3. **网络通讯**：本期实现局域网直连（xiaozhi WebSocket 协议：JSON 文本帧 + 二进制 Opus 帧）；以帧层抽象预留未来"远程中继服务器"扩展。
4. **模型管理重构**：管理页按模态重构，语音配置收编，收敛共享的按任务选模型组件。

### 已确认的方向性决策（本次评审）

| 决策点 | 结论 |
|---|---|
| 与旧 spec 关系 | 推倒重来，本文取代 2026-08-03 版 |
| 接入架构 | 内嵌机器人网关 crate（`nomifun-robot`），不复用渠道插件，不做 sidecar |
| 中继预留 | 帧层抽象（`RobotLink` / `RobotLinkSource` / `EndpointAdvertiser`），本期只实现 LAN 直连 |
| 配对方式 | 激活码（设备屏显 6 位码并播报，用户在伙伴远程 Tab 输码认领） |
| 小对话模型 | 本期不设计 |
| 备用对话模型 | 主模型失败时自动降级（边界见 §6） |
| VAD 形态 | 本期直接接模型 VAD（Silero，本地 ONNX 推理） |
| 模型管理重构 | 本期并发做"按模态重构管理页" |
| 云台 MCP 工具 | 本期顺手改固件（约 40 行） |
| 固件 OTA 升级托管 | 本期不做（设备继续 USB 刷机，OTA 接口只下发配置） |

### 关键前提（两侧代码考察结论，实现时不可违背）

**固件侧（xiaozhi-yuntai）**：
- 一切服务端配置来自 OTA 接口：设备启动 POST 设备报告到 `ota_url`（NVS `wifi.ota_url` 覆盖编译期 `CONFIG_OTA_URL`），响应中 `websocket{url,token,version}` 原样写入 NVS 并决定连接目标（`main/ota.cc:144-184`）。**响应只要含 `mqtt` 对象（哪怕为空）就会选中 MQTT 且无回落——OTA 响应必须永远带 `websocket`、绝不带 `mqtt`**（`main/application.cc:371-378`）。
- 传输：单条 WS，文本帧 JSON + 二进制帧 Opus。设备身份在升级请求头：`Authorization: Bearer <token>`、`Protocol-Version`、`Device-Id`(MAC)、`Client-Id`(NVS 持久 UUID)（`main/protocols/websocket_protocol.cc:100-109`）。
- 音频上行**硬编码不可协商**：Opus 16kHz / 单声道 / 60ms（`main/audio/audio_service.cc:39`）。下行默认 24kHz/60ms，服务端 hello 的 `audio_params` 可覆盖；下行只吃裸 Opus（无 PCM/MP3 通路）。二进制帧版本选 v1 = 裸 Opus 负载。
- **端侧 VAD 只驱动 LED，完全不断句**（`afe_audio_processor.cc:137-146` 的回调只被 LED 消费）；`mode=auto/realtime` 下设备永远不会主动结束一轮，断句必须由服务端判定并以 `{"type":"tts","state":"start"}` 推进状态机。
- 打断：设备发 `abort` 后本地**不清播放队列**（约 2.4 秒缓冲），服务端必须立即停流并回 `tts stop`，否则有明显拖尾。
- 下行必须按实时节奏 pacing：解码队列 40 包（约 2.4 秒），满则**静默丢包**；不发 `tts start` 时所有下行音频被丢弃。
- 保活：120 秒无入站消息判超时（`protocol.cc:81-90`）。
- 唤醒词命中后的顺序是：约 2 秒唤醒音频包先到 → `listen detect` → `listen start`，服务端不能假设先收到 start 才有音频。
- 设备是 MCP server：服务端发 `initialize`（可携带 `capabilities.vision.{url,token}`——**这是视觉 explain URL 唯一的下发通道**，不走 OTA）→ `tools/list` → `tools/call`。设备侧三条非标准约束：请求 `id` 必须是数字（字符串 id 被静默丢弃）；`notifications*` 方法一律忽略；`tools/list` 单响应 8000 字节上限、以 `nextCursor=<tool名>` 分页（`main/mcp_server.cc:154-306`）。
- 表情：`{"type":"llm","emotion":"<名>"}`，21 个规范名（neutral, happy, laughing, funny, sad, angry, crying, loving, embarrassed, surprised, shocked, thinking, winking, cool, relaxed, delicious, kissy, confident, sleepy, silly, confused），未知名回落 neutral；emoji 板将其映射为眼睛动画 + 舵机联动。
- **emoji 板未注册任何板级 MCP 工具**：云台（pan GPIO11 50-130°、tilt GPIO12 70-110°、中位 90/90）目前只能被表情联动间接驱动；本期固件改动补上（§8）。
- 激活：OTA 响应带 `activation{message,code,timeout_ms}`（不带 challenge 走 Activation-Version 1 简单流程）时设备屏显/播报 code 并轮询 `<ota_url>/activate`（202=继续等，200=完成）；用户按键可跳出激活态。
- 拍照上传是 chunked multipart（**无 Content-Length**、大量小 chunk、boundary 硬编码），字段 `question` + `file`，30 秒内必须返回 200，响应 body 原文作为 MCP 工具结果转交模型。

**nomifun 侧**：
- 伙伴人格：`CompanionProfileConfig`（`companion/companions/{id}/config.json`，原子写、`deny_unknown_fields`）；系统提示由 `build_companion_system_prompt(store, profile, channel_platform, …)` 构建，`channel_platform=Some(..)` 启用远程渠道风味并把记忆注入过滤为 profile/preference/knowledge 三类。
- 会话 dispatch 范式逐字可抄：`crates/backend/nomifun-channel/src/message_service.rs:416-500`（`send_message_with_idempotency_key` + `wait_for_runtime_subscription` + `AgentStreamEvent::Text` 流）。
- 打断原语：`ConversationService::cancel_with_origin`（不是 `runtime_registry.terminate()`）。
- 外部无会话调用者统一继承安装所有者（`GatewayDeps.authoritative_user_id`，`nomifun-public` 同款）。
- `/api/tts`、`/api/stt` 挂在会话鉴权 + instance-owner + CSRF 之内，设备打不通；且 `/api/stt` 被 `tools.speechToText.enabled` 开关连坐。**设备链路必须直调 invoke 层**（`ModelInvokeService` / `SttService::transcribe`）。
- 新路由挂载范式：`nest` 在会话鉴权之外（同 `/mcp`、`/v1`，`router/routes.rs:434-475`）；挂上共享 router 即自动同时出现在 loopback 与 LAN 两个监听器。
- LAN 监听器（25808，用户可开关）host_guard 只放行 IP 字面量 Host——ESP32 直连无障碍，域名会 403（未来中继不能复用该监听器入站，本设计的中继方向是 nomifun 出站连中继，不受此影响）。
- 现有 `/ws` 显式忽略二进制帧且缓冲溢出即断连——设备通道必须是全新 WS 端点。
- Rust 侧零音频依赖（无 opus/onnx/重采样）；`/api/tts` 的 `format:"pcm"` 是拿裸 PCM 的唯一出口（OpenAI 契约 24kHz/16bit/单声道）；`format:"opus"` 得到的是 Ogg 容器非裸帧。
- 视觉：invoke 层无视觉任务（`ChatTextRequest` 纯文本）；一次性 VLM 调用的正确路径是直调 `nomi_providers::LlmProvider::stream` + `ContentBlock::Image`（base64 内联）；`image` crate 已在依赖里。
- MCP 客户端接缝：`McpTransport { request / notify / close }`（`crates/agent/nomi-mcp/src/transport/`），新增"xiaozhi WS 信封"transport 即可，不改 manager/tool_proxy。
- 模型体系：`ModelTask`（chat / image_generation / image_edit / video_generation / speech_synthesis / speech_recognition / embedding / rerank）与 `ModelTrait`（vision_input 等）已是一等公民；`useModelsForTask` 是统一的"按任务找模型"数据入口；但选择器 UI 有 8+ 处重复实现，语音 STT 配置自成旧世界（硬编码 openai/deepgram 结构），TTS 无任何配置面。
- 状态推送范式：`UserEventSink` 用户级 WS 事件（`ssh.status` 同款），UI 三段式消费（快照 + 增量 + 重连再快照，`changedAt` 择新）。
- 伙伴"远程控制"Tab（`pages/nomi/workspace/tabs/RemoteTab/`）现有两节：「远程连接」（IM 机器人）与「远程访问」（访问令牌）；新 section 直接按序插入容器即可，attention 经 `onAttentionChange` 聚合。**命名注意**：现有文案里"机器人"指 IM bot，新专项文案必须区分（IM 侧按钮文案顺手改为"连接 IM 机器人"）。

## 1. 范围与非目标

**本期范围**：LAN 直连机器人网关（xiaozhi WS 协议）、激活码配对、伙伴远程 Tab"机器人连接"专项、语音全链路（Silero VAD 断句 + ASR + 流式分句 TTS + 表情标记）、MCP 工具桥 + 云台固件工具、视觉 explain 端点、伙伴总览页五类模型槽位、模型管理按模态重构、中继传输抽象预留。

**非目标（YAGNI）**：中继服务器实现（只留抽象缝）、固件 OTA 升级托管、MQTT+UDP 传输、二进制帧 v2/v3、服务端 AEC/声纹/唤醒词检测（设备本地已有唤醒）、流式 ASR 边说边转、小对话模型、TTS 情感韵律控制、固件签名、设备分组/多租户、eFuse HMAC 激活（challenge 流程）、桌面悬浮窗自身的语音播报（TTS 槽位为其留好地基但不接线）、桌面聊天链路的运行时模型降级（见 §6）。

## 2. 总体架构

新建 `crates/backend/nomifun-robot`，由 `nomifun-app::create_router` 以 `nest` 方式挂在会话鉴权之外，自动同时出现在 loopback 与 LAN 监听器上。**机器人功能依赖"局域网访问"开关**，UI 做依赖提示（添加机器人弹窗内提示并可一键开启）。

对设备暴露四个端点（均无会话鉴权，见各自鉴权方式）：

| 端点 | 方法 | 鉴权 | 作用 |
|---|---|---|---|
| `/robot/ota` | POST（兼容 GET） | 无（xiaozhi 生态固有形态，IP 限速兜底） | 设备报到：upsert 设备记录，下发 `websocket{url,token,version:1}` + `server_time` + 未绑定时 `activation{code}`；**永远带 `websocket`、绝不带 `mqtt`**；`firmware.version` 回设备当前版本（不触发升级） |
| `/robot/ota/activate` | POST | 无 | 激活轮询：未绑定回 202，已绑定回 200 |
| `/robot/v1` | GET（WS 升级） | `Authorization: Bearer <设备token>` | 主通道：JSON 文本帧 + v1 二进制 Opus 帧 |
| `/robot/vision/explain` | POST | Bearer 设备 token（经 MCP initialize 下发） | 拍照理解：chunked multipart（`question` + `file`）→ 视觉模型 → `{"success":true,"result":"..."}`，30 秒内回 200 |

### 2.1 中继预留：帧层抽象（本期只定接口 + LAN 实现）

网关核心不知道字节从哪来，只消费三个 trait：

- **`RobotLink`**：一条已鉴权的设备链接——双向帧流（`Frame::Text(String) | Frame::Binary(Bytes)`）+ 设备身份（robot_id / client_id / peer 信息）。
- **`RobotLinkSource`**：链接的生产者。本期唯一实现 `LanWsSource`（axum WS 路由 upgrade 后包装成 `RobotLink`）；未来 `RelayTunnelSource` 由 nomifun **主动出站**连中继服务器（复用仓库成熟的 `tokio-tungstenite` 出站范式），单隧道多路复用多台机器人，中继只透传 xiaozhi 协议帧不懂业务语义。
- **`EndpointAdvertiser`**：OTA 响应下发什么地址。本期 `LanAdvertiser`（`detect_all_lan_ipv4s` + 实际端口 → `ws://<ip>:<port>/robot/v1`，vision URL 同源推导）；未来 `RelayAdvertiser` 返回中继 wss/https 地址。中继阶段 OTA 与 vision 的 HTTP 请求同样经中继隧道转发，属 `RelayTunnelSource` 的职责范围。

中继上线时协议编解码、音频管线、会话桥、MCP 桥零改动。

### 2.2 会话 actor 与设备注册表

每条 WS 连接一个 **`RobotSession` actor**：

```
RobotLink ──> RobotSession
               ├─ ProtocolCodec     hello / JSON 词汇 / v1 二进制帧编解码
               ├─ UplinkPipeline    Opus解码 → PCM缓冲 → Silero VAD → WAV → ASR(invoke层)
               ├─ DispatchBridge    stt文本 → 伙伴会话 turn → AgentStreamEvent 订阅 → 降级重试
               ├─ DownlinkPipeline  增量分句 → [emotion]剥离 → TTS(invoke层) → PCM → Opus 60ms帧 → pacing下发
               └─ McpBridge         initialize(vision url+token) / tools/list 缓存 / tools/call 代理
RobotRegistry   {data_dir}/robot/robots.json（原子写，仿伙伴档案模式）
```

注册表字段：

```jsonc
{
  "robots": [{
    "robot_id": "aa:bb:cc:dd:ee:ff",   // Device-Id(MAC)，主键
    "client_id": "<uuid>",              // 固件 NVS 持久 UUID
    "name": "书桌机器人",                // 默认按板型生成，UI 可改
    "companion_id": null,               // 绑定的伙伴；null = 未绑定
    "token_hash": "<sha256 hex>",       // 设备 token 哈希（明文仅在 OTA 响应下发）
    "activation_code": "483920",        // 未绑定时有效，按设备稳定不轮换，绑定后清空
    "board": "esp32-s3n16r8-emoji",
    "firmware_version": "1.9.0",
    "last_seen": "<rfc3339>",
    "created_at": "<rfc3339>"
  }]
}
```

### 2.3 状态推送

仿 `ssh.status`：`UserEventSink` 发用户级 `robot.status` 事件（离线 / 在线空闲 / 聆听 / 对话中），REST 快照与 WS 事件共用 DTO；UI 三段式消费。

## 3. 配对与设备生命周期（激活码）

1. **指向 nomifun**：用户在机器人配网页"高级设置"填 OTA 地址。伙伴远程 Tab 的"添加机器人"弹窗显示可复制的 `http://<本机LAN IP>:<端口>/robot/ota`（多网卡列出全部候选），并提示 LAN 开关状态。
2. **报到发码**：未知 Device-Id 首次 POST `/robot/ota` 时立即创建记录并生成 256-bit hex token（仿 companion access token 的 `generate_random_hex_secret` + sha256 存储），随 `websocket.token` 当场下发；同时生成 6 位激活码随 `activation{code,message,timeout_ms:30000}` 下发。设备屏显并逐位朗读激活码，循环 POST `/robot/ota/activate` 收 202。
3. **输码认领**：用户在伙伴"远程控制"Tab 的"机器人连接"节点击"添加机器人"→ 输入 6 位码 → 该设备 `companion_id` 写为当前伙伴。
4. **完成**：下一轮 activate 轮询返回 200，设备退出激活态直接可用（token 早已在其 NVS，无需重启或刷新配置）。

约束与语义：
- 激活码按设备稳定、绑定后清空；输错码给出明确失败提示。
- 未绑定 token 连 WS 会在 hello 后被拒绝并断开，不能触发任何模型调用。
- **换绑/解绑**：UI 直接改 `companion_id`；会话线程按 `(robot_id, companion_id)` 维度隔离，换绑自然开新线程，旧线程保留。解绑后设备回到未绑定态（OTA 重新下发激活码）。
- 删除设备：吊销 token + 移除记录；设备再报到视为新设备。
- 一个伙伴可绑多台机器人；一台机器人同一时刻只属于一个伙伴。

## 4. 语音会话管线

**上行（说话 → 文本）**：
1. 设备 `listen start`（mode auto|manual）后收 16kHz/60ms 裸 Opus 帧 → libopus 解码累积 PCM（60 秒上限护栏）。
2. 断句：**Silero VAD（本地 ONNX 推理）逐帧判定**——语音起始后，连续静音超过"停顿判停时长"（默认 700ms）即收尾；`manual` 模式等设备 `listen stop`。灵敏度与停顿时长为伙伴级可调参数（§6）。
3. PCM 拼 WAV 头（约 44 字节，无需依赖）→ 经 `ModelInvokeService` 调伙伴 ASR 槽位模型 → 得文本。
4. 发 `{"type":"stt","text":...}` 屏显 → 注入伙伴会话 dispatch 开始 turn。
5. 空文本/纯噪声：发一对空 `tts start/stop` 让设备回聆听，不打扰模型。
6. 唤醒词前置音频（约 2 秒，在 `listen detect` 之前到达）：本期**丢弃**（不做声纹校验），避免唤醒词本身混入 ASR 文本；`listen start` 之后的音频才进入 PCM 缓冲。

**下行（回复 → 语音，流式分句）**：
1. 订阅会话 `AgentStreamEvent` broadcast，增量累积助手文本，按中英文句读（。！？!?；;\n 等）切句。
2. 首句就绪发 `{"type":"tts","state":"start"}`（这是设备进入播放态、接收音频的唯一开关）。
3. 每句：剥离句首 `[emotion:xx]` 标记 → 合法名发 `{"type":"llm","emotion":...}`（非法回落 neutral）→ 发 `{"type":"tts","state":"sentence_start","text":<剥离后文本>}` 屏显 → TTS 合成（经 `ModelInvokeService`，优先 `format:"pcm"` 24kHz 零解容器；容器格式经 symphonia 解码 + rubato 重采样到 24kHz 单声道）→ libopus 编码 24kHz/60ms 帧 → **按实时节奏 pacing 逐帧下发**。
4. 句间预取：合成第 n+1 句时播第 n 句，压首句时延。
5. turn 结束且尾句播完发 `{"type":"tts","state":"stop"}`；auto 模式设备自动回聆听形成连续对话。
6. 服务端 hello 回 `{"type":"hello","transport":"websocket","session_id":<uuid>,"audio_params":{"format":"opus","sample_rate":24000,"channels":1,"frame_duration":60}}`。

**打断**：设备 `abort`（含 wake_word_detected）→ 立即冲刷下行队列、取消 TTS 合成、发 `tts stop`，并调 `ConversationService::cancel_with_origin` 中止 turn（不杀 runtime）。

**保活**：网关每 60 秒发 `{"type":"ping"}`（固件对未知类型仅记日志，无副作用），避开设备 120 秒超时。

**并发**：每设备独立 actor；同一伙伴多端（桌面/IM/多台机器人）并用时，记忆经伙伴记忆系统天然共享，线程互不干扰。

## 5. 人格与会话形态

每 `(robot_id, companion_id)` 一条长期 `type='nomi'` 会话，`extra` 标记 `{robot_session: true, robot_id, companion_id}`（仿渠道会话；会话列表过滤器排除之，不混入普通工作会话）。

系统提示 = `build_companion_system_prompt(channel_platform=Some("robot"))`（人格 preset/custom、记忆注入三类过滤、渠道风味全部复用）+ 追加"物理身体"段：

- 你有一具物理身体：OLED 表情屏、可转动的头（云台）、扬声器与麦克风；
- 回复必须简短口语化（语音播报场景，建议每句 ≤ 40 字、总长 ≤ 3 句，除非用户明确要长内容）；
- 情绪标记协议：每句可选以 `[emotion:名]` 开头（限 21 个规范名），驱动表情与头部动作；
- 设备工具（云台/音量/状态等，经 MCP 桥注入时）的使用提示。

dispatch 复用 `send_message_with_idempotency_key` + `wait_for_runtime_subscription`；调用主体继承安装所有者（`authoritative_user_id`）。

## 6. 伙伴模型槽位（总览页"模型配置"重写）

`CompanionProfileConfig` 扩展（可选字段 + 默认值，向后兼容旧档案）：

| 槽位 | 字段 | 未配置时的回落 |
|---|---|---|
| 主对话模型 | `model`（现有） | 引导配置（现有 attention 逻辑） |
| 备用对话模型 | `fallback_model: Option<ProviderWithModel>`（新） | 无备用，失败即报错 |
| 语音活动检测 | `voice.vad { engine: "silero", sensitivity, min_silence_ms }` | 默认参数的内置 Silero |
| 语音识别 ASR | `voice.asr: Option<ProviderWithModel>` | 全局 `tools.speechToText` |
| 视觉大模型 | `vision_model: Option<ProviderWithModel>` | 主对话模型若带 `vision_input` trait 则用之 |
| 语音模型 TTS | `voice.tts: Option<{provider_id, model, voice}>` | 全局 `tools.textToSpeech`（本期新增的全局偏好，与 STT 命名对称） |

**备用对话模型语义（边界明确）**：
- 配置级失效（供应商被删/禁用、模型行失效）：模型解析链自动回落备用，全端生效。
- 运行时调用失败（API 报错）：**本期只在机器人链路做**——DispatchBridge 捕获 turn 失败且错误可归因于 provider 时，以备用模型重放该 turn 一次，日志与 UI 标注"已降级"；桌面聊天链路的运行时降级涉及 agent 引擎深改，列为后续。

总览页 `ModelsSection` 重写为五组行（对话主/备、VAD、ASR、视觉、TTS+音色），全部使用共享 `TaskModelSelect` 组件（§7），每行未配置时显示回落说明文案。原"语音与感知是应用级设置"的跳转行删除。

## 7. 模型管理重构（按模态）

管理页从"供应商为主视图"翻转为"**模态为主视图**"，左侧分区：**对话 / 语音 / 视觉 / 创作 / 嵌入与检索 / 免费模型 / 供应商与密钥 / 全局**。

- 模态区是 `provider_models` 按 `ModelTask` 的**过滤投影**（后端数据模型零改动）：对话 = chat；视觉 = 带 `vision_input` trait 的 chat 投影；语音 = speech_synthesis + speech_recognition + VAD 本地模型条目；创作 = image_generation/image_edit/video_generation；嵌入与检索 = embedding + rerank。每区内可直接启停模型、编辑任务归属/描述，并设置该模态的全局默认。
- **语音区（本期重点）**：TTS 首次获得配置面（全局默认：模型 + 音色，落 `tools.textToSpeech` 偏好，后端路由与偏好读取一并新增）；ASR 从旧的硬编码 provider 结构迁移为目录引用形态（兼容读取老配置，一次性迁移）；VAD 显示为"内置 Silero VAD（本地）"条目及其默认参数。音色（voice）字段为自由文本 + 已知供应商的建议列表。
- **供应商与密钥**区承接现在的两级折叠列表（`ModelModalContent`），职责收窄为：接入厂商、凭证与连接档案、模型行增删与高级覆写——不再承担"按用途找模型"的职责。
- 收敛共享组件 **`TaskModelSelect`**：`(task, traits)` → 供应商+模型二联选择器（TTS 变体带音色第三联），统一失效引用降级（"(不可用)"disabled 项）、空态提示、加载防抖（沿用 `useModelsForTask` 的 fail-safe）。本期至少替换：伙伴总览页全部槽位、语音区配置面；其余消费点（会话页/知识库/创作工坊等 8 处）逐步迁移，不强求本期完成。
- 已知债务（P1 protocol 列争用、P2 协议清单漂移、P3 双写路径等，见 `docs/specs/2026-07-28-multimodal-model-provider-redesign.zh.md` 偏差记录）**不在本期修复范围**，除非重构中顺手且零风险。

## 8. MCP 工具桥与云台

**工具桥（nomifun 侧）**：WS hello 后网关作为 MCP 客户端发 `initialize`，`capabilities.vision = {url: "<http基址>/robot/vision/explain", token: <设备token>}`；随后 `tools/list`（处理 8000 字节分页）缓存设备工具清单。实现为 `McpTransport` trait 的新实现（JSON-RPC 塞进 `{session_id, type:"mcp", payload}` 信封），不改 nomi-mcp 的 manager/tool_proxy。约束遵守：请求 id 用数字；不依赖 notification 送达。

设备工具以 `robot_` 前缀注入该伙伴的机器人会话（schema 转换自 MCP inputSchema）；模型调用 → RobotSession 经 WS 发 `tools/call`（30 秒超时，`stackSize` 按工具需要显式传大）→ 结果回填。设备离线时工具即时返回"设备离线"错误。

**固件改动（xiaozhi-yuntai，本期一并做，约 40 行）**：emoji 板注册云台 MCP 工具，参照 `main/boards/electron-bot/electron_bot_controller.cc` 样板，在 `EmojiBoard` 构造期注册：
- `self.gimbal.look(direction: left|right|up|down|center)`
- `self.gimbal.set(pan: int 50-130, tilt: int 70-110)`（Property 显式声明限位）
- `self.gimbal.get_position`

长动作（点头/摇头/转圈）投递到现有 `EmojiController` 动画队列，不在 tool 线程同步阻塞（默认栈 6144 且 `HeadRoll` 秒级耗时）。

## 9. 视觉

`/robot/vision/explain` 流式解析 chunked multipart（无 Content-Length；boundary 以 header 为准）→ 取 JPEG + question → 尺寸收敛（复用 `image_attachments.rs` 的限流常量思路与 `image` crate）→ **直调 `nomi_providers::LlmProvider::stream` + `ContentBlock::Image`**（一次性 VLM 调用，不经 agent 引擎、不走 invoke 层）→ 模型选伙伴 `vision_model` 槽位，回落主对话模型的视觉能力；均无则返回 `{"success":false,"message":"未配置视觉模型"}`（仍以 200 返回，让模型看到原因）。30 秒硬时限内响应。

当前 emoji 板无摄像头：端点与槽位先就绪，设备侧接上摄像头板即用（固件已支持 `self.camera.take_photo`）。

## 10. 错误处理

| 场景 | 行为 |
|---|---|
| 未知设备/坏 token 连 WS | hello 后拒绝并断开；设备屏显连接失败 |
| 已发 token 但未绑定伙伴 | WS 拒绝会话；OTA 持续下发 activation |
| ASR 失败/空结果 | 空转一对 `tts start/stop`，设备回聆听；记日志 |
| 模型 turn 失败 | 有备用模型则降级重放一次；仍失败发 `llm emotion:sad` + `sentence_start`（简短错误文案屏显）+ `tts stop`，不卡 speaking 态 |
| TTS 单句失败 | 该句降级纯屏显（sentence_start 已发），继续后续句；连续失败终止本 turn 下行并发 `tts stop` |
| 设备中途断线 | 清理 actor、取消上/下行管线与进行中 turn 订阅，会话线程保留；重连续用同一线程 |
| PCM 缓冲超 60 秒 | 强制收尾送 ASR |
| VAD 模型加载失败 | 回落能量 VAD（RMS 滑窗）并记警告，不阻断链路 |
| LAN 监听器未开启 | 添加机器人弹窗提示并引导开启；已绑定设备离线状态照常呈现 |

**可观测性**：每设备结构化日志（连接/绑定/ASR 文本与耗时/turn 首句时延/TTS 耗时/工具调用/降级事件），沿用平台现有日志风格。

## 11. UI 设计

**伙伴"远程控制"Tab 新增"机器人连接"section**（插在现有"远程连接"IM 节之后）：
- 已绑定机器人列表：名称（可改）、板型、在线状态药丸（`robot.status` 实时）、固件版本、last_seen；操作：重命名 / 解绑 / 删除。
- "添加机器人"按钮 → 弹窗：显示本机 OTA URL（多网卡候选 + 复制按钮 + LAN 开关状态提示）+ 6 位激活码输入框。
- attention 聚合：并入现有 `onAttentionChange`（如绑定设备 token 失效等异常态）。
- 文案区分：现有 IM 节按钮文案改为"连接 IM 机器人"；新节所有文案围绕"机器人设备/实体机器人"。i18n zh/en 双份（`localeKeyParity` 校验），结构测试跟上。

**伙伴总览页**：`ModelsSection` 重写为五组槽位行（§6）。

**模型管理页**：按模态分区重构（§7）。

## 12. 测试策略

- **单元**：协议 JSON 词汇与 v1 帧编解码；分句器（中英文、`[emotion]` 剥离、流式增量）；PCM↔Opus 回环；Silero VAD 判停（内置样本 WAV，含能量 VAD 回落）；WAV 头拼装；激活码/token 流转；`EndpointAdvertiser` 地址生成。
- **集成（不依赖真机）**：Rust 模拟设备客户端完整扮演固件——OTA 报到 → 激活轮询 → 绑定 → WS hello → 发预录 Opus → 断言 stt/llm/tts 消息序列与音频帧节奏 → abort 打断 → 断线重连 → MCP initialize/tools 桥。ASR/TTS/对话模型以 mock 注入。
- **UI**：新组件结构测试、i18n 键齐全性、`TaskModelSelect` 失效引用降级行为、robot.status wire 契约测试（仿 ssh-status-wire）。
- **真机验收**：配网指向开发机 → 激活绑定 → 连续对话（auto 与 manual）→ 打断即停 → 表情联动 → 云台工具调用 → （接摄像头后）拍照理解。

## 13. 新增依赖与风险

**Rust 新增依赖**：libopus 绑定（Opus 编解码，vendored 静态链接）、`ort` + Silero VAD 权重（约 2MB，随应用打包）、`symphonia`（mp3/wav 等容器解码）、`rubato`（重采样）。

**风险与降级路径**：
1. `ort`（onnxruntime）三平台打包是本期最大工程不确定点；受阻时降级为能量 VAD 先行、模型 VAD 随后（VAD 已抽象为 engine 可切换，`voice.vad.engine` 字段留好）。
2. VAD 判停参数需真机调参（默认值仅为起点，伙伴级参数可调）。
3. 模型管理重构面积大，但与机器人链路解耦，可并行或分先后实施。
4. 打断拖尾体验完全取决于服务端停流速度，实现时下行队列必须支持即时冲刷。

## 附：固件侧改动清单（xiaozhi-yuntai）

1. **必须（零代码）**：配网页高级设置将 OTA 地址指向 nomifun（`http://<LAN IP>:<端口>/robot/ota`）。
2. **本期（约 40 行）**：emoji 板注册云台 MCP 工具（§8）。
3. **可选（后续）**：接摄像头启用 `self.camera.take_photo`（固件已支持，vision URL 由 MCP initialize 下发）。

## 附：实现分期建议（供计划阶段参考）

1. **Phase 1 — 机器人域地基**：`nomifun-robot` crate、注册表、OTA/activate 端点、帧层抽象（trait + LAN 实现）、路由挂载、`robot.status` 事件。
2. **Phase 2 — 语音主链路**：WS 端点与协议编解码、音频管线（Opus/Silero VAD/ASR/TTS/pacing）、伙伴会话桥与流式分句下行、情绪标记、打断。
3. **Phase 3 — 模型体系**：伙伴档案槽位扩展、`tools.textToSpeech` 全局偏好、总览页 ModelsSection 重写、`TaskModelSelect` 组件、模型管理按模态重构。
4. **Phase 4 — 能力桥与 UI 收尾**：MCP 工具桥 + vision explain、伙伴远程 Tab"机器人连接"section、模拟设备集成测试完善、（固件仓库）云台 MCP 工具。
