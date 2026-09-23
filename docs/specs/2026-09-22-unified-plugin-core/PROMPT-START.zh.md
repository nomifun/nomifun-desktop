# Unified Plugin Core 新会话 Goal 启动 Prompt

将下面整段作为新 Codex 会话的第一条消息发送：

---

你正在仓库 `C:\Users\rika0\code\nomifun\bak\refactor\nomifun-desktop` 中执行一次最终的 Unified Plugin Core 重构。

请先完整读取并以此为唯一 Plugin 架构与施工权威：

`docs/specs/2026-09-22-unified-plugin-core/README.zh.md`

同时遵守仓库根目录 `AGENTS.md`，并读取与当前代码状态直接相关的现有 Plugin、Agent Action/Module、Desktop、DB、Runtime 和发布边界。若旧规格与 Unified Plugin Core 文档冲突，以 Unified Plugin Core 文档和其中列出的四项恒定产品目标为准。

请立即创建一个 Goal，目标为：

> 将当前 N1 安装聚合与 M1 发布聚合一次性 clean cut 为 Unified Plugin Core：一种 Plugin Package、一种本地 Plugin 身份、一套 JS Runtime/SDK、一个 generation 化 DataRoot、一条 Chat/Import 共用的 staging/validation/activation 链路，以及 Action + Binding 开放能力模型；完整接入 UI App、无 UI Agent/Desktop 增强、SQLite/KV/Files/Cache、配置/Credential、Preview、Migration、外部 Package/Backup，并物理删除旧双架构、兼容层、旧表、旧 API、旧 UI、旧合同、旧测试与过期文档。只有规格中的全部验收与完成审计通过后才将 Goal 标记为 complete。

执行要求：

1. 这不是规划任务。读取文档和审计现状后立即实施，并通过 Goal 持续工作直到真正完成或出现需要产品负责人决定的硬阻塞。
2. 不要把工作拆成分期产品交付，不要保留可运行的双架构中间态。内部可以按依赖顺序提交原子改动，但最终合并边界必须是完整 clean cut。
3. 不新增 N1/M1 adapter、旧 API alias、旧 manifest decoder、旧数据迁移器、feature flag 或“以后再删”的 TODO。
4. Plugin 子系统允许 clean-start；不得删除或破坏非 Plugin 用户数据。任何实际破坏性文件或数据库操作前必须精确确认目标。
5. 优先复用规格明确保留的底层机制：Artifact Store、安全路径原语、独立 Service process、取消/超时/进程回收、Surface MessageChannel、SQLite authorizer、Credential Store 和现有真实 Agent/Desktop 消费者。不要为了重构重写无关稳定基础设施。
6. Plugin 作者合同必须保持简单：统一 Manifest、Action + Binding、内联 JSON Schema、统一 SDK；外部目录/ZIP 与 Chat 产物必须进入同一个 `install_artifact`。
7. UI-only Plugin 必须零 Node 进程；有 Service 的 Plugin 使用每插件一个独立进程，不恢复 Shared Extension Host。
8. Preview 必须使用与正式运行相同的 SDK/Bridge/Storage adapter，只绑定临时 DataRoot；Preview 数据永不合并回正式数据。
9. 完整实现 generation 化 DataRoot、SQLite/KV/Files/Cache、JS migration、原子 Artifact/DataRoot 切换和单一 mutation journal。
10. Action/Binding 要接入当前统一 Agent Module/Action 与 Desktop 的真实消费者，但不能让 Plugin Core 再依赖复杂 Role/Provider/Consumer 图。不要误删其他核心领域仍在使用的全局合同。
11. 每次替换必须同步完成调用方改线、旧实现删除和测试更新；不要只增加 facade。
12. 保持用户无关改动，提交前检查 staged/unstaged 文件。需要创建分支时使用 `codex/` 前缀，不重写共享历史。
13. Renderer 或 UI 规则变化后运行 `bun run check:desktop-ui-boundary`；不增加移动端布局、手机快照或 880px 以下断点。
14. 使用最小定向检查推进，但在最终完成前运行与改动范围匹配的完整 Rust、UI、合同生成、边界、打包和桌面验证。平台条件不足的检查必须如实记录，不能伪记通过。
15. 允许使用子代理并行处理相互独立的只读审计、测试归类或明确隔离的实现任务；共享文件修改必须协调，主代理负责最终整合、删除审计和验证。
16. 持续更新 `docs/specs/2026-09-22-unified-plugin-core/README.zh.md` 的状态或在同目录增加唯一状态台账，但不得创建互相冲突的计划真相源。
17. 不要因为任务规模大、上下文压缩或一次检查失败而提前结束；保留进度并继续。只有全部验收、旧路径物理删除、文档与代码一致且无必要工作剩余时，才调用 Goal complete。

开始时请先完成以下动作，然后直接进入实现：

- 检查 Git 状态和当前分支；
- 读取规格全文；
- 建立现状 inventory：保留、重写、删除、真实消费者、DB 表、API、UI 路由和验证命令；
- 把 inventory 与规格逐项对齐，防止误删 Agent/Desktop 当前真实能力；
- 创建 Goal 并开始第一个能够同时落地新合同和删除旧入口的原子改动。

最终交付必须包含：

- Unified Plugin Core 全部生产实现；
- 新 canonical DB schema 与 clean-start 边界；
- 单一 Manifest、SDK、API、DTO、Bridge 和 UI；
- Chat 与外部 Import 的真实共用链；
- UI-only、headless、mixed Plugin 的真实运行证据；
- DB/KV/Files/Cache、Preview、Migration、Crash recovery、权限与 Backup 证据；
- 旧 N1/M1 生产代码、表、合同、路由、UI、测试、脚本和过期文档的删除证明；
- 完整命令与测试结果；
- 无剩余兼容层、双架构或隐含 TODO 的完成审计。

---
