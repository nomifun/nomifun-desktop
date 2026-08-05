# NomiFun Bridge Protocol v1（三端共享参考）

- 日期：2026-08-03
- 适用：`nomifun-tauri`（crates/backend/nomifun-bridge）、`nomifun-mobile`、`nomifun-bridge-server`
- 本文是**规范性**文档：三个项目的实现与测试以此为准；字段名、常量、算法不得偏离。

## 1. 角色与依赖库

| 角色 | 实现 | 加密库 |
|---|---|---|
| desktop | nomifun-tauri 内 `nomifun-bridge` crate | `crypto_box`（RustCrypto）、`sha2`、`hmac`、`hkdf`、`getrandom` |
| mobile | nomifun-mobile（uni-app, TS） | `tweetnacl`（box/hash）、`@noble/hashes`（hmac/sha256/hkdf） |
| relay | nomifun-bridge-server（Rust axum） | `sha2`、`hmac`（仅注册鉴权，不接触明文业务数据） |

## 2. 身份与加密

- 每端持有一对 **X25519 静态密钥**（NaCl box 密钥对）。
- `device_id` = `SHA-512(public_key_bytes)` 前 8 字节的小写 hex（16 个 hex 字符）。
  - Rust：`sha2::Sha512`；JS：`nacl.hash(pk)`。
- 加密：`c = crypto_box(plaintext_utf8_json, nonce, receiver_pk, sender_sk)`（X25519 + XSalsa20-Poly1305）。
  - nonce：每帧随机 24 字节。
- Base64：**标准字母表、带 padding**（Rust `base64::engine::general_purpose::STANDARD`；JS `uni.arrayBufferToBase64`/自实现）。

### 2.1 E2E 帧（E2EFrame）

```json
{"v":1,"from":"<device_id>","pk":"<base64 sender_pk，仅 pair_request 时携带>","n":"<base64 24B nonce>","c":"<base64 ciphertext>"}
```

- `pk` 仅在配对请求中出现（此时接收方还不知道发送方公钥）；其余场景省略，接收方按 `from` 查已配对公钥。
- 与中继控制帧的区分：JSON 含 `"type"` 键 → 控制帧；含 `"v"` 键 → E2E 帧。

### 2.2 明文内层消息

所有内层消息含 `kind` 与 `ctr`。`ctr` 为 **u64、按"发送方→接收方"方向严格递增、跨会话持久化**（desktop 存 `bridge/devices.json`，mobile 存本地 storage）。接收方拒绝 `ctr <= last_accepted`（静默丢弃并计数；同一连接连续 10 次失败 → 断开）。

```json
{"kind":"pair_request","ctr":1,"code":"<8位数字>","name":"<设备名>","platform":"android|ios|h5"}
{"kind":"pair_ok","ctr":1,"name":"<桌面名>","mac":"<hex>"}
{"kind":"pair_err","ctr":1,"code":"pair_invalid_code"}
{"kind":"rpc","ctr":N,"id":"<uuid>","method":"<见§5>","params":{}}
{"kind":"rpc_result","ctr":N,"id":"<uuid>","ok":true,"result":{}}
{"kind":"rpc_result","ctr":N,"id":"<uuid>","ok":false,"error":{"code":"","message":""}}
{"kind":"event","ctr":N,"name":"<见§6>","data":{}}
```

`pair_ok.mac` 计算：

```
prk  = HKDF-SHA256(ikm = utf8(code), salt = utf8("nomifun-bridge-pair-v1"), info = "", L = 32)
mac  = hex(HMAC-SHA256(key = prk, msg = desktop_pk_bytes || mobile_pk_bytes))
```

手机校验 mac 通过后才持久化桌面公钥（TOFU 绑定完成，防中继换钥）。

## 3. 中继协议（relay 明文控制帧，JSON over WS `/ws`）

```json
{"type":"register","role":"desktop|mobile","id":"<device_id>","ts":<unix_ms>,"auth":"<hex>"}
{"type":"registered"}
{"type":"forward","to":"<device_id>","frame":<E2EFrame>}
{"type":"deliver","from":"<device_id>","frame":<E2EFrame>}
{"type":"error","code":"auth_failed|peer_offline|rate_limited|too_large|bad_frame","to":"<可选>"}
{"type":"ping"}  {"type":"pong"}
```

- `auth = hex(HMAC-SHA256(key = utf8(server_key), msg = utf8(id + ":" + ts)))`；`|now - ts| > 300_000ms` 或 MAC 不符 → `error auth_failed` 并关闭。
- 同一 `id` 重复注册：新连接顶替旧连接（旧连接关闭）。
- `deliver.from` 由 relay 按发送连接的注册身份填写（不可伪造）。
- 目标不在线：回 `error peer_offline`（带 `to`）。
- 限制：单帧 ≤ 64 KB（超出 → `error too_large` 并关闭）；每连接 > 30 帧/秒 → `error rate_limited` 并关闭；90 秒无任何帧 → 服务器关闭连接（客户端 30s 心跳）。
- relay 无持久化、无法解密 `frame.c`。`GET /healthz` → `200 {"ok":true}`。
- 默认监听 `0.0.0.0:21190`。

## 4. LAN 直连传输

