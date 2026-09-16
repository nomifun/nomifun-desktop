# Coding 输出截断续接（slice54，未验证）

分支仍为 `rf/agent-capability-platform-v2`。本轮没有构建、测试、真实模型请求、迁移、commit 或 push。
Coding 构建身份更新为 `host2-coding-loop54`，Nomi 为 `host36`，新增策略源文件纳入 Coding 摘要。

## 问题与设计依据

原 Coding 在收到 MaxOutputTokens 后结束本轮，SDK 会投影 MaxTokens（并非 EndTurn），但没有
引擎自己的续接策略。若有半截工具参数，又会在 StepState.finalize 先行失败。
另外 OpenAI Chat 原生工具完成转换会把无法解析的参数包装成 `{"raw": ...}`，甚至为空参数
构造 `{}`；不能用修补后的对象冒充模型已经完成的函数参数。

本地 Codex `codex-api/src/sse/responses.rs` 将 response.incomplete 与 response.completed
明确分开；本轮沿用“不把截断当完成”的原则。下面的最多两次、修改上下文后续接，是 NomiFun
Coding 自己的有界策略，不声称 Codex 原实现具有相同的自动续接行为。

## 引擎策略

- 只有统一协议明确给出 MaxOutputTokens 才进入续接。EOF、连接故障、错误帧、坏协议、
  refusal/cancel 不转成续接许可，不新增 Broker 传输重试或路由切换逻辑。
- 每个 turn 最多两次续接，工具成功或用户 steering 不重置该计数；仍受原有 max_model_steps、
  单请求输出上限、累计流预算、上下文预算和取消机制约束。不增加模型额度。
- 在任何本步 tool admission 之前持久化 ModelOutputTruncated，记录步骤、全部作废提议 ID
  以及是否允许下一步。整个模型步的工具提议都不执行，包括已完成 JSON 的同批调用。
- 从本步只保留可见 Text，丢弃工具参数和推理/签名/opaque 续传；清除 provider parent 和
  message round ID。前面步骤的实际工具结果与效果不受影响。
- 追加引擎观察提示：原回答不完整、当前提议没有执行、前面效果不重放；需要的调用应以
  新 ID 和更小的完整参数重新规划，不能给旧半截 JSON 追加后半段。
- 返回正常模型边界，继续执行 steering、路径指令刷新、压缩、模型新 operation 及全部
  owner/权限约束。并不是跳过平台状态检查直接重发旧请求。
- 预算用完则 TurnFailed，保留输出截断事实，不把未完成任务作为 EndTurn 发布。
  continuation=true 只表示准许一次下一步，取消/压缩失败仍可阻止实际发送。

## 历史与产品投影

新的历史投影核对同一 model step、调用提议 ID 集合与未出现 ToolStarted/ToolCompleted；
有冲突时拒绝重建。经确认作废的提议从派生历史删除，保留部分文本和截断说明，不伪造失败
工具结果，也不把它们重新描述为“可能已经执行”。标记后出现本步工具派发/输出同样拒绝。
旧的、没有作废事实的缺失结果仍保守处理，不改变以前的未知效果隔离。

UI 投影移除尚未发布 ToolStarted 的调用缓存；不发布假的 Running/Completed 工具事件。
本轮未增加独立的截断提示 UI，也没有把新请求当作用户新消息或扩大原始任务范围。
永久标记不用于自动重启 checkpoint，恢复仍按既有 exact-build/owner 证明工作。

## 原生协议配套

- OpenAI Chat 只在正常/工具终态解析函数完成；参数必须是真正的 JSON 对象，空参数字符串
  和错误 JSON 不再被修补。length/refusal/cancel 不制造 ToolCallCompleted。
- Anthropic block_stop 遇到 JSON EOF 或缺失完整 thinking 文本/签名时暂存“不完整块”状态，
  不发布函数完成或不完整签名。此后不能出现新内容块，只有最终 max_tokens 可确认截断；
  正常/tool_use 等终态仍拒绝。非 EOF 的坏 JSON 不变成可续接截断。
- Responses output_item.done 可保留显式 incomplete/in_progress 的函数/消息快照；
  不完整函数只保留增量身份，不发 ToolCallCompleted。最终 success 不得含不完整项。
  response.incomplete 必须具备明确 reason，最终 output 仍逐项等于已关闭快照。
- 编码或协议发现缺失 block/item closure、矛盾身份/快照及其他非法序列时仍失败，
  不能凭自然语言“太长”或未收到数据就推断为合法 output-limit。

## 未验证与剩余范围

未运行编译或测试，旧通过记录不覆盖这些修改。两个原生状态机和 OpenAI Chat 的真实终态
兼容性仍需后续执行证据；不能宣称每家 provider 的所有截断序列均已支持。
缺失 block/item done 的流、redacted/adaptive/专有 hosted-tool 协议及独立产品提示仍未覆盖。
这是 Coding 特有执行策略；Nomi/社区 Engine 可以消费同一协议事实，但没有被强制采用该策略。
多 Engine 编译期注册、Agent 选择和 Session exact binding 的边界不变；整体 CAR 尚未完成。
