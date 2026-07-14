# 外部 Knowledge MCP 本地 Broker 架构

外部 Claude、Gemini、Codex 统一注册一条稳定命令：

```text
nomicore mcp-knowledge-stdio
```

配置文件只保存可执行文件路径和子命令，不保存 HTTP 端口、访问令牌、续租凭据、用户、知识库 ID、工具列表或写权限。历史 endpoint/token beacon 不属于该架构，也没有兼容读取路径。

## 两种启动入口

- 平台管理的 Agent / Terminal：父进程设置 `NOMI_KB_MCP_CAPABILITY`。stdio bridge 必须使用这份精确的会话 capability；环境变量存在但无效时直接失败，禁止回退到外部 Broker。
- 外部 CLI：环境变量不存在时连接 OS 本地 Broker，只发送协议版本和当前工作目录。主进程规范化真实目录并从数据库解析授权。

两种入口最终都使用同一个 `KnowledgeMcpConfig`、同一个短期 access/renewal 合约和同一个 `KnowledgeMcpServer` 工具执行边界。

## OS 身份边界

### Unix

- runtime directory 归当前 euid 所有，权限固定为 `0700`；
- Unix socket 归当前 euid 所有，权限固定为 `0600`；
- 服务端和客户端分别通过 Linux `SO_PEERCRED` 或 BSD/macOS `getpeereid` 校验对端 uid；
- symlink、非 socket 节点、外部 owner 或宽松权限均 fail closed。

### Windows

- 使用 `tokio` 原生 named pipe，不存在 TCP 降级；
- pipe 设置 protected DACL，唯一 ACE 是当前用户 SID 的访问权限，并启用 `PIPE_REJECT_REMOTE_CLIENTS`；
- 服务端通过 `GetNamedPipeClientProcessId` 读取客户端进程，客户端通过 `GetNamedPipeServerProcessId` 读取服务端进程；两端打开进程 token、读取 `TokenUser`，用 `EqualSid` 与当前用户 SID 比较。

OS 同用户校验是本地安装级信任边界；应用内安装 owner 是签发 capability 的业务身份。两者必须同时成立。

## 服务端解析与最小权限

外部请求使用 `deny_unknown_fields`，只能包含：

```json
{"version":1,"cwd":"/existing/workspace"}
```

服务端负责：

1. 对存在的目录执行 canonicalize，symlink 路径收敛到同一个真实 workspace；
2. 使用数据库中的 canonical installation owner，生成不可由客户端选择的 `ExternalProcess` session id；
3. 解析 workspace binding、知识库 ID 和 writeback policy；
4. bound workspace 仅签发绑定的知识库；unbound workspace 可 search/read 当前注册的知识库，但永不签发 `knowledge_write`；
5. 通过 `issue_for_external_process` 签发短期 access 和不可扩权的 renewal proof。

客户端不能提交或覆盖 user、session、kb IDs、tools、write policy。

## 生命周期

主进程在 Broker 连接任务中持有 `LoopbackCapabilityLease`。stdio bridge 在收到 bootstrap 后继续持有控制连接：

- bridge 正常退出、崩溃或被杀：控制连接 EOF，主进程立即 revoke；
- 连接发送额外数据或发生错误：按失败处理并 revoke；
- backend 停止或重启：Broker 在停止 HTTP renewal endpoint 前同步 revoke 全部 lease；
- 同时活跃的外部进程上限为 32。

旧进程不能在新 backend 实例中续租。

## 配置写入与 API 边界

Claude / Gemini JSON 采用结构化 merge，保留未知 top-level 字段和已有 MCP server；已有配置若是 malformed JSON 或 `mcpServers` 类型错误，操作失败并保持原文件不变。写入使用同目录临时文件、flush/fsync 后替换；Windows 使用 `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`。

Codex 配置由 `codex mcp add/remove` 自身维护，不直接解析或重写其 TOML。

注册和移除属于主机控制面写操作，HTTP route 同时要求：

- 已认证 identity 等于 installation owner；
- 请求带有桌面进程每次启动生成的 local-trust proof。

远程登录即使使用 owner 账户，也不能写主机配置或执行 `codex mcp`。模板和状态查询是 owner-only 的只读接口。
