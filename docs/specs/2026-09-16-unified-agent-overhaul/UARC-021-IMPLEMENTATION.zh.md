# UARC-021 Workspace Files、VCS、Process 与 Artifact Module 实施记录

> Wave 1 barrier：`4b65cf019e9e6942c5c3458d6db64d868c241bd2`
> Wave 2 实现提交：`efe80298f`
> 平台：Windows 实现与验证；macOS filesystem/process 仍待真机回归

## 交付

- `workspace.files`、`workspace.vcs`、`workspace.process`、`workspace.artifacts` 四个直接 Module 共发布
  19 个 exact Actions；`CapabilityKind` 不决定执行权，Action allowlist 才是授权事实。
- Resource resolver 从 frozen Snapshot 的全部 direct/dependency Capability 和 exact Action allowlist 推导最小
  operation。read-only files/VCS/artifact 只得 `read`，mutation 只得 `write`，Process Actions 只得
  `process_session/execute`；不存在 Module union 扩权。
- Store 要求 `Turn started -> exact tool/call-started -> effect/started` 的完整因果链；terminal/reconcile 必须保留
  effect、Turn、operation、Module、Action、input digest、strategy、resource identity 和 immediate causation。
- Workspace Artifact 使用目录能力、进程级 lease、RAII staging、content-addressed publication、父目录同步、
  bounded verified LRU、stale temp 分批清理和 outcome-unknown publication fence。
- VCS stage 使用仓库标准 `index.lock`，在锁内完成 read-modify-write 与原子 index replace；receipt 同时覆盖
  additions/deletions，并保持 nested workspace-relative、sorted、deduplicated。
- `.nomifun` owner namespace 对 symlink/junction alias、祖先交换和浏览/search/VCS 投影均 fail closed。

## 删除与保留

- Resource projection 删除 `fs.*`、碎片化 `vcs.*`、`process.exec/session`、`terminal.pty`、
  `workspace.bind` 的授权分支；canonical Module 缺少 frozen Action map 时直接 contract mismatch。
- 删除 Coding process wrapper；物理 process effect 仍由 Process Runtime/host owner 执行，不移动进 Runtime strategy。
- 保留 atomic file owner、process tree cleanup、Git effect receipt 和 typed resource binding。

## 验证

```text
cargo test -p nomifun-agent-domain-wave2 -- --test-threads=1
  17 passed
cargo test -p nomifun-agent-session -- --test-threads=1
  28 passed
cargo test -p nomifun-file -- --test-threads=1
  378 passed across all targets
cargo test -p nomifun-engine-core -- --test-threads=1
  15 passed
cargo test -p nomifun-js-kernel-adapter -- --test-threads=1
  2 + 23 passed
cargo test -p nomifun-app --lib agent_wave2_host -- --test-threads=1
  29 passed
cargo test -p nomifun-app --lib nomi_core_resource_bindings -- --test-threads=1
  10 passed
```

Process Runtime 的 Windows serial gate 已覆盖 lib 119 及 architecture/child/IO/process/PTY/request/registry/
supervisor/parent-death 集成组并全部通过。Cargo lease 始终串行使用。

## 平台状态

- Windows：verified。
- macOS：pending；文件 identity、Unix symlink、PTY/process tree 仍须在 Mac 真机验证。
