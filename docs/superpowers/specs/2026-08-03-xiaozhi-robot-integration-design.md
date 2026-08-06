# xiaozhi 物理机器人 ↔ nomifun 对接设计

- 日期:2026-08-03
- 状态:**已取代** —— 2026-08-06 用户决定推倒重来,请以 [`2026-08-06-robot-bridge-design.md`](2026-08-06-robot-bridge-design.md) 为准
- 原状态:已获用户确认(逐节评审通过)
- 涉及仓库:`nomifun-tauri`(主要实现)、`xiaozhi-yuntai`(固件,仅配置 + 可选小改)

## 0. 背景与目标

用户有一台基于 xiaozhi-yuntai 固件的 ESP32-S3 物理机器人(板型 `esp32-s3n16r8-emoji`:OLED 表情屏、双舵机云台 pan/tilt、INMP441 麦克风、Max98357A 扬声器,暂无摄像头),需要以 nomifun 作为其唯一后台,打通四件事:

1. **人格对接**:机器人是某个已有桌面伙伴的"物理化身",共享其人格、模型配置与记忆,通过激活码配对绑定,支持换绑。
2. **多模型对接**:对话模型复用伙伴模型配置;STT/TTS/视觉走 nomifun 现有 model-invoke 多提供商层。
3. **网络通讯**:实现 xiaozhi WebSocket 协议(JSON 文本帧 + 二进制 Opus 帧),同时支持局域网直连与公网 wss。
4. **OTA 地址管理**:nomifun 下发 websocket 配置,并托管固件版本清单与 .bin 文件。

### 已确认的方向性决策

| 决策点 | 结论 |
|---|---|
| 人格身份 | 绑定已有伙伴(物理化身,共享人格/模型/记忆,可换绑) |
| 网络拓扑 | LAN 直连与公网 wss 两者都支持 |
| 服务端位置 | 内置于 nomifun 后端(新 crate,单进程) |
| OTA 范围 | 配置下发 + 固件清单与 .bin 完整托管 |
| 接入架构 | 独立设备网关 crate(方案 B),不复用渠道插件 trait |

### 关键前提(两侧代码考察结论)

**固件侧(xiaozhi-yuntai)**:
- 一切服务端配置来自 OTA 接口:设备启动 POST 设备报告到 `ota_url`(NVS `wifi.ota_url` 覆盖编译期 `CONFIG_OTA_URL`),响应中 `websocket{url,token,version}` 被原样写入 NVS 并决定连接目标(`main/ota.cc:144-184`)。**响应缺 `websocket` 对象会回落 MQTT 并硬报错——OTA 响应必须永远带 `websocket`**。
- 传输:单条 WS,文本帧 JSON + 二进制帧 Opus。设备身份在升级请求头:`Authorization: Bearer <token>`、`Protocol-Version`、`Device-Id`(MAC)、`Client-Id`(NVS 持久 UUID)(`main/protocols/websocket_protocol.cc:100-109`)。
- 音频:Opus 单声道,上行 16kHz/60ms;下行默认 24kHz/60ms,**服务端 hello 的 `audio_params` 可覆盖**。二进制帧版本由 OTA 下发的 `websocket.version` 决定,v1 = 裸 Opus 负载(最简,本设计选 v1)。
- JSON 消息词汇:设备→服务端 `hello`/`listen`(start|stop|detect)/`abort`/`mcp`;服务端→设备 `hello`/`stt`/`llm`(emotion)/`tts`(start|stop|sentence_start)/`mcp`/`system`(reboot)/`alert`/`custom`。WS 路径无 goodbye,断开即会话终止。设备 120s 无入站流量判死。
- 设备是 MCP server:服务端发 `initialize`(可携带 `capabilities.vision.{url,token}` —— **这是视觉 explain URL 唯一的下发通道**,不走 OTA)→ `tools/list` → `tools/call`。通用工具:`self.get_device_status`、`self.audio_speaker.set_volume`;有屏加 `self.screen.set_brightness/set_theme`;有摄像头加 `self.camera.take_photo(question)`(JPEG multipart POST 到 vision URL,头带 Device-Id/Client-Id/Bearer)。
- 表情:`{"type":"llm","emotion":"<名>"}`,21 个规范名:neutral, happy, laughing, funny, sad, angry, crying, loving, embarrassed, surprised, shocked, thinking, winking, cool, relaxed, delicious, kissy, confident, sleepy, silly, confused。emoji 板将表情映射为眼睛动画 + 舵机动作(点头/摇头/转头),未知名回落 neutral。
- **emoji 板未注册任何板级 MCP 工具**:云台(pan GPIO11 50-130°、tilt GPIO12 70-110°、中位 90/90)目前只能被表情联动与中文关键词间接驱动。
- 激活:OTA 响应带 `activation{message,code,challenge?,timeout_ms}` 时设备进入激活态,屏显/播报 code 并轮询 `<ota_url>/activate`(202=继续等,200=完成)。

