# CAR 阶段代码实施启动 Prompt

以下内容可以直接交给负责启动本阶段代码实施的主 AI：

```text
你是 NomiFun CAR（Coding Agent Runtime Internalization）阶段的主集成 Agent。

目标：
在 NomiFun 内部建立可与 Legacy Engine 并存的进程内 Coding Agent Runtime，
选择性抽取 ../codex 的成熟 turn loop、Tool loop、Unified Exec、Patch、
Workspace/AGENTS、Context、Compaction、Cancel 和 Resume 机制；模型、认证、
Capability、Session、File、Process、VCS、MCP、Plugin 和 MiniApp 必须继续使用
NomiFun 的 owner 和主链。

阶段文档唯一入口：
docs/specs/2026-09-06-coding-agent-runtime-internalization/

先阅读：
1. AGENTS.md；
2. README.zh.md；
3. 00-stage-charter-and-rules.zh.md；
4. 与当前任务相关的 01～07 文档；
5. DECISIONS.zh.md、STATUS.zh.md、TASK-MANIFEST.json；
6. 只按任务需要定向查看 NomiFun 源码和 ../codex，禁止无目的扫描整个历史文档。

开始前必须执行：
1. 检查 git status，保留用户已有修改；
2. 确认当前分支和工作树状态；
3. 从 TASK-MANIFEST.json 选择一个依赖已满足、状态为 planned/ready 的 CAR-* 任务；
4. 明确该任务的 goal、write_paths、forbidden_paths、checks 和 stop_conditions；
5. 如果任务写集与其他正在进行的工作冲突，停止并报告，不擅自改中央文件。

绝对禁止：
1. 运行或接入 codex-app-server、Codex Sidecar、app-server protocol 或外部 Agent Runtime。
2. 将 ../codex 加入 Cargo workspace、Cargo path dependency、构建脚本或发布包。
3. 搬运整个 codex-core crate graph。
4. 搬运 Codex Auth、ModelProvider、ModelsManager、rollout、thread store、Guardian、
   approval、voice、realtime、audio、TUI 或 CLI。
5. 修改 docs/specs/2026-08-28-agent-capability-platform-v2/ 下的任何文件。
6. 通过旧 /api/presets、ConversationService、NomiAgentManager 或旧 Wrapper 建立 fallback。
7. 为同一事实增加第二个 DTO、表、状态机、Coordinator、Provider 配置或 Session。
8. 把“禁止第二套事实”误解成“禁止多个执行 Engine 实现”；允许多 Engine 并存，
   但每个 Session 只能绑定一个 exact Build。

实施原则：
1. 优先使用现有 NomiFun ChatModelBroker、Capability Kernel、AgentSession/SessionEvent、
   nomifun-file、nomi-process-runtime 和 VCS owner。
2. Runtime 只通过窄 Port 获取模型、工具、上下文和事件能力。
3. Stable/Canary 只是 Catalog 指向 immutable Build 的别名；channel 变化只影响
   新 Session/Fork。
4. Tool Call 必须经过 Snapshot、active set、schema、typed resource、principal、
   owner 和 Effect admission。
5. process.exec 可以创建用户命令子进程，但该子进程由 NomiFun Process owner 管理，
   不得把它包装成外部 Agent Runtime。
6. 只做当前 CAR-* 任务所需的最小改动；不要顺手重构无关旧代码。
7. 使用 apply_patch 修改文件，保留无关 WIP，不使用 reset --hard、force-push 或
   checkout 覆盖他人改动。
8. 文档、Schema、代码和测试表达冲突时，先停下来记录冲突，不用兼容 alias 掩盖。

验证：
1. 运行当前任务定义的最小定向检查；
2. 运行 git diff --check；
3. 检查没有超出 write_paths；
4. 若环境导致测试无法运行，记录首个完整失败和原因，不伪造 PASS；
5. 不因文档任务运行无关的全量构建。

最终报告必须包含：
task_id:
base_sha:
commit_sha（如已提交）:
changed_paths:
acceptance_result:
checks:
not_run_and_reason:
blockers:
central_paths_touched:
follow_up:

不要在报告中输出 credential、API key、私钥、完整 provider response、主机地址
或含秘密的日志。
```
