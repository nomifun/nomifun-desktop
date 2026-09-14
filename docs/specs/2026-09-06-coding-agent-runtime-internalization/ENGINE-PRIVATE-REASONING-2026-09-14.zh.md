# 活跃工具循环的协议绑定推理续传（slice55，未验证）

工作仅在 `rf/agent-capability-platform-v2` 本地进行，没有构建、测试、模型请求、
服务、迁移、commit 或 push。Coding 身份为 `host2-coding-loop55`，Nomi 为 `host37`。

## 缺口与参考

slice53 的 Anthropic 解码器拒绝 redacted_thinking，并把没有可见文本的 thinking
块视为未完成，即使它已经携带有效的非空签名。这会阻止后续工具调用以及结果回传。
不能通过丢弃块、生成占位推理或将 Anthropic signature 当作 Responses
encrypted_content 来修复。

本地 Codex `codex-rs/protocol/src/models.rs` 的 ResponseItem::Reasoning 将可见
summary、可选 content 与 encrypted_content 分开保存。本轮借鉴“可见文本与续传
数据分离”的建模原则，不声称 Codex 该文件实现了 Anthropic 协议或本轮的大小限制。

## 已写入代码

- 共享 Broker 新增 `ChatProviderReasoning`，仅有 AnthropicThinking 和
  AnthropicRedactedThinking 两个明确类型；分别保存原文/签名与原始 data。
  `ProviderReasoningBlock` 是完整原子事件，`ProviderReasoning` 是 assistant
  上下文内容，不允许作为用户、系统或工具结果输入。没有自由扩展的未知 JSON 入口。
- 原生 thinking 块按块累积文本和签名，block_stop 后发布一次完整事件；非空签名
  允许空文本。redacted_thinking 在开始时校验有界非空 data，在 stop 时发布。
  不为隐藏内容制造可见文本，不合并相邻的签名块/隐藏块。
- 每个 thinking 文本上限 1 MiB，签名和隐藏 data 各上限 256 KiB，仍受原有
  每消息 128 块、16 MiB wire、Coding 每 turn 8 MiB 语义事件等总预算限制。
  缺签名仍只能由明确 max_tokens 终态确认截断；不猜测有效签名、不补齐半块。
- 原生可见推理文本改为块闭合后一次性进入 Coding ReasoningDelta/UI，而非逐片
  发布；普通输出文本的流式呈现不变。此处优先保证块身份与完整签名对应。
- Coding 保留完整类型化块的顺序，供同一活跃工具循环的后续模型请求使用；只有
  visible_text 进入 Coding 事件和产品推理显示。Debug 实现也不输出块载荷。
- Anthropic 编码器逐字回传 thinking/signature 和 redacted_thinking/data。
  其他协议适配器在 transport 前明确拒绝类型化外来数据，不能转成普通文本、
  Responses 加密字段或静默丢弃后继续调用。Bedrock/Vertex 本轮也未放开。
- 摘要源只保留“private reasoning omitted”说明，不将签名或隐藏数据交给压缩
  请求。输出截断的现有策略只保留 Text，因此该步的类型化推理同样被作废。
- 共享 Responses Bridge 可接收和输出这一显式类型；旧文本消费者继续明确拒绝。
  社区 Engine 可以使用共享事实和编码器，不需要自己解释 Anthropic 原始块索引。
  编译摘要已纳入新增源文件。

## 保持的边界与剩余工作

平台仍拥有模型路由、凭据、Session、工具和效果；Engine 仅拥有循环与上下文策略。
没有新增 Agent 权限、Engine 热加载、会话中换引擎、工具重放或服务调用。

本轮只补齐活跃回合的 Anthropic 原生隐藏/签名块，不提供跨重启的秘密载荷持久化，
不改变历史投影仅回放可见事实的策略。压缩可能不保留旧推理，保留最新完整工具交换
时则按现有预算保留该交换中的完整块；不会把摘要当作有效签名替身。
协议内跨模型/凭据的签名兼容性仍由路由政策和供应商决定，本类型不承诺通用可移植性。
adaptive thinking、server tools、citation、Bedrock/Vertex 原生信封以及跨进程恢复
等未在此完成，Nomi 的独立循环也没有被替换成 Coding 循环。

未运行任何验证。代码落地不等同于已证明真实 provider 兼容、完整 CAR 验收通过或
Coding 能力全面优于 Nomi/Codex。
