# 官方内置 Agent 能力：低成本修复与后续缺口

日期：2026-09-08。起点：`rf/agent-capability-platform-v2` / `52df7dc8b`。

## 本轮边界

用户要求官方随安装包实现的能力具备真实执行路径；后续收窄为先修复低成本项，高成本仅记录。本轮不以改可用性标签代替执行接入。

成本口径：复用已经存在的会话内工具，或对无需持久资源绑定的现有服务增加薄适配。需要新增资源/凭据绑定、模型路由、跨会话身份、后台生命周期或跨领域写入合同的项，列入后续。本清单的“后续”表示超出本轮薄接入范围，不代表底层功能均未实现。

## 本轮实施

| 能力 | 实际接入 | 约束与验证目标 |
|---|---|---|
| `web.fetch` | `web_fetch` 原生工具复用 `nomifun_knowledge::HttpFetcher` | 公共 HTTP(S)、每跳地址校验、超时、Markdown 输出截断；遵守 Preset allowlist 与按需激活。HTML→Markdown 抓取不包含浏览器 JS 渲染。 |
| `agent.delegate` | 映射现有 `nomi_delegate` 与 `LocalAgentInvocationRunner` | 复用宿主 owner 授权和子任务能力上限；Preset 启动即用/按需位置覆盖普通会话默认延迟展示，不扩展父子工具权限。本轮复用既有本地委派，未新增跨会话 Plugin/业务资源传递。 |
| `schedule.store` | 映射现有 `cron_create`、`cron_list`、`cron_delete` 与会话绑定 `CronServiceSink` | 复用宿主 owner 授权、真实 CronService 持久化；补充删除时的会话归属校验。 |

以上三项的 Catalog 初始化和 Plugin Runtime 刷新均使用同一个 Nomi 映射判定，因此不依赖用户插件的 Node Runtime。运行时的网络/模型/服务配置错误仍按实际调用结果报告，不能把实现存在解释为每次调用必然成功。

## 目录统计与证据范围

基线内置声明 137 项，其中 `browser.render_content` 只面向 Knowledge 消费者，Agent 可见 136 项。修复前 39 项通过当前 Nomi 映射、97 项缺少映射；本轮新增 3 项后为 42 项通过、94 项待接入。该数字来自源码与目标清单交叉核对，不表示所有能力均经过真实模型、硬件或桌面端验收。

调查时运行中的桌面 API 直接读取返回 403，因此没有声称完成该旧进程的实时目录导出。回归使用隔离测试状态的真实 Router/Manager/HTTP 服务路径。用户动态安装的插件不计入本表，其原有不可用状态处理保持独立。

## 后续项：94 项，本轮不实施

