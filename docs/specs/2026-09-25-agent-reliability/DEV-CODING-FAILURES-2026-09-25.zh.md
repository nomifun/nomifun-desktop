# 开发环境两个 Coding 会话失败诊断与修复

## 调查范围与证据

调查对象是 `bun run dev` 的两个真实会话，用户输入均为「帮我写一个五子棋的H5游戏」：

| 会话 | Preset | 观察到的失败 |
| --- | --- | --- |
| `01a0d8e4-351f-7b30-b9bd-11bc68d4b4a8` | `coding.codex` | 首次写入嵌套路径失败，随后进入重规划、重复读取、补丁与进程重试、上下文压缩循环 |
| `01a0d8e5-fa36-7722-94cf-bf18deb97bb2` | 通用 | 连续三次输出文本形式的工具调用，未执行工具即以 `USER_LLM_PROVIDER_INVALID_TOOL_CALL` 失败 |

从正在使用的开发数据库只读获取 Session、Preset revision、Snapshot、canonical events、payload 和文件 owner effect 回执。实际数据目录是 `%LOCALAPPDATA%/NomiFun-dev-schema-b40e0155e59ea087`；两个会话的执行 build digest 均为 `6b0c50b33e200ef3ee2d34e55596b96ebc0ed6f00d78d8f4d21bcc343bb34ef2`。两者均使用 `step-3.7-flash` / `openai_chat`，均有文件读、写、补丁及进程权限。没有发现这些失败由 HTTP 429 或额度耗尽引起的证据。

诊断没有修改原数据库、Preset、会话状态或工作目录。详细只读快照和检查日志保存在忽略目录 `.tmp-agent-dev-failures-20260925/`，不包含凭据或模型思考文本。原失败记录不能转换为修复后的成功证明。

## 确定的失败链路

### 嵌套文件的父目录缺失

第一个会话第一步调用 `write_file(path="gomoku/index.html", content=...)`。序号 52 的 `effect/failed` 回执明确记录：文件 owner 无法解析目标父目录，Windows 返回 `os error 2`。

`write_file_for_agent_session` 和 Agent patch 的目标验证都复用了要求直接父目录已经存在的写入验证器。底层文件写入没有创建目录。因此一个完全正常的“创建新项目文件”请求会失败。同会话序号 439、589 等处再次出现相同问题。默认 workspace 最终正确限定到 `conversations/<session-id>/`，不是写错工作区。

截至本次后续只读快照，第一会话仍保留 running 状态，已有 42 个模型步骤、62 次 compaction_started、14 次 context_compacted。错误引发了恢复规划、文件重新观察、错误的 Windows 进程调用及更多压缩；这些是失败循环的一部分，不构成任务进展。

### 文件错误的原因在 Kernel 投影中丢失

owner 回执有具体原因，模型收到的却只有 `capability handler failed with INVALID_PAYLOAD`。模型无法区分父目录问题、源版本冲突或补丁行数错误，重复修改内容和调用格式。patch owner 已生成的发布/保留/回滚观察也被通用错误显示遮蔽。

### 文本形式的工具调用纠错缺乏协议约束

通用会话序号 17、23、29 均为 `model_response_rejected`。输出守卫识别到了正文中的工具调用标记；没有任何调用被交给工具执行层。此前纠错仅在上下文中加入文字提示，后续请求仍保持自动选择，模型持续使用相同错误格式，第三次触发有界停止。

两个 preset 的 instructions/persona 均为空，未发现其中配置了诱导该格式的自定义提示。此处是观察到的模型/接口输出问题与应用纠错机制不足的组合，不能据此推断所有模型或所有任务的成功率。

## 实际修复

