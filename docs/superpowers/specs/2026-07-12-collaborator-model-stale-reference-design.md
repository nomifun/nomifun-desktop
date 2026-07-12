# 协作模型失效引用彻底清理设计

## 目标

用户删除模型供应商或删除供应商中的模型后，这些供应商和模型不得继续出现在首页或会话内的“协作模型”中，也不得进入后续编排运行。仍然存在且可执行的协作模型必须保持原有顺序和选择状态。

## 根因

供应商删除和供应商列表刷新本身是正确的。问题来自供应商目录之外的两份持久快照：

- 首页把选择保存在 `nomi.orchestrationCollaborators`。
- 会话把“主模型 + 协作模型”保存在 `extra.orchestrator_model_range`。

这两份快照读取后没有与当前供应商目录对账。`GuidCollaboratorSelector` 直接使用旧数组计算选中值和数量，`caps_orchestrator` 也直接信任会话中的旧范围。因此残留既是展示问题，也是潜在的运行时失效模型问题。

## 设计原则

1. 当前供应商目录是模型身份是否仍存在的唯一真源。
2. 当前可执行模型集合是协作模型展示、提交和运行的唯一真源。
3. 初次异步加载没有完成时不能把“目录尚未返回”误判成“所有模型已删除”。
4. 删除与临时禁用分开处理：
   - 供应商 ID 不存在，或模型已不在该供应商的 `models` 中，属于永久失效引用，应从持久状态清理。
   - 供应商或模型仍存在但被禁用，属于暂时不可执行引用；不展示、不提交、不运行，但保留原持久选择，重新启用后可恢复。
5. 前端负责即时正确展示和状态修复，删除协调器负责历史数据清理，编排入口负责最终运行时防线。

## 前端模型目录与对账

### 目录状态

扩展 `useModelProviderList` / `useModelRange`，同时提供：

- `isLoading`：后端供应商列表和 Google Auth 状态均已完成首次解析后才为 `false`。
- `configuredPairs`：供应商和模型仍然存在的 `(provider_id, model)` 集合，包括暂时禁用项。
- `allPairs`：当前可用于协作编排的集合，沿用现有 enabled、`model_enabled`、function-calling 和 `excludeFromPrimary` 规则。

加载期间 UI 不展示旧选中值，也不执行持久清理，避免启动时误删全部选择。

### 纯对账函数

新增一个无 React 依赖的纯函数，输入旧 `TModelRef[]`、`configuredPairs` 和 `allPairs`，输出：

- `retained`：仍然配置存在的引用，保持原顺序并去重；用于回写持久状态。
- `active`：`retained` 中当前可执行的引用；用于 UI、提交和运行。
- `removed`：供应商或模型已真正删除的引用；用于判断是否需要回写。

这样首页、会话和选择器共享同一身份规则，测试不依赖 DOM 或 SWR。

### 首页

`GuidPage` 在目录 ready 后执行对账：

- `GuidCollaboratorSelector.value` 使用 `active`，所以失效或禁用项不显示、不计数。
- 新会话的 `orchestrator_model_range` 只使用当前主模型和 `active` 协作模型。
- 只有 `removed` 非空时，才把 `retained` 回写到 `nomi.orchestrationCollaborators`；禁用项不会因一次启动被永久删除。

### 已有会话

`ChatConversation` 对从 `extra.orchestrator_model_range.models.slice(1)` 水合的协作模型执行同一对账：

- 选择器和主模型切换后的范围重写只使用 `active`。
- 只有存在永久失效引用时，才回写清理后的 `retained` 范围。
- 现有主模型自愈逻辑继续处理被删除的会话主模型；协作模型清理不复制另一套主模型选择策略。

### 选择器自身

`GuidCollaboratorSelector` 仍以 `useModelRange` 的可执行目录生成选项，并只在主模型也属于 `allPairs` 时钉选主模型。目录未 ready 时显示未选状态并禁用交互，不能把未知的旧 token 交给 `NomiSelect`。

## 删除后的历史会话清理

扩展 `IConversationRepository`，提供按 `provider_id` 清理所有会话 `extra.orchestrator_model_range.models` 的专用方法。SQLite 实现必须：

- 仅处理合法 JSON、`mode = range` 且 `models` 为数组的行。
- 删除匹配供应商的模型对象，保持其他对象顺序和其他 `extra` 字段不变。
- 对空数组保持合法的 range JSON；运行时防线会在使用时回退。
- 对畸形 JSON、无范围的会话和其他供应商零影响。

