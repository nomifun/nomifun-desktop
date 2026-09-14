# 平台历史 Engine 的冷/热上下文清理（slice88）

2026-09-14，rf/agent-capability-platform-v2 本地工作树。源码实施及两个小文件
rustfmt；未运行测试、构建、服务、模型调用、迁移。不提交、不 push。

## 能力与设计

Coding 每轮从平台历史构建模型上下文，不能照搬 Nomi 的私有 Session 文件清理。
仅回收进程会在下一轮重新加载旧历史。现在通过编译期策略
`RuntimeEngineAdmission::uses_platform_history_context(binding)` 声明此类实现：
必须没有独立的私有历史副本，每轮使用平台历史端口，恢复义务不依赖对话投影。
默认 false，官方 Coding 开启；与 `uses_nomi_session` 同时开启属于冲突策略。
Catalog 用精确 build/digest/contract/profile 判定，缺失构建拒绝维护，不做 fallback。

## 主路径

1. Conversation 原有 preparation/reset fence、用户所有权、terminal 状态、保留
   Attempt 与 edit/resubmit 检查继续有效；不新增 Session owner。
2. 等待取消的构建/写回收束，使用现有 exact-slot teardown 回收活跃 runtime。
   清理失败不推进上下文起点；冷会话不启动工厂、不调用模型或工具。
3. DB 新的 `clear_terminal_engine_context` 在写事务中核对 created_at、exact extra、
   terminal 状态、无 active operation、无 accepted receipt/retained Attempt。
   将最新消息自增 row id 写入 backend-owned `engine_context_after_message_id`，
   递增 admission epoch；不删除任何消息、事件、receipt、artifact 或恢复记录。
4. 平台原生事件历史、兼容消息历史和历史翻页在同一读事务中读取持久起点，核对
   当前 turn epoch/operation。只返回起点之后的记录，旧游标不能越过边界。
   `has_older` 不把已清理历史报告为可翻页内容。
5. 普通 extra 更新、extra CAS、能力/MCP snapshot 替换禁止改写这个字段；陈旧
   全量 extra 保存返回冲突，不能覆盖刚提交的清理。公开 API 拒绝用户提供此字段。
   明确删除全部消息的 reset/clear_messages 才通过原聚合事务移除该边界。

## Fork 与保留信息

使用平台历史的 Engine 在 Fork 时，复制的消息就是历史来源，不再把同一份 JSON
重复嵌入 system_prompt。因此新子会话的上下文清理不会留下第二份历史副本。
Fork 继续是用户显式导入所选归档前缀的动作，会包含其选择的历史；父会话的 row-id
起点不复制到新的消息序列，精确 Engine 绑定仍继承。Nomi 私有路径保持原行为。

Coding 的 Patch 恢复读取仍独立于消息历史起点，清理上下文不能绕过失败 Patch
重新观察义务。模型/工具权限、Agent 配置、已选 Skill、工作区及安全恢复信息不被
当作对话历史删除。这不是安全擦除，也不是取消所有外部效果的证明。

## 尚缺证据及范围

- 尚未验证冷/热清理、下一轮重建、分页拒绝、Fork、配置更新竞争、取消与 DB 回滚。
- 任意私有持久化社区 Engine 的冷清理没有通用默认实现；不可谎报成功。此类引擎
  仍须提供自己的生命周期设计，不能为了获得入口而冒充平台历史实现。
- 不迁移旧 exact build 的 codec/绑定，不修复旧版 Fork 已有的私有 prompt 副本。
- 未新增数据库 migration 或用户可写配置开关；未证明整体质量优于 Nomi/Codex。

官方源码标识：Coding host2-coding-loop88 / Nomi host58。加入 DB 清理模块和
repository 源码到官方 digest。CAR 继续 in_progress，不是整体能力完成声明。
