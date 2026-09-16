# UARC-013 单一 Nomi Runtime Driver / Host Ports 实施记录

> 开始 barrier：`fe44927d5bb86c94f5b55f1e8736f817d2c1f4c7`
> 实现提交：`5f317024d63c6845d896d379c201254101410d1b`
> 官方 family：`nomifun.nomi`
> 平台：共享 Rust Runtime contract；Windows 执行工程验证

## 交付

- 新增内部 `NomiRuntimeDriver`/`NomiRuntimeDriverFactory` 命名合同；它复用已经通过生命周期验证的
  `EngineSessionDriver` 与 `HostedAgentRuntime`，不是第三方稳定 ABI，也没有运行期注册入口。
- `NomiRuntimeProvider` 在组合时冻结一个 descriptor、一个 exact binding、一个 factory 和一个
  admission policy。生产构建中不编译任意 family/channel catalog；旧 catalog 只在
  `test`/`test-support` 下保留给等待 UARC-052 删除的迁移测试。
- provider 只接受 `nomifun.nomi`、一个 `default` 内部 profile、exact build ID/digest/host contract；
  foreign family、build、digest 或 profile 在 factory 调用前拒绝。
- App 组合根先构造 `EngineSessionHost` typed ports，再把现有 source-integrated Driver 作为唯一 factory
  安装到 provider。该实现的内部 engine family 被 provider 重绑定为 `nomifun.nomi`；产品不再安装
  `nomifun.coding` 第二 family。
- official build digest 同时冻结 source-integrated implementation digest、Driver/provider contract、
  Session host 和 journal 源码；升级改变 build identity，不通过用户配置切换。
- Runtime registry 在任何 provider/factory 工作前拒绝非官方 family。组合层只构造
  `AgentRuntimeHandle::Registered`，不再分支或 downcast `AgentRuntimeHandle::Nomi`。
- official admission 明确使用 platform history context，不取得旧 Nomi private Session codec；模型、
  Kernel Tool、Resource、journal、process 与 cleanup 继续通过现有 typed host ports。
- Driver lifecycle gate 证明 one-active-turn、cancel 后 cleanup-before-terminal、Session teardown，以及
  cleanup 失败时 transport quarantine。

## 物理删除

- 删除 `NomiCoreApplication::compose_with_runtime_engines` 与
  `DesktopServer::start_with_runtime_engines`，桌面、Web 和 CLI 只能走固定组合根。
- 删除 App 对 `RuntimeEngineHost`/`SessionEngineDriverFactory` 的公共导出，并将 AppServices provider
  降为 crate-private。
- 删除 `RuntimeEngineHost` 的 register/register-channel/register-hosted API。
- 删除 `nomifun-app/examples/evidence_engine/**` 及其 Cargo example target。
- 删除整份 `coding_runtime_production` 多 Runtime 验收目标；该用例的核心断言是注册
  `customer.runtime`、选择 `nomifun.coding` 和跨 family 切换，不能改名后继续保留。
- 删除组合根的 Coding second-factory/restart-recovery registration；唯一 source-integrated Driver 由
  provider 直接持有。

## 串行边界与保留项

- 旧 Nomi factory/manager 源码仍作为待删实现存在，但传入 registry callback 的旧 factory 已不可达；
  物理删除归 `UARC-052`。
- 当前 source-integrated Driver 的自适应/长程 Coding 强化归 `UARC-020`；本任务只冻结唯一身份、
  lifecycle 和 host-port contract。
- Preset/API 中 `runtime_engine` 字段仍由 `UARC-014` 物理删除；UARC-013 已使任何新选择在服务端
  fail closed。
- Coding-specific host/recovery 文件仍作为 UARC-020 的迁移输入，因此暂不物理删除；它们已不形成
  第二 family 或第二 factory。

## 验证

```text
cargo test -p nomifun-ai-agent --lib -- --test-threads=1
  552 passed

cargo test -p nomifun-engine-core --lib -- --test-threads=1
  14 passed

cargo test -p nomifun-app router::runtime_engines::tests --lib -- --test-threads=1
  2 passed

cargo test -p nomifun-app router::state::tests --lib -- --test-threads=1
  6 passed

cargo test -p nomifun-app desktop::tests --lib -- --test-threads=1
cargo test -p nomifun-app bootstrap::nomi_core --lib -- --test-threads=1
  passed

cargo check -p nomifun-app --lib
cargo check -p nomifun-app --bin nomicore
cargo check -p nomifun-app --tests
  passed（既有 warning 与等待 UARC-020/052 的死代码 warning 保留）

cargo fmt -p nomifun-ai-agent -p nomifun-app -- --check
git diff --check
  passed

bun scripts/check-uarc-boundary.mjs --self-test
  passed；compatibility branches 28 → 26、multi-family literals 27 → 25、
  private transcript references 44 → 43，所有 13 组均未增长
```

额外 App 全量 transition probe 为 455/515。60 个失败均落在 UARC-010 后尚未迁移的旧 Capability
fixtures（空 Action grant、kind-only Context factory、旧 Wave2 schema/MCP 预期），没有
Runtime provider、Driver lifecycle、desktop/bootstrap/state 失败；这些由 `UARC-014/021/022` 的既定
任务处理，不恢复 Runtime selector 或兼容 family 来伪造全绿。
