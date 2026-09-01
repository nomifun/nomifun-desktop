# Canonical Remote API

NomiFun 只提供一个安装 owner 级 Remote 入口，并同时投影为 MCP 与 REST。Remote
只是传输层，不是 Agent 类型或能力 profile：

1. 本地 owner 创建 `RemoteBinding`，冻结 exact Agent Preset revision、resolved
   Snapshot 与 typed resources。
2. `open(binding_id)` 创建 durable `AgentSession`，返回 UUIDv7
   `agent_session_id`。
3. `turn`、`observe`、`cancel` 始终显式携带该 ID。

安装令牌只回答“调用者是谁”，不会选择伙伴、模型、profile、domain、workspace
或最近 Session。

可复制命令见 [Canonical Remote 对接示例](./remote-capability-api-examples.zh.md)。

## 安全模型

安装令牌是高权限凭据：

- 只交给明确授权的客户端；
- 优先使用 loopback、VPN 或可信私网；
- 网络暴露必须前置 TLS、防火墙与限流；
- 令牌失控后立即轮换或吊销；
- 不要把令牌写进 URL、日志、截图或问题报告。

Remote 使用 FullAuto，不存在 `confirm: true`、`needs_confirmation`、token scope
或 profile/domain selector。实际权限来自已认证 owner 与 AgentSession 冻结的
immutable Snapshot。

## 端点

| 端点 | 契约 |
| --- | --- |
| `/mcp` | Streamable-HTTP MCP，只公开 `open`、`turn`、`observe`、`cancel`。 |
| `POST /api/remote/open` | 从 `RemoteBinding` 打开 Session。 |
| `POST /api/remote/turn` | 在显式 Session 上启动一轮。 |
| `GET /api/remote/observe` | 从 exclusive cursor 之后读取 Event 与消息投影。 |
| `POST /api/remote/cancel` | 取消显式 Session 的活动 Turn。 |
| `/api/remote-bindings` | owner 的本地 Binding 管理 API。 |
| `/api/webui/access-token` | 创建/轮换、查询或吊销安装令牌。 |

Remote MCP/REST 每个请求都必须携带：

```http
Authorization: Bearer <installation-access-token>
```

已删除的 `/mcp-agent`、`/v1`、generic tool discovery/call，以及
`profile`/`domains` query 均不是兼容别名，会直接 fail-closed。

## 创建 RemoteBinding

先在 Agent Settings 创建并保存 Agent Preset，解析 exact revision，然后从本地
管理界面创建 RemoteBinding。Binding 只有：

- `remote_binding_id`
- `owner_user_id`
- `name`
- canonical `agent_binding`

更新 Binding 只影响之后打开的 Session。删除 Binding 只阻止新建，不会改写或
取消既有 Session。

## 安装令牌

明文只在签发时显示一次，SQLite 只保存 SHA-256 verifier。

```bash
# 桌面可信本地上下文，或 nomifun-web 中已登录的 installation owner。
curl -X POST http://127.0.0.1:<port>/api/webui/access-token

curl http://127.0.0.1:<port>/api/webui/access-token

curl -X DELETE http://127.0.0.1:<port>/api/webui/access-token
```

Headless host 可在启动时播种同一安装令牌：

```bash
NOMIFUN_ACCESS_TOKEN="$(openssl rand -hex 32)" \
  nomifun-web --host 127.0.0.1 --port 8787
```

后续启动若更换环境变量值，会轮换数据库中的 verifier。

## MCP 客户端配置

```json
{
  "mcpServers": {
    "nomifun": {
      "type": "streamable-http",
      "url": "http://127.0.0.1:25808/mcp",
      "headers": {
        "Authorization": "Bearer <installation-access-token>"
      }
    }
  }
}
```

`tools/list` 必须精确返回：

```text
open
turn
observe
cancel
```

MCP transport session ID 只负责连接生命周期，不能保存或复用为产品
AgentSession identity。

## Open 生命周期

`open` 会先提交 Session，再跨 Runtime 进程边界，因此第一次成功响应可能是：

```json
{"open_state":{"state":"opening"}}
```

继续用返回的 cursor 调用 `observe`。普通条件下 Session 会收敛到 `ready` 或
`open_failed`：调度上限 5 秒、完整 admission 上限 35 秒、失败事实持久化上限
10 秒。

若写入失败事实时 durable storage 不可用，错误会保留 `agent_session_id`、cursor
和 `recovery: "host_restart_reconcile"`。不要另建 Session 或盲目重复启动
sidecar；应保留该 ID，并先恢复数据库/Host。

## Canonical CLI

`nomicore` 直接映射四个 REST 操作：

```bash
export NOMIFUN_URL=http://127.0.0.1:25808
export NOMIFUN_ACCESS_TOKEN=<installation-access-token>

nomicore remote open <binding_id> \
  --initial-input '{"text":"你好"}' \
  --idempotency-key open-1

nomicore remote observe <agent_session_id> --after-seq 0 --limit 100

nomicore remote turn <agent_session_id> '{"text":"继续"}' \
  --idempotency-key turn-1

nomicore remote cancel <agent_session_id> \
  --idempotency-key cancel-1
```

未指定 idempotency key 时，CLI 会打印并使用一个新 key。网络结果不确定时应复用
打印出的 key。

## 常见错误

| Code | 含义 |
| --- | --- |
| `REMOTE_AUTH_REQUIRED` | 令牌缺失、无效或已吊销。 |
| `REMOTE_INVALID_REQUEST` | 未声明字段/query 或输入格式错误。 |
| `REMOTE_BINDING_NOT_FOUND` | 当前 owner 没有该 Binding。 |
| `REMOTE_SESSION_NOT_FOUND` | Session 不存在、属于其他 owner 或不是 Remote Session。 |
| `REMOTE_SESSION_OPENING` | Runtime admission 尚未到达 durable 终态。 |
| `REMOTE_OPEN_FAILED` | Runtime admission 失败；应读取 Session Event。 |
| `REMOTE_SESSION_BUSY` | Turn/cancel 与当前 Session 状态冲突。 |
| `SNAPSHOT_EXECUTOR_UNAVAILABLE` | 冻结 Snapshot 当前无法执行。 |

## 相关文档

- [Canonical Remote 对接示例](./remote-capability-api-examples.zh.md)
- [WebUI 远程访问](./webui-remote-access.zh.md)
- [Web 服务部署](./web-server-deployment.zh.md)
- [Computer Use 与 Browser Use](./computer-browser-use.zh.md)
