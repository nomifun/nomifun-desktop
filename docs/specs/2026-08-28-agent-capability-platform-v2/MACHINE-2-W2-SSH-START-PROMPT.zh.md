# 机器 2 启动 Prompt：Wave 2 SSH Owner

> 分配日期：2026-09-02
> Lane：`M2-W2-SSH`
> TODO：`W2-002`
> 主机代码 checkpoint：`f97b281c669d9298413008921a2d65407473ffa9`
> 机器 2 分支：`rf/m2-w2-ssh-owner`

将下方 Prompt 原样交给机器 2 的 coding agent。机器 1 是 integration owner，在机器 2
工作期间继续 Wave 5、中央接线、旧组合图拆除及其他非 SSH 工作，不等待本 lane 完成。

```text
你正在执行 NomiFun Agent Capability Platform v2 的机器 2 独立开发任务。

任务：
- Lane：M2-W2-SSH
- 对应全局 TODO：W2-002
- 目标：为 ssh.fs.read、ssh.fs.write、ssh.exec、ssh.sudo 实现真实 production owner
  primitive。
- 本任务只交付 SSH 领域实现、测试和中央接线说明；不得自行宣称 W2-002、C8、HP-1
  或发布已经全局 closed。

一、固定 Git 基线

仓库：nomifun-desktop
远端主分支：origin/rf/agent-capability-platform-v2
必须精确基于：

f97b281c669d9298413008921a2d65407473ffa9

执行：

git fetch origin --prune
git cat-file -e f97b281c669d9298413008921a2d65407473ffa9^{commit}
git switch --track -c rf/m2-w2-ssh-owner origin/rf/m2-w2-ssh-owner
git rev-parse HEAD
git status --porcelain

HEAD 必须精确等于上述 SHA，工作树必须为空。若同名本地分支已存在，不要 reset 或覆盖；
使用新的 clean clone/worktree。若远端任务分支尚未出现，再从上述 SHA 新建同名分支，
但不要从主分支最新 HEAD 猜测基线。

开始前完整读取并执行：

1. AGENTS.md
2. docs/specs/2026-08-28-agent-capability-platform-v2/DECISIONS.zh.md
3. docs/specs/2026-08-28-agent-capability-platform-v2/02-capability-catalog-and-agent-presets.zh.md
4. docs/specs/2026-08-28-agent-capability-platform-v2/03-target-architecture.zh.md
5. docs/specs/2026-08-28-agent-capability-platform-v2/04-migration-and-validation-plan.zh.md
6. docs/specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md

二、独占写集

仅允许修改或新增：

- crates/backend/nomifun-ssh/src/**
- crates/backend/nomifun-ssh/tests/**
- crates/shared/nomi-ssh/src/**
- crates/shared/nomi-ssh/tests/**

这两个 SSH crate 在本 lane 期间由机器 2 独占。

不得增加第三方或 workspace dependency；不得修改任何 Cargo.toml 或 Cargo.lock。优先使用
现有 tokio、serde、async-trait、thiserror、nomifun-common 与 nomi-ssh 能力。

三、禁止写路径

禁止修改允许范围以外的任何路径，尤其包括：

- Cargo.toml、Cargo.lock
- crates/backend/nomifun-agent-contracts/**
- crates/backend/nomifun-agent-domain-wave1/**
- crates/backend/nomifun-agent-domain-wave2/**
- crates/backend/nomifun-agent-domain-wave3/**
- crates/backend/nomifun-agent-domain-wave4/**
- crates/backend/nomifun-agent-domain-wave5/**
- crates/backend/nomifun-agent-platform/**
- crates/backend/nomifun-v4-root/**
- crates/backend/nomifun-app/**
- crates/backend/nomifun-gateway/**
- scripts/**
- docs/specs/**
- vendor/codex-runtime/**
- 任何 generated contract、Gate、manifest、evidence 或 build output

若发现必须修改禁止路径才能继续，停止该部分并写入最终 integration notes；不得越界修改。
不得提交凭据、主机地址、私钥、token、测试账号、本机绝对路径或任何 secret。

四、实现合同

1. 新增 action-specific typed owner API。

必须有职责等价于以下类型的明确 API，命名可遵循本 crate 现有风格：

- SshFsReadCommand / SshFsReadOutcome
- SshFsWriteCommand / SshFsWriteOutcome
- SshExecCommand / SshExecOutcome
- SshSudoCommand / SshSudoOutcome
- SshActionContext
- SshActionOwner
- typed SshActionError

不得以任意 JSON、字符串路由、旧 SshBackend facade、metadata-only success 或 mock fallback
充当 production owner。

2. 必须真实复用现有 SSH authority 与 transport。

Owner 必须调用现有 SshConnectionPool、SshHostService 和 nomi-ssh transport。上下文至少
明确包含：

- authenticated principal/owner ID；
- canonical AgentSession ID，不新增 ConversationService/Nomi dependency；
- 已绑定的 SshHostId；
- operation/idempotency identity；
- remote cwd 或明确默认值。

Host ID 必须通过 SshHostId 解析；host book 查询和凭据解密继续使用 owner-scoped
SshHostService。不得从 action input 接受 hostname、username、明文凭据或私钥来绕过
resource binding。

可以为 pool 增加 AgentSession 命名的新入口，同时保持旧公开 API 可编译；不要大规模
破坏仍由主机使用的 conversation compatibility API。

3. 所有 IO 必须有界。

至少固定并测试：

- 非空、有限长度的 POSIX remote path，拒绝 NUL；
- read 最大字节数，必须在 transport 读取阶段限制，不能无限读取后截断；
- write 最大 payload；
- command 最大字符数，不超过 32 KiB；
- timeout 有非零下限和有限上限；
- captured output 默认不超过 256 KiB；
- 截断返回 truncated=true，同时 transport 继续排空到 sentinel，保持协议同步。

上限应集中定义并由测试固定。

4. 提供严格写入语义。

现有 write_file_atomic 在 rename 失败后 remove+rename，存在非原子窗口。新 owner 不得把
这种结果报告为 atomic success。为新 owner 提供严格写入口：

- 使用同目录临时文件；
- flush/sync 后提交；
- 只有能证明原子替换时才返回 Applied；
- 服务端不支持可靠覆盖时 fail-closed，或返回明确 uncertain/unavailable；
- 不得删除目标后再伪装成原子成功；
- 失败时尽力清理临时文件并保留真实 outcome。

为兼容旧消费者，可以新增 strict API，而不是无审计地改变旧方法语义。

5. 物理分离 exec 与 sudo 权限。

必须保证：

- ssh.exec 不能获得、触发或自动注入 sudo password；
- ssh.sudo 是唯一允许使用 sudo credential 的 action；
- sudo 使用固定、可识别的调用方式；
- 密码最多注入一次；
- 非 sudo 程序的 password prompt 绝不能收到 sudo password；
- 无 sudo credential 时立即返回 typed unavailable，不进入交互等待；
- timeout/cancel 后不得把密码或残留输入留给下一条命令。

必要时在 nomi-ssh 增加 per-submission responder 或独立受控 channel；不要继续让普通
exec 共享全局 sudo responder。

6. 处理 timeout、cancel 和 disconnect。

- 不把 Future drop 当作可靠取消；
- owner/coordinator 持有执行直到终态或清理完成；
- timeout/cancel 后 shell 恢复到可复用状态，或明确 recycle/drop link；
- transport 断开返回 typed unavailable/uncertain；
- 自动重连不得重复执行 write、exec 或 sudo Effect。

7. Outcome 必须可生成主机 receipt。

write、exec、sudo outcome 至少返回：

- action/operation identity；
- bound ssh_host_id；
- succeeded/failed/uncertain 状态；
- bytes、exit code、timeout、truncated 等适用事实；
- 不含 secret 的稳定诊断；
- 足以生成 receipt 的确定性事实或摘要。

不要在 SSH crate 添加仅内存 idempotency journal，也不要伪造 durable receipt。机器 1
会在 agent_wave2_host 中接入 canonical Effect reservation/journal。

8. 安全要求。

- Debug、Display、error chain 和测试断言均不得泄漏凭据；
- read/write 必须走 SFTP，不得拼接 shell 命令；
- command 不得隐式升级权限；
- host-key changed 必须 fail-closed；
- unknown transport outcome 不得转成 success；
- 不得添加 mock/synthetic production fallback。

五、完成定义

本 lane 只有同时满足以下条件才可提交：

- 四个 action 均有 action-specific typed owner API；
- owner 使用真实 pool、host service、SFTP/shell transport；
- read/write/command/output 全部有界；
- strict write 不把 remove+rename 当原子成功；
- exec 与 sudo credential authority 已物理分离；
- timeout/cancel/disconnect 不会自动重复 Effect；
- outcome 能区分 succeeded、failed、uncertain；
- secret redaction、owner binding、host binding、host-key change 有测试；
- 旧 SSH host book、routes、pool API 保持可编译；
- changed paths 全部位于允许范围；
- Cargo.lock 没有变化。

不要修改中央 host 来完成 dispatch。最终报告必须列出机器 1 需要完成的
Wave2ApplicationHost 接线步骤。

六、最小验证

运行：

cargo fmt -p nomi-ssh -p nomifun-ssh -- --check
cargo check --locked -p nomi-ssh -p nomifun-ssh
cargo test --locked -p nomi-ssh --lib
cargo test --locked -p nomifun-ssh --lib
cargo test --locked -p nomifun-ssh --tests -- --test-threads=1
git diff --check
git status --short

如果本机有可用的真实本地 sshd，再运行现有 SSH lifecycle/fault tests。若真实 sshd、
sudo 或特定认证方式不可用，只记录第一次完整失败/缺失原因和人工复验步骤；不要盲目
重试，不要生成 PASS，不要为绕过环境写不优雅的 mock 测试。

不要运行 workspace 全量测试或 C8 Gate。

七、冲突控制

- 机器 1 不修改 nomifun-ssh 或 nomi-ssh，直到本 lane 回传或明确释放。
- 对公开 pool/transport API 优先 additive change，避免破坏旧 app consumer。
- 中央 agent_wave2_host、domain DTO、Effect journal、manifest 和 resource binding 接线
  全部由机器 1 完成。
- 不 merge 主分支后续提交，除非机器 1明确要求；出现必要修复时优先提交当前独立写集，
  再由机器 1 集成。

八、提交与 GitHub 回传

提交前确认 changed paths 全部位于独占写集。

建议提交：

git add -- crates/backend/nomifun-ssh crates/shared/nomi-ssh
git diff --cached --check
git commit -m "feat(ssh): add bounded agent capability owners"
git push -u origin rf/m2-w2-ssh-owner

禁止 force-push。

最终回传必须包含：

- base SHA；
- commit SHA；
- branch 名；
- changed paths；
- 每条验证命令及 PASS/FAIL；
- 未运行测试及原因；
- 机器 1 所需中央接线清单；
- 仍未满足的 W2-002 全局完成条件。

不要修改或声称关闭 GLOBAL-CLOSURE-TODO。
```
