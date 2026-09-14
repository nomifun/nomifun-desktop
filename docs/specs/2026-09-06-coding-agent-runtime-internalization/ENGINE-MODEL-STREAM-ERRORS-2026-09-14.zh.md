# 模型流内错误分类与每次请求的解码隔离（slice51，未验证）

本地 `rf/agent-capability-platform-v2`，未 commit/push。
仅源码实现与局部格式化，未运行构建、测试、模型调用、服务或迁移。

## 现有缺口和 Codex 参考

slice49 接通 HTTP 非成功响应的明确上下文超限，但 Broker 的流解码仍把所有命名
`error` 当作 ProviderUnavailable，Responses `response.failed` 没有专门分支。
因此 HTTP 200 后以流内错误返回的上下文拒绝无法进入 Coding 的一次性压缩恢复。

参考本地 Codex `codex-rs/codex-api/src/sse/responses.rs`：`response.failed` 分支
读取结构化错误，并以精确 `context_length_exceeded` 代码识别上下文错误，同时区分
配额、限流及其他失败。未照搬自然语言重试等待时间解析或 Provider 诊断原文输出。

接线时还发现官方 adapter 的 OpenAI Chat/Gemini 解码状态保存在共享实例中，异常退出
没有终止帧时可能残留；后续无 ID 帧可能被关联到旧请求。这个问题直接影响有界恢复
和并发 Session 的正确性，因此同一切片完成生产请求级的状态隔离。

## 错误分类

新增 Broker 内部 `provider_errors.rs`，在普通文本/工具帧解码之前识别：

- 命名 `error` / `*.error` 事件；
- `message` / `json` 帧根部明确的 error envelope，不搜索嵌套工具参数或输出；
- OpenAI Responses 的 `response.failed`（含 generic SSE message 中的 type 声明），
  从 `response.error` 读取错误，核对可选 status 为 failed。

代码/type/status 按明确机器标识分类，具体代码优先于通用 type：

| 明确错误类别 | Broker 结果 | 传输策略 |
|---|---|---|
| context_length_exceeded / prompt_too_long | PromptTooLong | Never；Engine 可改变上下文后另发请求 |
| rate_limit_exceeded / rate_limit_error | RateLimited | 既有 Broker Failover 策略和次数上限 |
| overloaded_error / server_error / internal_server_error | ProviderUnavailable | 既有 Broker Failover 策略和次数上限 |
| 认证/权限明确拒绝 | AuthenticationFailed | Never |
| 参数/请求明确拒绝 | InvalidRequest | Never |
| 配额/使用资格拒绝 | ProviderUnavailable | Never |
| 内容策略拒绝 | UnsupportedFeature | Never |

未知的既有命名 error 保留原有 Failover 语义，但只返回固定诊断；新识别的未知
failed-response/JSON error 默认 Never。不会根据自然语言 message 推断上下文超限、
付费资格或等待时间，不将服务端原始诊断复制进 canonical 错误。

错误 envelope 的关键字段类型不正确，或同时出现非空 output/choices/candidates/
delta/text/content/tool_calls/usage 等输出字段时，返回 ProtocolViolation/Never。
这些混合帧不被当作“尚未输出的超限”而丢弃输出后重发；响应中非空 usage 也不被
当成零成本失败。未知 Provider 特有字段/错误仍需后续明确适配，不声称覆盖所有协议。

## Engine 与 Broker 的边界

Broker 继续在已经交付任意语义事件后强制 Never，并标记 semantic_output_committed。
Coding 的自身观察标记与 Broker 标记都必须允许，且回合尚有步数、尚未使用恢复机会，
才可以按 slice49 强制压缩并以新 operation 发起请求。

本轮不在 Engine 内实现传输重试，不更改冻结路由，不增加第二个凭据 owner，
不放宽工具准入或自动重放工具。Nomi/社区 Engine 同样可见平台的错误事实，但不被
强制使用 Coding 的上下文策略。

## 解码状态生命周期

新增 `ChatFrameDecoder` 和 `ChatProtocolAdapter::new_frame_decoder()`：

- 六种官方 adapter 都为每个 Broker transport attempt 返回全新 decoder。
- OpenAI Chat/Gemini 的响应、工具参数拼接、usage 和终止状态只存在于该 decoder。
- 成功、流错误、取消、接收端关闭或 EOF 都随该 attempt 的返回释放解码状态。
- 一个请求失败后，后续压缩请求、同路由重试或其他 Session 不再继承其匿名流状态。
- 兼容保留直接 `decode_frame` API。自定义旧 adapter 默认 None，Broker 仍可调用
  旧接口；自定义有状态 adapter 必须实现新工厂或自行提供等价隔离，不能据此宣称
  所有第三方旧实现自动获得隔离。

这不是新 Session 存储、Engine 动态加载或 provider 协议扩展的运行时挂载入口。

## 状态

Coding `host2-coding-loop51`、Nomi `host33`；两种构建摘要纳入 Broker adapter、
broker 和新分类器源码。没有真实流执行证据；待后续验证明确/未知/混合错误、
输出前后恢复边界、取消清理、失败后匿名帧、并发 Session 及旧 adapter 兼容。
自然语言超限、更多 Provider 专有 envelope 和 Bedrock header-only exception 分类
仍未补齐，整体 Engine 目标仍进行中。
