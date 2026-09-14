# CAR 阶段状态

> 2026-09-14 macOS 接手补充：基线 `2ff029512` 已同步，本轮修复将本地提交后重建、未 push。
> arm64 原生开发启动及 runner/CLI 退出已验证；Nomi 真实四阶段通过，Coding 创建通过、
> 第二轮修改报 `UNKNOWN_UPSTREAM_ERROR`，不能宣布完整执行链或发布验收完成。
> 本轮修复、检查、制品状态与未验证项见 [MACOS-DELIVERY-2026-09-14.zh.md](MACOS-DELIVERY-2026-09-14.zh.md)。
> 下方 Windows 阶段描述保留为历史记录；当前交付范围见 TASK-MANIFEST.json。

> 2026-09-14 推送交接补充：用户已授权提交与推送完整源码。引擎实现基线
> `739b5e431`、官方模板引擎入口 `c619d8a9a` 已提交，与上游 `16dabde80` 合并为
> `4a774ee59`。macOS 接手说明与本次实际复核结果见
> [MACOS-HANDOFF-PROMPT.zh.md](MACOS-HANDOFF-PROMPT.zh.md)。
> 合并后 Windows 编译、83 项 UI 回归、24 项脚本回归通过；综合检查中的旧术语
> 门禁仍失败，真实双引擎创建冲突也未在本次复测或解决。此次为开发交接，不是正式发布验收。
> 下文“未提交/未 push”及旧远程基线描述保留为此前阶段记录，不作为最新 Git 状态。

> 更新时间：2026-09-14
>
> 当前阶段：**Windows 多 Engine 开发收尾。已同步远端平台变更并完成冻结能力、统一 Plugin Product、迁移冲突及 Agent 工作台集成；Windows 桌面编译、UI 类型检查及定向工程回归已有通过记录。Windows 0.7.6 NSIS 制品已构建并通过完整性检查，启动与真实模型验收另行记录；不将社区示例或跨平台工作作为本机前置条件。**
>
> 唯一状态源：本文与 `TASK-MANIFEST.json`。上一阶段
> `2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md` 不记录 CAR 状态。

## 阶段状态

**用户最新范围修订：本机只交付 Windows；独立社区 Engine 示例不再作为验收项；
macOS/Linux 转为跨机器交接 TODO。** 已合入远程至 `52857f39c`，本地 merge
`f6db504e6`；恢复 loop90 工作后已整合静态 enabled 能力模型、统一 Plugin 与迁移编号。
当前构建为 Coding loop91 / Nomi host60。历史“未验证”仅描述各切片当时状态，
本轮实际检查结果以 [WINDOWS-DELIVERY-HANDOFF-2026-09-14.zh.md](WINDOWS-DELIVERY-HANDOFF-2026-09-14.zh.md)
为准。源码工程检查与安装包/真实模型交付验收分别记录，不相互替代。

随后按用户要求新增 Windows 安装包与 StepFun 真实模型验证，见
[WINDOWS-PACKAGE-LIVE-2026-09-14.zh.md](WINDOWS-PACKAGE-LIVE-2026-09-14.zh.md)。
模型名由用户明确更正为 `step-3.7-flash`。NSIS 新制品和资源完整性已通过；
不覆盖本机已安装的 0.7.5，不把包内程序的隔离启动视为安装/升级/卸载全生命周期通过。
本轮最终结果：包内程序 backend health 200，但当前提升权限会话下 UI 自动化与正常退出
未验证；Nomi/Coding 两个真实模型首轮任务均报 `CONFLICT`，未取得工具成功证据。
完整交付验收未通过，具体证据和后续阻塞见上述报告，不宣称产品发布已完成。

Windows 开发集成已完成：桌面/后端及生产集成测试编译、完整 UI typecheck 通过；
UI 80、Coding 36、Core 14、迁移 9 项定向回归全部通过。代码保留在当前本地
工作树，未 push；跨平台仅交接 TODO。整体 CAR 的产品发布验收不据此标记完成。

2026-09-14 已按用户最终目标重新核对核心源码位置，并纠正主架构文档中“通用
Runtime 强制拥有循环、Coding 只是 profile”的过时描述。清单见
[ENGINE-READINESS-2026-09-14.zh.md](ENGINE-READINESS-2026-09-14.zh.md)。
随后 slice74 已将默认产品的 Wrapper 依赖和旧宿主入口隔离为非默认构建特性，
能力目录移至中立 control-plane。详见
[ENGINE-WRAPPER-ISOLATION-2026-09-14.zh.md](ENGINE-WRAPPER-ISOLATION-2026-09-14.zh.md)。
这一步是历史中间状态，不能用特性隔离替代删除验收。

Slice75 随后物理删除 macOS 的外部 Runtime 导入/暂存/打包链，并将 release-lock
切为不含 sidecars 字段的 v2；旧锁明确拒绝、不自动转换。详见
[ENGINE-PACKAGING-CUTOVER-2026-09-14.zh.md](ENGINE-PACKAGING-CUTOVER-2026-09-14.zh.md)。
Slice76 已进一步物理删除两个 Wrapper crate、旧桥接/宿主/路由与兼容开关，
迁出共享 Wave1 owner 和目录测试。详见
[ENGINE-WRAPPER-REMOVAL-2026-09-14.zh.md](ENGINE-WRAPPER-REMOVAL-2026-09-14.zh.md)。
Slice77 已拆出平台能力词汇清单，删除外部执行器线协议/发布 fixture，退役旧阶段
gate 与 macOS sidecar 探测，并归档与当前设计冲突的旧删除计划。详见
[ENGINE-PROTOCOL-CUTOVER-2026-09-14.zh.md](ENGINE-PROTOCOL-CUTOVER-2026-09-14.zh.md)。
Slice78 补齐 Bedrock 流异常头解析及 Broker 分类，见
[ENGINE-BEDROCK-ERRORS-2026-09-14.zh.md](ENGINE-BEDROCK-ERRORS-2026-09-14.zh.md)。
Slice79 将 Messages 云端 adapter 接入完整原生循环，修复 Bedrock 请求形状，
并给 typed reasoning 加上实际产出路由绑定；Vertex 生产路由仍明确不支持。详见
[ENGINE-MESSAGES-CLOUD-LOOP-2026-09-14.zh.md](ENGINE-MESSAGES-CLOUD-LOOP-2026-09-14.zh.md)。
Slice80 修复模型 SSE 的追加前边界、增量分帧、EOF/错误终止和无事件源的调度让出，
详见 [ENGINE-STREAM-FRAMING-2026-09-14.zh.md](ENGINE-STREAM-FRAMING-2026-09-14.zh.md)。
Slice81 将模型请求的固定总超时拆为 setup 和完整帧 idle deadline，并补齐 JSON/AWS
读取调度及错误响应体限时，详见
[ENGINE-STREAM-DEADLINES-2026-09-14.zh.md](ENGINE-STREAM-DEADLINES-2026-09-14.zh.md)。
Slice82 补齐未暴露工具名的整批预检和错误反馈纠正循环，保留原权限与执行边界，见
[CODING-TOOL-NAME-RECOVERY-2026-09-14.zh.md](CODING-TOOL-NAME-RECOVERY-2026-09-14.zh.md)。
Slice83 改为消息记录边界优先的压缩分片，补齐摘要私有元数据隔离和追加前边界，见
[CODING-COMPACTION-RECORDS-2026-09-14.zh.md](CODING-COMPACTION-RECORDS-2026-09-14.zh.md)。
Slice84 补齐社区渠道声明及桌面源码组装回调，复用冻结目录和类型化失败清理，见
[ENGINE-COMPOSITION-2026-09-14.zh.md](ENGINE-COMPOSITION-2026-09-14.zh.md)。
Slice85 将生产组装错误接回清理，补齐 Registry 可等待永久关闭及服务端完整资源保留，见
[ENGINE-SHUTDOWN-2026-09-14.zh.md](ENGINE-SHUTDOWN-2026-09-14.zh.md)。
Slice86 将冷构建交给 Registry 持有，补齐丢弃等待者及异常构建的隔离边界，见
[ENGINE-ACQUISITION-2026-09-14.zh.md](ENGINE-ACQUISITION-2026-09-14.zh.md)。
Slice87 移除按 family 推断 Nomi 私有 Session 协议的分支，接入精确构建注册策略，
并对实际 runtime 的声明不一致执行隔离清理，见
[ENGINE-EXACT-SESSION-POLICY-2026-09-14.zh.md](ENGINE-EXACT-SESSION-POLICY-2026-09-14.zh.md)。
Slice88 接入平台历史 Engine 的冷/热上下文清理：持久历史起点、运行时回收、历史
翻页边界与配置写入防覆盖，Coding 开启；Fork 不再重复注入 system prompt，见
[ENGINE-CONTEXT-CLEAR-2026-09-14.zh.md](ENGINE-CONTEXT-CLEAR-2026-09-14.zh.md)。
Slice89 补齐 Fork/导入消息前缀与 Coding 原生 turn 的连续重建；前缀先于事件
压缩应用，受清理起点和预算约束，不重复或跨过缺失 turn 拼接，见
[CODING-HISTORY-PREFIX-2026-09-14.zh.md](CODING-HISTORY-PREFIX-2026-09-14.zh.md)。
Slice90 为压缩记录加入有界、自包含的保留上下文及当前输入位置引用，补齐窗口外
工具批次的重建；媒体/私有状态不落入副本，宿主仍应用观察限额，旧记录不伪造，见
[CODING-COMPACTION-CHECKPOINT-2026-09-14.zh.md](CODING-COMPACTION-CHECKPOINT-2026-09-14.zh.md)。
按用户最新要求，本轮在此收尾，不继续扩展其他能力；检查点自身仍受历史读取窗口约束。
当前官方构建为 Coding loop90 / Nomi host59；Session owner 不变。
CAR-08 保持 in_progress；源码删除不能替代构建、主链行为与制品验收，未执行验证。

| 项目 | 状态 |
|---|---|
| 文档系列 | in_progress |
| 源码/许可证基线 | completed |
| 通用 Engine Catalog/exact Binding 持久接线 | implemented |
| 编译期注册及 Engine 自声明兼容性准入 | implemented（CAR-D-021） |
| 开放 Registered Runtime / Coding adapter | implemented |
| 隔离 Coding Engine Core | completed |
| Model Broker 原生取消/中央适配 | in_progress |
| Kernel Tool admission/owner adapter | pending_validation |
| Process owner adapter | pending_validation |
| Patch/File/VCS/Workspace owner 接线 | in_progress（File/VCS、process.exec 及 Session 隔离 fs.snapshot；新增未验证） |
| Context/Compaction/Resume contracts | in_progress（路径指令/压缩/正常重放及保守异常恢复已接；进程中断未知仍隔离） |
| AgentSession 主链与异构 Registry | in_progress（按 CAR-D-019 保留现有 owner） |
| 旧 Wrapper 删除 | in_progress（crate/宿主/桥接/路由/打包/外部线协议已删除，旧 gate 退役；主链/构建/制品证据未完成） |
| 生态消费者接入 | in_progress（官方 Skill、共享 MiniApp/Robot 工具与 Robot 动态上下文已接 Coding/社区端口；其他生命周期/消费者继承待完成） |
| 三平台发布 | planned |

## 任务快照

