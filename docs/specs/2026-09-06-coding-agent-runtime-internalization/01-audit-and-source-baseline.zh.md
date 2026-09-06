# CAR 代码质量与源码基线审计

## 1. 审计快照

本阶段使用以下本地源码快照作为设计输入：

```text
NomiFun workspace:
  当前工作区：本仓库
  审计日期：2026-09-06

Codex workspace:
  相对路径：../codex
  branch：main
  commit：6af345407d9c2a568da9d01b6c4b81a9e61495c0
```

Codex 当前 `codex-rs/core/src` 约有 495 个文件、约 20 万行生产和测试代码。
`codex-core` 的 Cargo 依赖约 50 个 Codex 内部 crate，涵盖模型、认证、历史、
插件、MCP、沙箱、Realtime 和产品协议。

Codex 仓库具备较成熟的工程质量基础：

- `cargo fmt`；
- cargo shear；
- Bazel；
- Linux/macOS/Windows 构建；
- core integration test；
- exec/Wine 相关验证；
- 大量 Tool、Session、Patch、Context 和 Process 行为测试。

本机对 `codex-core` 的 offline check 曾因缺失 Git 依赖 `runfiles` 未完成。
该结果只能说明环境无法完成离线依赖解析，不能记为源码编译通过或失败。

## 2. Codex 质量判断

### 2.1 值得抽取的成熟机制

| 领域 | 主要路径 | 判断 |
|---|---|---|
| Turn/Session | `codex-rs/core/src/session/` | 行为成熟，但模块过大、深耦合 Codex Session/Config |
| Tool Registry/Router | `codex-rs/core/src/tools/registry.rs`、`router.rs` | Tool 暴露、schema、runtime 匹配和错误处理成熟 |
| Parallel Dispatch | `codex-rs/core/src/tools/parallel.rs` | 有真实并发闸门和 cancellation 处理，可收窄后复用 |
| Unified Exec | `codex-rs/core/src/unified_exec/` | 输出、stdin、PTY、取消、超时和进程状态覆盖较完整 |
| Patch | `codex-rs/core/src/apply_patch.rs`、`tools/handlers/apply_patch*` | 解析和边界测试较充分 |
| AGENTS.md | `core/src/agents_md*`、`context/world_state/agents_md.rs` | 适合 Workspace 指令发现和层级合并 |
| Context/Compaction | `core/src/context/`、`context_manager/`、`compact*` | 窗口管理和摘要思想成熟，但持久化绑定 Codex |
| Steer/Cancel | `session/turn_input.rs`、`turn_suspension.rs` | 有可借鉴的 turn 边界和取消语义 |
| Review/Plan | `tools/handlers/plan*`、相关 context | 可转成 NomiFun Skill/Workflow，不复制审批体系 |

### 2.2 不适合整体复制的部分

| 耦合 | 典型依赖 | CAR 处置 |
|---|---|---|
| 模型和认证 | `codex-api`、`codex-login`、`codex-model-provider` | 删除，接 `ChatModelBroker` |
| 模型目录和配置 | `codex-models-manager`、`codex-config` | 删除，接 Snapshot/Model Route |
| 历史和持久化 | `codex-history`、`codex-rollout`、`codex-thread-store` | 删除，接 SessionEvent 和可丢弃 cache |
| 权限产品 | Guardian、approval、permission profile | 删除，接 ThinAuthority + FullAuto |
| 插件产品 | `codex-plugin`、Core Plugins、Extension API | 删除，接 NomiFun Catalog |
| 外部协议 | app-server、JSONL、server request | 删除，接进程内 Rust Port |
| 媒体能力 | voice、Realtime、audio、image | 永久排除 |
| 产品入口 | TUI、CLI、app-server UI | 永久排除 |

## 3. NomiFun 当前 Runtime 基线

当前 `crates/backend/nomifun-codex-runtime` 的优点：

- 外部 executable、工作目录和 digest 校验；
- managed process tree；
- inherited credential handle；
- stdio frame 限制；
- checkpoint containment/digest 校验；
- dispose 幂等和定向测试。

但它的产品定位不能作为 CAR 目标：

- `RuntimeProcessConfig::pinned_app_server` 固定启动
  `app-server --listen stdio://`；
- `CodexRuntimeClient`、`RuntimeIngressPort` 和协议层围绕外部 JSONL；
- `RuntimeStartTurnBrokerBridge` 目前只接受文本输出；
- reasoning、Tool Call、Native Responses Item、Provider Round 进入当前桥接会失败；
- Host 需要管理外部 Runtime process、hello、credential channel 和 dispose。

因此：

```text
可复用：进程清理、边界测试、错误分类、测试方法
不可复用为目标：Sidecar 协议、外部启动器、hello、native_action RPC、Wrapper 主链
```

CAR-08 负责最终删除旧 Wrapper 的生产可达路径；不在旧 Wrapper 上继续加功能。

## 4. 源码抽取策略

源码抽取不是复制整个目录，而是对每个候选模块执行：

```text
固定 commit
  → 读取源文件及其许可证
  → 标记 copy / adapt / rewrite / exclude
  → 替换 NomiFun owner/Port
  → 迁移行为测试
  → 删除未使用 Codex 依赖
  → 运行 NomiFun formatter/lint/test
```

机器可读清单见 `CODEX-SOURCE-MANIFEST.json`。每个进入生产的复制或改写文件必须：

- 保留适用的版权和许可证声明；
- 对修改内容留下显著 modification notice；
- 在发布 NOTICE 中保留上游和传递依赖归属；
- 通过最终的 transitive license audit。

## 5. 主要审计风险

| 风险 | 可能表现 | 防护 |
|---|---|---|
| 巨型模块迁移 | 将 `session/mod.rs` 整体搬入 NomiFun | 以 Actor、Model、Context、Tool、Lifecycle Port 拆分 |
| 隐式全局状态 | Config、feature flags、analytics、thread store 渗入 Runtime | 只接不可变 RuntimeProfile 和显式 Port |
| Provider 泄漏 | Runtime 直接使用 OpenAI/Codex client | 所有模型请求经 ChatModelBroker |
| 历史双主链 | SessionEvent 与 rollout 同时成为事实 | SessionEvent 唯一事实，cache 可丢弃 |
| 工具旁路 | Tool 直接调用具体 handler/Browser/Plugin | 统一 Capability Kernel admission |
| 范围膨胀 | Realtime、Guardian、Code Mode 等不断进入 P0 | 以 source manifest 和任务禁区阻断 |
| 许可证遗漏 | 只看根 LICENSE，不看源文件/传递依赖 | CR-00 和 CAR-10 release audit 双重检查 |
