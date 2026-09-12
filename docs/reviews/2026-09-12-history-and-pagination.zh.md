# R4 审计记录：历史分页与数据库数值边界

日期：2026-09-12。续接入口：[全局审计台账](audit-progress.zh.md)。
仅完成下列子范围，不代表会话、数据库或其余 111 个模块已整体审完。

## R3-03：消息历史请求必须归属当前会话和刷新版本

审查路径：NomiChat → useMessageLstCache → IPC 消息分页 → ConversationService
的所有权校验、游标解析和结果排序 → SQLite keyset 查询。

初始 7 个真实 hook 回归中，旧实现 5 个失败、2 个通过。失败包括：

- A→B→A 后，旧 A 页被新 A 接受。
- 新会话继承旧 loadingOlder，不能开始自己的分页。
- 同会话刷新后旧分页仍占有加载状态，阻塞替代请求。
- 刷新最新窗口但保留较旧页后，盲目 prepend 把分页消息插到错误位置。
- 被新请求取代的初始加载完成后，提前清掉新请求的 loading。

测试实际挂载 useMessageLstCache 和真实 Context Provider，只替换传输/事件边界，
通过可控 Promise 指定完成顺序，不复制 hook 业务实现。
生产面板通常按 conversation ID 重新挂载，因此切换回归是 hook 契约保护，
不声称所有导航错误都已在线上页面观察到；同 key 刷新确实存在生产入口。

修复和精简：

- 以已提交的 scope 隔离会话访问，以 revision 隔离同会话刷新；layout cleanup 使旧 scope 失效。
- 最新请求和旧页请求各自持有加载归属，旧成功/旧失败都不能替当前请求清理状态。
- 在 React functional updater 内再次检查版本，覆盖 enqueue 后才切换/刷新的情况。
- 旧页和最新窗口共用现有按时间/持久身份合并，保留实时消息并消除重叠重复。
- 三个历史刷新来源合并订阅和错误处理：终态事件、重连、HTTP 轮询 settle。
- 全仓确认唯一生产调用 NomiChat 始终启用窗口分页，删除未使用的 10000 条全量模式及选项。
- 删除三个重复源字符串断言（含 reconnect 结构测试文件），保留生产者/emitter 的其他测试。

最终新增 13 个行为测试，覆盖旧成功/旧失败、重试、单飞、重叠页顺序、最新请求归属、
跨会话响应、StrictMode effect replay、Provider 存活时卸载、终态过滤、事件解绑。
早期 14 个包含不存在生产调用的模式切换用例，随全量模式一起删除，不保留测试专用兼容层。
没有为 IPC 发明物理取消接口；本次是隔离过期结果，而非宣称取消后端请求。

测试夹具调整记录：嵌套 StrictMode wrapper 未触发根 effect 重放，改用 renderHook 的
reactStrictMode 选项；零参数 reconnect listener 错传 undefined 导致一次类型检查失败，
已改为无参数调用，未用类型断言或放宽类型掩盖。最终 UI 全套是在这些修改和模式删除后重新执行。

## R4-01：分页加法和乘法先于 SQL 发生溢出

实际内存 SQLite 回归中，会话的 7 个失败用例覆盖五个 limit + 1 查询边界和
两个独立 offset 乘法；另一个 keyset 正例通过。扩展检索后，需求和素材仓储各补一个
真实列表用例，旧实现均因乘法 overflow panic 失败。以上合计 9 个失败回归，不是 9 个不同接口。

修改范围：repository/sqlite_conversation.rs、sqlite_requirement.rs、sqlite_workshop.rs，
私有共享模块 pagination.rs；测试复用既有数据库装配。

- 五处 lookahead 先转换 i64 再加 1，不改变零值默认或额外降低查询上限。
- 三类仓储共用 1-based page_offset：先归一化页码再饱和乘法。超过 SQLite i64 行数范围
  的偏移保持在上界，不能绕回较早页面；需求仍限 1..200，素材仍保留有符号参数的原有归一化。
- 用 usize 行数比较代替把结果长度缩回 u32。
- 搜索只 pop 多查的一条，删除为截断而进行的整页字符串/行克隆。

新增 8 个会话数据库回归、2 个相邻列表回归和 1 个共享数值单测。
正例校验同时间戳按 message_id 断平局，以及页间插入新消息后不漏旧页、不重复。
保留普通消息与 local-day 的可见性条件差异；未把 hidden 语义不同的 SQL 强行合并。
creation_task 的 limit 先校验 1..100，并非相同的无界加法，未为此重复改写。
本批只修数值正确性；无界大查询的资源限额、查询计划和全部服务层策略仍待进一步审查。

## 验证流水

- bun run check：通过。
- bun test --cwd ui：3304 通过、0 失败，609 文件；相对 R3 为 3294 − 3 + 13。
- bun run build:ui：通过；既有 >500 kB chunk 告警仍登记 R2-04，没有提高阈值。
- cargo test -p nomifun-db --test conversation_repository -- --test-threads=4：91 通过。
- cargo test -p nomifun-db --lib repository::sqlite_conversation::tests -- --test-threads=4：66 通过。
- 提取共享 offset 后重新执行 --lib pagination：6 通过（含三个既有分页回归）。
- 提取共享 offset 后重新执行 --test conversation_repository pagination::：8 通过。
- cargo test -p nomifun-db --lib -- repository::sqlite_requirement::tests repository::sqlite_workshop::tests --test-threads=4：46 通过。
- 模块清单核对：111 个模块 / 21 个唯一问题，无缺漏或重复；git diff --check 通过。

前两条数据库全模块结果在共享函数提取前产生；提取后的 14 个定向用例覆盖全部修改路径，
明确区分验证时间点，不沿用旧全模块结果冒充新版本全模块重跑。
未运行整个 Rust workspace、桌面打包、macOS/Linux、真实浏览器端到端或外部业务服务。

## 下一步与累计差异

R2-01b：MountUnload 在在途 acquire/release/SDK service 期间的安全准入与资源回收；
R3-01：activate 的 SDK 精确 MountLoad 绑定。两项仍未修复，继续使用原编号。
其余模块按总台账推进，不把此次分页横向检查等同于相关业务模块完成。

收尾时已进一步读取 extension-host.mjs 的完整 dispatch、sdkFor/hostCall，以及
supervisor.rs 的 Submit、响应处理和 residency 更新；尚未新增此范围测试或生产修改：

- MountUnload 没有按 Mount 的静默准入；Rust 仅在 Ack 后删 residency。
- JS 的 acquire/release 都能跨 await，卸载时删资源表不能阻止较早 acquire 随后重新登记。
- ResourceRelease 仅含 handle_id，Rust JavaScriptResourceHandle 只有 Host 实例/代际，
  没有 Mount 驻留轮次；同代卸载重载后重复 handle_id 的旧 release 风险待真实回归确认。
- SDK 服务任务虽已由代际 JoinSet 管理，尚无按 Mount 的卸载屏障。初始化时 SDK 与
  pending MountLoad 的精确绑定仍为 R3-01，不能因修卸载而直接允许任意未知 handle。

下一轮先针对上述交错建立可控 Node 回归，再确定卸载准入、失败清理和句柄复用策略；
无仓内生产 unload_mount 调用并不等于可以删除公开协议。

当前 R1–R4 累计源码/测试/依赖清单/脚本 82 文件，+1924 / -3138，净减少 1214 行，
删除 22 个旧文件（均不含审计文档）；未提交、未推送，既有 .githooks/ 不变。