| 任务 | 状态 | 依赖 | 备注 |
|---|---|---|---|
| `CAR-00` | completed | 无 | Codex commit、选取/排除、LICENSE/NOTICE 和 owner 边界已核对 |
| `CAR-00A` | completed | `CAR-00` | Coding family Catalog、immutable Build、Stable/Canary alias 和 exact Binding |
| `CAR-01` | completed | `CAR-00A` | 独立 `nomifun-coding-engine`，经通用工厂接入默认组合根 |
| `CAR-02` | in_progress | `CAR-01` | Broker 原生取消已实现；当前生产 Session/真实 Provider 的端到端验收待接线 |
| `CAR-03` | pending_validation | `CAR-02` | Kernel adapter、Snapshot/active-set admission、标准 Tool surface 本地完成；等待主链验证 |
| `CAR-04` | pending_validation | `CAR-03` | `nomi-process-runtime` adapter 已完成；Windows start/wait/stdin/timeout/cancel/output 已验证，跨平台待远程 |
| `CAR-05` | in_progress | `CAR-03` | 9 个初始 File/Patch/VCS 工具已接默认 owner；逐工具与平台验收待补 |
| `CAR-06` | in_progress | `CAR-03` | 路径 AGENTS/预算/压缩/重放及保守恢复已接线但未验证；不自动续跑旧 checkpoint |
| `CAR-07` | in_progress | `CAR-04`～`CAR-06` | 默认 owner、目录、精确绑定、Broker/Kernel 和动态 UI 入口已接；完整生态/恢复验收待补 |

## 2026-09-14 最新实施：模型 SSE 有界分帧（未验证）

共享传输在追加/解码前限制单行、事件及 chunk；增量游标避免反复搬移剩余数据。
支持 CR/LF/CRLF、分块换行、UTF-8/BOM 和正确空 data 行拼接；未提交事件 EOF
不合成为终态，错误或合法 DONE 后释放 SSE 源。持续无事件的 ready-only 源让出
调度，但尚未改变生产的总请求超时策略。没有执行测试、构建或真实模型验证。

## 2026-09-14 实施：Messages 云端循环及签名推理路由隔离（未验证）

Bedrock/Vertex adapter 选择完整 Messages 原生状态机，错误投影优先处理。
Bedrock 不再发送 model/stream；平台参数默认值不能重注入或改写准入后的结构，
有效输出上限收紧后再次检查 thinking 预算。Broker 为 typed thinking/redacted
块绑定实际产出路由，并在后续准入/编码时匹配；旧无来源块不能静默跨路由续接。
这是源码实施，未运行；Vertex 仍缺 project/location 的生产配置合同并保持拒绝。

## 2026-09-14 实施：Bedrock 结构化流异常（未验证）

共享模型传输在双 CRC 之后读取 AWS 帧头，保留明确异常类型，拒绝重复/截断/
类型冲突及混入输出的错误包。Broker 区分限流、暂时不可用、鉴权、参数和模型
执行失败；未知错误、模型超时/流错误不自动重试，已提交语义输出的重试屏障不变。
不解析自然语言诊断来猜上下文溢出，不回传原始错误消息；终端错误后停止消费此流。
这仅补齐错误通路，不代表 Bedrock 全量原生工具/推理闭环已完成或验证。

## 2026-09-14 实施：平台能力清单与旧协议切换（未验证）

平台能力清单不再包含 Codex source pin、profiles、RPC 或 FullAuto 握手字段；
它仅供能力目录物化，不替任何 Engine 授权。删除旧命令/hello/native-action/
release wire 与 fixture，保留 Kernel authority、持久事件和 checkpoint 兼容合同。
合同生成源码及相关序列化摘要定点更新，未执行 Rust 生成器或证明完整生成一致性。
旧 C1–C9/AP-7 门禁明确拒绝调用；仅保留 informational contract-closure。
macOS 工程预检保留 Host/签名/包检查，不再启动 sidecar；不能代替 CAR 发布验收。
六份旧删除计划和旧组装清单移入 historical/agent-v2，历史摘要输入仍保留，
现行组装清单明确 Conversation/Nomi/Coding/编译期扩展的归属。
Coding host2-coding-loop77 / Nomi host49 将中立清单及合同源码纳入构建摘要。

## 2026-09-14 实施：旧 Wrapper 宿主与依赖物理删除（未验证）

两个旧 crate、旧组合根和外部进程加载/桥接/路由，以及隔离用兼容特性已删除；
共享 Wave1 owner 迁到 agent_wave1_host.rs，13 个领域测试与 2 个目录测试迁出。
平台继续拥有 Conversation 和领域能力，所有 Engine 共用同一产品宿主；桌面
不再保留宿主选择枚举。旧 AgentPlatform 专用测试删除，不计为新引擎测试覆盖。
Cargo.lock 仅定点删除旧引用，未经 Cargo 重生成；旧 schema/门禁仍待清理。
构建标识为 Coding host2-coding-loop76 / Nomi host48，所有运行验证均未执行。

## 2026-09-14 实施：旧 Runtime 打包链删除（未验证）

macOS 环境变量导入、hello 校验、二进制暂存与 sidecar lock 注入已删除；
Tauri 不再映射旧暂存目录。发布锁 v2 删除 sidecars 字段，并同步生成调用方。
原有 Host 架构检查、签名、公证、主程序/包/许可证摘要仍保留；没有运行这些操作。
Engine 标识不变，CAR-08 仍未整体完成。

## 2026-09-14 实施：Wrapper 默认构建隔离（未验证）

目录提供者迁至 control-plane；public 传输不默认依赖旧 AgentPlatform；app 的旧
Wrapper/宿主/路由/模型桥接/桌面启动与清理统一受非默认 legacy-codex-wrapper
控制。当前产品继续使用原 Conversation owner 和编译期多 Engine。未运行构建、
测试、服务或迁移；旧源码与打包残留尚待删除。源码细节和余项见上述 slice74 报告。

## 2026-09-14 实施：Patch 显式源版本前置条件（未验证）

任务 CAR-05 / CAR-06，详见
[CODING-PATCH-SOURCE-2026-09-14.zh.md](CODING-PATCH-SOURCE-2026-09-14.zh.md)。
fs.patch 每个文件可指定 expected_source：existing + 完整 SHA-256，或 absent。
共享 File owner 在任何目标发布前检查；摘要包括 BOM、全部行结束符和 hunk 外
内容。冲突返回该文件索引，不自动删除前置条件或重试。发布阶段继续沿用原字节
比较和创建 no-clobber，不宣称外部编辑被锁定。省略/any 保留旧逐行匹配语义。
canonical schema、Coding 工具 schema/说明、源集成类型及官方构建指纹已同步。
Coding `host2-coding-loop73`，Nomi `host46`；未构建/测试、服务/模型调用、迁移
执行或 commit/push。限定格式化发现并修正工具说明中的旧非法转义；整体仍未完成。

## 2026-09-14 历史实施切片：压缩后的多批次工具原文保留（未验证）

任务 CAR-06，详见
[CODING-MULTI-BATCH-CONTEXT-2026-09-14.zh.md](CODING-MULTI-BATCH-CONTEXT-2026-09-14.zh.md)。
原先只保留最近一批工具交换；现在从最新批次向前，在既有 32 KiB 可选尾部、
总 token/字节/消息预算内，最多保留连续三批、合计 64 个调用。保持完整调用/
结果配对、批次间输入及响应顺序，不跨权限消息边界或歧义 ID，不跳过中间批次。
最新未经过后续响应的图片仍为必须保留；完整历史仍进入摘要源，不增摘要调用。
正常恢复按精确有序 ID 解析相同连续后缀；独立历史归档仅检查本 turn 可见后缀，
缺失的更早前缀不补造、不导入。Coding `host2-coding-loop72`，Nomi `host45`。
未构建/测试、启动服务、调用模型、执行迁移或 commit/push；整体目标仍未完成。

## 2026-09-14 历史实施切片：资源调用的工作区观察失效（未验证）

任务 CAR-03 / CAR-06，详见
[CODING-RESOURCE-OBSERVATIONS-2026-09-14.zh.md](CODING-RESOURCE-OBSERVATIONS-2026-09-14.zh.md)。
Coding 将资源参数准备与宿主调用分开，本地格式拒绝不产生工作区失效记录；准备
完成后，在跨资源端口前持久化 Patch 重读义务和工作区观察失效，并刷新路径指令。
资源初始化可能启动 stdio 进程，故此处保守失效，不代表实际远端派发或修改成功。
同一 turn 内已完成的 Patch 重读遇到后续潜在副作用也重新失效；串行批次逐调用
复查重读门禁，非 Patch 副作用在调用前持久化义务。未自动重读、重试或运行检查。
Coding `host2-coding-loop71`，Nomi 保持 `host45`；无构建/测试、服务/模型调用、
迁移执行或 commit/push。整体目标仍未完成，旧检查不覆盖本轮改动。

## 2026-09-14 历史实施切片：Nomi 显式 MCP 图片入口（未验证）

任务 CAR-03 / CAR-07，详见
[ENGINE-NOMI-MCP-MEDIA-2026-09-14.zh.md](ENGINE-NOMI-MCP-MEDIA-2026-09-14.zh.md)。
Nomi mcp_resource_read 接入 format=image，复用平台 blob 摘要、有界图片准备和
清理回执。运行时将已核验模型支持与原生 vision 激活开关一次绑定，资源宿主独立
核对冻结视觉选择/primary route/constraints/代号；不伪造 Coding 激活或 causality。
返回 typed Image 转 Nomi 原生图片附件，scope 持有包括解码在内的完整任务。
这补齐上一切片的 Nomi 图片入口，不改变两个 Engine 各自的执行循环。
Coding `host2-coding-loop70`、Nomi `host45`；未构建/测试、运行服务或模型、执行迁移、
commit/push。尚无运行证据，整体目标仍未完成。

## 2026-09-14 历史实施切片：MCP 二进制描述与显式图片读取（未验证）

任务 CAR-03 / CAR-06 / CAR-07，详见
[ENGINE-MCP-MEDIA-2026-09-14.zh.md](ENGINE-MCP-MEDIA-2026-09-14.zh.md)。
MCP owner 接受有界 text/blob，普通分页和持久回执只投影二进制描述。共享可选
read_image 及 Coding format=image 接入原 retained resource owner，额外核对
视觉权限、primary ImageInput、原始 blob SHA，复用平台有界解码/重编码；无
隐式 base64 文本回退、客户端 URL/文件读取或自动重放。Nomi 本轮仅共享安全描述，
尚无显式资源像素入口。Coding `host2-coding-loop69`、Nomi `host44`，旧会话不热换。
未构建/测试、启动服务、调用模型、执行迁移或 commit/push；整体目标仍未完成。

## 2026-09-14 历史实施切片：模型能力驱动的输出预算（未验证）

任务 CAR-02 / CAR-06，详见
[CODING-MODEL-BUDGET-2026-09-14.zh.md](CODING-MODEL-BUDGET-2026-09-14.zh.md)。
Coding 不再统一压到 4096 输出 token：按平台候选能力交集、context/8 和 16384
取最小，未知模型保留原默认值。显式更小额度同时约束发送与压缩预留，主模型
边界核对冻结预算；固定预算说明进入模型上下文。不增加步数、权限或截断续接
次数。Coding `host2-coding-loop68`，Nomi 仍 `host43`。未构建/测试、调用模型、
执行迁移或 commit/push，尚无质量/费用/延迟实测证据，整体目标仍未完成。

## 2026-09-14 历史实施切片：长工具输出模型上下文预算（未验证）

任务 CAR-03 / CAR-06，详见
[CODING-TOOL-CONTEXT-2026-09-14.zh.md](CODING-TOOL-CONTEXT-2026-09-14.zh.md)。
Coding 按 canonical capability 对进程日志和 Git diff 正文作首尾摘录，保留操作
元数据，完整编码结果不超过 24 KiB 且必须比原结果小。原结果仍先进入既有证据与
档案路径，历史派生上下文使用同一策略。档案新增精确 call_id 筛选及 source 已有
界限说明，不重跑工具、不宣称保存全量输出。Coding `host2-coding-loop67`，Nomi
保持 `host43`。未运行构建/测试、服务、模型调用、迁移或 commit/push，整体未完成。

