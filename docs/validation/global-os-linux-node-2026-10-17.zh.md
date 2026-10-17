# Linux Node 与用户工具生命周期验收（2026-10-17）

本页分列 Ubuntu 26.04 / WSL2 上的运行时组件证明与实际正式 release App 的默认 Agent 产品证明。组件补验复用现役共享准备入口、MCP owner、canonical Session store 与 Agent-only database reset，新增一个显式注册的 opt-in integration test；后续产品验收使用仓库外脚本驱动已安装的正式 App。Node 补验没有修改运行时生产实现、依赖、默认 Agent 权限、预算或 lockfile。

## 已运行的边界

原 `node-native.log` 中的 `official_node_offline_npm_lifecycle_argv_and_restart` 为 **1 passed**，另有 70 项 runtime unit tests。原 fixture 将 HOME 指到私有目录，不能把它记录为生产用户 HOME 的验收，也没有运行 MCP 或 Agent-only reset。它证明完整官方发行、离线 npm lifecycle、literal argv、用户 CLI、缓存修复、测试子进程重启及卸载。

新增 `crates/backend/nomifun-app/tests/linux_node_lifecycle.rs` 使用 UID 1000、真实 HOME `/home/muri`、唯一私有 XDG/data/cache/npm-cache；显式 PATH 最初仅包含 `/bin/sh` 的私有链接。全部 Node `execPath` 均来自该 fixture 的受管官方发行。复用已有 `approved.asset`，运行前核对当前组件 manifest 的 SHA-256，不重新下载 Node 或访问 npm registry。源码在 `nomifun-app/Cargo.toml` 的 `[[test]]` 中显式注册，避免 `autotests=false` 下漏跑。

该 test 在三个独立进程中实际完成：

1. 离线全局安装唯一 fixture package；postinstall 经受管 Node 运行，保留真实 HOME。
2. 运行 CLI 与 MCP stdio server，核对空参数、中文空格、引号、换行、shell 元字符按 literal argv 传递。通过公开 `McpOwner::invoke` 完成 initialize、tools/list、tools/call；返回后分别确认四个真实 MCP PID 已消失，owner 清理成功。
3. 通过公开 `AgentSessionStore` 创建 Session、提交 `session/ready`，接受并精确取消两个 Turn。调用公开 `nomifun_db::reset_agent_data`，证明 1 Session、2 Turns、9 Events 变为零，installation owner 保留。
4. Agent-only reset 前后，用户 CLI 的 dev/inode/UID/SHA-256 不变，CLI 与 MCP 实际再次运行成功。
5. 首个宿主测试进程退出后，删除并重新供应该 fixture 的 runtime cache；第二个进程直接启动持久 CLI，再经 MCP 调用成功。第三个新进程再次成功，然后正常 npm uninstall，最后删除整个唯一私有 fixture。

两个 Store Turn 是被接受、取消的组件 fixture。它们之间的 CLI 调用没有经过 App Runtime/Kernel 的工具派发，不能把直接 CLI 结果归因到这些 Turn，也不构成 native desktop App 后继 Turn 或 MCP Capability/Effect 的端到端证明。当前 generation 的公开 Agent-only reset 不等于 retired-generation startup clean cut；后者使用独立数据库 lineage 测试证明。runtime cache replacement 也不能称为跨版本软件升级。

## 当前实测证据

证据根：`/home/muri/nomifun-linux-evidence-20261017`。

- `node-lifecycle-native-v4.log`：当前源码针对已成功 registered Cargo build 的精确依赖 artifact 重链接后真实执行，**1 passed，0 failed**。
- `node-lifecycle-component.json`：`status=passed`、`uid=1000`、`home=/home/muri`、`no_download=true`、`uninstalled=true`、`owned_fixture_removed=true`。SHA-256 为 `817f530ce61c626dcffc41e86b6ed8ac684733b32c7e9c72ce3885280ca280a9`。
- `node-lifecycle-component-build/report-home-v4-compile.json` 与 `.log`：该次独立 rustc 重链接的源码 SHA、完整 argv、12 个按 Cargo fingerprint 精确匹配的 dependency artifacts，exit 0；不改 Cargo fingerprints，也不重建并发修改中的 App 生产源。
- `node-lifecycle-build-first.log`：显式注册目标的首次 Cargo build 成功。首次执行 fixture 漏提交 ready 边界的失败保存在 `node-lifecycle-native-first.log`；补 ready 时 typed `SessionEventKind` 写法的编译失败保存在 `node-lifecycle-component-build/ready-boundary-v2-compile.log`。修正后的 v3 与 v4 成功日志、JSON 均保留，未覆盖早期结果。

官方 Node archive SHA-256：`c33c39ed9c80deddde77c960d00119918b9e352426fd604ba41638d6526a4744`；实际 Node executable SHA-256：`1bec56ef7cfa9a76f3e0b7c0a87f220eb73f23102b9c0b4c7529a3f7c3ce7c31`。

