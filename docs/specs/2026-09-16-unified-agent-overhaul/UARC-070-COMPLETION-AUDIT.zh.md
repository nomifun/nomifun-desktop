# UARC-070 跨平台完成审计

> 审计日期：2026-09-20
> 分支：`rf/agent-capability-platform-v2`
> macOS production artifact source：`6ab013f68bfcdae46ffaeecf471bd4933ed34a67`
> macOS evidence closeout：`5fd6c6f0643206283aec17be83b8de8d5853f10e`
> Windows post-Mac implementation source：`2a426e7af3367ba47101fa3e676a71c89bc57af6`
> UARC-070 audit start：`93aad2c42a32900b8cc8eeadc696b4e8bf56f37e`
> late shared renderer commit：`daef16c9b41ba24604cc778e2450e3f22515fa62`
> Windows merge/revalidation source：`f08acea45d07a56e67a2f8afd1f359b9ec6aafd5`
> product-flow repair source：`4f249fd50a1036d560fcb244f67b98826eeebc6b`

## 1. 结论

本审计曾在 `8e1adf1ce` 形成 29/29 的 provisional closeout，但推送时发现远端在审计期间新增 shared
renderer commit `daef16c9b`。该提交已通过 merge `f08acea45` 无损接入并在 Windows 完成定向、完整 UI、
静态、production renderer 与 880×600/宽窗口视觉回归。生产旧架构、Rust/native、Browser、Process 和
package 代码均未变化。

随后从同一远端分支接入的修复被整合为 `4f249fd50`。它不再只是 renderer 增量，而是同时修改 canonical
AgentSession 消息/Turn 终态、immutable Agent 切换、AgentExecution collaboration、AutoWork 生命周期、
Companion 控件和 shared renderer。Windows 已对该源码完成全量 Rust/UI、静态、production build、真实商业
模型产品流与 880×600 视觉复验。

Mac signed/notarized artifact source 仍是 `6ab013f68`，早于上述共享代码。按 UARC-063 的“current source +
artifact”合同，不能把旧哈希冒充当前源码的产品或 package 证据。UARC initiative 保持 `active`；macOS 必须
在真实 Apple Silicon 主机对当前源码重建 App/DMG，并复验消息、Agent 切换、协作、AutoWork、CEF/TCC 与
应用生命周期。未改动的低层 native 证据可以保留，但不能替代当前产品组合证据。

显式 `not_run` 项继续保持 `not_run`，未被计作 pass：Keychain 持久化、attached Chrome 实连、Screen
Recording granted/Retina screenshot、真实中文/日文 IME composition，以及因主机全局关闭而无法执行的
Gatekeeper interactive assessment。这些项目已由用户从本轮必要门槛中移出；产品已有能力没有被删除。

## 2. Requirements-to-evidence 矩阵

下表的 `verified` 表示存在命令或原生证据。shared task 的平台结论由两部分组成：最终共享语义 gate，以及
该 production source 在真实 Mac arm64 App 中的编译、启动和相应原生 owner 证据；不把 Windows 原生结果
替代 CEF、TCC 或 App/DMG 证据。

