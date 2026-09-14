# Coding：按需能力与流事件边界（2026-09-13）

状态：本地代码已接线，未运行验证；整体目标仍进行中。
分支：`rf/agent-capability-platform-v2`。未 commit、未 push。
本切片 build 后缀为 `host2-coding-loop4`，旧 Session 不自动迁移或回退。

## 1. 本轮补齐的能力

- 增加 `search_capabilities` / `activate_capability` 两个 engine 控制工具。
  搜索只读取 Agent 已锁定且未激活的能力索引，不激活、不安装、不访问全局目录。
  激活必须单独提交，混合批次不执行任何工具；依赖束只能来自 Snapshot 已编译计划。
- 工具 Schema 从同一锁定 Snapshot 的完整选择集合预编译，在内存中按 Kernel 活跃集合
  过滤后才提供给模型。这个预编译视图不改变 Kernel 权限，也不会把所有能力自动激活。
- 平台沿用 `SessionCapabilityState`，不建立 engine 私有权限注册表。
  激活检查 accepted root、owner、admission epoch、模型 operation 领取记录和活跃代次；
  等待已拥有的工具任务结束，并检查实际 process owner 是否持有未清理进程。
- 在原 `conversation_runtime_events` 写入 `capabilities_activated` 后才应用内存代次。
  持久化结果不确定时保留失败标志，拒绝后续模型调用、下一轮准备及成功状态投影；
  必须重新构造 runtime，从真实日志恢复，不能推断“没有激活”。
- 正常重开同一 Session 时，按日志插入顺序恢复连续代次，逐项核对 exact engine build、
  Session、Snapshot、根 turn 和依赖束。仅恢复活跃集合，不调用任何工具动作。
  新 Fork 是新 Session，本切片不会把父 Session 的活跃集合私自复制为其权限状态。
- 激活后刷新下一次模型请求的 ToolPlan，清除旧 provider continuation parent，
  同步 AGENTS 读取使用的权限代次。新激活 `fs.read` 时，在后续推理前读取根仓库指令。
  失败或不完整的指令读取不宣称成功。图片输入仍要求在本轮准备时已激活视觉能力。

这仍遵守 Agent 工作台 → immutable Revision → exact Session build 的配置归属。
能力激活不是执行引擎切换，也不是打包后挂载 engine 或 Plugin。

## 2. 资源限制与流协议

- 模型主循环按整个序列化事件统计每轮最多 8 MiB，包括 ID、供应商元数据、usage、
  文本、参数、推理与签名，不再仅统计内容字段。计数使用有界 writer，不额外构造
  同样大小的序列化副本；每轮最多 65,536 个事件。
- 模型工具 ID 最多 256 字节，工具名最多 128 字节，provider metadata 最多 16 KiB；
  provider round ID 最多 4096 字节，单条 usage 最多 16 KiB。自动指令读取的 ID
  使用独立保留前缀与递增序号，不能由模型冒充，也不再包含整条路径。
- 每次压缩请求也使用完整流事件预算；丢弃的推理或重复 usage 不能无限绕过限制。
- 每个 Coding Agent 最多选择 128 个 capability，完整工具面最多 128 个 action。
  搜索 query 最多 512 字节，limit 1～16，结果索引不超过 24 KiB；索引字段另有限制。
- 工作区指令路径限制 4096 字节；修复目录名含 `AGENTS.md` 时错误替换整个路径的问题，
  现在只替换叶子文件名为 `AGENTS.override.md`。

这些是宿主/engine 的资源保护，不等于 provider tokenizer，也不构成性能或正确性验收。

## 3. Codex 源码借鉴

继续只读参考 `multi/codex` 中：

- `codex-rs/core/src/tools/handlers/tool_search.rs`
- `codex-rs/core/src/tools/handlers/tool_search_spec.rs`

采用“发现延迟工具，再更新下一次模型调用工具面”的思路；NomiFun 将搜索和激活分离，
并接回自身 Snapshot/Kernel/Conversation 体系。当前搜索沿用平台的有界文本匹配，
没有移植 Codex 的 BM25、动态插件安装或原生 provider tool-search 协议。

## 4. 尚未完成的工作

- MCP exact-lock / schema / 实际 owner 在当前生产数据库上的接线；MiniApps 及
  非普通 function-tool 的 Plugin 生命周期。
- 用户 steer / follow-up 在授权安全边界进入当前循环：现有同步 `steer(text)` 接口
  不能单独作为已持久化输入证明。需要将 receipt/turn 身份传到 runtime，并处理完成竞态。
- 未有清理证明的跨进程树重启恢复、人工隔离处置、checkpoint 安全续跑；
  未审计 Plugin owner 的异常重启仍不自动解封。
- Git push 的凭据 owner 与崩溃持久外部副作用回执；不能为了暴露按钮直接放开。
- 通用 host SDK、成功执行的社区 engine 参考实现、非制品 Skill 兼容、分页资源、
  shell 动态路径/递归/符号链接的指令覆盖。
- 平台消费者、真实模型、升级及跨平台验证。没有证据表明本版本优于 Nomi 或匹配 Codex。

按用户要求，没有运行构建、测试、模型评测、桌面 E2E 或全量检查。只进行了实现、源码
阅读和新/小模块的定向格式化；上述机制仍须后续验证，不能据此标记整体完成。
