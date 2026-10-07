# 插件开发模块实施记录

状态：实施中。完整范围以 README.zh.md 为准；以下结果只证明实际覆盖的行为，不能宣布整个重构完成。

## 当前实现

- 可选插件开发模块、普通会话工具调用、默认 Agent preflight、工作台保存继续和来源关联已接入。Agent 与 GUI 复用同一 Plugin Core。
- 独立 Creator、生成与取消 API、整包 JSON 生成和独立 worker 已删除。
- 草稿增量修改、结构检查、真实 UI/headless 预览、Action/DOM 验证和准确 Artifact 安装已接入。验收用例在执行前持久化，失败、超时和取消不能抹去必需用例。
- Host 检查插件交付要求，模型文本和旧 plugin_id 不能证明本次交付。授权 token 不返回模型；缺少授权通过现有 Runtime 暂停，卡片使用 canonical execution/resume 续作。
- 普通 Agent 首次运行与恢复共用受管工作目录逻辑，不额外授予 Workspace 能力。
- 会话展示成果、预览、验证、文件和未交付续作入口。过期响应不会覆盖新页面状态，新的授权请求重置确认框。
- 008 迁移添加来源、请求指纹、幂等键和验证事实。009 保留旧消息为不可变 imported_context，删除 messages_json 与 generating 状态，不伪造会话或执行回执。
- 删除已安装插件时清理草稿基线引用，保留历史交付记录。Plugin Core 和业务数据独立于开发模块。
- 作者指引遵守真实包合同：UI 为 ui/index.html，Service 为 service/main.mjs。
- 010 迁移在统一安装 journal 中保存草稿关联，插件指针与草稿基线原子提交。激活回滚同步恢复基线，未变更的已安装版本保存不重新安装。
- 正式交付记录关联实际安装版本与 Binding，适用只读 Action 使用正式实例调用并验证 schema；写入与外部效果不在生产重放。UI 交付需要实际已安装 Surface 的 SDK/Host 往返，超时保留已安装但未交付状态。
- 修改或续作入口的初始读取消息不承担立即交付义务；用户补充需求后，实际文件修改自动关联当前请求并触发交付校验。新请求可以更新验收条件，同一请求内的失败条件不能被弱化。
- 请求合同可声明 expected_count（1–32），Host 同时核对本次实际成果和已声明数量。该字段的用例证明部分交付不能结算，不证明自然语言需求的自动拆分已经完整。
- 开发模块的预览清理参与现有 Runtime 的清理证明，仅启用此模块的会话执行该分支；临时正式 Surface 在验证结束或中断后关闭。草稿、验证记录和已安装应用保留。
- UI 验证等待检查同一 canonical Turn 的取消事实，停止不再等待完整的 25 秒 UI 超时。安装提交之后取消仍保留已安装实例，但不能产生交付成功记录。
- 已交付后被删除的实例在会话中标为历史成果，不再显示为仍在开发。preflight 保留真实服务错误，不把全部错误归为 Agent 配置缺失。
- 插件暂停任务可提交补充文字：输入写入原 canonical Turn 的 steer 记录，再复用原执行恢复；不新建会话或回合，不增加预算，也不自动证明资源清理。过期状态和同键改写均被拒绝。
- 工作台往返保留发送前编辑的需求和附件引用，沿用同一 owner 和同一意图 token。
- 模块查询复用 Core 的凭据引用列表。验证上下文关联 SDK、Host 版本与所用凭据的加密记录摘要；不返回加密记录或明文。引用轮换或停用后旧证据不能结算，新预览清除旧交付投影。
- 新增 plan 创建操作，源码修改前固化本请求的全部成果标识、当前草稿的成果类型、必需功能和对应精确用例。修复期间拒绝改写已接受计划；不同请求可以重新规划。计划仍由同一 Agent 提取，不另建需求产品或调度器。
- Host 校验 UI/Service 形态和所有计划成果的分别交付。持久化要求关联实际重建后的读取；test_action 的 restart 使用 Core 重建 Service 并保留临时 DataRoot，test_ui 使用 reopen 后的真实断言。
- 计划可声明当前会话的实际使用用例。结算读取本回合真实安装结果之后的 Agent dispatch、原输入与成功 settlement；全局注册或开发模块的预览调用不算消费证明。未取得证明时明确保留为当前会话接入待完成。相关实际 Runtime/Node 接续用例已通过，真实 Provider 驱动的产品验收仍未完成。
- 当前会话的工具接入在清理已证明的原任务暂停点重新构建 Runtime 和冻结消费视图，保持 Agent、Snapshot 与请求身份。活动的关联 AgentExecution 会阻止重建。Host 暂停有效完成提议时保留可恢复的结果批次，不把它投影为已成功交付。
- 按用户补充要求，通用官方预设默认启用插件与小程序模块及完整的 11 项创建操作。工作台按账号、数据集和 Agent 保留编辑草稿与选择，侧栏切页返回不再重置开关；个人配置保存、放弃修改与版本冲突仍遵守原配置合同。开启和关闭均保留为草稿，正式生效仍需保存。
- 插件库、插件导航计数和固定入口订阅会话开发模块发出的 plugins.changed，并在连接恢复后重新读取服务端列表。保留 GUI 的本地变更通知；列表忽略已过期响应，避免旧空结果覆盖新生成的条目。创建仍进入会话，草稿可返回原会话，已保存插件可打开统一详情。

