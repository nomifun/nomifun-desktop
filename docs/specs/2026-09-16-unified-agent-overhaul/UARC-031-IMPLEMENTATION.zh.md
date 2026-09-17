# UARC-031 Channel、Companion 与 Customer Service Module 实施记录

> Wave 2 barrier：`67417aa08`
> Wave 3 实现提交：`8454133b124a3d38636f882b098de80ce17028c0`
> 平台：Windows 实现与验证；macOS shared compile 仍待 Mac 主机

## 交付

- `channel.messaging`、`companion`、`customer.service` 三个 Module 只发布真实业务 Actions：
  reply/send、learn/evolve、notes.read/notes.write/handoff。
- 三个 Module 同时发布 binding-derived `BeforeTurn` Context contribution。Channel group policy、Companion
  persona 与 Customer dialogue 是内部 scene schema，不进入 authoring catalog、Preset Capability ID 或
  Action allowlist。
- Channel 新 owner 在 durable receipt admission 后执行 send/reply，并通过 exact Channel resource 激活 ingress；
  Customer binding authority 与 ingress lease 在解绑、删除和失败路径上统一释放。
- Companion owner 从所选 resource 读取 persona 并执行 learn/evolve；`companion.memory` 继续由 Wave 1 的
  唯一 memory owner 提供，未重复登记。
- Customer Service 的 dialogue policy 从 scene binding 派生；工具过滤只读取 exact Action IDs
  `knowledge/search`、`knowledge/read` 与 `customer.service/notes.read`，不再把 Module 或旧点号 ID 当授权。
- Resource resolver 同时推导 Action effect 与 scene 所需最小 operation：Channel receive/manage、Companion
  read、Customer read 不由额外 authoring grant 伪造。

## 物理删除

- 删除 App 中无 owner 的旧 `agent_wave4_host.rs` wrapper，并由 Channel、Companion、Customer 各自 domain
  owner 取代。
- 删除 channel pairing/receive/group-policy、companion persona/roster、customer dialogue 的 authoring
  Capability 路径；内部 scene identity 仅作为绑定派生 schema discriminator 保留。
- 不保留旧 Capability → Action 映射或 fallback dispatch。

## 验证

```text
cargo test -p nomifun-agent-domain-wave4 --lib -- --test-threads=1
  target contract 6 passed
cargo test -p nomifun-channel --lib -- --test-threads=1
  353 passed
cargo test -p nomifun-companion --lib -- --test-threads=1
  275 passed
cargo test -p nomifun-customer-service --lib -- --test-threads=1
  31 passed
cargo test -p nomifun-app --lib nomi_core_wave4 -- --test-threads=1
  10 passed
```

新增 exact Action 回归证明 Customer Service 的三项只读工具均按 canonical slash Action ID 入表；Channel、
Companion、Customer 的跨 owner、缺 resource、disabled/rebound 与 outcome-unknown 路径均 fail closed。Wave 3
二次只读审查未发现 P0/P1。

## 平台状态

- Windows：verified。
- macOS：pending；Channel transport、Companion 生命周期与 Customer ingress 仍须在 Mac 真机验证。
