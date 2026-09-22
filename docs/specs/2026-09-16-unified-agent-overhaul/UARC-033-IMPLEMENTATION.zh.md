# UARC-033 Requirements、AutoWork 与 AgentExecution 收敛实施记录

> Wave 3 barrier：`5bec65d182d5798d5fa99398a39ae59a218bbba1`
> 平台：Windows 实现与验证；macOS shared compile 仍待 Mac 主机

## 交付

- AutoWork 只保留队列、claim、lease 与策略；执行统一提交到持久 `AgentExecution`，Attempt、retry、
  attention、cancel 与 canonical receipt 不再由 Requirements 维护第二套状态机。
- 2026-09-22 产品边界修正：复用 `AgentExecution` 仅复用持久执行/回执能力，不改变 AutoWork 的
  执行对象。AutoWork Attempt 复用用户绑定的主 AgentSession，以独立 `automation` link 绑定精确
  Attempt；不得创建协作子 Session、投影协作画布或在终态向主会话重复投影 summary。
- AutoWork 配置迁入 canonical Agent Store 的 append-only `automation/config-committed` 事实；REST、boot
  resume 与 runner 使用同一 owner-scoped CAS，不再依赖 `conversations.extra.autowork`。operation receipt
  保留原始 expected revision，A→B→重放 A 只返回历史回执，不回滚 Store head 或 live loop。
- 附件先从 exact saved Binding/Snapshot 解析 frozen workspace，在 Session operation lease 内执行
  AgentExecution preflight、幂等 staging 与第二次 admission；detached `spawn_blocking` 自身持有 Session
  operation lease，caller abort 不会让 delete fence 与仍在运行的 copy/publish 重叠。prompt 只包含
  workspace-relative 路径，宿主 `data_dir`/source path 永不泄露。
- canonical delete 使用 process-owned saga：Store admission fence、AutoWork/AgentExecution strict cleanup、
  前后 effect blocker、Browser/Cron/Requirement/SSH cleanup、精确 tombstone；重启恢复 deleting rows，
  pending/unknown 与清理未知均保持 durable quarantine。
- `110_uarc_agent_deletion_audit.sql` 增加 bounded、non-private、append-only override audit ledger；私有
  Session events 在 tombstone 时仍全部清除，显式 owner 风险接受事实在重启后可审计且不伪装 domain outcome。
- 物理删除 Agent-path IDMM crate、Gateway/UI/i18n/route/test surface，以及 Conversation 内无 caller 的
  supervision、turn-scope、continue 与 failover seam。Terminal 独立领域数据仍由后续 schema owner 处理。

## 删除与保留

- 删除 AutoWork 自有的 Conversation/Terminal receipt 状态机、parallel attempt receipt、IDMM hook 与
  Agent UI 控件；主 AgentSession 的实际 Turn 仍由 AgentExecution 通过 canonical Session owner 投递。
- 保留 Requirements business facts、queue policy、AgentExecution DAG/Attempt，以及 terminal-specific legacy
  storage 作为后续 `UARC-053/054` 的明确迁移输入；未添加永久 compatibility translator。

## 验证

```text
cargo test -p nomifun-requirement --lib -- --test-threads=1
  60 passed
cargo test -p nomifun-agent-execution --lib -- --test-threads=1
  103 passed
cargo test -p nomifun-agent-session --lib -- --test-threads=1
  34 passed
cargo test -p nomifun-app --test requirements_e2e -- --test-threads=1
  10 passed
cargo test -p nomifun-app --lib session_boundary_tests -- --test-threads=1
  11 passed
cargo test -p nomifun-db --test agent_store_reset -- --test-threads=1
  3 passed
cargo test -p nomifun-db --test id_schema_contract -- --test-threads=1
  20 passed
cargo test -p nomifun-db --test published_main_migration_upgrade -- --test-threads=1
  5 passed
```

全 App probe 为 520/530；仅 10 个已由 `UARC-042` 接管的 Robot canonical-session 过渡用例失败，
没有 UARC-033 新回归。商业模型证据仍只使用 StepFun Coding Plan `step-3.7-flash`：直接 provider probe
已成功；canonical Runtime dispatch 由 `UARC-051/052` 完成前，不把集成超时声称为通过。

## 平台状态

- Windows：verified。
- macOS：pending；shared contract 尚须随 Wave 6 在 Mac 真机编译与生命周期验证。
