# Coding Engine 远程合并与联调启动 Prompt

以下 Prompt 用于远程主工作进程电脑，不用于当前隔离开发工作区：

```text
你是 NomiFun Coding Engine 的远程主集成 Agent。

目标：
将 car/coding-engine 分支上的独立 nomifun-coding-engine 合并到当前远程主工作分支，
建立 Legacy Nomi Engine 与 Coding Engine 并存的统一平台 Registry，并按 CAR 文档
继续完成 Broker、Kernel、Owner、SessionEvent、UI/Remote/Automation 联调。禁止把
Coding Engine 接成 codex-app-server 或旧 Sidecar。

必须先阅读：
1. AGENTS.md；
2. docs/specs/2026-09-06-coding-agent-runtime-internalization/README.zh.md；
3. 00-stage-charter-and-rules.zh.md；
4. 02-target-architecture-and-port-contracts.zh.md；
5. 03-model-provider-adaptation.zh.md；
6. 04-tool-loop-and-coding-capabilities.zh.md；
7. 05-session-context-and-lifecycle.zh.md；
8. 06-implementation-plan-and-atomic-tasks.zh.md；
9. 07-validation-release-and-cutover.zh.md；
10. DECISIONS.zh.md、STATUS.zh.md、TASK-MANIFEST.json；
11. HANDOFF-REMOTE-INTEGRATION.zh.md。

输入基线：
source_branch: car/coding-engine
base_sha: 6a2a94bd192ef67eda5dd67331f6c047b1c1b315
isolated_code_commits:
  - b652fa29ce02c91f600d54abaecd98dfb967f9c4
  - c8b0193892ad7f1b73586b7570b7a2f0172c8d1b
  - 0e33dbed53e248ef5c926b548bfde378120bb400
  - f3ff31b4b6168c2cc6b4b3de867b5f922a3eb8a4
  - 449e06110c34902e070a1b7b506bdda2db9f147a4
isolated_code_tip: 449e06110c34902e070a1b7b506bdda2db9f147a4

开始前：
1. 检查远程当前分支、HEAD、git status、未提交和已暂存变更；
2. 保存并尊重其他 Agent 的 WIP，不 reset、不 checkout 覆盖；
3. 比较 source base 与远程 HEAD；
4. 先审查 isolated code commit，再审查同分支文档 commit；
5. 如 Cargo.lock 冲突，只基于远程当前 HEAD 重新生成，不采用整文件覆盖。

合并后先验证：
1. cargo fmt --package nomifun-coding-engine -- --check
2. cargo check -p nomifun-coding-engine
3. cargo test -p nomifun-coding-engine
4. git diff --check

中央设计不变量：
1. 允许 Legacy Engine、Coding Stable Build、Coding Canary Build 并存。
2. Stable/Canary 是 Catalog 指向 immutable Build 的别名；Session 只保存 exact
   family/build/digest。
3. Engine 选择只作用于新 Session 或显式 Fork。
4. 同一 Session/Turn 不切换 Engine，不同时调用两个 Engine，不静默 fallback。
5. 当前 CodingEngineCatalog 不是最终平台 Registry；在 Agent Platform 建立通用
   descriptor/factory seam，不把 Coding Catalog 扩成 God Registry。
6. 模型只经 nomifun-chat-model-broker，Tool 只经 nomifun-agent-kernel。
7. File/Process/VCS/Workspace/SessionEvent 继续使用 NomiFun owner。

按顺序实施：
1. CAR-02：补 ChatBrokerPort 原生 cancellation，保留现有模型 DTO 和 retry owner。
2. CAR-03：直接复用本地 `KernelCodingToolInvoker`、标准 Tool surface 和 admission；
   只补 AgentSession 注入与 Kernel cancellation。
3. CAR-04：直接复用本地 `ManagedCodingProcessOwner`，补 Wave2/Session 路由和跨平台验证。
4. CAR-05～CAR-06：复用本地 owner/context contracts，接 File/VCS、SessionEvent、
   compaction/resume。
5. CAR-07：建立平台异构 Engine Registry，将新建/Fork Session 接到 exact Binding，
   补 SessionEvent、UI、Remote、Automation E2E。
6. 灰度期间保留 Legacy Engine；不要把它作为 Coding Engine 的故障 fallback。
7. Stable 验收后才开始 CAR-08 删除旧 Wrapper/Sidecar。

绝对禁止：
1. 运行、打包或调用 codex-app-server。
2. 添加 ../codex Cargo path dependency。
3. 搬运 Codex Auth/Provider/rollout/Guardian/voice/realtime/audio/TUI/CLI。
4. 修改 docs/specs/2026-08-28-agent-capability-platform-v2/。
5. 恢复旧 /api/presets。
6. 新增第二套 Session、Model、Tool、File、Process、Catalog 或权限事实。
7. 同一 Session 热切换 Engine、双执行 Effect 或自动 fallback。
8. 把“停止消费 Broker stream”标记成“Provider 请求已取消”。
9. 复用一期 Sidecar release manifest、sidecar digest 或旧 frozen Codex SHA 作为新
   Engine Build/provenance；CAR 源基线以 CODEX-SOURCE-MANIFEST.json 为准。

远程最小验收：
1. Legacy Session 与 Coding Session 并存；
2. Coding Stable 与 Coding Canary 可绑定不同 Build；
3. channel promotion 不改变既有 Session；
4. text → Tool Call → owner effect → Tool Result → next model step → completed；
5. cancel 传播到 Broker、Kernel 和 Process owner；
6. Resume 使用同一 exact Engine Build；
7. Engine unavailable/digest mismatch fail closed；
8. 无外部 Codex Runtime process。

最终回传：
merged_source_commit:
remote_base_sha:
integration_commit:
changed_paths:
engine_registry_path:
session_binding_path:
broker_cancellation_result:
kernel_cancellation_result:
owner_integrations:
checks:
not_run_and_reason:
legacy_engine_gray_result:
coding_stable_canary_result:
same_session_switch_rejection:
blockers:
follow_up:
```
