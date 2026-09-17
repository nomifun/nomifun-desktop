# UARC-020 自适应统一 Runtime 与长程 Coding 实施记录

> Wave 1 barrier：`4b65cf019e9e6942c5c3458d6db64d868c241bd2`
> Wave 2 实现提交：`efe80298f`
> 平台：Windows 实现与验证；macOS shared compile 仍待 Mac 主机

## 交付

- 保留一个官方 `nomifun.nomi` Runtime、一个 Driver 生命周期和一个回合循环。普通文本回合不创建
  长程账本；只有工具循环、steering、requirements、compaction 或 continuation 需要时才延迟进入长程路径。
- Coding Engine 继续保留 compaction、requirements ledger、completion evidence、history、task continuation
  和 patch/effect evidence，但不再存在 light/standard/durable 静态档位或第二 Coding family。
- Runtime build digest 覆盖 Module、Kernel、Store、Workspace effect owner、MCP exact owner 与宿主组合源码；
  restart recovery 只在 exact build、boot-frozen generation、进程树回收和 durable effect fence 全部成立后证明中断终态。
- cancel-before-prepare、model-open cancel、steering、multi-compaction、unknown effect 与 teardown quarantine 使用同一
  one-active-turn owner；简单回合不承担这些按需结构的额外状态。
- Runtime/Compiler 只消费 frozen Snapshot 的 Module + exact Action grants；不读取旧 activation journal，也不从
  Runtime family 推断 Capability。

## 物理删除

- 删除 `nomifun-coding-engine/src/process.rs` 的 engine-neutral process wrapper。
- 删除 `CodingEngineEvent::CapabilitiesActivated` 与 App 的旧 activation restore reader。
- 删除旧 private process recovery reader；保留的 `ProcessWitness` 只证明宿主进程树 dispatch/quiescence。

## 验证

```text
cargo test -p nomifun-coding-engine -- --test-threads=1
  40 passed

cargo test -p nomifun-engine-core -- --test-threads=1
  15 passed

cargo test -p nomifun-ai-agent --lib -- --test-threads=1
  548 passed；新增 device-proxy retirement focused test 另 1 passed

cargo test -p nomifun-app --lib -- --test-threads=1 \
  --skip robot_wiring::unified_tests \
  --skip bootstrap::environment::tests::v3_validation_failures_preserve_data_with_or_without_prior_retirement
  499 passed / 0 failed / 11 filtered

cargo check -p nomifun-app --lib
cargo check -p nomifun-app --lib --features browser-use,computer-use
  passed
```

11 个过滤项不属于 UARC-020：10 个 Robot 旧 device-MCP fixture 由 `UARC-042` 改成 exact Actions；
1 个 Bootstrap SQLite WAL 字节级比较测试可独立复现为不稳定项，逻辑数据校验单测可通过，最终 Windows 回归前闭合。

## 平台状态

- Windows：verified。
- macOS：pending；本任务未在 Mac 上补 shared compile，不据此声称跨平台完成。
