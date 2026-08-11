# NomiFun 移动远控（nomifun-mobile + nomifun-bridge-server + 桌面端 bridge 模块）设计

- 日期：2026-08-03
- 状态：已批准（用户确认框架选型/技术栈/配对方式/桌面端内嵌模块，并授权全程自主实施）
- 交付物：
  1. `~/src/nomifun-mobile` — uni-app（Vue3 + Vite + TS）三端应用（Android / iOS / H5）
  2. `~/src/nomifun-bridge-server` — Rust（axum/tokio）中继服务器（类 rustdesk-server，盲转发密文）
  3. `nomifun-tauri` 仓库内新增 `crates/backend/nomifun-bridge` 桥接模块 + 桌面端设置 UI

## 1. 目标与边界

手机端是"遥控器"，不是客户端全功能镜像：

- **能做**：向电脑上的会话下达指令（发消息到已有/新会话）、查看会话状态与进度快照、接收任务完成反馈（最终结果摘要）、处理待确认项（工具调用确认）、管理定时任务（cron CRUD + 立即执行）。
- **不做**：不在手机上调用任何模型能力；**不接收过程数据**（不订阅 `message.stream` 的 thinking/tool-call 流），只接收结果与配置列表，防止手机存储被打爆。
- 单条结果载荷上限 16 KB（超出截断并标记 `truncated`），列表接口分页且只含精简字段；手机本地仅持久化配置与最近 50 条事件。

## 2. 总体架构

```
┌────────────┐   LAN 直连(WS, E2E加密)    ┌──────────────────────────┐
│ nomifun-   │◄──────────────────────────►│ nomifun-tauri 桌面端      │
│ mobile     │                            │  └ nomifun-bridge crate   │
│ (uni-app)  │   公网中继(WS, E2E加密)     │     ├ LAN 监听 0.0.0.0:25810│
│            │◄──────────┐    ┌──────────►│     ├ 中继出站客户端        │
└────────────┘           ▼    ▼           │     ├ 配对管理(QR/配对码)   │
                  ┌──────────────┐        │     ├ 精简 RPC 处理器       │
                  │ nomifun-     │        │     └ 事件转发器            │
                  │ bridge-server│        │  复用: ConversationService, │
                  │ (盲转发密文)  │        │  CronService, BroadcastBus │
                  └──────────────┘        └──────────────────────────┘
```

- **两种连接方式并存**：
  - **局域网直连**：桌面 bridge 在 `0.0.0.0:25810`（dev 25811）开一个独立轻量监听（只挂 bridge 路由，与 WebUI LAN 监听互不依赖）。手机通过扫码（QR 内含 LAN 地址）或子网探测（并发 `GET /bridge/info`，300ms 超时）发现设备后直连 WS。
  - **公网中继**：桌面 bridge 主动**出站**连接中继服务器（穿 NAT），手机同样连接中继；中继按 `device_id` 撮合并盲转发密文帧。手机与电脑各自配置中继地址 + key（类 rustdesk）。
- 两种方式共用同一套 E2E 加密与 RPC 协议；中继模式下中继服务器只见密文。

## 3. E2E 加密方案

密码学原语选用 NaCl `crypto_box`（X25519 + XSalsa20-Poly1305），Rust 侧用 RustCrypto `crypto_box` crate，JS 侧用 `tweetnacl`，两者字节级互操作（用固定向量做互操作测试）。

- **身份**：桌面与每台手机各持一对 X25519 静态密钥。桌面密钥存于数据目录 `bridge/identity.key`（0600）；手机存于 `uni.setStorage`。`device_id` = `SHA-512(公钥)` 前 8 字节的小写 hex（16 字符）。
- **消息加密**：每帧 `payload = crypto_box(plaintext, nonce24_random, peer_pk, self_sk)`，信封为 `{v, from, pk?, n, c}`（以协议文档 §2 为准）；`ctr` 位于明文内层，为每方向单调递增计数器，接收方拒绝 `ctr ≤ last_seen`（抗重放）。前向保密列为后续工作（v1 不做，YAGNI）。
- **配对（密钥交换）**：
  1. 桌面设置页生成**一次性配对码 PC**（8 位，TTL 5 分钟，单次使用）+ 二维码。QR 内容：`{v:1, id, pk, lan:{ip,port}?, relay:{url,key}?}`（公钥经物理可信信道传递，无 MITM）。
  2. 手机扫码（H5/无摄像头时手动输入"桥接串"文本 + 配对码）→ 发送 `pair_request`：信封明文携带 `mobile_pk`，密文 = box(含 PC 的配对载荷, desktop_pk, mobile_sk)。
  3. 桌面校验 PC → 持久化 `mobile_pk`（`bridge/devices.json`，原子写）→ 回 `pair_ok`，内含 `HMAC-SHA256(key=HKDF(PC), msg=desktop_pk‖mobile_pk)`，手机校验后完成 TOFU 绑定——即使中继作恶也无法换钥（不知 PC 无法伪造 MAC）。
  4. 配对后中继模式全程端到端加密；桌面可在设置页吊销任一已配对设备。

## 4. 中继协议（nomifun-bridge-server）

明文控制帧 + 盲转发数据帧，JSON over WebSocket（`GET /ws`）：

