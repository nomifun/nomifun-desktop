# Computer Use 与 Browser Workspace

NomiFun 将桌面范围的 Computer 自动化与会话中的原生浏览器区分开。两者都受 Agent 能力授权约束，网页或模型提示不能自行授予权限。

## 会话浏览器（v2，实施中）

通过会话工作区的“浏览器”按钮打开。旧全局浏览器管理/设置页及兼容重定向已移除。

用户和 Agent 看到的是同一个原生网页。Agent 工作期间锁定用户页面输入；停止 Agent 并等待在途操作完成清理后，用户才能操作。隐藏面板会保留页面和标签，不是接管，也不是重置。

开发前端项目时，先启动开发服务器，再让 Agent 在会话浏览器中打开 localhost 地址，观察页面、执行交互并验证结果。导航、点击、键盘输入和标准 HTML 选择操作使用真实浏览器，交互表面不是截图流。

Windows 原生输入已有真实 smoke 验证。macOS、frame、dialog、文件和打包等完整产品验收仍在进行，请参阅[架构说明](../architecture/browser-platform.zh.md)与[实施记录](../specs/2026-09-13-browser-workspace-v2-progress.zh.md)中的当前限制。

## 可选系统浏览器（Windows，实施中）

在 Agent 工作台“网页”分类选择 `nomi_system_browser`，再从会话标题栏的“系统浏览器”连接正在运行、已登录的 Chrome 144+。
首次使用需在 Chrome 的 `chrome://inspect/#remote-debugging` 中启用并批准连接，然后选择允许本会话使用的标签。
不要求重启 Chrome，不创建替代 Profile，也不导入 Cookie、密码或历史。Chrome 原生许可覆盖所选个人资料；标签限制由 NomiFun 额外执行。

当前支持主文档观察、导航、点击、输入、按键和滚动。页面仍在原 Chrome 窗口中，不是嵌入式截图；没有测试面板或接管模式。
Agent 运行时不能在 NomiFun 更改授权；NomiFun 无法物理锁定 Chrome 地址栏、关闭按钮或用户撤销许可。
断开只释放连接和自动化状态，不关闭用户浏览器或标签。连接请求可以取消，连接结果不确定时只刷新状态，不自动重连或重放操作。

主文档的真实输入已在独立临时 Chrome 中验证；个人登录页面授权实测、完整 iframe/文件/对话框矩阵与发行验收仍未完成。
Edge、macOS 和 Linux 尚未声明支持。这项能力与内嵌浏览器、`nomi_local_websearch` 和厂商搜索相互独立。

## 可选网页搜索

Agent 工作台“网页”分类中的 `nomi_local_websearch` 与模型厂商的 `web.search` 是两项独立能力。本地搜索通过隔离的 Headless 运行时将查询发送给搜索引擎，不使用会话登录态，不要求模型原生搜索，也不授予浏览器自动化权限。

Windows 桌面会识别已安装的 Chrome 120+；没有合格安装时不能启用。发现过程不启动浏览器，实际查询才启动隔离运行时并核对版本。
搜索词发送给 Bing，引擎域名通过 Google Public DNS（HTTPS）解析，不读取会话登录态。Chrome 更新导致旧绑定失效时需重启应用。
Windows 目录接线及公网查询已验证；主应用完整交互和安装包验收仍在进行，macOS 尚待后续移交实施。

## Computer Use

Computer 仍是独立的桌面控制能力。在 Agent 工作台选择对应能力，在“设置 → Computer Use”管理桌面控制设置和操作系统权限。

独立 Nomi 的 Computer 配置仍可使用：

```toml
[tools]
max_recent_images = 3
[tools.computer]
enabled = true
max_screenshot_edge = 1568
```

## macOS 权限

Computer 能力首次使用需在「系统设置 → 隐私与安全性」中授权宿主应用：

- **辅助功能（Accessibility）**：鼠标键盘合成输入需要此项（未来 a11y 树读取/动作亦只需此项）。
- **屏幕录制（Screen Recording）**：截图需要此项（截图全黑或失败时检查）。

当前为反应式诊断：权限缺失时，工具结果会给出授权指引。

## 工具语义与审批

- Computer 为单工具 + `action` 参数形态。
- 只读 action（`screenshot`、`cursor_position`、`list_windows`、`wait`）按 **Info** 类审批——AutoEdit/Default 模式自动放行；操作类 action（点击、输入、滚动、拖拽、`focus_window` 等）按 **Exec** 类——Default 模式需用户确认。
- Plan mode 下 Computer 整工具不可见（只读规划阶段不操作桌面）。
- Browser 观察属于 Info，导航和页面修改属于 Exec；运行 guard 与 exact capability policy 不受网页内容影响。
- 推荐工作流：`screenshot` 观察 → 操作 → 再次 `screenshot` 验证。

## 截图与 token 治理

- 截图自动降采样到长边 ≤ `max_screenshot_edge`（默认 1568px，Anthropic 视觉推荐区间），最终 PNG 还受 5 MiB 上限约束；高熵画面会再次确定性降采样，坐标几何始终以实际发给模型的图片为准，并自动映射回真实屏幕（含 Retina 缩放）。
- 历史消息中只保留最近 `max_recent_images`（默认 3）张工具结果图片，并同时受每次请求最多 20 张和累计编码体积预算约束；超出的附件会被剥离，但保留文本和省略说明。提供商调用失败后也会清除待重放的历史截图，再持久化可恢复会话，避免会话文件、请求 token 和网关负载持续膨胀。
- OpenAI 协议的 tool 消息不支持图片：图片以紧随其后的 user 消息（`image_url` data URI）传递，并标注来源 call id。Anthropic/Bedrock/Vertex 走原生 `tool_result` 图片块。
- 外接 MCP 工具回传的图片同样经 `McpToolProxy` 映射进图片管道（单图 ≤ 5 MiB 上限）。

## 替代路径：其他外接 MCP

除内置 Computer 与 native Browser 外，仍可外接任意社区 MCP server（在 MCP 设置中添加），与上述能力互不冲突（工具名不同）。
