# Coding 扩展输入与上下文资源接入

状态：implementation-in-progress，新增实现未验证。工作分支为
`rf/agent-capability-platform-v2`；无新提交、无 push。本轮不运行构建、测试、
模型评测或桌面 E2E，遵循用户此前排除验证工作的要求。

## 归属不变

Engine 在 Agent 工作台选择，保存到不可变 Agent revision；Session 锁定 exact build。
官方 Nomi/Coding 与社区 Engine 都经编译、打包、启动注册，打包后不能挂载 Engine。
下面的 Plugin/Skill 支持是消费平台已经批准且锁定的能力制品，**不是动态加载 Engine**。
Coding 负责执行循环、规划和上下文策略；平台继续负责权限、凭据、资源和持久化事实。

本次 build 后缀为 `host2-coding-loop3`。新增模块进入 build digest；旧 build 的 Session
不静默迁移。本轮没有实现历史构建打包保留或升级迁移。

## 本轮补入的实现

| 范围 | 代码行为 | 边界 |
|---|---|---|
| Plugin 工具 | 仅投影 Snapshot 中初始已激活、锁定 PluginMount 的 Tool actions；schema 读取 exact artifact，调用仍经 Kernel | 不扫描未选择插件；不接入 Plugin 生命周期类型、Hidden/CodeMode actions；最多 128 actions |
| Skills | 核对编译时 registry、Skill version/body digest、制品清单和内容 hash；正文加入独立上下文 | 当前仅制品内 Skill，依赖必须初始激活；不读取任意工作区 SKILL.md 或旧全局 Skill 库 |
| Skill 资源 | `read_context_resource` 按 exact id 读取已锁定的参考文档、模板、示例、脚本文本 | 这是引擎内上下文读取，不是新 Kernel 权限；不会执行脚本、访问任意路径或下载资源 |
| 附件 | 文件引用与 accepted delivery receipt 对比；非图片显示为数据路径，图片复用 Nomi 的安全解码器 | 图片需要初始 `llm.vision`，Broker 再检查 ImageInput；使用 Session 的 write_root 限制；桌面沿用用户选择绝对文件的既有信任模型 |
| 多模态压缩 | 传输字节与模型成本分开估计；压缩只看媒体描述，不把 base64 送给摘要模型 | 每张缩放图暂按 4096 token 保守预留，并用 usage 校正；不是准确 tokenizer；当前请求图片保留，历史不重新读盘 |
| 事件日志 | text delta 合并；不持久化推理/参数 delta；工具结果采用有限正文或明确截断摘要 | live 模型工具结果不被历史摘要替换；媒体二进制不进入回放；已正常记录的结果不重复写 host settlement |
| 指令隐私 | 自动 AGENTS 读取、显式 read_file 的 AGENTS 结果及 InstructionsUpdated 持久化时改为摘要/标记 | 当前规则正文留在 turn 上下文；UI 工具结果同样不保存正文；不是对任意 shell 输出或模型复述的通用敏感信息过滤器 |
| 快照 | `fs.snapshot` 接入 Coding；Wave2 为每个 Session/root 建独立服务和唯一临时仓库命名空间 | Git 项目基线为 HEAD，非 Git 为临时基线；baseline 操作只读内容、不恢复文件；runtime teardown/restart 后不承诺继续保留 |
| 清理 | Session teardown 清理快照并释放本 Session 的 Kernel resource scope | resource release 使用可重复等待的同一完成结果；失败/超时不以空映射作为成功证据 |

资源界限：最多 16 个 Skill，正文单文件 16 KiB、合计 24 KiB；参考资源单文件
16 KiB、最多 64 项/合计 512 KiB。制品文本必须是 UTF-8。超限明确拒绝，不悄悄截断
Skill 规则。资源目录保留在上下文，资源正文只在被读取后进入模型消息。

日志界限：普通执行记录最多 3200 条/4 MiB；清理与终态可使用预留区，总计不超过
4095 条/8 MiB，为恢复补写终态再留一条。工具持久化投影目标为每项 32 KiB。
历史候选为最近 32 轮、合计 16 MiB，超出时剔除较旧完整轮次，不因长期累计直接封死
下一轮。日志不足会在继续准入工具前中止，不通过丢弃 ToolStarted 来绕过证据要求。

## 借鉴 Codex 的部分

读取本地参考项目 `multi/codex` 的 `core/src/context_manager/history.rs`、
`context_manager/normalize.rs` 和 `compact.rs`：复用“媒体载荷不是文本 token”、
“不支持或未保留的媒体必须显式标记”、以及“压缩后的事实不能伪装成新授权”的思路。
没有增加 Codex path dependency、sidecar 或独立 provider client；没有复制新的产品宿主。
未做对比评测，不能据此声称 Coding 已优于 Nomi 或达到 Codex 的成熟度。

## 尚未关闭的工作

- 按需能力的搜索/激活、活动工具面刷新和跨重启恢复。
- MCP 的 exact lock/schema/owner 生产接线与清理证明；MiniApps 和 Plugin 生命周期类型。
- 当前制品以外的 Skill 库兼容，以及非 UTF-8/更大 Skill 资源的分页读取策略。
- Git push 的凭据边界和跨重启副作用证明；当前不向 Coding 暴露该工具。
- 中断命令没有进程树清理证明时仍隔离；未知 Plugin owner 同样不自动解封。
- Steer/follow-up 安全边界、checkpoint 续行、共享 host SDK/第三方完整参考实现。
- Shell 动态路径、递归与符号链接指令范围，以及所有此前排除的验证工作。

关联任务：CAR-03（工具投影/准入）、CAR-05（workspace snapshot）、CAR-06（附件、
Skill 资源、压缩和日志）、CAR-07（生产组装和资源 teardown）。均不因本轮写入代码而
升级为已验收完成；历史通过的测试不覆盖当前新增实现。
