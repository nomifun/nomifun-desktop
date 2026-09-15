# 移除插件替换会话页：范围与验收

## 产品决定

用户明确否定“插件替换会话页”能力：它破坏了标准会话体验，不再继续美化或开发。
U0 会话页替换及 U1 页面内业务表单一并移出计划。**普通带 UI 的 App、常驻类型插件不受影响。**
after_tool 仍按此前成本取舍取消；before_model、before_tool 保留。

## 移除范围

- Agent 设置中的页面选择、默认偏好、参考视图草稿入口。
- 专属 AgentSessionViewHost、参考会话客户端及其 Inspector 页面。
- 作者 agent_view 声明、新建模板、Session UI catalog/binding 和 Surface 的会话授权。
- 仅供会话替换视图使用的 SDK agentSession 桥接接入。

会话统一使用已有标准 `/conversation/:id` 界面。旧 `/agent-sessions/:id` 页面链接只作
到同一 canonical conversation 的导航，不建立另一套客户端，也不改引擎绑定。

## 保留范围与数据边界

- 普通插件库、独立 App Surface、HTML/CSS/JS 页面与现有作者入口。
- Node 普通/continuous Service、storage/KV、工具贡献及生命周期管理。
- 正常 Agent Session 控制 API、会话历史、模型/引擎配置与工具权限。
- 已保存插件、历史声明、ui_binding_json 等数据不自动删除或迁移；保留必要的历史解析，
  但不能再授予 Session UI 权限。历史混合插件的普通 App/Service 不因旧声明被整体禁用。
- 不恢复实验开关，不提供默认关闭但仍可启用的会话替换入口。

## 本轮证据与当前状态

基线为 `8490163e1` 上的工作区。生产入口与专属客户端已移除，以下记录区分本机已验收和仍待验证的范围。

移除前的原生复现：旧参考视图可以发送并显示 `BASELINE`，切回专属内置回退页后却只
显示 `left/right` 标签、正文丢失；页面顶端大量视图管理与技术说明占据会话区域。
截图保存在本机 `.git/hook-product-validation/audit/17-*` 至 `20-*`。
此前尝试的 U1 新模板写入未成功应用，用户叫停后不再继续该模板；相关前端试改随能力移除清理。

## 验收清单

1. 标准会话入口与旧页面链接都进入同一标准会话；正文、工具、文件和输入区保持原功能。
2. 无 Agent 页面替换页签、偏好选择、参考视图入口，不再等待插件视图偏好加载。
3. 新发布的会话替换声明与显式 Session Surface/bridge 请求明确拒绝。
4. 普通带 UI App 能发布、打开、写入/读取 KV，常驻 Service 能启停及正常调用。
5. 历史混合插件保持普通 App/Service 可用，不改旧数据和发布摘要。
6. before_model/before_tool 及现有普通插件回归、TypeScript、桌面 880×600 边界与综合检查。
7. macOS 原生启动和正常退出；Windows 记录对应共享代码复核，正式制品另列验证。

本轮不自动推送或发布制品。测试只在明确的隔离数据目录进行；不通过读取已退出 app
的窗口状态来验退出，避免观察工具自动重启到默认开发目录。

## 已完成的验证

