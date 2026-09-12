# R10 审计记录：Host 文件发布与入口边界

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。

## R10-01：并发发布

审查 materialize_bundled_extension_host、ImmutablePluginModule::verify_for，以及 app 的
plugin_platform/runtime foundation 调用和 adapter 的入口构造。新建并发发布回归：
16 线程同时 materialize，旧实现在第一轮有 15 个 AlreadyExists 错误、1 个成功。
旧实现还直接在最终路径写入，存在可观察半文件窗口；本次失败实测是 AlreadyExists，
不把未单独捕获的半文件窗口写成已复现。

修复将完整写入与 sync 放在同目录临时文件，persist_noclobber 非覆盖发布；竞争失败方
校验已发布文件。没有覆盖或删除既有损坏文件。临时文件由 tempfile 管理，复用 workspace
已锁定版本，将该 crate 的依赖从 dev 移至正常依赖；没有引入新第三方版本。
合并重复错误包装，并按内置文件长度加一限制缓存校验读取，不因异常缓存文件分配任意大缓冲。

保留普通文件、完整内容及 canonical 目录检查。目录仍须为应用拥有且不被不可信主体写入；
不是针对同权限恶意文件替换的沙箱，也不宣称断电后的目录项持久性。Unix 临时链接清理遵循
tempfile 的平台语义；本轮只在 Windows 实测并发结束后无临时文件残留。

## 入口与调用边界

- 插件目标、绝对 main.mjs 名称、文件存在和内容 digest 在 ensure_generation 之前验证。
- adapter 从已验证 manifest 构造入口；生产平台存储另验证包 inventory、单文件大小及符号链接。
  只核对了这些调用边界，不标记整个 artifact store 或 app 路由已审。
- 未将 Host 的单入口 digest 校验扩张为对任意 JS import 的沙箱/持续防篡改保证。
- runtime foundation 的 validate_foundation 虽带 allow(dead_code)，有真实调用，不能据此删除。

## 验证

新增 5 项测试：并发发布（4 轮各 16 线程）、损坏文件不覆盖、目录不替换、五类无效入口
不启动代际，以及发布后的真实 Hello/Mount/调用/停止链路。

- 修复前文件测试 2/1；修复后 3/0；补充后定向 5/0。
- cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-agent-kernel：115/0
  （Kernel 32、Host 单元 11 + 集成 68、adapter 2 + 2）。
- git diff --check、模块清单通过。未改进程创建边界；R9 进程边界检查证据仍适用于该边界。
- 未跑 UI、全 Rust workspace、桌面包或其他平台。

## 接续

R11 检查 Failed 状态与实际进程树清理证明之间的关系。当前 ensure_generation 只看 Running，
commit_fence_for_mount 在非 Running 时直接返回 NotResident；清理超时或启动取消后，
底层仍可能接管清理，必须确认重启和提交不会越过未完成的清理证明。
此项尚未修复；R7 的正常取消回收回归不能证明异常清理下的重启屏障。
