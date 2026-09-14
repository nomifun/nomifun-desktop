# 消费者冻结 Binding 与 Engine 继承（2026-09-14，未验证）

## 源码发现

Remote 已使用统一 `create_session_idempotent`，随后通过同一 Conversation owner 和
Runtime Registry 发送，不需要另一套 Remote Engine。Cron 的默认应用适配却直接调用
ConversationService；更重要的是，Cron Agent resolver 从保存的 Binding/revision/snapshot
生成 `AgentResolvedSnapshot` 时只保留展示、模型与能力投影，丢失了完整 Binding。
这使定时任务创建的 Session 无法从冻结 Agent 恢复其 Engine 选择。

Channel/AgentExecution 的创建适配同样直达 ConversationService。Channel 当前专用聊天
创建仍是没有 canonical Agent 的旧路径，不能因为函数被统一就宣称新增了产品选择能力。

## 本轮写入

- `AgentResolvedSnapshot` 新增可缺省 `canonical_binding`，保存完整 revision ref、
  resolved snapshot ref 和 typed resource binding。它不是 Engine selector，也不是
  可脱离保存 artifacts 单独使用的授权。旧 JSON 缺字段仍可读，None 不序列化。
- canonical Agent 纯投影保留这个引用；Cron resolver 原有冻结快照持久化链因此可以
  传递引用，不在 scheduled turn 重新读取可变的 Agent 编辑状态。现有 renderer mapper
  保留快照字段，TypeScript 契约补充对应可选字段。
- 默认 Cron、Channel 和 AgentExecution 创建适配统一进入应用 Session owner。
  携带 canonical Binding、尚未带宿主 metadata 的消费者创建请求，由宿主查询保存的
  artifacts、核对 owner/Binding 以及完整投影，补齐宿主 metadata 和 canonical extra。
  冲突的 Agent 类型、投影字段或 MCP 选择不被静默改写。
- Engine 从保存的 Agent revision 的 selector 解析，随后执行该 Engine 注册的 Snapshot
  和 Session overlay 准入。消费者不能额外指定 Engine，缺少 Engine host 时明确失败。
  初始 MCP ID 仅来自冻结 Binding；这不绕过 Kernel/资源/目录的后续准入。
- 同一创建键重试先读取已经创建的 Session 的 exact Engine；channel 改变不影响这次
  重试，Fork 仍继承父 exact binding。owner/Agent Binding 或显式 Engine 不一致则冲突。
  并发创建仍由 Conversation repository 仲裁，底层在复用行时同时核对 Engine 与不可变
  Session metadata，堵住“相同 Engine、不同 Agent/资源”的竞争窗口。
- ConversationService 对带 canonical Binding 的快照要求已准入 metadata 与 exact Engine，
  防止遗漏的消费者从底层直接创建而回退到 Nomi。没有新增 SessionStore、身份映射表或
  Engine 动态注册入口。

## 未完成边界

1. **旧快照不自动迁移。** 没有 canonical_binding 的已有 Cron/历史快照仍属旧路径；
   无法从旧展示字段证明完整资源 Binding，不能根据当前可变 Agent 猜测或自动升级。
2. **AgentExecution 约束仍需适配。** 协作 Attempt 目前以旧 Nomi Read/Grep/Glob/Bash
   名称表达工具限制，brief 覆盖 system_prompt。统一创建入口并不使这套限制天然适用于
   Coding/社区 Engine；与 canonical projection 冲突时现在失败，不能忽略限制或冒称
   已有完整的多 Engine 协作支持。下一步需要平台级工具约束和独立任务上下文处理。
3. Channel 专用旧聊天没有新增 Agent 选择 UI；显式绑定现有 Session 时继续由其持久
   Engine 决定运行。其他消费者全部端到端继承仍需逐项覆盖。
4. Cron 注入的非冻结/未选择 Skill 仍不能变成 canonical Engine 的额外授权；已有打包
   Skill 与 on-demand 准入边界保持，不把本次 Binding 传递视为新资源接入。
5. 创建前查询不是新的事务锁；冲突返回不授权自动重建或重放工具。Engine 缺失仍失败，
   不回退到默认 Engine。尚未做应用升级与旧任务兼容性执行验证。

## 状态

Coding `host2-coding-loop39` / Nomi `host22`，摘要补充消费者投影和公共 Snapshot 契约。
新旧 JSON 可缺省设计不需要新数据库迁移；本轮未执行任何迁移。仅源码阅读和本地补丁，
按用户要求未运行构建、测试、计划任务、Remote 请求、外部服务或模型调用。
结构体测试 fixture 仅补齐新可选字段，没有运行或宣称通过。

分支保持 `rf/agent-capability-platform-v2`，没有 commit/push。整体 CAR/多 Engine 全能力
目标仍未完成；本切片证明的是源码接线，不是实际运行证据。