## 2026-09-14 历史实施切片：逐 owner 清理异常隔离（未验证）

任务 CAR-03 / CAR-04 / CAR-07，详见
[ENGINE-CLEANUP-ISOLATION-2026-09-14.zh.md](ENGINE-CLEANUP-ISOLATION-2026-09-14.zh.md)。
共享效果 witness、生产资源收尾和逐进程 cancel 增加 panic 隔离，失败不会跳过
后续正常返回的清理项。观察到的 scope 失败在下一次 await 前保留，等待取消或
后续成功不能抹除；不把捕获 panic 当作 reaped/清理成功。Coding inbox 关闭异常
仍尝试资源清理。Coding `host2-coding-loop66`，Nomi `host43`，exact binding 不热换。
未运行构建/测试、服务、模型调用、迁移或 commit/push；整体目标仍未完成。

## 2026-09-14 历史实施切片：并行结果逐项收取与模型排列（未验证）

任务 CAR-03 / CAR-06，详见
[CODING-PARALLEL-RESULTS-2026-09-14.zh.md](CODING-PARALLEL-RESULTS-2026-09-14.zh.md)。
Coding 并行只读批次逐项收取、校验和记录，路径指令检查仍先于搜索片段发布。
实际结果记录顺序与模型上下文排列分开；新增 ToolResultsOrdered 保存原 proposal
顺序，历史重建核对完整批次再采用该排列，中断未收取结果仍未知。原 Session/权限
冻结和平台任务清理不变。Coding `host2-coding-loop65`，Nomi 保持 `host42`。
未运行构建/测试、迁移或 commit/push，整体目标仍未完成。

## 2026-09-14 历史实施切片：Coding 逐项工具结果记录（未验证）

任务 CAR-03 / CAR-06，详见
[CODING-INCREMENTAL-RESULTS-2026-09-14.zh.md](CODING-INCREMENTAL-RESULTS-2026-09-14.zh.md)。
串行工具返回后先核对身份/结构并完成指令后处理，逐项记录结果，再进入下一效果；
Patch 成功清除恢复义务也移到合法结果持久化之后。后续取消/失败不吞掉已记录前缀，
不重复发布批次结果，不把部分完成当作任务成功。并行等待策略未改变。MCP 模板
目录同时补充复合值支持声明。Coding `host2-coding-loop64`，Nomi `host42`。
Git 专属凭据/网络传输仍缺失；未运行构建/测试或 commit/push，整体目标仍未完成。

## 2026-09-14 历史实施切片：MCP 列表/字典模板参数（未验证）

任务 CAR-03 / CAR-07 / CAR-09，详见
[ENGINE-MCP-COMPOSITE-TEMPLATES-2026-09-14.zh.md](ENGINE-MCP-COMPOSITE-TEMPLATES-2026-09-14.zh.md)。
共享模板端口支持字符串列表和字符串值字典；补齐 explode、命名/空值语义、稳定
字典顺序和输入/输出总预算。两个官方 Engine 共用参数 schema；平台在效果前拒绝
嵌套/null/数字、未知变量和复合前缀，不改变 exact-server 目录、收据及清理边界。
Coding `host2-coding-loop63`，Nomi `host41`。二进制/订阅及服务端授权生命周期仍
未完成；未运行构建/测试、迁移或 commit/push，整体目标仍未完成。

## 2026-09-14 历史实施切片：持久化旧回合工具历史加载（未验证）

任务 CAR-06，详见
[CODING-PERSISTED-TOOL-HISTORY-2026-09-14.zh.md](CODING-PERSISTED-TOOL-HISTORY-2026-09-14.zh.md)。
平台增加同用户/Session、固定当前 root 之前的收据分页；Coding 通过可选历史端口
每次加载一个旧回合，核对 exact binding 和事件结构后原子导入有界文本档案。
补齐初始模型窗口之外的持久化结果回读，明确来源、入档排序和最近回合去重；
不执行原工具或生成新完成/清理证据。全文索引、损坏/超预算日志跳过和跨会话读取
未实现。Coding `host2-coding-loop62`，Nomi `host40`。未运行构建/测试、迁移或
commit/push，整体目标仍未完成。

## 2026-09-14 历史实施切片：压缩后的工具原文按需回读（未验证）

任务 CAR-06，详见
[CODING-TOOL-ARCHIVE-2026-09-14.zh.md](CODING-TOOL-ARCHIVE-2026-09-14.zh.md)。
Coding 在模型窗口外保留有界回合内工具文本档案，新增字面搜索与 exact ID 分页
回读。档案在初次压缩前接收宿主已提供历史，之后收录回合结果；容量、截断、淘汰、
来源及原错误均显式呈现。回读不执行原工具、不提供新完成证据或 Patch 重读证明。
未实现全量数据库/跨重启检索。Coding `host2-coding-loop61`，Nomi 保持 `host39`。
未运行构建/测试或 commit/push，整体目标仍未完成。

## 2026-09-14 历史实施切片：多服务器 MCP 资源接入（未验证）

任务 CAR-03 / CAR-07 / CAR-09，详见
[ENGINE-MCP-RESOURCE-SERVERS-2026-09-14.zh.md](ENGINE-MCP-RESOURCE-SERVERS-2026-09-14.zh.md)。
两个官方 Engine 和社区共享端口支持最多 16 台冻结资源服务器，可与逐工具服务器
组合；多服务器资源请求必须明确 server_id，单服务器省略兼容。工具仍只有自己的
exact server，纯资源服务器不继承其他工具的 invoke。分页绑定服务器和查询，核对
owner envelope；产品资源选择同步支持，不改变 Engine 注册或 Session 冻结策略。
Coding `host2-coding-loop60`，Nomi `host39`。没有运行验证或 commit/push；其他协议、
生态生命周期及恢复能力仍有缺口，整体目标未完成。

## 2026-09-14 历史实施切片：历史批次完整性与结果顺序（未验证）

任务 CAR-06，详见
[CODING-HISTORY-BATCHES-2026-09-14.zh.md](CODING-HISTORY-BATCHES-2026-09-14.zh.md)。
历史重建按模型步和唯一调用身份配对，重复结果不覆盖；继续/压缩/普通完成前必须
有完整批次，只有中断末尾可以保留未知结果。真实结果遵循持久化发布顺序，不按
模型声明结束顺序重排；恢复失败不部分修改调用者上下文。保留宿主零步中断代表
未知计数的既有语义，不推断没有执行。Coding `host2-coding-loop59`，Nomi `host38`。
没有运行验证或 commit/push，整体目标仍未完成。

## 2026-09-14 历史实施切片：工具派发与完成证据分离（未验证）

任务 CAR-03 / CAR-06，详见
[CODING-DISPATCH-ACCOUNTING-2026-09-14.zh.md](CODING-DISPATCH-ACCOUNTING-2026-09-14.zh.md)。
Coding 在真实工具端口调用前记录当前批次的尝试事实，区分引擎暂缓与派发后失败。
未派发调用仍触发重新规划，但不虚增工作区 epoch、命令观察或 Patch 恢复读取；
完成观察携带 invocation_attempted，未尝试不能支撑成功结论。该事实不代替平台
owner 派发/回收证明。Coding `host2-coding-loop58`，Nomi 保持 `host38`。
未运行验证或 commit/push，整体目标仍未完成。

## 2026-09-14 历史实施切片：续接/复核后的工具原文尾部（未验证）

任务 CAR-06，详见
[CODING-CONTEXT-FOLLOWUP-2026-09-14.zh.md](CODING-CONTEXT-FOLLOWUP-2026-09-14.zh.md)。
压缩定位最近完整工具批次，不再被后续普通/部分回答遮住；按原序保留批次和后续
输入/说明，配对不完整仍拒绝。引擎说明计入 32 KiB 可选尾部预算，实际 accepted
inputs 单独强制保留；必需图片和已过后续模型边界的图片区分处理。补齐消息条数
预检/最终限制，避免压缩后再次裁剪必需输入。Coding `host2-coding-loop57`，Nomi
保持 `host38`。未运行验证或 commit/push，整体目标仍未完成。

## 2026-09-14 历史实施切片：进程 owner 中途回收凭据（未验证）

任务 CAR-04 / CAR-06，详见
[ENGINE-PROCESS-BARRIERS-2026-09-14.zh.md](ENGINE-PROCESS-BARRIERS-2026-09-14.zh.md)。
平台在进程操作前持久化确切 owner 派发，全部持有进程确认回收后写入对应 barrier；
后续派发使旧证明失效。精确重启恢复可关闭有完整证明的中断历史，不重放命令。
启动失败保留结构化清理信息，未知启动不能靠空 map 解封。并修复截断提议 ID 在
历史缓冲中丢失的问题。Coding `host2-coding-loop56`、Nomi `host38`；未运行验证或
commit/push。运行中崩溃缺乏进程树证明等场景仍隔离，整体目标仍未完成。

## 2026-09-14 历史实施切片：隐藏/签名推理的活跃工具循环续传（未验证）

任务 CAR-02 / CAR-06，详见
[ENGINE-PRIVATE-REASONING-2026-09-14.zh.md](ENGINE-PRIVATE-REASONING-2026-09-14.zh.md)。
共享类型区分 Anthropic 签名块和 redacted_thinking，保留空文本有效签名、块边界与
原始隐藏数据；Coding 仅将可见文本发布到事件/UI，完整块用于活跃循环同协议回传。
其他协议明确拒绝，摘要排除私有载荷，输出截断沿用整步作废策略。原生推理文本现在
在块闭合后发布。当前 Coding `host2-coding-loop55`、Nomi `host37`；没有运行验证，
未 commit/push。跨重启、其他专有协议及生态生命周期仍有剩余，整体目标未完成。

## 2026-09-14 历史实施切片：Coding 输出上限的有界续接（未验证）

任务 CAR-02 / CAR-06，详见
[CODING-OUTPUT-LIMIT-2026-09-14.zh.md](CODING-OUTPUT-LIMIT-2026-09-14.zh.md)。
明确 MaxOutputTokens 时整批工具提议均不执行，先记录作废事实，保留可见文本并清除不完整
续传链，再回正常边界以新 operation 续接；每 turn 最多两次，额度和最大步数不提升。
历史与 UI 清除经核对的未执行提议，不伪造工具结果。OpenAI Chat 移除坏 JSON 修补，
Anthropic/Responses 可核对的截断终态已接入；没有 block/item closure 的序列仍失败。
当前 Coding `host2-coding-loop54`、Nomi `host36`。未运行验证或 commit/push，整体目标仍未完成。

## 2026-09-14 历史实施切片：Anthropic 原生工具块及签名循环（未验证）

任务 CAR-02 / CAR-06，详见
[ENGINE-ANTHROPIC-LOOP-2026-09-14.zh.md](ENGINE-ANTHROPIC-LOOP-2026-09-14.zh.md)。
生产 attempt 单独关联 Anthropic tool_use/input_json_delta/block_stop，消息结束前核对块与
stop_reason；分片推理签名按块拼接，累计用量合并后只发布一次。Responses/Anthropic 共用
有界 wire 计数器；请求侧显式输出上限、合法 thinking 预算及工具选择约束已写入。
当前 Coding `host2-coding-loop53`、Nomi `host35`。redacted/adaptive/server-tool 与专有协议等
仍未完成；旧规范化 Anthropic 夹具不证明真实请求能力。本轮未运行验证或 commit/push。

