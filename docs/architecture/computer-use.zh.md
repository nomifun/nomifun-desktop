# Computer Use 架构

[English](computer-use.md)

Computer Use 由 NomiFun 私有 worker 和 vendored Cua Driver 提供。宿主不链接原生桌面驱动，不保存另一套执行事实；AgentSession、Turn、Event、Payload、Effect 继续是唯一 Agent 事实链。本文记录现役接缝，平台和发布验收状态见[专项进展](../handoffs/computer-use-rebuild.zh.md#12-单份执行进展)。

## 注册与冻结契约

包为 `nomifun.computer`，挂载为 `domain-computer`，能力为 `computer`，角色为 `system.computer_use`。资源种类为 `computer`，本机资源 ID 为 `local-desktop`，资源操作为 `observe`、`input`、`launch`。

[冻结清单](../../crates/backend/nomifun-agent-contracts/contracts/computer-tools.v1.json)保存真实 worker 的平台 `list` 结果；[精简覆盖](../../crates/backend/nomifun-agent-contracts/contracts/computer-tools.overrides.v1.json)提供标题、描述、加载分组、许可元数据与参数裁剪。`scripts/computer/generate-contracts.mjs` 从这两个输入生成 22 个精确 Action、Wave2 typed operation、工具展示与两个官方模板的授权集合。不要在宿主、UI 或文档复制另一份动作表。

canonical 授权 schema 接受已采集平台的定义；模型工具使用当前宿主平台的精确冻结 schema。当前清单含 Windows、Linux 实采定义，macOS 采集仍待对应平台执行。清单缺失某平台不构成该平台原生验证。

## 调用与进程

调用链为 Kernel → Wave2 → `router/computer_host.rs` → `nomifun-computer` → `nomifun-computer-worker`。宿主检查原调用的角色、provider、Snapshot、资源 owner、动作与操作许可；批量中的每步继续受本次冻结的精确 Action allowlist 限制。

`nomifun-computer` 按职责分为 `protocol`、`worker`、`result`、`surface`、`permissions`。worker 位于独立 Cargo 工作区 `apps/computer-worker`；宿主直接启动它，通过继承 stdin/stdout 使用上游私有 JSON 行协议 v1，没有守护服务、网络控制端点或新的事件通道。

worker 按需启动、按代际绑定驱动会话并串行派发；不同会话公平排队。排队 Stop 撤回尚未派发的请求；在途操作等待实际结果，超出动作预算时强杀并记结果未知。崩溃后的后继调用可以启动新代际并重新绑定，但不重放原动作。没有会话且空闲时自动退休；应用退出在数据库关闭前收束 worker，不能把移除缓存或 Future 取消当作原生回收证明。

## 结果、效果与模型分档

观察不建立 Effect；动作沿同一 canonical Effect 记录原调用与结果，Computer 的 `resource_key` 为空。驱动的 confirmed、partial、unverifiable、suspected_noop 均保留实际证据；派发前拒绝与排队撤回表达零效果，传输丢失、崩溃或超时强杀表达 unknown。动作已投递不证明用户目标已完成，后续观察和后置条件仍须核验。

无 ToolCalls 时不下发 Computer 工具；无 ImageInput 时只提供可用的无图工具与参数，并拒绝像素寻址。支持图片时使用完整档。模型提示、工具名和标题来自冻结输入；延迟工具按现役工具发现链加载。可信 session、图片尺寸与内部字段由宿主剥离或注入，模型输入不能取得会话或许可权威。图片使用现有 typed image part 和历史裁剪链；OpenAI Chat 的图片放在工具结果后的附件消息中。

Broker 按每次实际尝试的路由与适配器能力重新投影工具和提示，包括重试与 failover。投影保持原请求身份与历史消息；图片历史不能靠改写消息进入不支持图片的路由。进程内 attempt hook 在编码前登记本次表面，并在已校验的工具调用交付 Runtime 前绑定证据；该证据用于核对本次实际下发的 schema，不授予执行权限，也不写入另一套持久账本。

## 系统许可与平台边界

许可由冻结的 `system_permissions` 元数据派生，在实际调用时检查。缺权沿现有 `SYSTEM_PERMISSION_REQUIRED` 载体返回，未执行不是动作成功。状态入口为 `GET /api/computer/status`；系统许可继续使用 `/api/system/permissions` 及其 request/open-settings 入口，没有旧 Computer 权限别名。

macOS 目标为 14+，worker 由 App 直接启动并与 App 使用相同正式签名身份；TCC 归属和安装包行为需要在该平台实际验证。Windows 使用当前登录会话和普通完整性，提权窗口保留驱动的结构化拒绝，不借管理员执行设施绕过 UIPI。Linux 当前路径为 X11/XWayland 与 AT-SPI；原生 Wayland capture 尚未启用，portal、桌面 backend 与 GNOME helper 不能仅靠源码或 Xvfb 证明可用。

vendor 固定 commit `4d959bbfa57f0c89f310330e849500b222084194`；本地变更仅通过[登记补丁](../../vendor/CUA-DRIVER-PATCHES.md)和同步脚本应用。Linux 输入清理的 guardian 属于 worker 内部原生资源责任，不是宿主桌面实现或第二执行账本；其最终平台异常矩阵仍须独立验收。

## 升级与打包

依赖已退役 Computer 契约的会话不转换或修复，使用既有契约拒绝路径提示新建会话，历史保留。个人 Agent 需在工作台重新勾选新动作；官方 General 与 Companion 模板为新会话授予全部冻结动作。canonical 存储、数据代际和原有历史不因本重构改写。

各平台构建先独立构建 worker，从 Cargo 实际 executable 产物复制为 Tauri externalBin。开发入口注入 `NOMIFUN_COMPUTER_WORKER`；macOS 开发 App 使用其 bundle 内已签名副本。worker 先签、再封 App；正式证书、公证、安装包、真实模型与各端原生验收分别记录，脚本和单元测试不能替代这些证据。UI/API 契约版本为 43。
