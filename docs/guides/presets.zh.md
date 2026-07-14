# 设定

**设定（Preset）**是一份可复用的启动配置。它固化 Agent、Execution Step、伙伴
或定时任务应该如何启动，但不会把这份配置变成另一个身份或执行器。

设定库入口是 **`/presets`**。技能是独立领域能力，入口是 **`/skills`**。

## 设定与其他概念的边界

| 概念 | 负责什么 | 不负责什么 |
| --- | --- | --- |
| 设定 | 指令、适用目标、偏好 Agent/模型、技能范围、知识范围、示例和选择元数据 | 运行进程、会话历史或伙伴身份 |
| Agent | Nomi、Codex、Claude、Gemini、远程 Agent 等可执行后端 | 用户可复用配置 |
| 伙伴 | 持久身份、人格、形象、记忆和关系状态 | 可复用启动模板本身 |
| Skill | 可发现、可加载的一项聚焦能力 | agent/模型选择或完整画像 |

因此，同一份设定可以启动普通会话、物化为 AgentExecution Step、配置伙伴，也可以
作为定时任务模板。伙伴画像或一次成功的协作角色也可以复刻为设定，而不会与
来源身份混为一谈。

## 一份设定可以固化什么

设定模型支持：

- 名称、头像、用户描述，以及供 Agent 路由使用的能力描述；
- 多语言名称、描述、指令和示例提示词；
- 适用目标：会话、Execution Step、伙伴、公开伙伴、定时任务；
- 有序的偏好 Agent，以及每位用户单独选择的首选 Agent；
- 带 provider 标识的偏好模型；
- 明确包含的技能，以及禁止自动注入的 builtin 技能；
- 绑定知识库和继承/追加/替换知识范围策略；
- fallback 与是否允许 Agent 协作自动选择；
- 受众/场景标签、启用状态、排序和最近使用状态。

## 来源与编辑规则

设定目录合并三类来源：

| 来源 | 来自哪里 | 编辑规则 |
| --- | --- | --- |
| Builtin | `crates/backend/nomifun-app/assets/builtin-presets/` 内嵌目录 | 内容只读，需要定制时先复制；启用、排序、首选 agent 等用户状态单独存储。 |
| User | SQLite 中的关系化设定记录 | 可完整编辑和删除。 |
| Extension | 已安装扩展的 `presets` contribution | 在设定库只读；生命周期由所属扩展管理。 |

用户指令和头像资源位于 NomiFun 数据目录的 `preset-instructions/` 与
`preset-avatars/`。删除用户设定时会一并清理关联资源。

## 解析与不可变快照

选择设定不只是前端筛选。真正执行前，后端会针对目标调用设定解析器，并按以下
优先级确定配置：

1. 本次启动显式传入的 override；
2. 用户首选 agent，再按设定中的有序偏好尝试；
3. 只有设定允许 fallback 时，才选择可用的兜底项。

解析器会校验 agent/模型可用性，选取本地化指令，合并技能 override，并物化知识
范围，最终生成 `ResolvedPresetSnapshot`。快照同时记录设定 id/revision，以及本次
确定的 agent、模型、指令、技能和知识策略。

会话、定时任务与 AgentExecution Step 都会持久化这份快照。因此目录中的设定后来发生
修改，也不会静默改变已创建的目标。允许自动选择的设定可被 Agent 协作复用；
对支持的目标，用户也始终可以显式选择。

## API

| 操作 | Endpoint |
| --- | --- |
| 列表 / 创建 | `GET`, `POST /api/presets` |
| 读取 / 更新 / 删除 | `GET`, `PUT`, `DELETE /api/presets/{id}` |
| 用户状态 | `PATCH /api/presets/{id}/state` |
| 针对目标解析 | `POST /api/presets/{id}/resolve` |
| 头像 | `GET /api/presets/{id}/avatar` |
| 批量导入 | `POST /api/presets/import` |
| 标签 | `GET`, `POST /api/preset-tags`; `PUT`, `DELETE /api/preset-tags/{key}` |

Builtin 与 Extension 设定的目录内容不能直接修改，需要先复制为用户设定。
CLI 型 Agent 仍要求宿主机安装对应 CLI；把它选为偏好不会自动安装工具。

## 相关

- [MCP 与技能](./mcp-and-skills.zh.md)
- [模型故障转移队列](./model-routing.zh.md)
