# Git 效果归属与 Coding local/file push 接入（未验证）

本切片为 CAR-05 / CAR-07 的本地源码实施。当前 Coding 为
`host2-coding-loop43`，Nomi 为 `host26`。没有执行 push、构建、测试、服务调用
或数据库迁移；旧阶段的通过记录不能用于证明本切片可运行。

## 已写入源码

- 复用平台 `HostedEffectReceipts`，新增 `git` domain；用户、Session、源回合与
  admission epoch 取自正在运行的 Conversation 及 accepted delivery receipt。
  记录 exact tool operation、固定 action、输入/绑定/Snapshot/registry generation 摘要。
  模型不能提供 owner、资源目录或凭据归属。现有 Session 工具任务所有权负责阻止
  旧任务跨入下一回合；这里没有另建 Session owner。
- 新增迁移 `099_conversation_git_effects.sql`，事务内重建并复制旧表；不改 098
  校验和，保留旧记录 ID、唯一约束、精确准入、不可改身份和禁止删除的触发器。
  新增仅 Git 使用的规范工作区摘要，以及同工作区仅一条 pending 的数据库唯一索引。
  不同 Session/用户不能用另一个 Conversation 绕过同工作区的 pending push。
- Conversation Wave2 host 必须安装持久凭据端口才能派发 push。先保存 pending，
  再进行已有资源级幂等准入。新调用被资源日志拒绝时记为派发前 rejected；资源日志
  重放记为 returned 历史观察，不执行第二次 push。独立非 Conversation Wave2 owner
  继续使用它原有的资源日志，不声称其记录拥有 Conversation 归属。
- worker 成功后，先保存 Wave2 结果，再保存归属凭据，最后确认 RAII settlement。
  中途取消、未知结果或落盘失败不清除 pending。`NotApplied` 只保证目标 ref 未更新，
  不保证没有对象传输，因此保守记录为 returned acknowledged_error，而非无派发；
  这类源输入也不能自动重放。需要用户确认后用新指令处理。
- Nomi 已选 push Session 的 replay witness 不再一律拒绝；按永久源输入凭据判断。
  没有相关效果或仅派发前 rejected 的输入通过本层检查；其他效果 witness 仍可拒绝。
  清理和模型边界同时检查本地 worker 状态与永久工作区 pending。
- 共享 Engine 工具、效果上下文、清理，以及 Wave2 工作区/进程派发边界检查持久隔离。
  Coding 允许显式选入 `vcs.push`，仍受 Snapshot、Kernel、执行约束和工具级别限制。
  不是为所有 Agent 自动授权，也不是新增全局 Engine 切换入口。
- Coding 重启恢复识别 Git domain，并匹配 exact epoch/turn、operation、canonical
  action、`git_push` 名称、ToolStarted 与平台 dispatch。pending 始终隔离；多余或
  冲突凭据拒绝关闭历史。当前 exact build 无凭据表示尚未进入 Git owner 派发；
  不能用旧 build 或单个 ToolCompleted 推断效果。关闭历史不会重发 push。
- 两个官方 Engine build/digest 包含新迁移，旧绑定不会被静默解释为新恢复协议。

## 边界及仍待完成

只支持预先配置的 local/file 目标、显式单 refspec、固定 commit OID；不支持
SSH/HTTPS 凭据、force 或删除远端 ref。尚无平台 Git 网络凭据 owner。

工作区摘要是准入规范路径身份，不是操作系统文件 ID 或所有 Git worktree 的公共
repository identity。路径别名/替换、其他进程/非 Conversation owner、其他工作区
同时向相同目标 push、任意 shell 操作，不属于这个唯一索引提供的全局隔离保证。
本层边界检查也不等于跨 Session 文件修改与 push 的原子事务。

未知效果仍需人工核对，没有把“忽略隔离”包装为恢复按钮；人工解决协议、跨重启
进程树证明、网络凭据与生态其他生命周期仍待完成。构建/迁移/实际操作证据均未产生，
整体 Engine 完善目标不能据此标记完成。
