# UARC-034 Schedule、Notification、Remote 与 SSH 实施记录

> Wave 3 barrier：`5bec65d182d5798d5fa99398a39ae59a218bbba1`
> 平台：Windows 实现与验证；macOS 原生证据待后续平台 Wave

## 交付

- Schedule 收敛为一个 Module、四个 slash Actions；Notification 与 Remote 保持 typed ingress/service，
  不作为 Agent grant。Cron relation 同时支持 canonical Store-only Session 与明确 legacy input。
- embedded Cron mutation 由 process-owned Drop guard 发布确定 terminal/OutcomeUnknown；panic、abort、caller
  drop 与 same-operation replay 不再永久挂起。4096 receipt 容量满时 fail closed，不重放已知或未知操作。
- SSH 收敛为 `ssh/fs.read`、`ssh/fs.write`、`ssh/exec`、`ssh/sudo`，物理退休 `ssh.connect` grant；
  sudo nonce exactly-once、`sudo -n` 与 no-new-privileges execution 均保持。
- SSH pool 新增 Session/link retirement 与 action lease；全部 SFTP/exec/sudo 在 transport 前持有 lease。
  teardown 是单次 process-owned operation，并发/取消 caller 共享 terminal receipt；Lost、Reaped 与
  AlreadyDown receipt 均在 Store terminal fact commit 前保留，Store append 失败后的同进程 retry 重放原证据。
- canonical delete 在 Cron reservation fence 后删除 exact job IDs、取消 timer/移除 generated skill；SSH cleanup
  先提交 durable started fact，再写 succeeded/uncertain，崩溃恢复不以“map 为空”伪造安全证明。
- cleanup pending 允许重入 owner，但 final tombstone gate 仍阻断；installation-owner manual override 只由
  独立 audit ledger 解除 quarantine，不把未知 SSH teardown 改写成成功。
- Remote MCP 与 App owner 的 idempotency key 上限统一为 128 visible ASCII bytes。

## 验证

```text
cargo test -p nomifun-cron --lib -- --test-threads=1
  197 passed
cargo test -p nomifun-cron --test service_integration -- --test-threads=1
  64 passed
cargo test -p nomifun-db --test cron_repository -- --test-threads=1
  43 passed
cargo test -p nomifun-public --lib -- --test-threads=1
  27 passed
cargo test -p nomifun-ssh --lib -- --test-threads=1
  40 passed
cargo test -p nomifun-gateway --lib -- --test-threads=1
  107 passed
```

`pool_lifecycle` 19/19 runner 通过；本 Windows 主机没有可用 sshd/ssh-keygen，因此其中 18 个真实 sshd
case（包括 retire/close race）按既有 harness 自跳过，未伪称为原生实跑证据。

## 平台状态

- Windows：verified（共享服务、SQLite、pool/receipt 与 App composition）。
- macOS：pending；SSH/通知/Remote 的 Mac 实机交互与 package lifecycle 仍由平台 Wave 验证。
