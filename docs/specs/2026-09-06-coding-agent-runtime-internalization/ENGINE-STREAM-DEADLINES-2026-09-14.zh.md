# 共享模型流的分阶段超时（slice81）

2026-09-14，rf/agent-capability-platform-v2 本地工作树。
Coding host2-coding-loop81 / Nomi host53。源码实现，未测试、构建、启动服务、调用模型、迁移、commit 或 push。

## 原问题与参考

原 SingleAttemptRequest.timeout 通过 reqwest RequestBuilder.timeout 应用于建连至整个
响应体结束；生产为 120 秒，所以正常持续输出的长任务也会被总时限截断。
读取固定 Codex 基线 6af345407d9c2a568da9d01b6c4b81a9e61495c0 的
codex-rs/codex-api/src/sse/responses.rs：在 eventsource 之上对 stream.next() 设置
idle timeout。本轮借鉴“完整事件等待”的分层，不复制其实现，不新增 Engine 私有传输。

## 源码变更

- timeout 现在约束凭据解析、签名、连接/上传和响应头阶段；新增 idle_timeout，
  生产两项均为 120 秒，合法范围分别为大于零且不超过 24 小时。
  不再给单次 HTTP builder 设置总响应体 timeout。
- 新 chat_deadline.rs 在共享 SingleAttemptStream 外层等待完整帧，覆盖 SSE、AWS
  event-stream 和完整 JSON 响应。字节滴流、未完成事件和 SSE 注释不重置计时。
  合法完整协议事件（包括 Provider 的 JSON ping）可以推进计时；这不是任务进展证明。
- 定时器在消费者请求下一帧时启动，收到一帧后撤销；消费者处理上一帧的背压时间
  不误算为 Provider 停滞。正在等待的 future 被放下再轮询不会重置已有计时器。
- 超时为 InvokeErrorKind::Timeout；EOF、解析/传输错误、超时和 Bedrock exception
  释放源及解析缓冲，终止后不反复报错、不创建后台读取任务、不合成模型 Completed。
  完成仍由 Broker 验证，已有 semantic-output-committed 禁止重放屏障保留。
- 非成功 HTTP 响应的有界诊断体也限时读取。体超时保留已知 HTTP 分类和 Retry-After，
  不将 HTTP 400 改为可 failover 的 Timeout，也不从不完整正文推断上下文超限。
- JSON/AWS 连续 ready 源每轮读取最多 256 步或达到 64 KiB 后让出；共享错误体读取
  每 256 块让出。AWS 在追加前限制单块 16 MiB、累计至多一个未完成帧加一块（32 MiB）。
  这些是协作调度及分配边界，不是硬实时抢占或性能测量。
- 原有测试构造体补充 idle_timeout 字段，仅适配接口，未新增或执行测试断言。
  官方 build ID 推进，摘要纳入新模块；不迁移旧 Session 的 exact build。

## 客户端与取消边界

保留注入的 reqwest::Client，不重建客户端、不绕过代理/TLS 策略。
生产 EngineSessionHost 已用 Client::new()，其默认没有总请求超时。
本地 reqwest 0.12.28 的 RequestConfig::fetch 对空 request timeout 会回退客户端值，
所以不能声称 timeout_mut=None 可禁用调用者配置的总时限。公共构造器说明了该条件：
自定义客户端的总超时/read timeout 仍是额外限制，不使用超大 Duration 假装无限。
取消通过现有 Broker 丢弃调用/流 future 释放传输；没有新增取消 owner、路由重试或凭据轮换。
同步解析及自定义 resolver/authenticator 仍须遵守协作执行约定，计时器不是线程抢占器。

## 未验证与剩余事项

没有运行长流、无首帧、注释滴流、分片 AWS/JSON、错误体卡住、背压、取消或真实 Provider
回归；没有声称行为已通过。后续应覆盖总时长超过 120 秒但帧间等待正常的成功路径，以及
语义输出前后的超时重试差异。既有三平台/实际模型/生成器一致性证据仍缺失。
Git 网络凭据 owner、未知外部效果恢复、MCP 服务端主动生命周期、Vertex 独立生产配置等
其他合同边界未由本轮改变，整体 CAR 不标为完成。