## 当前证据

| 检查 | 结果与覆盖范围 |
| --- | --- |
| plugin_e2e，009 后 | 8 项通过。普通会话完成实际工具写入、Node 执行、失败用例、拒绝弱化预期且无额外调用、修复、两次授权恢复、安装、正式命令调用，以及删除后历史保留；另覆盖 Core 存储、备份与不同形态。Provider 决策为脚本，不证明真实模型成功率。 |
| plugin_authoring_migration | 1 项通过。旧 schema 的三种状态保留原始历史、ID、文件和时间；历史不可修改且 generating 不再可写。 |
| id_schema_contract | 6 项通过，覆盖迁移链、初始化 schema、ID、索引和 guard 注册。 |
| native_execution_recovery 的 owner_pause_resume | 1 项通过，原任务与写入跨恢复代次保留。009 没有修改恢复代码。 |
| UI 模型、边界、bridge 和续作 wire | 19 项通过；不代替视觉验收。 |
| TypeScript | 当前历史合同和续作变更后通过。 |
| Cargo 默认 features | 009 后的 cargo check -p nomifun-app 通过。 |
| i18n | 当前文本的类型与中英文检查通过。 |
| 原子安装 lifecycle | 8 项通过，其中 3 项新增：提交后激活中断恢复、草稿 CAS 冲突不损害旧版本、激活回滚恢复正确基线。 |
| SDK | 最新 11 项通过，包括正式 Surface 就绪时的 Host 往返；不代替实际产品交互。 |
| 正式交付主链路 | 最新 plugin_e2e 10 项通过（执行 10.56 秒）。增加普通修改回合自动校验、两项请求不得由一项成果结算，以及正式 UI 验证期间取消：3 秒内返回、临时 Surface 被撤销、安装保留、无成功交付。最后一项使用 fixture UI 回报验证中断合同，不替代真实 DOM 验收。 |
| 桌面构建和入口观察 | 最新 frontend build 和带 tauri/custom-protocol 的隔离 desktop build 通过。独立复制验收二进制，避免其他构建覆盖。真实窗口重新核验插件库创建进入工作台、完整创建操作默认勾选与中文操作名称、缺模型提示和编辑保留；旧空状态入口已消失。实际缩至 880×600 后入口与保存反馈可用。 |
| 本轮前端检查 | 12 项插件边界、canonical resume 和读取入口合同测试通过；类型、7503 个国际化 key、Desktop UI boundary、Unified Plugin 与 Agent vocabulary 检查通过。 |
| 本轮数据库检查 | id_schema_contract 6 项与历史迁移 1 项通过，包括新增的当前任务消息引用。 |
| 补充输入与恢复 | Native Store 定向用例 1 项通过；实际会话恢复后 Provider 请求包含补充文字，重复提交仍为 1 个原回合和 1 个输入回执，过期 checkpoint 与额外预算被拒绝。Provider 为 wiremock，不证明真实生成成功率。 |
| 凭据上下文 | 定向用例 1 项通过：加密记录变化使摘要失效，停用及明文引用被拒绝，报告不包含记录内容。相关 plugin_e2e 10 项在这些变化后通过，执行 9.99 秒。 |
| 最新前端 | 回复、读取入口、往返和边界 15 项通过；类型、7506 个国际化 key、Desktop UI boundary、Unified Plugin 检查与 frontend build 通过。新增回复表单尚无真实模型驱动的窗口验收。 |
| 最新桌面 | 回复及凭据上下文变更后的隔离 desktop build 通过，耗时 1 分 5 秒。二进制已复制到隔离验收 App；保留正在运行的窗口和数据集，下一次启动使用新版本。日志 /tmp/nomifun-plugin-reply-desktop-build.log。 |
| 实现前计划 | 模块合同 3 项通过。最新 plugin_e2e 10 项通过（10.15 秒），其中拒绝失败后弱化计划、拒绝弱化原用例且没有额外调用、修复、实际写入样本并重建 Service 后读取，以及计划要求两项时一项不得结算。 |
| 计划相关边界 | TypeScript、15 项前端定向用例、7507 个国际化 key、Desktop UI boundary、Unified Plugin、Agent vocabulary 与 generated contract check 通过。 |
| 计划构建 | frontend build 与隔离 desktop build 通过；前端完成后同步检查桌面包，增量构建 6.84 秒。已复制到隔离验收 App，保留正在运行的窗口和数据集。 |
| 消费证据定向检查 | 1 项通过：精确安装之后的目标 Action dispatch、输入与 settlement 才可作为证明；开发模块调用、不同输入或缺少安装边界不被接受。该用例为协议 fixture，尚未证明新工具在真实会话安全接续后实际调用成功。类型、7508 个国际化 key 与 Desktop UI boundary 检查通过。 |
| 原会话实际消费 | 最新 plugin_e2e 11 项通过（10.92 秒）。新增用例真实安装 agent.tool、暂停、原会话安全恢复、重建冻结工具视图并经真实 Agent invoker/Node 返回 CURRENT CALL 后完成。Provider 决策为脚本，但消费和执行不是模拟回报。另有 history 回归 3 项及原 owner_pause_resume 1 项通过。 |
| 消费路径构建 | 最新 frontend build 与隔离 desktop build 通过，已更新隔离 App 的下一次启动二进制，保留运行窗口与数据。日志 /tmp/nomifun-plugin-current-use-ui-build.log、/tmp/nomifun-plugin-current-use-desktop-build.log。 |
| 通用默认与开关保留 | 23 项工作台定向测试通过，覆盖通用默认开启、手动关闭及重新开启后侧栏往返、个人 Agent 草稿恢复与放弃修改、账号和数据集隔离、旧版本草稿拒绝恢复。TypeScript、Desktop UI boundary 与 generated contract check 通过；官方预设目录 3 项服务端集成检查通过，包括通用配置实际编译与插件安装后的目录可用性。日志 /tmp/nomifun-agent-plugin-settings-tests.log、/tmp/nomifun-agent-plugin-default-contract-tests.log。 |
| 创建入口默认文案 | 2026-10-03 修复 pluginIntent 持续触发会话重置、消费导航状态后又清空输入框的问题。重置只由一次性导航请求触发。最新隔离桌面窗口实际点击插件库“创建插件”后跳转会话，输入框显示并保持“帮我创建一个插件”；没有发送消息。相关导航与插件入口测试 10 项、TypeScript、Desktop UI boundary、frontend build 和隔离 desktop build 通过。日志 /tmp/nomifun-plugin-prefill-navigation-tests.log、/tmp/nomifun-plugin-prefill-ui-build.log、/tmp/nomifun-plugin-prefill-desktop-build.log。 |
| 会话生成后显示在插件库 | 19 项前端定向检查通过，其中 3 项页面交互验证创建入口、返回后的草稿与插件展示和打开、会话更新与断线恢复后的列表计数刷新，以及旧响应不能覆盖新条目。最新 plugin_e2e 11 项通过（10.78 秒），包括真实 Node 修复、安装后插件库 API 包含成果及原会话消费；Provider 决策为脚本。TypeScript、Desktop UI boundary、Unified Plugin boundary 和 frontend build 通过。日志 /tmp/nomifun-plugin-library-display-ui-tests.log、/tmp/nomifun-plugin-library-display-e2e.log、/tmp/nomifun-plugin-library-display-ui-build.log。 |
| Desktop UI、Unified Plugin 与 Agent vocabulary | 当前变更后通过；最低视口仍为 880×600。 |
| 较早结果 | 模块合同 2 项、SDK 10 项和平台检查已通过。未改变的部分不重复运行。 |

