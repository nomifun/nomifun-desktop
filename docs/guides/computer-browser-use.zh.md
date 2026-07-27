# Computer Use 与 Browser Use（计算机控制与浏览器自动化）

NomiFun agent 内置/接入两项可选的系统级能力：

- **Computer**（computer use，进程内 Rust）：截屏、鼠标键盘合成输入、窗口枚举/聚焦——让 agent 看到并操作本机桌面。crate：`nomi-computer`（xcap + enigo）。
- **Browser**（browser use，主进程统一托管）：应用只创建一个 `BrowserSessionHub`，由它管理 Chromium Host、Browser Lane、身份、资源队列和清理。`nomi-browser-engine` 提供 CDP 驱动，`nomi-browser` 提供 Lane-aware 工具适配；Native Nomi、Gateway、ACP/Codex、远程 Agent 和并行 AgentExecution attempt 都接入同一个 Hub。

> 注：早期的外接 `@playwright/mcp` sidecar，以及 Native/Gateway/ACP 各自持有私有 `BrowserTool` 或 Chromium 的路径均已移除。`mcp-browser-stdio` 现在是带作用域能力的 Hub 代理，不创建浏览器或 profile。
>
> 当前文档只描述已落地路径：桌面端的系统设置开关、进程内
> browser/computer 工具，以及对应的 build feature 门控。

两者都是高权限能力。当前桌面产品构建在对应 feature 存在时默认开启，
用户可在系统设置中关闭；无头 Web/服务器构建则不承诺桌面控制或托管
浏览器能力。

## 启用与关闭方式

### 1. 桌面端系统设置（推荐）

桌面应用在系统设置中提供两个页面：

- **Browser Use**（`/settings/browser-use`）
- **Computer Use**（`/settings/computer-use`）

当前桌面构建默认把两个能力开关设为开启；关闭任一开关会持久化到用户偏好，
后续新会话不会获得对应能力。Browser 设置还提供：

- **浏览器来源**：系统 Chrome/Edge 可执行文件或 managed source；
- **呈现方式**：普通 Primary Agent 任务以 Chromium `--headless=new` 静默运行；仅用户在 Browser 管理页显式“前台打开”时会创建 headful 替代 Host；
- **资源策略**：Automatic、Resource saving、High concurrency；
- 高级资源上限（仅在需要诊断或精细调优时修改）。

右侧边栏的 **Browser** 页面（`/browser`）只展示 running/queued Lane 的状态、
容量、队列、身份、owner 与生命周期，并在权限允许时关闭单个 Lane、某个
conversation 的 Lane 或全部 Lane。对 running Primary Lane，它还提供“前台打开”，
将该 Lane 切换到 headful 替代 Host。该页面仍不嵌入页面，也不提供页面输入、
tab 控制、用户接管或地址导航。

### 2. 会话级

创建会话时在 `extra` 中传开关（camelCase 与 snake_case 均可）：

```json
{ "computerUse": true, "browserUse": true }
```

### 3. 宿主级环境变量

```bash
NOMIFUN_COMPUTER_USE=1   # 所有 nomi 会话默认启用 Computer
NOMIFUN_BROWSER_USE=1    # 所有 nomi 会话默认允许接入主进程 BrowserSessionHub
```

### 4. nomi CLI / 配置文件

`~/.nomi/config.toml` 或项目 `.nomi/config.toml`：

```toml
[tools]
max_recent_images = 3        # 历史中保留的工具结果图片总数（旧图自动剥离省 token）

[tools.computer]
enabled = true
max_screenshot_edge = 1568   # 截图长边像素上限

[tools.browser]
enabled = true
allowed_origins = []         # 可选 origin 白名单；空=全放行，仅纵深防御
# 注：普通 Primary Agent 任务使用 Chromium --headless=new；仅用户在 Browser
# 管理页显式“前台打开”时，才创建 headful 替代 Host。
# browser_path / idle_timeout_secs / 私有 headless ownership 均为旧兼容字段，
# 不能绕过 BrowserSessionHub 的身份、容量或生命周期策略。
```

