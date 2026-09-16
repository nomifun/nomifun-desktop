# CAR-00 源码与许可证基线任务启动 Prompt

这是本阶段第一个原子任务的可直接执行 Prompt：

```text
你负责 NomiFun CAR 阶段的 CAR-00：源码、许可证和边界冻结。

任务目标：
只完成设计输入审计，不修改生产代码。固定 ../codex 的源码 commit，核对 NomiFun
当前 Runtime/Model/Kernel/Session 边界，形成可复查的选择性抽取、依赖替换、排除和
许可证记录，为 CAR-01 之后的代码任务提供唯一输入。

必须阅读：
1. AGENTS.md；
2. docs/specs/2026-09-06-coding-agent-runtime-internalization/README.zh.md；
3. 00-stage-charter-and-rules.zh.md；
4. 01-audit-and-source-baseline.zh.md；
5. 02-target-architecture-and-port-contracts.zh.md；
6. CODEX-SOURCE-MANIFEST.json；
7. NomiFun 当前：
   - crates/backend/nomifun-codex-runtime/
   - crates/backend/nomifun-chat-model-broker/
   - crates/backend/nomifun-agent-kernel/
   - crates/backend/nomifun-agent-platform/
8. ../codex 当前 checkout 的：
   - codex-rs/core/src/session/
   - codex-rs/core/src/tools/
   - codex-rs/core/src/unified_exec/
   - codex-rs/core/src/context/
   - codex-rs/core/src/compact.rs
   - codex-rs/core/src/agents_md.rs
   - LICENSE 和 NOTICE

允许修改：
1. docs/specs/2026-09-06-coding-agent-runtime-internalization/ 下的 CAR-00 相关文档；
2. 本目录 CODEX-SOURCE-MANIFEST.json；
3. 本目录 STATUS.zh.md 和 TASK-MANIFEST.json 中与 CAR-00 直接相关的内容。

禁止修改：
1. docs/specs/2026-08-28-agent-capability-platform-v2/ 下的任何文件；
2. 任何 crates/、Cargo.toml、Cargo.lock、scripts/、packaging/ 生产代码；
3. vendor 外部 Runtime、app-server、Sidecar 或 credential channel；
4. ../codex 源码；
5. 任何旧 /api/presets、Conversation 或 Nomi Runtime 兼容入口。

必须确认：
1. Codex commit 是 6af345407d9c2a568da9d01b6c4b81a9e61495c0；
2. 选定 source path 都存在；
3. 每组 source 都有 copy/adapt/rewrite/exclude 和 NomiFun owner；
4. voice、realtime、audio、TUI、CLI、Guardian、app-server、Codex Auth/Provider/rollout
   已明确排除；
5. Codex root LICENSE/NOTICE 和传递依赖审计要求已记录；
6. 不新增 ../codex Cargo path dependency 或外部 Runtime binary；
7. 当前旧 Wrapper 引用只被登记为后续 CAR-08 删除范围，不被伪装成已完成。

最小检查：
1. 解析 CODEX-SOURCE-MANIFEST.json；
2. 验证每个非通配符 source path 在 ../codex 存在；
3. 检查 LICENSE 和 NOTICE；
4. git diff --check；
5. 检查 changed paths 全部位于本目录。

如果发现某个候选模块无法脱离 Codex 产品依赖，标记为 exclude 或后置，不要为了
保持源码相似度扩大范围。不要启动 live model、app-server 或任何外部 Runtime。

最终报告：
task_id: CAR-00
base_sha:
commit_sha:
changed_paths:
selected_source_groups:
excluded_source_groups:
owner_replacement_summary:
license_summary:
checks:
not_run_and_reason:
blockers:
central_paths_touched:
follow_up:
```
