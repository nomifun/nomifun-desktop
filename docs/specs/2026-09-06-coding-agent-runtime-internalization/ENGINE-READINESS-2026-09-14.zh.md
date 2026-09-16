# 多 Engine 目标与剩余闭合清单

> 最新范围修订：以下早期清单中的“独立社区示例运行”和“三平台交付”不再是
> 本机 Windows 前置条件；Coding 是第二种 Engine 接入实现。macOS/Linux 只记
> 交接 TODO。本轮 Windows 集成已落地，工程检查结果见
> [WINDOWS-DELIVERY-HANDOFF-2026-09-14.zh.md](WINDOWS-DELIVERY-HANDOFF-2026-09-14.zh.md)。

日期：2026-09-14。源码基准为本地 rf/agent-capability-platform-v2 当前工作树，
非已提交/构建制品。最新 Windows 集成为 Coding loop91 / Nomi host60，已同步
远端 `52857f39c`。桌面编译、UI 类型检查及定向回归结果集中记录于上述交接文档；
下方“未验证”是早期切片当时的记录，不代表本轮没有运行工程检查。

Slice90 补齐有界、自包含的压缩保留上下文，避免窗口内压缩记录引用窗口外工具
批次时重建失败；当前输入仍来自本轮已接受记录，历史副本不是执行证据。旧记录
及检查点自身在窗口外的限制不被掩盖。见 CODING-COMPACTION-CHECKPOINT-2026-09-14.zh.md。
按用户要求，本轮至此收尾，未开展验证或额外生态扩展。

Slice89 补齐 Coding 原生事件重建前的有界 Fork/导入消息前缀，避免第一轮本地
事件产生后丢失继承上下文；压缩按原顺序替换历史，新增共享历史前缀端口遵守
清理起点与当前 turn 身份。见 CODING-HISTORY-PREFIX-2026-09-14.zh.md；未验证。

Slice88 为平台历史 Engine 增加持久上下文起点和冷/热清理路径，Coding 编译期
声明支持；历史重建/翻页遵守起点，配置写入不能覆盖它，恢复义务仍保留。
Fork 对此类 Engine 不再把复制历史二次放入 system_prompt。见
ENGINE-CONTEXT-CLEAR-2026-09-14.zh.md；私有社区 codec 的冷清理仍非默认能力，未验证。

Slice87 将 Nomi 私有恢复和 Plugin scope 的选择改为精确构建源码策略；默认拒绝，
实际 runtime 与声明不一致时通过 exact-slot 清理并保留失败隔离。缺失旧构建仍可
走独立精确恢复钩子，不按 family 使用新版 codec。见
ENGINE-EXACT-SESSION-POLICY-2026-09-14.zh.md；未运行验证。

## 不变的验收目标

Agent 是身份与配置；Engine 是可由多个 Agent 复用的任务执行策略，拥有不同的
循环、规划或上下文机制。官方默认 Nomi 通用 Engine 和 Coding Engine；社区
只通过源码集成、编译和重新打包注册。选择属于 Agent 工作台，Session 冻结
exact build，不能热换、静默 fallback 或借 Fork 更新引擎。平台仍拥有模型、
工具、权限、资源、Session 事实和副作用结算。

不能将这一目标替换为“又完成了一个增强切片”，也不能用更多功能替代验收。

## 当前源码证据与缺失证据

Slice86 修复调用者丢弃导致冷工厂 future 中止的缺口。三个生产获取入口共用持有任务，
取消独立 child token；异常状态在全局准入锁/每 Session gate 释放前记录，空槽不能
替代无资源证明。见 ENGINE-ACQUISITION-2026-09-14.zh.md；未运行并发/取消验证。

Slice85 将生产组装错误从 panic 包装接回入口清理，Registry 增加永久准入关闭、
在途构建屏障与 exact-slot 清理 flight；host 关闭在 SQLite 前等待，引擎清理超时
仍保留资源。独立服务端组合失败保留完整服务图。见 ENGINE-SHUTDOWN-2026-09-14.zh.md；
未运行并发/取消/错误恢复或实际退出验证，不是整体生命周期闭合证明。