| Task | 完成门槛结论 | Windows | macOS |
| --- | --- | --- | --- |
| `UARC-000` | inherited dirty worktree 26/26 文件归属并形成 barrier | verified | n/a |
| `UARC-001` | reachability、storage、timing、platform gaps 可机器读取 | verified | verified：late artifact gap 已机器记录为 `implemented_unverified` |
| `UARC-010` | Module 多 contribution 与 exact Action grants | verified | n/a：纯合同 |
| `UARC-011` | generation-5 canonical Agent Store baseline | verified | n/a：纯数据合同；Mac fresh-root 另由 051/054/063 覆盖 |
| `UARC-012` | 单一 AgentSession owner/API/receipt | verified | n/a：纯 owner/API 合同 |
| `UARC-013` | 单一 official Runtime factory 与 typed ports | verified | n/a：纯 composition 合同 |
| `UARC-014` | generic compiler/projection；无 Runtime-family branch | verified | n/a：纯编译合同 |
| `UARC-020` | adaptive unified loop、long-horizon/restart/cancel | verified：定向与最终 all-target | verified：packaged canonical Agent/Kernel turns、真实 StepFun、cleanup |
| `UARC-021` | Files/VCS/Process/Artifact exact Actions 与 owner effects | verified | verified：真实 Workspace patch + Process 240 + Terminal 146 |
| `UARC-022` | Skill/MCP/Plugin/Connector 无 Runtime/broad proxy | verified | verified：shared contract + arm64 production compile/start + process shutdown evidence |
| `UARC-030` | Web/Knowledge/Memory 隐藏 provider mechanics | verified | verified：shared contract + exact-source arm64 product compile/start |
| `UARC-031` | Channel/Companion/Customer scene-derived contributions | verified | verified：shared contract + exact-source arm64 product compile/start/cleanup |
| `UARC-032` | Creation/Workshop/Office/Attachment product modules | verified | verified：shared contract + production renderer/App compile/start |
| `UARC-033` | AutoWork 复用 AgentExecution；Agent path 无 IDMM | verified | implemented_unverified：新 preflight/pause/rollback 与 execution flow 待 current Mac 产品复验 |
| `UARC-034` | Schedule/Notification/Remote/SSH 保持 typed platform boundary | verified | verified：shared contract + arm64 product compile/start/cleanup |
| `UARC-040` | Browser 是任意授权 Agent 的 Resource/Module | verified | verified：正式 AgentSession → Kernel → CEF owner 链路 |
| `UARC-041` | Windows WebView2 Browser UI/Resource，无 Browser Session | verified | n/a |
| `UARC-042` | Computer/Robot exact Action 与 truthful availability | verified | implemented_unverified：native evidence 保留；新 Computer 设置入口待 Mac UI 复验 |
| `UARC-050` | Official Agents/Workbench/module UI 完整可用 | verified：880×600 + UI 3545 | implemented_unverified：immutable Agent selector 与 collaboration UI 待 Mac 复验 |
| `UARC-051` | 所有 Agent 操作只读写新 Store/API | verified | implemented_unverified：新消息身份/终态与 immutable Session flow 待 current Mac 复验 |
| `UARC-052` | 单一 execution loop；无旧 Runtime compatibility | verified | verified：arm64 product single Agent/Kernel path + zero reachability |
| `UARC-053` | 旧 capability/store/IDMM/Browser entry 零可达 | verified | implemented_unverified：零可达保持；replacement UI 待 current Mac artifact 复验 |
| `UARC-054` | fresh startup 不创建后删除历史 Agent schema | verified | verified：absent root 与 empty root 产品启动均通过 |
| `UARC-060` | Windows candidate 与 Mac shared-source barrier | verified | n/a：Windows acceptance task |
| `UARC-061` | macOS managed CEF Browser parity | n/a | verified：34/34、renderer crash、real StepFun、Command-Q cleanup |
| `UARC-062` | macOS Process/PTY/Computer/lifecycle parity | n/a | verified：Process 240、Terminal 146、Computer 92 + physical/native evidence |
| `UARC-063` | current-source arm64 product/package acceptance | n/a | implemented_unverified：旧制品完整通过，但不含 `daef16c9b` renderer |
| `UARC-064` | Mac 合入后 Windows 不回归 | verified：WebView2/system/UI/model/package | n/a |
| `UARC-070` | 全部决策、任务、平台与 owner 闭合 | verified | implemented_unverified：等待 current-source Mac UI/package 增量证据 |

## 3. 最终工程 gate

### Windows current source

| Gate | 结果 |
| --- | --- |
| `cargo test --workspace --all-targets --no-fail-fast -- --test-threads=1` | pass；修复后全部 workspace/all-target suites 0 failure |
| `cargo test --workspace --doc --no-fail-fast -- --test-threads=1` | pass；有效 doctest 1 passed，文档声明项 1 ignored |
| `bun test --cwd ui` | pass；3,545/3,545 at `4f249fd50` |
| product-flow focused Rust/UI tests | pass；message、Agent Session、collaboration、AutoWork pause/resume/rollback |
| `bun run check` | pass；包括 1,917-source 880×600 desktop boundary 与所有静态 scanner |
| production renderer build | pass；7,559 transformed modules |
| real 880×600 preview | pass；Agent menu、collaboration actions、Escape focus return、horizontal overflow=0 |
| Windows WebView2 default + unified Agent smoke | pass |
| StepFun Coding Plan `step-3.7-flash` product flow | pass；message terminal + new Session + collaboration + AutoWork；凭据仅由 Credential Manager 隔离注入 |
| NSIS candidate install/start/health/CDP/cleanup/uninstall | 14/14 pass |

Windows candidate：

```text
dist/desktop/NomiFun_0.7.6_x64-setup.exe
size:   64,094,527 bytes
sha256: 8f7b0fdc3d33b8cbd69f78de6cdbd2be2ec0c9e89ce6fcf06eb341801ca83a57
host sha256: e5a724d474197de4fc2fb40f4218a6335f74b78398713aa49c257cb8cfa0f63e
```

### macOS 既有冻结证据（低层 native 可复用，artifact/产品流不代表当前源码）