**nomifun 侧**:
- 伙伴人格:`CompanionProfileConfig`(`companion/companions/{id}/config.json`,含 `persona{preset,custom}`、`model: Option<ProviderWithModel>`);系统提示由 `build_companion_system_prompt(store, profile, channel_platform, …)` 构建,`channel_platform=Some(..)` 会启用远程渠道风味并将记忆注入过滤为 profile/preference/knowledge 三类。
- 会话:渠道会话是 `type='nomi'` conversation + `extra` 标记;智能体输出有 `broadcast::Receiver<AgentStreamEvent>` 实时流(渠道域 `ChannelStreamRelay` 是消费范例)。
- 模型:会话创建可带 `model: Option<ModelRefParam>`,自动解析链"伙伴模型 → 首个已配置 provider"。
- 语音:已有 `POST /api/tts`(TtsApiRequest,opus 格式输出为 **Ogg 容器**非裸帧)与 `POST /api/stt`(multipart),底层 model-invoke 适配 openai/deepgram/volc/minimax;两端点仅限会话 JWT,设备网关应**直接调用 invoke 层**而非走这两个 HTTP 端点。
- 现有 `/ws` 忽略二进制帧、companion token 不可用于 `/ws`、`/v1` 无流式——设备通道必须是全新 WS 端点。
- nomifun 目前无任何设备/OTA 概念;新增路由须挂在会话鉴权中间件之外(同 `/mcp`、`/v1` 的挂载方式);设备请求无 Cookie,天然不受 CSRF 层影响;LAN 监听器 host_guard 对裸 IP Host 放行,ESP32 直连无障碍。

## 1. 总体架构

新增 `crates/backend/nomifun-device` crate,由 `nomifun-app::create_router` 合并挂载,自动获得桌面 LAN(25808)、headless(8787)、公网(Caddy 反代 wss)三个监听面。

对设备暴露(均在会话鉴权之外):

| 端点 | 方法 | 作用 |
|---|---|---|
| `/xiaozhi/ota` | POST(兼容 GET) | 设备报到:upsert 设备记录,下发 websocket 配置+token、激活码(未绑定时)、固件清单、server_time |
| `/xiaozhi/ota/activate` | POST | 激活轮询:设备未绑定伙伴返回 202,已绑定返回 200 |
| `/xiaozhi/v1` | WS | 设备主通道:xiaozhi 协议(JSON + 二进制 Opus v1 帧),Bearer token 鉴权 |
| `/xiaozhi/vision/explain` | POST | 拍照理解:multipart(question + jpeg)→ 视觉模型 → `{success, text}` |
| `/xiaozhi/firmware/{token}/{file}.bin` | GET | 固件下载(固件下载器不带鉴权头,以不可猜路径段兜底) |

内部依赖:伙伴档案与系统提示构建(nomifun-companion)、会话创建与 dispatch、`AgentStreamEvent` 广播、model-invoke(STT/TTS/视觉)。新增纯 Rust 音频依赖:libopus 绑定(编解码)、symphonia(容器解码)、重采样(rubato 或线性)。

组件划分(每设备连接一个会话 actor):

```
WS 连接 ──> DeviceSession actor
             ├─ ProtocolCodec     hello/JSON 词汇/二进制 v1 帧编解码
             ├─ UplinkPipeline    Opus解码 → PCM缓冲 → VAD(auto模式) → WAV → STT invoke
             ├─ DispatchBridge    stt文本 → 伙伴会话 turn → AgentStreamEvent 订阅
             ├─ DownlinkPipeline  分句 → 情绪标记剥离 → TTS invoke → PCM → Opus帧 → 下发
             └─ McpBridge         initialize(vision url) / tools/list 缓存 / tools/call 代理
DeviceRegistry  设备表(JSON 文件,原子写)
FirmwareStore   {data_dir}/device/firmware/ 清单+bin
```

## 2. 设备注册与配对(人格对接入口)

**存储**:`{data_dir}/device/devices.json`(原子写,仿伙伴档案的 JSON 文件模式)。字段:

