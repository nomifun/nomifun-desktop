# Codex Coding 预设与 Coding Runtime 配合检查

检查对象：Agent 工作台的 `coding.codex` 官方预设，以及服务注册的 `nomifun.coding` / `coding` 执行引擎。

结论：当前不能完整配合。预设没有默认选择 Coding Runtime；35 项预设能力仅有 14 项进入 Coding 的内置能力准入范围，另有 21 项会被拒绝。准入通过也不等于已完成真实模型端到端验证。本次未修改产品运行逻辑。

## 1. 默认引擎绑定缺失

- `ui/src/renderer/pages/agentSettings/OfficialTemplateOverview.tsx:25` 的 `documentFromTemplate` 复制能力和 Skill 绑定，没有为 Coding 预设设置 `runtime_engine`。
- 服务端官方模板创建路径 `crates/backend/nomifun-agent-control-plane/src/service.rs:494` 同样使用 `runtime_engine: None`。
- `crates/backend/nomifun-app/src/router/runtime_engines.rs:223` 的默认绑定为 `nomifun.nomi` / `stable` / `default`；`agent_binding` 在没有显式选择时使用它。

所以截图中的“默认引擎（Nomi）”符合当前代码，并非仅仅显示错了名称。`CodingNative` 编译配置或 Nomi 的 `coding` 模式也不能替代显式的 `nomifun.coding` 引擎绑定。

## 2. 35 项能力与准入清单

预设来源：`crates/backend/nomifun-agent-contracts/contracts/presets/official-preset-seed-manifest.payload.json` 的 `templates.coding.codex.enabled_capabilities`。

准入来源：`crates/backend/nomifun-app/src/router/coding_tool_surface.rs:207` 的 `CodingAdmission::validate_snapshot`，其中基础清单来自 `nomi_core_wave2.rs:45`，另加入 `llm.vision` 和 `mcp.resource`。插件、产品和冻结 MCP 工具有独立来源条件，不会使这些内置能力自动获得支持。

| 能力 | Coding 内置准入 | 说明 |
| --- | --- | --- |
| `session.attachments.read` | 不通过 | 尚未接入这个能力 ID |
| `agent.execution.plan` | 不通过 | 内部计划数据结构不等于该平台能力已接入 |
| `agent.execution.steer` | 不通过 | 已有输入追加端口，但这个能力 ID 未接入 |
| `agent.execution.observe` | 不通过 | 执行事件不等于平台观察能力已接入 |
| `fs.read` | 通过 | `read_file` |
| `fs.search` | 通过 | `search_files` |
| `fs.write` | 通过 | `write_file` |
| `fs.patch` | 通过 | `apply_patch` |
| `workspace.bind` | 不通过 | 宿主绑定会话工作区，不代表该能力 ID 已接入 |
| `workspace.artifacts` | 不通过 | 未列入支持清单 |
| `vcs.status` | 通过 | `git_status` |
| `vcs.diff` | 通过 | `git_diff` |
| `process.exec` | 通过 | `exec_command`，含受回合管理的交互进程操作 |
| `process.session` | 不通过 | 不能用 `process.exec` 的交互功能推定该能力已实现 |
| `terminal.pty` | 不通过 | 未列入支持清单 |
| `skill.catalog` | 不通过 | 宿主读取已选择的 Skills，不等于支持全部 Skill 管理能力 |
| `skill.describe` | 不通过 | 同上 |
| `skill.invoke` | 不通过 | 同上 |
| `skill.hooks` | 不通过 | 同上；执行钩子需独立校验 |
| `agent.delegate` | 不通过 | 未接入该子 Agent 委派能力 |
| `agent.fork` | 不通过 | 未接入该分支能力 |
| `fs.delete` | 通过 | `delete_path` |
| `fs.watch` | 不通过 | 未接入该文件监听能力 |
| `fs.snapshot` | 通过 | `workspace_snapshot`；当前为会话基线，不能恢复文件 |
| `vcs.stage` | 通过 | `git_stage` |
| `vcs.commit` | 通过 | `git_commit` |
| `vcs.push` | 通过 | `git_push`；仅已配置的本地/file 远程，不支持 SSH/HTTPS 凭据、强推和删除引用 |
| `mcp.connect` | 不通过 | 当前使用冻结 MCP 工具映射和宿主资源端口 |
| `mcp.tool_proxy` | 不通过 | 不支持预设中的原生全工具代理能力 |
| `mcp.resource` | 通过 | 使用宿主资源端口；仍需资源和权限配置 |
| `mcp.oauth` | 不通过 | 未接入这个能力 ID |
| `web.search` | 不通过 | 未列入内置支持清单；另行选择的 MCP/插件工具是不同能力 |
| `web.fetch` | 不通过 | 同上 |
| `citation.render` | 不通过 | 未列入支持清单 |
| `llm.vision` | 通过 | 还需兼容的图像输入模型与资源 |

