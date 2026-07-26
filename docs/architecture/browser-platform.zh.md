# 浏览器平台架构

本文说明 NomiFun 内置浏览器的生产架构、信任边界、并发模型和运维接口。浏览器平台的执行单位是 Browser Lane，而不是 Agent 会话或单个 Chromium 进程。

## 核心模型

应用主进程只构造一个 `BrowserSessionHub`。Hub 是以下状态的唯一权威：

- 受管 Chromium Host 及其显式关闭；
- Lane 的所有权、生命周期、排队和取消；
- Primary、Anonymous、Authenticated Replica 与 Isolated 身份路由；
- owner lease、viewer token 与 control lease；
- 全局资源策略、动态遥测和周期清理；
- Browser 页面使用的库存快照和用户级实时事件。

一个 `BrowserHost` 对应一个受管 Chromium 进程树和一条 CDP 连接。一个 Host 可以承载多个 Lane。每个 Lane 有独立的 target/tab 集合、活动 target/frame、ref generation、操作 gate、取消令牌、下载归属和用户控制状态。

同一 Lane 的语义操作严格串行；不同 Lane 可以并发。全局操作 permit 只限制资源消耗，不承担正确性串行化。

## 身份模式

| 模式 | 用途 | 存储语义 |
| --- | --- | --- |
| `primary` | 普通交互式浏览、登录和账户操作 | 使用应用管理的稳定 profile；多个 Primary Lane 实时共享身份状态 |
| `anonymous` | 公开网页和知识源抓取 | 使用临时 profile，不读取 Primary cookies 或站点存储 |
| `authenticated_replica` | 有界的只读认证抓取扩展 | 使用带 generation 的时间点副本；不会自动回写 Primary |
| `isolated` | 切换账户、退出测试、不可信浏览或显式隔离 | 使用独立临时身份 |

Replica 上声明为可能修改身份或持久账户状态的操作会失败为
`needs_primary_identity`。可信动作分类在进程内完成；模型输入不能降低该分类，也不能伪造“已由用户确认”。

Primary profile 属于 NomiFun，不使用用户个人 Chrome 或 Edge profile。API、Agent 工具和 renderer 都不会收到 profile 路径、原始 CDP endpoint 或调试端口。

## 调用入口

所有生产入口最终使用同一个 Hub：

- Native Nomi 在 runtime 创建时获得主进程签发的 `BrowserLaneClient`；
- Gateway 从认证的 conversation/runtime 上下文解析 `CallerIdentity`，再转发到 Hub；
- ACP browser stdio 只持有短期、可续期且限定 audience/operation 的 loopback capability；
- 知识 URL 渲染器使用固定的 Anonymous Lane；
- Browser 登录接口使用受管 Primary Lane；
- HTTP 管理接口和嵌入式 Viewer 直接调用 Hub 的用户级管理边界。

模型只能选择长度受限的 Lane 名称。`user_id`、conversation、runtime instance、attempt、owner lease、允许的操作和有效期都来自可信的应用上下文。

runtime、attempt、远程连接或 capability 结束时必须撤销 owner lease，并关闭该 owner 的 Lane。应用退出时先等待 Hub 显式关闭所有 Lane 和 Host，再完成进程退出。

## Agent 工具

原有 Browser 动作仍可省略 `lane_id`，此时使用调用方的 `default` Lane。平台另外提供：

- `browser_open`：幂等打开默认或命名 Lane；
- `browser_fork`：创建扩展 Lane；
- `browser_list` / `browser_status`：读取 Lane、身份、容量和恢复状态；
- `browser_close` / `browser_close_all`：关闭当前 owner 的 Lane；
- `browser_crawl_many`：有界并发地抓取一组 URL，并负责 Lane 的创建、复用、取消和清理。

当容量不足时，调用返回结构化 `queued` 状态，而不是返回一个被隐藏全局锁阻塞的“就绪”句柄。队列元数据包含位置、建议并发、owner/全局活动和排队数量、重试延迟及稳定原因码。

## 资源与生命周期

Automatic 策略从系统总内存和逻辑 CPU 推导安全上限。运行期间定期采样可用内存、CPU 压力和受管 Chromium RSS，并动态计算 `normal`、`pressured` 或 `critical`。

调度遵循以下约束：

- owner 的首个 Lane 权重为 4，扩展 Lane 权重为 1；
- 同优先级在 owner 之间轮转；
- 排队年龄提升有效优先级；
- 取消立即移除队列项；
- 资源平衡不抢占正在执行的操作；
- 压力回收优先处理空闲扩展或 Crawl Lane，并保护 owner 唯一活动 Lane。

正常 idle expiry 为 10 分钟；压力状态下可回收 Lane 的 idle expiry 为 2 分钟。周期 sweep 同时处理过期 owner/control/viewer 凭据和 Host warm timer。

## 嵌入式 Viewer

`embedded` 是新安装的默认显示模式。Viewer 展示 Agent 正在操作的同一 target，不创建第二个页面会话。

安全边界如下：

- Viewer token 短期、单次使用，并绑定用户和 Lane；
- WebSocket 校验认证身份、Origin、Lane 路径和 token；
- 二进制消息只承载 JPEG，JSON 消息只承载固定的元数据和输入词汇；
- 帧队列只保留最新帧；
- 用户首次输入取得当前 Lane 的 control lease；
- control lease 每 10 秒续期，30 秒未续期自动归还；
- 输入按实际显示内容区域和浏览器 viewport 归一化；
- control lease 不改变网络、审批、下载或 secret policy。

Screencast 不可用时 Viewer 可以退化为有界截图轮询。流失败不会阻止 Lane 管理和关闭。

## HTTP 管理接口

认证 API：

- `GET /api/browser/overview`
- `GET /api/browser/lanes`
- `POST /api/browser/lanes/{id}/close`
- `POST /api/browser/conversations/{id}/close`
- `POST /api/browser/close-all`
- `POST /api/browser/lanes/{id}/return-control`
- `POST /api/browser/lanes/{id}/viewer-token`
- `GET /api/browser/resource-policy`
- `PUT /api/browser/resource-policy`
- `GET /api/browser/lanes/{id}/view`（WebSocket）

所有用户可见库存和实时事件都按认证用户过滤。改变状态的 HTTP 请求继续使用应用现有的 CSRF 防护。

## 设置迁移

显示模式键为 `agent.browserUse.displayMode`，可取：

- `embedded`
- `external`
- `headless`

读取优先级为：

1. 已存在的 `displayMode`；
2. 旧 `silent === false` 映射为 `external`；
3. 旧 `silent === true` 映射为 `headless`；
4. 两者都不存在时写入并使用 `embedded`。

迁移后不再写入旧的 `agent.browserUse.silent`。

显示模式在应用组合根创建 `BrowserSessionHub` 和 Browser Host 工厂时读取。运行中修改会持久化，但不会把现有 Host 在 headful/headless 之间热切换；重启 Nomi 后，新模式对所有后续 Host 生效。

## 稳定错误

浏览器错误至少包含稳定机器码、安全说明、是否可重试和建议的下一步；适用时还带 Lane、容量、队列或恢复元数据。主要错误包括：

- `browser_capacity_queued`
- `system_memory_pressure`
- `lane_closed_by_user`
- `owner_lease_expired`
- `stale_browser_epoch`
- `stale_lane_ref`
- `target_crashed`
- `browser_restarted`
- `identity_replica_stale`
- `needs_primary_identity`
- `viewer_stream_failed`

错误文本不得包含 cookie、站点存储值、CDP endpoint、调试端口或 profile 路径。
