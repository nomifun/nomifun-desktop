# Nomi 逐工具 MCP 恢复与 Coding 指令刷新（未验证）

> 历史切片：下文单服务器限制由
> [ENGINE-MCP-MULTI-SERVER-2026-09-14.zh.md](ENGINE-MCP-MULTI-SERVER-2026-09-14.zh.md)
> 的多服务器精确资源绑定替代；其他未验证标记仍有效。

任务：CAR-05 / CAR-06 / CAR-07。仅在 `rf/agent-capability-platform-v2` 本地继续实施，
没有 commit/push，没有运行构建、测试、评测、迁移演练或 E2E。
本切片最新 Nomi build 为 `host6`，Coding build 为 `host2-coding-loop18`。
局部 rustfmt 仅用于格式化，不代表编译或行为验证通过。

## Nomi 逐工具 MCP：源码准入已开放

此前 host5/loop16 文档中的“仍关闭准入”属于历史状态。
当前 Nomi 和 Coding 都可投影 Agent Snapshot 精确选中的 HTTP MCP 工具；
仍限一个服务器，拒绝与原生 MCP connect/proxy/resource/oauth 全量发现混用。
Factory 同时检查 eager/deferred 原生 MCP 工具名，清空原生服务器发现配置。
连接版本、资源归属、schema、capability/action 和 lock 仍由平台核对，
Session overlay 不能扩大 Agent 的服务器选择。没有新增聊天页引擎切换入口。

这里没有将 transcript 回滚当成远端事务回滚：

- migration 097 给既有 owner 凭据增加 bounded observation，结果与 settlement 同一 SQL
  UPDATE 持久化。旧 settled 记录没有观察时明确标注不可用，不补造历史。
- 已派发 MCP 的原始 source message 不允许自动换模型重发、剔图重试或编辑重提，
  即使其协议会话已经清理完毕。源身份沿现有 accepted turn receipt 关联。
- 自动重试与编辑预检经过 Nomi turn gate 和 owner replay witness；回退执行再次核对。
  未实现 replay proof 的其他 effect owner 默认拒绝证明，不以 cleanup 冒充 replay 安全。
- 每次 Nomi 模型调用前，required ContextContributor 从平台读取最近效果记录；
  读取失败或超时停止模型调用。记录独立于 transcript 的回退、清空和压缩。
- 最近 16 条、记录累计 32 KiB、最终 JSON 64 KiB 上限；较大观察保留摘要和截断预览，
  标明省略数量。远端输出仅是不可信数据，settled 既不表示撤销也不表示任务成功。
- 未收敛的 owner 凭据仍阻止 Session 构造、终止证明和启动恢复。
  失败终止也不能通过残留 system response 再触发 continuation。

本保护针对新的平台逐工具 MCP 路径。旧 native MCP、MiniApp、Robot 和非工具 lifecycle
没有因此获得同等的持久效果证明，不将此切片描述为所有外部效果恢复完成。

## Coding：效果后的指令刷新

原有工具调用前目录检查之外，增加效果后的模型边界刷新：

- 记录本 turn 已检查的目录，包括当时没有指令文件的目录。
- 修改、失败的潜在修改、进程观察，以及与运行中进程重叠的工具结果使指令视图失效；
  下一次模型推理、压缩、规划和完成报告之前，重新读取已知作用域。
- 实际内容变化会使旧 completion report 失效，设置 needs_replan，清除 provider
  continuation parent，并写入 InstructionsUpdated / PlanUpdated 事件。
- 一次刷新内，共享祖先和文件不存在的结果使用 bounded 缓存；跨刷新不复用。
  目录最多 64 个，缓存最多 4096 项 / 256 KiB，仍受既有读取和上下文预算限制。
- 初始已授权的指令读取失败或截断不再让模型基于半份规则继续。
  没有 fs.read 授权时仍不隐式增权，也不退回引擎原生文件系统读取。
- 空 AGENTS.md 不再终止子目录发现；存在但为空的 AGENTS.override.md 遮蔽同目录
  AGENTS.md，仍继续查找下层。展示顺序显式按深度再按路径排序，拒绝控制字符路径。

参考了本地 Codex `codex-rs/core/src/agents_md.rs` 的 ancestor chain、存在性优先的
override、空内容跳过及受限文件系统读取设计。没有复制其原生文件访问绕过平台授权。
前序 retry 设计参考 `streamable_http_retry.rs` 和 `responses_retry.rs` 对握手、模型请求
和工具效果重放的区分；并未把 HTTP 重试通用化为工具事务可重放。

## 尚未完成，不以本切片关闭整体目标

- 多 MCP 服务器需要 typed resource / Kernel cardinality 设计，不能只移除数量检查。
- stdio、legacy SSE、MCP resources 及 server-initiated 生命周期仍未接入新路径。
- MiniApp/Robot/非 function Plugin lifecycle 的 retained effects 与持久凭据仍待补。
- 跨启动未知进程树、人工隔离解决、可证明安全的 checkpoint continuation 仍待完善。
- Git push 的完整权限/外部效果证明、非文本/非 artifact Skills 仍待完善。
- 本切片刷新已知目录，不解析任意 shell 动态路径、不遍历所有递归修改子目录，
  也不提供 symlink 目标指令作用域证明；运行中进程/外部编辑的原子文件快照问题未解决。
- 完成报告仍校验引用和时效，不是用户需求语义覆盖或测试成功的独立证明。
- 所有新增实现的编译、迁移、consumer/upgrade/真实 Provider/跨平台验证均未运行，
  原因是用户明确排除验证工作；历史通过记录不能覆盖当前改动。
