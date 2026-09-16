# 资源明确失败与未知效果分离（源码实现，未验证）

继续在本地 `rf/agent-capability-platform-v2` 实施，未 commit/push。
Coding `host2-coding-loop47`、Nomi `host30`。按用户要求，没有运行构建、测试、
E2E、服务调用或数据库迁移；只阅读源码并局部格式化。整体目标仍未完成。

## 修正的行为缺口

此前资源 owner 将“完整目录没有所需 URI”“服务器不提供模板方法”等明确失败也
返回为外层错误。平台只结算外层成功，于是这些正常拒绝会留下 pending，进而阻止
后续模型回合。这混淆了“没有获得资源”和“无法确定调用是否返回/是否完成清理”。

本切片在 MCP owner 内区分三类结果，不放宽 Agent 或资源绑定：

| 观察 | 平台记录与 Engine 行为 |
|---|---|
| 有效资源数据，并完成协议清理 | 结算凭据，返回数据页，`is_error=false` |
| 有效初始化后未声明 resources；完整目录缺少 URI/模板；资源目录或读取返回匹配 ID 的有效 JSON-RPC error，并完成协议清理 | 结算“明确失败”，每页 `is_error=true`；Engine 收到工具失败，不当成资源数据 |
| 超时、断线、未匹配响应、坏协议/数据、初始化失败或清理失败 | 外层错误，保留 pending/隔离，不能从错误名称推断已无效果 |

目录途中发生错误不会保留部分目录为授权；不会继续派发资源读取。直接资源目录与
模板目录仍彼此独立，不因一种方法未实现而暗中切换另一种读取方式。

## owner 与权限边界

`McpResourceFailure` 是 owner 生成的结构化事实，放在远端 result 之外，包含
resources unavailable、URI/template not listed，或资源方法及数字 RPC 错误码。
不复制服务器的原始 error message/data，避免把凭据或不受信任指令带入平台诊断。

RPC rejection 必须来自完成初始化后的资源请求，协议版本、响应 ID、result/error
互斥结构都符合要求；不按任意 `McpOwnerError` 的字符串/错误码解除隔离。
初始化、凭据处理和其他协议路径不适用这一分类。

只有独立清理预算内 `close` 明确返回成功，owner 才发布明确失败结果。stdio 启动
可能有副作用，HTTP/SSE 的远端服务也可能有内部或后台效果；明确失败与成功数据
一样，都不构成无效果、回滚或远端物理静止的证明。清理失败优先于明确业务失败。

## 两种 Engine、共享端口和恢复

共享资源分页从 owner 外层读取 failure，而不是远端 result 中的同名字段。
每一页都携带宿主生成的 `is_error` 和 failure；即使 JSON 片段很短，失败事实也不会
消失。Coding 和 canonical Nomi 都按此标记生成失败 ToolResult；缺少有效标记也不
被解释为成功。SDK 的 `EngineResourcePort` 文档同步说明该响应契约。

永久 MCP 凭据仍在任何 owner/OAuth/初始化/进程效果之前创建。明确失败也计入每
回合 64 次资源调用限制。结算观察额外保留 `resource_outcome`，其状态为 available
或 rejected，且明确 `rollback_proven=false`；不会把 resource outcome 与其他 MCP
工具的远端 JSON 字段混为一谈。

回合恢复上下文在裁剪远端观察时单独保留 outcome。旧凭据缺少该元数据时保持未指明，
不自动补成成功。没有新表或迁移，也不修改旧 pending 为 settled。

Coding journal 的 owner_returned 表示 owner 已明确返回并落盘结算，不表示资源成功。
现有 exact-build 恢复继续检查真实凭据与派发关联，只关闭历史，不重放工具。
源输入 retry/edit 的永久效果检查仍阻止把已结算事务当作可以重发的旧请求。
没有新增传输自动重试或模型成功判定。

## 未完成范围

本切片只修正资源路径，未将同样分类扩展到一般 MCP tools/call 或初始化失败；没有
提供未知效果人工解除流程。二进制资源、复合模板参数、订阅/主动授权、Git 网络凭据、
跨 Session 工作区协调等仍见 TASK-MANIFEST 剩余清单。当前改动尚未编译或运行验证。
