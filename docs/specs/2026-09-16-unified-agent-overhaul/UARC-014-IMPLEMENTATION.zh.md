# UARC-014 AgentPreset vNext Compiler 与通用 Capability Projection 实施记录

> 开始 barrier：`e7babfd127c5e29d05a18e323243ae5b9b7765eb`
> 实现提交：待提交
> 平台：共享 Rust contract/compiler；Windows 执行工程验证

## 交付

- `AgentPresetRevisionPayload`、API DTO 与 Session 创建响应不再包含 `runtime_engine` selector 或
  `runtime_engine_binding`。Control Plane 不再读取 Runtime family/profile 来决定 capability、模板或
  required feature。
- AI Agent 的生产 runtime catalog 收敛为一个 `NomiRuntimeProvider` 与一个
  `RuntimeEngineFactory`；任意 family catalog/selector 基础设施已物理删除。
- Kernel Compiler 从物化 Module 的真实 Action、Context、Event 与 Resource contribution 生成
  Snapshot。`CapabilityKind` 只保留目录展示语义，不再授予执行权或决定 middleware phase。
- exact Action grant 始终与物化 Action contract 交集；Coding profile 不再隐式扩张 runtime feature
  权限。中间件排序直接读取 Action phase，消费者继续对 Product source、descriptor、resource 与
  frozen authority 做 fail-closed 校验。
- App 的 Agent projection 改为核对 Binding → Revision → Snapshot 的 exact owner/digest chain，并从
  Snapshot 派生 module、resource、skill 与 model。旧 Capability ID → Nomi tool、Browser、Computer、
  MCP、runtime-profile 布尔表已物理删除。
- 新增 `capability-retirement.v1.json` 与 typed validator：冻结原始 inventory digest、source package、
  disposition、目标 Module/Resource/Platform owner 和后续 UARC owner；每个 route 必须包含
  `UARC-053` 最终删除 owner，且 136 个旧 ID 必须恰好覆盖一次。
- retirement manifest 已进入 canonical schema/validation fixture/digest ledger 生成链；篡改、重复、
  遗漏、错误 package 或无 owner 均 fail closed。

## 物理删除

- 删除 `AgentRuntimeEngineSelection`、`AgentRuntimeEngineSelector`、`RuntimeEngineSelection`、
  `RuntimeEngineSelector` 及其 catalog/select 测试基础设施。
- 删除 Preset/API/App 新建 Session 路径中的 `runtime_engine` 与 `runtime_engine_binding` 字段。
- 删除 Control Plane 的 runtime validator、Coding template/profile 分支和 Runtime-driven profile
  mutation；Compiler 构造器与 compile 调用也不再接受无效的 Runtime/template compatibility 参数。
- 删除 Nomi projection 中约 1,500 行 Capability ID 手写映射与 capability-specific availability
  分支。
- 删除 Kernel 的 CodingNative feature inflation 与 kind-driven middleware ordering。

## 串行边界与保留项

- 旧 ID 的 authoring/Runtime projection 已从本任务生产入口删除；现存 Domain manifests、迁移
  inventory 与 UI 检查点是 Wave 2–6 的显式迁移输入，由 retirement manifest 中的 owner 与
  `UARC-053` 最终物理删除，不提供兼容翻译器。
- `runtime_engine_binding` 仍存在于旧 Conversation persistence/registry 路径，只用于 UARC-052/054
  clean cut 前读取历史事实；canonical AgentPreset/AgentSession API 已不能选择或返回它。
- 单 provider 的旧 `catalog()` 调用名和 registry callback shape 仅留在 crate 内，指向同一个
  provider；物理删除归 `UARC-052`，不形成第二 factory 或 family。
- 本任务没有 UI 产品行为或布局改动。已冻结的双 Runtime 设置页检查点继续归 `UARC-050`，不能当作
  最终产品 UI；下述 milestone 修复仅补 license header 与同步既有测试。

## Wave 1 milestone gate 修复

- 全量 UI 首次运行暴露 6 个 source HEAD 既有失败：Guid 结构断言、已删除 Creative Studio 侧栏
  文件、read-only transcript 未 mock 新 emitter、Model capability 数量常量，以及 3 个缺 license header
  的既有文件。修复仅同步当前产品结构与测试隔离，不改变 UI 行为或布局。
- Core workspace 首次运行暴露 Browser recovery 日志测试的 Windows 并发分支不稳定，以及 Computer
  Browser 引导断言仍要求旧 `browser navigate` 文案。前者保留 fail-closed report/no-secret 断言并仅在
  实际捕获 event 时校验 reason；后者改为验证明确的 Conversation Browser/system-browser capability。
- 修复后 UI 为 3,575/3,575，Browser Engine 为 296/296（另 9 个环境型 ignored），Computer 为
  98/98（另 7 个原生环境型 ignored），Desktop 为 144/144（另 3 个真机/网络型 ignored）。

## 验证

```text
cargo test -p nomifun-agent-contracts --lib -- --test-threads=1
  107 passed（含 136/136 retirement exact coverage）

cargo test -p nomifun-api-types --lib -- --test-threads=1
  534 passed

cargo test -p nomifun-agent-kernel --lib -- --test-threads=1
  60 passed

cargo test -p nomifun-agent-control-plane --lib -- --test-threads=1
  48 passed；UARC-010 的 2 个旧 kind/direct-middleware transition failure 已消除

cargo test -p nomifun-ai-agent --lib -- --test-threads=1
  545 passed

cargo test -p nomifun-engine-core --lib -- --test-threads=1
  14 passed

cargo test -p nomifun-app router::nomi_core_agent_projection::tests --lib -- --test-threads=1
cargo test -p nomifun-app router::nomi_core_tool_discovery::middleware_validation_tests --lib -- --test-threads=1
cargo test -p nomifun-app router::runtime_engines::tests --lib -- --test-threads=1
cargo test -p nomifun-app router::state::tests --lib -- --test-threads=1
  4 + 7 + 2 + 6 passed

cargo check -p nomifun-app --lib
cargo check -p nomifun-app --bin nomicore
cargo check -p nomifun-app --tests
  passed（既有 warning 与后续 Wave 待删源码 warning 保留）

cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check
cargo fmt <UARC-014 touched packages>
git diff --check
  passed

bun scripts/check-uarc-boundary.mjs --self-test
  passed；production files 3,035；legacy matches 4,094 → 3,947；
  runtime.preset_selector 45 → 21；multi-family 27 → 25；
  compatibility branches 28 → 23；private transcript 44 → 43
```

完整 App transition probe 为 444/497。53 个失败已按新合同收敛为后续 Feature task 的真实迁移工作：

- `UARC-021`：27 个 Workspace/Process/Git fixture 仍用旧跨 Action state-projection payload；
- `UARC-022`：26 个 Skill/MCP/Plugin/Robot fixture 仍缺 exact Context factory/Action grant，或保留旧
  MCP acknowledgment 预期。

这些失败不经过 Runtime selector、Runtime-family capability branch 或旧 Nomi handwritten projection；
因此不恢复已删除的兼容层。Wave 2 合并后必须统一重跑完整 App gate。
