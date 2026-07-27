# 浏览器平台架构

本文说明 NomiFun 内置浏览器的生产架构、信任边界、并发模型和运维接口。浏览器平台的执行单位是 Browser Lane，而不是 Agent 会话或单个 Chromium 进程。`/browser` 仍是状态与生命周期管理页面；它可以显式前台打开已有的 running Primary Lane，但不是页面渲染或控制表面。

> **2026-07-27 取代性说明：**普通 Agent Browser Use 默认以 Chromium `--headless=new` 运行 Primary，不创建操作系统浏览器窗口。只有用户在 Browser 管理页显式“前台打开”或进入显式登录流程时，Hub 才会安全关闭旧 headless Host，并用应用托管 profile 创建 headful 替代 Host；这不会恢复早期的内嵌 preview、Viewer、用户接管或页面输入能力。

## 核心模型

应用主进程只构造一个 `BrowserSessionHub`。Hub 是以下状态的唯一权威：

- 受管 Chromium Host 及其显式关闭；
- Lane 的所有权、生命周期、排队和取消；
- Primary、Anonymous、Authenticated Replica 与 Isolated 身份路由；
- owner lease 与可信调用方能力；
- 全局资源策略、动态遥测和周期清理；
- Browser 管理页面使用的状态、容量、身份和生命周期库存快照，以及用户级实时事件。

一个 `BrowserHost` 对应一个受管 Chromium 进程树和一条 CDP 连接。一个 Host 可以承载多个 Lane。每个 Lane 有独立的 target/tab 集合、活动 target/frame、ref generation、操作 gate、取消令牌和下载归属。

同一 Lane 的语义操作严格串行；不同 Lane 可以并发。全局操作 permit 只限制资源消耗，不承担正确性串行化。

## 身份模式

| 模式 | 用途 | 存储语义 |
| --- | --- | --- |
| `primary` | 普通交互式浏览、登录和账户操作 | 使用应用管理的稳定隔离 profile；普通 Agent Browser Use 以 `--headless=new` 启动受管 Host，不创建窗口；仅用户在 Browser 管理页显式“前台打开”时会安全关闭 headless Host，并用同一应用托管 profile 创建 headful 替代 Host；多个 Primary Lane 实时共享身份状态 |
| `anonymous` | 公开网页和知识源抓取 | 使用临时隔离 profile，不读取 Primary cookies 或站点存储；可 headless |
| `authenticated_replica` | 有界的只读认证抓取扩展 | 使用带 generation 的时间点隔离副本，不会自动回写 Primary；可 headless |
| `isolated` | 切换账户、退出测试、不可信浏览或显式隔离 | 使用独立临时身份；可 headless |

Replica 上声明为可能修改身份或持久账户状态的操作会失败为
`needs_primary_identity`。可信动作分类在进程内完成；模型输入不能降低该分类，也不能伪造“已由用户确认”。

Chromium 可执行文件可来自系统 Chrome/Edge 或 managed source；来源只决定使用哪个二进制。无论来源如何，NomiFun 都会启动受管进程并应用自己的隔离 profile，绝不打开用户个人 Chrome/Edge profile。API、Agent 工具和 renderer 都不会收到 profile 路径、原始 CDP endpoint 或调试端口。

## 调用入口

所有生产入口最终使用同一个 Hub：

- Native Nomi 在 runtime 创建时获得主进程签发的 `BrowserLaneClient`；
- Gateway 从认证的 conversation/runtime 上下文解析 `CallerIdentity`，再转发到 Hub；
- ACP browser stdio 只持有短期、可续期且限定 audience/operation 的 loopback capability；
- 知识 URL 渲染器使用固定的 Anonymous Lane；
- Browser 登录接口使用受管 Primary Lane，除非用户在 Browser 管理页显式“前台打开”，否则仍保持 headless；
- HTTP 管理接口只调用 Hub 的用户级库存、资源策略和生命周期边界，其中包括前台打开已有的 running Primary Lane。

模型只能选择长度受限的 Lane 名称。`user_id`、conversation、runtime instance、attempt、owner lease、允许的操作和有效期都来自可信的应用上下文。

runtime、attempt、远程连接或 capability 结束时必须撤销 owner lease，并关闭该 owner 的 Lane。应用退出时先等待 Hub 显式关闭所有 Lane 和 Host，再完成进程退出。
Native Agent turn 正常完成或取消时也会关闭该 turn owner 的 Lane。最后一个 Lane 完成 target 清理后，所属 Host 立即关闭；正常清理不等待 idle expiry、周期 sweep 或 warm timer。

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