Slice84 补齐 RuntimeEngineHost 的编译期渠道声明，以及桌面带类型化清理结果的
源码注册回调；独立社区示例声明 stable。见 ENGINE-COMPOSITION-2026-09-14.zh.md。
组装后拒绝注册、已有 Session/Fork exact binding 不变；未运行桌面/渠道回归。

Slice83 增加记录边界优先的摘要分片和摘要专用 Provider 元数据清理，保持已有压缩
原子替换及紧密分片容量。见 CODING-COMPACTION-RECORDS-2026-09-14.zh.md；未运行回归。

Slice82 增加 Coding 未暴露工具名的整批预检、逐项错误反馈和有界模型纠正循环，
不改变工具 owner 或恢复证明。见 CODING-TOOL-NAME-RECOVERY-2026-09-14.zh.md；未运行回归。

下表的“已定位”只说明实现存在，不证明编译、行为、性能或平台兼容性。

| 要求 | 当前源码位置（仓库相对路径） | 完成证据状态 |
|---|---|---|
| Agent 工作台配置 Engine | ui/src/renderer/pages/agentSettings/AgentPresetEditor.tsx、AgentRuntimeEngineSelector.tsx | 编辑器绑定 draft.document.runtime_engine；未运行 UI/保存链路 |
| 两个官方默认 Engine | crates/backend/nomifun-app/src/router/runtime_engines.rs::install | 注册 nomifun.nomi 与 nomifun.coding，再注册显式扩展；未运行产品目录 |
| 仅源码注册、组装后关闭 | RuntimeEngineHost::register / install、bootstrap/nomi_core.rs::compose_with_runtime_engines | OnceLock 与组装锁拒绝迟到注册；未运行生命周期回归 |
| Session exact binding / Fork 继承 | runtime_engines.rs::agent_binding、nomi_core_session.rs 的 create/Fork 路径 | 持久绑定和父绑定核对已定位；未运行创建/重启/Fork |
| Engine 自己的执行策略 | nomifun-coding-engine/src/turn.rs、planning.rs、context_lifecycle.rs；nomifun-ai-agent 的 Nomi 实现 | Coding 独立循环已定位，不是 Nomi prompt profile；无质量对比证据 |
| 开放真实宿主接口 | nomifun-ai-agent/src/engine_sdk.rs；nomifun-app/src/router/engine_session_host.rs、engine_kernel_session.rs、engine_journal.rs | 生命周期与策略分离、真实端口接线已定位；未验证端到端清理 |
| 独立社区 Engine 示例 | nomifun-app/examples/evidence_engine/{main,driver,model,history}.rs | 独立研究/证据循环源码存在，不调用 Coding loop；尚无成功运行记录 |
| Coding 文件/命令/上下文主链 | standard_tools.rs、共享 File/Process owner、context/history 模块 | 多轮调用、修改、进程、压缩/续接实现存在；新增实现未编译测试 |
| 旧外部 Wrapper 退出生产图 | 根 Cargo.toml、nomifun-app/Cargo.toml、nomifun-public/Cargo.toml、agent-contracts/src/engine_features.rs | **源码切换，未验收**：slice75/76 删除打包链及旧 crate/宿主/桥接/兼容开关；slice77 删除外部线协议并拆出平台词汇，旧 gate 退役、旧删除计划归档；仍缺构建/行为/制品证据 |
| 跨平台和真实模型交付 | CAR-02/03/04/05/06/07/10 验收项 | **缺失**：历史检查不能覆盖当前大幅改动，用户本轮排除验证 |

## 仍需实施的闭合工作

### 1. CAR-08：旧 Wrapper 与旧组装图退出

Slice74 的默认依赖隔离已由 slice76 物理删除替代：nomifun-agent-platform 和
nomifun-codex-runtime crate、旧 Fresh-v4 组合根/制品加载/路由/桥接、可选兼容
特性均已删除。remote_runtime.rs 保留当前 Nomi 的任务租约与关闭协调器。
共享目录归 control-plane，Wave1 领域适配器归 agent_wave1_host.rs；相关测试
源码迁出。详见 ENGINE-WRAPPER-REMOVAL-2026-09-14.zh.md。