## 2026-09-14 历史实施切片：原生 Responses 工具循环与推理续传（未验证）

任务 CAR-02 / CAR-06，详见
[ENGINE-RESPONSES-LOOP-2026-09-14.zh.md](ENGINE-RESPONSES-LOOP-2026-09-14.zh.md)。
共享 Broker 增加 attempt 内的原生 Responses 状态机，分别关联 item/call、核对参数增量及完成快照，
只将完整成功调用映射为 ToolCalls。Coding 保留完整推理块和活跃回合的加密续传；函数图片/音频
结果以结构化内容回送，失败标记保留，工具 schema 不再被错误声明为 provider strict 子集。
当前 Coding `host2-coding-loop52`、Nomi `host34`。没有新增权限/owner 或热加载。
其他原生协议、部分输出呈现、跨重启与生态生命周期仍未完成；未运行验证或 commit/push。

## 2026-09-14 历史实施切片：模型流内错误与每次请求的解码隔离（未验证）

任务 CAR-02 / CAR-06，详见
[ENGINE-MODEL-STREAM-ERRORS-2026-09-14.zh.md](ENGINE-MODEL-STREAM-ERRORS-2026-09-14.zh.md)。
共享 Broker 分类命名/根部错误及 Responses response.failed，明确超限可进入 Coding
既有一次性压缩恢复；已交付语义输出或错误帧混有未交付输出时不能自动恢复。
官方协议改为每次 transport attempt 独立解码，避免失败残留影响后续匿名帧或 Session。
当前 Coding `host2-coding-loop51`、Nomi `host33`。自定义旧有状态 adapter 仍需自行迁移，
更多专有错误未补齐。未运行构建、测试、服务或迁移，未 commit/push；整体目标未完成。

## 2026-09-14 历史实施切片：Coding 压缩后的近期工具交换保留（未验证）

任务 CAR-06，详见
[CODING-CONTEXT-TAIL-2026-09-14.zh.md](CODING-CONTEXT-TAIL-2026-09-14.zh.md)。
预算内保留最近完整工具交换的原始实时结果，调用/结果按整批匹配，普通文本交换
上限 32 KiB；不满足预算则使用包含该批次的摘要。待交付图片仍强制保留。
保留追加输入相对交换的顺序及重复次数；持久事件仅引用调用 ID，历史使用已有的
有界结果，不重新执行工具或加载图片。当前 Coding `host2-coding-loop50`，Nomi
保持 `host32`。未运行构建、测试、服务或迁移，未 commit/push；整体目标未完成。

## 2026-09-14 历史实施切片：Coding 上下文超限后的有界压缩恢复（未验证）

任务 CAR-02 / CAR-06，详见
[CODING-CONTEXT-LIMIT-RECOVERY-2026-09-14.zh.md](CODING-CONTEXT-LIMIT-RECOVERY-2026-09-14.zh.md)。
生产调用层从完整有界的错误响应中识别明确超限代码，通过共享 Broker 传递类型化事实。
Coding 只在没有语义输出且尚有模型步数时允许一次压缩续接，输入估算目标锚定被拒请求
的 75%；保留必需约束/待交付图片，预算不够则失败。新请求使用新 operation，不重放
工具、不换 Engine、不自行改路由。当前 Coding `host2-coding-loop49`、Nomi `host32`。
其他协议的自然语言/流内超限错误尚未补齐分类，未运行构建、测试、服务或迁移，
未 commit/push；整体目标未完成。

## 2026-09-14 历史实施切片：MCP 工具明确失败的结算与 Engine 反馈（未验证）

任务 CAR-06 / CAR-07，详见
[ENGINE-MCP-TOOL-OUTCOMES-2026-09-14.zh.md](ENGINE-MCP-TOOL-OUTCOMES-2026-09-14.zh.md)。
有效 `isError: true` 与匹配 ID 的 tools/call RPC error 不再被直接当作未知调用；
仅完成独立清理后返回，平台先结算永久凭据，再经 Kernel 返回明确失败。Nomi/Coding
都保留失败语义，永久观察及恢复裁剪保留 isError。坏响应/超时/初始化/目录和清理
失败仍隔离，不新增重试或回滚声明。当前 Coding `host2-coding-loop48`、Nomi `host31`。
未运行构建、测试、服务或迁移，未 commit/push；整体目标未完成。

## 2026-09-14 历史实施切片：资源明确失败与未知效果分离（未验证）

任务 CAR-06 / CAR-07，详见
[ENGINE-RESOURCE-OUTCOMES-2026-09-14.zh.md](ENGINE-RESOURCE-OUTCOMES-2026-09-14.zh.md)。
资源 owner 将有效初始化后的能力缺失、完整目录缺项和关联 RPC error 记录为明确失败，
仅在协议清理完成后结算。两种官方 Engine 都收到工具失败；共享每页标记和永久恢复
元数据保留该事实，不误认数据读取成功。超时、坏响应、初始化/清理失败仍保持隔离，
未添加自动重试、回滚声明或人工清除。当前 Coding `host2-coding-loop47`、Nomi `host30`。
未构建、测试、调用服务或运行迁移，未 commit/push；整体目标仍未完成。

## 2026-09-14 历史实施切片：MCP 资源模板与受约束参数化读取（未验证）

任务 CAR-06 / CAR-07，详见
[ENGINE-MCP-TEMPLATES-2026-09-14.zh.md](ENGINE-MCP-TEMPLATES-2026-09-14.zh.md)。
共享 owner 支持完整有界模板目录，模板在同一协议会话内精确匹配并展开为读取 URI；
不会使客户端访问任意 URL。Coding 独立控制入口和 canonical Nomi ToolSearch 均已接入，
沿用永久效果凭据、取消持有、清理和分页摘要约束。支持受界限约束的 RFC 6570 标量
profile，复合变量不支持。两个入口补齐 active state 的 Snapshot 身份检查。
当前 Coding `host2-coding-loop46`、Nomi `host29`。没有构建、测试、服务调用或迁移，
没有 commit/push。二进制/订阅/主动授权/人工未知恢复等仍待完成，整体目标未完成。

## 2026-09-14 历史实施切片：Nomi 资源入口迁入共享平台 owner（未验证）

任务 CAR-06 / CAR-07，详见
[ENGINE-NOMI-RESOURCES-2026-09-14.zh.md](ENGINE-NOMI-RESOURCES-2026-09-14.zh.md)。
canonical Nomi Session 的 MCP 文本资源调用使用共享 owner、分页和永久效果凭据；
保留 Nomi 工具名及 ToolSearch 策略，按需身份绑定 Snapshot/资源摘要，真实调用核对
Kernel active generation。取消保留效果任务，启动阶段不连接资源服务器；配置文件
不能重引入这条原生旁路。允许单服务器资源与冻结工具组合，拒绝混用原生全工具代理。
资源每回合 64 次限制下沉到凭据准入。当前 Coding `host2-coding-loop45`、Nomi `host28`。
旧非 canonical Nomi 兼容路径未迁移，模板/二进制/订阅/主动请求授权等仍待完成。
未执行构建、测试、服务调用或迁移，未 commit/push，整体目标未完成。

## 2026-09-14 历史实施切片：MCP 文本资源端口与 Coding 上下文接入（未验证）

任务 CAR-06 / CAR-07，详见
[ENGINE-MCP-RESOURCES-2026-09-14.zh.md](ENGINE-MCP-RESOURCES-2026-09-14.zh.md)。
共享平台资源端口按已选且激活的 `mcp.resource` 和唯一服务器 connect/read binding
授权；HTTP/SSE/stdio 共用完整有界目录与文本读取。Coding 增加独立资源控制入口，
采用字节分页和内容摘要续页约束。任务与永久 MCP 凭据由平台持有，取消不丢失收尾；
重启按 claimed 模型记录和资源派发凭据匹配，只关闭历史，不重放资源操作。
资源型 Agent 的服务器选择依据已解析 binding 校验，前端与后端统一单服务器限制。
当前 Coding `host2-coding-loop44`、Nomi `host27`。Nomi 新资源入口迁移、模板/二进制/
订阅和服务器主动请求授权仍未完成。未执行构建、测试、服务调用或迁移，未 commit/push。

## 2026-09-14 历史实施切片：Git 源回合归属与 Coding 本地目标接入（未验证）

任务 CAR-05 / CAR-07，详见
[ENGINE-GIT-ATTRIBUTION-2026-09-14.zh.md](ENGINE-GIT-ATTRIBUTION-2026-09-14.zh.md)。
新增 099 迁移源码，将 Git 纳入共享永久效果凭据，保留旧记录与触发器；按规范工作区
对 pending push 建立跨 Session 唯一限制。派发前记录所属输入/回合与绑定摘要，
明确返回后才落盘结算；未知、取消和落盘失败保持隔离。Nomi 按永久源输入记录判断
重放，Coding 接入显式选中的 local/file push 和对应重启历史审计，不自动重发工具。
`NotApplied` 不等于无对象传输，因此不自动重试。当前 Coding `host2-coding-loop43`、
Nomi `host26`。网络凭据、人工未知效果解决和全局工作区事务仍未实现。
未执行任何 push、构建、测试、服务调用或数据库迁移，整体 Engine 目标尚未完成。

## 2026-09-14 历史实施切片：Git worker 生命周期与目标固定（未验证）

任务 CAR-05 / CAR-07，详见
[ENGINE-GIT-LIFECYCLE-2026-09-14.zh.md](ENGINE-GIT-LIFECYCLE-2026-09-14.zh.md)。
平台 push owner 持有 blocking worker 的共享退出凭据，取消/超时不丢失任务；独占锁
保持到真实退出，未知结果和回执落盘失败持续隔离。执行使用已解析的 local/file 目标
与确定 commit OID；拒绝 libgit2 URL rewrite 将目标改到其他仓库或网络协议。
共享 Engine 在工具/回合/上下文边界检查同工作区的 Git 状态，清理使用准入时固定的
路径身份；Nomi 的已选 push Session 增加效果清理及模型边界检查，不凭空授权自动重放。
当前 Coding `host2-coding-loop42`、Nomi `host25`。Coding push 尚未开放：按用户源输入
归属的永久回执、跨重启效果判断和网络凭据 owner 仍待接入；没有执行任何 push。
未运行构建/测试/服务验证，未 commit/push，整体 Engine 目标尚未完成。

## 2026-09-14 历史实施切片：Coding 追加附件及冻结 Skill 提示（未验证）

任务 CAR-06 / CAR-07，详见
[CODING-STEERING-MEDIA-2026-09-14.zh.md](CODING-STEERING-MEDIA-2026-09-14.zh.md)。
Conversation 不再统一拒绝追加附件，按 Engine 的上下文追加支持声明准入；共享 SDK
默认拒绝，Coding 开放经持久化 receipt 核对的文件引用、已选 Skill 提示和有界图片。
队列/工具派发共用回合锁；图片仅存活于当前模型上下文，历史保留引用与交付观察，
不重新读取文件或重发旧输入。前端交付失败保留待确认草稿并阻止自动改发下一轮，
恢复、重排或点击继续均不能自动清除该标记。
当前 Coding `host2-coding-loop41`、Nomi `host24`（共享入口契约也有变更）。
MiniApp Service 启停仍归平台，不新增 Engine 生命周期 owner；其他生态资源、
未知效果人工处置与实际验证仍待完成。未运行构建/测试/服务验证，未 commit/push。

## 2026-09-14 历史实施切片：协作任务输入与跨 Engine 工具约束（未验证）