最终生产 source freeze 后，通过显式注册的标准 Cargo 目标再次执行：`node-lifecycle-cargo-final.log` 为 **1 passed，0 failed，13.32 秒**，进程 exit 0。`node-lifecycle-cargo-final.json` 的 SHA-256 为 `e01269a20d2a6d00a2a1314192d99a65f1eabc6ea28bf41a8304f408c02cfdbb`。再次确认 UID 1000、HOME `/home/muri`、1/2/9 条 Session/Turn/Event 变为 0/0/0；CLI dev 71、inode 151779、SHA-256 `6af9da8d851587f9991a97e534db24bfd9390001cb075b007f4574cfb910f201` 跨 reset/cache replacement/restart 保持一致。MCP PID 236498、236664、236786、237079 均已消失，正常卸载、无下载、唯一私有 fixture 已删除。此标准 Cargo 结果是本页当前主证据；先前重链接结果和失败历史仍保留。

## 正式 release App：默认 Agent 三个真实 Turn 与正常重启

该产品验收已完成，外部 driver 进程 **exit 0**。入口为 deb 实际安装的 `/usr/bin/nomifun-desktop`，安装文件 root-owned，App 及全部受测 Node/npm/CLI 以 UID 1000、真实 HOME `/home/muri` 运行。重启前后 executable SHA-256 均为 `b4a9306dcd6d12e59370a6225250b17a2e90c70e443b57cb4af0271690ae6cd2`，不是替代测试宿主。

主报告位于证据根下 `node-agent/reports/node-lifecycle-chat.minimal-1792217805802-5a13dd49-cda0-47ce-a6c8-245bd5b9978a.json`，SHA-256 **`02170f4281104130d6bb0608f7cd72f157db2fa20f791d497ac80672a0647e16`**，`status=passed`。正式 Session 为 `01a14881-747e-71c3-8c68-774082e6dafa`，使用未扩权的默认 `chat.minimal`。三个阶段都有各自的真实 accepted Turn、canonical 工具调用归因、`completed/answered` 终态、`cleanup_proven=true`、`pending_effects=0`、`unknown_effects=0`：

| 阶段 | accepted message / Turn 输入 | 实际行为与完成合同 |
| --- | --- | --- |
| `node-install` | `01a14881-7604-7663-acf6-457a4245c0e7` | 裸 `node` 复用已经真实准备的官方发行；裸 `npm --global --offline` 安装唯一离线包，postinstall 真正执行，再运行裸 CLI 与 literal argv；正常 `report_completion` |
| `node-successor` | `01a14881-85b7-7233-82c0-ac62f54b4943` | 同 Session 的新 Turn 无重装运行原裸 CLI；本次单次工具调用没有启动 task ledger，实际模型表面无 `update_plan/report_completion` 且 tool choice 允许直接回答，按正式 `direct_answer_non_ledger` 合同得到 canonical answered |
| `node-restarted-cleanup` | `01a14881-e8ad-7582-b155-f1f8013dc34d` | 实际 App 正常退出并重启后，新 Turn 再次运行同一裸 CLI，然后由正式 `npm uninstall` 卸载并通过 `delete_path` 删除唯一 fixture；正常 `report_completion` |

原 App PID **277137** 正常退出为 0 后，新的实际 App PID **278572** 打开同一 private data/XDG，API 从 `127.0.0.1:36693` 变为 `127.0.0.1:35731`。同一 owner JWT 重新认证，同 Session、owner 和完整冻结 binding 均保持一致，binding SHA-256 为 `186b85a016cc388d0546d6671bc146ebe3683d8390cf80592d67a650241a2aca`。driver 保持自己的本地脚本 provider 存活，没有用新身份、新 Session、替代 CLI 或重新安装来满足重启存续。

App PATH 与每个正式工具请求的显式 PATH 均从该私有 fixture 的 `node-shell-only` 开始，其中只有 `sh -> /bin/sh`。宿主本身存在用户 Node，本次不会删除或修改它；请求显式 PATH 按现役合同排除用户登录 shell 的 Node 路径。实际裸 Node/CLI PATH、postinstall PATH、UID、HOME 和 `process.execPath` 都由原生子进程输出并独立核对，裸 CLI 的 PATH 仅包含私有 shell、私有 durable tools bin 和受管 runtime bin。npm 自身为 lifecycle 增加的 `node_modules/.bin` 路径原样记录，实际解释器仍为 SHA 固定的受管 Node 22.22.0。

唯一包为 `nomifun-agent-node-3f868088883e493ca60b1a39bec681e8`，正常默认 prefix 在 private `xdg-data/nomifun-user-tools/npm`。从安装后、第一个 CLI、后继 Turn、原 App 退出、重启后新 Turn 到卸载前，CLI 的 owner、mode **0700**、dev **71**、inode **175531**、链接 inode **175533** 和文件 SHA-256 **`24f454f8559f8a0d5637ac20cb14a9185a74158b41c54d9521af3fb96caceea0`** 精确一致。0700 来自实际 App 的 `umask 077`，验收核对 owner 可读/执行、无 setid 或 group/other write，并未 chmod 工具来满足 fixture。空参数、中文空格、双/单引号、换行和 shell 元字符均按原始 literal argv 核对，副作用标记始终不存在。

