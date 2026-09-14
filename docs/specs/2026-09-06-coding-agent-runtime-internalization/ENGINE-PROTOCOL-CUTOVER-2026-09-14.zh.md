# 平台能力词汇与外部协议退役（slice77）

日期：2026-09-14。分支 rf/agent-capability-platform-v2，本地未提交工作树。
Coding host2-coding-loop77 / Nomi host49。未构建、测试、启动服务、调用模型、
执行迁移、commit 或 push；本文不是验收报告。

## 本轮源码实现

- 新 PlatformFeatureInventoryPayload 仅含 schema_version 和受限 feature ID 集合。
  保留已有 22 个词汇供目录物化，移除 source pin、profile、RPC、native action
  与 FullAuto 外部握手含义。state.rs 使用此中立输入；每个 Engine 仍须独立准入。
- 删除旧 hello/command/binding/native-action/release wire、漂移/发布/握手 fixture
  及 Rust 导出。保留有真实消费者的 Kernel authority、持久 profile/checkpoint
  和 RuntimeEventEnvelope/Ack；这些不是外部 Engine 装载接口。
- 合同生成器改用中立清单，不再生成 runtime_command/runtime_hello/release fixture。
  历史 runtime_protocol_digest 键保留，但仅覆盖共享 snapshot/feature schema；
  不能解释为已认证外部 RPC。相关 seed、schema、envelope 和 ledger 摘要定点同步。
- 旧 gate-agent-v2 大量 Wrapper 阶段分支物理删除；仅 contract-closure 保留，其他
  命令在执行/报告写入之前失败。macOS 预检删除 app-server/hello/sidecar 及凭据
  文件探测，旧参数（包括直接传入 options）明确拒绝。Host/包/签名检查仍保留。
- 六份旧 D-014 删除计划与旧组装 inventory 移入 contracts/historical/agent-v2。
  历史 JSON 内容保留，生成器明确从归档读历史摘要输入，不用旧清单指挥当前删除。
  现行 current-composition.json 明确原 Conversation、两个官方 Engine、编译期社区
  扩展以及共享端口/领域 owner，不再声称 Codex 替代 Nomi 或删除 Conversation。
- 两种官方 Engine 构建摘要新增 engine_features.rs、平台词汇 JSON 与 runtime.rs。
  新构建身份不更新既有 Session/Fork 的冻结绑定，不提供静默 fallback。

## 源码整理与证据限制

读取并追踪了新旧类型和脚本调用点；共享合同消费者保留。序列化摘要只做定点编辑，
大整数 schema 以原始数字文本参与摘要，未通过 JavaScript Number 重写。没有执行
Rust 生成器，不能声称全部生成输出与当前源码一致，也未证明 Cargo.lock 一致性。
修改了 macOS 脚本测试源码以表达新行为，但没有运行测试。

旧 gate 的退役不等于现行 CAR 发布 gate 已实现或通过。先前报告中的旧宿主、
sidecar 结果和历史阶段 passing notes 均不能覆盖当前工作树。

## 未完成项

CAR-08 保持 in_progress：仍缺生成器/Cargo/主链/制品的实际证据。能力边界继续见
ENGINE-READINESS-2026-09-14.zh.md：Git 网络凭据 owner、未知进程跨重启证明、
MCP 服务端主动生命周期及其他既有任务边界未因清理旧协议而自动完成。
本轮解决的是平台和 Engine 的合同/组装边界，不宣称 Coding 质量优于 Codex
或 Nomi，也不把“源码存在”标为“整体能力完善”。
