# MCP 资源上下文接入（源码实现，未验证）

本切片继续在本地 `rf/agent-capability-platform-v2` 实施。Coding 构建标识为
`host2-coding-loop44`，Nomi 为 `host27`（共享 MCP owner、准入契约变更）。
没有执行构建、测试、服务调用、数据库迁移、commit 或 push。

## 职责与授权

`mcp.resource` 仍是 ResourceProvider，不新增 canonical Tool action。Agent 选择能力
与资源绑定；Engine 决定何时请求上下文；平台负责连接、权限、任务持有、永久效果凭据
及清理。没有新增 Engine 动态挂载点，社区仍需源码集成、编译、打包注册。

共享 `EngineResourcePort` / `EngineResourceRead` 描述列表与文本读取。生产 Session
检查 bundled 资源能力、当前 Registry/Snapshot、激活代次、执行上限和唯一服务器的
编译后 policy；服务器配置按准入时引用重新核对，必须启用且属于当前安装用户。
资源读取要求 `connect + read`，不借用 `invoke` 或远端工具 schema。

Coding 提供 `list_mcp_resources` / `read_mcp_resource` 两个模型控制入口，只在 Agent
选中该能力时安装端口；按需能力须先激活。它们不伪造 ToolStarted 授权，实际派发写入
独立 host resource 记录。混合工具批次不执行任何调用；资源读取不并行批量派发。

Agent 工作台沿用既有资源选择器。工具映射的多服务器模式不适用于未映射的资源消费者；
资源型 Agent 只允许一个 MCP 服务器。Session 的 overlay 与服务器名称只是投影，
校验以平台已解析的 Agent binding 为准，资源专用服务器不要求存在工具目录。

## 协议、分页与生命周期

- HTTP、legacy SSE、stdio 共用现有 MCP owner，要求初始化明确声明 resources。
- 每次读取先完成同一协议 Session 的资源目录，最多 32 页、256 项、合计 1 MiB；
  重复 URI、重复游标、无效元数据或未完成目录均拒绝。
- 只读完整目录中的精确绝对 URI；客户端不自行打开该 URI 指向的路径或 URL。
  远端返回内容也必须标识同一 URI。
- 当前仅文本，单次原始结果最多 1 MiB、最多 64 个内容项；不将 blob 冒充文本或图片。
- 模型结果是序列化 JSON 的 UTF-8 字节片段，每页 4..8192 字节，外层结果也有上限。
  非零 offset 必须带前一页 sha256。摘要绑定实际序列化字节，不只绑定等价 JSON。
  每页重新观察远端，内容变化立即拒绝续页；没有稳定内容缓存或远端快照保证。
- 每回合最多 64 次资源请求。因此不能保证在一个回合内读完每个接近最大体积的资源；
  输出显式保留 total_bytes、next_offset 和 eof，不把已读片段描述成完整结果。
- 初始化/OAuth/stdio 启动前先持久化该回合的 MCP 效果凭据。协议总时限与清理时限分开，
  HTTP 清理 2 秒、stdio 清理 10 秒。明确返回并完成清理后才结算凭据。
- 调用 future 被取消不取消平台持有的事务任务；激活及回合清理加入资源任务等待。
  未知结果、清理失败、凭据失败保持隔离，不能因其名为 read 就推断无副作用。

Coding 新输入与资源任务注册共用派发边界锁。任务注册后释放锁，用户可继续追加输入，
但不会把尚未收尾的旧请求伪装成新输入之后才发起。社区 Engine 使用共享端口时同样
需要在激活、下一次模型请求、清理之前等待已持有的请求，不能只丢弃等待者。

## 重启历史

`host_resource_dispatch` 关联 call、完整 causality、请求摘要与资源 binding；派发前
检查该模型 operation 已被 Broker 准入。`host_resource_settled` 记录 owner 返回状态。
Coding 恢复核对 exact build、Session/回合/源消息/Snapshot、此前已 claimed 的模型记录、
操作摘要和唯一性；资源凭据与普通 MCP 工具凭据分开匹配。

pending 凭据直接阻止恢复。对于本 exact build，没有资源凭据代表未进入 owner 事务；
已有 settled 凭据仅允许关闭中断历史，不授权重新读取。成功 settlement 记录必须匹配
成功 owner 凭据，剩余无法归属的凭据拒绝恢复。持久历史读取跳过平台记录，不将其
误解析为 Coding 策略事件或重新发起请求。

## Codex 借鉴与未覆盖项

参考本地 Codex 的 `codex-rs/core/src/tools/handlers/mcp_resource/read_mcp_resource.rs`：
资源 handler 与 Session MCP 执行分离，读取前检查服务器访问权。Nomifun 使用自身
冻结 Agent policy/资源绑定/Conversation 凭据，不复制 Codex 的服务器名路由作为授权。

本切片不包括 URI templates、binary 资源、subscriptions、服务器主动 sampling/roots/
elicitation 授权、持久 MCP Session 或人工未知效果解除。Nomi 的既有资源上下文路径
没有迁移到新动态端口；不要将共享协议实现描述成两种 Engine 已拥有完全相同的资源入口。
其他生态生命周期、跨 Session 工作区协调等仍在阶段状态中保留。整体 Engine/CAR
目标未宣告完成，所有本切片修改仍未经过编译或运行验证。