把 conversation repository 注入 `AppProviderDeletionCoordinator`。供应商成功删除后的 `cleanup_soft_refs` 同时清理全局故障转移队列和历史会话范围。该清理保持 best-effort 语义：供应商删除已提交后，清理错误记录日志，但不伪装成删除失败。

从供应商中单独删除模型不会经过供应商删除协调器，因此仍由前端对账和运行时防线覆盖。

## 后端运行时防线

在 `caps_orchestrator` 增加基于 `ProviderSummary` 的纯校验函数：

- 会话持久范围：过滤不存在、禁用或不再可执行的模型，保持剩余顺序并去重。
- 过滤后为空：尝试当前会话主模型；主模型也无效时再走现有 Auto 展开。
- 显式工具参数：不静默修改调用者意图；发现无效 `(provider_id, model)` 时返回明确的 Bad Request。
- `lead_model` 必须在过滤完成后从最终范围第一项计算，绝不能保留已删除的旧主引用。

`create` 和会话原生的扁平派发路径必须使用同一校验函数。这样即使旧数据来自旧版本、备份恢复、删除清理失败或非 UI 调用，也不能生成带失效供应商的 fleet member。

## 数据流

1. 供应商列表或 Google Auth 状态加载完成。
2. 前端将旧协作选择分成 `retained`、`active`、`removed`。
3. UI 只展示和提交 `active`；仅永久删除项触发持久清理。
4. 删除整个供应商时，后端同时清理所有历史会话的软引用。
5. 创建编排前，后端用实时 provider summaries 再校验最终模型范围。
6. 只有通过校验的模型进入 lead 选择、角色匹配和 fleet snapshot。

## 错误处理

- 前端自动清理失败：保留当前已过滤展示，记录明确的 console error；下次目录刷新或页面挂载重试。
- 目录加载失败：不清理旧持久状态，不展示未经验证的残留 token。
- 历史会话 SQL 清理失败：沿用删除协调器的 best-effort 日志语义；运行时校验仍保证安全。
- 显式运行范围含失效模型：向调用方返回具体失效 pair，不创建部分或悄悄变更的运行。

## 测试策略

### 前端

- 纯函数测试：删除整个供应商后只移除其模型，其他模型顺序保持。
- 纯函数测试：供应商存在但单个模型被删除时只移除该模型。
- 纯函数测试：禁用项进入 `retained`、不进入 `active`。
- 纯函数测试：重复引用去重，首次出现顺序保持。
- 加载门测试：目录未 ready 时不触发持久回写。
- 结构测试：首页和会话构建范围使用 `active`，选择器不再消费未经校验的旧数组。

### 后端

- repository 测试：清理多个会话中的目标 provider，保留数组顺序、其他 provider 和其他 extra 字段。
- repository 测试：畸形/无关 extra 不受影响。
- deletion coordinator 测试：供应商软清理同时覆盖 failover queue 和 conversation ranges。
- `caps_orchestrator` 单元测试：会话范围过滤失效 pair、空范围回退、最终 lead 来自过滤后第一项。
- `caps_orchestrator` 单元测试：显式范围遇到失效 pair 返回清晰错误。

### 验证命令

- `bun test <新增前端测试文件> <相关结构测试文件>`
- `bun run typecheck`
- `cargo test -p nomifun-db <conversation cleanup test> -- --nocapture`
- `cargo test -p nomifun-app provider_deletion -- --nocapture`
- `cargo test -p nomifun-gateway caps_orchestrator -- --nocapture`
- `cargo check --workspace`
- `git diff --check`

## 非目标

- 不改变供应商删除的 hard-binding 阻止规则。
- 不改变供应商或模型启用/禁用交互。
- 不迁移协作模型的存储格式。
- 不清理已创建编排 run 的 fleet snapshot；历史运行必须保持可审计，修复只影响后续展示和新运行。

## 验收标准

1. 删除供应商后，其供应商组、模型选项、已选 token 和协作模型计数均不再出现。
2. 删除供应商中的单个模型后，该模型不再出现，其他协作模型不受影响。
3. 首页旧配置和已打开会话的旧范围自动清理；删除整个供应商时历史会话范围也被批量清理。
4. 禁用但未删除的模型不展示、不提交、不运行，重新启用后仍可恢复原选择。
5. 应用启动加载期间不会误清空用户的全部协作模型。
6. 新编排的最终模型范围和 lead 中不可能包含当前不存在或不可执行的供应商/模型。
