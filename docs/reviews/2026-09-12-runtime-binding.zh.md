# R12 审计记录：Runtime 绑定取消安全

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。

## R12-01：停止期间不能丢失绑定

两个真实 Node 回归在旧实现失败：停止 future 取消后绑定消失，停止尚未完成时并发
commit_fence 返回 NotResident；另一个回归中，Hello 失败的进程已完成清理，绑定却因
Failed 状态永久拒绝退役。

修复保持绑定 mutex 直到停止与清理确认完成，仅在成功后置空。取消或错误保留原绑定，
不再 take→await→错误回填，也不再将 process_count 当成物理清理证明。
Host 新增 confirm_stopped，只观察已有清理凭据；Running 拒绝，非 Running 等待真实
清理完成。Runtime 改变时替换旧 supervisor 也使用此确认入口。

该接口不永久关闭准入。已核对 Runtime coordinator 的写租约与 App participant 调用，
停止调用方仍负责排除新的需求；没有另建终止机制或绕过进程运行时。

## 验证与限制

- App 定向 2/0，两项均有修复前失败证据。
- 扩展 Host 两个清理证明测试，覆盖新增公开确认入口的未完成/已完成凭据；
  新增 Running 确认拒绝且不杀进程的回归。
- 最终 Host/adapter/Kernel 118/0（32、14 + 68、2 + 2）。
- 第一版取消夹具错误地期待 HostShutdown 调用插件 deactivate，该次失败不算缺陷证据；
  改用明确延迟 HostShutdown Ack 的协议夹具后，才得到有效红→绿结果。
- Runtime 改变分支通过源码追踪和同一公开证明接口回归覆盖，未伪造跨 Runtime 真实切换。
- 未重复无改动的 UI、底层进程测试或全 Rust workspace；未验证其他操作系统。

## 接续

R13 检查在途 Mount 的提交判定，以及 auto_apply_commit_permit 已持有 Runtime 读租约后
再次申请读租约的等待环。插件提交之后的准入/持久化契约还需继续全链路审计。
