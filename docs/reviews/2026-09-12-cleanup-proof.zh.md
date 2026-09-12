# R11 审计记录：清理证明与安全准入

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。

## R11-01：非 Running 不等于已完成清理

在持有真实存活 Node 进程清理凭据的单元夹具中，分别模拟启动已取消（无 current handle）
和已发布 Failed 的状态。先加入凭据存储及底层只读 accessor、未实施屏障，两个回归均失败：

- 启动取消状态越过 restart、mount fence 和 stop 的 NotResident 路径。
- Failed 状态越过 restart 和 mount fence；原 stop 已拒绝，不将它写为新修复。

这是精确状态注入 + 真实底层进程/凭据，不宣称在系统中注入了实际 Job 清理故障。

修复复用 nomi-process-runtime 的 ChildProcessCleanup 观察凭据：

- ManagedChildProcess 只新增 cleanup_receipt accessor；不改终止、Drop relay、平台 reaper 或重试算法。
- spawn 后第一次 await 之前同步将凭据保存在 SupervisorState。调用方取消启动也不会丢失屏障。
- 非 Running 的重启、mount fence 和无 current 的 stop 先有界等待同一凭据。
  超时/失败保留凭据且不消耗新 generation；成功后才清除。没有按 PID 重建身份或创建第二套终止权限。
- Running 的正常请求仍复用原 Actor；Failed 发布仍在服务取消之后，避免把进程 proof 与服务任务归属混淆。
- 物理清理未确认时不返回 NotResident；底层随后完成时可重试，不永久把正常取消当成不可恢复错误。

## R11-02：无调用的旧入口

全仓引用检查确认 HelloTimeout 变体无构造/匹配，实际超时统一走有安全诊断的 HelloRejected；
host_failure_response 及其专用常量也无调用。删除这三个旧定义，未改正常请求拒绝的语义。

## 验证

- 两个准入回归修复前 0/2，修复后 2/0；包含凭据保留、不增加代号、清理后可退役凭据。
- 扩展 R7 真实启动取消测试：确认根/子进程清理后，同一 supervisor 成功启动 generation 2。
- 新增底层凭据回归：观察不杀死存活进程，显式 shutdown 和 Drop 接管后都能等到 proof。
- 最终 Host/adapter/Kernel：117/0（32、13 + 68、2 + 2）。
- 底层 child_process_builder 8/0、architecture_contract 15/0；进程边界检查通过。
- 未跑全部共享运行时测试、全 Rust workspace、桌面包、macOS/Linux 或 UI。

## 接续

R12 转入 app/router/plugin_runtime_host.rs 的 Runtime 绑定层。发现 stop_for_runtime_switch
先 take 再 await，调用取消可能丢失绑定；Stopped 分支也不能替代底层 proof。
同 Runtime demand 会保留 supervisor（不绕过 R11），但 Runtime 改变的替换分支仍用 process_count 判断。
这些调用方问题独立登记 R12，不将本批标为全局屏障已完成。Host 在途 Mount 的提交判断仍须继续检查。