- `{type:"register", role:"desktop"|"mobile", id:<device_id>, auth:<HMAC-SHA256(server_key, id‖ts)>, ts}` — server_key 即双方配置的中继 key，防开放中继滥用；时钟偏差容忍 ±5 分钟。
- `{type:"forward", to:<device_id>, ...E2E信封}` — 服务器按注册表转发给目标连接；目标离线回 `{type:"error", code:"peer_offline"}`。
- `{type:"ping"}/{type:"pong"}` 心跳（30s），死连接清理。
- 内存注册表 `device_id → sender`，无任何持久化；单帧上限 64 KB；每连接限速（每秒 30 帧）。
- 部署：单二进制，`nomifun-bridge-server --listen 0.0.0.0:21190 --key <key>`；提供 Dockerfile 与 docker-compose.yml；`GET /healthz`。

## 5. 桌面端 bridge 模块（crates/backend/nomifun-bridge）

- **BridgeService**：由 `nomifun-app` 组装（依赖 ConversationService、CronService、`BroadcastEventBus`），生命周期仿照 `desktop.rs::DesktopServer`（start/stop/status + `watch` 状态订阅）。Tauri 命令：`bridge_start/bridge_stop/bridge_get_status/bridge_generate_pairing/bridge_list_devices/bridge_revoke_device/bridge_set_relay_config`。
- **RPC 处理器**（解密后分发，JSON-RPC 风格 `{id, method, params}` → `{id, ok, result|error}`）：
  - `device.info` — 设备名/版本/能力
  - `conversations.list` — 分页，仅 `{id, title, status, updated_at}`
  - `conversations.send` — 向已有会话或新会话发指令（`origin` 标记为 `companion`）
  - `conversations.status` — 状态快照（Pending/Running/Finished + 当前轮次耗时）
  - `conversations.result` — 最近一次最终助手消息（≤16KB 截断）
  - `confirmations.list` / `confirmations.confirm`
  - `cron.list/create/update/delete/runNow`（复用 `CronService` 与现有 DTO 精简版）
- **事件转发器**：订阅用户事件总线，仅转发结果类事件到已连接手机（协议 §7）：`task.completed`（含结果摘要）、`conversations.attention`（由本地 `message.stream` 的 permission 元数据触发，仅带会话 ID）、`cron.executed`。**不转发 `message.stream` 过程内容**。
- **传输**：
  - LAN：独立 axum 监听 `0.0.0.0:25810`，路由仅 `GET /bridge/info`（无鉴权发现探针，返回 id/名称/公钥指纹/版本，CORS `*` 以支持 H5）与 `GET /bridge/ws`（鉴权即 E2E 配对本身：未配对客户端只能发起带有效配对码的 pair_request）。
  - 中继：tokio-tungstenite 出站连接，断线指数退避重连（1s→60s 封顶）。
- **存储**：数据目录 `bridge/` 下 `identity.key`（0600）、`devices.json`、`config.json`（中继地址/key/开关），原子写；不动 SQLite（v1 保持模块自包含、低侵入）。
- **桌面 UI**：设置页新增"远程桥接"分区（React SPA）：LAN 开关、中继配置、配对二维码弹窗（`qrcode.react` 已在依赖中）、已配对设备列表与吊销。

## 6. nomifun-mobile（uni-app）

- 技术栈：Vue3 + Vite + TypeScript + pinia；加密 `tweetnacl`；H5 一条命令起 dev server 调试；App 端 HBuilderX 云打包出 Android/iOS。
- 页面（保持极简，4 个 tab/页面）：
  1. **设备** — 已配对设备列表（在线状态）、添加设备（App 扫码 `uni.scanCode` / H5 粘贴桥接串 + 配对码、LAN 子网探测列表）
  2. **会话** — 会话列表（状态徽标）→ 详情：发指令输入框、状态快照、最近结果卡片、待确认项（确认/拒绝）
  3. **任务** — 定时任务列表 / 新建 / 编辑 / 删除 / 立即执行 / 最近执行结果
  4. **设置** — 中继服务器地址与 key、本机密钥指纹、清空本地缓存
- 连接层：统一 `BridgeClient`（策略：优先 LAN 直连，失败回退中继），E2E 封装与 RPC/事件分发独立成 `src/core/`（纯 TS，可用 vitest 在 node 下单测）。
- 存储纪律：`uni.setStorage` 仅存配对信息/中继配置/最近 50 条事件（每条 ≤8KB 摘要），无过程数据。

## 7. 错误处理

- 中继不可达/对端离线：UI 显式区分"中继不可达"与"电脑离线"；桌面端断线自动重连。
- 解密失败/计数器回退：静默丢帧并计数，连续 10 帧失败断开连接（防攻击噪声）。
- 配对码错误/过期：明确错误码 `pair_invalid_code`；配对码单次使用后立即失效。
- RPC 超时：手机端 15s 超时报错；幂等（`conversations.send` 复用桌面既有 idempotency key 机制）。

## 8. 测试策略

- **nomifun-bridge**（Rust 单测）：配对状态机、crypto_box 与 tweetnacl 固定向量互操作、抗重放计数器、RPC 路由与载荷截断。
- **nomifun-bridge-server**（Rust 集成测试）：双 WS 客户端注册-转发-离线错误、auth 失败拒绝、限速。
- **nomifun-mobile**（vitest）：`src/core/` 加密/信封/RPC 分发单测（与 Rust 侧共享测试向量）；H5 端到端手动验收。
- **端到端验收**：本机起 bridge-server（docker 或裸跑）+ 桌面端 + H5 手机端，走通"配对 → 发指令 → 收完成反馈 → 建定时任务 → 立即执行 → 收执行结果"。

## 9. 明确不做（v1）

- 前向保密（会话临时密钥轮换）、多桌面聚合视图、消息历史同步、文件传输、语音/推送通知（APNs/FCM）、中继服务器持久化与集群。