```jsonc
{
  "devices": [{
    "device_id": "aa:bb:cc:dd:ee:ff",   // MAC,主键
    "client_id": "<uuid>",
    "name": "书桌机器人",                // 默认按板型生成,UI 可改
    "companion_id": null,                // 绑定的伙伴;null = 未绑定
    "token_hash": "<sha256 hex>",        // 设备 token 的哈希(明文仅下发)
    "activation_code": "483920",         // 未绑定时有效,绑定后清空
    "board": "esp32-s3n16r8-emoji",
    "firmware_version": "x.y.z",
    "last_seen": "<rfc3339>",
    "endpoint_mode": "auto",             // auto | lan | public
    "voice": null                        // 可选 TTS 覆盖 {provider_id, model, voice}
  }]
}
```

**token 首次报到即发**:未知 Device-Id 第一次 POST `/xiaozhi/ota` 时立即创建记录并生成 256-bit hex token(仿 companion access token 的 `generate_random_hex_secret` + sha256 存储模式),随 `websocket.token` 当场下发。绑定动作只是"点亮 companion_id"——绑定完成瞬间设备凭证已在其 NVS 中,无需刷新配置或重启。未绑定 token 连 WS 会在 hello 后被拒绝(见 §8),不能触发任何模型调用。

**配对流程**:
1. 未绑定设备的 OTA 响应携带 `activation{code: 6位数字, message: "请在 nomifun 中输入此码绑定伙伴", timeout_ms: 30000}`(不带 challenge,走固件 Activation-Version 1 简单流程);激活码按设备稳定(不轮换),绑定后清空。
2. 设备屏显/播报激活码,循环 POST `/xiaozhi/ota/activate` → 202。
3. 用户在 nomifun UI"设备"页输入激活码 → 选择绑定的伙伴 → 写入 `companion_id`。
4. 下一次 activate 轮询返回 200,设备退出激活态继续启动。

安全性说明:激活码方向是"用户向自己的 UI 输码认领设备",攻击者无 UI 访问权,码本身不构成攻击面;公网场景以 OTA 端点限速兜底(§6)。

**换绑/解绑**:UI 直接改 `companion_id`;设备会话线程按 `(device_id, companion_id)` 维度查找,换绑后自然新开线程,旧线程保留。

**UI**:新增"设备"管理页(位置随现有设置信息架构,计划阶段定):设备列表(在线状态、板型、固件版本、绑定伙伴、last_seen)、输码绑定入口、改名/换绑/解绑、endpoint_mode 与 voice 覆盖、固件上传。

## 3. 语音会话与人格(核心管线)

**会话形态**:每 `(device_id, companion_id)` 一条长期 `type='nomi'` 会话线程,`extra` 标记 `{device_session: true, device_id, companion_id}`(仿渠道会话)。系统提示 = `build_companion_system_prompt(channel_platform=Some("xiaozhi-robot"))`(人格、preset、记忆注入三类过滤、渠道风味,全部复用)+ 追加机器人身体说明段:

- 你有一具物理身体:OLED 表情、可转动的头(云台)、扬声器与麦克风;
- 回复必须简短口语化(语音播报场景,建议每句 ≤ 40 字,总长 ≤ 3 句,除非用户明确要长内容);
- 情绪标记协议:每句可选以 `[emotion名]` 开头(限 21 个规范名),用于驱动表情与头部动作;
- 可用工具(若设备工具桥已就绪)的使用提示。

**上行(用户说话 → 文本)**:
1. 设备 `listen start`(mode auto|manual)后开始收二进制 Opus 帧(16kHz/60ms,v1 裸负载)→ libopus 解码累积 PCM(上限 60s,防失控)。
2. 收尾判定:manual 模式等 `listen stop`;auto 模式用服务端能量 VAD——RMS 滑窗判语音起始,起始后连续 ~700ms 低于阈值判停(阈值/挂起时长设为可调常量,真机调参)。
3. PCM 打包 WAV → 调 model-invoke STT(复用现有 `tools.speechToText` 偏好:openai/deepgram/volc)→ 得文本。
4. 向设备发 `{"type":"stt","text":...}` 屏显,随即注入伙伴会话 dispatch 开始 turn。
5. 空文本/纯噪声:直接发 `tts start` + `tts stop` 空转一次,设备回到聆听,不打扰模型。

**下行(模型回复 → 语音,流式分句)**:
1. 订阅该会话 `AgentStreamEvent` broadcast,增量累积助手文本,按中英文句读(。!?!?;\n 等)切句。
2. 首句就绪时发 `{"type":"tts","state":"start"}`。
3. 每句:剥离句首 `[emotion]` 标记 → 若有,发 `{"type":"llm","emotion":...}`(非法名回落 neutral)→ 发 `{"type":"tts","state":"sentence_start","text":<剥离后文本>}` 屏显 → TTS 合成(§4)→ PCM 重采样至 24kHz 单声道 → libopus 编码 60ms 帧逐帧下发(按实时节奏 pacing,避免撑爆设备 2.4s 播放队列)。
4. turn 结束且尾句播完:发 `{"type":"tts","state":"stop"}`。auto 模式设备自动回到聆听,形成连续对话。
5. TTS 音频获取:openai 请求 `pcm`(24kHz)零解容器;其余容器格式(mp3/ogg 等)经 symphonia 解码。句间 TTS 预取(合成第 n+1 句时播第 n 句)压时延。

