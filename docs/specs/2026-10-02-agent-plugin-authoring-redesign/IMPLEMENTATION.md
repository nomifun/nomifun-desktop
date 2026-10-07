# 插件与小程序实施说明

更新：2026-10-08。当前产品设计见 [README.zh.md](README.zh.md)。本文只记录现行实现与本轮验证；被替换的交互、迁移记录和旧构建证据保存在 Git 历史中。

## 当前实现

产品入口与创作均由插件 Tab 承担。插件库提供搜索、类型筛选、常用、创作记录、回收站和用户分类；作品内容与管理操作使用统一的桌面布局。普通会话、欢迎页和 Agent 编辑器不接管作品创作过程。

创作使用 canonical AgentSession 与现有 Nomi Runtime。Session 归属读取现有 opening 事实，没有引入新的任务事实表、transcript 或插件 checkpoint。首个文件产生前的任务也能从创作记录找回；产物文件和已有创作 Session 保留，用户可从工作台继续。普通会话的当前代草稿通过新 PluginAuthoring Session 准入继续，使用作品文件和配置，不复制或读取原 Agent 历史；删除 legacy_reference、read_only 的普通 Session 回退。

创作保留用户 Agent 的 Skill、MCP 和资源配置，删除 plugin-only 能力上限与三次失败、64 步的插件专属观察器。依然使用原生 Session 权限、预算和 checkpoint，不添加另一套创作暂停或绑定机制。

创作成果通过统一 Plugin Core 保存并使用。用户无需发布、管理版本、执行检查或额外确认主机能力。Agent 的自动检查与交互测试仍是创作工具，内部保留真实运行证据；不把它们设计成用户必须完成的产品步骤。保存接口仍使用当前作品并发标识与真实内容摘要，保证源码、配置和数据写入正确。

版本恢复和上一版本指针已从生命周期与用户界面删除。恢复接口只恢复回收站中的同一作品。文件系统事务中的失败回滚用于恢复未完成的原子写入，不构成版本管理功能。

安装和导入统一返回 `result: { outcome: 'installed', plugin }`。删除权限扩展确认对象、确认请求字段、待审批返回分支以及 authoring.approve API。导入界面从目录、ZIP 或备份读取作品概览后直接导入；副本与更新的配置和身份语义保持明确。

导入与设置使用独立的 PluginDialogs.module.css。三种导入来源采用同尺寸卡片，操作位于固定 footer；正文随桌面可用高度滚动。配置和直接运行共用 PluginParameterFields：简单 Schema 使用命名表单，数字、布尔值及枚举保持实际 JSON 类型，选填空值省略；复杂 Schema 保留完整 JSON 和说明。配置结构和主机能力收进高级设置，凭据按实际需要选择已有引用。切换“独立副本”后再返回更新，会恢复已有配置与凭据。异步读取完成时校验当前弹窗请求，过期返回不能填入重新打开的弹窗。

工作台、预览和正式作品继续共享真实 Surface 组件。凭据内容来自现有 Host Credential Store；导出中不携带秘密值。关闭、停用、回收、恢复和重新启动后的作品状态以持久化插件记录为准。

## 代码入口

| 区域 | 入口 |
| --- | --- |
| 作品库与导航 | PluginLibraryPage、PluginWorkspace、pluginPlatformModel、pluginLibraryState |
| 创作与产物 | PluginAuthoringPage、PluginAuthoringArtifacts、pluginAuthoringLaunch |
| 使用与管理 | PluginRunPage、PluginPinnedEntries、PluginImportDialog、PluginConfigurationDialog |
| 真实界面与存储调用 | PluginSurfacePanel、pluginPlatformBridge、nomifun-plugin-platform |
| canonical 产品 Session | plugin_authoring_sessions.rs、CanonicalAgentSessionOwner、AgentSessionStore |
| 创作工具 | nomifun-plugin-development、plugin_development.rs、plugin_authoring.rs |
| 保存与生命周期 | nomifun-plugin-platform/install.rs、repository.rs、nomifun-app/router/plugin.rs |

