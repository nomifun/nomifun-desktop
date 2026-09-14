# 生产 MCP 每工具目录与串行刷新（loop14 / loop15）

日期：2026-09-14。分支：`rf/agent-capability-platform-v2`。
状态：源码已写入，**没有运行构建、测试、评测或 E2E**；不是整体 CAR 完成声明。
前序执行端口见 `ENGINE-MCP-PORT-2026-09-14.zh.md`。

## 每工具接入

- 从当前产品 `mcp_servers` 仓库读取已启用、未删除的 HTTP 服务器工具目录。
  每个服务器最多 256 个工具，总目录最多 1024 个工具；配置、目录、schema 和描述有大小限制。
  同一服务器的工具合同有错时整体不发布该服务器，避免产生误导性的半份目录。
- 一个远端工具对应一个 source-owned bundled registration、一个 capability 和一个 invoke action。
  ID 根据服务器 ID 与远端名称生成；schema digest、连接版本和不可变描述进入注册身份。
  不把整个服务器暴露为可由模型随意填写工具名的通用路由器。
- Coding 工具面从 Registry 中的固定描述投影真实 JSON Schema；Snapshot 必须包含精确 MCP lock，
  typed resource 必须指向同一服务器、安装用户和连接版本，并授予 connect/invoke。
  Session extra 不能另外扩展服务器或工具权限。
- 工具输入必须是对象并通过 schema 校验。schema 使用禁止外部读取的 retriever；
  不再递归把任意 JSON 数据中名为 `$ref` 的属性误判为 schema 引用。
- MCP action 一律按 ExternalTransmit 处理，不因为服务器声明 readOnly 就绕过副作用管理。
  使用前序公共工具日志、真实 HTTP owner、任务收敛和不确定调用隔离路径。
- 默认 Nomi 暂时拒绝新路径，包括未显式填写 engine 的默认 Snapshot；原生 Nomi MCP 路径保留。
  这是暂存的能力差异，不是 Engine 必须使用不同平台权限的最终设计。

## 运行中更新

- 应用启动时物化目录；生产 MCP 配置服务在创建、编辑、启停、删除、导入及连接测试落库后请求刷新。
- MCP 与 Plugin 使用同一个 Registry 发布锁，持锁后才读 MCP 仓库。
  发布组合为静态 bundled registrations + 当前 MCP registrations + 当前 Plugin registrations，
  后续 Plugin 更新不会丢失最新 MCP 集合。
- Kernel 验证整组后原子替换。新 MCP 集合在无 await 间隙内留存；目录顺序稳定，
  内容没有变化时刷新不会无故增加 generation。目录可用性也会重新计算。
- 数据库写入与 Registry 发布不是一个事务。若写入已完成而发布失败，API 明确返回
  `CatalogPublicationPending` 对应的 Conflict；不能宣称保存被回滚。
  `POST /api/mcp/catalog/refresh` 可重试发布，不改配置、不重新访问远端服务器。
  请求取消或应用退出发生在两者之间时，可显式刷新或重启重新物化。
- 不修改既有 Agent revision、Session Snapshot 或 engine binding，也不将旧工具名称自动映射到新工具。
  新目录沿现有全局 Registry generation 规则发布；旧 Session 可能因 generation/贡献/连接变化被拒绝，
  不保证对正在执行的其他 Session 透明更新。正在进行的请求仍由原来的 owner/task 收敛。
- 即使目录发布暂未完成，实际调用仍检查当前数据库的启用状态、schema 和连接版本，
  旧目录不是绕过配置撤销的授权来源。

## 连接测试的版本保护

- 产品仓库更新连接版本采用 `MAX(updated_at + 1, now_ms)`，避免同毫秒修改或系统时钟回退复用版本。
- 连接测试前捕获服务端保存的版本。若请求测试的是未保存的编辑器值，可以返回测试结果，
  但不把结果写入当前服务器目录。
- 保存测试结果使用仓库 compare-and-swap：仅在原版本仍然有效、服务器未删除时，
  一次 SQL 更新 status/tools/last_connected/下一版本。较早发出的慢请求不能覆盖较新的编辑或测试。
- 修改或重新导入连接配置会清空旧工具列表，需重新发现工具后才重新发布。
  缺少版本证明的旧持久化 API 不允许用于已绑定 live catalog publisher 的生产服务。

## 架构约束保持不变

目录刷新只处理 MCP 配置和工具数据，不载入 Engine 代码。
官方仍只有 Nomi 与 Coding 两个 Engine；社区 Engine 仍需源码/依赖接入、重新构建后注册。
Engine 选择属于 Agent 工作台的修订配置，不增加聊天页面切换入口。

当前 Coding build 后缀为 `host2-coding-loop15`，Nomi 为 `host3`。
相关目录发布/服务/仓库源码已纳入 build digest；不重写旧绑定、不自动回退。

## 明确未完成

- 默认 Nomi 对新 MCP owner 的任务、取消和清理接入。
- 一个 Snapshot 中多个 MCP 服务器的 typed resource cardinality；stdio、legacy SSE、
  MCP resources 和服务器主动请求生命周期。
- 跨进程重启的外部副作用收据、人工解除隔离、安全 checkpoint continuation。
- MiniApp、非 FunctionTool Plugin 生命周期、非制品 Skill/二进制资源等其他生态路径。
- 完成报告对用户需求覆盖的语义判定；现有实现只检查计划项、观察来源和时效。
- 真实消费者、Provider、跨平台及以上新增代码的验证（按用户要求本次不执行）。

后续不能以“端口/目录已接通”替代“全部生态能力完成”或“已验证能运行”。
