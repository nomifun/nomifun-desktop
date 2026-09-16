# Git 工作任务生命周期与执行目标固定

日期：2026-09-14。分支：`rf/agent-capability-platform-v2`。
源码实现与局部 rustfmt，未构建、未测试、未执行服务或 Git push，未 commit/push。
当前 Coding `host2-coding-loop42`、Nomi `host25`；不是 Git 全能力或 CAR 完成声明。

## 修复的执行边界

原 push owner 超时后丢掉 blocking worker 的等待句柄，同时释放 async push 锁；
本地线程可能仍在执行，而调用方已经拿到超时错误。现在：

- worker 的共享 completion 在第一次 await 前由平台 owner 保存，副本等待者取消
  不消耗清理凭据；仓库 push 锁转移到 worker，保持到真实退出。
- 请求路径检查和 Git 工作进入 blocking worker，超时时间包含该工作。超时请求取消
  后继续等待退出；外围引擎仍可按自己的等待期限隔离会话，但不能宣称线程已终止。
- worker panic 或调用方消失会置未知状态。即便随后任务返回，未知状态也不自动解除，
  不根据晚到结果重复派发。
- Wave2 的 RAII settlement guard 跨越 owner 返回、结果序列化与永久效果回执写入。
  任一取消、异常或落盘失败都保留未知状态；只有回执确认后才解除该次 pending 标记。
  清理区分 worker 退出、永久记录已确认和远端效果已知三个事实。

这没有承诺原生阻塞调用一定在固定时长内退出。进程关闭及未知效果的人工作业仍需
既有隔离流程；不能用一个 timeout 冒充清理完成。

## 固定提交与远端目标

原路径在推送前再次 find_remote，并使用原 refspec 中可变的 HEAD/分支名；配置或
引用变化可能让实际发送与之前观察不同。现在用确定的 commit OID 构造单一 refspec，
创建指向已解析 canonical local/file 路径的匿名 remote，不再次选择命名远端。

进一步阅读所依赖的 libgit2 源码发现：匿名 remote 仍会应用 insteadOf/pushInsteadOf。
因此在真正 push 前再次检查该 remote 的有效 push URL，要求仍指向本次准入的同一本地
目标；改写到网络协议或另一仓库明确拒绝。没有读取/输出用户的 Git 配置或凭据。

这些检查不是对文件系统路径替换的原子隔离，不阻止其他进程修改仓库；本次没有新增
force push、任意 URL、用户名密码参数或机器凭据继承。

## 平台与 Engine 接线

共享 EngineKernelSession 在有工作区/进程能力时捕获 canonical 路径身份，工具派发、
新回合和宿主效果上下文检查同工作区 Git owner 状态；清理加入 Git worker completion。
清理不重新 canonicalize 已删除、替换或重命名的路径，因此不会因为路径变化丢掉
原 owner。没有相关能力的 Engine 不额外要求工作区路径解析。

Nomi 已选 `vcs.push` 且未被 Session 约束排除时，装配 WorkspaceGitWitness，加入
现有 EngineEffectScope 与必需模型边界 context。没有新增 Session 或引擎生命周期
管理器。Nomi 未选择 push 的普通会话不额外禁用自动重放。

## 尚未完成的接入

现有 Wave2 Git 效果记录按 resource/capability/key 归档，不能证明“原始用户输入可以
安全重放”。本次没有把这个缺口用内存空表或普通 ToolCompleted 代替：

- Coding 的 `vcs.push` 仍不在 admission allowlist；需要用户源输入、turn、epoch、
  exact Session 的持久效果归属及跨重启恢复判断之后再开放。
- Nomi 已选 push 的 Session 暂时拒绝自动 retry/edit-resubmit，即使该条输入可能
  尚未用到 push。此保守范围应在源输入归属回执完成后收窄，不能宣称体验已完整。
- HTTPS/SSH 需要应用拥有的凭据与网络执行 owner；当前 git2 build 禁用对应默认功能。
  local/file 基础改进不代表网络 Git push 已实现。
- 共享内存 owner 状态检查不是跨 Session/Fork 的原子事务，也不是跨重启状态恢复。
  其他原生 Nomi 工具、独立消费者和外部文件系统活动的完整协调仍待补齐。

## 源码参考

本地 Codex `codex-rs/core/src/unified_exec/process.rs` 的 OutputTaskGuard、持有的
JoinHandle、统一进程状态与 Drop 清理，以及 `process_manager.rs` 的
InitialExecCommandGuard，帮助明确取消时的状态归属。本次没有复制它的进程 owner，
也不把其 terminate/Drop 行为当成 Git 原生线程可被强制杀死的证据。

另读取当前依赖 libgit2 1.9.4 的 `remote.c`、`push.c`：匿名 remote 会应用 URL
改写，push 源表达式走 revparse。这是实现依据，不是运行验证。上述任何源码阅读或
格式化均不计为本切片的构建、测试、平台行为或质量验收证据。
