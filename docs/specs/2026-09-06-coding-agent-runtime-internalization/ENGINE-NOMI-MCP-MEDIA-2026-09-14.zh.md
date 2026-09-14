# Nomi 显式 MCP 图片接入（2026-09-14，未验证）

## 状态与范围

CAR-03 / CAR-07，slice70。接续 slice69，补齐其明确保留的 Nomi 显式资源图片
入口。所有工作仍在本地 `rf/agent-capability-platform-v2`，未 commit/push。
未运行构建、测试、E2E、服务、模型调用或迁移；只有源码阅读、修改和局部格式化。
整体 CAR 仍在实施中，本文不是运行效果或发布验收证明。

当前官方标识 Coding `host2-coding-loop70`、Nomi `host45`。Coding 标识变化是因
其 build digest 包含共享宿主/资源源码，本切片未修改 Coding 执行循环。
既有 Session exact binding 不热换，社区仍只能源码集成、编译打包并注册。

## 实现

- Nomi `mcp_resource_read` 增加 `format=image`，使用共享
  `EngineResourceImageRead` 的 server/query/content_index/expected_source_sha256。
  uri 与 template+variables 互斥；图片禁止分页字段；普通分页禁止图片字段；
  显式 null、未知字段、非小写 64 位摘要和越界 index 拒绝。
- `NomiMcpResourceInvoker::read_image` 是可选默认拒绝方法。旧的 text-only
  实现无需增加一个虚假的成功图片实现。没有用 Coding causality 构造 Nomi
  请求；结果内部 call_id 使用真实的 Nomi scoped tool operation，并在转换前核对。
- 新增 `NomiResourceImageAuthority`。平台资源 adapter 和 Nomi 工具共享同一
  initially-unbound handle；Nomi 运行时构建才将实际解析的模型图像支持和原生
  vision activation `Arc<AtomicBool>` 绑定到 OnceLock。未绑定、无开关、未激活、
  模型不支持均拒绝；二次绑定失败，不接受工具 JSON 中的所谓 image-authorized 标志。
- 现有 Skill image policy 的注入点拓展为 context image policy，一次同时接上
  Skill 和资源图片。Skill 行为不变。Nomi 原生 `activate_vision_input` 仍是
  on-demand vision 的激活方式，不伪造 Kernel 的 llm.vision activation，
  也不让 MCP ResourceProvider 激活依赖顺带授予视觉。
- 平台还独立检查从已验证 Revision/Snapshot 得到的 primary ImageInput、
  编译选择的 bundled/platform-builtin llm.vision、ExecutionConstraints、
  exact snapshot 和资源代号。Nomi 的 active vision 使用上述真实原生开关，
  不以 Coding active-set 中是否有 llm.vision 取代它。
- ResourceOwner 统一观察方法供 page/image 复用，串行 guard 持有到全部图片
  准备结束。原操作身份、精确冻结 server/connect/read、目录成员核对、回合
  资源预算、owner receipt、未知效果隔离均不变。图片读取前校验视觉，远端
  返回后及解码后再次校验资源 generation/视觉；owner envelope 身份独立核对。
- 从远端 IO 到 cleanup、持久回执和图片 decoder 的完整 future 都由既有
  EngineEffectScope 持有。取消等待不丢失清理所有权，不重跑事务。
- 媒体准备完全复用 slice69 的 platform `engine_mcp_media`：原始 blob SHA、
  PNG/JPEG/WebP MIME/魔数匹配、有界解码、去 metadata、缩放/重编码、两项并发
  decoder 和被 blocking 任务持有的 permit。没有新增本地文件/URL reader。
- 返回前核对 EngineToolResult 身份及总量；只接受有界错误文本，或一段说明加
  一个 PNG/JPEG typed Image。说明上限 24 KiB、编码图片上限 2 MiB；拒绝 audio、
  任意媒体、错误夹带图片或伪装成成功的无像素结果。合法图片映射为 Nomi 原有
  `ToolResult.with_images` / ToolImage，沿用已有 provider/图片预算/历史处理。

## 产品与模型行为

Agent 仍在工作台选择 Engine，Engine 不变成用户逐条调用的 Agent，也没有新增
全局 Engine 切换页面。两个官方 Engine 共享资源 owner/媒体约束，分别保留自己的
工具名、激活策略、执行循环和上下文政策。

在 Nomi 中先使用 mcp_resource_list 或 mcp_resource_templates 取得已绑定资源，
以 mcp_resource_read 普通分页读取 descriptor；随后显式单独请求 image 格式，
携带对应 content_index 与 binary.source_sha256。若 llm.vision 是 on demand，
须先通过已有 activate_vision_input 激活。Resource ToolSearch 激活不能代替视觉。

每次读取仍会重新观察远端；图片失败可能发生在远端已经返回且清理之后。
失败不等于“没有效果”，不触发自动重试或 base64 文本回退。MIME/摘要/索引变化
不会返回像素。原始摘要不证明整个远端快照，也不同于重编码像素的摘要。

模型支持是运行时构建时由既有 exact provider 解析确认的事实，不声称本切片新增
逐调用模型能力探测、runtime downgrade 通知或新的 provider 协商机制。Provider
发送、失败和旧图片裁剪仍走 Nomi 原实现。新增代码没有实测质量或性能结论。

## 剩余事项

本切片解决的是 Nomi 资源图片入口，不代表支持任意二进制下载/解析、PDF/音频/
视频/其他图片格式、MCP subscriptions/server-initiated 请求授权或全生态 lifecycle。
Git 网络凭据绑定、未知外部效果人工协调、无 witness 的跨启动进程恢复、跨 Session
工作区协调及实际验证仍未在这里完成。没有改动 Codex 固定参考基线或引入 Guardian。