启用 Browser 后，Hub 首次需要 Host 时按需解析系统 Chrome/Edge 可执行文件或
managed source，无需 Node/npm/Playwright。来源只决定二进制；进程始终由 NomiFun
托管并应用隔离 profile，绝不读取或共用用户真实 Chrome/Edge profile；同一个
user-data directory 也不会被两个存活 Chromium 同时打开。

## 构建形态（feature 门控）

| 宿主 | Computer（进程内） | Browser（进程内 native CDP） |
|---|---|---|
| 桌面应用（nomifun-desktop） | ✅ 默认编译（`computer-use` feature） | ✅（`browser-use` feature；首次自动获取 Chrome） |
| nomi CLI | ✅ 当前 `nomi-cli` manifest 启用 | ❌ 当前 `nomi-cli` manifest 未启用 |
| Web/服务器（nomifun-web、Docker） | ❌ 不编译（无显示器；xcap/enigo 不进二进制） | ❌ 当前 headless web host 未启用 `browser-use` feature |

`computer-use` feature 链：`apps/desktop` → `nomifun-app` → `nomifun-ai-agent` → `nomi-agent` → `nomi-computer`。Web 构建若配置中误开 computer，仅记录 warning，不报错。Browser 由 `browser-use` feature 门控（`nomifun-browser-platform` / `nomi-browser` / `nomi-browser-engine`）。

## Browser Lane、身份与并发

- 一个 Agent runtime 的默认 Lane 在其生命周期内保持稳定；同一 AgentExecution 的并行 attempt
  使用不同 LaneKey，不会因为 companion 或 conversation 相同而被合并。
- 同一 Lane 内的导航、观察和动作严格串行；不同 Lane 可以并行，target、frame、
  ref、tab、download 和 cancellation 状态互不串线。
- 普通交互式浏览默认使用 **Primary shared live identity**。多个 Primary Lane
  在普通 Agent 任务中使用 Chromium `--headless=new`，不创建操作系统浏览器窗口；
  显式“前台打开”会安全地用应用托管 profile 创建 headful 替代 Host。它们共享
  cookies、站点存储和
  其他 profile-backed 身份状态，但不共享活动 target、frame、ref、操作 gate
  或下载归属。
- 公开读取默认使用 **Anonymous crawl**，不携带 Primary cookies/站点存储。
  有界只读认证扩展可使用 **Authenticated replica**，副本变更不会自动写回
  Primary；可能修改登录或持久账户状态的动作必须回到 Primary。
- 切换账户、退出测试、不可信浏览或用户显式隔离使用 **Isolated identity**。
  crawl、replica 与 isolated Host 均可 headless 运行。

容量有明确上限。超过安全预算的 Lane 会进入可取消队列，返回
`browser_capacity_queued` 或 `system_memory_pressure`，并携带队列位置、原因、
建议并发和重试延迟。此时应等待、复用已有 Lane、降低并发，或让批量公开读取使用
`browser_crawl_many`；不要尝试额外启动浏览器绕过限制。

## Browser 工具与生命周期

现有导航、观察、动作、截图、tab、下载和 debug action 都可传可选 `lane_id`；
省略时使用调用方默认 Lane。平台管理 action 包括：

- `browser_open`：幂等打开默认或命名 Lane；
- `browser_fork`：创建扩展 Lane；
- `browser_list` / `browser_status`：查看 Lane、身份、容量、队列和恢复状态；
- `browser_close` / `browser_close_all`：关闭当前 owner 的一个或全部 Lane；
- `browser_crawl_many`：有界并发处理一组 URL，并负责 Lane 复用、排序、取消和清理。

