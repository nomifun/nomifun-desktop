# 原生 Responses 到 Engine 工具循环（slice52，未验证）

分支：`rf/agent-capability-platform-v2`。本轮未构建、测试、请求服务、运行迁移、commit 或 push。
Coding 构建身份更新为 `host2-coding-loop52`，Nomi 为 `host34`；共享 Broker 的新增源文件纳入摘要。
这是一轮源码实现，不是生产可用性或整体 CAR 完成证明。

## 实际缺口与参考

原适配器把 `response.output_item.added` 一律变成 NativeResponsesItem，而 Coding 明确拒绝该项。
真实函数参数增量通过 item_id/output_index 关联，不能把它们直接当作 call_id/name；原通用解析没有这层状态。
因此规范化事件夹具可以工作，并不代表原生 Responses 能完成 Coding 工具循环。

参考本地 `multi/codex/codex-rs/codex-api/src/sse/responses.rs` 的 item 完成、参数/摘要事件区分，
以及 `codex-rs/protocol/src/models.rs` 的 FunctionCallOutputContentItem / ContentItems 表达。
采用的是协议职责和结构化结果设计，不接入 Codex 二进制、SDK、会话或工具执行 owner。

## 已写入的行为

- 官方 Broker 每个 attempt 通过 request-aware decoder 工厂取得独立 Responses 状态。
  保留旧工厂的默认委托；原有规范化事件路径仍在，单流不能混用原生和规范化事件。
- 原生 `response.created`、输出项 added/done、函数 arguments delta/done、文本/拒绝/摘要
  delta/done、content/summary part added/done、completed/incomplete 转换为统一事件。
  SSE 命名事件和 `message/json` 内的 type 声明均可识别；矛盾声明拒绝。
- response ID、顺序号（存在时严格递增）、输出索引、item ID、call ID、函数名分别核对。
  参数完成内容必须延续已有增量；最终参数必须是 JSON 对象，完整调用只发出一次。
  最终 response.output 必须与已闭合项逐一完全一致，不能补漏或悄悄覆盖已输出结果。
- 原生流上限为 65,536 帧、累计序列化数据 16 MiB、128 项、每项 128 个内容索引、
  64 个唯一函数调用及单调用参数 256 KiB。Engine 原有的更小/独立预算仍生效。
- 有函数调用的成功终态映射为 ToolCalls；拒绝不冒充正常完成。Coding 仍在完整终态和
  自身参数/计划/权限检查后才执行；中断、错误、半截 JSON 和终态矛盾均不派发工具。
- 新统一事件 ReasoningBlock 是完整块，不与 ReasoningDelta 重复投递。
  Coding 的活跃模型历史保留块边界、摘要和 encrypted_content，支持无摘要的加密块；
  UI/永久 Coding 事件只发布可见摘要，不记录加密内容。压缩沿用私有推理省略策略，
  闭合回合的历史重建仍不恢复加密链，不将它当跨重启 checkpoint。
- `store:false` 请求显式申请 reasoning.encrypted_content；返回 response ID 仅作观测，
  不自动变成 ProviderRoundId 或下一轮 previous_response_id。显式传入 parent 的旧接口未移除。
  带 opaque Responses reasoning 的请求不得编码成其他协议，以免无声丢失或错用。
- 函数结果保留文本/图片/音频内容结构和 is_error 失败说明；图片不再序列化成一段 JSON 文本。
  工具声明使用 `strict:false`，不把平台含可选字段的 schema 错报成 provider 严格子集；
  不改 schema，不扩大工具权限，实际参数仍由 Kernel 校验。
- 未设置的 reasoning effort/summary 不发送 null/`none`；读取 Responses 嵌套推理/cache token 用量。

普通 consumer 不收到 message/function/reasoning 的 NativeResponsesItem。
显式 preserve_native_responses_items consumer 可额外收到完成后的原生项；它必须避免重复消费
规范化事件和原生快照。未知 provider-hosted 项在普通 Engine 入口直接拒绝，不转成本地工具。

## 尚未完成或未证明的范围

- 未运行任何编译/测试或真实 Provider 请求，本轮代码与继承的修改均没有新增运行证据。
- 自定义 adapter 若覆盖旧 factory，默认新工厂仍委托它；有请求级状态时需实现新入口。
  adapter 的 legacy direct decode_frame 仍是旧通用路径，不等价于生产 attempt 解码器。
- Responses audio output、原始 reasoning_text、annotation 事件、custom/free-form 工具、
  provider-hosted 工具的完整流式协议未接入。未知事件失败，不假装完整支持。
- incomplete 只在已闭合输出与最终快照一致时转换结束原因；包含未闭合/不完整函数或消息的
  响应仍作为协议失败处理，不补齐、不执行。尚未实现部分输出的专门产品呈现。
- 摘要按完整推理项交付，不是逐 token 的推理 UI；多 summary part 合并为可见摘要，
  opaque 内容保留但不是原始全部原生字段的无损存档。
- Anthropic/Bedrock/Vertex 等真实原生工具协议仍需继续补齐，不能用六个 adapter 类型的存在
  代替协议覆盖；跨重启进程证明、未知副作用人工处理、部分生态生命周期等仍未完成。

所有变化属于共享协议基础设施与 Coding 上下文策略。Agent 继续选择编译期注册的 Engine，
Session 继续绑定 exact build；不新增动态挂载、同 Session 切换、静默 Nomi fallback 或工具重放。