Slice75 已删除 macOS 外部 Runtime 打包链和发布锁 sidecars 字段。Slice77 删除
contracts/runtime 的外部线协议/导出/fixture，state.rs 改用中立平台能力词汇。
持久 Session event/profile/checkpoint 和 Kernel authority 仍有消费者，继续保留。
六份旧删除计划及旧组装清单归档至 contracts/historical/agent-v2；保留历史摘要
输入，不再作为 CAR 的删除指令。新 current-composition.json 明确保留现行 owner。
旧 C1–C9/AP-7 CLI 退役，macOS sidecar 探测删除；保留 contract-closure 与 Host
工程预检并不等于已有完整 CAR 验收门禁。详见 ENGINE-PROTOCOL-CUTOVER-2026-09-14.zh.md。
仍需 Cargo/生成器一致性、运行主链和制品证据；CAR-08 未完成。共享 owner 不删除，
不迁移到第二套 Session 存储；CAR-D-019 仍有效。

### 2. 已知能力边界不是通过状态

- Git push 目前仅 local/file remote；agent_wave2_vcs_push.rs 明确拒绝未接入
  应用级 Git 凭据 owner 的 HTTPS/SSH。后续完善应先定义该 owner 与授权合同，
  不借用 Provider/MCP/进程环境凭据，也不在工具参数接收秘密。
- 进程重启恢复只接受已有 exact turn/boot/连续清理凭据；缺少凭据仍隔离。
  不能用 PID 或本地 future 退出代替进程树证明，未知外部效果不自动重放。
- MCP 任意二进制解析、服务端主动生命周期、其他生态消费者的非工具生命周期
  仍有边界；应按原 CAR-09 任务处理，不由 Engine 私有插件系统接管。
- 独立任务语义正确性无法由工具返回零退出码或模型 completion 账目证明。
  更多提示、约束和账目不能替代真实场景质量评估。
- Bedrock 异常头分类在 slice78 补齐；slice79 已接 Messages 完整原生解码、云端
  请求形状和 typed reasoning 的 exact-producing-route 绑定，但未运行真实多轮闭环。
  Vertex adapter 的接线不解除 manifest 对 project/location 配置合同缺失的拒绝。
  Provider 特有上下文错误、托管工具/其他私有状态和消费者事件覆盖仍不能视为完成。
- Slice80 已修复 SSE 追加前边界、事件分隔/EOF/错误结束和无事件源的调度让出；
  尚未执行分块/取消回归。Slice81 已拆为 setup/完整帧 idle timeout（生产分别 120 秒），
  保留注入客户端约束，补齐 JSON/AWS 调度让出及错误体限时/HTTP 分类保留；
  详见 ENGINE-STREAM-DEADLINES-2026-09-14.zh.md，不能推断全流式通路已验证。

以上不要求随意扩张成全部行业功能；按已有任务合同和用户明确范围闭合。

## 验证与发布工作（仍按用户要求不执行）

核心源码存在之后，至少仍需适当范围的 Rust/UI 检查，以及真实 Session 的
创建→工具→续接、取消/清理、恢复、压缩、Agent/Fork exact binding、社区参考
Engine 运行和三平台交付证据。不能把格式化或检索结果当作这些验收。
若继续要求“整体完成并得到验证”，最终需要用户解除此前的验证排除；在此之前
可以继续有根据的实施，但不得宣布整体完成。

## 本次合同纠正

02-target-architecture-and-port-contracts.zh.md 的旧 2.3/2.4 描述把通用 Runtime
定义为唯一模型循环、Coding 定义为 profile，与 CAR-D-021 和用户要求不一致。
现明确 Engine owns loop/planning/context，共享 SDK 只提供生命周期与已准入端口；
同时修正原 Conversation owner 和工作台→Revision→Session 的关系。
这是对既定设计的同步，不是改变目标或授权第二套 owner。旧隔离交付段标为历史。

合同整理本身没有改变构建标识；后续 slice74 已推进代码和构建标识。
两阶段均未构建、测试、启动服务、调用模型、执行迁移、commit 或 push，
没有把任何运行验收标为通过。整体目标保持未完成。
