# HTTP MCP：受限服务器消息生命周期（未验证）

任务 CAR-03 / CAR-05 / CAR-07。只在 `rf/agent-capability-platform-v2` 本地实施，
没有 commit/push。按用户要求未运行构建、测试、评测、外部 MCP 服务调用或 E2E。
仅做源码阅读和局部格式化；以下不是执行验收结论。
Nomi build：`host8`；Coding build：`host2-coding-loop20`。

## 参考与架构归属

继续阅读本地 Codex 的 `codex-rs/rmcp-client/src/elicitation_client_service.rs` 和
`logging_client_handler.rs`：其服务器请求按方法和已声明支持能力分派，用户交互走独立
处理路径，不混入工具结果。本切片借鉴这个边界，没有移植其 OpenAI 私有方法或宣称版本兼容。

Nomifun 的两个官方 Engine 共用 canonical HTTP owner。Engine 仍只决定循环、规划与
上下文；凭据、远端事务、会话和协议应答由平台持有。新增服务器请求不能扩大 Agent 的
Snapshot/资源授权，也没有改变仅源码接入并重新打包注册 Engine 的政策。

## 已写入的协议行为

- `notifications/initialized` 省略 id，而不是序列化 id:null。
- 区分 SSE 中的请求、通知、响应；只有与数值 request ID 精确同类型匹配的结果才能结束请求。
- ping 返回空对象结果；未声明的 sampling/elicitation/roots/其他方法返回 -32601，
  非对象 params 返回 -32602。固定错误不回显远端参数、凭据或工作区内容。
- 服务器 ID 允许有界非空字符串或整数，拒绝 null/复用 ID；从初始化到工具结束共用
  64 次请求预算。请求 ID 名字空间与客户端独立，不与工具 call ID 混同。
- 应答走原 endpoint、原 canonical credential、原 Session 和原事务 deadline；
  不使用服务器下发 URL，不重连、不重放，也不重置 30 秒默认事务时限。
- 通知与应答 POST 只接受空 202 acknowledgment，不递归处理新 SSE/JSON 请求。
  协议请求只接受 application/json 或 text/event-stream，仍限制响应总字节数。
- SSE 按字节增量完成行，正确吞掉跨 HTTP chunk 的 CRLF；支持单 CR/LF、首行 BOM、
  多行 data、注释；最多 4096 个事件/8MiB。只接受默认/message 数据事件，拒绝 legacy
  endpoint 等数据事件。id/retry 字段不会触发重连或更换 endpoint。
- 响应按 result/error 字段是否存在进行互斥校验，不能以 error:null 绕过。
- 收到 tools/list_changed 中止冻结事务，不静默重载目录；收到当前 requestId 的取消
  通知中止请求。其他有界通知不改变目录、权限或模型上下文。

## 失败与效果

此次没有新增 remote dispatch 队列、并发事务或第二套效果账本。远端主动消息应答仍在
原 pending receipt 覆盖的事务内；应答失败或超时同样进入原 cleanup 路径。
tools/call 已开始后发生协议错误/目录变更/取消，不代表工具没有执行；仍返回未知结果，
保留效果隔离。只有原调用成功且会话清理成功，平台才可结算已有 receipt。

请求在匹配结果事件处结束，不等待可能永不结束的 SSE HTTP EOF。没有独立 GET
订阅流，因此这里只处理当前 POST 响应流内实际收到的通知，不声称目录具备远端原子性。

Nomi 构建摘要补入 owner.rs/owner_stream.rs；Coding 原本已包含。两个 build 均递增，
不让旧 Session 以新代码的恢复假设继续运行。未知 exact build 仍不静默回退。

## 未完成与兼容性影响

完整 sampling、elicitation、roots、resources、stdio/legacy SSE 以及用户介入生命周期
仍未实现；返回“不支持”只保证协议边界，不等于实现这些能力。没有后台服务器订阅流。
MiniApp/Robot/非 function Plugin 效果凭据、安全 checkpoint 恢复、跨启动未知进程证明、
复杂指令作用域与真实 consumer/Provider/跨平台验收仍需后续工作。

旧服务器若返回 204 initialized ACK、缺失 Content-Type 或将数值 ID 改为字符串，现在
会被明确拒绝。现有 owner fixture 的 ACK 与 ID 断言已同步，但没有执行测试。
后续需覆盖服务器 ping/拒绝应答、请求预算、跨 chunk 分帧、通知取消、混淆响应、
应答失败/超时以及远端调用后未知效果隔离；不能引用历史测试证明本切片可用。
