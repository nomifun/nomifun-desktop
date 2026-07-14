# 远程 OpenClaw 接入与 Nomi Agent 控制指南

本文说明如何让 NomiFun 连接一台远程 OpenClaw Gateway，以及如何通过桌面会话或 Nomi Agent 的能力工具控制它。

> 当前远程运行时只正式支持 **OpenClaw Gateway**。ZeroClaw 和通用远程 ACP 尚未提供可用适配器。

## 一、能力边界

当前支持：

- 通过 `ws://` 或 `wss://` 连接远程 OpenClaw Gateway；
- OpenClaw Gateway v4 的连接挑战、设备身份和协议握手；
- 无认证、Bearer Token、密码三种认证方式；
- 创建或恢复 OpenClaw 会话、发送消息、流式接收回复；
- 取消当前回复、清理远端会话上下文；
- 将 OpenClaw 的批准请求映射到 NomiFun 会话确认流程；
- 由 Nomi Agent 通过受控能力创建远程会话、下发任务和查询状态；
- 从桌面会话界面或受信任的本地会话 API 取消任务。

当前不支持：

- 把任意 ACP stdio Agent 直接当作 WebSocket 服务连接；
- 直接远程连接 Hermes CLI；
- ZeroClaw 或其他自定义协议。

## 二、准备 OpenClaw Gateway

在远端主机启动 OpenClaw Gateway，并确认 NomiFun 所在机器能够访问其 WebSocket 地址，例如：

```text
wss://openclaw.example.com/gateway
```

仅限受信任局域网或本机调试时，也可使用：

```text
ws://192.168.1.20:PORT/gateway
```

请在 OpenClaw 侧准备以下认证方式之一：

| 认证类型 | NomiFun 配置值 | 用途 |
|---|---|---|
| 无认证 | `none` | 仅适合本机或隔离网络 |
| Bearer Token | `bearer` | 推荐用于服务令牌 |
| 密码 | `password` | 使用 Gateway 密码认证 |

NomiFun 为每个远程 OpenClaw 配置生成独立的设备身份。首次连接时，Gateway 可能要求批准该设备；请在 OpenClaw 管理端核对并批准，不要批准来源不明的请求。

## 三、在桌面端添加远程 OpenClaw

1. 打开 **设置 → Agent → 远程 Agent**。
2. 点击添加，协议选择 **OpenClaw**。
3. 填写名称和 Gateway WebSocket URL。
4. 选择认证类型，并按需填写 Token 或密码。
5. 如使用自签名 TLS，阅读下一节后再启用“允许不安全证书”。
6. 点击“测试连接”。该操作会执行真实的 OpenClaw 协议与认证握手，而不只是检查端口是否可达。
7. 保存配置。若界面提示等待设备批准，请在 OpenClaw 侧批准后重试握手。
8. 回到新会话页面，选择该远程 Agent 并发送消息。

保存的认证凭据会加密存储；详情接口和能力工具只返回掩码，不应向模型或日志暴露明文凭据。

## 四、自签名 TLS

`wss://` 默认验证服务器证书、证书链和主机名。生产环境应使用受信任 CA 签发的证书。

“允许不安全证书”仅用于自签名证书或封闭测试环境。启用后，NomiFun 会跳过该连接的 TLS 证书验证，但应用层 OpenClaw 认证仍会执行。

风险包括：

- 中间人可冒充 Gateway；
- Token、密码及会话内容可能被截获；
- 设备身份可能与错误的服务端建立信任。

建议优先采用以下方案：

1. 为 Gateway 配置受信任证书；
2. 使用 WireGuard、Tailscale、SSH 隧道等受控网络；
3. 通过反向代理终止 TLS，并只向可信来源开放；
4. 只有无法配置证书的短期测试环境才启用该选项。

`allow_insecure` 不表示允许任意明文 URL。远程 OpenClaw 地址仍必须是 `ws://` 或 `wss://`；`ws://` 只应出现在回环地址、隔离局域网或加密隧道内。

## 五、网络安全建议

远程 Agent 可以在另一台机器上执行模型和工具，其权限可能接近远程代码执行。至少应做到：

- 不把 Gateway 端口直接暴露到公网；
- 使用防火墙或安全组限制 NomiFun 来源地址；
- 为每个客户端使用独立 Token，定期轮换并及时吊销；
- 不在聊天、截图、日志或能力调用参数中粘贴凭据；
- 对远端文件、Shell、浏览器等工具使用最小权限；
- 对删除、执行命令和外部写入保留确认机制；
- 监控异常会话、重复握手和认证失败；
- 删除远程 Agent 配置时，同时在 OpenClaw 侧撤销对应设备和凭据。

## 六、通过 Nomi Agent 控制远程 OpenClaw

Nomi Agent 不直接读取远程 Agent 的密钥。它通过 NomiFun Gateway 的受控能力引用已保存配置的 `remote_agent_id`，底层凭据只在后端连接阶段解密。

### 1. 查找远程 Agent

调用：

```json
{
  "tool": "nomi_remote_agent_list",
  "arguments": {}
}
```

从结果中取得目标 OpenClaw 的数字 `id`。必要时可调用：

```json
{
  "tool": "nomi_remote_agent_handshake",
  "arguments": { "id": 12 }
}
```

