# Agent 会话可靠性排查与修复（2026-09-23）

## 现场证据与边界

用户提供的三张截图分别显示：任务结束时出现笼统的上游错误；执行过程展示了重复的模型进度文字和多条红色工具失败；文件侧栏为空。当前机器可见的 NomiFun 数据库中没有截图所指会话的持久化记录，因此无法把该会话的最终错误精确归因到某一条底层异常。随后在默认 `bun run dev` 数据目录中亲自复现同一贪吃蛇请求：通用预设、StepFun `step-3.7-flash` 会话 `01a0ce76-a19b-7451-aa49-56abaaea52ac` 最终失败，磁盘和侧栏均无文件。持久化事件表明首次 `write_file` 在引擎内被计划门槛拒绝；模型接连提交无效完成记录；回合最终因上下文压缩达到 `MaxOutputTokens` 而失败。以下处理同时覆盖这个现场链路和独立发现的跨预设问题。

## 根因

1. **首次文件操作的执行协议自相矛盾。** 首轮模型只能看到普通工具，`update_plan` 尚未暴露；单次 `write_file` 却会立即激活任务账本并要求先调用 `update_plan`。模型收到“未执行”后可能误以为文件已写入，继续提交无证据的完成报告，引发循环与压缩失败。真实桌面复现证实了这条链。
2. **指令预检根路径不通。** Agent Runtime 用 `read_file(format=instruction_scope, path=".")` 观察工作区根目录；文件资源层同时允许这个格式，却把 `"."` 交给只接受普通相对文件名的绑定解析器。文件操作执行到预检阶段时也会因此被延迟。
3. **内部预检污染会话展示。** `agent-instructions:*` 是引擎发起的内部读，但实时流和持久化投影把它显示成普通工具。一次内部预检失败因此产生用户可见的红色“读文件失败”，并放大后续工具失败的视觉噪声。
4. **侧栏根目录查询契约冲突。** 前端 `absoluteToRelativePath` 把工作区根目录编码为 `"."`；后端 `list_workspace_level` 只接受空串或 `"/"` 代表根目录。实际写出的文件也因此无法经会话工作区接口列出，侧栏显示为空。
5. **本地故障被误报为上游故障。** 模型步数耗尽、计划未关闭或完成记录未通过时，运行时安全停止回合；原来的通用 `Conflict` 分类可能落入 `UNKNOWN_UPSTREAM_ERROR`。真实复现最终出现的压缩输出上限是本地运行时错误，也不应笼统归因于服务商。
6. **原始中间文字展示过长。** 执行轨迹直接渲染模型的整段中间说明；尚未结束的回合还把最新一段模型文字暂时当作最终回答，导致过程文字占满主对话区。

## 全局处理原则与本次实现

- 工作区根目录统一采用 `"."` 作为模型与前端可见的读取/列举表示；内部转换成绑定根目录。普通写入和删除仍须使用规范化的文件相对路径。
- 单次、用户已授权的工作区工具调用可直接执行并返回真实收据，不再强迫模型调用首轮未暴露的计划工具。重复工作区调用、显式计划、用户中途更正或任务续接仍启用来源锚定的计划与完成账本，并在后续副作用前执行门槛。
- 若模型连续四次提交被拒绝的同一种计划/完成控制调用，运行时在保存第四次拒绝回执后结束为本地“任务未完成”，避免无效重试把上下文耗尽并触发压缩错误；其它工具调用、成功控制调用或新接受的用户指令会清零计数。
- 引擎内部预检继续写入审计事件，但不生成用户可见的工具行；旧会话中已保存的同类工具行也在渲染时过滤。模型主动调用的工具仍显示完整结果与失败详情。
- 尚未结束的模型文字留在实时过程折叠区，回合结束后才将最终回答呈现在主对话区。过程轨迹只显示一次相同的中间说明；长说明默认呈现短预览，可展开原文。运行时明确声明“未执行”的本地预检拒绝显示为中性“未执行”记录，保留可展开诊断；真正的本地或远程执行失败仍保留红色失败状态。
- 文件侧栏在文件内容事件到达时刷新；回合结束时再对账一次，以覆盖命令或其他工具改变文件却没有发出文件服务事件的情况。刷新节流与取消归属当前会话订阅，切换会话不会继承上一个会话的待刷新计时器。
- 已知执行守卫结束的回合使用 `NOMIFUN_TASK_INCOMPLETE`，提示用户先检查已经产生的文件和操作，再继续任务；旧会话中能由精确守卫详情识别的笼统错误也在读取时重分类。服务商错误保持原有分类。
- 本地运行时异常使用 `NOMIFUN_INTERNAL_ERROR`，权限拒绝使用 `NOMIFUN_PERMISSION_ERROR`；真实模型 API/流错误仍按上游错误处理。
- 真实模型验收固定走独立数据目录，并验证文件内容、成功工具回执、会话终态和侧栏所用工作区列举接口。密钥仅通过测试进程标准输入传递，不写入仓库或测试日志。