最新日志：/tmp/nomifun-plugin-cancel-aware-e2e.log、/tmp/nomifun-plugin-task-scope-db.log、/tmp/nomifun-plugin-task-scope-ui-tests.log、/tmp/nomifun-plugin-task-scope-typecheck.log、/tmp/nomifun-plugin-task-scope-ui-build.log、/tmp/nomifun-plugin-bundled-desktop-validation.log。最低窗口截图：/tmp/nomifun-plugin-desktop-minimum-layout.jpg。

回复与上下文的最新日志：/tmp/nomifun-plugin-reply-store.log、/tmp/nomifun-plugin-reply-runtime-fixed.log、/tmp/nomifun-plugin-credential-context-fixed-test.log、/tmp/nomifun-plugin-reply-context-workflow.log、/tmp/nomifun-plugin-reply-final-ui-tests.log、/tmp/nomifun-plugin-reply-launch-typecheck.log、/tmp/nomifun-plugin-reply-final-ui-build.log。

计划的最新日志：/tmp/nomifun-plugin-plan-contracts.log、/tmp/nomifun-plugin-plan-immutable-workflow.log、/tmp/nomifun-plugin-plan-generated-contract-check.log、/tmp/nomifun-plugin-plan-typecheck.log、/tmp/nomifun-plugin-plan-ui-tests.log。

