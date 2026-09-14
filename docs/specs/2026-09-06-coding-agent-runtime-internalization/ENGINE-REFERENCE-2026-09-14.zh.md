# 独立 Engine 参考接入（2026-09-14，未验证）

分支仍为 `rf/agent-capability-platform-v2`，未 commit/push。按用户要求不运行构建、
测试、评测或 E2E；仅源码实施和局部 rustfmt。整体 CAR 仍未完成。

新增 `crates/backend/nomifun-app/examples/evidence_engine/` 独立编译目标：真正使用现有
Session、Broker、Kernel、日志、历史和资源清理端口的证据分析循环。它不调用 Coding
实现，没有伪造模型/工具端口或空清理方法，也不加入产品默认注册表。

循环是“1～3 个研究问题 → 限步只读研究 → 证据板 → 带有效来源 ID 的综合回答”。
自己的上下文策略、事件格式和流处理可与 Coding 独立演化。官方仍预置 Nomi/Coding；
社区在源码/依赖接入并重新打包后才可注册，Engine 不变成新的 Agent 产品身份。

接入过程发现并修正 `NomiCoreApplication::compose_with_runtime_engines` 的签名缺口：
回调原先只有 `&RuntimeEngineHost`，不能调用需要 `&Arc<Self>` 的 `register_session_hosted`。
现在保留 Arc，复用真实 Session-host 工厂，不暴露 `AppServices` 或另造 Session owner。
既有按普通方法调用的闭包可依赖 deref coercion；显式标注旧回调参数类型的源码集成需调整。

Example 使用 canonical `fs.read/fs.search` schema，自定义模型侧工具名；先核对整批调用，
再按固定 Snapshot/generation 执行。借鉴本地 Codex `core/src/tools/parallel.rs` 固定
StepContext 的原则，平台继续保留已准入的 Kernel 任务，不照搬 AbortOnDrop 取消副作用。
每个结果持久记录后才标记观察，真实资源清理后才记录 Engine 终态。

Build digest 使用最终宿主可执行文件 SHA-256，避免只哈希示例文件而漏掉公共 SDK/依赖/
构建选项变更。升级不热迁移旧 Session。此策略用于参考实现，不改变官方 Coding 的
`host2-coding-loop11` 描述；本切片没有改 Coding 循环或 shared-resource 的行为。

详细接入、预算、局限和后续手动验证命令见该 example 的 `README.zh.md`。目前不能将
“源码接入已完成”表述为“已成功运行”。MCP/MiniApps/非函数 Plugin、Git push 凭证与
外部副作用恢复、非制品 Skill、动态路径指令、安全 checkpoint 续跑、人工隔离处理、
任务完成证据以及多消费者/多平台验证仍待完成。
