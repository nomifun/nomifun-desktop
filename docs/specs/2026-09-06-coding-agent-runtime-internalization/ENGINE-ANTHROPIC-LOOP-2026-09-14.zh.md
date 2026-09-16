# Anthropic 原生工具/推理循环（slice53，未验证）

开发分支仍为 `rf/agent-capability-platform-v2`。没有构建、测试、真实模型请求、迁移、commit 或 push。
Coding identity 为 `host2-coding-loop53`，Nomi 为 `host35`；新 decoder 和公共 wire budget 纳入构建摘要。

## 本轮源码实现

原通用解析能处理规范化 tool_call 事件，但不会将 Anthropic 的 content_block_start(tool_use)、
input_json_delta、content_block_stop 关联成完整函数调用，也没有把 message_delta.stop_reason
与实际调用核对。现在官方 Anthropic adapter 的生产 attempt 使用独立状态机：

- 校验唯一 message_start、assistant 角色、初始空 content、消息用量及内容块顺序。
  同一时刻仅有一个原生块，索引必须连续；块结束后不能再接受其 delta。
  message_delta 关闭内容阶段，message_stop 必须已有明确 stop_reason 且所有块已关闭。
- tool_use.id/name 与块索引分开保留；参数 JSON 增量按原顺序拼接，在 block_stop 解析对象。
  完整初始 input 与后续 JSON 增量不能同时提供内容。最多 64 个唯一调用、128 个内容块，
  单调用参数最多 256 KiB；tool_use stop reason 必须与实际函数调用一致。
- 标准 text 与 thinking 分片成为统一文本/推理事件；signature_delta 在同一 thinking 块内
  累积，至 block_stop 才发出一个完整 ReasoningSignature，最多 256 KiB。
  签名后不得继续追加 thinking 文本；缺少文本或签名的块拒绝。
  Coding 使用前轮已修正的签名块分隔逻辑，不把下一个 thinking 块追加进已签名块。
- message_start 和 message_delta 的 usage 按累计快照合并，不重复相加；已知计数必须
  是非负整数且不减少。message_stop 才发出唯一 Usage 和 Completed。
  end_turn/stop_sequence、tool_use、max_tokens、refusal 映射明确；pause_turn 不冒充本地工具续接。
- `message/json` 内 type 声明与命名 SSE 两种包装可识别；原生与规范化流不能混用。
  ping 可出现在 message_start 前，但不选择 legacy 模式，不能混入其他载荷。
  显式流内错误沿用共享安全错误分类；重试和路由仍只归 Broker。
- 新 wire_budget 把原 Responses 的无额外序列化分配计数器抽为共享设施：每次原生 attempt
  最多 65,536 帧、累计 16 MiB；Anthropic heartbeat 也计入。实际 Engine 自身预算继续生效。

工具完成事件不是执行授权。Coding 原有完整消息终态、JSON 一致性、规划、用户 steering、
Kernel 授权及 owner 效果记录仍在执行前生效。半截 JSON、未闭合块、错误/EOF 不派发工具，
也不自动重放已经产生的效果。

## 请求侧约束

- Anthropic Messages 要求显式 max_output_tokens，不从猜测的型号默认值生成上限。
- 当前支持 budgeted thinking：预算至少 1024 且严格低于 max_output_tokens；不支持的
  adaptive/effort、显式 concise/detailed summary 控制会报错，不静默吞掉配置。
- budgeted thinking 与 Required/Specific 工具选择冲突时发送前拒绝。
- 开启 PromptCache 策略且存在 system 指令时，将该静态前缀写为 ephemeral 缓存块。
  没有 system 前缀时不伪造缓存内容；不承诺 provider 缓存命中或计费降低。
- 现有 Bedrock/Vertex 请求复用 Anthropic 编码 helper，因此继承这些请求约束；它们的
  专有响应流尚未在本轮接入，不能据此宣称对应协议已完成。

## 仍未完成及证据限制

本轮没有执行验证。已有 recorded/anthropic.json 使用 128 thinking tokens、512 output tokens
和 effort=medium，这不是当前 budgeted thinking 契约可接受的真实请求；该旧规范化夹具
不能作为本轮原生能力证据。本轮未调整或运行测试夹具，后续验证工作需单独处理。

redacted_thinking 需要独立的无损不透明续传契约，目前明确拒绝，不伪造成 Responses 加密块
或普通文字。空文本签名块、citation/server-tool 块、pause_turn、adaptive thinking、专有
Bedrock/Vertex 帧和截断块的部分输出产品呈现仍未完成。已合法闭合的 max_tokens 可投影结束
原因；半截工具 JSON 在块结束时即失败，不拿它请求执行。

这仍是共享模型协议层能力，不是新增 Agent 类型或 Engine。引擎可替换、编译期注册、
Session exact binding、不热挂载及平台工具 owner 边界均不变；整体 CAR 未完成。