## 验证矩阵

| 场景 | 验证结果 |
| --- | --- |
| StepFun `step-3.7-flash` 最小会话、Agent 协作与自动工作链 | 真实模型通过 |
| StepFun `step-3.7-flash` 工作区文件创建、写入回执、根目录列举和正常回复 | 真实模型通过 |
| 官方 `coding.codex` 预设使用 StepFun 创建文件并完成会话 | 真实模型通过 |
| 官方 `coding.codex` 预设生成 `snake_game.html`，检查 HTML/脚本/键盘控制、成功回执及根目录列举 | 真实模型通过，约 3 分 37 秒 |
| 官方 `assistant.general` 预设在具备 Browser/Computer 能力的独立桌面服务中连续完成普通对话、文本文件与贪吃蛇 HTML；核对每个回合的终态、收据和根目录列举 | 真实模型通过 |
| 开发版 Tauri 窗口、官方通用预设、同一贪吃蛇请求 | 修复后 21 秒完成；一次成功写入，`snake_game.html` 12,708 字节；交付卡片、侧栏文件树和 HTML 预览均正常；预览中“重新开始”按钮实际可用 |
| 官方 `companion.default` 预设创建伙伴会话并完成回复 | 真实模型通过 |
| 官方 `creative-studio.default` 预设创建画布会话并完成回复 | 真实模型通过 |
| 六个官方预设的能力目录与不可变配置编译（桌面 Browser/Computer 能力环境） | 目录与配置测试通过 |
| 官方通用预设编译；伙伴和创意入口的会话创建；客户服务产品绑定可用性 | 路由集成测试通过 |
| 客服独立对话域的消息合并、串行执行、失败通知、转人工和策略授权 | 31 项单元测试通过 |
| 根目录指令范围与文件列举；内部预检审计/展示分离；错误分类 | 文件服务 226 项单元测试及定向 Rust 测试通过 |
| 单次文件操作直通、多步操作仍受计划门槛约束、重复无效完成报告提前终止 | Agent Runtime 53 项单元测试通过 |
| 过程文字去重/展开、文件事件匹配、类型与桌面 UI 边界 | 定向前端测试及检查通过 |
| 桌面 Rust 目标与 WebUI 生产构建 | 编译检查通过 |
| 仓库 `bun run check`、Rust 格式及差异检查 | 通过 |

通用预设依赖桌面 Browser/Computer 能力宿主。在无这两个宿主的独立后端进程中，它按设计返回 `CAPABILITY_NOT_MATERIALIZED`；真实模型验收因此使用了具备这些能力的桌面服务。测试中的 Browser Runtime 故意不可用，任务没有调用 Browser/Computer 动作，所以不能由此推断浏览器或桌面控制本身可用。伙伴与创意预设的真实模型测试只覆盖各自会话的普通回复，没有覆盖机器人、渠道、媒体创作等外部资源。`customer-service.default` 是客服产品的策略预设；访客消息由客服领域的 `CsDialogueEngine` 一次性模型调用处理，不走普通桌面 AgentSession 会话。客服对话的资源和模型调用仍需单独验收。

## 持续回归入口

- `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --model-smoke`
- `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --file-smoke`
- `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --coding-smoke`
- `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --game-smoke`
- `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --general-desktop-smoke`
- `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --companion-smoke`
- `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --creative-smoke`
- `cargo test -p nomifun-app --test official_preset_catalog_integrity --features "browser-use computer-use"`
- `cargo test -p nomifun-app --test nomi_core_route_gap`

真实模型测试为显式运行的忽略测试，要求通过现有 runner 的凭据隔离入口提供 StepFun 密钥。