| Gate | 结果 |
| --- | --- |
| managed CEF child NSView | 34/34，含 crash、input、popup/dialog/permission/file/download 与 shutdown |
| packaged canonical Agent/Kernel/CEF | terminal receipt + cleanup；真实 StepFun functional workflow |
| Process/PTY/Computer | 240 + 146 + 92；Accessibility、physical modifiers、Terminal UTF-8/resize、Command-Q |
| production arm64 App | host、CEF framework、5 helpers 均 arm64；Developer ID deep/strict valid |
| DMG | signed、notarized、stapled；mounted identity、release lock、fresh-root startup、cleanup passed |

Mac artifact：

```text
App host sha256: 39d021de265b79ad51540fbe5e868a49f3912d3e102c79fbb096f2cf12d9ae53
DMG sha256:      25059e0e0d6199d8727ab5734f2bd608a698c2a1dc5020d32cec8617c5e52565
notary id:       ec5f0545-61fc-4bbd-a6ca-ca843cc77a3c
```

原始 `build.noindex` 报告属于 Mac 主机本地产物，未复制到 Windows checkout；本审计使用 Mac Integration
已提交的逐项 evidence、哈希和 notarization ID，不声称在 Windows 重放 Mac 原生命令。

CEF child NSView、Process/PTY 与 Computer physical/TCC 的未改动低层证据可按结果复用规则保留；但
`4f249fd50` 已改变 shared Runtime/API/UI，因此 packaged canonical Agent flow、App/DMG 和产品组合不能复用为
current-source 证据，必须在 Mac 重建并按 handoff 复验。

## 4. 生产可达性、删除与 retained 审计

- 普通 UARC scanner：13 个 legacy group 全部 0；open baseline anomaly 0；当前 Mac gap 1。
- `bun scripts/check-uarc-boundary.mjs --completion` 现在按设计失败，精确阻止 UARC-033/042/050/051/053/063/070 和
  initiative 在 current-source Mac evidence 补齐前重新关闭。
- completion scanner 现在同时验证 initiative/task 状态、Windows/macOS 平台状态、依赖存在性与 dependency
  closure；manifest 一旦为 `complete`，普通 scanner 也自动执行 completion contract。
- 首次 completion run 暴露 `automation.autowork_parallel_receipts` 86 个命中。逐项核对后确认均为 retained
  Requirements queue claim owner/count facts；旧 `AutoWorkSessionPort`、旧 Conversation delivery receipt 表、
  `MAX_ATTEMPTS` 和基于 `attempt_count` 的 retry branch 均为 0。matcher 已改为拒绝真正的私有 delivery/retry
  authority，不再把同一个 canonical AgentSession UUID 的 Conversation 产品投影误判为第二 Session。
- Browser 产品 production path 内无 `WKWebView`/`WKWebViewConfiguration`；搜索命中仅为 HTML
  `webkitdirectory` 属性和“不修改 WebKit private API”的注释。macOS Browser 仍是独立 CEF child NSView。
- 旧 Nomi/Coding factories/managers、multi-runtime registry/selector、private transcript、Agent IDMM、旧 Browser
  Session entry 和旧 capability projection 的生产路径均不存在。
- Process scanner 的 macOS external Browser/Downloads 例外仅允许 exact reviewed human-only OS handoff；任意
  path 仍由 self-test 证明 fail closed。
- UARC 初始 source 至当前 worktree 的 changed path 全部落入 manifest write set；消息/Session/API 归
  UARC-012/051，collaboration/AutoWork 归 UARC-033，Computer UI 归 UARC-042，Agent selector/Workbench 归
  UARC-050，replacement cleanup 归 UARC-053。
- 保留的 CEF/native fixture、Windows candidate smoke、Mac packaging validator 和 Browser GUI fixture 均是可维护
  regression gate；未发现无 owner 的临时代码或永久 compatibility wrapper。`build.noindex`/`target`/`dist`
  制品不进入 Git。

## 5. 当前开放闭包

- 22/29 task 保持 `complete`；`UARC-033/042/050/051/053/063` 为 `integrated`，`UARC-070` 为 `active`。
- Windows required platform 全部 verified；上述七项的 macOS 状态为 `implemented_unverified`。
- `TASK-MANIFEST.json` initiative status 为 `active`，completion scanner fail closed。
- 唯一剩余工作是外部 Mac current-source 产品/制品复验。未改动的 CEF 34/34、Process 240、Terminal 146、
  Computer physical/TCC 低层证据可保留；current-source shared Rust/UI、真实 StepFun 产品流、App/DMG 与
  CEF/TCC/lifecycle integration 必须重新出证。

Windows release 首次编译曾在 `nomifun-gateway` 的 rustc/LTO 处发生一次
`STATUS_ACCESS_VIOLATION`；相同 clean source/命令精确重跑通过 release、NSIS 和 14-check candidate smoke，故记录
为已复现并越过的工具链瞬态，不豁免任何产品 gate。
