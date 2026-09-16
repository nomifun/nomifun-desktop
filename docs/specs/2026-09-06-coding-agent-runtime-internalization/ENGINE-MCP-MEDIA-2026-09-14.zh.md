# MCP 二进制描述与显式图片读取（2026-09-14，未验证）

## 范围和状态

CAR-03 / CAR-06 / CAR-07，slice69。工作分支仍为
`rf/agent-capability-platform-v2`，不 commit/push。按用户要求未运行构建、测试、
E2E、服务、模型调用或迁移；仅源码阅读、实现与局部 rustfmt。
本文不是验证报告，也不代表整个 CAR 或所有 Engine 能力已经完成。

官方构建身份更新为 Coding `host2-coding-loop69`、Nomi `host44`，均将新增平台
媒体投影源码纳入 digest。既有 Session exact binding 不自动更新，不热切换，
缺失构建仍失败；社区扩展只能源码集成、编译打包并注册。

## 已写入的能力

1. MCP 协议 owner 接受互斥的 `text` / `blob`，拒绝双字段、null、非字符串及
   非标准带 padding 的 base64；原结果编码上限仍为 1 MiB、contents 为 1–64。
   所有 blob 解码后累计不超过 512 KiB。每项 URI 仍须等于所请求的 URI，
   MIME 如存在须为非空、有界且无控制字符的字符串。协议解码不是视觉授权。
2. 共享平台的普通分页和持久化 MCP 回执先做描述投影。read contents 只保留
   URI、MIME、零基 `content_index` 和文本，或 binary 的原始字节数、原始
   SHA-256 与 payload omission 说明。read 扩展 metadata 不进入此投影，
   不能伪造宿主 descriptor 或夹带另一份 blob。既有 list 输出政策不变。
   这不是通用敏感信息过滤器：服务端主动写入 text 的字符串仍是非可信文本。
3. `EngineResourcePort::read_image` 为独立可选方法，默认明确拒绝，不回退成
   base64 文本；输入 `EngineResourceImageRead` 包含同一 server/query、
   `content_index` 和必填 `expected_source_sha256`。只支持 read/read-template，
   不支持对 list 做图片转换；此 SHA 是原始解码 blob 的摘要，不是页摘要。
4. 生产平台沿用同一个资源 admission、steering/activation 临界区、64 次回合
   资源操作预算、retained task、实际 MCP owner、持久回执及清理屏障。图片在
   远端 IO 前还须具有 active `llm.vision`、约束允许且 primary model 支持
   ImageInput；解码前、返回前再次核对 exact generation/snapshot/vision。
   Broker 发送时的独立模型能力校验不变。
5. 图片只能来自服务器返回的 blob，MIME 限于 PNG/JPEG/WebP；与观察 descriptor
   的原始 SHA 不同、空内容、索引失效、类型不符都拒绝。固定内部参考扩展名选择
   解码格式，魔数须匹配；绝不将资源 URI 当成本地路径或客户端抓取 URL。
6. 图片准备复用已有平台 decoder：尺寸/像素/分配限额，最长边缩到 1568，移除
   metadata，重编码至原有 1.5 MiB 图像字节限额，返回 base64 不超过 2 MiB。
   媒体结果经 `EngineToolResult` 校验，只含有界说明和 typed Image。
   同时最多两项 MCP 图片解码，blocking decoder 持有 semaphore permit；调用方
   放弃等待不释放仍在运行的解码名额。外层由平台资源任务持有。

## Coding 使用流程

先通过原有 list/template 选择冻结服务端的资源，然后普通读取获得描述：

```json
{"server_id":"selected-server-id","uri":"resource://example/screenshot","format":"page"}
```

普通输出仍是分页 JSON；从组装的 `result.contents` 取得 `content_index` 与
`binary.source_sha256` 后，显式单独调用：

```json
{"server_id":"selected-server-id","uri":"resource://example/screenshot","format":"image","content_index":0,"expected_source_sha256":"<上次 descriptor 中的 64 位小写十六进制摘要>"}
```

第二例中摘要是占位说明，必须替换为真实值。模板读取则保留相同 uri_template
与 variables，不与 uri 混用。image 模式禁止 offset/limit/expected_sha256；
page 模式禁止 image 字段，显式 null 拒绝。Coding 先检查当前 image input，
宿主仍独立检查全部权限。没有新 Tool 名称、Tool grant 或自动 capability 激活。

每次调用都会重新观察远端，不是缓存或原子快照。原始 blob 摘要一致仅说明所选
字节一致，不保证其他内容项或远端状态不变。普通分页摘要域升级为
`engine-resource-page-v3`，覆盖 server/query/投影 JSON；旧页摘要不能续页，
被省略的 metadata 变化也不构成完整远端版本检测。

## 回执、失败和历史

远端 owner 返回且协议清理完成后，先保存不含原始 blob 的回执，然后准备像素。
已知远端 rejection 返回 `is_error=true` 且无图片；权限、摘要、解码或投影失败
不会抹除已发生的远端观察。无论失败还是成功，都不自动重放资源事务。
未知 owner/cleanup 结果继续按原隔离政策处理，不将 projection 失败伪装成
“远端未执行”。取消 Engine waiter 不等于取消或清理完平台任务。

图片进入现有 Coding 媒体窗口、事件描述和压缩政策；不新增原始二进制库、
图片持久档案或像素重放机制。Nomi 共享 descriptor/receipt 投影，不自动采用
Coding 的 image 控制策略。本切片尚未给 Nomi 资源 Tool 增加显式像素入口。
历史既有回执不迁移；新投影规则不宣称能追溯清除过去已经保存的文本。

## Codex 参考及剩余边界

继续使用既定源码基线 `6af345407d9c2a568da9d01b6c4b81a9e61495c0`，阅读
`codex-rs/core/src/tools/handlers/mcp_resource/read_mcp_resource.rs` 和
`codex-rs/core/src/tools/handlers/mcp_resource.rs`：参考显式 server/URI 读取、
资源操作事件与受限模型输出投影分层。该基线的 handler 将资源结果序列化并
截断为文本；本次 descriptor/显式图片接口是 nomifun 平台边界下的实现，
不声称 Codex 采用相同摘要、像素预算、权限或回执算法，不更换基线、不引入 Guardian。

仍未实现任意二进制下载/解析、PDF/音频/视频/其他图像格式、MCP subscriptions
及服务端主动请求授权生命周期、Nomi 显式资源图片入口。更广的 Git 网络凭据、
未知效果人工协调、跨启动进程证明及跨 Session 工作区协调也不属于本切片。
这些边界不能用 descriptor、图片内容、cleanup receipt 或现有测试历史代替。
本轮没有质量、性能、费用、跨平台或端到端可用性证据。
