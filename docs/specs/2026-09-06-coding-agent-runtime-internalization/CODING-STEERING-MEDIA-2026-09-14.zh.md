# Coding 追加输入的附件与冻结 Skill 提示

日期：2026-09-14。分支：`rf/agent-capability-platform-v2`。
本切片只有源码实现与局部格式化，未运行构建、测试、E2E、服务调用或数据库迁移。
未 commit/push；不能据此宣称 Coding 已优于 Codex，或 CAR 已完成。

## 接入结果

- Conversation 原先统一拒绝 steer.files；现在新请求先做有界校验，再通过当前
  Engine 的 `supports_steering_context` 判断支持。已有永久回执的重放不要求旧引擎
  仍存活，也不因本次新输入限额而失去幂等回放。IDMM 保留无附件的原限制。
- Registered Runtime 与共享 EngineSessionDriver 默认不支持上下文追加；默认文本
  适配器显式拒绝非文字字段。社区引擎可在源码接入时实现该能力，不必修改产品调用方。
- 内部 delivery 携带 files/inject_skills；Coding 从永久 receipt 重新解析并逐字段
  比较。Skill ID 必须已在当前 Agent Snapshot 锁定，提示不是新选择、动态加载或授权。
- 附件引用不是文件内容证据；普通文件仍需经已选工具读取。图片复用初始输入的
  平台路径/根目录/视觉能力检查与有界解码，不让 Engine 直接读文件。
- 图片准备与追加/工具准入/结束共享回合锁，支持取消与 10 秒准备截止时间；准备后
  再核对持久化回合 epoch/operation。每回合累计追加至多 16 条输入、4 张图片、
  4 MiB 编码图片载荷；超限明确拒绝，不以截断冒充接受。
- 相同 receipt 的重复请求在文件准备前识别，不重读已变化文件，不重复消耗图片预算。
  图片只保留在活跃输入与必保留模型上下文中；批次校验完成后才更新引擎输入账本。

## 历史与交付语义

入队确认只代表入队，既不代表模型看到，也不代表模型遵循或完成任务。
永久事件不序列化图片载荷；正常边界、回合关闭前未消费和意外退出后的交付不确定
分别保留原有观察语义。历史投影保留附件引用和 Skill 提示，明确图片不会重放，
绝不因恢复 Session 而重新读取旧路径或自动重新入队。消息行与 WebSocket 消息也
保留输入的文件引用和 Skill 字段。

产品入口原先 catch 任意 steer 失败后自动改成下一轮，这会在响应丢失时重复执行。
现保留 `requires_review` 草稿并阻止整个命令队列自动发送；标记随队列持久化，
重挂载、重排、普通 resume 不会清除。用户先核对会话，再通过编辑取回草稿决定
是否重新发送，或移除；普通队列条目不受此标记影响。保留机制仍受既有队列容量和
浏览器存储边界约束，不是服务端长期草稿存储或未知效果解除凭据。

## 参考与架构边界

参考本地 Codex `codex-rs/core/src/session/input_queue.rs` 中 typed pending input、
活跃回合锁及接收/消费区分；没有引入其 Session owner 或复制独立附件权限系统。

同时回看 MiniApp 的 frozen capability 与 `m1_application.rs`：当前 Agent 合约
暴露 Service actions，Service 的启动、连续运行和停止仍由平台管理。现有 Agent
action 调用把 Service 运行错误映射为 Runtime，不把运行后错误冒充 Invalid/NotFound
的派发前拒绝。本切片未新增非工具资源合约，也没有另造 Engine 生命周期 owner。

## 版本及剩余工作

Coding `host2-coding-loop41`；Nomi `host24`（共享 Conversation/扩展接口有变化）。
固定构建摘要已经覆盖改动的 Rust 源文件，不修改既有 Session 的 exact binding。
Nomi 本次不获得图片追加支持；它明确拒绝，而不是忽略附件。UI 没有新增 Skill
动态选择入口；API 接受的 Skill 提示仅引用既有固定选择。

MCP resources/获授权的服务端发起交互、MiniApp 额外资源与生命周期效果回执、
未知效果人工处置、跨 Session/Fork 协调和实际运行证据仍未闭环。本切片的历史测试
断言仅按待确认草稿语义更新，未执行，不能作为当前实现的验证结果。

后续 Git 源码调查：已有 Wave2 push owner 仅允许配置好的 local/file remote，
不允许 force，HTTPS/SSH 凭据 authority 尚未接入；超时后 blocking worker 的退出
证明也需要补足。因此本次没有仅把 `vcs.push` 加入 Coding allowlist 来宣称完成，
没有执行任何 Git push。队列容量不足时返回保留失败，不显示草稿已保存的成功提示；
浏览器存储/容量失败后的独立草稿恢复仍是产品边界。
