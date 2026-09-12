# 代码质量审查记录：2026-09-12

本文件保留 R1 的结果；后续进度和剩余范围以[逐模块审计台账](audit-progress.zh.md)为准。

## 覆盖与结论

本轮已直接修改代码并完成回归验证，不是全仓逐行审查完成声明。
初步统计自有 Rust、TypeScript 和脚本（含测试）约 150 万行。
审查结合了前端入口依赖图、全仓引用搜索、构建配置、测试基线，以及
Agent Kernel / Session、前端写入队列、画布保存和草稿生命周期的人工检查。

本轮变更涉及 38 个源码、测试和依赖清单文件：新增 502 行、删除 2,500 行，
净减少 1,998 行；统计包含新增回归测试，不包含本记录。删除了 20 个旧文件。
未改动既有未跟踪目录 `.githooks/`，未提交或推送。

## 已修复的问题

| 位置 | 原问题与影响 | 修复与证据 |
| --- | --- | --- |
| `nomifun-agent-kernel/src/registry.rs` | 批量释放在首个错误处退出，其他已移出注册表的资源得不到释放 | 合并三条清理路径；尝试所有选中句柄，返回首个错误，保持作用域隔离 |
| 同上 | Provider 返回身份不匹配的句柄时，拒绝操作却未释放已获取资源 | 合并 direct / Role 句柄验证与去重逻辑；拒绝前释放，保留清理失败信息 |
| `nomifun-agent-session/src/store.rs` | observe 分别获取 Session、Head、事件、消息，可能拼接不同提交版本 | 同一只读事务返回一致快照；复用事务内分页及投影查询，也统一 head / cursor 的读取 |
| `serializedLatestWriteQueue.ts` | 错误或完成回调拒绝后，队列 tail 被拒绝，后续写操作被跳过 | 对内部调度链单独恢复，调用方仍能收到其回调错误 |
| `casSaveController.ts` | 保存过程中撤销到旧基线，被误标为已保存，提前解除离页保护 | 在途写入始终保留待保存状态，等待其完成后按新 revision 保存撤销结果 |
| 根 `package.json` | Bun 隔离布局下第三方包无法解析位于 UI workspace 内的 React 类型，引发大量类型错误 | 在根开发依赖提供同版本 React 类型；无需放宽 strict 或修改第三方声明 |

会话并发测试在修复前实际复现：同一 observation 的 Session.next_seq 为 6，
Head.last_seq + 1 却为 8。前端两个队列回调测试和画布撤销测试也先失败后通过。
Kernel 新增测试覆盖释放失败、未绑定/错误身份句柄、作用域隔离及重复获取去重。

## 删除与合并

- 删除未接入产品入口的旧 Creative Studio Projects 页面、列表模型、兼容服务和归档包装，
  以及只验证这些废弃包装的测试。现行 Canvas 功能及历史 URL 重定向不变。
- 删除未使用的 EmojiPicker、MarqueePillLabel、旧 TTS 请求封装、Google URL 辅助函数、
  旧 Agent 参数构建器及检测类型、旧通用响应订阅和运行时生命周期钩子。
- 生命周期策略测试改为调用产品正在使用的 Nomi fence；保留当前运行时的结构检查。
- 草稿存储移除退役 Claude 分支、单分支 switch、无必要泛型和类型断言。
  新增真实 React hook 测试，覆盖函数式更新、切换、重挂载、附件保留及清空。
- 模型健康状态测试直接调用产品使用的规则，删除测试中的独立实现副本。

删除判定不能只依靠静态 import 图。
`IconParkHOC` 会被 `ui/vite.config.ts` 在构建时注入：首次构建发现这一隐式引用后，
已完整恢复该组件，随后生产构建通过。生成的协议类型、测试夹具、构建脚本入口
也不能按“主入口不可达”直接删除。

## 最终验证

- `bun run check`：通过，包括类型、i18n、主题、图标及架构边界检查。
- `bun test --cwd ui`：3,288 个测试通过，0 失败，共 608 个测试文件。
- `bun run build:ui`：生产构建通过。
- `cargo test -p nomifun-agent-kernel -p nomifun-agent-session -p nomifun-agent-control-plane -p nomifun-agent-platform`：104 个测试通过，0 失败、0 忽略，另有空文档测试集通过。
- `git diff --check`：通过。

Rust 验证使用仓库 `scripts/run-dev.mjs` 提供的 Windows 工具链初始化，
仅设置测试子进程环境，未改变系统配置。未运行整个 Cargo workspace、桌面安装包、
macOS/Linux 平台验证或真实外部模型服务调用；上述通过结果不代表发布验收完成。

## 未完成的审查范围与已观察到的风险

- 超大模块仍需独立深审：Browser Hub（约 2.4 万行）、Knowledge Service（约 2.2 万行）、
  Conversation Service / Stream Relay，以及前端 CreativeCanvasProductRoute。
  这些行数包含文件内测试；本轮未进行单纯搬文件式拆分。
- `readOnlyConversation.sideEffects.test.ts` 和 `steerDraftSurvival.test.ts` 等测试仍存在
  在测试里重写业务逻辑的方式。它们不能替代直接运行产品实现的回归测试。
- 生产构建仍提示部分压缩后 chunk 超过 500 kB，最大约 2.24 MB；
  本轮未通过提高告警阈值掩盖这一问题，也未作未经测量的打包拆分。
- 本轮没有对权限边界、数据库全部逻辑引用、所有平台进程退出路径给出全面正确性保证。
  后续应围绕这些边界继续分批审查和验证。