| 范围 | 结果 | 证据 / 限制 |
| --- | --- | --- |
| 前端标准链接、普通 App、H1a 回归 | 11 文件、55 测试、296 断言通过 | 包括旧链接到同一标准会话、不创建 Session/Surface、Service/KV 保留；非浏览器截图测试 |
| Product SDK | 5 项通过 | `bun run test:plugin-sdk`；确认无 agentSession 门面，Service/KV/错误码/超时不重试保持 |
| 后端退役与普通 App | 4 项通过 | `cargo test -p nomifun-app --test plugin_ui_sessions`，包含历史绑定不变、拒绝新声明、普通 Surface/KV、目录不泄漏退役 slot |
| 历史混合插件 | 1 项通过 | `service_application retired_agent_view`；历史 Active 使用测试仓储构造，验证 continuous 主机状态、普通 Surface/工具调用和 catalog digest 不变，不是 Windows 实机证明 |
| 历史声明解析 | 1 项通过 | `--lib historical_agent_view`；可读旧声明，但不能物化新发布 |
| 真实 Node | 8 + 3 项通过 | `service_process`、`service_storage_ipc`；调用、取消、进程退出与真实 SQLite/KV 回环 |
| 普通插件作者/保存流程 | 14 项通过 | `nomifun-app --lib router::plugin_product`；普通 UI-only 和常驻 Service 源码仍被接受 |
| 保留的工具消费者 | 8 项通过 | `plugin_product_discovery`；before_model、before_tool、发现、权限、真实 Node 取消和停用；模型使用 fixture，不新增实模声明 |
| 综合与合同 | 通过 | `bun run check`、`agent-v2-contract check`、`git diff --check` |

首次失败及修正：最初误把两个子模块当独立 Cargo test target，命令未运行；随后修正新测试
的静态借用和缺失 formal catalog entry。历史混合 Service 测试最初读错显式 start 计数器，
改为核验 continuous 的实际 Host Running 状态。作者测试改用公开 KV bridge 验证关闭后的
失效，不扩大私有 API。旧 Service 测试复用了同一取消句柄，现为独立调用建立独立句柄，
保留真实在途取消、handler 计数和停用断言，并单独重跑末条用例通过。未放宽生产保护。

### macOS 原生结果

- 当前源码 `bun run dev --no-watch` 编译并启动；进程打开的数据库经 lsof 确认为本轮
  `.git/hook-product-validation/before-tool-native-p1z81Z/data/nomifun-backend.db`。
- 原 `Model Recovery Check` 会话在标准 `/conversation/:id` 页面正常显示原始输入与
  `BASELINE` 回复；没有视图选择栏、专属 Inspector 或替换客户端。
- 个人 Agent 设置不再提供“Agent 页面”页签；普通插件库、创建/导入入口和独立 App
  Surface 保留。已打开现有“敏感文件检查”普通 App 页面并核对内容显示。
- Command-Q 后 dev 退出 0，测试 app 实例数为 0，61853/5173 端口均关闭；没有再用
  窗口读取触发自动重启。此次退出前没有活动子进程，活动 Node 取消/回收由上述专项证明。
- 截图：`audit/21-standard-conversation-restored.png`、`22-agent-without-page-replacement.png`、
  `23-ordinary-plugin-app-preserved.png`；日志、退出证据均在同一 `.git/hook-product-validation/`。

受改动文件已包含在两个内置 Engine 的源码摘要中，构建标识更新为 Nomi host64、Coding
loop97，旧会话/Agent 精确引擎绑定不自动改写。原生只读历史验证不等于旧构建可被新构建
静默续接；新会话按现行引擎选择处理。

### 未验证及跨平台接续

本次没有生成新的正式 app/DMG、签名或公证，也没有 Windows 实机结果；旧制品不能代表
本次移除后的源码。Windows x64 按下面步骤复核，Linux 不纳入首发：

```text
在 rf/agent-capability-platform-v2 核对交接提交和本地改动，不 reset/强推。
读取本退役记录和 AGENTS.md；会话页替换已取消，不恢复 U0/U1 或 after_tool。
使用隔离目录，确认标准会话与旧链接、Agent 无页面页签、普通 App Surface/KV/常驻 Service。
串行运行 plugin_ui_sessions、plugin_product_discovery、plugin_product 库测试、
service_application retired_agent_view、historical_agent_view、service_process/service_storage_ipc；
运行 test:plugin-sdk 与 bun run check。核对各项实际通过/失败，不能把本机结果当 Windows 证明。
保留旧插件、绑定及 catalog digest 数据；验证 Session UI grant/bridge 和新声明被拒绝。
正式构建、签名和安装验收另记；没有额外授权不上传、发布或推送。
```
