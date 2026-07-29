# 浏览器平台架构

本文说明 NomiFun 内置浏览器的生产架构、信任边界、并发模型和运维接口。浏览器平台的执行单位是 Browser Lane，而不是 Agent 会话或单个 Chromium 进程。`/browser` 仍是状态与生命周期管理页面；它可以管理安装级 Primary 显示默认值，也可以改变已有 running Primary Lane 所属 Host 的当前可见性，但不是页面渲染或控制表面。

> **2026-07-27 取代性说明：**新安装的全局默认值为 `headless`，普通 Primary 工作因此以 Chromium `--headless=new` 运行，不创建操作系统浏览器窗口。安装 owner 可实时把该默认值改为 `external`；对 Lane 显式“前台打开”只是一次性改变当前 Host 的可见性，不会改写默认值。两条路径都不会恢复早期的内嵌 preview、Viewer、用户接管或页面输入能力。

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
| `primary` | 普通交互式浏览、登录和账户操作 | 使用应用管理的稳定隔离 profile；全局默认值 `headless` 以 `--headless=new` 启动 Chromium，安装 owner 也可显式选择实时生效并持久化的 `external` 默认值；用户还可只对当前 Primary Host 执行前台或后台切换而不改变该默认值；多个 Primary Lane 实时共享身份状态 |
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
- Browser 登录接口使用受管 Primary Lane，并遵循当前可信 Primary 显示默认值；用户仍可在 Browser 管理页执行一次性前台或后台切换；
- HTTP 管理接口只调用 Hub 的用户级库存和生命周期边界（包括切换已有 running Primary Lane 的当前可见性），以及安装 owner 的显示与资源策略控制。

模型只能选择长度受限的 Lane 名称。`user_id`、conversation、runtime instance、attempt、owner lease、允许的操作和有效期都来自可信的应用上下文。

runtime、attempt、远程连接或 capability 结束时必须撤销 owner lease，并 drain 该 owner 的 Lane。Native Agent turn 正常完成或取消时使用同一 owner 级 drain；session/conversation 关闭会 drain 对应 Lane、等待 target cleanup，并关闭因此变空的 Host。应用退出与安装级“全局关闭所有浏览器”会先阻止新 open，再 drain 所有 Lane、待处理 cleanup/retirement 和受管 Host，之后才能报告完成。正常清理不依赖 idle expiry、周期 sweep 或 warm timer。

## Agent 工具

原有 Browser 动作仍可省略 `lane_id`，此时使用调用方的 `default` Lane。平台另外提供：

- `browser_open`：幂等打开默认或命名 Lane；
- `browser_fork`：创建扩展 Lane；
- `browser_list` / `browser_status`：读取 Lane、身份、容量和恢复状态；
- `browser_close` / `browser_close_all`：关闭当前 owner 的 Lane；
- `browser_crawl_many`：有界并发地抓取一组 URL，并负责 Lane 的创建、复用、取消和清理。

容量不足时，`browser_open` 可以成功返回结构化 `queued` Lane，而不是返回一个被隐藏全局锁阻塞的“就绪”句柄。队列元数据包含位置、建议并发、owner/全局活动和排队数量、重试延迟及稳定原因码。在 Lane 变为 `running` 之前，页面 `wait` 在内的所有普通页面 action 都返回显式工具错误：`ok: false`、`dispatched: false`、稳定码 `browser_capacity_queued` 和重试元数据；`browser_status` 仍是成功的轮询操作。

## 资源与生命周期

Automatic 策略从系统总内存和逻辑 CPU 推导安全上限。运行期间定期采样可用内存、CPU 压力和受管 Chromium RSS，并动态计算 `normal`、`pressured` 或 `critical`。

调度遵循以下约束：

- owner 的首个 Lane 权重为 4，扩展 Lane 权重为 1；
- 当全局还没有活动 Lane、系统内存低于完整预留但仍高于临界底线时，只允许一个基础可用 Lane 使用临界底线启动；第二个首 Lane 和所有扩展 Lane 仍必须满足完整内存预留，进入 `critical` 后一律不启动；
- 同优先级在 owner 之间轮转；
- 排队年龄提升有效优先级；
- 取消立即移除队列项；
- 资源平衡不抢占正在执行的操作；
- 压力回收优先处理空闲扩展或 Crawl Lane，并保护 owner 唯一活动 Lane。

正常 idle expiry 为 10 分钟；压力状态下可回收 Lane 的 idle expiry 为 2 分钟。周期 sweep 只是处理过期 owner 凭据与遗留 Lane 状态的恢复兜底，不是正常清理路径；显式关闭 Lane 或 Agent turn 结束后，最后一个 Host 会立即退出。

对应用托管的稳定 profile，完成可证明的正常 Host 关闭后，会删除属于该次启动且已完成的 ownership marker 和 `DevToolsActivePort`，同时保留 cookie、站点存储及其他稳定 profile 数据。清理保持 fail-closed：若无法重新验证精确进程树已经退出或 marker 确属本次启动，则保留这些运行时文件供权威恢复，且不能把该操作报告为干净关闭。

## Browser 管理页面边界

