# Coding 上下文超限后的有界压缩恢复（slice49，未验证）

所有源码改动位于本地 `rf/agent-capability-platform-v2`。未运行构建、测试、
服务请求或迁移，未 commit/push；本文不是通过验收或整体 Engine 完成声明。

## 缺口与参考

此前 Broker 已有 `ChatModelErrorCode::PromptTooLong`，但生产
`invoke_error_to_chat_error` 没有相应映射。服务端明确拒绝上下文长度会变成一般参数
错误或 ProviderUnavailable；Coding 只有基于预估预算的主动压缩，没有错误后的恢复。
因此模型限额配置不准确、估算偏差时，即使历史可以压缩，任务仍直接结束。

参考本地 Codex：

- `codex-rs/core/src/session/turn.rs` 的 ContextWindowExceeded 分支：在特定
  review-budget 场景中压缩后续接，用状态位限制恢复次数，避免无效压缩循环。
- `codex-rs/core/src/compact.rs` 的超限处理及近期输入保留思路。
- `codex-rs/core/src/compact_remote_v2.rs` 的输入分组和保留策略。

本次采用“明确超限 → 有界压缩 → 新请求”的设计，不照搬 Codex 特定 review 状态、
自动换模型或删除最老记录重试的实现。原有接受输入、任务状态、路径指令和待交付
图片保护继续保留；最近普通文本工具结果仍可能进入派生摘要，不声称逐字保留。

## 平台：提供明确的错误事实

`nomifun-model-invoke` 的有界错误体读取返回诊断片段和独立的上下文拒绝标记：

- 仅在 64 KiB 上限内完整读到 EOF 后解析 JSON；截断、读取失败、无效 JSON 均不推断。
- 只接受 HTTP 400/413/422，且 `error.code` 或 `error.type` 精确为
  `context_length_exceeded` / `prompt_too_long`。
- 不搜索自然语言 message，不把一般 413、invalid_request_error、配额不足或
  错误信息内引用的代码当成超限。
- 分类先于原有 500 字符诊断裁剪；原有凭据脱敏保留，标记不携带原始响应内容。
- 旧调用者仍看到原来的 HTTP 错误分类；Broker 通过只读访问器投影为
  `PromptTooLong + RetryDirective::Never`，不对相同输入进行传输重试或失败转移。

没有增加 Provider 凭据入口。所有共享 Broker Engine 都能得到同样的错误事实，
是否压缩和怎样压缩仍由各自 Engine 决定。自然语言错误、其他 Provider 的专有结构
以及 HTTP 200 流内错误尚未新增分类；它们维持原有失败路径。

## Coding：一次性的改变上下文续接

生产循环同时处理打开流时的错误，以及流在任何语义输出之前返回的错误：

1. 仅接受类型化 PromptTooLong，Broker 的 semantic_output_committed 必须为 false；
   Engine 本身也记录是否已经观察到任意语义事件。文本、思考签名、工具参数增量、
   ProviderRoundId、usage 等出现后，不允许这条恢复路径。
2. 同一回合最多一次；剩余模型步数不足时直接保留原错误，不发起压缩请求。
3. 记录 `ContextLimitRecoveryStarted { rejected_step }`。该事件只表示进入恢复策略，
   不证明恢复完成。丢弃失败流并清除 provider continuation parent。
4. 压缩目标锚定原被拒请求：输入估算量必须低于它的 75%，且不高于原模型安全预算。
   下个边界新收到的 steering 或动态观察不能抬高这个目标。
5. 接受输入、固定指令、任务/计划状态、工具表与待交付图片仍是必保项；如果这些项
   已经超出新目标，在发送摘要请求前报错，不能为了恢复而丢掉约束。
6. 摘要与最终替换也检查缩减后的预算，沿用每回合 32 次摘要调用上限、无工具摘要、
   取消和先持久化再替换的约束。摘要失败不递归恢复。
7. 成功后回到正常边界，以新模型 operation ID 继续；保留收窄预算。没有执行、重放
   或自动重试任何工具，没有清除未知效果凭据，没有切换 Engine/Agent/冻结路由。

这是估算值的强制缩减，不是精确 tokenizer 或服务端接受保证；再次超限仍结束回合。
本轮总模型步数不会因恢复重置，工作区/资源 owner 权限也不受影响。

## 版本及边界

Coding `host2-coding-loop49`；共享调用层语义变化对应 Nomi `host32`。
两种官方构建摘要加入相关生产 Broker host 和 model-invoke 源码。
Nomi 不复用 Coding 的恢复策略，社区 Engine 也不被强制绑定该策略。

本轮没有运行验证，也没有新增实际模型或恢复执行证据。仍需后续覆盖分类负例、
打开流/流中拒绝、已产生语义输出、重复拒绝、最小必保上下文超限、取消、步数上限、
新 operation 准入及工具不重放。整体其他生命周期/工作区恢复缺口仍在 TASK-MANIFEST 中。
