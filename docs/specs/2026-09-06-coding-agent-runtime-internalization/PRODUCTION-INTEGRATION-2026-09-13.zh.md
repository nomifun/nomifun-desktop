# 默认生产 owner 的首批 Coding 接入

分支：`rf/agent-capability-platform-v2`。仅本地开发，不 push。
本记录延续 CAR-D-019；不是迁移到隔离 Fresh-v4 的另一套 Session 权威。

## 接线结果

默认链路为 Conversation-backed Session owner → 现有 Runtime Registry →
exact-binding Catalog → Nomi/Coding/用户注册实现。新建会话默认 Nomi，
不改变无 binding 的旧会话路径。所有新 AgentSession 保存 family/build/digest/
host-contract/profile；执行时不重新解析 channel。同一会话不能热换引擎，
派生会话默认继承精确绑定，指定 `runtime_engine` 才改为另一已安装实现。

`GET /api/runtime-engines` 返回注册描述；`POST /api/agent-sessions` 与
`POST /api/agent-sessions/{id}/forks` 接受：

```json
{
  "runtime_engine": {
    "selector": { "selection": "channel", "family_id": "nomifun.coding", "channel": "stable" },
    "profile": "coding"
  }
}
```

界面新建选择器采用 exact selector，目录/profile 来源均为宿主发现结果。
可复用 `NomiCoreApplication::compose_with_runtime_engines` 注册受信任的 Rust
工厂；不提供 HTTP 上传可执行代码或动态库 ABI。注册在 router 组装后关闭。

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

恢复上下文来自现有 Conversation 的有界消息（最多 4096 条/1 MiB），
工具历史仅作为不可信数据，不重放工具。当前隐藏的自动化输入也被纳入。

## 明确未完成

- 不支持 Process、snapshot/push、on-demand、Skills/MCP/MiniApp、附件和注入 Skills。
  不兼容 Snapshot/选择拒绝；真实运行时还检查最终 Skills/MCP overlay。
- AGENTS 分层上下文、自动 compaction、checkpoint 私有状态恢复尚未接生产。
- 非 Nomi 异常重启回合不借用 Nomi 日志证明安全，保留隔离，不自动重放副作用。
- Remote/Automation 仍共享默认 owner，但尚无各自的引擎选择 DTO/完整端到端覆盖。
- 未删除旧 Wrapper/未切换默认引擎；未做付费模型、桌面视觉、macOS/Linux 或发布验收。
- 内置 build digest 是开发期源码/依赖指纹，并非已签名发布制品证明；正式多版本
  分发、旧构建保留和升级迁移策略仍属发布工作。既有 exact binding 不自动升级。

验证命令与最终结果见 `STATUS.zh.md`、`TASK-MANIFEST.json`，不将本切片标成整个 CAR 完成。
