# 机器 2 启动 Prompt：精简 SSH Owner

> 修订日期：2026-09-02
> 权威指令：`05-system-capability-replacement-foundation.zh.md` §6.2
> Lane：`M2-W2-SSH`
> 代码基线：`f97b281c669d9298413008921a2d65407473ffa9`
> 指令基线：`d6de517056bb9d1b7eec1ec8e0c32e34cba97f40`
> 分支：`rf/m2-w2-ssh-owner`

旧 Prompt 已废止。将下方 Prompt 原样交给机器 2 的 coding agent。

```text
你正在执行 NomiFun 一期止损后的机器 2 SSH 独立任务。

先完整读取：

1. AGENTS.md
2. docs/specs/2026-08-28-agent-capability-platform-v2/05-system-capability-replacement-foundation.zh.md
3. 本文件

05 与旧设计、GLOBAL TODO 或历史 Prompt 冲突时，以 05 为准。

任务目标：

- 为 `ssh.fs.read`、`ssh.fs.write`、`ssh.exec`、`ssh.sudo` 提供真实、最小、
  可接入 production 的 typed owner。
- 只交付 SSH 领域实现、定向测试和主机接线说明。
- 不修改中央 host，不宣称 C8、HP-1、一期或发布完成。

一、Git 基线

执行：

git fetch origin --prune
git switch --track -c rf/m2-w2-ssh-owner origin/rf/m2-w2-ssh-owner
git status --porcelain
git merge-base --is-ancestor d6de517056bb9d1b7eec1ec8e0c32e34cba97f40 HEAD

要求：

- 工作树为空；
- 最后一条命令退出码为 0；
- 不 reset、不 force-push、不改写历史；
- 若本地已有同名分支或未提交工作，使用新的 clean clone/worktree。

二、独占写集

仅允许修改：

- crates/backend/nomifun-ssh/src/**
- crates/backend/nomifun-ssh/tests/**
- crates/shared/nomi-ssh/src/**
- crates/shared/nomi-ssh/tests/**

禁止修改：

- 所有 Cargo.toml、Cargo.lock；
- agent-contracts、domain-wave、agent-platform、nomifun-app、gateway；
- scripts、Gate、manifest、generated、evidence、docs；
- vendor/codex-runtime；
- 允许范围以外的任何文件。

若必须越界才能继续，记录接线需求并停止该部分，不要自行扩大写集。

三、必须实现

1. 最小 typed API

提供职责等价于以下 API，具体命名遵循现有 crate 风格：

- SshFsReadCommand / Outcome
- SshFsWriteCommand / Outcome
- SshExecCommand / Outcome
- SshSudoCommand / Outcome
- SshActionContext
- SshActionOwner
- SshActionError

不得用任意 JSON、字符串 dispatcher、metadata-only acknowledgement 或 mock fallback
冒充 owner。

2. 真实 authority 与 transport

- 使用现有 SshHostService、SshConnectionPool 和 nomi-ssh transport；
- context 携带 authenticated owner、AgentSession ID、已绑定 SshHostId 和 operation identity；
- Host ID 经过 typed 解析；
- host book 与 credential 继续按 owner 查询；
- action input 不接受 hostname、username、password、private key 或 token；
- 不新增 ConversationService/Nomi dependency。

3. 基本边界

- remote path 非空、有长度上限、拒绝 NUL；
- read/write payload 有上限；
- command 不超过 32 KiB；
- captured output 默认不超过 256 KiB，并明确 truncated；
- timeout 有非零下限和有限上限；
- read/write 使用 SFTP，不拼 shell 命令；
- 写入使用同目录临时文件 + rename，并准确报告服务端实际结果。

不要求证明所有 SSH/SFTP 服务端都提供跨实现的绝对原子覆盖；不要建设通用 CAS、
distributed receipt 或 reconcile 平台。

4. exec 与 sudo 分离

- `ssh.exec` 永不读取或注入 sudo credential；
- `ssh.sudo` 是唯一可使用 sudo credential 的 action；
- 无 sudo credential 时立即 typed fail；
- password 最多响应一次，只响应明确 sudo prompt；
- 普通程序的 password prompt 不能收到 sudo password；
- timeout/cancel 后关闭或回收连接，不能把残留输入留给下一条命令。

5. 失败与重试

- host-key changed 明确失败；
- disconnect/timeout/cancel 返回真实 typed error；
- 结果未知时不得自动重复 write/exec/sudo；
- 不建设通用 `succeeded/failed/uncertain/reconciled` 状态机；
- 不在本 crate 预制中央 Effect journal 或复杂 receipt。

6. Secret

- credential 不进入 command、outcome、Debug、Display、日志和测试快照；
- 测试不得包含真实主机、账号、token、password 或 private key。

四、明确停止

不要实施：

- 跨所有服务端的绝对原子覆盖证明；
- 通用 uncertain/reconcile 平台；
- 中央 Effect journal 的复杂 receipt；
- 为旧 API 建长期兼容层；
- 无真实 SSH 环境时的大规模网络模拟证明；
- workspace 全量 Gate、C8 或跨平台 evidence。

旧 API 只做当前编译所需的最小 additive 兼容；主机后续会删除旧入口。

五、最小完成定义

- 四个 action 有 typed owner API；
- 真实复用 host service、pool、SFTP/shell；
- owner/host binding、基本限额、host-key change、sudo 隔离有测试；
- timeout/cancel 会关闭或回收连接；
- unknown result 不自动重试；
- Secret 不泄漏；
- 旧 SSH crate 当前调用方仍可编译；
- changed paths 全部位于独占写集；
- Cargo.lock 无变化。

六、验证

运行：

cargo fmt -p nomi-ssh -p nomifun-ssh -- --check
cargo check --locked -p nomi-ssh -p nomifun-ssh
cargo test --locked -p nomi-ssh --lib
cargo test --locked -p nomifun-ssh --lib
cargo test --locked -p nomifun-ssh --tests -- --test-threads=1
git diff --check
git status --short

若本机有真实测试 sshd，再运行对应现有 lifecycle tests。若没有真实 sshd/sudo/认证资源：

- 只记录一次完整阻塞原因；
- 给出用户可手工执行的命令；
- 不反复重试；
- 不写 mock 绕过；
- 不生成 PASS。

七、提交与回传

提交：

git add -- crates/backend/nomifun-ssh crates/shared/nomi-ssh
git diff --cached --check
git commit -m "feat(ssh): add minimal agent capability owners"
git push -u origin rf/m2-w2-ssh-owner

禁止 force-push。

回传必须包含：

- base SHA 与最终 commit SHA；
- branch；
- changed paths；
- 每条验证命令与结果；
- 未运行的真实环境测试及原因；
- 主机需要完成的最小接线清单；
- 已删除或避免的旧复杂度。
```