关闭 Lane 只会让相关浏览器调用收到类型化错误，不会关闭 conversation 或
AgentExecution。attempt 完成/取消、runtime 终止、conversation 删除、远程连接断开、
capability 过期和应用退出都会撤销 owner lease 并触发权威清理。Native Agent turn
正常完成或取消时也会关闭该 owner 的 Lane；完成 target 清理后，若这是 Host 的
最后一个 Lane，Host 会立即退出，不等待 idle expiry 或周期 sweep。

### 显式前台打开 running Primary Lane

需要查看真实受管浏览器时，进入 `/browser`，选择身份为 Primary、状态为
`running` 的 Lane，再点击“前台打开”。普通 Host 是真正的 headless 进程；Hub 会
安全关闭它、递增 browser epoch，并用同一应用托管 profile 启动 headful 替代
Host。系统会尽力恢复 Lane 的活动 URL，但旧 target/frame/ref 已失效；应刷新库存
并 fresh observe 后再继续。该操作不会改变 Lane 所有权，也不会恢复内嵌 preview、
用户接管或页面输入表面。queued、failed 和非 Primary Lane 不能前台打开。

对应的认证管理接口为 `POST /api/browser/lanes/{id}/foreground`；改变状态的请求
继续使用现有 CSRF 防护。它不是 Agent 可调用的 Browser action。创建 Primary
登录 Lane 也不会绕过该规则或自动打开窗口；前台打开仍须由用户在 Browser 页面
显式触发。

## macOS 权限

Computer 能力首次使用需在「系统设置 → 隐私与安全性」中授权宿主应用：

- **辅助功能（Accessibility）**：鼠标键盘合成输入需要此项（未来 a11y 树读取/动作亦只需此项）。
- **屏幕录制（Screen Recording）**：截图需要此项（截图全黑或失败时检查）。

当前为反应式诊断：权限缺失时，工具结果会给出授权指引。

## 工具语义与审批

- Computer 为单工具 + `action` 参数形态。
- 只读 action（`screenshot`、`cursor_position`、`list_windows`、`wait`）按 **Info** 类审批——AutoEdit/Default 模式自动放行；操作类 action（点击、输入、滚动、拖拽、`focus_window` 等）按 **Exec** 类——Default 模式需用户确认。
- Plan mode 下 Computer 整工具不可见（只读规划阶段不操作桌面）。
- Browser 工具按动作语义派生审批类别：只读观察（如 `observe`/快照）→ Info，写操作（导航、点击、输入等）→ Exec；Browser 管理页不能触发这些页面动作。egress、approval、secret、下载、full-power 与不可逆动作护栏继续生效，模型输入不能伪造可信批准。
- 推荐工作流：`screenshot` 观察 → 操作 → 再次 `screenshot` 验证。

## 截图与 token 治理

- 截图自动降采样到长边 ≤ `max_screenshot_edge`（默认 1568px，Anthropic 视觉推荐区间），最终 PNG 还受 5 MiB 上限约束；高熵画面会再次确定性降采样，坐标几何始终以实际发给模型的图片为准，并自动映射回真实屏幕（含 Retina 缩放）。
- 历史消息中只保留最近 `max_recent_images`（默认 3）张工具结果图片，并同时受每次请求最多 20 张和累计编码体积预算约束；超出的附件会被剥离，但保留文本和省略说明。提供商调用失败后也会清除待重放的历史截图，再持久化可恢复会话，避免会话文件、请求 token 和网关负载持续膨胀。
- OpenAI 协议的 tool 消息不支持图片：图片以紧随其后的 user 消息（`image_url` data URI）传递，并标注来源 call id。Anthropic/Bedrock/Vertex 走原生 `tool_result` 图片块。
- 外接 MCP 工具回传的图片同样经 `McpToolProxy` 映射进图片管道（单图 ≤ 5 MiB 上限）。

## 替代路径：其他外接 MCP

除内置 Computer 与 native Browser 外，仍可外接任意社区 MCP server（在 MCP 设置中添加），与上述能力互不冲突（工具名不同）。
