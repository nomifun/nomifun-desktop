# R7 审计记录：Host 启动、取消与诊断收尾

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。保留 R1–R6 改动。

## R7-01：启动诊断也必须有背压、保密和生命周期约束

核对 ensure_generation/spawn_generation、Hello 校验、startup_failure_detail、Actor 退出和
drain_stderr；向下核对 nomi-process-runtime 的 ManagedChildProcess::shutdown/Drop 及接管路径，
没有重写底层清理机制，也没有把该共享 crate 整体标为已审。

真实 Node 修复前结果：4 失败、1 通过。

- 握手前写 2 MiB stderr 并等待写回调的合法 Host，被误判 Hello 超时。
- 失败启动把 stderr 的 fixture-startup-secret 原文带回调用方。
- serde 的 Hello 解析错误把 fixture-hello-secret 输入值带回调用方。
- 启动失败返回错误，但公开订阅状态仍是 Stopped，未发布失败代际。
- 取消 Hello 等待后，现有 ManagedChildProcess 的 Drop 已能回收根和子进程；通过
  夹具就绪文件取得确切 PID，并使用 signal 0 只读探测确认回收，早于夹具 5 秒兜底退出。
  因此这不是“取消泄漏”的修复；保留机制并增加回归保护。

首次测试编译错误是测试端把消耗 self 的 output 接在返回 &mut self 的 args 后，修正后才取得
上述行为失败证据，不把编译失败算作产品缺陷。

## 实现

- Hello 等待前即启动 stderr 排水，只保留字节/换行计数；没有缓冲任意日志正文。
- 诊断任务由 JoinSet 持有，启动 Future 被取消时自动中止。进程仍由 ManagedChildProcess
  单独持有，取消后走已有 Drop 清理接管，避免重复终止权限和额外 detached task。
- 删除 startup_failure_detail 原文拼接和多个重复失败清理分支。握手解析/契约/绑定失败使用
  固定安全原因；stderr 只附计数，不把解析器、Node 或插件的任意输入带回公开状态。
- 启动与 Actor 退出共用 cleanup_process；进程 shutdown 和 stderr 收尾共享同一截止时间。
  stderr 写端一直不关闭时，中止并 join 诊断任务，不无限阻塞终态；进程清理错误不再被吞掉。
  清理超时仍保留底层清理权限交给 Drop 接管，不声称超时等于物理进程树已清空。
- 启动错误发布带实际 generation 的 Failed，之后仍可按原规则需求重启，不重放失败请求。
- 原 Hello 对 runtime、role、generation、process 以及契约的精确绑定均保留并增加负例。

## 验证与失败尝试

新增 9 项：真实 Node 6 项、诊断单元 3 项；4 项有红→绿证据，取消清理原本即通过。
定向测试 9/0；覆盖实际回收、错误保密、重试、5 类 Hello 身份错误、排水统计、开放写端超时和任务取消。

第一次跨层：Kernel 32/0、Host 单元 9/0、Host 集成 53/1；唯一失败为取消测试在
读取 started.json 前的 2 秒准备等待到期，未进入取消阶段。修正测试同步：该模式单独给予
10 秒启动准备/Hello 等待，仍由就绪文件触发取消，取消后的回收期限保持 2 秒且早于 5 秒兜底退出。
没有放宽生产超时，也不拿失败运行当作最终通过。

最终 cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-agent-kernel：99/0
（Kernel 32、Host 单元 9 + 集成 54、adapter 单元 2 + 集成 2）。进程边界和 Node 夹具语法检查通过。
未改 UI，未重复 UI 全套；未运行全部 Rust workspace、桌面打包或 macOS/Linux。

## 接续范围

下一步检查运行中的协议入口：服务请求是否精确绑定活动 role，重复 request_id 是否会执行
两次服务，异常响应是否导致 pending 过早移除、错误原因是否引用不可信内容。
全局在途请求/服务配额、清理证明失败后的重启策略以及 Host 其余导出文件/路径边界仍未标完成。
未提交、未推送；既有 .githooks/ 保持不变。
