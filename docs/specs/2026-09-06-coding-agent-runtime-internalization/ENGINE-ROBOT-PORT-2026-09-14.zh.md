# 共享 Robot 端口与动态上下文（2026-09-14，未验证）

本切片属于 CAR-05 / CAR-06 / CAR-07。只在 `rf/agent-capability-platform-v2`
本地修改；没有 commit、push、构建、测试、外部设备调用或数据库迁移执行。
当前源码构建标识为 Coding `host2-coding-loop30`、Nomi `host15`。
这不是整体 CAR 完成声明，也不是硬件运行或多平台质量证明。

## 已写入源码的能力

| 能力 | 平台归属与 engine 消费方式 |
|---|---|
| robot.display / motion / device_tools | 精确设备工具目录、原始 schema 冻结、严格参数校验、串行 retained Tool 调用、双层效果回执 |
| robot.link / audio | 只有已激活的选中权限才能取得本地 lease；不是模型 function tool，也不会连接默认设备 |
| robot.vision | 读取平台已产生的近期文字观察；独立 context 端口，不主动拍照、不增加模型调用 |

公共 `EngineKernelSession.robot_tool_plan()` 与 `install_tools` 连接真实 Robot owner。
完整工具计划包含已选择的初始和按需能力，但它只是预览；实际调用仍检查 live active set、
generation、principal、Session、Snapshot、action、资源策略及 host 冻结 plan。
Coding 已合并该计划，社区源码集成 engine 也可消费；不复用 Coding 的规划/循环策略。

工具 Agent 必须显式选择 `robot.link`。选择它不等于激活：若它是按需能力，模型需要先请求
平台激活。音频工具还要求 active `robot.audio`。既有 Kernel 的 durable activation/restore
仍是唯一 active-set authority；新端口不自动增加能力或修改 Agent revision。
设备工具目录最多 128 项，合并后的 Coding 工具面仍最多 128 项。

## 生命周期与冻结边界

- Nomi 的生命周期 activate 阶段只检查 readiness，不再注册无 owner 的长期授权；取得
  context lease 时，在最后一次 await 之后同步完成注册及返回所有权。
- 同一 owner/Session/capability 重复获取共享一个 Arc lease，最后一个持有者释放才撤销。
  Weak 缓存不保活 Session；清理用原子 compare/remove，旧 lease 不会移除新一代授权。
- Coding 的 lease 在 retained Tool 内懒取得并保留到 Session 清理；清理显式关闭，外部
  仍持有 ToolHost Arc 也不能重新打开 Session。失败清理同样尝试撤销本地授权。
- 工具调用同时冻结 device name 和设备原始 input schema。重连后在选择 live client 的
  同一 registry read lock 内比较；provider hardening 后的 schema 用于模型参数验证。
  JSON Schema 的外部资源读取被禁用。

这些 lease 目前只是本地调用权限，**不是物理资源已静止、音频已播放完或共享设备已断开**。
其他 Plugin/Service 初始化、BackgroundService 和非工具生命周期不能据此宣称全面解决。

## 动态上下文与 Coding 策略

公共 `robot_vision_context(generation)` 要求 open turn、精确 active generation、冻结的
platform contribution/资源/context schema；异步读取后再次检查回合和 generation。
实际 owner 检查当前设备/Companion 关系，观察沿用 5 分钟有效期。
观察源 question/answer 分别限 1024/8192 UTF-8 字节，截断有可见标记；JSON 上下文超过
16 KiB 明确拒绝，不静默截断成错误 JSON。没有新鲜观察时返回 None。

Coding 新增可选 `CodingLiveContextPort`，与 immutable Skill resource 分开：

1. 初始压缩前和每次后续模型循环前读取当前观察，宿主核对 accepted turn/epoch/root。
2. 固定上下文槽替换旧观察，不追加无限历史；按需激活后的下一轮立即刷新。
3. 先刷新再执行上下文预算/compaction；port 有取消、超时和字节预算。
4. 观察变化或过期清除 provider continuation parent，并使旧 completion review 失效。

文字观察始终是不可信数据，不是用户指令，也不是 Coding 的测试证据或任务完成证明。
这里借鉴本地 Codex `codex-rs/core/src/tools/parallel.rs` 的执行上下文保留和串/并行边界：
冻结工具面归属于调用的上下文，资源 owner 不因等待者取消而被误认为完成；没有直接套用
Codex 的 abort-on-drop 到物理效果执行上。

## 回执与重启

真实 Robot adapter 继续在设备调用之前写 `conversation_hosted_effects`，实际 Robot owner
保留原有物理效果 ledger。公共派发日志补充冻结 `model_name`，不保存原始参数。
Coding 恢复核对 canonical action、call/operation、设备工具名及同 turn/epoch 的
returned/rejected owner 回执。缺失、重复、pending、错配都保守隔离；不自动重放。

有回执只允许结束旧的宿主请求历史；returned 不证明命令的长期物理后果已经停止。
生命周期尚未就绪而被拒绝的请求若没有 hosted 回执，重启仍可能保守隔离；没有把日志文本
当作无副作用证明。人工解隔离、物理停止证明与 checkpoint 续跑仍未完成。

## 仍需后续处理

本切片之外的非工具生态生命周期、MCP 其他 transport/resources/交互、安全 checkpoint、
异常进程树跨启动证明、动态路径指令范围、语义完成核对及消费者继承仍有未完成项。
按用户要求未运行验证，因此本切片不提供编译成功、测试通过或真实设备运行结论。
