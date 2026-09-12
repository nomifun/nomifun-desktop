# R13 审计记录：在途 Mount 查询与 Runtime 租约

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。

## R13-01：查询必须包含已准入但未驻留的 Mount

真实 bundled Host 在 activate 中等待 SDK 服务时，旧查询只读驻留表并返回 NotResident。
回归先失败后通过；同时覆盖成功激活、失败激活清理、其他未加载 Mount 不被误阻塞。

查询改为有界 Actor 命令。提交者的 admission 读锁先排空到 FIFO，查询再同时读取 Actor
的 pending 与 resident 状态；不另维护第二份在途集合。新增命令在退出时一并交付错误，
排队和回复等待共用截止时间，取消/超时释放 admission。

注意：这是针对已准入工作的一次性查询，不是禁止今后加载的持久锁。
后续仍须深审 PluginService 提交→注册发布和 Kernel dispatch 的全程准入契约；本批不宣称
所有配置/替换事务已经获得跨持久化屏障。

## R13-02：已有读租约时不能重复申请

auto_apply_commit_permit 持有 Runtime 读租约后，内部再次申请同一公平 RwLock 的读租约。
排队写租约等待外层读租约，内层读租约又等待写租约，形成等待环。
测试使用真实 RuntimeAuthority、只读 selection store，先 poll 写请求进入排队；旧实现
查询超时，新实现通过。不是仅对锁算法做脱离生产方法的模拟。

自动应用显式传入已有租约，并与普通 commit 查询共用 Runtime 精确匹配与停止路径。
删除重复的租约申请、Runtime 匹配及 Host 停止分支。Busy 仍映射为 None。

## 验证

- 两个新行为回归均有有效红→绿证据。
- 两个新单元回归覆盖查询等待已有提交者、队列满、Actor 未回复的超时和取消释放。
- 最终 App 绑定定向 3/0；Host/adapter/Kernel 121/0（32、16 + 69、2 + 2）。
- 清单 111 模块 / 32 唯一问题，差异空白检查通过。
- 本批未修改底层进程管理或 UI，未运行全 Rust workspace、桌面包或其他平台。

## 接续

转入公共 nomi-redact 完整小模块审计。Host/PluginService 的提交后准入及 JS 其他入口
仍在台账中保留待审，不以本批测试代替其审计。