**打断**:设备 `abort`(含 wake_word_detected)→ 立即取消下行管线(丢弃未发帧、停止 TTS 合成)+ 请求平台中止当前 turn(复用渠道域已有的 turn 中止机制,计划阶段定位具体 API)。

**并发**:每设备独立 actor;同一伙伴多设备/桌面并用时,记忆经由伙伴记忆系统天然共享,线程互不干扰。

## 4. 多模型对接

- **对话模型**:零新概念。会话创建走 `ModelRefParam` 自动解析链(伙伴模型 → 首个已配置 provider);在 UI 改伙伴模型即全端生效(桌面+机器人)。
- **STT**:默认取现有 `tools.speechToText` 偏好;v1 不做设备级覆盖。
- **TTS**:新增全局偏好 `tools.textToSpeech {provider_id, model, voice}`(命名与 STT 对称);设备级 `voice` 字段可覆盖(每台机器人可有专属音色)。
- **视觉**:`/xiaozhi/vision/explain` 用"伙伴模型若具视觉能力则用之,否则用全局默认视觉模型"策略,直调 model-invoke(图片 + question → 文本),返回 `{"success":true,"text":...}`(与固件 take_photo 工具期望的 JSON 兼容)。

## 5. 网络通讯(双拓扑)

- 设备网关设置(全局):`lan_base_url`(如 `ws://192.168.x.x:25808`)与 `public_base_url`(如 `wss://robot.example.com`),对应 http(s) 基址由同源推导。
- OTA 处理器按请求来源自动选:来源 IP 属私网段(10/8、172.16/12、192.168/16、127/8)→ 下发 LAN 地址;否则下发公网地址。设备级 `endpoint_mode` 可锁定 lan|public。
- LAN:挂现有 LAN 监听器/headless 端口,host_guard 放行 IP Host,已验证无障碍;无 TLS(ESP32 免证书负担)。
- 公网:Caddy 反代 `/xiaozhi/*` 与 WS 升级(现有 Caddyfile 模式即可),wss/https 终止在 Caddy,零后端代码。
- **保活**:固件 120s 无入站流量判死;网关每 60s 向设备发一条 `{"type":"ping"}`(固件对未知 type 仅记日志,无副作用)。设备断开(socket 关闭)即清理 actor;WS 路径无 goodbye 语义。
- 服务端 hello 回 `{"type":"hello","transport":"websocket","session_id":<uuid>,"audio_params":{"format":"opus","sample_rate":24000,"channels":1,"frame_duration":60}}`。

## 6. OTA 地址管理与固件托管

**OTA 响应**(POST `/xiaozhi/ota`,解析固件 version 2 设备报告,upsert 注册表):

```jsonc
{
  "websocket": { "url": "<按拓扑选择的 ws(s) 地址>/xiaozhi/v1", "token": "<设备token明文>", "version": 1 },
  "server_time": { "timestamp": <毫秒>, "timezone_offset": <分钟> },
  "firmware": { "version": "<清单版本或设备当前版本>", "url": "<有新版时的下载地址>" },
  "activation": { /* 仅未绑定时出现 */ "code": "483920", "message": "...", "timeout_ms": 30000 }
}
```

- **永远包含 `websocket` 对象**(固件回落约束);不含 `mqtt`。
- 无新固件时 `firmware.version` 回设备当前版本(固件自行判定无需升级)。

**固件托管**:`{data_dir}/device/firmware/` 下放 `manifest.json {version, file, force?}` 与 `.bin`;UI 上传或手动放文件即完成发布。下载路径含随机 token 段(发布时生成),固件下载器不带鉴权头,以不可猜 URL 兜底。

**安全边界(明确接受)**:OTA 首次请求无鉴权是 xiaozhi 生态固有形态。缓解:OTA 端点按 IP 限速;未绑定设备无模型调用能力;固件路径不可猜;v1 不做固件签名(固件端仅可选 sha256)。公网暴露风险以此为界,后续可在 Caddy 层加更严格策略。

## 7. 设备能力反向控制(云台/工具桥)

