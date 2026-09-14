# Coding 未暴露工具调用的纠正循环（slice82）

2026-09-14，rf/agent-capability-platform-v2 本地工作树。
Coding host2-coding-loop82；Nomi 保持 host53。源码实现，未运行验证。

## 缺口与参考

原 turn.rs 在普通工具分发时对未知名称返回 ToolNotExposed，直接终止整个 Coding turn。
模型无法读取工具错误来纠正拼写或失效名称。可选内部控制分支也未统一检查本轮实际
发送的工具定义；即使某个控制未暴露，仍可能进入其处理器后才被拒绝。

固定 Codex 基线 6af345407d9c2a568da9d01b6c4b81a9e61495c0 的
codex-rs/core/src/tools/registry.rs 在找不到工具时返回 RespondToModel，区分可纠正的
调用错误与底层执行失败。本轮采用这一分层；整批不分发是 NomiFun 的明确策略，
不声称 Codex 也采用相同批处理规则。

## 实现

- tool.rs::reject_unexposed_batch 对照本轮实际发送的 model_request.input.tools，
  包含按可用性配置的引擎内部控制；不查询完整平台目录来给缺失名称补权限。
- 只要一项名称不在本轮定义中，整批逐项返回 is_error 工具结果。未暴露调用指出
  不可用名称；其他调用明确说明本批未执行。要求按现有定义/schema 纠正并使用新 ID。
  不返回隐藏工具目录、不解析别名、不自动激活能力、不推测用户额外授权。
- 预检位于所有内部控制、仓库指令发现、Kernel/tool admission 和真实调用之前。
  因此“本批未执行”只描述本批模型调用，不否定早先批次或模型前的已有平台工作。
  原副作用/进程/权限 owner 和活动代次校验不变。
- 结果沿用 ToolCompleted、归档、工具结果上下文和已存在的正常重放路径，保留全部
  call/result 配对及模型顺序。平台工具没有 ToolStarted，dispatch.attempted 仍为 false，
  不产生成功观察或修改 workspace epoch。引擎内部/未知调用计入失败观察而非平台证据。
- 原完成报告失效、计划标记 needs_replan；下一模型边界看到错误和计划状态，再由模型
  纠正或报告无法继续。仍受原 max_model_steps、重复 call ID、完成账目和取消约束。
  不新增传输重放、后台自动调用或无限重试。
- 已暴露但宿主 binding 丢失仍保留内部合同错误；截断/重复 ID、参数增量不一致、
  非对象/非法 JSON 等协议问题不在本轮放宽为可执行输入。

## 边界与证据

现有 closed-turn 重放允许没有 ToolStarted 的错误工具结果，这也是既有排队/控制拒绝
使用的路径；本轮没有更改恢复判据、事件格式或数据库 schema。
没有运行未知名称、未知+写入混合批、禁用内部控制、取消、纠正后执行、日志重放或
真实模型回归。仅做源代码衔接和局部格式化，不能声称任务成功率已经提高。
Nomi 不消费本次 Coding 分发策略，因此不改变 Nomi 构建标识；Coding 摘要已包含
turn.rs/tool.rs。旧 Session 不自动改绑，没有 commit/push，也未构建、测试、启动服务或迁移。
