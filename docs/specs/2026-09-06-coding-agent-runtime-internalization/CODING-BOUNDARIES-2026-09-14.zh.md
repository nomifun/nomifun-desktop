# Coding：工具准入、资源分页与命令证据

日期：2026-09-14。分支：`rf/agent-capability-platform-v2`。
本切片仅完成下列源码接线；未构建、未测试、未评测、未提交、未 push，整体 CAR 仍在进行中。
生产 Coding build 后缀更新为 `host2-coding-loop6`；旧 Session 不自动迁移。

## 1. 追加输入与工具准入的原子边界

此前串行调用先查询收件箱，再异步记录 `ToolStarted`，两者之间仍可接受新输入。
现增加 `CodingEventSink::admit_tool` → `CodingRuntimeHost::admit_tool`，生产宿主在同一
active-turn 锁内核对消息根、wire turn、取消/清理状态、追加输入并持久化准入记录。

- 输入先入队：返回 false；不写 `ToolStarted`，不调用工具，闭合原调用/结果对并延后重规划。
- 工具先准入：日志先落库；后到输入不能撤销已准入调用，在后续模型边界处理。
- 串行调用在拒绝准入后延后该批剩余调用；只读并行调用逐个准入，不再整批预记开始。
- 锁不覆盖工具执行；仍由原平台 owner 持有、等待及清理在途副作用。
- 准入日志写入失败会关闭收件门并取消该 turn，不能当作普通工具错误后继续执行。
- UI 投影只在准入成功后发出；不重复持久化。正数 step 的 `ToolStarted` 不能走普通 emit 绕过。
- step 0 的授权 AGENTS 只读发现仍走原记录路径；不因此增加权限。

默认 sink/host 方法仅适用于没有并发输入 owner 的宿主。支持 steering 的二次开发宿主必须
覆写条件准入并使用同一同步边界；这是受信任代码契约，不是 Rust trait 自动提供的沙箱。
该边界不承诺“回执成功即模型已经读取”，也不是跨崩溃 exactly-once 执行证明。

## 2. 不可变 Skill 资源分页

`read_context_resource` 接受 `id`、可选 `offset` 和 `limit`：

- offset 是 UTF-8 字节偏移，必须落在字符边界；从 0 开始，续读使用返回的 `next_offset`。
- limit 为 4～16384 字节，默认 8192；响应含 `offset/end_offset/total_bytes/next_offset/eof`。
- 按字符边界缩小页，并将响应作为工具文本的二次 JSON 编码控制在 24 KiB 内，为宿主
  32 KiB 结果日志上限留空间；因此实际页可小于请求 limit，但不会隐式丢弃中间文字。
- EOF 时 next_offset 为 null；空资源也有明确 EOF，不返回无限不前进的游标。
- 资源单文件上限由 16 KiB 提高到 256 KiB；最多 64 个、合计 512 KiB 的冻结资源预算不变。
- Skill 指令正文仍为单文件 16 KiB、组合上下文 24 KiB。大参考资料不能挤占指令正文额度。

宿主按剩余总预算限制读取，仍校验制品清单、路径约束、文件大小、内容摘要和 UTF-8。
读取的是已冻结内存资源，不是执行期间按任意路径加载；脚本仍只是文本，不提供二进制资源、
运行脚本、安装扩展或打包后挂载 engine 的能力。

## 3. 命令结果与变更时序

修正 `CodingWorkStatus` 将进程工具响应粗略计作命令结果的问题：

- running、stdin/resize 的普通响应、调度失败、steering 延后不是已成功退出的命令。
- 仅明确 exited、exit code 0、清理证明成立且工具未报错时增加成功命令计数。
- cancelled/timed_out/lost/非零退出或缺清理证明不计成功；重复读取同一终态不重复计数。
- 未证明清理的终态仍保留在待清理进程集合，不能以“看见终态名称”证明资源已退出。
- 每次可能的修改、命令启动和 stdin 推进保守的 workspace observation epoch。
  失败或延后的修改也可令旧证据失效，不假定错误等于完全没有副作用。
- 命令启动后发生新修改，或它与另一进程重叠时，稍后的退出结果不能建立最新工作区的观察。
- 保留最近 16 个命令观察及省略数量；引用启动和结果 call ID，不重复存储参数、环境和输出。
- 完成复核后出现新的修改、命令终态或失败，重新允许复核；总模型步数预算仍保持不变。

这不是文件版本证明、测试分类器或任务完成判定器。退出码 0 不等于测试通过；模型还须核对
原调用、真实输出与用户任务。用户禁止验证时仍不能执行验证。当前没有证据证明 Coding 优于
Nomi 或与 Codex 等价。

## 4. 本轮 MCP 接线结论及剩余工作

源码定位确认：`agent_wave2_mcp.rs::SqliteMcpRuntimeBindingSource` 读取 Fresh-v4 的
`server_id/owner_user_id/connection_config_ref`、`mcp_tool_materializations` 和 package config；
当前 Conversation 产品链使用 v3 `mcp_server_id/transport_config/tools/updated_at`，并由
`nomi_core_session.rs::exact_session_mcp_selection` 核对资源绑定。两者不是同一个目录投影。
本切片没有放开 MCP 白名单，也没有退回旧 GenericMcpToolProxy 绕过 Coding 的 Kernel 准入。

后续仍需完成：

1. MCP 当前产品目录的精确 mapping/schema/connection 投影、凭据 owner 和执行/清理链路；
   MiniApps 与非 function Plugin 生命周期。
2. Git push 的凭据与崩溃持久副作用回执；进程跨启动清理证明、人工隔离处理、安全续跑。
3. 公共 model/tool/history/state 宿主 SDK 与实际第三方 engine 示例；非制品 Skill 库和二进制资源。
4. 动态 shell/递归/符号链接指令范围，以及更强的任务特定规划和完成依据。
5. 用户允许后再运行定向验证、真实 provider、消费者继承与升级/多平台验收。

本轮参考既定 Codex 基线 `6af345407d9c2a568da9d01b6c4b81a9e61495c0` 的
`codex-rs/core/src/session/turn.rs` 中 pending input 在模型边界进入历史的机制；
原子持久准入与制品分页为适配 nomifun 平台 owner/不可变制品约束的本地实现，并非照搬上游。
