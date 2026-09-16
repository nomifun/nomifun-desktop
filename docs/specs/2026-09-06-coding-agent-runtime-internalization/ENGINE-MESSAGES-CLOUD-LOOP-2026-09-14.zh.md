# Messages 云端闭环及签名推理路由隔离（slice79）

2026-09-14，rf/agent-capability-platform-v2 本地工作树。
Coding host2-coding-loop79 / Nomi host51。未测试、构建、调用模型、执行迁移、commit 或 push。

## 实际缺口

Bedrock/Vertex 虽复用 Messages 请求编码，却没有选择完整的 AnthropicDecoder；
tool_use/参数增量/块结束/消息结束可能仍落到旧规范化分支。ProviderReasoning 又仅允许
Anthropic 协议，导致云端签名推理无法续接。Bedrock 还携带 model/stream 字段，而
invoke-with-response-stream 的模型与 streaming 已由 URL 决定。

同时，原有 typed reasoning 只区分协议，没有记录具体产出路由。直接放宽协议判断
会让相同格式的签名被带到不同 Provider/model/config revision，不能这样接通。

## 源码实现

- ChatProtocol::uses_anthropic_messages 只表达 wire 家族。三个 Messages adapter 的
  每次 attempt 使用独立完整原生解码器，保留块顺序、工具 ID/参数、usage、终态和
  有界帧策略；先处理云端异常投影，避免已进入原生模式后丢失 Bedrock 异常类型。
- Bedrock 编码移除 model/stream，保留固定 anthropic_version；Vertex 编码移除
  model。Vertex 仅完成 adapter 层接线，不改变当前生产路由的拒绝条件。
- typed thinking/redacted_thinking 新增可选 route_digest。解码器产出未绑定块，
  Broker 依据实际产出 attempt 的 ResolvedChatRoute 计算并绑定，包括 route/revision、
  provider、model、protocol、connection/config digest、credential ref 和 features。
  解码器预填 origin 会被拒绝；route_digest 不含凭据明文，也不发给 Provider。
- 下次请求在 causality claim/凭据申请之前排除不匹配路由；编码器再次核对。
  包含不同 origin 的混合历史、没有来源的历史块、配置修订后的旧块不会被猜测、
  静默剥离或转成普通文字。仍可使用实际相同的 failover candidate，不必误绑 primary。
  digest 是误路由防护，不是签名真实性证明或新增 Session 权限；签名验证仍属 Provider。
- 平台合并 Provider 参数时，Messages 的 model/stream/messages/system/tools/
  tool_choice/thinking/version 只来自已准入请求，不由默认参数再次注入；采样默认值
  保留。Messages 固定使用 max_tokens，不再套 OpenAI 的自定义上限字段。
  宿主最终上限收紧后再次检查 thinking budget >=1024 且小于有效 max_tokens；
  不静默改变推理策略或发出已知无效请求。

借鉴仍限于固定 Codex 基线 6af345407d9c2a568da9d01b6c4b81a9e61495c0 的
core/src/client.rs 对 provider/session/turn 与续接状态的作用域区分，以及既有事件
生命周期原则；没有声称复制 Codex 的 Bedrock/Vertex 实现，也没有改变源码基线。

## 保留边界

- nomifun-model-invoke/src/manifest.rs 明确拒绝 Vertex：其 endpoint 需要独立
  project_id/location 合同，当前通用模板只支持 model。没有虚构配置或绕过该 owner，
  所以不能宣称 Vertex 生产可用。
- 保护覆盖 typed ProviderReasoning。旧规范化签名、其他协议私有状态、server-tool
  pause_turn、adaptive thinking 等仍需独立处理；没有新增云端托管工具执行权限。
- Coding 仍在完整消息终态后通过 Kernel 派发工具；坏流/不完整工具不自动执行。
  opaque reasoning 仍只在 live context，调试输出遮蔽，不新增跨重启私有历史存储。
  社区引擎使用共享 Broker 可获得同一合同，不新增 Engine 私有 Provider 客户端。
- 没有运行任何验证。需要后续覆盖真实 Bedrock 多轮工具/签名续接、原生与规范化混用、
  failover 后来源绑定、配置变更、混合来源拒绝、有效预算、异常后的无重放和旧 fixture
  兼容性；历史通过记录不覆盖新增实现。Nomi 消费者能否完整消费新事件也不由
  共享 Broker 接线本身证明。