`/browser` 是状态与生命周期管理页面，不是浏览器执行表面。它只用于查看
Lane/Host 状态、容量与队列、身份和 owner 信息，并关闭 Lane、conversation
下的 Lane 或安装范围内的 Lane。安装 owner 还可实时修改全局 Primary 显示默认值
和资源策略。对 running Primary Lane，其 owner 可以显式执行“前台打开”或“转到
后台”，两者都不会改变全局默认值。

该页面不嵌入图像流，不建立专用 Viewer WebSocket，也不提供用户接管、页面
输入、tab 操作或地址导航入口。在全局 `headless` 默认值下，普通 Agent Browser
Use 以 `--headless=new` 启动 Primary，不创建隐藏或最小化窗口。“前台打开”是
当前 Primary Host 的一次性显示请求，不会改变持久默认值；“转到后台”是其对称
操作。只要 live Primary Host 需要在 headless 与 headful 之间切换——无论源于
前台、后台还是默认策略变更——Hub 都会以新 browser epoch 替换整个共享 Primary
Host，并重新绑定该 Host 上所有 live Primary Lane。Hub 会尽力恢复各 Lane 的活动
URL，但旧 target/frame/ref 必然失效；调用方必须刷新库存并 fresh observe，不能
复用旧 ref。这些操作不授予页面输入或接管能力。关闭操作仍由 Hub 执行，不会关闭
conversation 或 AgentExecution；若关闭的是所属 Host 的最后一个 Lane，target
cleanup 后 Host 会立即退出。

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
- `POST /api/browser/lanes/{id}/background`
- `POST /api/browser/lanes/{id}/close`
- `POST /api/browser/conversations/{id}/close`
- `POST /api/browser/close-all`
- `GET /api/browser/display-mode`
- `PUT /api/browser/display-mode`
- `GET /api/browser/resource-policy`
- `PUT /api/browser/resource-policy`

`POST /api/browser/lanes/{id}/foreground` 按认证用户过滤，仅接受该用户拥有且身份为
Primary、生命周期为 `running` 的 Lane。如果该 Lane 位于普通 headless Primary
Host，Hub 会用同一应用托管 profile 安全替换整个共享 Host、递增 epoch，并重新
绑定所有 live Primary Lane。Hub 只尽力恢复活动 URL，客户端必须刷新库存并 fresh
observe。它只是当前 Host 的一次性请求，不改变全局默认值、不转移页面控制权，
也不是模型可调用的 Browser action。`POST /api/browser/lanes/{id}/background`
执行对称的 headless 切换，同样不改变全局默认值。

`GET` 与 `PUT /api/browser/display-mode` 是安装 owner 用于管理持久 `headless` 或
`external` Primary 默认值的接口。确认成功的 `PUT` 会立即应用到 live Hub；若
running Primary Host 必须改变模式，则新 epoch Host 替换及所有 Lane 重绑完成后
才返回成功，后续 Primary 启动也使用持久化后的选择。

仅安装 owner 可用的 Primary 登录兼容流程还提供
`POST /api/browser/login/open`、`POST /api/browser/login/close` 和
`GET /api/browser/login/status`。这些接口创建或复用普通 Hub Primary Lane，并
遵循而不覆盖当前全局显示默认值：`headless` 下须由用户在 `/browser` 对 running
Lane 显式“前台打开”才会显示，`external` 下 Primary Host 已按策略可见。它不会
创建第二浏览器、内嵌页面或授予 Browser 管理页页面控制权。

所有用户可见库存和实时事件都按认证用户过滤。改变状态的 HTTP 请求继续使用应用现有的 CSRF 防护。

安装级“全局关闭所有浏览器”只有在结果同时包含
`remaining_lane_count = 0`、`remaining_cleanup_count = 0` 和
`remaining_managed_host_count = 0` 时才确认成功。仅有成功 HTTP 状态或已 detach
的 Lane 数量并不代表 drain 完成；任一 remaining 计数缺失或非零都必须作为未确认
或未完成清理展示。

## 设置迁移

产品提供两个可信的应用级默认可见策略：`headless`（新安装默认，普通 Agent
Browser Use 以 `--headless=new` 静默运行 Primary）与 `external`（安装 owner
显式选择默认前台可见，Primary Host 以真实窗口启动）。安装 owner 可从 Browser
Use 设置页或 `/browser` 修改；确认成功的更改会实时应用到 live Hub 并持久化，
无需重启应用。普通 Agent action、lane 名称或工具 JSON 无权覆盖。无论选择哪一项，
Anonymous、Authenticated Replica 与 Isolated Host 都继续按 Hub 策略 headless
运行；对 running Primary 执行一次性前台/后台操作也不会改写该默认值。

迁移契约：

- 新安装持久化 `agent.browserUse.displayMode = headless` 和显示策略版本 `2`；
- 只有带版本 `2` 的显式 `headless` 或 `external` 用户选择会被保留；
- 所有未带版本的历史值（包括旧 `silent=false` 推导出的 `external`），以及版本
  `2` 下缺失或无效的模式，都会一次性修复为 `headless`；
- 旧 `silent` 键不再写入；偏好存储读取失败时使用 `headless` 兜底且不写迁移状态。

该迁移不允许普通任务绕过用户策略打开窗口；`embedded` 不会作为可选项回归，
也不恢复截图预览、Viewer token、接管或输入桥接。

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
