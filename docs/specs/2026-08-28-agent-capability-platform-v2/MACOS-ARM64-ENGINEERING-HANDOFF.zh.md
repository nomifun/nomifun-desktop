# macOS arm64 工程交接：转 Windows 全盘验收

> 日期：2026-09-01
>
> 交接类型：工程连续性 handoff，不是 HP-2，不是 C8-MA PASS。
> 机器可读版本：`MACOS-ARM64-ENGINEERING-HANDOFF.json`

## 交接结论

macOS arm64 侧已完成当前主机可执行的工程定向验证，工作树 clean，分支已
普通推送。由于真实 Codex arm64 sidecar、Universal 双架构应用和可用的
binding/token/provider lifecycle 资源不在本机，继续在 macOS 上扩展业务 owner
不会产生有效的原生验收证据。

因此将下一阶段交给 Windows x64：先在最终 clean source 上重新冻结
`candidate_source_sha` 与四字段 cohort tuple，再执行 C8-WIN-PRE 全盘测试。

## 当前 checkpoint

```text
branch: rf/agent-capability-platform-v2
ref: refs/heads/rf/agent-capability-platform-v2
verified remote SHA before this handoff metadata: 6f2ad3eca1ad7bede4a1a3c500b06bd91ba41f36
worktree: clean
host: Darwin arm64
translated: 0
rustc host: aarch64-apple-darwin
```

交接 JSON 固化了这个 clean source checkpoint；交接文件本身是工程文档，不是
候选 PASS evidence。Windows 开始测试前仍须核对当前 remote HEAD，并以最终
checkout 的 clean SHA 重新生成 candidate tuple。

## 当前 canonical inputs

```text
confirmed_decision_contract_digest:
b45efce157933d72671a9158ff87d4a84b5b288bc8ec6bf3688226497c6e0cf5

canonical_schema_manifest_digest:
e28723d7fc524cfdd351c6fc8cc17b8a48d8fd1f5be16a7aebd395ce669f98ff

platform_validation_fixture_digest:
fa3cd9c542bab988afc366d512c279e34f33bef07bf2546a78094845f81bb948

runtime_release_digest:
c4075b2f7c118fa5eeeb6fc4a0b21cf940d5af6a8acc080e1c8721a8a738a380

Cargo.lock SHA-256:
26e121277eb2054fc43f80dbfc72b7a8ee4fc2cebcc8294752217944989dfb14
```

注意：当前 platform payload 内的 `candidate_source_sha` 仍是历史
`7a2ade3c...`。这不是有效的当前 C8 tuple；不要手工把它改成包含该文件的
当前 commit，避免产生自引用假闭合。最终 candidate 冻结时由 validation
owner 统一重新生成 payload、fixture 和 tuple。

## macOS 已验证

- `cargo check --locked -p nomifun-app -p nomifun-web -p nomifun-desktop`
- 六个 Domain crate 定向测试：58 passed
- `router::agent_platform_host`：18 passed
- `router::agent_wave2_host`：16 passed
- `nomifun-file`：202 passed
- `nomifun-codex-runtime`：31 passed
- `nomi-tools`：311 passed
- `bun run check:i18n`：7078 keys / 33 modules
- `bun run build:ui`：7720 modules transformed
- contract check、C7 informational gate、C8 self-test、format/diff checks
- macOS helper：native host、空 root、health、137 capability inventory、
  进程清理、arm64 app 与 DMG 基础检查通过

这些是定向/工程检查，不是 C8-MA full Gate。

## macOS 未闭合项

1. 缺少真实 arm64 sidecar：

   ```text
   expected SHA-256:
   7863db3a77545eec8966483f26fb5b493aea6e285ac35b5c29d0920342438060
   logical path:
   runtime/macos/arm64/nomifun-codex-runtime
   ```

2. 当前构建产物只有 `arm64`，不是要求的 `arm64+x86_64` Universal app。
3. 没有 hello metadata、真实 Remote binding、access token、provider credential
   和 endpoint，因此未执行：

   ```text
   open → ready → initial turn → observe → cancel → dispose
   ```

4. 没有生成 `PlatformCellEvidence` PASS；没有宣称 C8-MA、HP-2 或 C8-MERGE。

## Windows 接手步骤

### 1. 核对仓库与环境

```powershell
git fetch origin refs/heads/rf/agent-capability-platform-v2
git rev-parse HEAD
git ls-remote origin refs/heads/rf/agent-capability-platform-v2
git status --short --branch
```

要求 local HEAD 与 remote SHA 相等，且 worktree clean。不要 reset、覆盖或
force-push。

### 2. 核对基础合同

```powershell
cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check
bun run gate:agent-v2 -- --self-test
bun run gate:agent-v2 -- c7-domain-waves
```

### 3. 准备真实 Windows runtime

提供与 release input 对应的真实文件：

```text
runtime/windows/x64/nomifun-codex-runtime.exe
expected SHA-256:
36f175f56e065560749fcc16caffbe06639eece66e19b655ea9104052d85cab4
```

同时提供配套 hello metadata。不得用 macOS binary、模拟器、空文件、mock 或
旧 release artifact 替代。

### 4. 冻结最终 candidate

在 Windows validation owner 确认最终 clean source 后，重新生成并对账：

- `candidate_source_sha`
- `runtime_release_digest`
- `platform_validation_manifest_digest`
- `confirmed_decision_contract_digest`
- Cargo.lock 与 generated fixture digest

任何 input 变化都会使旧 native evidence stale；不能沿用旧 SHA 的 evidence。

### 5. 执行全盘验收

```powershell
bun run gate:agent-v2 -- c8-win-pre
```

该 Gate 负责 Windows whole-candidate/all-scene、workspace serialized cargo test、
UI check/build、fresh root、lifecycle/fault、package 与 process-tree cleanup。

必须另行记录真实：

```text
runtime/hello
open → ready → initial turn → observe → cancel → dispose
```

如果 workspace/UI 或 residual Gate 失败，记录真实失败和影响范围；不要用
allowlist、mock、旧 evidence 或跨平台结果改写为 PASS。

## 交回材料

Windows 完成后应回传与最终四字段 tuple 绑定的：

- `PlatformCellEvidence.json`
- native Windows x64 fingerprint
- Host/sidecar/package/helper artifact digests
- exact command results 与 raw-log digests
- 未执行项目及原因
- 若 tuple 变化，明确标记旧 macOS/Windows evidence stale

在 Windows C8-WIN-PRE 全量 Gate 真正通过前，macOS 侧不再继续扩展功能或
生成任何“通过”证据。