任务 CAR-07，详见
[ENGINE-ATTEMPT-CONSTRAINTS-2026-09-14.zh.md](ENGINE-ATTEMPT-CONSTRAINTS-2026-09-14.zh.md)。
canonical Attempt 的 brief/step_spec 进入持久化 user 输入，不再覆盖 Agent 固定指令；
版本化 Session 约束经可信创建持久化，重试/复用/回合准入核对。Coding/社区共享宿主过滤
工具表并在真正派发前核对 exact binding；Coding 激活检查依赖。Nomi 在资源 materialize
前收窄并在动态注册后保留原生权限上限。受限任务不启动通用生态 context/lifecycle。
当前 Coding `host2-coding-loop40`、Nomi `host23`。ReadShell 明确允许有写入效果的命令，
不宣称 OS 沙箱；旧消费者迁移、生态生命周期、未知效果处理和实际验证仍待完成。
未运行验证，未 commit/push。

## 2026-09-14 历史实施切片：消费者 Binding 保留与统一 Engine 准入（未验证）

任务 CAR-07，详见
[ENGINE-CONSUMER-BINDING-2026-09-14.zh.md](ENGINE-CONSUMER-BINDING-2026-09-14.zh.md)。
补齐 Cron 冻结投影丢失完整 Binding、消费者创建直达底层服务的问题；canonical 快照
携带可回查引用，由 Session host 核对保存 artifacts 后解析 Engine。创建键重试沿用
已创建 exact Engine，并发复用同时核对不可变 Agent metadata。底层拒绝带 canonical
Binding 但缺失宿主 Engine 准入的快照，不悄悄回退。
当前 Coding `host2-coding-loop39`、Nomi `host22`。旧无 Binding 快照不自动迁移；
AgentExecution 的旧工具白名单/brief overlay 仍需适配，不能算完整消费者支持。
未运行验证，未 commit/push。

## 2026-09-14 历史实施切片：共享 HTTP MCP 目录发现（未验证）

任务 CAR-03 / CAR-07，详见
[ENGINE-MCP-HTTP-DISCOVERY-2026-09-14.zh.md](ENGINE-MCP-HTTP-DISCOVERY-2026-09-14.zh.md)。
设置页 HTTP/SSE 网络目录发现统一进入共享 owner；删除无界 HTTP 读取、单页目录与
忽略 initialized 错误的旧路径。完整分页、协议截止时间、独立清理、严格配置请求头与
有界认证提示均复用/接入。当前 Coding `host2-coding-loop38`、Nomi `host21`。
搜索命中指令能力已保留；未运行验证，未 commit/push，其他资源生命周期仍待实现。

## 2026-09-14 历史实施切片：Coding 搜索命中指令加载（未验证）

任务 CAR-05 / CAR-06，详见
[CODING-SEARCH-INSTRUCTIONS-2026-09-14.zh.md](CODING-SEARCH-INSTRUCTIONS-2026-09-14.zh.md)。
搜索结果进入模型前严格解码命中路径并通过现有 fs.read 加载目录规则；不完整搜索仍可
返回已有合法命中，规则无法完整加载则明确暂扣片段且不授予额外权限。串行后续调用
推迟重新规划，只读并行在结果发布前补齐上下文；事件持久化失败不再降级为读取警告。
当前 Coding `host2-coding-loop37`、Nomi `host20`。未运行验证，未 commit/push；
不是原子文件快照、不透明 shell 访问治理或整体 CAR 完成。

## 2026-09-14 历史实施切片：共享 MCP stdio 进程、执行与目录发现（未验证）

任务 CAR-03 / CAR-05 / CAR-07，详见
[ENGINE-MCP-STDIO-2026-09-14.zh.md](ENGINE-MCP-STDIO-2026-09-14.zh.md)。
共享 owner/product catalog 接入 stdio，复用完整目录/schema/结果协议与效果记录。
本地 MCP 使用平台受管进程树、最小环境、子进程 PATH 解析及有界 stdin/stdout；
工具返回仍需全树清理成功，取消/超时不自动重启。设置页 stdio 目录发现同样接入，
清理未知不再报告成功。当前 Coding `host2-coding-loop36`、Nomi `host20`。
未 commit/push，未运行验证；不是 OS 沙箱、效果回滚、持久 MCP 会话或整体 CAR 完成。

## 2026-09-14 历史实施切片：共享 MCP legacy SSE 执行与目录发现（未验证）

任务 CAR-03 / CAR-05 / CAR-07，详见
[ENGINE-MCP-LEGACY-SSE-2026-09-14.zh.md](ENGINE-MCP-LEGACY-SSE-2026-09-14.zh.md)。
共享 owner 接入明确配置的 SSE，同源消息端点、独立 POST 回执/关联响应、有界持续流、
完整目录分页及已有未知效果处理；产品目录物化与运行时绑定同时接通。设置页 SSE 目录
发现改用共享解析，超时不遗留后台读取任务。无重连、重放或跨源跳转。
当前 Coding `host2-coding-loop35`，Nomi `host19`。未 commit/push，未运行验证。
释放 SSE 不是远端服务关闭或效果回滚；stdio、其他生态生命周期和恢复仍待完成。

## 2026-09-14 历史实施切片：Patch 恢复状态跨回合接线（未验证）

任务 CAR-05 / CAR-06，详见
[CODING-PATCH-RESUME-2026-09-14.zh.md](CODING-PATCH-RESUME-2026-09-14.zh.md)。
Coding 新增永久恢复状态事件，Patch 调用前记录目标，成功/完整重读后记录清除；取消或
崩溃不丢掉约束。新回合独立于聊天消息/历史窗口加载 exact Session 状态，部分历史读取
不当当前证据；状态缺失、绑定不符或来源未结束拒绝继续。无新增数据库表/迁移。
当前 Coding `host2-coding-loop34`，Nomi 保持 `host18`。未 commit/push，未运行验证。
人工隔离处置、跨 Session 协调、跨启动进程证明及其他生态能力仍未完成，整体 CAR 尚未完成。

## 2026-09-14 历史实施切片：Patch 失败回执与重新观察约束（未验证）

任务 CAR-03 / CAR-05 / CAR-06，详见
[CODING-PATCH-RECOVERY-2026-09-14.zh.md](CODING-PATCH-RECOVERY-2026-09-14.zh.md)。
平台区分发布前/后错误，返回逐文件发布及恢复观察，新建目标保留而不竞态删除；失败回执
经既有错误 journal 持久化，结算未确认不吞错并阻止同范围新键重试。Coding 对实际失败
Patch 建立有界待重读状态，禁止旧批次读取解除、后续效果绕过及未观察目标的完成声明。
当前 Coding `host2-coding-loop33`、Nomi `host18`。未 commit/push，未运行验证。
跨回合结构化恢复、多文件事务、外部编辑/重命名隔离及其他生态能力仍未完成，整体 CAR 尚未完成。

## 2026-09-14 历史实施切片：Patch 文本语义与 File/VCS 参数契约（未验证）

任务 CAR-03 / CAR-05，详见
[CODING-PATCH-CONTRACTS-2026-09-14.zh.md](CODING-PATCH-CONTRACTS-2026-09-14.zh.md)。
修复生产标准工具 schema 被 Wave2 open object 覆盖的缺口；补全 write/patch/Git 参数及
Kernel 前置检查。Patch 保留 CRLF/LF/CR、BOM 和 EOF 政策，支持中部纯插入/删除，读取、
搜索与 patch 行号一致；新建意图不转成覆盖，发布前核对预期字节，结果提供版本摘要。
当前 Coding `host2-coding-loop32`、Nomi `host17`。未 commit/push，未运行验证。
多文件事务、外部并发完全隔离、恢复及其他生态生命周期仍未完成，整体 CAR 尚未完成。

## 2026-09-14 历史实施切片：路径指令发现与递归刷新（未验证）

任务 CAR-05 / CAR-06，详见
[CODING-INSTRUCTION-SCOPES-2026-09-14.zh.md](CODING-INSTRUCTION-SCOPES-2026-09-14.zh.md)。
平台 fs.read 增加有界 instruction_scope 元数据及文本专用 typed absence；Coding 在工具前
发现真实目标规则，递归检查删除/暂存范围，操作后重扫已登记范围并清除失效规则。
不完整扫描阻止操作，别名要求重新提交而不静默改写；只读发现的新规则同样清除 provider
续接和旧完成报告。当前 Coding `host2-coding-loop31`、Nomi `host16`。
未 commit/push，未运行验证。任意 shell 实际路径、并发重命名隔离、安全恢复及其他生态
生命周期仍未完成，整体 CAR 尚未完成。

## 2026-09-14 历史实施切片：共享 Robot 工具、lease 与动态上下文（未验证）

任务 CAR-05 / CAR-06 / CAR-07，详见
[ENGINE-ROBOT-PORT-2026-09-14.zh.md](ENGINE-ROBOT-PORT-2026-09-14.zh.md)。
公共 Session 工具宿主接入三类 Robot 工具及 link/audio 共享 lease；Coding 接入初始与按需
选择，禁止自动选择设备或扩大授权。设备原始 schema 与名称一起冻结并在重连调用时核对。
robot.vision 通过独立公共 context 端口提供现有近期观察；Coding 每轮替换动态槽，观察变化
清除 provider parent 并使旧完成 review 失效。恢复核对匹配设备名的 owner 回执，不重放。
当前 Coding `host2-coding-loop30`、Nomi `host15`；未 commit/push、未运行验证。
返回结果／释放本地 lease 不代表物理静止。其他生态生命周期、安全恢复与实际执行证据
仍未完成，整体 CAR 尚未完成。

## 2026-09-14 历史实施切片：共享 MiniApp 工具端口与 Coding 接入（未验证）

任务 CAR-05 / CAR-06 / CAR-07，详见
[ENGINE-MINIAPP-PORT-2026-09-14.zh.md](ENGINE-MINIAPP-PORT-2026-09-14.zh.md)。
公共 EngineKernelSession 已提供 exact MiniApp schema/工具计划与真实 Service 调用端口；
Coding 合入初始及按需 MiniApp 工具，Nomi 复用同一平台调用／凭据实现。初始 MiniApp 加入
共享 active set，公共模型 journal 在 claim 时拒绝 hosted pending；清理与激活同样检查。
恢复仅闭合有匹配 MiniApp owner 凭据的中断历史，不重放。当前 Coding `host2-coding-loop29`、
Nomi `host14`。未 commit/push，未运行验证。Robot 的其他 Engine 接入、非工具生命周期、
人工解隔离与真实执行证据仍未完成，整体 CAR 尚未完成。

## 2026-09-14 历史实施切片：MiniApp/Robot 宿主工具效果归属（未验证）

任务 CAR-05 / CAR-06 / CAR-07，详见
[ENGINE-HOSTED-EFFECTS-2026-09-14.zh.md](ENGINE-HOSTED-EFFECTS-2026-09-14.zh.md)。
Nomi scope 保留 MiniApp/动态工具任务；真实 app 适配器在派发前写 exact-turn 凭据，
记录动作／输入摘要和有界结果。未知结果阻断模型／工具推进、自动重试、编辑重发及启动恢复；
取消等待关闭当前回合入口，结果未知关闭 Session。returned 不代表物理静止或 Service 退出。
修复数据库 trigger 契约的声明顺序依赖。migration 098 未执行；当前 Coding
`host2-coding-loop28`、Nomi `host13`。未 commit/push，未运行验证。
Coding/社区 MiniApp/Robot 工具面、非工具生命周期、人工解隔离等仍未完成，整体 CAR 尚未完成。

## 2026-09-14 历史实施切片：Skill 冻结发布与多 Engine 消费（未验证）

