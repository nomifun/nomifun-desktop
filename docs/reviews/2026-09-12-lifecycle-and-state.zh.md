# R3 审计记录：Host 生命周期与 UI 状态归属

日期：2026-09-12。续接入口：[全局审计台账](audit-progress.zh.md)。
本批只完成以下子范围，不代表 111 个模块已全部审完；保留 R1/R2 未提交变更。

## R2-01c：Host 服务任务不应越过代际停止边界

审查路径：ExtensionHostServices → GenerationActor 的 JS→Host 请求处理 →
stop_generation / run 清理 → public state / commit fence → 新代际准入。

原实现使用独立 tokio::spawn，只把完成结果投递到内部 channel。Actor 不持有任务，
停止静默检查也只看 Host→JS 的 pending 请求。因此插件可以发起 SDK 请求而不等待，
在普通 invoke 已完成后仍留下运行中的 Rust 服务。

使用真实 Node Host 和可控服务 Future，四个回归在修复前均失败：

- 服务仍等待时 stop_generation 返回成功，而不是 NotQuiescent。
- 崩溃状态已经发布，但旧服务的 Drop 计数仍为 0。
- 没有普通在途请求时，悬挂服务不触发请求超时。
- 服务 panic 后，Actor 没有感知任务失败，也没有取消它的兄弟任务。

修复：用 Actor 拥有的 JoinSet 替换 detached spawn 和完成通知 channel；
停止检查涵盖服务任务；每个服务使用现有 request_timeout；task panic 进入整代失败清理。
退出时取消并等待服务任务，再发布 Stopped/Failed 或交付 fence。
停止开始后新服务请求得到失败响应，不进入 handler。第五个协议夹具强制在
HostShutdown 与 Ack 之间发请求，验证这个入口不会被普通 Host 的快速 Ack 隐藏。
公共失败原因只包含 panic 类型，不携带任意 panic payload；没有改变全局 panic hook。

五个服务测试最终均使用双工作线程 Tokio runtime，覆盖成功响应往返、拒绝忙碌停止、
崩溃取消及重启、悬挂超时、panic 取消兄弟任务和关闭后拒绝新请求。
测试文件位于 tests/extension_host/service_lifecycle.rs，复用主集成测试的装配与真实 Node。

边界与限制：仓内检索只发现默认 DenyExtensionHostServices，没有实际业务服务实现接入；
不能据此认定已经发生线上数据库写入事故。trait 文档明确 handler 必须协作让出执行权，
不能把持续写入任务再自行脱离出去；取消 Future 不会撤销此前已提交的外部副作用。
尚未覆盖 IPC 背压、服务并发限额及全部启动失败路径。

## R2-05：公共 Context 明确为本地状态

完整核对三个生产使用点：消息列表、消息加载标志、LocalImageView 的工作目录。
它们由 HOC.Wrapper 装配并经 setter 更新，没有受控 value 的生产调用。
旧 effect 的 isFirst 永远为 true，是死分支；逻辑或初值还会把 false 等合法值替换为默认值。
真实 Provider 回归在旧实现中把 false 读成 true。

- Provider 改为明确的可选 initialValue，保留只在挂载时初始化的原有状态语义。
- 默认值改用工厂，每个实例惰性创建；删除每次渲染的 JSON 克隆和不可达 effect/ref。
- 使用 React 的 Dispatch/SetStateAction，删除重复类型包装；稳定 Context 值对象。
- 三个调用点及只读消息 hook 的测试 Provider 已全部迁移。

六个测试覆盖 false / 0 / 空字符串 / null、连续函数式更新、父组件重渲染和初值变化不重置、
实例隔离、重新挂载及非 JSON 数据。没有把 Context 改成受控组件，也没有修改 HOC 的泛型装配。

## R3-02：消息批处理的可变索引污染

Messages/hooks.ts 把原始列表的索引保存在 WeakMap 中，批处理却不断原地修改此索引。
缓存键仍是旧快照，值已对应新列表。直接调用真实 drainPendingMessageUpdates，
模拟 React 对同一快照重复求值：tips 更新后 plan 移位，第二次求值把两行变成三行，旧实现失败。

删除跨求值共享的 WeakMap，索引只在每次 updater 求值内部构建一次，仍供整个批次复用。
普通成功更新本来会产生新数组，旧缓存没有把更新后的索引登记到新数组上；本次不退化为
每条消息重建完整索引。另删除从未被写入或导出的 beforeUpdateMessageListStack 及空消费循环。
生产边界回归验证多次重放结果一致、原快照未改写，既有重入队列/卸载 drain 回归继续通过。
该用例验证真实 updater 的重放契约，不声称已覆盖所有 React 并发渲染或浏览器端到端场景。

## 最终验证

- bun run check：通过，包含 TypeScript、资源和架构边界检查。
- bun test --cwd ui：3294 通过、0 失败，609 文件；比 R2 增加 7 个真实生产边界回归。
- bun run build:ui：通过；仍有 >500 kB chunk 告警，R2-04 保留，不抬高阈值。
- cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-agent-kernel：59 通过，0 失败。
- Context / 消息批处理 / 只读 hook 的定向测试：19 通过。
- 模块清单核对与 git diff --check：通过。清单仍为 111 个 Rust/UI 模块边界，其他跨切面另表登记。
- 五个双线程服务测试最终定向复验通过，包括公共失败原因不包含原始 panic payload。
- 清单脚本新增问题编号重复检查：20 个唯一编号；正常清单通过，重复编号及模块漏项/重复/过期四个内存负例均失败，未在磁盘写入假记录。

UI 最终命令在消息缓存删除后重新执行；Rust 跨层命令在公共错误信息收敛后重新执行，
没有沿用较早版本的绿色结果。未运行整个 Rust workspace、桌面打包、macOS/Linux 或外部业务服务。
仓库默认禁用全局 rustfmt，本批只对新增 Rust 测试显式格式化，避免改写不相关文件。

## 未完成和下一检查点

1. R3-03：Messages/hooks.ts 的 useMessageLstCache / loadOlder。先用真实 hook 的受控异步响应
   复现 A→B→A、同 key 刷新、分页 loading 归属；当前只确认它缺少 newest load 的序列检查，尚未定论。
2. R2-01b：MountUnload 删除资源表但未调用 release；还需协调在途 acquire、release 和 Host 服务。
   仓内无 unload_mount 生产调用，不据此删除公开 API，也未提交只补 release 而不处理竞态的局部修补。
3. R3-01：activate 已收到 SDK，但 Rust 只有 MountLoad Ack 后才登记服务所查的 Mount；
   需复现初始化阶段 SDK 调用，再按精确 MountLoad 上下文和失败清理设计绑定。
4. 其余模块按总台账推进；Browser、Knowledge、Canvas 等仍未完成深审。

未提交、未推送，既有 .githooks/ 保持不变。

R1–R3 累计源码、测试、依赖清单和校验脚本共 71 个文件，+1308 / -2907，
净减少 1599 行（不含审计文档）；删除 21 个旧文件。本批增加回归覆盖，因此净减行数比 R2 少，
不以删除必要测试来追求更低行数。
