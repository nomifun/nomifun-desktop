# CAR 阶段状态

> 更新时间：2026-09-06
>
> 当前阶段：**本地隔离 Coding Engine 核心能力已完成；尚未接入 AgentSession 生产主链**
>
> 唯一状态源：本文与 `TASK-MANIFEST.json`。上一阶段
> `2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md` 不记录 CAR 状态。

## 阶段状态

| 项目 | 状态 |
|---|---|
| 文档系列 | in_progress |
| 源码/许可证基线 | completed |
| 多 Engine Catalog/Binding | completed |
| 隔离 Coding Engine Core | completed |
| Model Broker 原生取消/中央适配 | in_progress |
| Kernel Tool admission/owner adapter | pending_validation |
| Process owner adapter | pending_validation |
| Patch/File/VCS/Workspace owner 接线 | planned |
| Context/Compaction/Resume contracts | pending_validation |
| AgentSession 主链与异构 Registry | planned |
| 旧 Wrapper 删除 | planned |
| 生态消费者接入 | planned |
| 三平台发布 | planned |

## 任务快照

| 任务 | 状态 | 依赖 | 备注 |
|---|---|---|---|
| `CAR-00` | completed | 无 | Codex commit、选取/排除、LICENSE/NOTICE 和 owner 边界已核对 |
| `CAR-00A` | completed | `CAR-00` | Coding family Catalog、immutable Build、Stable/Canary alias 和 exact Binding |
| `CAR-01` | completed | `CAR-00A` | 独立 `nomifun-coding-engine`，未接生产组合根 |
| `CAR-02` | in_progress | `CAR-01` | Broker adapter 已存在；原生 Provider 取消和中央合同测试未完成 |
| `CAR-03` | pending_validation | `CAR-02` | Kernel adapter、Snapshot/active-set admission、标准 Tool surface 本地完成；等待主链验证 |
| `CAR-04` | pending_validation | `CAR-03` | `nomi-process-runtime` adapter 已完成；Windows start/wait/stdin/timeout/cancel/output 已验证，跨平台待远程 |
| `CAR-05` | planned | `CAR-03` | File/Patch/VCS/Workspace 通过 Wave2 owner 接线，未接 AgentSession 主链 |
| `CAR-06` | pending_validation | `CAR-03` | AGENTS/context/compaction/checkpoint contracts 已完成；SessionEvent/resume 主链未接 |
| `CAR-07` | planned | `CAR-04`～`CAR-06` | 远程主工作进程完成异构 Engine Registry 与 AgentSession 接线 |
| `CAR-08` | planned | `CAR-07` | 删除旧 Wrapper/Sidecar |
| `CAR-09` | planned | `CAR-08` | 平台生态与非 Agent consumer |
| `CAR-10` | planned | `CAR-08`、`CAR-09` | 三平台发布和 Stable admission |

## 当前本地交付

基线：

```text
branch: car/coding-engine
base_sha: 6a2a94bd192ef67eda5dd67331f6c047b1c1b315
code_commits:
  - b652fa29ce02c91f600d54abaecd98dfb967f9c4
  - c8b0193892ad7f1b73586b7570b7a2f0172c8d1b
  - 0e33dbed53e248ef5c926b548bfde378120bb400
  - f3ff31b4b6168c2cc6b4b3de867b5f922a3eb8a4
  - 449e06110c34902e070a1b7b506bdda2db9f147a4
code_tip: 449e06110c34902e070a1b7b506bdda2db9f147a4
```

已验证：

```text
cargo fmt --package nomifun-coding-engine
cargo check -p nomifun-coding-engine
cargo test -p nomifun-coding-engine
git diff --check
```

最后一次定向测试结果：`36 passed; 0 failed`。

未运行：

- `cargo clippy -p nomifun-coding-engine --all-targets -- -D warnings`：当前 stable
  toolchain 未安装 `cargo-clippy`；
- 全仓构建/测试：本次只新增未接生产主链的独立 crate，按仓库规则使用定向检查；
- live Provider、Broker 原生取消、AgentSession/SessionEvent E2E：这些属于远程中央接线；
- File/Patch/VCS 的真实产品路由：当前通过 Kernel/Wave2 owner adapter 留出合同，
  尚未接入生产 AgentSession。

## 已知集成缺口

- `ChatBrokerPort::open_chat_stream` 尚无 cancellation 参数；当前 adapter 只能停止
  转发/消费，不能证明底层 Provider 请求立即取消；
- Capability handler 合同尚无 `CancellationToken`；当前 Kernel adapter 只能在调用
  future 层 fail-fast；
- 当前 `CodingEngineCatalog` 只管理 Coding family，不是最终异构平台 Registry；
- `NativeResponsesItem` 与音频输出在 Coding P0 明确 unsupported；
- SessionEvent durable projection、UI、Remote/Automation、Workspace/File/VCS owner
  的生产接线尚未完成。

远程交接与禁止事项见 `HANDOFF-REMOTE-INTEGRATION.zh.md`。

## 状态更新规则

任务状态变更必须同时写明：

- `task_id`；
- 变更原因；
- commit SHA（如果已有代码提交）；
- 实际验证命令；
- 未运行项和准确原因；
- blocker 或 follow-up。

不在本文写入 API key、credential、主机地址、完整模型响应或秘密日志。