| 来源包 | 数量 | 能力 ID | 未纳入本轮的原因 / 后续工作 |
|---|---:|---|---|
| `nomifun.model-media` | 9 | `llm.realtime`、`llm.embedding`、`llm.rerank`、`llm.image.generate`、`llm.image.edit`、`llm.video.generate`、`llm.audio.tts`、`llm.audio.asr`、`llm.vision` | 需要逐任务连接模型路由、凭据解析及输入/输出适配；视觉还需明确模型支持与上下文投递。当前声明处理器不能作为真实模型执行器。 |
| `nomifun.web-research` | 2 | `web.search`、`citation.render` | web.search 缺搜索执行服务与结果来源合同；citation.render 缺会话来源集合、稳定引用标识和上下文贡献接入。HTTP 抓取不等同搜索或引用渲染。 |
| `nomifun.agent-execution` | 3 | `agent.fork`、`agent.execution.steer`、`agent.execution.observe` | fork/observe/steer 需要会话、执行标识、权限及生命周期绑定；现有服务端控制接口不等于 Agent 可调用工具。不得直接开放任意会话操作。 |
| `nomifun.workspace-execution` | 4 | `fs.delete`、`fs.watch`、`fs.snapshot`、`vcs.push` | 删除需接会话工作区锚定与真实文件服务；watch 需事件订阅和取消；snapshot 需确定快照持久化合同；push 需远端凭据、分支/授权与执行回执。不能映射为无限制 Bash。 |
| `nomifun.ssh` | 5 | `ssh.connect`、`ssh.fs.read`、`ssh.fs.write`、`ssh.exec`、`ssh.sudo` | 现有 SSH 会话族需要 ssh_host 资源选择、凭据和连接租约贯通，且必须避免本地/远端路径混淆；sudo 另需受控提权输入。 |
| `nomifun.knowledge` | 5 | `knowledge.mount`、`knowledge.source.sync`、`knowledge.autogen`、`knowledge.embedding`、`knowledge.rerank` | 挂载/同步涉及知识库资源绑定与后台任务；autogen/embedding/rerank 涉及模型路由和结果写入。当前 search/read/write 工具不覆盖这些动作。 |
| `nomifun.companion-memory` | 4 | `memory.companion.recall`、`memory.companion.write`、`memory.companion.merge`、`memory.companion.evolve` | 需将 Agent 的 companion_memory 资源绑定贯通到伙伴专属 sink、存储和演进流程，避免写入项目记忆或任意伙伴数据。 |
| `nomifun.mcp-connectors` | 6 | `mcp.connect`、`mcp.tool_proxy`、`mcp.resource`、`mcp.oauth`、`connector.data.read`、`connector.data.write` | MCP 服务端绑定、OAuth、资源读取与工具 materialization 的真实授权链尚未对应这些全局占位 ID；需按实际 Server/Tool 来源接入并保留精确绑定。 |
| `nomifun.browser` | 1 | `browser.site_memory` | site_memory 已有浏览器配置/实现，但需明确 Agent 选择与全局 opt-in、站点作用域、历史存储及隐私设置的关系，不能仅用 HostOnly 标记完成。 |
| `nomifun.requirements` | 4 | `requirements.read`、`requirements.write`、`requirements.status`、`requirements.claim` | 需要把当前会话的需求作用域接到领域读写/认领服务。已有 requirement_update_status/complete 与这四个声明的语义不完全相同，不能靠名称近似映射。 |
| `nomifun.autowork-scheduler` | 3 | `autowork.runner`、`schedule.timer`、`schedule.agent_trigger` | 剩余 runner/timer/agent_trigger 是调度生命周期贡献，需要初始化、触发来源、运行与取消接入；需要确认这些机制是否应作为可选 Agent 能力展示。 |
| `nomifun.idmm` | 3 | `idmm.observe`、`idmm.intervene`、`idmm.fallback_policy` | 观察/干预/回退是回合中间件，需要宿主事件、状态和取消生命周期；不能按普通工具的注册状态判断执行就绪。 |
| `nomifun.companion` | 4 | `companion.persona`、`companion.roster`、`companion.learn`、`companion.evolve` | 需要目标伙伴资源选择、persona/context、学习和演进服务接入；不得借助默认伙伴或复活已删除的 summon 路径。 |
| `nomifun.channel` | 5 | `channel.receive`、`channel.reply`、`channel.send`、`channel.pairing`、`channel.group_policy` | 需要渠道配对、消息入站/出站、群组策略及取消/恢复生命周期；当前部分备用 host 明确返回无执行者。 |
| `nomifun.customer-service` | 4 | `customer_service.dialogue`、`customer_service.notes.read`、`customer_service.notes.write`、`customer_service.handoff` | 需绑定客户、对话、备注和交接目标，并接入服务权限与写入结果；备用 host 的占位动作不能直接进入产品链。 |
| `nomifun.robot` | 6 | `robot.link`、`robot.audio`、`robot.vision`、`robot.display`、`robot.motion`、`robot.device_tools` | 需配对设备资源、音视频流、设备动作与断线取消机制；真实物理动作需要设备验证，无法只靠静态适配完成。 |
| `nomifun.creation` | 5 | `creation.text`、`creation.image`、`creation.image_edit`、`creation.video`、`creation.audio` | 需把 generation_provider、模型任务、资产输出和可追踪产物贯通；已有普通会话图片工具的权限/输出合同不能无条件复用到所有 Preset。 |
| `nomifun.workshop` | 6 | `workshop.canvas.read`、`workshop.canvas.edit`、`workshop.asset.read`、`workshop.asset.write`、`workshop.template.run`、`workshop.director` | 需对接 canvas/asset_library 目标资源与编辑/模板/导演服务，确认当前产品保留的入口；存在跨领域写入与产物生命周期。 |
| `nomifun.office` | 4 | `office.preview`、`office.document.edit`、`office.sheet.edit`、`office.slides.edit` | 需接入真实文档类型、资产资源、编辑服务和产物保存/预览合同；目前能力声明本身没有这些执行端口。 |
| `nomifun.miniapp` | 4 | `miniapp.read`、`miniapp.edit`、`miniapp.publish`、`miniapp.serve` | M1 已有应用服务，但这些 Agent 动作尚需映射 miniapp 目标、草稿/Release、发布/服务权限和执行结果；不在本轮修改 M1 合同。 |
| `nomifun.notification` | 2 | `notification.webhook`、`notification.desktop` | Webhook/桌面通知属于事件消费者，需要宿主投递端口、目标授权和生命周期，并确认是否属于 Agent 自选能力。 |
| `nomifun.remote-ingress` | 5 | `remote.mcp`、`remote.rest`、`ingress.web`、`ingress.mobile`、`ingress.channel` | Remote/Ingress 是平台传输入口，需按消费方重新确认职责与可用性；不能把开放 REST/MCP/移动端入口视为给 Agent 一个普通工具。 |

