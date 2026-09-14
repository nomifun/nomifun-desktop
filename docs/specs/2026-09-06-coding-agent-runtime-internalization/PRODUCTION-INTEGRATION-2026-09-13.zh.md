# 默认生产 owner 的首批 Coding 接入

分支：`rf/agent-capability-platform-v2`。仅本地开发，不 push。
本记录延续 CAR-D-019；不是迁移到隔离 Fresh-v4 的另一套 Session 权威。

> 后续新增实现见 [CODING-LOOP-2026-09-13.zh.md](CODING-LOOP-2026-09-13.zh.md)：
> process.exec、根仓库指令、自动压缩、执行反馈与关闭轮次重放已继续接线，但按用户
> 要求未验证。下文是首批接入的历史记录，其“尚未接入”描述与验证结果仅对应当时切片。

## 接线结果

默认链路为 Conversation-backed Session owner → 现有 Runtime Registry →
exact-binding Catalog → Nomi/Coding/用户注册实现。Agent 未配置引擎时默认 Nomi，
不改变无 binding 的旧会话路径。所有新 AgentSession 保存 family/build/digest/
host-contract/profile；执行时不重新解析 channel。同一会话不能热换引擎，
派生会话继承父会话的精确绑定，不提供独立引擎覆盖参数。

产品配置归属已按用户反馈纠正：入口在 **Agent 工作台 → 选中 Agent → 设置 →
运行时引擎**，首页不出现 runtime 选择器。`GET /api/runtime-engines` 返回注册描述；
Agent 的 `draft.document` / 不可变 Revision payload 可保存：

```json
{
  "runtime_engine": {
    "selector": { "selection": "channel", "family_id": "nomifun.coding", "channel": "stable" },
    "profile": "coding"
  }
}
```

工作台选择器采用 exact selector，目录/profile 来源均为宿主发现结果。
引擎配置参与 Revision digest、草稿脏状态、预览与保存。旧 payload 省略该字段，
序列化与原摘要保持兼容。预览／保存校验安装身份及当前 Coding 能力边界。

`POST /api/agent-sessions` 只选择 Agent，由唯一 owner 从对应的已保存版本解析引擎；
模型覆盖产生的内部 Agent 版本也保留该配置。创建／Fork API 均拒绝独立的
`runtime_engine` 字段。修改 Agent 只影响新会话，旧会话／其 Fork 保留原绑定；
会话内切换到使用不同引擎的 Agent 会报错，须从该 Agent 新建会话。

可复用 `NomiCoreApplication::compose_with_runtime_engines` 注册随应用编译的 Rust
工厂及必需的兼容性策略；按 CAR-D-021 不允许打包后挂载，不提供 HTTP 上传可执行
代码或动态库 ABI。注册在 router 组装后关闭，二次开发须重新构建打包。

## 当前 Coding 能力

准入 `fs.read/search/write/patch/delete` 与 `vcs.status/diff/stage/commit`，
均须在冻结 Snapshot 的 initial 集合中，并绑定服务器解析的 workspace。
工具 schema 来自 Wave2 owner，权限仍经 Kernel，不从提示词获得权限。

Chat Broker 使用生产 provider/connection/credential 库和精确 Chat route。
每次调用先持久记录操作，再在活跃 Conversation epoch/receipt 下原子领取；
重复调用或越过回合边界被拒绝。测试只替代模型 HTTP 响应，不替代 host/kernel。

`conversation_runtime_events` 从属于已有的永久 turn receipt：记录 Coding
语义、模型调用领取及 `host_tool_settled`。这是现有 owner 的执行证据，不是
第二个会话状态机。遵循主库逻辑引用约束，无物理外键；绑定不可变触发器和
receipt 所属校验进入 schema 注册表。记录当前随 receipt 保留，不随 UI 清屏删除。

取消不撤销已提交的文件/Git 副作用。已开始工具在宿主持有的任务中结算，
cleanup 等待真实退出并记录结果后才发布终态；等待者超时不会丢掉证明，
任务 panic 保留隔离。进程执行尚未启用，不声称具备进程退出证明。

宿主从现有 Conversation 提供最近的有界历史候选（最多 4096 条历史，文本总量
含当前输入最多 8 MiB），并始终附加经过持久凭据校验的当前用户输入，包含隐藏的
自动化输入。工具历史仅作为不可信数据，不重放工具。

Coding 自身选择初始模型上下文（默认最多 128 条历史、序列化输入 2 MiB），
按完整历史回合截断并记录 `context_prepared`。每次模型调用前重新检查字节预算；
执行中工具链不能因裁剪而丢失因果关系，超限明确失败，尚无自动摘要压缩。
这些 Engine 算法预算与宿主历史候选窗口是不同层次，不声称已提供完整历史分页 SDK。

兼容性现在通过注册的 `RuntimeEngineAdmission` 校验，不再在 Session 路由中特判
Coding family。Coding 能力查询读取实际工具准入使用的 `SessionCapabilityState`。

## 明确未完成

- 不支持 Process、snapshot/push、on-demand、Skills/MCP/MiniApp、附件和注入 Skills。
  不兼容 Snapshot/选择拒绝；真实运行时还检查最终 Skills/MCP overlay。
- AGENTS 分层上下文、自动 compaction、checkpoint 私有状态恢复尚未接生产。
- 非 Nomi 异常重启回合不借用 Nomi 日志证明安全，保留隔离，不自动重放副作用。
- Remote/Automation 在共享 owner 的创建入口继承 Agent 配置，不另设引擎选择 DTO；
  所有生态消费者的完整端到端覆盖尚未完成。
- 未删除旧 Wrapper/未切换默认引擎；未做付费模型、桌面视觉、macOS/Linux 或发布验收。
- 内置 build digest 是开发期源码/依赖指纹，并非发布制品证明；应用升级时的旧构建
  兼容、保留和迁移策略仍待实现。既有 exact binding 不自动升级；不建设独立 Engine
  动态分发/安装器。通用模型/工具/历史/状态宿主 SDK 仍待抽取。

验证命令与最终结果见 `STATUS.zh.md`、`TASK-MANIFEST.json`，不将本切片标成整个 CAR 完成。
