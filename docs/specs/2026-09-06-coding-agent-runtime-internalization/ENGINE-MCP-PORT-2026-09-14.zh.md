# 当前产品 MCP 执行端口与协议边界（源码实现，未验证）

本切片在 `rf/agent-capability-platform-v2` 本地实施，没有 commit/push，
没有运行构建、测试、评测或 E2E。Coding build 更新为 `host2-coding-loop13`。
这不是 MCP 产品接入完成声明，也不是整体 engine 能力完成声明。

## 已写入的实现

### 生产数据与授权

`nomifun-app/src/router/nomi_core_mcp.rs` 使用当前产品的
`IMcpServerRepository`，不查询 Fresh-v4 的 materializations/package-config 表。
当前 `McpServerRow` **没有 user_id/owner_user_id 列**；目录属于安装用户，
适配器核对 `AppServices.authoritative_user_id`，不能从模型输入推定目录所有权。

执行必须同时匹配：Snapshot 的 MCP lock、当前唯一资源绑定、安装用户、
connect/invoke 授权、未删除且启用的服务器、`mcp-server:<id>@<updated_at>`
连接引用、固定工具 key 和 schema digest。资源 typed_parameters 不接受额外路由参数。
目录与配置解析错误使用固定诊断，不把 transport headers/env 或凭证写入模型错误。

固定 key 规则为 `nomi.mcp.v1.<canonical digest of [server_id, remote_name]>`，
materialization revision 为 1。**后续生产目录发布器必须使用同一规则**；
不能把已有任意 Plugin key 猜成 remote name，更不能由模型传 server/tool 身份。
本规则没有改写已有 Agent revision 或 Session Snapshot。
目前执行端仅接受 canonical `mcp.tool_proxy`；多工具能力目录的独立映射仍待实现。

配置上限 256 KiB、持久工具目录 1 MiB/256 个工具；工具身份不能重复，
不得把缺失的 schema 默认为任意对象。配置只读一次形成此次调用的固定事实，
不从后续模型参数重取 transport。认证复用平台 OAuth 服务/仓库接口；
验证 URL 后保留配置中的精确 endpoint 作为凭证查询键，不因补斜杠/去默认端口
而改查另一个 OAuth 记录。

### 独立分发和清理

生产 builtin composition 给 `NomiCoreWave2Host` 装配 MCP owner。
MCP 在 workspace 分发前按自己的 resource contract 进入端口；不要求伪造文件授权，
也不把 MCP 纳入 `coding_capability_ids()` 的工作区能力集合。

操作 ID 由完整 principal/session/operation 元组做 canonical digest 后映射成
协议安全 ID，不截断原 ID；平台日志仍持有原操作身份。

同一 Session 的 MCP 请求串行进入。已进入 owner 的任意失败、任务取消或 panic
都会保留进程内不确定标记，拒绝后续调用；本地目录/输入校验失败发生在此之前。
这里有意保守，包括尚无明确无副作用证明的初始化/凭证失败，不自动解除隔离。
最多保留 1024 个忙碌/不确定 Session 项，不能靠淘汰记录遗忘未知结果。

`EngineKernelSession` 在关闭工具准入并 join 已派发工具之后核对 MCP 状态。
已无本地进程不等于远端执行和资源清理已确定，不能据此前置发布成功 cleanup witness。

### HTTP 协议处理

- `tools/list` 读取全部分页后才调用固定工具，即使前面已经找到目标也继续检查
  后续同名冲突；限制 32 页、1024 个工具、累计 8 MiB、4 KiB cursor，拒绝重复 cursor。
- 分页与初始化/调用使用同一个外层 deadline，不按页重置预算。
- Streamable HTTP 的 SSE 响应按完整事件读取，收到相关联响应即返回，不再等待
  长连接 EOF；总字节和事件数量有界，按完整 UTF-8 frame 解码。
- JSON-RPC 版本、相关 ID、result/error 互斥和响应结构受约束。
  Sampling/elicitation 等 server-initiated request 明确拒绝，不自动取得平台权限。
- 初始化后的 Session ID 不允许被后续响应替换；协议版本 header 由 owner 管理，
  不接受静态配置覆盖。清理继续使用原 Session ID。
- `tools/call` 派发之后发生错误返回 `MCP_OUTCOME_UNKNOWN`，包括工具报错可能
  已产生部分副作用的情况。清理失败优先保留为 `MCP_SESSION_CLEANUP_FAILED`，
  不再被先前的操作错误掩盖。此通道没有自动重试。

## 向 Codex 学习的边界

阅读参考仓库 `multi/codex` 的：

- `codex-rs/codex-mcp/src/pagination.rs`：总 deadline、页数/条目/cursor 上限及循环检测。
- `codex-rs/codex-mcp/src/client_tool_catalog.rs`：目录身份/版本与调用一致，过期调用先拒绝。

采用上述约束原则，按 NomiFun 现有 owner 接口实现；没有复制 Codex 的动态目录
发布策略。NomiFun 的 Session 继续按不可变 Snapshot 固定权限与 schema。

## 仍未完成，不能开启支持声明

1. 生产工具目录到 Kernel materialized mapping、Agent revision/Snapshot 的发布链路，
   尤其每个远端工具的独立 capability/action/schema 身份、工具选择和变更处理。
   当前公共 canonical proxy 的单 capability 不能冒充服务器的全部工具。
2. Coding 的 MCP exposure 和 admission：**仍保持关闭**，没有设置 `policy.mcp = true`。
   新端口只有在调用方提供真实可编译的 lock 时才执行，不制造临时 Snapshot。
3. Stdio 与旧式 SSE transport owner、MCP resources、server-initiated requests 的生命周期。
   HTTP 响应使用 SSE 格式不等于已支持旧式 SSE transport。
4. 跨启动外部副作用 receipt/远端状态核对及人工解除隔离；当前隔离标记只在进程内，
   不能用它证明重启后安全。Coding recovery 继续拒绝未审计外部 owner，不开放自动续跑。
5. 真实 Provider/服务器与三平台的协议、错误路径和升级验证；本切片未运行这些工作。

MiniApps、非函数型 Plugin 生命周期、Git push、递归/动态 AGENTS 范围、
语义级需求覆盖与其他既有未完成项没有因为此端口实现而完成。