远程 Agent 配置能力使用独立的 `remote` capability domain。受信任的桌面
Nomi 会话可使用 `list/get/create/update/delete/test/handshake`；通过外部
`/mcp`、`/mcp-agent` 或 `/v1` 进入的 Remote surface 只能读取
`list/get`，不能创建、修改、删除配置，也不能主动测试或握手，以免外部
调用者把 NomiFun 变成内网探测器。IM Channel surface 同样不能调用这些
配置和主动连接能力。

### 2. 创建远程会话

调用通用会话能力：

```json
{
  "tool": "nomi_create_conversation",
  "arguments": {
    "name": "远程 OpenClaw 任务",
    "agent_type": "remote",
    "remote_agent_id": 12
  }
}
```

记录返回的 `conversation_id`。会话配置只保存远程 Agent 的数字引用，不复制认证凭据。

### 3. 下发任务

```json
{
  "tool": "nomi_send_to_conversation",
  "arguments": {
    "conversation_id": 123,
    "content": "检查远端项目的测试失败并给出修复建议。"
  }
}
```

发送是异步的。不要在目标会话仍处于处理中时重复发送；遇到 busy 错误时先查询状态。

### 4. 查询结果

```json
{
  "tool": "nomi_conversation_status",
  "arguments": {
    "conversation_id": 123
  }
}
```

该能力返回运行摘要及最近的会话消息，可用于判断是否仍在处理并读取最终回复。

### 5. 取消或清理

当前 Gateway 工具集尚未单独暴露 `nomi_cancel_conversation`。取消正在运行的 turn 时，可在桌面会话界面点击停止；受信任的本地客户端也可调用：

```text
POST /api/conversations/{conversation_id}/cancel
```

不再需要时，使用 `nomi_delete_conversation` 删除 NomiFun 中的会话。删除远程 Agent 配置是独立的破坏性操作，不应仅因结束一次任务而执行。

如果 Nomi Agent 是通过 `/mcp`、`/mcp-agent` 或 `/v1` 被外部控制，仍应使用伙伴访问令牌和 Remote surface 权限模型。详细接入方法参见：

- `remote-capability-api.zh.md`
- `remote-capability-api-examples.zh.md`

## 七、Hermes 支持说明

Hermes 当前作为**本地 ACP CLI Agent**受支持，默认启动方式是：

```text
hermes acp
```

使用前需要：

1. 在运行 NomiFun 的同一台机器安装 Hermes；
2. 确保 `hermes` 命令位于 NomiFun 进程的 `PATH`；
3. 在 Agent 管理中启用 Hermes；
4. 以 ACP 会话方式使用。

Hermes ACP 是基于本地进程标准输入/输出的协议，不能把 `hermes acp` 的地址直接填入远程 OpenClaw URL。

Hermes 上游还提供两种更适合远程接入的接口：

- TUI Gateway：JSON-RPC over WebSocket，通常挂载在 `/api/ws`，包含
  `session.create/resume/history`、`prompt.submit`、`session.interrupt/steer`、
  `approval.respond`、澄清/提权/秘密输入事件等完整交互；
- OpenAI-compatible API Server：HTTP + SSE，适合只需要对话、运行状态和审批的客户端。

因此远程 Hermes 的推荐设计不是“ACP over WebSocket”，而是新增独立
`HermesJsonRpcManager`：连接 TUI Gateway `/api/ws`，把 Hermes 的 JSON-RPC
方法和事件映射到 NomiFun 的会话、流事件、取消及确认模型。若需要兼容只部署
`hermes acp` 的主机，也可以在远端部署 stdio Bridge，例如：

```text
NomiFun
  → 经过认证的 HTTPS / WebSocket / MCP / A2A 连接
  → Hermes Bridge
  → 本地 stdio
  → hermes acp
```

无论采用原生 TUI Gateway 还是 stdio Bridge，适配器都必须负责会话生命周期、
协议帧转换、流式事件、取消、确认请求、会话恢复、认证、TLS 和审计。在该适配器
实现并经过端到端测试前，远程 Agent 页面不会把 Hermes 或通用 ACP 宣称为可用协议。

## 八、故障排查

### 连接立即失败

- 检查 URL 是否使用 `ws://` 或 `wss://`；
- 检查端口、防火墙、DNS 和反向代理的 WebSocket Upgrade；
- 确认 Gateway 服务正在监听外部接口，而不只是 `127.0.0.1`。

### 认证失败

- 确认选择的是 Bearer Token 还是密码；
- 更新配置时，不要把界面显示的掩码当作新凭据提交；
- 在 OpenClaw 侧确认 Token 未过期、密码未变更。

### 等待批准

- 在 OpenClaw 侧查找新的设备配对请求；
- 核对设备信息后批准；
- 若请求异常，拒绝它并轮换凭据。

### 自签名证书失败

- 推荐把自签 CA 加入系统信任或换成受信任证书；
- 仅在确认网络隔离且服务端身份可信时启用“允许不安全证书”；
- 检查反向代理证书的主机名是否与 URL 一致。

### Hermes 不显示

在启动 NomiFun 的同一环境执行：

```powershell
Get-Command hermes
```

若找不到命令，请安装 Hermes、修正 `PATH`，然后重启 NomiFun 让 Agent Registry 重新探测。