任务 CAR-06 / CAR-07，详见
[ENGINE-SKILL-PUBLICATION-2026-09-14.zh.md](ENGINE-SKILL-PUBLICATION-2026-09-14.zh.md)。
Skill Library 详情可预览源文件与摘要，再显式生成不可变 Plugin 候选；不自动启用、修改 Agent
或迁移 Session。沿用现有 Project/revision 检查，断连不自动重试，不宣称跨步骤原子提交。
共享 exact Skill 读取与纯资源类型已抽出，Coding、Nomi 和编译期社区 Engine 均有消费端口；
Nomi 补上有界正文／资源索引、分页文本和 gated 图片工具。Engine 加载政策保持编译期接入。
当前 Coding `host2-coding-loop27`、Nomi `host12`。未 commit/push，未运行验证。
多消费者生态生命周期、效果结算、安全恢复与任务语义核对仍未完成，整体 CAR 尚未完成。

## 2026-09-14 历史实施切片：显式跨回合任务延续（未验证）

任务 CAR-06 / CAR-07，详见
[CODING-TASK-CONTINUATION-2026-09-14.zh.md](CODING-TASK-CONTINUATION-2026-09-14.zh.md)。
宿主仅投影最近 exact-bound 闭合回合，Coding resume_task 引用当前输入显式续接完整需求账本，
保留最早来源并强制重新规划；旧完成说明只作历史，不恢复工具／进程／验证证据。
当前输入可显式修改导入的历史范围；候选独立保留于压缩上下文，必需状态超预算时明确失败。
当前 Coding `host2-coding-loop26`，Nomi `host11`。未 commit/push、未运行验证；
语义判定、Skill 冻结接入、生态生命周期及安全恢复仍未完成，整体 CAR 尚未完成。

## 2026-09-14 历史实施切片：工作区图片及待观察像素（未验证）

任务 CAR-05 / CAR-06 / CAR-07，详见
[CODING-WORKSPACE-IMAGES-2026-09-14.zh.md](CODING-WORKSPACE-IMAGES-2026-09-14.zh.md)。
fs.read(format=image) 经平台文件授权／有界解码，共享 Engine 工具端口在日志之前转换为
真实图片输入；须 active llm.vision 及 exact primary ImageInput，Coding 只允许单调用 batch。
压缩保留最新待观察图片的完整 call/result 配对，预算不足明确失败，不在首次模型输入前丢图。
当前 Coding `host2-coding-loop25`，Nomi `host11`。未 commit/push、未运行验证。
原分辨率／裁剪和其他媒体格式未支持；Skill 冻结接入、生态生命周期与安全恢复仍未完成。

## 2026-09-14 历史实施切片：工作区文本分页与搜索（未验证）

任务 CAR-05 / CAR-06，详见
[CODING-TEXT-PAGES-2026-09-14.zh.md](CODING-TEXT-PAGES-2026-09-14.zh.md)。
平台 `fs.read` 已接严格参数、UTF-8 分页、整文件摘要续读和完整 JSON 页预算；读取仍经
Session workspace owner，不赋予引擎任意路径权限。Coding 仓库指令按游标／摘要完整拼接，
拒绝变化、超限或部分规则。搜索改为 fresh scan、有界源字节／结果／遍历预算，披露跳过及
不完整原因，命中片段携带源码偏移／摘要供续读。当前 Coding `host2-coding-loop24`，Nomi `host10`。
旧 Skill Library 缺少 frozen contribution／revision 链路，仍待平台导入与版本锁接入；
没有用可变目录读取绕过它。未 commit/push，未运行验证；整体未完成。

## 2026-09-14 历史实施切片：交互命令观察链（未验证）

任务 CAR-04 / CAR-06，详见
[CODING-INTERACTIVE-EVIDENCE-2026-09-14.zh.md](CODING-INTERACTIVE-EVIDENCE-2026-09-14.zh.md)。
命令记录保留原始 launch epoch，并另记同一进程的连续交互来源 epoch；正常 stdin/EOF/resize
不再必然使最终成功退出永久失去时效资格。只有此前未过期、独占已知运行进程且请求/结果 ID
一致的成功交互可延续来源链。失败、其他修改、并发重叠和来源省略不能被后续交互洗成有效。
终态观察携带启动/最多 8 个交互调用 ID；完成上下文保留有界链，观察窗口增加 32KiB 上限。
仍须真实退出/清理证明，且不推断命令内部测试时序、测试语义或任务成功。
当前 Coding build 为 `host2-coding-loop23`，Nomi 保持 `host9`。未 commit/push，未运行验证。
跨启动进程证明、安全恢复及其他未完成能力不因此改变状态。

## 2026-09-14 前序实施：来源关联需求账本与完成覆盖（未验证）

任务 CAR-06 / CAR-07，详见
[CODING-REQUIREMENTS-2026-09-14.zh.md](CODING-REQUIREMENTS-2026-09-14.zh.md)。
update_plan 新增只增不减的 requirements：每个 ID 固定描述及已接受输入引用，
禁止用同一 ID 改写原始记录；省略旧条目不会删除它们。每条当前回合已接受输入至少须有来源记录。
report_completion 同时覆盖当前计划与全部已登记需求，不能通过删改计划步骤静默丢项。
scope_changed 需要晚于原需求的真实已接受输入引用，并在最终输出披露原义务和范围变化；
引用匹配仅证明来源，不独立判断用户是否真的撤销需求。工具观察仍须通过现有时效/结算检查。
该切片 Coding build 为 `host2-coding-loop22`，Nomi 为 `host9`。没有 commit/push，未运行验证。
仍缺独立的需求提取完整性/语义相关性证明；此实现不是自动验收器，也不授权运行用户排除的测试。
协议、生态与安全恢复等其他剩余能力继续保持进行中。

## 2026-09-14 前序实施：Coding 图片 Skill 资源（未验证）

任务 CAR-06 / CAR-07，详见
[CODING-SKILL-IMAGES-2026-09-14.zh.md](CODING-SKILL-IMAGES-2026-09-14.zh.md)。
冻结 Skill 资源新增 PNG/JPEG/WebP 输入，宿主在制品清单/摘要校验后限量解码与重新编码，
模型用已有 read_context_resource 精确 ID 按需读取图片。文本资源继续分页，脚本不执行。
每 Session 最多 4 张/每源文件 4MiB，图片输出单独受大小预算约束，每次只准单调用读取。
active llm.vision 与 exact primary route 的 ImageInput 共同决定可读性；已有按需激活
成功持久化后更新该状态和资源索引。同 generation 不能改变权限，Broker 每次发送仍复核。
历史记录继续省略图片二进制，重放/摘要不把描述符当作图片内容。
该切片 Nomi build 为 `host9`，Coding build 为 `host2-coding-loop21`，新增实现未验证。
没有 commit/push，未运行构建、测试、评测或外部服务调用。非制品 Skill 库、其他二进制格式、
工作区图片工具以及其余协议/生态/恢复能力仍需推进，不能据此认定整体完成。

## 2026-09-14 前序实施：HTTP MCP 受限服务器消息生命周期（未验证）

任务 CAR-03 / CAR-05 / CAR-07，详见
[ENGINE-MCP-PROTOCOL-2026-09-14.zh.md](ENGINE-MCP-PROTOCOL-2026-09-14.zh.md)。
共享 HTTP owner 将服务器请求、通知与工具结果分离；支持 ping，并对未授权的
sampling/elicitation/roots/其他请求返回固定协议错误，不扩大 Engine 权限。
应答复用同一 endpoint、凭据和 Session，限定全事务 64 次服务器请求，禁止递归消息流。
SSE 增量解析支持跨 chunk CRLF、CR/LF、BOM、多行 data；通知不再发送 id:null。
严格校验空 202 acknowledgment、JSON/SSE Content-Type、响应字段存在性及 ID 类型。
收到工具目录变更或当前请求取消时中止，已派发效果仍按未知处理，不自动重试。
该切片 Nomi build 为 `host8`，Coding build 为 `host2-coding-loop20`；Nomi 摘要补入共享 owner 源码。
没有 commit/push，未运行构建、测试、评测或 E2E。完整 server-initiated 用户交互、
其他协议/生态生命周期及安全恢复仍未完成；协议级拒绝不等于这些能力已实现。

## 2026-09-14 前序实施：多 MCP 服务器精确绑定与产品接线（未验证）

任务 CAR-03 / CAR-05 / CAR-07，详见
[ENGINE-MCP-MULTI-SERVER-2026-09-14.zh.md](ENGINE-MCP-MULTI-SERVER-2026-09-14.zh.md)。
Nomi/Coding 逐工具 HTTP MCP 支持最多 16 个冻结服务器。Kernel 按工具 lock.server_id
选择唯一资源，其他资源仍保持单值规则；产品解析、资源多选、启动投影和切换回显同步接入。
持久变更前拒绝不匹配的 MCP overlay，不再等运行时失败后才发现错误。
仍串行派发，任何服务器效果未知都会隔离整个 Session；没有放松效果证明或自动重放。
该切片 Nomi build 为 `host7`，Coding build 为 `host2-coding-loop19`。
没有 commit/push，未运行构建、测试、评测或 E2E（用户排除验证）。
其他传输/生态生命周期、复杂指令作用域和跨启动恢复仍待完成，整体目标保持进行中。

## 2026-09-14 前序实施：Nomi MCP 效果恢复与 Coding 指令刷新（未验证）

任务 CAR-05 / CAR-06 / CAR-07，详见
[ENGINE-NOMI-MCP-2026-09-14.zh.md](ENGINE-NOMI-MCP-2026-09-14.zh.md)。
当时 Nomi 新逐工具 HTTP MCP 源码准入已开放，限单服务器且禁止 native MCP 混用。
原始 source 已派发远端事务时禁止自动重试/编辑重提；每轮模型调用强制补入平台效果观察，
独立于 transcript 回滚/清空，未知状态仍隔离。migration 097 保存 bounded observation。
Coding 在潜在效果后、下一轮推理前刷新已知目录指令；规则变化使计划与完成报告失效。
空文件/override 优先级、层级排序和单次刷新缓存一并补齐。
当时 Nomi build 为 `host6`，Coding build 为 `host2-coding-loop18`。
无 commit/push；除局部格式化外未运行构建、测试、评测、迁移或 E2E（用户排除验证）。
多服务器限制已由最新切片推进；其他生态 owner、动态/递归/符号链接目录作用域和跨启动恢复仍待完成。

## 2026-09-14 前序实施：MCP 持久效果凭据、Coding 恢复与 Nomi 投影准备（未验证）

详见 [ENGINE-MCP-RECOVERY-2026-09-14.zh.md](ENGINE-MCP-RECOVERY-2026-09-14.zh.md)。
真实 MCP owner 在远端事务前写入 exact-turn pending 凭据，只有结果和清理均成功才 settled。
共享清理与启动恢复均检查持久未知状态；Coding 核对工具意图、派发和 owner 凭据，
可闭合已收敛的中断历史，不能重放工具。Nomi 冻结 schema 投影与 native lane 隔离已写入，
**当时其私有 transcript rollback / 自动 retry 尚未具备对应效果语义，新通道拒绝准入；现由上方切片补入**。
当时 Coding build 为 `host2-coding-loop16`，Nomi build 为 `host5`。
没有运行构建、测试、评测、迁移演练或 E2E，整体目标仍未完成。

## 2026-09-14 前序实施：Nomi Kernel 工具任务归属与收敛（未验证）

