# MCP 资源模板与两个官方 Engine（源码实现，未验证）

本地分支仍为 `rf/agent-capability-platform-v2`，未 commit/push。
当前 Coding `host2-coding-loop46`、Nomi `host29`。遵照用户要求，没有运行构建、
测试、E2E、服务请求或数据库迁移。源码阅读和局部格式化不构成运行验证。

## 设计与 Codex 参考

继续参考本地 `multi/codex/codex-rs/core/src/tools/handlers/mcp_resource/` 中的
`list_mcp_resource_templates.rs`、`read_mcp_resource.rs`：Engine 提供上下文操作，
Session 检查服务器权限，MCP owner 负责协议。没有复制其产品 Session、凭据管理或
运行时进程，也没有引入 Codex 依赖或新的外部服务。

Nomifun 保持自身冻结 Agent binding 和平台效果归属：两个 Engine 使用相同资源
协议，但各自决定发现、激活与模型循环。社区编译期集成的 Engine 也可使用扩展后的
`EngineResourceQuery`；没有打包后挂载或会话内 Engine 切换。

## 实现入口

- Coding 增加 `list_mcp_resource_templates`，加入控制工具名称保留及独立调用批次。
- canonical Nomi 增加 `mcp_resource_templates`，使用原有 ToolSearch、冻结身份和
  EngineEffectScope；不重新开启原生 MCP bootstrap。
- 两者的 read 工具接受互斥的两种参数：`uri`，或者 `uri_template + variables`。
  后者的 variables 必须是字符串映射，可以为空；不能同时传 uri，也不能给 list
  传读取参数。显式 null、列表和对象变量不被接受。
- 共享平台 owner 调用 `resources/templates/list`，完整读取至多 32 页、256 项、
  1 MiB 的目录；拒绝重复模板、循环 cursor 和超界元数据。模板列表不要求服务器
  同时实现 `resources/list`，直接 URI 读取也不依赖模板方法。
- 模板读取在同一协议 Session 中重新取得模板目录，要求模板逐字匹配，再展开 URI
  调用 `resources/read`；返回 contents 的 URI 必须与展开结果完全相同。

示例：模板 `repo://{owner}/{project}/files{/path}{?revision}` 可以使用字符串映射，
其中 path 中的斜杠会百分号编码。只有服务器提供的 `{+path}` 才保留其保留字符。
展开后的 URI 只发给已授权服务器，不由客户端当作 URL 或本地路径打开。

## 标量展开与边界

平台自有的纯函数实现 RFC 6570 标量 profile：普通、保留字符、fragment、label、
path、path-parameter、query 和 query-continuation 操作符，逗号变量序列、标量 explode
与 Unicode 字符前缀。变量名保留精确拼写，包括合法百分号编码；不做宽松别名匹配。
未提供的变量省略，额外变量拒绝，空字符串与不存在区分。

模板/展开 URI 至多 4096 字节、变量及表达式至多 64 个、变量名至多 256 字节，
变量值合计至多 4096 字节。前缀截断要求未百分号编码、且不含 `%` 的字符串输入，
避免截断已有编码序列；列表/对象复合展开不在本切片范围。模板目录标记
`scalar_expansion_supported` 并列出变量名，不支持的语法不被当作任意 URI 通配授权。
该标记表示语法 profile；具体变量、展开绝对 URI 和实际读取仍可能被拒绝。

输入展开/页参数在永久凭据之前做本地检查。真正远端调用继续使用唯一服务器的
`mcp.resource`、connect/read policy、启用状态与冻结配置；不需要也不新增 invoke。
也补齐了两个入口的 active state 与 compiled Snapshot 一致性检查。

## 效果、分页与恢复

模板操作复用已有保留任务、64 次/回合永久资源凭据限制、初始化前写凭据、独立清理
预算和未知结果隔离。目录与内容均为不可信上下文。续页携带相同读取参数及前页摘要，
每次重新观察；服务数据变化会拒绝拼接。无新增自动重试、缓存或旧请求重放。

新增源码进入两个官方 Engine build digest；旧 exact build 的 Session 不静默升级。
没有改变 Agent 授权对象，也没有为 ResourceProvider 虚构 canonical Tool action。

## 仍待完成

资源二进制、复合模板变量、订阅、长期连接与服务器主动 sampling/roots/elicitation
授权未实现。Git 网络凭据、跨 Session 工作区协调、未知效果人工处理和其他生态
生命周期仍在剩余清单。所有新增代码未编译或执行，不能宣称整体 CAR 已完成。
