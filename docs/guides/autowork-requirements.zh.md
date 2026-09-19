# AutoWork 与 Requirements

AutoWork 是 NomiFun 的旗舰自动化能力：一块 **需求看板**（requirements board）加上每个 AgentSession 各自的 **执行循环**，由它驱动 Session 中冻结的 Agent 逐条处理这些需求，无需你全程盯着。

你登记需求，按 tag 分组，把 tag 绑定到一个 AgentSession，AutoWork 循环就会按顺序认领、执行并完结它们。当某条需求进入终态时，可以触发 **完成通知**（Lark/飞书 webhook），让你的团队第一时间知道结果。

这里描述的所有内容都是 **后端权威** 的：AutoWork 在进程启动时自动恢复，无论你是否打开 UI 都会运行。

![AutoWork tag-sessions 总览](../images/autowork-01-tag-sessions.png)

## 概念

| 术语                  | 含义                                                                                                                                                |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Requirement**       | 一个工作单元：标题、内容（实际指令）、tag、`order_key`（按字典序比较的字符串）以及状态。存储在 SQLite 中。                                                |
| **Tag**               | 任意字符串，用来把需求归入一个队列。绑定关系、看板列以及 webhook 路由都以 tag 为键。                                                                          |
| **Status**            | `pending` → `in_progress` → `done`（或 `failed` / `cancelled`）。看板视图每个状态对应一列。                                                            |
| **Claim & lease**     | AutoWork 循环原子地把某 tag 中 `order_key` 最小的 `pending` 需求转为 `in_progress`，并写入一份带过期时间的租约（lease）。                                          |
| **Lease sweeper**     | 一个后台任务（每 60 秒一次），把租约过期且 owner 已不存活的 `in_progress` 行停放到 `needs_review`；仅凭过期不能证明模型或工具效果可安全重放。 |
| **AutoWork 循环**    | 每个目标对应一个循环：认领 → 注入 → 等待 → 完结 → 重复。每个绑定的会话有一个循环。它是常驻的：队列空了就空闲等待，不会退出。                                  |
| **Target**            | 一个具有 immutable Agent binding 和已解析模型的 canonical AgentSession。Terminal AutoWork 已退出产品。 |
| **执行回执** | AgentExecution 统一拥有 Attempt、AgentSession Turn 和 canonical terminal receipt；AutoWork 只根据这份持久结果推进队列。 |
| **Completion notifier** | 当需求进入 `done`/`failed`/`cancelled` 时触发的 Lark/飞书 webhook。按 tag 绑定。                                                                  |

## 单条需求的生命周期

```
pending  ──claim_next()──▶  in_progress (lease)  ──admission──▶  AgentExecution runs
                                  │                                   │
                                  ▼                                   ▼
                       sweeper parks ambiguous           AgentExecution receipt
                       expiry in needs_review                        │
                                                                     ▼
                                                            done | failed | cancelled
                                                                     │
                                                                     ▼
                                                       CompletionNotifier fires (best-effort)
```

当 tag 为空时 AutoWork 循环 **不会** 退出。它会等待唤醒通知（外加一个 10 秒兜底轮询），并永久持续认领，因此向已绑定的 tag 新提交的需求几乎是即时被拾取。

它仅在以下情况退出：

- 你对该目标关闭了 AutoWork；
- 绑定触达了 `max_requirements` 上限（此时配置会被持久化为已禁用，使该上限在重启后依然生效）；或
- 对应的 AgentSession 被删除。

## 三种视图

AutoWork 在每个视图中的数据完全相同，视图只是不同的"镜头"。

### 需求列表 — `/requirements`

扁平表格。可按 tag、状态或全文搜索过滤。可批量删除选中行。点击行可以打开详情抽屉；**编辑** 路径是 `/requirements/:id/edit`，**新建需求** 通过 `/requirements?new=1` 打开，旧的 `/requirements/new` 会重定向到这里。

![需求列表](../images/autowork-02-list.png)

### 看板 — `/requirements?view=board`

针对所选 tag，每个状态一列。这里有意 **不** 通过拖拽来改状态；请使用详情抽屉。看板会在每次 `requirements.*` 实时事件触发时重取数据，因此能跟随 AutoWork 循环实时变化。

![需求看板](../images/autowork-03-kanban.png)

### Tag sessions — `需求平台 → 扩展能力 → 自动执行`

AutoWork 的管理面板（`/requirements/extensions?tab=autowork`）。列出所有 tag、所有 AgentSession 绑定及每条绑定的实时状态（`Idle`、`Active` 或 `Paused`）。每个 tag 的完成 webhook 现在在旁边的 **通知** tab（`/requirements/extensions?tab=notify`）。

这里是你"巡视舰队"的地方。要在某条绑定上 **启动** AutoWork，请打开会话本身并在那里切换 AutoWork 开关——那才是绑定 tag、设置 `max_requirements` 和持久化配置的标准位置。

![Tag sessions 管理面板](../images/autowork-01-tag-sessions.png)

## 提交一条需求

在列表页点击 **新建需求**（或访问 `/requirements?new=1`）。表单包含：

- **标题**：简短的标签。
- **Tag**：选择已有 tag 或键入一个新值。tag 在首次使用时会被创建。
- **内容**：交给已绑定 Agent 的实际指令。当作 ticket 来写：上下文足够让智能体不必反问就能开始，并附上清晰的“完成定义”。
- **Order key**：用于队列排序的字符串。按字典序排列，因此常见模式如 `1.0`、`1.1`、`1.2.0` 等等。值越小越早。
- **状态**：默认是 `pending`。你也可以在这里手动把某行标记为 `done` 或 `cancelled`。

提交后该行进入队列。如果已有会话绑定到该 tag，它会立刻被唤醒并开始处理这条需求（前提是没有别的需求排在它前面）。