详见 [ENGINE-NOMI-EFFECTS-2026-09-14.zh.md](ENGINE-NOMI-EFFECTS-2026-09-14.zh.md)。
公共 `EngineEffectScope` 已接 Nomi Kernel 工具：Stop 关闭准入，调用方取消不丢失任务句柄；
终止、异常退出、销毁及会话恢复前检查任务和 MCP owner 收敛，超时保留隔离。
当时 Nomi build 为 `host4`。当时缺失的逐工具投影与 owner 持久凭据由上方切片补入，
效果感知回退/重试当时仍缺，准入尚未开放（最新状态见上方）；
MiniApp/动态工具/非工具 Lifecycle 尚未被本次 Kernel invoker 包装覆盖。
没有运行构建、测试、评测或 E2E，整体目标仍未完成。

## 2026-09-14 前序实施：MCP 每工具目录、Coding 准入与串行刷新（未验证）

详见 [ENGINE-MCP-CATALOG-2026-09-14.zh.md](ENGINE-MCP-CATALOG-2026-09-14.zh.md)。
每个 HTTP MCP 工具独立物化注册与 schema，Coding 按 Snapshot/typed resource 精确投影与准入。
生产配置变更和带版本 CAS 的连接测试触发同一 Registry 发布锁下的刷新；
未变化目录不增加 generation，发布失败明确告知保存已发生，可显式重试刷新。
数据库连接版本单调递增；不迁移旧 Agent revision/Session，不装载 Engine 代码。
当前 Coding build 为 `host2-coding-loop15`，Nomi build 为 `host3`。
默认 Nomi 新路径、多个服务器及其他传输/生态/恢复仍未完成。
没有运行构建、测试、评测或 E2E。

## 2026-09-14 前序实施：生产 MCP 端口与协议边界（未验证）

详见 [ENGINE-MCP-PORT-2026-09-14.zh.md](ENGINE-MCP-PORT-2026-09-14.zh.md)。
按安装用户与真实产品仓库核对连接版本、授权和固定 schema，独立于工作区分发；
已派发 owner 失败/取消保持 Session 隔离，公共资源清理核对远端未确定状态。
HTTP owner 增加分页边界、流式 SSE 关联结果读取、固定 Session ID、
协议 header 与不确定副作用错误语义。当前 build 为 `host2-coding-loop13`。
**该前序切片当时尚缺目录物化与 Coding admission；已由上方 loop14/15 源码补入，仍未验证**。
没有运行构建、测试、评测或 E2E，不能把执行端口接线等同于产品接通。

## 2026-09-14 前序实施：Coding 完成报告与观察依据（未验证）

详见 [CODING-COMPLETION-2026-09-14.zh.md](CODING-COMPLETION-2026-09-14.zh.md)。
新增 `report_completion`：覆盖当前计划项，明确 supported/unverified/blocked 并引用
真实工具观察；新工具结果、控制变更、steering 输入及工作区 epoch 使旧报告失效。
缺失/过期报告或 blocked 事项不能走正常任务完成；unverified 项由引擎追加到最终文本。
此处校验来源和时效，不宣称可自动证明需求覆盖或测试语义。当前 build 为
`host2-coding-loop12`。没有运行构建、测试、评测或 E2E。

## 2026-09-14 前序实施：独立 Engine 源码参考（未验证）

详见 [ENGINE-REFERENCE-2026-09-14.zh.md](ENGINE-REFERENCE-2026-09-14.zh.md)。
新增独立 Cargo example `evidence-engine`，通过公开生产端口实现规划、限步只读研究、
证据板和综合回答，拥有自己的上下文策略与日志格式，不包装 Coding，也不加入默认产品。
修正组合回调丢失 Arc、无法调用 session-hosted 注册入口的问题。引擎绑定覆盖最终二进制。
本次没有运行构建/测试/实际模型调用；官方 Coding build 仍为 `host2-coding-loop11`。
独立循环的源码实现不再 pending，成功执行与完整生态/恢复仍不能标为完成。

## 2026-09-14 前序实施：公共生产资源装配（未验证）

详见 [ENGINE-RESOURCES-2026-09-14.zh.md](ENGINE-RESOURCES-2026-09-14.zh.md)。
新增 `EngineKernelSession`：从 owner 解析的 workspace 和类型化授权编译 Snapshot，
装配真实 Kernel 工具，按 accepted root 管理进程 scope 与不可丢弃的清理结果。
Coding 已使用同一装配/清理入口。进程 owner 和契约下沉到 `nomifun-engine-core`，
平台不再依赖 Coding 进程类型。当前 build 为 `host2-coding-loop11`。
未运行构建或测试；独立 engine 参考循环及 MCP/MiniApps/外部副作用/跨启动恢复仍待完成。

## 2026-09-14 前序实施：公共模型事实、工具宿主与历史边界（未验证）

详见 [ENGINE-EFFECTS-2026-09-14.zh.md](ENGINE-EFFECTS-2026-09-14.zh.md)。
Coding 每回合通过公共接口读取主/备用模型预算事实；未知值策略仍由 engine 决定。
生产工具使用公共 EngineToolHost，持久派发意图、按只读/副作用门控执行、持久结算后返回，
增加同回合去重、清理入口关闭与观察屏障。旧消息回退与重启日志先检查大小再加载正文。
本切片 build 为 `host2-coding-loop10`；没有运行构建、测试或评测。
完整公共资源装配、独立 engine 示例和生态/跨启动恢复仍待完成。

## 2026-09-14 前序实施：公共日志、历史与持久结算（未验证）

详见 [ENGINE-JOURNAL-2026-09-14.zh.md](ENGINE-JOURNAL-2026-09-14.zh.md)。
公共 writer 复用 Conversation 日志表，托管写入任务、维护预算和一次性模型领取；
Coding 的普通事件、steering、能力激活和模型请求已接入。公共历史先检查整轮大小，
再读取事件，由 Coding 自己解释 codec。工具任务包含持久结算，失败不能通过清理证明。
公共 Session 宿主也已提供真实 Broker 模型端口，Coding 委托同一装配方法。
该切片 build 为 `host2-coding-loop9`；当时通用 effect/资源宿主和独立 engine 示例仍待完成。

## 2026-09-14 前序实施：公共工具端口与 Session 接入（未验证）

详见 [ENGINE-PORTS-2026-09-14.zh.md](ENGINE-PORTS-2026-09-14.zh.md)。
新增独立 `nomifun-engine-core`，Coding 使用其工具契约与 Kernel adapter，生产调用固定
Session 和完整已选映射；复核副作用/并行分类，派发后由宿主保留工具任务。
公共 `EngineSessionHost` 从当前 Conversation owner 解析 exact binding、revision、Snapshot
和 accepted receipt，Coding 工厂与每轮准备已接入。`register_session_hosted` 给社区 driver
提供相同的生产 Session 解析入口，不提供额外授权或动态安装。
该切片 build 为 `host2-coding-loop8`。完整公共 history/effect facade 与独立 engine 示例当时仍未完成。

## 2026-09-14 前序实施：公共 Engine SDK 基础（未验证）

详见 [ENGINE-SDK-2026-09-14.zh.md](ENGINE-SDK-2026-09-14.zh.md)。
Coding 已通过公共 HostedAgentRuntime 运行，复用取消、UI 代次隔离、清理后终态记录和
保留同次 teardown 结果的机制。模型端口已移入 Broker；生产工具托管改用公共 EngineTaskGroup。
编译期 `register_hosted` 简化社区 driver 注册，但尚未提供通用的生产端口装配与第三方成功执行示例。
该切片 build 为 `host2-coding-loop7`；没有新增第三个默认引擎，也没有运行该切片验证。

## 2026-09-14 前序实施：原子准入、资源分页与命令证据（未验证）

详见 [CODING-BOUNDARIES-2026-09-14.zh.md](CODING-BOUNDARIES-2026-09-14.zh.md)。
追加输入检查与 ToolStarted 持久化共用 turn 锁；拒绝准入不执行工具，日志失败取消本轮。
Skill 参考资源支持有界 UTF-8 分页，单资源可达 256 KiB，正文及资源总预算保持限制。
命令观察区分真实终态、清理证明、启动调用和代码变更时序；新工作令此前完成复核失效。
该切片 build 为 `host2-coding-loop6`。MCP 仍缺当前产品目录与精确 owner 链路的适配，
没有只放开白名单。MiniApps、push、安全续跑及公共 SDK 等仍未完成。本轮未运行验证。

## 2026-09-14 前序实施：有回执的追加输入（未验证）

详见 [CODING-STEERING-2026-09-14.zh.md](CODING-STEERING-2026-09-14.zh.md)。
通用接口传递已持久化回执与目标轮次；Coding 在模型/串行工具/结束边界处理追加文字，
保留需求原文、不授予新权限、不强行中断在途副作用。补齐收件关闭竞态、未投递记录和
重启后投递状态未知的历史投影；延后的调用不再预记为 ToolStarted。
该切片 build 为 `host2-coding-loop5`。仍不将整体标记完成，也没有运行该切片验证。

## 2026-09-13 前序实施：按需能力与流事件边界（未验证）

详见 [CODING-ACTIVATION-2026-09-13.zh.md](CODING-ACTIVATION-2026-09-13.zh.md)。
已增加独立的能力搜索/激活批次、Kernel 同源活跃集合及 ToolPlan 刷新、持久化后应用代次、
同一 Session exact-build 激活日志恢复，以及真实 process owner 的激活安全边界检查。
同时补齐完整流事件预算、工具身份/metadata 限制及新 fs.read 的指令读取刷新。
当前 build 为 `host2-coding-loop4`。MCP、MiniApps、push、steer、安全续跑及通用 SDK 等
仍未完成。本轮未运行构建、测试或评测，不以源码实现替代验证结果。

## 2026-09-13 前序实施：扩展输入与上下文资源（未验证）

详见 [CODING-EXTENSIONS-2026-09-13.zh.md](CODING-EXTENSIONS-2026-09-13.zh.md)。新增
Snapshot 锁定 Plugin 工具、制品内 Skills/按需资源读取、accepted delivery 附件核对、
共用图片解码、多模态预算及压缩、指令日志摘要化和有界历史，以及 Session 隔离快照。
Session 关闭释放 Kernel 资源，重复等待同一清理完成结果。该切片 build 为
`host2-coding-loop3`，旧 Session 不自动迁移。按需能力、MCP、MiniApps、push、steer
及未知副作用恢复等仍未完成；本轮不运行构建、测试或模型评测。

## 2026-09-13 前序实施：控制循环与保守恢复（未验证）

详见 [CODING-CONTROL-2026-09-13.zh.md](CODING-CONTROL-2026-09-13.zh.md)。
已增加状态化 update_plan、失败后的副作用门控、路径感知指令、turn-owned 交互进程，
以及按 exact build 注册的重启恢复接口。没有命令启动或已有清理证明的中断回合可
关闭历史并通过平台 CAS 解封；正在执行命令且清理证明缺失的回合仍隔离。
本轮不运行构建、测试或模型评测。当前标识为 `host2-coding-loop2`，旧 build 不自动迁移。

## 2026-09-13 前序实施：Coding 执行机制（未验证）

详见 [CODING-LOOP-2026-09-13.zh.md](CODING-LOOP-2026-09-13.zh.md)。新增受控命令执行、
根仓库指令、模型窗口预算/usage 校正、有界自动压缩、执行观察反馈和关闭轮次的结构化
历史重放。仍未完成独立规划器、交互进程与异常重启恢复。

按用户要求，本次没有运行构建、测试或模型评测。下方通过结果均属于此前切片，
不是最新工作区的验收结果。旧 exact build 不自动迁移到新的 `host2-coding-loop1`。

## 2026-09-13 前序实施：编译期多引擎（CAR-D-021）

第一阶段仅支持二次开发添加实现/依赖并重新打包应用。注册在组装后关闭，
不提供打包后挂载、动态安装或热更新。官方预置 Nomi 与 Coding；Agent 工作台配置归属不变。

