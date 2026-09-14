# 共享模型流的有界 SSE 分帧（slice80）

2026-09-14，rf/agent-capability-platform-v2 本地工作树。
Coding host2-coding-loop80 / Nomi host52。未测试、构建、调用模型、迁移、commit 或 push。

## 已定位缺口

原 SseFrameStream 仅在缓冲中找不到换行时检查长度，带换行的大块可绕过单行限制；
多行 data 在 JSON 解码前无限累计，后面的 Engine/wire budget 无法保护这一层。
还存在反复 drain 导致拷贝、未处理 lone CR、EOF 对未消费 buffer 的处理不一致、
错误之后可继续读取及 ready-only 注释/空块源不让出调度的问题。

## 源码实现

- 分帧迁至 chat_sse.rs，保留同一个平台单次传输入口，不增加 Engine 网络客户端。
  使用 pending chunk + 游标和有界 line/event 缓冲，不反复搬移剩余整块数据。
- 单行在追加前受 configured limit 限制；默认仍为 1 MiB，配置允许 1 byte～16 MiB。
  每个事件最多 16 MiB 规范化行字节及 65,536 行，覆盖 data/event/注释/忽略字段。
  data 拼接另有同一字节上限，不靠 JSON 解析后的预算补救。
- 生产传输 chunk 在复制成 Vec 前拒绝超过 16 MiB；SSE 自身也防止自定义源绕过。
  这不保证 HTTP 库从未分配过原始 chunk。非流式 JSON 改为在 append 前检查上限。
- 支持 LF、CRLF、lone CR、跨 chunk 的 CRLF、首行 UTF-8 BOM 及跨 chunk UTF-8。
  空的 data 行仍参与多行拼接；comment-only/event-only 记录不伪造 JSON 事件。
  id/retry 不触发重连或重放。
- 只有空行分隔才提交 SSE 事件；EOF 遇到未提交内容报错，不将半截或无分隔 JSON
  当作成功终态。正常 EOF 也不合成模型 Completed，完成仍由 Broker 协议校验。
  [DONE] 与 event 名冲突时报错；合法标记后结束这次源，不处理尾随数据。
- 错误只返回一次并释放 SSE 源和自有缓冲。无事件的 ready-only 源在累计工作预算
  后主动唤醒并让出，便于上层取消和其他 Session 调度；这不是重置超时。
- 原 ParseError→ProtocolViolation/Never 映射和已有语义输出后的禁止重放屏障保留。
  两个官方 Engine 的摘要纳入新模块，Session exact build 不自动切换。

## Codex 参考与边界

读取固定基线 6af345407d9c2a568da9d01b6c4b81a9e61495c0 的
codex-api/src/sse/responses.rs：其使用 eventsource_stream 处理事件边界，并在外层
区分 idle timeout、错误和协议终态。本轮借鉴边界分层，没有复制其 parser，也没有
引入新的依赖或声称达到 Codex 的流式性能。

当前 reqwest 请求仍采用既有总超时（生产调用为 120 秒），尚未改为 Codex 式独立
事件 idle timeout。SSE 有界化不等于所有 JSON/AWS 源的调度公平性均已闭合。
没有运行分块、边界、取消或真实 Provider 回归；旧 SSE 测试源码保留但未运行。
后续需覆盖多事件合并块、换行分片、BOM/UTF-8、空 data、多行上限、超大块、EOF、
错误后终止和持续注释取消。不能把本轮源码阅读/rustfmt 算作这些验证通过。