## 绑定 AgentSession

一条绑定形如 `(conversation, agent_session_id, tag, max_requirements?)`。

打开任意会话。头部有一个 **AutoWork** 控件。选择 tag，可选地设置完成上限，然后启用。

每一轮中发生的事：

1. AutoWork 循环认领该 tag 中下一条 `pending` 需求。
2. 它把该 claim generation 作为幂等 source 提交给 AgentExecution，并使用所选 Session 中冻结的 Agent Snapshot 与 Resource bindings。
3. AgentExecution 统一拥有 Attempt Session、retry/adaptation、等待人工、取消与 canonical Turn receipt；绑定会话显示关联执行，并接收最终报告。
4. 成功回执把 Requirement 归约为 `done`；失败、效果不确定或不安全取消会归约为 `failed`/`needs_review` 并暂停 tag，不会自动重放效果。
5. 会话头会明确显示 `Paused` 和原因。检查关联执行后点击 **恢复执行**；只有这次显式用户动作才会重排失败项并继续队列。

## 启动恢复——它在你不在场时也会运行

AutoWork 循环的活跃集合存放在内存中，但每条绑定的 `enabled`、`tag`、`max_requirements` 都作为 append-only canonical AgentSession automation fact 持久化。进程启动时后端会枚举安装 owner 的已启用 Session 绑定并 **自行启动** 这些循环。要让 AutoWork 工作，你不必打开会话页面；UI 只是展示后端权威状态。

这就是为什么"AutoWork 只在我开着标签页时才工作"是一个 bug 而不是 feature。如果你观察到这种现象，去检查 AutoWork 循环日志中是否有该用户/目标的 resume 失败记录。

## 完成通知（Lark / 飞书）

当需求进入终态时，会调用 `CompletionNotifier`。今天它做的事：

1. 查找该需求 tag 的 **per-tag 设置**——如果该 tag 没有设置或没有绑定 webhook，通知器静默 no-op。
2. 按 id 查找绑定的 webhook；如果它处于禁用状态，no-op。
3. 构造一张 Lark 互动卡片，字段如下：
   `需求id` · `需求名` · `需求内容`（截断到 500 字符） ·
   `完成状态`（`done`/`failed`/`cancelled`） ·
   `完成记录(报告)`（本轮中捕获的 completion note，截断到 500 字符）。
4. POST 到 webhook URL。如果该 webhook 配置了 secret，请求会按 Lark 自定义机器人的标准方案签名（`HMAC-SHA256(key="{ts}\n{secret}", msg="")`，base64）。
5. 失败会以 `warn` 记录并吞掉——一个不稳定的 webhook 永远不会影响需求状态。

### 配置步骤

1. 进入 **需求平台 → 扩展能力 → 通知**（`/requirements/extensions?tab=notify`）并 **Create webhook**：填写名称、Lark 自定义机器人 URL，以及（可选的）匹配 secret。点 **Test** 发一张卡片，验证机器人可达。
2. 在同一个 **通知** tab 里找到该 tag，从 per-tag 下拉框中挑选 webhook。设置按 tag 保存。

你可以随时改变某个 tag 指向哪个 webhook，包括清空绑定以静音该 tag 的通知。

![Per-tag webhook 路由](../images/autowork-05-webhook-binding.png)

## 路由与 API

| 用途                              | 位置                                                              |
| --------------------------------- | ----------------------------------------------------------------- |
| 需求列表                          | `/requirements`                                                  |
| 看板（按 tag）                    | `/requirements?view=board`                                      |
| Tag sessions 管理                 | `/requirements/extensions?tab=autowork`                         |
| 通知配置                          | `/requirements/extensions?tab=notify`                           |
| 新建 / 编辑                       | `/requirements?new=1`、`/requirements/:id/edit`                 |
| 旧版 `/autowork`、`/requirements/tag-sessions` | 重定向到 `/requirements/extensions?tab=autowork`    |
| 旧版 `/requirements/new`、`/requirements/kanban` | 重定向到当前 query-param 路由                       |
| 列出 / 创建需求                   | `GET /api/requirements`、`POST /api/requirements`                |
| Tags                              | `GET /api/requirements/tags`                                     |
| Tag 绑定（管理）                  | `GET /api/requirements/tag-bindings`                             |
| Per-tag 看板                      | `GET /api/requirements/board?tag=…`                              |
| 获取 / 更新 / 删除                | `GET|PUT|DELETE /api/requirements/:id`                           |
| 状态 / 完成                       | `POST /api/requirements/:id/status`、`…/complete`（claim authority 仅供内部 runner） |
| AutoWork 开关 / 状态              | `POST /api/requirements/autowork`、`GET …/autowork/:kind/:tid`   |
| Webhooks                          | `GET|POST /api/webhooks`、`…/{id}`、`…/{id}/test`                 |
| Per-tag webhook                   | `GET|PUT /api/tags/:tag/settings`                                |

## 实现注记（写给好奇的你）

- 活跃 claim 会记录 typed AgentSession owner、单调递增 generation 和 opaque capability；公开状态路由不能铸造或替换这份 authority。
- AutoWork 循环的 `wake` Notify 与 `RequirementService` 共用；任何会重置回 pending 或创建工作的状态变更都会触发它，循环也会在每次 `claim_next()` 调用前后用 armed-then-await 的方式包起来，因此在"claim 返回 None"和"await"之间到达的唤醒永远不会丢。
- claim generation 与 capability 会被哈希为 AgentExecution 的幂等 source operation；opaque capability 不写入 Execution 聚合，也不会暴露给模型。
- completed、failed、partially failed、cancelled 与 outcome-unknown receipt 都有明确的 Requirement 归约；效果不确定时一律停止自动重放并转人工复核。