正常 idle expiry 为 10 分钟；压力状态下可回收 Lane 的 idle expiry 为 2 分钟。周期 sweep 只是处理过期 owner 凭据与遗留 Lane 状态的恢复兜底，不是正常清理路径；显式关闭 Lane 或 Agent turn 结束后，最后一个 Host 会立即退出。

## Browser 管理页面边界

`/browser` 是状态与生命周期管理页面，不是浏览器执行表面。它只用于查看
Lane/Host 状态、容量与队列、身份和 owner 信息，并关闭 Lane、conversation
下的 Lane 或安装范围内的 Lane。对 running Primary Lane，用户还可以显式执行
“前台打开”。

该页面不嵌入图像流，不建立专用 Viewer WebSocket，也不提供用户接管、页面
输入、tab 操作或地址导航入口。普通 Agent Browser Use 以 `--headless=new` 启动
Primary，不创建隐藏或最小化窗口。“前台打开”会安全关闭旧 headless Host、递增
browser epoch，并用应用托管 profile 创建 headful 替代 Host，再重新绑定 Lane。
Hub 会尽力恢复各 Lane 的活动 URL，但旧 target/frame/ref 必然失效；调用方必须
刷新库存并 fresh observe，不能复用旧 ref。该操作不授予页面输入或接管能力。
关闭操作仍由 Hub 执行，不会关闭 conversation 或 AgentExecution；若关闭的是
所属 Host 的最后一个 Lane，target 清理后 Host 会立即退出。

## Agent 审批与安全边界

Browser 页面没有页面操作权限。Agent 发起的只读观察按 Info 类处理；可能改变
页面、账户或外部状态的操作按 Exec 类处理，并继续经过应用审批、出口、secret、
下载和 full-power 策略。模型 JSON 不能构造可信身份、放宽动作分类或伪造带外
批准；敏感或不可逆操作保持 fail-closed。

## HTTP 管理接口

认证 API：

- `GET /api/browser/overview`
- `GET /api/browser/lanes`
- `POST /api/browser/lanes/{id}/foreground`
- `POST /api/browser/lanes/{id}/close`
- `POST /api/browser/conversations/{id}/close`
- `POST /api/browser/close-all`
- `GET /api/browser/resource-policy`
- `PUT /api/browser/resource-policy`

`POST /api/browser/lanes/{id}/foreground` 按认证用户过滤，仅接受该用户拥有且身份为
Primary、生命周期为 `running` 的 Lane。如果该 Lane 位于普通 headless Primary
Host，Hub 会安全关闭旧 Host，并用同一应用托管 profile 创建 headful 替代 Host。
这个过程会改变 browser epoch，并使旧 target、frame 与 ref 状态失效；Hub 只尽力
恢复 Lane 的活动 URL，客户端必须刷新库存并 fresh observe。该端点不转移页面
控制权，也不是模型可调用的 Browser action。

仅安装 owner 可用的 Primary 登录兼容流程还提供
`POST /api/browser/login/open`、`POST /api/browser/login/close` 和
`GET /api/browser/login/status`。这些接口创建或复用普通 Hub Primary Lane，但不
绕过 headless 默认，也不会自动前台打开 Chromium；用户仍须在 `/browser` 对该
Lane 显式“前台打开”。它不会创建第二浏览器、内嵌页面或授予 Browser 管理页
页面控制权。

所有用户可见库存和实时事件都按认证用户过滤。改变状态的 HTTP 请求继续使用应用现有的 CSRF 防护。

## 设置迁移

产品显示模式固定为 `external`。新安装写入
`agent.browserUse.displayMode = external`；这里的 `external` 表示真实、可前台恢复的
受管窗口只会在显式前台操作后创建，不表示自动弹窗；普通 Agent Browser Use
仍以 `--headless=new` 运行 Primary。历史 `embedded`、
`headless`、无效值以及旧 `agent.browserUse.silent` 都只用于兼容读取，并收敛为
`external`，不再写入旧 `silent` 键。该迁移不允许普通任务打开窗口；Anonymous、
Authenticated Replica 与 Isolated Host 也继续按 Hub 策略 headless 运行，除非另有
可信流程明确要求。

`agent.browserUse.source` 选择系统 Chrome/Edge 优先或 managed source 优先；它不
授权复用个人 profile，也不改变 Hub 的身份、容量、审批或生命周期策略。

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

错误文本不得包含 cookie、站点存储值、CDP endpoint、调试端口或 profile 路径。
