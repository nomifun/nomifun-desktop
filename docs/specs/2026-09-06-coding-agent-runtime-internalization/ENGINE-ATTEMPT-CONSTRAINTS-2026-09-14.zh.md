# 协作 Attempt 的任务输入与跨 Engine 约束（未验证）

本切片属于 CAR-07。仅本地 `rf/agent-capability-platform-v2`，未 commit/push。
当前 Coding `host2-coding-loop40`，Nomi `host23`；未运行构建、测试、E2E、服务调用或迁移。

## 修复的接入缺口

上一切片保留了 Agent canonical Binding，但 Attempt 仍把 brief 放进 system_prompt，
再用 Nomi 的 Read/Grep/Glob/Bash 名称覆盖 allowed_tools。这会与 immutable Agent
投影冲突，也不能限制采用另一组工具名的 Coding/社区 Engine。

现在 canonical Attempt 不再覆盖 Agent 的 persona、固定指令、Skill 或工具投影。
brief 与 step_spec 编码为同一条持久化初始 user 输入的两个 JSON 字段，沿既有
creation key、execution link、accepted message 和 delivery receipt 执行。它们不是新的
system 指令或执行授权；旧无 canonical Binding 的 Attempt 保留旧路径，不猜测迁移。

## 平台约束

`execution_constraints` 是带版本的后台 Session metadata，独立于 Agent/Snapshot/Engine。
范围值复用已有 AgentToolPolicy，不另建一套执行角色或 Engine-specific 权限枚举。
仅 canonical snapshot 的可信创建路径可保存；公共 create/update 不可写。创建键重试在
应用预检查和数据库仲裁后均比较该值；共享资源上下文与 accepted-turn receipt 检查保持一致。
它只能缩小已保存 Agent 的权限，不给没有选择 fs.read/process.exec 的 Agent 增权。

| 工具范围 | 允许的已选能力 | 明确不意味着 |
|---|---|---|
| Full | Agent 已选工具；深度上限可另行排除 agent.delegate | 平台外任意授权 |
| ReadOnly | 平台内置 fs.read / fs.search；固定 Skill 数据和已选图像上下文仍可用 | OS 级只读沙箱、禁止平台日志写入 |
| ReadShell | ReadOnly 加平台 process.exec | 只读命令；Shell 可以产生写入或外部效果 |

受限范围不接受第三方自报 read-only 的 MCP/Plugin/MiniApp/Robot 工具。
`llm.chat` / `llm.vision` 是上下文能力例外，不由此增加可执行工具。

### Coding / 源码集成 Engine

- `EngineKernelSession::compile_tool_plan` 先核对原始 schema/动作/资源映射，再过滤范围。
- Coding 将已有工具映射再次交给共享宿主过滤后安装，避免受限任务因携带完整表而整体失败。
- install 再核对过滤后映射，实际派发 wrapper 在 media/Robot/MiniApp adapter 之前检查
  完整 frozen binding；调用者换名、伪造 capability 或 effect class 不能借此调用工具。
- Coding search 仅返回允许的按需能力；activate 同时检查依赖 bundle。
  即便第三方 Engine 改变自身 active set，实际端口仍受已安装工具映射限制。
- 受限 Session 不解析 Robot/MiniApp 工具、不获取 Robot vision context；只读不打开 process scope。
- mandatory 已有效果恢复观察与清理保留，不因权限收窄而隐藏历史未知效果。

### Nomi

- 从 owner 读取约束，在 materialize 时排除越界 Kernel tools；受限任务不初始化一般
  ContextContributor / lifecycle，因此不是先启动外部资源再隐藏工具。
- 不解析受限 MiniApp/Robot tools；后续 host dynamic/MiniApp 注入同样拒绝越界。
- 原生工具策略先求交，动态表扩展后再求交；空集合仍 enforce allowlist。
  ToolSearch、后续注册或 Gateway 的限定来源工具名不能重新加入被排除的委派入口。
- 受限任务关闭原生 MCP discovery/resource/proxy、browser/computer 及 knowledge/companion
  配置；清除本地配置里可能残留的 MCP server 与 automation enable 标记。
- ReadShell 覆盖 Bash、exec_command、write_stdin；不将不同别名误当不同权限。

## Codex 借鉴与未覆盖边界

本次继续阅读本机 Codex 的 `codex-rs/core/src/tools/orchestrator.rs` 与 `registry.rs`：
借鉴执行前集中权限检查、工具展示与真实执行分离的边界；没有复制其整个运行时，
也没有声称已具备其 approval/network-policy/OS sandbox 能力。

源码集成 Engine 是随产品一起编译的可信代码。Rust trait/工具表约束不是针对恶意 native
代码的隔离层；打包后仍不允许挂载 Engine。Shell 也不是无法绕过平台 API 的安全容器。
受限 Nomi 暂不消费一般动态上下文；当前实现明确牺牲这部分上下文能力以保留约束。

本切片不完成持久 MCP resources/server-initiated 生命周期、非工具 MiniApp 生命周期、
旧消费者 provenance 迁移、跨 Session/Fork 未知效果协调和人工解除，以及真实端到端验证。
因此仍是 source-implemented / unvalidated，不标记整体 CAR 完成。