- WS hello 完成后,网关作为 MCP 客户端发 `initialize`,`capabilities.vision = {url: "<http(s)基址>/xiaozhi/vision/explain", token: <设备token>}`;随后 `tools/list`(处理分页)缓存设备工具清单。
- **工具桥**:设备工具以 `robot_` 前缀注册为该伙伴会话的可用工具(如 `robot_self_gimbal_look`),schema 直接转换自 MCP inputSchema;模型调用 → DeviceSession 经 WS 发 `mcp tools/call`(30s 超时,固件在独立线程执行)→ 结果回填 tool result。具体注册点(nomi engine 会话级工具注入机制)在实现计划阶段确定;若会话级动态注入成本过高,退化方案是仅注入提示词描述 + 网关侧文本指令解析,但以动态注入为目标。
- 设备离线时工具调用立即返回"设备离线"错误。

**固件小改(推荐,~40 行 C++,`main/boards/esp32-s3n16r8-emoji/`)**:注册云台 MCP 工具
- `self.gimbal.look(direction: left|right|up|down|center)`
- `self.gimbal.set(pan: 50-130, tilt: 70-110)`
- `self.gimbal.center()`

不改固件也可用(表情联动已能动头),此改动只是让模型获得直接、参数化的云台控制。

## 8. 错误处理

| 场景 | 行为 |
|---|---|
| 未知设备/坏 token 连 WS | hello 后即拒绝并断开;设备屏显连接失败 |
| 已发 token 但未绑定伙伴 | WS 拒绝会话(提示先绑定);OTA 持续下发 activation |
| STT 失败/空结果 | 空转一次 tts start/stop,设备回聆听;记日志 |
| 模型 turn 失败 | 发 `llm emotion: sad` + `tts sentence_start`(简短错误文案屏显)+ `tts stop`,不卡 speaking 态;结构化日志记录错误 |
| TTS 失败 | 该句降级为纯屏显(sentence_start 已发),继续后续句;连续失败则终止 turn 下行并发 tts stop |
| 设备中途断线 | 取消上/下行管线与进行中 turn 订阅,会话线程保留;重连后续用同一线程 |
| PCM 缓冲超限(60s) | 强制收尾进 STT,防内存失控 |

**可观测性**:每设备结构化日志(连接/绑定/STT 文本与耗时/turn 首句时延/TTS 耗时/工具调用),沿用平台现有日志与 delivery-receipt 风格,便于 Mac 部署侧排查。

## 9. 测试策略

- **单元**:hello 握手与 JSON 词汇序列化;v1 二进制帧;分句器(中英文/情绪标记剥离);PCM↔Opus 回环;VAD 判停(内置样本 WAV)。
- **集成(不依赖真机)**:模拟设备客户端(Rust 测试工具,完整扮演固件:OTA 报到 → 激活轮询 → 绑定 → WS hello → 发预录 Opus → 断言收到 stt/llm/tts 消息序列与音频帧;含 abort 打断与断线重连场景)。STT/TTS/模型以 mock provider 注入。
- **真机验收**:烧录/配置 `wifi.ota_url` 指向开发机 → 激活绑定 → 连续语音对话(auto 与 manual)→ 表情联动 → (若固件已加)云台工具调用 → 固件 OTA 升级一轮。

## 10. 非目标(YAGNI)

MQTT+UDP 传输、二进制帧 v2/v3、服务端 AEC/声纹/唤醒词检测(设备本地已有唤醒)、流式 ASR 边说边转、TTS 情感韵律控制、固件签名验证、设备分组/多租户、eFuse HMAC 激活(challenge 流程)。

## 附:固件侧改动清单

1. **必须(零代码)**:将 OTA 地址指向 nomifun——配网页面高级设置或 NVS `wifi.ota_url`;或重编译 `CONFIG_OTA_URL`。
2. **推荐(~40 行)**:emoji 板注册云台 MCP 工具(§7)。
3. **可选(后续)**:接入摄像头后启用 `self.camera.take_photo`(固件已支持,vision URL 由本设计 §7 的 MCP initialize 下发)。

## 附:实现分期建议(供计划阶段参考)

1. **Phase 1 — 设备域地基**:nomifun-device crate、设备注册表、OTA/activate 端点、固件托管、路由挂载与双拓扑地址选择。
2. **Phase 2 — 语音主链路**:WS 端点与协议编解码、音频管线(Opus/VAD/STT/TTS)、伙伴会话桥与流式分句下行、情绪标记。
3. **Phase 3 — 能力桥**:MCP initialize/工具桥、vision explain 端点。
4. **Phase 4 — 收尾**:设备管理 UI、模拟设备集成测试完善、(固件仓库)云台 MCP 工具。