- desktop 监听 `0.0.0.0:25810`（被占用则退化为系统分配端口；实际端口写入状态与 QR）。
- `GET /bridge/info` → `200 {"app":"nomifun","v":1,"id":"<device_id>","name":"<桌面名>","pk":"<base64>","version":"<app版本>"}`，响应头 `Access-Control-Allow-Origin: *`（供 H5 探测）。
- `GET /bridge/ws` → WebSocket，双向直接收发 **E2EFrame**（无 relay 包装）；`{"type":"ping"}/{"type":"pong"}` 亦可用。
- 未配对客户端在此连接上只允许发起 `pair_request`（其余内层消息在配对前一律丢弃）。

## 5. 配对载荷（QR / 桥接串）

- QR 文本：`nomifun-bridge:` + base64url(JSON)：

```json
{"v":1,"id":"<device_id>","pk":"<base64>","name":"<桌面名>","lan":{"host":"192.168.1.10","port":25810},"relay":{"url":"wss://...","key":"<server_key>"},"code":"12345678"}
```

- `lan`/`relay` 均可选（至少其一）。
- 桥接串（手动兜底）：同一 JSON **去掉 `code`** 后的 `nomifun-bridge:` + base64url；配对码由用户单独输入。
- 配对码：8 位随机数字，TTL 300s，单次使用，同一时间仅一个有效码；错误码 `pair_invalid_code`。

## 6. RPC 方法（mobile → desktop）

通用错误码：`not_found` | `bad_request` | `upstream_error` | `pair_required`。列表 `limit` ≤ 50。任何 `result` 序列化后 > 16 KB：截断其中文本字段并置 `truncated: true`。

| method | params | result |
|---|---|---|
| `device.info` | `{}` | `{"name","version","platform","agent_types":["..."],"capabilities":["conversations","cron","confirmations"]}` |
| `conversations.list` | `{"cursor"?,"limit"?}` | `{"items":[{"id","name","type","status","is_processing","updated_at"}],"has_more":bool}` |
| `conversations.create` | `{"name"?,"type"?}`（type 缺省为桌面默认 agent 类型） | `{"id","name","type"}` |
| `conversations.send` | `{"conversation_id","content"}` | `{"msg_id","completed","result_ok"?,"result_text"?,"truncated"?}` |
| `conversations.status` | `{"conversation_id"}` | `{"id","name","status","runtime":{"state","is_processing","pending_confirmations","active_turn_id","processing_started_at"}}` |
| `conversations.result` | `{"conversation_id"}` | `{"message_id","text","truncated","created_at"}`（最近一条助手消息） |
| `conversations.cancel` | `{"conversation_id"}` | `{}` |
| `confirmations.list` | `{"conversation_id"}` | `{"items":[<桌面 Confirmation 原样>]}` |
| `confirmations.confirm` | `{"conversation_id","call_id","data"?,"always_allow"?}`（桌面端自行合成 UUIDv7 `msg_id`：真实端点对其做严格校验，而 `confirmations.list` 的条目不含 msg_id，agent 实现忽略该值） | `{}` |
| `cron.list` | `{}` | `{"items":[<CronJobResponse 原样>]}` |
| `cron.create` | `{"name","schedule":<CronScheduleDto>,"message","conversation_id"?,"agent_type"?}` | `<CronJobResponse>` |
| `cron.update` | `{"cron_job_id","patch":<UpdateCronJobRequest>}` | `<CronJobResponse>` |
| `cron.delete` | `{"cron_job_id"}` | `{}` |
| `cron.runNow` | `{"cron_job_id"}` | `{}` |

- desktop 实现方式：对既有 axum Router 做 **in-process oneshot**（携带 `x-nomi-local-trust` 头，以安装所有者身份），复用现有 HTTP 契约（幂等键 = rpc `id`；`conversations.send` 置 `origin:"companion"`）。

## 7. 事件（desktop → mobile，`kind:"event"`）

只推结果类事件，**绝不转发 `message.stream` 过程数据**：

| name | data |
|---|---|
| `task.completed` | `{"conversation_id","turn_id","status","result_text"(≤2048字符),"truncated","ts"}` |
| `conversations.attention` | `{"conversation_id"}`（触发：桌面在本地 `message.stream` 元数据中检测到 `type` 为 `permission`/`acp_permission` 时仅转发会话 ID——不含任何过程内容；`turn.completed` 的 `pending_confirmations>0` 作为兜底。同一会话 5s 内去重。手机端收到后按需调 `confirmations.list`） |
| `cron.executed` | `{"cron_job_id","status","error"?,"ts"}` |

## 8. 互操作测试向量

- Rust 侧生成 `bridge_e2e_vectors.json`：`{desktop_sk,desktop_pk,mobile_sk,mobile_pk,nonce,plaintext,cipher_b64,pair_code,pair_mac_hex,device_id_of_desktop_pk}`（全部 base64/hex 字符串）。
- 同一文件检入 nomifun-tauri（`crates/backend/nomifun-bridge/tests/vectors/`）与 nomifun-mobile（`tests/vectors/`），JS 侧 vitest 必须复现：解密 `cipher_b64` 得 `plaintext`、按 §2.2 复算 `pair_mac_hex`、按 §2 复算 `device_id`。
