# R6 审计记录：Host IPC 背压与出站边界

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。保留 R1–R5 未提交改动。
本批沿用唯一问题 R6-01，不重复登记 R5 已解决的 Mount/资源/SDK 生命周期问题。

## 证据与范围

沿 GenerationHandle 请求准入 → Actor Submit/Stop → 服务 JoinSet 完成 → stdin 写入
→ watchdog → 代际清理逐段检查。原入站 read_json_line 已由 take 限制读取大小，未将其误报为无界。

新增真实 Node 夹具：合法 Hello 和 MountLoad Ack 后暂停 stdin，进程继续存活；4 秒后兜底退出，
只用于避免旧实现失败时遗留测试进程，断言必须在更短截止时间内完成。4 个测试在修改生产代码前全部失败：

- 合法上限内的 4 MiB 请求填满管道后，300 ms watchdog 没有生效，1.5 秒外层等待超时。
- 同样的背压阻塞了 stop_generation 的准入，500 ms 内无法返回 NotQuiescent。
- 独立 SDK 服务已经执行完成，但响应写入被阻塞，且没有待处理的正向请求为其兜底超时。
- 本地超大请求未在出站拒绝，回显响应触发入站上限并关闭健康 Host，无法继续小请求。

这修复的是可复现的公共运行时缺陷，不声称已观察到线上事故。

## R6-01 的实现与设计取舍

- 删除 Actor 三处直接 await write_json_line 及该旧函数。新增私有 outbound 模块，统一请求、
  关闭和服务响应的编码/排队/分段写入；不是另启独立后台任务。队列与 stdin 仍归同一个 Actor 所有。
- 写入作为 tokio::select 的独立分支，每步最多进行一次 write，完成帧后 flush。偏移保存在队列帧中，
  其他事件抢占后不会从头重发，也不会交叉不同帧。失败则沿已有代际清理路径终止进程。
- 队列按 command_queue_capacity 有界，含正在写入的帧；满时只向未发送的普通请求返回 QueueFull。
  服务结果不可静默丢弃，因此服务响应编码失败/队列满走代际失败；停止前也检查未发完的响应。
- max_frame_bytes 同时限制两个方向，含换行和 JSON 转义。通过受限 Write 在序列化过程中拒绝超限，
  不先 to_vec 分配任意大的完整副本再检查；被拒绝帧从未写入管道，不污染后续协议。
- 正向请求原截止时间覆盖准入与出站排队。命令队列 reserve 及 admission 锁的等待有界，
  已在命令队列中过期的普通请求不再下发。停止准入也有界；失败清理时先关闭命令接收端再交付失败。
- SDK 服务完成后携带原截止时间，写入不重新获得完整 request_timeout。watchdog 检查整个出站队列，
  包括排在长截止时间帧后面的短截止时间帧；write 前也拒绝已过期帧，避免抢在下一 tick 前下发。
- 不采用“给原 write_all 加 timeout”的方案：它仍使 Actor 在等待期间无法处理更早的截止时间与停止请求。
  也不新增 writer task，避免引入额外的任务关闭、错误通道与排队同步状态。

## 回归与验证

本批新增 13 项测试：7 项真实 Node 集成、6 项传输/准入单元测试。
仅上述 4 项有修改前失败证据，其余为修复设计的补充边界，不混报红→绿数量。

补充覆盖：合法 4 MiB 帧真实往返、队列溢出不立即终止健康代际、超大服务响应安全失败；
小容量 duplex 下部分写入取消后正确续传、两帧顺序与完整字节一致、换行/转义大小边界、
关闭管道、已过期帧不发送，以及准入超时后不会稍后偷偷入队。

- 首次修复后 4/0；增加队列/服务边界后真实 IPC 集成 6/0。
- 中间跨层 Host/adapter/Kernel 88/0（早于最后增加合法大帧和过期帧两个测试，不充当最终证据）。
- 最终 cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-agent-kernel：90/0。
  Kernel 32，Host 单元 6 + 集成 48，adapter 单元 2 + 集成 2。
- 定向复验 cargo test -p nomifun-js-host --test extension_host ipc_backpressure -- --test-threads=2：7/0。
- bun run check:process-runtime-boundary：通过；git diff --check：通过；模块清单 111 / 唯一问题 23，无缺项或重复。
- 只显式格式化本批新增的 Rust 文件，未运行全局 rustfmt；JS 夹具 Node 语法检查通过。

未改 UI，因此不重复 UI 全量，R4 的 UI 3304/0 仍是对应版本最近证据。
未运行全 Rust workspace、桌面包、其他操作系统或真实外部业务服务。

## 下一检查点与未覆盖范围

本批不是整个 Host 的审计完成。下一步检查 spawn_generation/ensure_generation 的启动失败与取消，
包含 Hello 等待、stderr 排水和错误暴露、ManagedChildProcess 的取消所有权，以及退出后的终态发布。
协议恶意响应/服务请求重号、全局在途请求与服务任务配额仍待审；出站队列有界不等于整个 Host 内存已全面有界。
不把这些待审范围当作已确认缺陷，也不因已有跨层测试通过而标记整个模块完成。

没有删除用户文件、提交或推送；既有 .githooks/ 保持不变。

R1–R6 累计（不含文档）：91 个源码/测试/依赖清单/脚本文件，+3316 / -3282，净增加 34 行；
累计删除旧文件仍为 22。本轮新增的回归和有界传输实现使总行数上升，不能再沿用 R5 的净减少 535 行。
这是包括测试在内的实际工作树差异，不把拆文件或删除测试算作优化成果。
