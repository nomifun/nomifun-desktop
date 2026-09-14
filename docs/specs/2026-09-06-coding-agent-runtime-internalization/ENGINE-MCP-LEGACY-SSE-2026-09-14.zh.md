# 共享 MCP legacy SSE 接入（2026-09-14，未验证）

任务：CAR-03 / CAR-05 / CAR-07。只在本地 `rf/agent-capability-platform-v2`
修改源码；未 commit/push，未构建、测试、运行连接检测、调用模型/外部服务或执行迁移。

## 本次实现

- canonical MCP owner 支持明确配置的 `sse`，与 Streamable HTTP 共用冻结工具、
  参数及资源准入、完整分页目录比对、服务器消息处理和 durable effect receipt。
  产品目录物化与运行时绑定均接受 `http` / `sse`；不是另一套 Engine，也不新增聊天切换入口。
- SSE 使用 `2024-11-05` 握手；HTTP 保持 `2025-03-26`，不自动降级或转换传输方式。
  GET 必须返回 200、`text/event-stream` 和首个 `endpoint` 事件；后续 endpoint 变化拒绝。
- 相对/绝对消息 URL 必须与配置 URL 同源，不允许 userinfo、fragment 或跨协议/主机/端口
  路由。HTTP 明文仅允许 localhost 或 IP 字面量，其他域名需 HTTPS。生产 HTTP client
  禁止重定向；注入 client 的调用方也必须禁止重定向，事后 URL 核对不能阻止已发生的泄漏。
- POST 202 只表示消息接收，最多消费 16KiB 文本，不当工具结果。请求同时等待 POST 回执
  和 SSE 关联响应，避免大响应堵住流后迟迟无法发送 POST 回执。请求、分页与工具调用共用
  owner 截止时间；不重连、不使用 Last-Event-ID，不自动重放可能已生效的调用。
- SSE 全事务最多 8MiB、4096 个事件边界；保留同 chunk 内的未消费字节及 CR/LF、UTF-8
  分片状态。共享目录最多 32 页、1024 工具和 8MiB，检查全部页面、重复工具名和循环游标。
- 服务端 ping 与未授权方法拒绝沿用同一端点、凭据及消息预算。目录变更或当前请求取消
  中止事务；tools/call 派发后失败仍是未知效果，不变成可安全重试。
- 设置页 SSE 目录发现复用相同握手、解析、关联和完整目录枚举，删除旧的不受限后台读取器
  及对应废弃解析辅助代码。超时直接释放调用拥有的响应流，不留下独立 reader task。
  保留有界 401 认证提示；目录发现结果不构成 Agent 调用授权。
- 静态 header 禁止覆盖 Host、消息长度/传输编码、连接控制、协议 Session 和恢复标记；
  owner 的 Authorization 标记为敏感值。配置发现同样禁止路由/传输控制覆盖。

## 架构与清理边界

Nomi、Coding 和编译期社区 Engine 继续经同一 Session/Kernel 工具宿主消费 MCP。
新增 MCP 传输不是运行时安装 Engine，Engine 注册仍只在源码集成和应用打包时完成。

legacy SSE 没有标准 Session DELETE：结束只释放本地响应流，不声明远端服务退出、
后台活动静止或工具效果回滚。工具失败沿用宿主保守隔离，不能通过换传输绕过未知效果。
请求中的服务端消息回复仍在外层超时内顺序发送，不支持任意双向客户端能力/持续订阅。

Coding 构建后缀为 `host2-coding-loop35`，Nomi 为 `host19`；两个源码摘要均纳入
新 SSE transport 与目录发现源码。既有 exact Session 不自动换构建。

## 借鉴与剩余工作

参考本地 Codex `codex-rs/rmcp-client/src/http_client_redirect.rs` 的凭据/正文同源边界
与明文域名重绑定风险，以及 `streamable_http_retry.rs` 的共享截止时间。此实现不增加其
重定向或重试功能，不声称完整复刻 Codex 的 MCP 栈。

stdio 进程所有权接入、resources/订阅、经授权的 sampling/elicitation/roots 等生命周期
仍未实现；旧 HTTP/stdio 设置页目录发现尚未全部迁移到共享 owner。跨启动进程证明、
人工隔离处置和其他 CAR 剩余项目不因 SSE 接入而完成。新增代码未编译/执行验证，
不能据此宣称生产可用或多 Engine 总体完成。
