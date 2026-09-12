# R5 审计记录：Mount 生命周期与资源租约

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。
保留 R1–R4 改动，仅完成下面的 Host/资源/SDK 子范围，未把整个 Host 标记完成。

## 实际审查路径与失败证据

核对 supervisor 的 load/unload/invoke/context/acquire/release 公共入口、Actor Submit、
pending 和服务 JoinSet、响应校验、Mount residency，以及 extension-host.mjs 的完整 dispatch。
继续核对 Kernel adapter 对 JavaScriptResourceHandle 的传递：它保留完整句柄，不解析或重建 ID。
仓内仍未发现 unload_mount 的生产调用或实际业务 ExtensionHostServices 接入；这是公开边界的
可复现缺陷修复，不声称已观察到线上事故，也不据此删除公开 API。

第一组真实 Node 回归：新增 9 项全部在修复前失败，原有 5 个服务生命周期用例通过。

- 卸载两个资源后，没有任何 release 回调记录。
- 同 Host 代际卸载重载、同 Mount 释放重获两种情况下，旧句柄都释放了新资源。
- 并行 release 把回调执行了两次。
- 在途资源获取、独立 SDK 服务都未阻止卸载成功。
- activate 使用 SDK 时被当成未知 handle，导致整个 Host 失败。
- 旧 SDK 闭包在重载后绑定到了新 Mount。
- 卸载时的资源清理失败没有被处理，因为原实现根本没有调用 release。

相邻资源适配又补 3 项，在该路径修复前全部失败：非法/重复获取结果未释放，
release 复制后丢失原对象接收者，拒绝清理失败后继续保留运行中的代际。
本批合计 12 项有红→绿证据；最终新增 18 项，不声称其余 6 项都跑过原版失败。

## R2-01b：按 Mount 静默卸载，按获取轮次持有资源

- JS Mount 明确 loading / active / unloading / closed；invoke、context、acquire、release
  共用请求计数，SDK 单独跟踪。忙碌 Mount 拒绝卸载；开始卸载后拒绝新工作。
- Rust Actor 同时按 Mount 检查 pending，覆盖 JS 回调完成但响应尚未交付的窗口。
  Actor 记录资源租约所属 Mount，使 release 同样参与屏障；不同 Mount 不互相阻塞。
- 删除公共 load_mount 的重复 residency 快路径，以 Actor 的串行准入为准；正在卸载时
  不会再被旧驻留快照误报加载成功。既有并发相同初始化合并仍保留。
- 插件本地资源 ID 不再直接充当对外释放权限。Host 为每次获取分配不重复的 opaque ID，
  即使同代重载或复用 binding，旧 lease 也不会命中新资源；不存在的 lease 释放是幂等空操作。
- 并行 release 共用同一个 Promise；已完成后移除双方索引。失败的普通 release 可按原语义重试，
  不声称任意部分失败回调的外部副作用都能做到 exactly-once。
- 卸载依次释放所有资源，一个失败不跳过其他资源，然后 deactivate；等待清理期间发起的
  SDK 请求收尾，最后 Ack。出现部分清理失败时通过现有整代失败路径关闭 Host，
  不把部分销毁后的插件恢复为 active，也不把任意清理异常文本带入公共失败原因。

## R3-01：初始化 SDK 只绑定精确预留上下文

Rust 服务查找在已驻留 Mount 之外，仅允许匹配 pending MountLoad 的确切 mount_handle_id，
复用此前唯一性预留；不是放行未知 handle。真实服务记录并断言完整配置/目录/目标上下文相等。

JS 在激活前登记生命周期对象，在失败激活或卸载完成时关闭该对象的 SDK，再等待已发出的
服务请求收尾，之后才结束 MountLoad/MountUnload。旧 SDK 对象不能因同名 handle 重用而复活。
失败激活遗留的挂起服务仍受既有请求超时/代际取消约束，没有无界 await。

增加未知 handle 的协议负例、失败激活等待服务后可在同代重试、悬挂服务超时取消、
卸载中拒绝调用/加载但允许清理 SDK 和其他 Mount、deactivate 发起不等待的服务仍必须收尾。
两个负等待断言使用已进入服务的通知和 30 ms 超时，不依靠长时间 sleep 制造竞争。

## R5-01：拒绝获取结果也必须接管清理

JS 拒绝非法/重复的本地 ID 时释放本次返回的资源，不触碰此前已注册的 owner。
有效与拒绝路径都将 release 绑定到原始返回对象，保留方法的 this。
拒绝路径若连清理也失败，同样进入整代失败；共享内部 CleanupError 分类，避免插件随意
抛出的带 code 对象被误当作 Host 的清理失败标记。

## 最终验证与范围限制

- cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-agent-kernel：77/0。
  其中 Host 41、Kernel 32、adapter 单元 2、真实跨层集成 2。
- 资源拒绝/接收者定向回归：修复前 3 失败，修复后 3 通过。
- Node --check：生产 Host、main 插件夹具、服务协议夹具均通过。
- 关键生命周期定向复验：mount_lifecycle + service_lifecycle，23/0。
- bun run check:process-runtime-boundary：通过（首次 10 秒工具超时中断，确认无遗留进程后重跑完成）。
- git diff --check、模块清单核对通过：111 模块、22 个唯一问题，无缺项或重复。

只显式格式化本审计新建的两个 Rust 生命周期测试文件，未运行全局 rustfmt。
本批未改 UI，因此未重复 UI 全套；R4 的 UI 3304/0 仍是该版本的最近证据。
未运行整个 Rust workspace、桌面包、macOS/Linux 或外部业务服务；未来接入的 handler 仍须
遵守协作取消约定，已提交的外部副作用不会因为关闭 Future 而自动回滚。

## 后续检查点

继续 Host IPC 与启动失败：read_json_line 已使用 take 限制入站帧；不能把 read_until
本身当作无界读取缺陷。下一处重点是 write_json_line：Actor 直接 await 管道写入，
慢读/停止读取的 Node 可能阻塞 watchdog 和关闭消息；出站帧上限和服务响应写入也需检查。
尚未为该问题写回归或修改生产代码，不把猜测标为修复完成。

R1–R5 当前累计源码/测试/依赖清单/脚本 84 文件，+2706 / -3241，净减少 535 行，
删除 22 个旧文件，不含文档。本批增加必要并发回归，不用删测试来追求行数下降。
未提交、未推送，原有 .githooks/ 不变。