## 本轮定向验证

| 检查 | 已执行结果 |
| --- | --- |
| PluginDialogs.interaction.test.tsx | 4 项通过：单次导入无额外审批；副本切回更新保留原配置及并发标识；保存现有设置与拒绝无效 JSON；失败显示在设置弹窗内并可重试 |
| 插件 UI、桥接与入口回归 | 78 项通过，覆盖参数表单与 JSON 往返、源码保存、原配置预览、暂停精确重试、分类检索、网络失败、Surface 生命周期及实际 API 调用 |
| Control plane 插件绑定 | 4 项通过：继承用户 Agent 和官方模板完整能力、冻结 Skill/MCP、共享正常编译缓存、保留原模型与 owner 校验 |
| plugin_e2e | 21 项通过，使用 HTTP 测试宿主及模拟模型响应，实际运行插件 Service、存储和主机动作；覆盖创作修复、数据重启、原会话工具调用、多产物未完成拒结算、草稿即时关联且不导入普通会话历史 |
| Plugin Platform 定向 Rust 回归 | 21 项通过：安装、生命周期、迁移与草稿文件替换 |
| 插件交付与预览主机调用 | 交付检查 2 项、预览默认声明能力及显式收窄 1 项通过 |
| 数据库与 clean cut | 当前/未知/缺口/checksum lineage 4 项、reset 2 项、已有数据迁移并连续两次重启 1 项通过；源码、配置、分类、凭据引用和当前数据保留 |
| Native checkpoint 与压力处理 | 2 项通过，保留普通 Runtime 的精确暂停、恢复与资源契约 |
| UI 构建、类型与文案 | 生产构建、typecheck、i18n 类型生成及一致性检查通过 |
| PluginPlatformBoundary.test.ts | 7 项通过：统一桥接、真实 Surface、创作归属、WebUI 本地操作边界、凭据引用、统一作品形态、退役检查与版本 UI |
| pluginPlatformLocales.test.ts | 1 项通过：中英文键一致且无退役产品词汇 |
| bun run check:desktop-ui-boundary | 通过，最低桌面视口 880×600 |
| 真实组件浏览器视觉 QA | 1280×800、880×600；外层导航 64px 与展开 240px（最窄内部 pane 640px）；作品库、创作初始与源码、后台能力使用与管理、导入与设置、明暗主题均已查看，无横向溢出 |
| 浏览器交互 QA | 搜索「纪要」筛至一个作品；分类筛选；名称与分类保存；ZIP 概览；高级设置收纳；创作示例填入及发送启用；成果折叠与源码切换。均经生产组件事件处理与隔离接口返回完成 |
| 参数设置浏览器 QA | 880×600 下实际修改字段并检查 JSON 同步；四个参数、编辑 JSON 入口与固定保存/取消按钮可见，文档宽度没有溢出 |

UI 测试使用受控 DOM 和接口数据，证明组件调用、状态和边界行为；浏览器视觉 QA 使用真实生产组件、样式与资源，作品、模型、Session 与桥接返回由隔离 fixture 提供，外层导航为简化占位。Rust 集成测试实际运行 Node 插件与本地存储，数据库测试重新打开持久化文件；它们不代替真实模型供应商生成和整个桌面应用进程的验收。

typecheck、i18n、Desktop UI、Agent Session、Unified Plugin、UARC 和 Process Runtime 边界均已执行。视觉复核覆盖空态、检索、分类、创作、管理、导入与设置，并覆盖明暗主题与 880×600。

## 真实环境验收

实际 Provider 的自然语言需求提取、生成与修复仍需独立实测，覆盖 UI-only、后台能力、综合插件及已有作品更新。脚本模型响应不替代这些证据。

真实桌面任务还需复核补充输入、当前会话工具调用、880×600 使用、完整退出重启与数据恢复。该验收需要可用的 Provider 配置；只针对实际失败修复并运行受影响检查。
