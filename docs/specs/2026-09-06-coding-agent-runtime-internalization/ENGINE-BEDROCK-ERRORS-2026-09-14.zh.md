# 共享模型端口：Bedrock 结构化流异常（slice78）

2026-09-14，rf/agent-capability-platform-v2 本地工作树。
Coding host2-coding-loop78 / Nomi host50。未执行验证、迁移、模型调用、commit 或 push。

## 缺口与借鉴

chat_executor.rs 原先检查 AWS event-stream 帧长度及两个 CRC，但跳过全部头字段，
直接解码 JSON。Bedrock 的 exception/error 类型和代码可能只在头中，普通 message
诊断体因此丢失错误语义。对照固定 Codex 基线 6af345407d9c2a568da9d01b6c4b81a9e61495c0
的 codex-api/src/sse/responses.rs response.failed 分类：学习的是根据明确协议错误
码分流、把恢复策略交给循环的原则，不声称 Codex 提供本实现的 Bedrock 协议代码。

## 实现

- 新 chat_bedrock_headers.rs 在 CRC 检查后读取有界头：最多 128 KiB/128 项，
  遍历 AWS 十种值类型，拒绝截断、重复头、非字符串伪头及消息类型冲突。
- event/chunk 保持既有 payload 通路；exception/error 提取明确类型。现有无头
  gateway 帧保持兼容，不从自然语言消息猜错误码。
- 错误体仅允许诊断字段，拒绝夹带 model chunk、tool 或 usage；头部直接承载的
  异常可有空 payload。只向 Broker 投影 code，不传播原始 message/originalMessage。
- Broker 对限流和暂时不可用使用既有有界 Failover；鉴权、参数、资源缺失、未知
  及模型超时/流错误不自动重试。ValidationException 不等同于 PromptTooLong。
- 成功解析异常、坏帧、传输失败或截断 EOF 后停止当前帧流，不重复输出同一错误，
  不消费错误之后缓存的模型数据。已有 Broker 语义输出屏障继续阻止自动重放。
- 两个官方 Engine 的 build digest 纳入 chat_executor.rs 和头解析模块，共享
  改进可由社区 Engine 经平台模型端口使用，不新增 Engine 私有 Provider 客户端。

## 边界与未执行项

没有运行测试/构建/真实 AWS 请求，也未新增成功执行证据。仅做源码阅读和两个
小模块的 rustfmt。仍需后续覆盖分片帧、重复/截断头、CRC、混合输出、header-only
异常、未知代码、已输出后失败和重试上限；这些检查本轮按用户要求不执行。
Bedrock 全量原生 Anthropic 工具/推理闭环、HTTP 特有错误及自然语言上下文溢出
不在本切片的完成声明中，不能用错误通路修复声称整个 Provider 已验收。
