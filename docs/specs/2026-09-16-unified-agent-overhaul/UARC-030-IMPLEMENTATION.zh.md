# UARC-030 Web、Knowledge 与 Memory Module 实施记录

> Wave 2 barrier：`67417aa08`
> Wave 3 实现提交：`8454133b124a3d38636f882b098de80ce17028c0`
> 平台：Windows 实现与验证；macOS shared compile 仍待 Mac 主机

## 交付

- 将 Web、Knowledge 与 Memory authoring surface 收敛为四个产品 Module：`web.research`、
  `knowledge`、`project.memory`、`companion.memory`，分别发布 2、4、2、2 个 exact Actions。
- Web search/fetch 只暴露产品级 `web.research/search` 与 `web.research/fetch`。搜索 provider、HTTP
  transport、模型选择和引用渲染仍是宿主实现事实，不成为 Agent grant。
- Knowledge 的 search/read/write/autogen 由 `nomifun-knowledge` 的真实 owner 执行；Resource binding
  按 exact Action 推导最小 read/write operation，跨 owner、缺失或 operation 不足均 fail closed。
- Project/Companion memory 使用独立 authority adapter、typed resource 与 durable receipt；scratch、citation
  与 recall 内部细节不再成为可 authoring Capability。
- 引用 provenance 从 Session 范围内的真实查询/读取结果派生，`citation.render` 不再提供独立授权面。
- 官方 preset seed、目标 contribution inventory、Snapshot Action projection 与生成摘要已同步到四个 Module。

## 物理删除

- 删除 `nomifun-ai-agent/src/web_fetch.rs` 直接 Web Tool 入口。
- 删除 `local_web_search/binding_tests.rs` 及旧 `nomi_local_websearch` product capability identity。
- 删除 embedding/rerank/mount、citation render、memory citation/scratch 等只服务旧 authoring 架构的
  Capability；没有添加 ID 翻译器或永久兼容层。

## 验证

```text
cargo test -p nomifun-agent-domain-wave1 --lib -- --test-threads=1
  4 passed
cargo test -p nomifun-knowledge --lib -- --test-threads=1
  329 passed
cargo test -p nomifun-ai-agent --lib -- --test-threads=1
  542 passed
cargo test -p nomifun-app --lib agent_wave1 -- --test-threads=1
cargo test -p nomifun-app --lib nomi_core_resource_bindings -- --test-threads=1
  62 + 11 passed（Wave 3 合并 gate）
```

Contract generator、target inventory、Browser feature App check、UARC boundary self-test 与
`git diff --check` 均通过。Boundary 中 legacy authoring matches 由本 Wave 合并前的 1,350 降至 1,333；
剩余项均有后续 manifest owner。

## 平台状态

- Windows：verified。
- macOS：pending；Knowledge 文件 identity、provider transport 与 Memory 生命周期仍须在 Mac 真机验证。