1. **工作区内自动创建父目录。** 新增 `nomifun-file/src/workspace_write.rs`，将无副作用的路径验证和创建阶段分离。检查最近存在的祖先、workspace containment 和 `.nomifun` 保护范围，再通过目录句柄逐段创建，不跟随创建途中的链接。写入前再次验证目标身份。只应用于 Agent workspace 写入和 patch，不放宽其他文件管理接口。
2. **保留整批补丁预检。** 所有路径、源版本、hunk 和大小限制通过后才创建父目录或写文件。已有文件的原子发布、并发前置条件和失败观察继续保留。
3. **提供可操作的文件错误反馈。** Kernel 对 workspace write/patch 返回有界的分类修复提示。patch 的合法文件索引观察得以保留；主机绝对路径、源文件内容和原始诊断不会转发给模型。
4. **加强原生工具协议纠错。** 仅在有界纠错期间且有已授权工具可用时使用 `ChatToolChoice::Required`，保留已有 Specific 选择。有效响应后恢复通常选择；用户追加新输入时解除这一约束，但不重置重试额度。仍然不执行正文中的伪工具调用，也不增加既有重试预算。
5. **更新工具说明和运行时 build digest。** 明确说明文件/补丁会创建缺失父目录，并把新文件 helper 纳入执行版本摘要，避免旧检查点被误认成新实现。

## 验证

测试采用隔离临时工作区与构造的 provider 响应，覆盖真实应用 API → preset → provider 编码 → Runtime → Kernel → File owner → canonical journal 链路，不使用原会话作为测试沙箱。

| 检查 | 结果 | 直接覆盖 |
| --- | --- | --- |
| `nomifun-file` 库测试 | 230 通过 | 嵌套文件和 patch 创建；整批无效 patch 不创建目录；既有文件/非法父目录保留；Windows junction 越界和 owner namespace 拦截 |
| `nomifun-agent-runtime` 库测试 | 最终版本 113 通过 | 协议纠错、预算、恢复、有效调用后恢复选择、追加输入解除约束；拒绝正文伪调用 |
| `nomifun-engine-core` 库测试 | 24 通过、1 项既有 ignored | 文件错误分类与诊断脱敏、patch 发布观察保留；ignored 是依赖 Bun PATH 的进程专项，与本次修改无关 |
| `cargo test -p nomifun-app --test native_coding_reliability --features browser-use,computer-use` | 2 通过 | 实际 Coding/通用模板、同一 OpenAI Chat 编码路径、默认会话/指定工作区、写入与读回、唯一写入回执和 canonical terminal |
| `git diff --check` | 通过 | 修改格式 |

库测试实际先运行组合命令 `cargo test -p nomifun-file -p nomifun-agent-runtime -p nomifun-engine-core --lib`；随后补充“新输入解除纠错约束”后，单独重新运行 `cargo test -p nomifun-agent-runtime --lib`。所有 Rust 命令通过既有 Windows 工具链环境 wrapper 执行。

应用级场景分别为 6、7 次构造模型请求；通用场景多出的 1 次是故意注入的格式错误。两个场景均只有 1 次实际写入、0 次文件 owner 失败。写入后显式读回会激活既有多步骤完成核验，因此夹具提交了关闭计划和 `report_completion`；未修改产品完成条件。最初夹具只安排写、读、文本回复，因未满足这一既有协议而失败；检查后补齐夹具流程，原检查日志保留。

夹具 HTML 只用于文件/协议验证，完成账户明确标记游戏功能为 `unverified`，这些测试不是完整五子棋可玩性验收。未修改 renderer，因此没有运行 UI 全量检查或桌面边界检查。

另进行了 **1 次最小真实 StepFun 协议兼容性请求**，只提供无副作用的 `check_value` 函数定义，不执行返回的工具。结果为 HTTP 200、`finish_reason=tool_calls`、1 个原生工具调用、无文本伪调用，共 296 tokens。该结果证明当前授权接口接受纠错参数，不是完整 Coding 任务的真实模型验收。StepFun 的工具协议定义可参见[官方工具调用文档](https://platform.stepfun.com/docs/zh/api-reference/tool-call)。

## 交付边界

这些修复针对本次两个会话中有证据的失败，不构成 99% 成功率证明。原会话的失败/未完成状态保留；没有重开终态回合，没有清空预算，没有把重发任务冒充恢复。没有运行跨天体验、大规模付费批量或全仓库测试；没有提交或推送。

本次编译并验证了包含 `browser-use,computer-use` 的应用链路。未重新启动用户的桌面开发进程、未在原会话重跑完整真实模型五子棋任务。重新启动 `bun run dev` 后加载修复；旧回合的继续/取消仍遵循 canonical execution 的版本和 owner 检查，不直接编辑数据库状态。