postinstall 与三次 CLI、裸 Node probe 的实际观察 PID **278406、278449、278466、278483、278848** 均已消失；所有 exec 同时要求正式 owner 的 `cleanup.reaped=true`。正常卸载后 package 目录、CLI 链接、唯一 Node fixture 和其外层 product fixture 都已实际不存在。概要保存在 `node-agent/final-node-product-summary.json`，SHA-256 `032577917acdf189ccefc83dcbeb85490fc74eb02f70c7bfe5e736f0a83460cb`。

### 自动准备与真实 Effect 的独立事实

最终通过主体明确记录 `initial_managed_distribution_absent=false`，它验证的是同一真实发行缓存的使用、npm/CLI、后继 Turn 与正常 App 重启。首次自动准备由较早的真实默认 Agent 调用独立证明，不能把最终通过主体写成重新提取 Node：

- `node-agent/reports/node-lifecycle-chat.minimal-1792217471589-6d56e543-7fe4-4ba0-836d-b752e3adf965.json`，SHA-256 `9c67509093dbae3c8022232dd68a7a88f2c3d6f163e022a3e85f972885f3e1cb`。该 run 开始时 `distribution-v1` 确实不存在，Host 仅预存已校验 archive；正式 App 中的裸 Node 调用自动发布完整官方 **6194 entries** 的 Node/npm/npx，真实返回 Node 22.22.0、UID 1000、实际 HOME 和 pinned executable SHA。整个 run 后来因 fixture 强制 0755 而失败，状态仍保留 **failed**，原成功准备调用没有被改写或重放为整次成功。
- `node-agent/third-auto-canonical-effects.json`，SHA-256 `50f0be731f907268d8398089858f78378046226d3a2bc34ecb5746234fbabe9c`。独立 SQLite read-only 审计确认上述第三 run 的 bare prepare 与 npm install 两条真实 Effect 均为 **returned**，其 started/succeeded Event 引用属于确切 Session/Turn。这里不把 `runtime/progress` 的 host dispatch/settled 单独等同于 Effect Returned。
- `node-agent/passed-cached-lifecycle-canonical-effects.json`，SHA-256 **`b84e1a82cf18860091033c2f853d96af55c06b79160887a2bfa1849519215d7a`**。独立审计确认最终通过主体的三个 accepted Turns 有 **3 + 1 + 3 = 7 条 returned Effect**，同 Session 的 started/succeeded Event 引用全部匹配。9 次 host 调用中的两次 `instruction_scope` 仅为观察，不伪计为 mutation Effect。审计通过 SQLite URI `mode=ro`、`query_only` 和必要表/列 authorizer，连接 changes 为 0，不读取 auth/token 或输入输出 payload，不写数据库。

本节四个先前失败报告均原样保留并先经确切取消、零 pending/unknown 和新清理 Turn 结算：App PATH 未显式限制 task PATH 而找到用户 Node；npm user/global config 误用同一文件；0755 fixture 假设与实际 umask 077 不符；复制 driver 把单次工具 Turn 的合法直接回答误限制为 desktop。修正全部位于仓库外验收脚本或私有配置，不改生产代码、默认权限和预算；成功结果来自之后新的真实运行，未把旧失败提升为通过。 四次失败的实际取消、fresh cleanup、原生 PID 与文件删除闭环另见 `node-agent/failed-run-closure-proof.json`，SHA-256 `af6a61c4ade06a2c976f4f7a0080127672b27c369d2b34c82c865f6c108483ea`。

原 MCP owner 与 Agent-only reset 的结果仍保持上文组件范围。本节实际证明默认 Agent 的用户 CLI 后继 Turn 与正常正式 App 重启；没有执行产品级 Agent-only reset 后的 CLI 回归，也不把同版本重启或缓存替换声称为跨版本应用升级。脚本模型仅提供确定性工具调用，不代表外网模型推理验收。

## 可重复执行

先具备当前 build 已校验并保留的官方 Node archive；测试发现 archive 缺失时直接失败。本轮可在真实 Linux 仓库执行：

```sh
cargo test --locked -p nomifun-app --test linux_node_lifecycle -- \
  --ignored --exact linux_managed_node_mcp_reset_and_new_processes --nocapture
```

可设置 `NOMIFUN_LINUX_NODE_LIFECYCLE_REPORT` 指向一个不存在的新 JSON 文件，输出不含凭证的报告。每次分配新私有目录；测试保持本机 HOME，npm userconfig、prefix、cache 与 runtime 均在私有 fixture 内，不读取或修改用户原有 npm config/tools。