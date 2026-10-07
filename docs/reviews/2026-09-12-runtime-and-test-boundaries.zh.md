# R2 审计记录：运行时并发与测试边界

日期：2026-09-12。续接总入口：[逐模块进度台账](audit-progress.zh.md)。
本批不是全仓审计完成声明；未审模块和只读过部分路径的模块均保留待办。

## 已完成范围

| 问题 | 生产位置 | 结论与证据 |
| --- | --- | --- |
| R2-01a | js-host/supervisor.rs | 并发首次 invoke_demand 会同时进入 load_mount；原 Actor 把完全相同的在途加载当作身份冲突。真实 Node 测试先失败后通过 |
| R2-01a | 同上 | 由串行 Actor 复核驻留状态、合并完全相同的 MountLoad。初始化成功/拒绝/Host 超时向所有等待者交付；不同 context 仍冲突；不同 Mount 不加全局加载锁；普通请求不增加结果复制 |
| R2-01a | js-kernel-adapter/src/lib.rs | 核对 ResourceHandle 的 binding/kind/id 及 Host 实例/代际传递；删除构造后立即丢弃的 identity。其余注册映射未全部深审 |
| R2-02 | readOnlyConversation.sideEffects.test.ts | 删除测试里的独立谓词，挂载真实 useNomiMessage，拦截 transport 边界；验证两种模式的指标显示/写回、增量/替换缓冲、终态后处理 |
| R2-02 | NomiSendBox.tsx / steerOrQueue.ts | 提取实际使用的 steering 交付与失败入队边界，测试直接调用该实现；覆盖成功、延迟失败、附件快照、多类错误及队列错误传播 |
| R2-06 | ui/package.json / types / 14 个测试导入 | 统一 bun:test；删除 Vitest 的手写声明；Bun 声明缩减为官方 test API 导入及现有 equality 契约兼容，减少重复类型维护 |

调用关系核对到：Kernel 注册的 ResourceProviderFactory → NodeResourceProxy →
ExtensionHostDemandPort → Supervisor / GenerationActor → JavaScript Host。
Supervisor 另通过 nomi-process-runtime 管理子进程。仅阅读 process-runtime 的导出/关闭接口，
未审计全部 OS 实现，不能给它标记完成。

## 测试类型的取舍与失败记录

1. 原手写 bun:test 声明缺少 mock/spy API，新增测试运行通过但 typecheck 失败。
2. 尝试完整 @types/bun 后，Bun 的 fetch.preconnect 扩展污染了 renderer 的浏览器全局类型；
   同时官方 equality 将品牌 ID 与原始 wire 字面量的比较当作类型错误。未修改业务类型、
   未加入 fixture 的双重断言、未放宽 tsconfig 来绕过问题。
3. 最终直接依赖与本机 runner 相同的 bun-types 1.3.14，只导入 bun-types/test。
   toBe/toEqual/toStrictEqual 保持此前接受 unknown 期望值的运行时比较契约；其他测试 API 使用官方类型，
   没有复制 mock/spy 的类型实现。Bun 的非测试全局不进入浏览器编译。
4. Bun 移除临时 @types/bun 依赖后仍残留一个 ui/node_modules/@types/bun junction；
   检查其完整目标后只移除了本批创建的链接，缓存包内容保留，需要时可重新安装恢复。
5. 只读 hook 首次测试夹具用了普通 404 对象，真实错误边界拒绝它；改用 BackendHttpError 后通过。
   这属于夹具修正，不是放宽产品错误判定。

## 验证

- `cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-agent-kernel`：54 通过，0 失败/忽略。
- 三个新增并发回归再连续执行 3 轮：每轮 3 通过；包括 rejected/hanging activation 的等待者交付和重试。
- `bun test --cwd ui`：3287 通过、0 失败，608 文件。数量比 R1 少 1，是将 5 个测试副本用例替换为 4 个真实 hook 场景，并非关闭测试。
- `bun run build:ui`：通过。仍有超过 500 kB 的 minified chunk 告警，最大约 2.24 MB；R2-04 保留待审。
- `bun run check`：通过（类型、资源及架构边界检查）。
- `bun scripts/check-review-inventory.mjs`：111 个模块边界一致。内存注入漏项、重复项、过期项均被拒绝。
- `git diff --check`：通过。

R2 没有跑全 Rust workspace、桌面安装包、多平台测试或真实外部服务。
steerOrQueue 测试验证交付/入队函数，不替代完整 SendBox DOM 交互或后端幂等契约测试。

## 未完成与下一步

- R2-01b：MountUnload 清理资源表但不调用 release；当前没有仓内生产调用，先核对契约、
  在途 acquire 和未来/外部使用，不凭静态不可达直接删除公开 API。
- R2-01c：Host services.handle 的独立任务与 stop_generation 的静默屏障。
  从 fire-and-forget 服务调用建立复现，检查任务能否跨越进程停止后的提交边界。
- R2-05：通用 createContext 的 isFirst 分支永不解除。已核对三个生产使用点由 HOC 装配，
  当前通过 setter 更新；应先明确 prop 的初始化/受控语义，避免修复表面分支时重置消息列表。
- Browser、Knowledge、Conversation、Canvas 等大模块仍按总台账继续，不能用本批绿色回归代替深审。

R1 + R2 当前源码/测试/依赖清单累计 64 个文件，+837 / -2795，净减少 1958 行
（包括回归测试和清单校验脚本，不包括审计文档）。删除 21 个旧文件；R1 的变更完整保留。
未提交/推送，既有 .githooks/ 不变。
