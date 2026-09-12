# R17 审计记录：代理辅助进程生命周期

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。

## R17-01：进程退出不是 stdout EOF 或后代清理证明

旧实现先轮询父进程退出，再无超时地 read_to_end。真实测试程序派生一个继承
stdout 的后代后退出，200ms 探测实际等待约 1.535s，回归在旧实现失败。

删除独立的原始进程轮询/kill/read 路径。macOS/Linux 探测命令统一使用已有
ChildProcessBuilder 和 ManagedChildProcess：stdout 读取与父进程等待共享预算，
输出限制 64KiB；非零退出、超限或超时不接受输出。随后用同一所有者执行有界
250ms shutdown，取消则交回既有 Drop relay/platform reaper，不新增 PID 清理机制。

同步 API 使用独立线程上的 current-thread Tokio runtime，因此也能从已有 Tokio
runtime 调用。预算包含读取/等待，另有清理宽限及底层 Drop fallback；不是整个
同步函数严格在 200ms 返回的承诺。Windows 生产仍直接读取注册表。

## 验证与限制

- 新增 3 项行为测试：后代持管道、成功/失败/超限输出、已有 runtime 内调用；均通过。
- 后代持管道测试记录 PID 与 platform_start_key，确认同一后代在 1s 内消失，早于
  夹具的 1.5s 自行退出后备；不使用裸 PID 杀进程。
- 最终 cargo test -p nomifun-net：50/0，1 个专供子进程启动的夹具显式 ignored。
- bun scripts/check-process-runtime-boundary.mjs 通过；清单 111 模块 / 38 问题。
- Cargo.lock 只增加已有 nomi-process-runtime、tempfile 依赖边；没有新第三方版本。
- 没有运行 macOS/Linux 原生命令和对应平台清理测试，不将 Windows 结果当作平台验证。
- 代理缓存并发、精确凭据脱敏和共享客户端回退仍待接续，网络模块未完成。