计划构建日志：/tmp/nomifun-plugin-plan-ui-build.log、/tmp/nomifun-plugin-plan-desktop-synced-build.log。

消费证据日志：/tmp/nomifun-plugin-consumer-evidence-test.log、/tmp/nomifun-plugin-consumer-typecheck.log。

原会话消费最新日志：/tmp/nomifun-plugin-current-use-full-workflow.log、/tmp/nomifun-plugin-current-use-history-regression.log、/tmp/nomifun-plugin-current-use-owner-resume-regression.log、/tmp/nomifun-plugin-current-use-ui-regression.log。

## 剩余工作与顺序

1. **真实生成与原失败基线。** 使用可用且支持工具调用的实际 Provider，重复运行 UI-only、headless、mixed 和更新需求；核对完整需求映射、修复次数、失败阶段与交付事实。脚本 Provider 不替代该证据。
2. **完整桌面流程。** 在上述真实任务中完成工作台保存往返、授权或补充输入、当前会话消费、880×600 使用、完整退出重启及数据恢复的窗口验收。当前入口已观察，完整创建尚未实测。
3. **依验收结果修正及最终审查。** 只针对实际失败修复，完成相关维护性审查与必要检查，不反复扩展设计或重跑无变化的绿色项目。

当前外部依赖：隔离验收数据库再次查询为 0 个启用 Provider；原环境只有已停用的 NomiFun Free Model。已请求用户提供已有 Provider 名称或在隔离窗口配置模型，仍未收到可用配置。没有读取或输出用户凭据。该条件已连续多个目标回合保持不变。

尚未达到完成条件：真实模型对自然语言全部需求的正确提取与验收计划映射，以及真实 Provider 的生成与桌面完整验收。实现前计划、执行证据和当前会话消费已取得相关代码与集成证据；现有绿色测试不能替代上述真实模型验收。隔离数据集再次核对仍为 0 个启用 Provider。

## 执行效率

- 按剩余项推进，不反复审计已证明的行为；只根据具体缺口增加工作。
- 先定位合同或失败阶段，再修改，减少尚无证据的推演。
- 相关修改成批完成后运行最小检查。只在代码变化、失败或新增疑点时重复验证。
- 独立读取和前端检查并行；共用目标目录的 Cargo 检查顺序运行，避免锁竞争。
- 只跟踪工具确认的有效 handle，日志独立命名；已结束的命令不重复启动。
- GUI 和外部模型等待不阻塞其余工作，脚本测试不替代真实生成验收。
- 本轮发现共享构建目录被另一条 cargo run 持有，等待后继续，没有中断无关进程。插件停止曾等待完整 UI 超时，已用真实中断用例定位并修复。
- 共享 API 调用处仅补 plugin_delivery: None 以保留原业务语义。Agent 启用策略按用户补充要求仅调整通用官方预设的插件开发模块；操作名称修正仅匹配 plugin.development。桌面启动配置仅存在隔离临时 App，不修改产品配置或用户数据集。
- 2026-10-02 耗时核对：目标累计约 8 小时 9 分钟，主要问题为过长推演和重复收尾。后续固定剩余验收清单，只按已证实缺口修改，相关变更批量验证；已通过项在没有新变化或失败时不再重复检查。缺少真实模型的部分保留外部依赖，不用继续扩展设计替代验收。

原有概念 assets 未修改；保留用户和其他会话的工作。
