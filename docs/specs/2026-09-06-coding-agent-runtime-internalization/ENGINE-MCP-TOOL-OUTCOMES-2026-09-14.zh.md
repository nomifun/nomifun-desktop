# MCP 工具返回失败与 Engine 反馈（源码实现，未验证）

继续在本地 `rf/agent-capability-platform-v2` 工作，未 commit/push。
当前 Coding `host2-coding-loop48`、Nomi `host31`。没有运行构建、测试、服务、E2E
或数据库迁移。已修改旧结构校验断言以匹配新语义，但没有执行该测试。

## 现有缺口与改动

源码显示：`validate_tool_result` 将有效 `isError: true` 直接转为 owner error，
随后 `invoke_session` 因 tools/call 已开始而转为 `MCP_OUTCOME_UNKNOWN`。
因此，服务器正常报告工具执行失败，也会留下永久 pending 并阻止后续模型回合。

本切片将 MCP 的两种明确返回统一为可观察的失败结果：

- `tools/call` 返回结构有效、带 `isError: true` 的 result：保留原始结果作为不可信
  数据，仍检查 content 数组及其条目结构，不把失败标记当作跳过校验的理由。
- `tools/call` 返回匹配请求 ID 的有效 JSON-RPC error：owner 转成 `isError: true`
  的结果，只携带数字 RPC 错误码和固定说明，不复制远端 error message/data。

调用仍须先经过初始化、完整冻结工具目录及精确 schema 检查。坏协议、缺失/不匹配
响应、超时和断线仍是外层错误。独立协议清理失败优先于明确工具失败；仅清理成功后
才能将上述结果返回给宿主。这里记录的是“返回了失败”，不是无副作用或回滚证明。

本地 Codex 的 `codex-rs/core/src/tools/handlers/mcp.rs` 使用 MCP ToolOutput 和结构化
CallToolResult 的路径仍作为分层参考；没有引入其 Session、审批或凭据 owner。

## 平台结算早于 Engine 失败投影

共享 `McpOwnerAdapter::invoke` 返回经校验的 owner 结果，Ok 可能包含 isError。
canonical Conversation 宿主先将结果写入永久 MCP 凭据、完成内存占位结算，再通过
`project_mcp_tool_result` 转成 `MCP_TOOL_RETURNED_FAILURE`。

这样 Nomi/Coding 和使用相同 Kernel 工具端口的社区 Engine 都经现有错误通道得到
失败，不需要 Engine 自己猜测远端 JSON，也没有给 Kernel 增加 MCP 专用执行循环。
Coding 保留 Kernel 错误代码；Nomi 的安全错误投影新增固定说明，不泄露原始诊断。
原始工具输出在永久恢复上下文中仍明确标注为不可信观察，不是新权限或任务完成依据。

已有 Wave2 非 Conversation MCP 入口也使用相同结果投影，避免将 isError 当成普通
成功输出；没有借此声称该旧入口获得了 Conversation 的永久效果归属和恢复能力。

## 永久观察与重放约束

MCP 工具结果的 isError 在 4096 字节观察截断后仍显式保留。每轮恢复上下文另行携带
`tool_reported_is_error`，不依赖正文是否被再次裁剪；旧记录缺失标记时保持未指明。
false 只代表该结果未报告工具错误，不证明用户任务完成。

没有把远端自定义的 metadata 提升为平台结算证明。所有已派发事务依然受到永久
源输入 retry/edit 检查约束；同一操作不能因为失败而自动重发。无新增传输重试或
旧 pending 自动解除，成功清理也不等于远端物理静止。

另外，当永久凭据预留失败、尚未进入 owner 时，释放内存中的活动占位；可能已经
落盘的 pending 仍保留。与资源路径一致，明确的本地准入失败不能凭空制造远端活动。

## 保留边界

初始化、凭据、工具目录/冻结 schema 不匹配等失败仍未分类为可结算的明确失败。
未知效果人工处置、二进制资源、复合模板变量、订阅/主动授权、Git 网络凭据与跨
Session 工作区协调仍在剩余清单。所有新增实现未编译或执行，整体 Engine 目标未完成。