共 14 项通过内置能力 ID 准入、21 项不通过。12 个标准工具的模型名称与操作定义位于 `crates/backend/nomifun-coding-engine/src/standard_tools.rs`；工厂在 `coding_runtime_host.rs` 中组装工具计划、Skills、资源端口和输入追加端口。

`crates/backend/nomifun-ai-agent/src/runtime_admission.rs:103` 会拒绝任何不在支持集合中的已启用能力。`crates/backend/nomifun-agent-control-plane/src/compiler.rs:335` 将引擎校验失败转换成 `AGENT_RUNTIME_ENGINE_UNAVAILABLE` 并清除候选快照。因此完整预设直接选择 Coding 后，在编译能够走到引擎校验的情况下会被拒绝；不能通过只扩充白名单来声称功能已支持。

## 3. 默认 Nomi 路径也存在 MCP 配置冲突

官方 Coding 预设同时启用了 `mcp.resource` 和 `mcp.tool_proxy`。

`crates/backend/nomifun-app/src/router/runtime_engines.rs:274` 明确拒绝这一组合：平台 MCP 资源应与冻结的逐工具授权配合，不允许同时采用原生全工具代理。

`crates/backend/nomifun-app/src/router/state.rs:967` 在未显式指定引擎时也调用 Nomi 校验。所以保留截图中的默认选择并不能避开这一阻塞；其他模型、资源或能力检查也可能更早报告错误。

## 4. 工作台没有提前显示引擎兼容性

`OfficialTemplateOverview.tsx:48` 的保存可用状态仅依据能力是否已物化。

`AgentRuntimeEngineSelector.tsx` 根据引擎目录生成选项，不接收完整能力清单，也没有编译兼容性结果。因而用户能选到 Coding 并看到可保存状态，最终仍可能被服务端拒绝。能力存在于平台与能力能被当前引擎执行，需要分别表达。

## 5. 修复方向

1. 以完整 Coding Agent 能力契约为目标，逐项接通 21 项缺口的真实处理器、工具/上下文映射、权限边界、资源绑定和恢复行为，复用现有宿主实现；不要只放宽白名单。
2. 统一预设中的 MCP 接入方式，处理 `mcp.resource` 与 `mcp.tool_proxy` 的冲突，并同步更新官方预设、能力基线及相关契约。
3. 在兼容性成立后，让前端模板初始化和服务端模板创建一致地选择 Coding Runtime；明确构建/通道选择策略，不隐式改变已有 Agent 或会话。
4. 工作台调用权威的预览编译结果，提前列出不兼容能力；不能仅根据平台能力是否存在显示“可保存”。
5. 增加完整 `coding.codex` 的生产链路测试，覆盖预设创建、保存、会话启动、35 项能力的执行或生命周期行为、恢复和失败处理。

官方模板来源路径的 `validate_template_baseline` 还要求保留完整能力基线；直接删除不支持能力会改变产品承诺，并可能触发 `CODING_CODEX_NATIVE_INCOMPLETE`。工作台“另存为我的 Agent”当前通过普通创建接口提交完整 document，不能把它与保留官方模板来源的创建路径混为一谈。

## 6. 本次验证范围与结果

- 从预设 JSON 提取全部 35 个能力 ID，与实际 Coding 内置准入源码交叉核对，得到 14/21 的差异。
- 阅读默认引擎解析、模板创建、引擎工厂、准入、保存编译和模型工具投影路径。
- 执行 `bun test --cwd ui src/renderer/pages/agentSettings/AgentRuntimeEngineSelector.test.tsx src/renderer/pages/agentSettings/engineRevision.integration.test.tsx`：6 通过、2 失败。两项失败发生于启动页探针，原因是测试没有初始化 browser storage generation；不能据此判定引擎执行失败，也不能将整组测试声称为通过。
- 现有 UI 集成测试使用 `chat.minimal` 和伪造服务端保存响应；生产测试 `coding_runtime_production.rs` 的主链路选择的是 `fs.write`。这些用例不能证明完整 35 项官方预设兼容。
- 本次未运行 Rust 生产链路测试或真实模型端到端测试；上述不兼容判断来自当前源码的明确准入与组合约束。