这些待接入项仍保留在目录/模板中。它们在当前宿主的不可用状态暂时保留；删除占位声明、改变模板集合或调整平台内部贡献的展示属于后续实施范围。

## 验证记录

已执行结果：共 24 项针对性测试通过。

- `cargo test --locked -p nomifun-ai-agent --lib web_fetch::tests`：4 passed。本地 HTTP 实际请求、HTML 转 Markdown、输入错误、地址限制、HTTP 错误、大输出截断与工具上限。
- `cargo test --locked -p nomifun-ai-agent --lib repaired_builtins_register_and_execute_without_widening_preset_scope`：1 passed。三项工具按能力上限注册，空能力拒绝全部，启动即用/按需保持一致；Cron 工具调用测试 sink。
- `cargo test --locked -p nomi-agent --test bootstrap_test preset_delegate_placement_and_ceiling_are_preserved`：1 passed。通过真实 Bootstrap/委派 runner 执行子任务，模型为本地测试 Provider；父子工具权限与两个 placement 均保持。
- `cargo test --locked -p nomifun-cron --test service_integration builtin_schedule_sink_persists_own_jobs_and_rejects_other_conversation_deletion`：1 passed。隔离 SQLite 的创建/读取/删除及跨会话删除拒绝。
- `cargo test --locked -p nomifun-app --lib nomi_core_agent_projection::tests`：15 passed。两个 placement 仅映射对应工具，零能力上限、原有能力和动态插件边界回归通过。
- `cargo test --locked -p nomifun-app --test nomi_core_route_gap nomi_core_catalog_exposes_native_nomi_capabilities -- --test-threads=1`：1 passed。真实 Router 的全局目录返回三项内置能力 `materialized`，错误码为空。
- `cargo test --locked -p nomifun-app --test nomi_core_route_gap nomi_core_accepts_on_demand_placement_without_widening_initial_tools -- --test-threads=1`：1 passed。真实 resolve-preview 接受三项修复能力与既有 VCS 能力的按需选择，initial_count 仍为 0。

未执行安装包构建、运行中桌面重启、真实外部模型/硬件验证；没有将源码统计或测试 Provider 结果写成已安装客户端的 live PASS。

## 源码定位

- `crates/backend/nomifun-app/src/router/nomi_core_agent_projection.rs`：当前产品的原生能力→工具映射。
- `crates/backend/nomifun-app/src/router/state.rs`：声明目录初始化与可用性。
- `crates/backend/nomifun-app/src/router/plugin_platform.rs`：动态 Runtime 后的可用性刷新。
- `crates/backend/nomifun-agent-domain-support/src/lib.rs`：完整内置声明。
- `crates/backend/nomifun-app/src/router/agent_platform_host.rs`：备用 Fresh-v4 的领域执行组合，不能直接视为当前桌面已启用。
- `crates/backend/nomifun-ai-agent/src/web_fetch.rs`：本轮新增的 HTTP 抓取薄适配。
- `crates/backend/nomifun-cron/src/sink.rs`：当前会话的原生定时任务服务适配。