本轮完成：

- `RuntimeEngineAdmission` 随工厂必需注册；内置与社区实现共用 Snapshot/Session overlay
  兼容性校验。Agent 保存、创建/Fork、修改能力、切换 Agent、runtime 构造均接入；
  Session 业务路由不再特判 Coding family。实际权限仍由 Kernel 决定。
- Coding 能力查询读取工具准入所用的同一份 `SessionCapabilityState`，修复运行后 409。
- Coding 真正调用有界上下文组装：历史裁剪按完整回合，保留当前输入，记录
  `context_prepared`；每次模型调用前检查预算。执行中超限明确失败，不截断当前工具链，
  不将这一行为描述为自动压缩。宿主只提供最近的有界历史候选。
- 缺失构建错误不再建议无法执行的“安装或 Fork”，明确要求包含原构建的应用发行版。

本轮验证（与下方历史结果分开）：

- `cargo test -p nomifun-coding-engine --lib`：43 通过。
- `cargo test -p nomifun-ai-agent --lib runtime`：82 通过；含 Coding adapter、注册准入、
  任意 family overlay 拒绝、在工厂执行前阻断，以及取消/退出隔离。
- `cargo test -p nomifun-app --test coding_runtime_production`：1 通过；增加组装后拒绝注册、
  Coding 运行前后能力查询一致、生产 `context_prepared`、社区 Engine Snapshot/overlay
  拒绝；原真实 Kernel 文件写入、冷重建、绑定和 Fork 校验继续通过。
- `cargo test -p nomifun-app --test nomi_core_route_gap nomi_core_agent_session_projects_saved_chat_binding_without_internal_inputs -- --exact`：1 通过，37 filtered out。
- `git diff --check` 与任务清单 JSON 解析：通过。未改 UI，不重复 UI 全量检查。

下一步仍是公共模型/工具/历史/状态宿主 SDK 与真正成功执行的第三方参考 Engine，
随后完善异常重启、应用升级兼容及 AGENTS/compaction/checkpoint。当前注册与准入测试
不等于第三方完整执行验收，也不把整体多引擎平台标记完成。未做付费模型或跨平台验证。

## 2026-09-13 历史实施：默认生产链路

详见 [`PRODUCTION-INTEGRATION-2026-09-13.zh.md`](PRODUCTION-INTEGRATION-2026-09-13.zh.md)。
全部工作仍在 `rf/agent-capability-platform-v2`，没有 push。

- 新建会话持久化不可原地更换的 exact Engine binding，冷恢复不重新解析 channel。
  Fork 继承父会话绑定，不接受独立引擎覆盖。
- `RuntimeEngineHost` 在默认组合根安装 Nomi/Coding/受信任扩展工厂；
  `NomiCoreApplication::compose_with_runtime_engines` 提供二次开发注册入口。
- Coding 使用真实 Conversation、Chat Broker、Kernel 与 Wave2 文件/Git owner；
  不引入第二个 SessionStore。事件与模型调用领取证据从属于原有 turn receipt。
- **按 CAR-D-020 纠正产品层级**：引擎由 Agent 工作台的 Agent 设置配置，保存为
  Revision payload；首页仅选择 Agent。目录仍从 `/api/runtime-engines` 动态发现。
  Session owner 从保存版本继承引擎，创建／Fork DTO 不再接受引擎覆盖。
- 当前仅准入 9 个初始工作区工具。进程、按需能力、Skills/MCP/MiniApp、附件、
  compaction/checkpoint、异常重启证明和完整 Remote/Automation 继承行为验证仍未完成。

CAR-D-020 入口与配置归属纠正后的验证：

- `cargo check -p nomifun-app --tests`：通过。
- `cargo test -p nomifun-agent-contracts -p nomifun-agent-control-plane --lib`：
  86 + 26 通过；包含旧 payload 序列化／摘要兼容、引擎参与版本摘要。
- `cargo test -p nomifun-app --test coding_runtime_production`：1 通过。
  通过工作台预览／保存／重开编辑器配置引擎；无 Session override 运行真实 Coding
  文件工具；模型派生版本保留配置；修改 Agent 后新会话用 Nomi，旧会话及 Fork 保留
  Coding；缺失构建／摘要漂移／错误 profile 阻止保存；独立第三方 Agent 分派不回退。
- 原默认 Session projection/fork 定向用例：1 通过，37 filtered out。
- UI 6 文件定向验证：38 通过，156 条断言；含下拉框真实选择／恢复默认／缺失构建／
  channel 回显、草稿脏状态、工作台测试流程和首页无 runtime override。
- `bun run typecheck`：仍未通过，`bun:test` 声明缺失及测试文件类型错误；本次日志
  未报告生产 UI 文件错误。未做桌面视觉、付费 Provider 或多平台发布验证。

首次生产接线验证（`80e1f3ede`；以下为历史，不混同）：

- `cargo check -p nomifun-app --tests`：通过。
- 生产默认路由 E2E：1 通过；包含真实文件写入/冷恢复/绑定保护/显式 Fork/
  第三方工厂分派且失败不回退。模型使用本地 HTTP fixture，无付费请求。
- Coding host 取消/退出证明：3 通过；runtime 定向测试：71 通过。
- DB `id_schema_contract`：20 通过。
- 原默认路由 Session projection/fork 用例：1 通过（其余 37 本次未重跑）。
- UI 目录和创建行为测试：10 通过，45 条断言。
- `bun run typecheck`：未通过，当前依赖环境缺少 `bun:test` 类型声明，产生
  测试文件类型错误；日志未报告生产 UI 文件错误，不能宣称全量 typecheck 通过。

以下“主重构分支本地合入”记录是继续实施前的历史验证，不能用其中的
“生产未接线”描述覆盖上面的最新状态。
| `CAR-08` | planned | `CAR-07` | 删除旧 Wrapper/Sidecar |
| `CAR-09` | planned | `CAR-08` | 平台生态与非 Agent consumer |
| `CAR-10` | planned | `CAR-08`、`CAR-09` | 三平台发布和 Stable admission |

## 2026-09-13 主重构分支本地合入

- 目标分支 `rf/agent-capability-platform-v2`，起点 `08caa20d7`。
- 已按顺序取入 CAR 的 8 个独立提交；对应本地 tip `924b6dccb`。
- 本地实现提交：`6d87dc232`（开放目录/Registered/Coding adapter、取消与兼容修复）。
- Kernel 测试更新为当前 contribution lock、Revision digest 和目标资源绑定合同。
- CAR-02：Broker 原生取消和 Coding adapter 已接通；端到端 Session/真实 Provider
  验收仍未执行，不将该任务标记 completed。
- CAR-07：采用当前 Nomi-core owner，通过开放接口接入任意用户 Runtime。
  Nomi factory 已返回 Registered；通用目录拒绝 Build 覆盖、摘要漂移和未知 profile。
  Coding adapter 在现有 registry 中验证构造、取消、事件投影和清理失败隔离。
  尚未安装默认生产 host；没有切换默认路由或引入第二套持久 Session。
- 完整证据与建议见 `LOCAL-INTEGRATION-2026-09-13.zh.md`。
- 已验证 `cargo test -p nomifun-coding-engine -p nomifun-chat-model-broker`：
  Coding **40 passed**；Broker unit **10 passed**、conformance **21 passed**。
- 已验证 `cargo test -p nomifun-ai-agent --lib runtime_`：**70 passed**，
  覆盖开放目录、registry、runtime state 和相关 option 合同。
- 已验证 `cargo test -p nomifun-app --lib router::chat_broker_host::tests`：
  **9 passed**，并编译经过 Conversation/Remote/Automation 等下游依赖。
- `cargo test -p nomifun-ai-agent --lib coding_runtime::tests`：**7 passed**；
  `cargo test -p nomifun-ai-agent --lib factory::tests`：**3 passed**。
- `cargo test -p nomifun-app --test nomi_core_route_gap`：**36 passed，2 failed**。
  `installation_token_is_limited_to_headless_product_control_planes` 的 `/api/plugins`
  期望 403，实际 200；`agent_session_model_selection_is_exact_persistent_and_keeps_the_agent_unchanged`
  未提供 `fs.read` 所需 workspace selection，收到 `RESOURCE_SELECTION_REQUIRED` 422。
  路由、资源解析器和测试文件相对目标起点 `08caa20d7` 无改动，失败发生在 Runtime
  构造前；这是静态差异核对，**未在旧 checkout 上重跑基线**，不宣称全绿或已修复。
  同一已构建测试二进制单独运行插件认证用例（`--exact`）仍失败，排除了仅由
  本次并行运行导致的偶发现象；没有改动认证逻辑或放宽断言。
- `git diff --check` 与 manifest JSON 解析通过。新模块单独执行 rustfmt；
  未把仓库默认 `disable_all_formatting = true` 下的空操作当作格式验证。
- 未运行真实付费 Provider、桌面 UI E2E、macOS/Linux、发布构建或全仓测试；
  本次没有生产接线，不宣称 UI/多引擎灰度闭环完成。

## 源分支隔离交付（2026-09-06 历史基线）

基线：

```text
branch: car/coding-engine
base_sha: 6a2a94bd192ef67eda5dd67331f6c047b1c1b315
code_commits:
  - b652fa29ce02c91f600d54abaecd98dfb967f9c4
  - c8b0193892ad7f1b73586b7570b7a2f0172c8d1b
  - 0e33dbed53e248ef5c926b548bfde378120bb400
  - f3ff31b4b6168c2cc6b4b3de867b5f922a3eb8a4
  - 449e06110c34902e070a1b7b506bdda2db9f147a4
code_tip: 449e06110c34902e070a1b7b506bdda2db9f147a4
```

已验证：

```text
cargo fmt --package nomifun-coding-engine
cargo check -p nomifun-coding-engine
cargo test -p nomifun-coding-engine
git diff --check
```

最后一次定向测试结果：`36 passed; 0 failed`。

未运行：

- `cargo clippy -p nomifun-coding-engine --all-targets -- -D warnings`：当前 stable
  toolchain 未安装 `cargo-clippy`；
- 全仓构建/测试：本次只新增未接生产主链的独立 crate，按仓库规则使用定向检查；
- live Provider、Broker 原生取消、AgentSession/SessionEvent E2E：这些属于远程中央接线；
- File/Patch/VCS 的真实产品路由：当前通过 Kernel/Wave2 owner adapter 留出合同，
  尚未接入生产 AgentSession。

## 已知集成缺口

- CAR-02 已新增 `open_chat_stream_cancellable`；取消覆盖本机 attempt future。
  当前生产 Session 的端到端验收仍待接线，不承诺 Provider 服务端停止计算；
- Capability handler 合同尚无 `CancellationToken`；当前 Kernel adapter 只能在调用
  future 层 fail-fast；
- 原 `CodingEngineCatalog` 仍只管理 Coding family；新增通用 `RuntimeEngineCatalog`
  可注册任意实现，但尚未与生产 Session 创建/Fork 事务和 API 发现入口连接；
- `NativeResponsesItem` 与音频输出在 Coding P0 明确 unsupported；
- SessionEvent durable projection、UI、Remote/Automation、Workspace/File/VCS owner
  的生产接线尚未完成。

远程交接与禁止事项见 `HANDOFF-REMOTE-INTEGRATION.zh.md`。

## 状态更新规则

任务状态变更必须同时写明：

- `task_id`；
- 变更原因；
- commit SHA（如果已有代码提交）；
- 实际验证命令；
- 未运行项和准确原因；
- blocker 或 follow-up。

不在本文写入 API key、credential、主机地址、完整模型响应或秘密日志。
