# Coding 压缩上下文检查点（slice90）

日期：2026-09-14。仅本地 `rf/agent-capability-platform-v2` 源码实现；未验证。
按用户最新要求，本轮只收尾压缩跨窗口恢复，不扩展 Engine/Provider/MCP 功能。

## 修复目标

生产重建最多读取最近 32 个原生 turn。原 `ContextCompacted` 只保存摘要和
`retained_tool_call_ids`；窗口中最早的压缩若保留了窗口外的工具批次，严格重放
无法找到对应调用/结果，导致恢复失败。扩大窗口不能消除这类边界问题。

参考本地 Codex 固定提交 `6af345407d9c2a568da9d01b6c4b81a9e61495c0` 的
`codex-rs/core/src/compact.rs`：`CompactedHistoryMetadata` 将检查点元数据与
replacement history 分开，持久化替换历史。本实现借鉴自包含替换记录，而非
照搬 Codex 的 Session owner、存储格式或私有 provider 状态。

## 实现

- `compacted_history.rs` 定义 `CodingCompactedItem`：当前 turn 已接受输入只存
  索引，其余保留消息存便携副本。反向匹配保持重复同文输入的次数和顺序。
- `ContextCompacted.retained_context` 为可选字段；新压缩写 `Some`，旧记录缺失
  字段仍走严格引用重放。保留摘要与最多三批/64 个工具 ID 的既有策略。
- 生成副本不复制图片/音频二进制、provider round/metadata、私有推理；媒体变为
  明确不可推断内容的描述，AGENTS 指令读取结果省略正文。System 角色不可进入。
- 当前用户附件不存入副本；重放从当轮原始请求和 steering 记录重建，再按索引
  放回其原有位置。拒绝缺失、重复、越界或乱序的当前输入。
- 检查点限制为 4096 项、2MiB 序列化数据，追加前计数并最终检查整体；超限报错，
  不偷偷删掉工具或当前输入。捕获和检查完成、事件写入成功后才替换 live input。
- 宿主对副本中的工具结果继续应用既有 32KiB 持久观察投影和指令省略策略；
  live input 仍保留本轮必须交给模型的原图，持久历史不会重新加载图片。
- 正常重放检查完整、连续、唯一的调用/结果后使用自包含替换，不依赖窗口外 ID。
  历史读取的精确 Session/build/snapshot、终态和清理起点约束不变。
- 单 turn archive 检查继续只用本轮原始事件，不把检查点里的跨轮工具结果导入为
  本轮执行证据；不会执行工具，也不解除恢复隔离或生成完成凭据。

## 边界与收尾

Coding 身份更新为 `host2-coding-loop90`，新模块进入构建摘要输入；Nomi 未改，
保持 `host59`。不新增 Session owner，不更改社区源码注册或打包后禁止挂载约束。
不迁移旧精确构建、不伪造旧记录丢失的工具结果。

本次解决的是**读取窗口内存在新压缩记录、其保留批次在窗口外**的恢复失败。
不声称无界恢复全部历史：检查点本身若也落在读取窗口外，仍受既有历史窗口限制；
没有自动跨越清理起点、归档取回或无限递归拉取。副本可能已是限长观察，不保证
恢复原始工具输出、私有推理或二进制内容，不能视作 fresh workspace observation。

仅阅读源码并对五个相关小模块执行 rustfmt。没有运行测试、构建、服务、模型调用、
数据库迁移、提交或 push。CAR 整体验收仍未完成，不能由此宣称运行质量已验证。
