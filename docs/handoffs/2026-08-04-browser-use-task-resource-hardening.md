# Browser Use 单任务资源治理与泄漏封堵交接（2026-08-04）

> 状态：专项分支检查点，包含大量已验证实现，也包含两条正在收口的 WIP（Primary 持久档案围栏、下载生命周期最终修复）。接手者必须先执行本文“第一轮接手检查”，不要把整份分支直接当成已发布版本。

## 1. 分支与检查点

- 专项分支：`codex/browser-use-task-resource-hardening`
- 分叉基线：`fa493e1f21fad8dcacbd4c4cb3feb842d285f312`
- 仓库：`https://github.com/nomifun/nomifun-tauri.git`
- 本机工作目录：`C:\Users\Developer\code\nomifun\nomifun-tauri`
- Git 归属：仓库本地设置为 `NomiFun Contributor <nomifun@users.noreply.github.com>`；`.githooks` 已启用。不得使用 AI/模型/机器人身份提交，也不得增加 AI attribution trailer。
- 仓库硬规则：绝对禁止 `.github/workflows/*.yml` 和 `.github/workflows/*.yaml`。本检查点归档前应再次确认数量为 0。

本分支的目标不是把全局浏览器资源固定卡在 1 GiB。正确目标是：

1. 多任务并发的总吞吐随物理内存、CPU 和当前压力弹性扩展；
2. 单个用户可见任务的保留资源、并发、队列、标签页、子进程、HTTP 在途、下载和可归因内存均有边界；
3. task/runtime/Host/Lane 轮换不能“洗掉”同一任务的配额；
4. 正常、错误、取消、panic、进程崩溃和应用退出都必须把精确清理权威交给可持续重试的生命周期闭环；
5. 共享 Chromium Host 的物理 RSS/CPU 无法精确归因时必须如实表述，不能把采样估算伪装成 OS 硬配额。

## 2. 用户报告与根因地图

用户看到“一个 Agent 使用一个浏览器，但资源管理器出现很多浏览器进程且内存持续超过 1 GiB”。需要区分两部分：

- Chromium 多进程本身是正常设计：一个 Browser Host 会派生 renderer、GPU、network/service utility、crash handler 等多个进程；“进程数大于 1”本身不是泄漏证据。
- 本仓原实现存在会让进程、任务、队列、响应、profile 或 staging 资源不收敛的真实路径。重复触发后，正常的 Chromium 多进程模型会放大这些生命周期缺口，最终表现为进程数、RSS、容量压力和磁盘占用持续上升。

已定位并已处理或正在处理的主要根因如下：

### 2.1 任务身份与清理身份混用

- `TaskResourceFamilyKey` 是用户可见任务的资源计费边界；同一 conversation 可能有多个短命 runtime，它们必须共享配额。
- `RuntimeCleanupKey` / exact owner lease / exact Host epoch 是生命周期清理权威；清理不能按模糊 task/family 扩大范围。
- 旧实现的一些路径会在 runtime 或 Host 轮换时清空任务计费，另一些延迟清理会按过宽范围处理 sibling。
- 当前原则：family 只计费，exact runtime/owner/Host/Lane 才能清理。

### 2.2 Lane/Host 发布与取消窗口

- Scheduler admission 到 Lane 双索引发布之间曾有 await/cancel 窗口，可形成 active ghost，永久占用容量但不出现在 Lane inventory。
- queued promotion、cold launch、replacement publication、late driver publication、pending cleanup 和 Host finalization 均需要 RAII 或同步线性化。
- 已加入 unpublished admission guard、published restart guard、exact cleanup ledger 和 retained retry authority；相关代码集中在 `nomifun-browser-platform/src/hub.rs`。

### 2.3 启动、清理 worker 与 panic

- 启动取消、factory timeout、worker spawn 失败、cleanup worker panic、profile scan caller abort 都曾可能丢失 cleanup debt 或生成重复 worker。
- 当前采用 generation/live-generation、单 live worker、panic catch、指数退避、durable cleanup ticket/lease、独立 current-thread cleanup runtime，以及“完成结果单项邮箱”防止被取消的 waiter 丢失结果。

### 2.4 无界 payload / retained state

- CDP message、target/session/debug 标识、观察结果、页面正文/HTML、crawl schema/tree、可靠事件队列、审批队列、StorageState/Vault 等已增加结构与字节上限。
- 原 Primary 登录态捕获会对 IndexedDB 调用全量 `getAll()`，单次即可产生 GiB 级 renderer/CDP/serde 克隆链。现在 Managed Browser Use 严格 cookies-only；一般 StorageState 仅保留有界 cookies + localStorage，不再捕获 IndexedDB。
- 旧 vault 读取改成 `take(limit + 1)`，防 metadata TOCTOU；超限 fail-closed 且不覆盖旧文件。

### 2.5 Remote/Gateway/stdio 前置堆积

- 只在 Hub 深处限流不足以约束进入 Hub 前的 HTTP future、JSON body、slowloris、stdio response task 和 stdout 背压。
- Remote MCP 已增加 session/inflight/headerless initialize/body absolute deadline；Gateway 按 family 共享 8 个请求许可；stdio 有输入/输出/并发/handler deadline/request-id RAII/响应体流式上限。
- rmcp 1.7 完成响应 task 不能及时 reap 的第三方缺口目前用每 child 1024 请求 lifetime fuse fail-closed。第 1025 次要求重启 child；本仓无法证明所有外部客户端都能透明续接，后续应优先升级或修复 rmcp。

### 2.6 profile 与磁盘

- Anonymous profile 已有 512 MiB / 50k entries / 30 min / 256 navigation 的 Host 生命周期围栏、单飞采样、精确轮换和删除闭环。
- Primary 是稳定共享档案，原先不受 profile footprint 策略约束；站点可持续写 IndexedDB、CacheStorage、Service Worker、OPFS 等。
- 已把 64 MiB HTTP cache、32 MiB media cache、禁用 GPU shader disk cache 应用于所有平台托管 profile（Primary/Anonymous/Replica/Isolated），但这些 flags 不能限制整个 Primary origin storage。
- Primary 独立 2 GiB / 100k entries sticky fence 已收口（含故障注入测试）：首次操作前强制单飞采样，达界或检查失败后永久阻断本 Hub 生命周期内的新 Primary open/dispatch，精确停止旧 Host，不自动重启，不删除 Cookies/Local Storage/IndexedDB/Service Worker/CacheStorage/OPFS。详见第 5.1 节。

### 2.7 下载 staging 目录曾是跨 Host 数据破坏源

- 旧实现让**所有** managed Host 共享 `<data_dir>/downloads` 作为 Chromium
  `allowAndName` 落点（`derive_host_config` 恒置 `workspace_dir = None`，Host 级
  fallback 又回落 `data_dir`），且 5 分钟 mtime 扫描器只看文件年龄。
  结果：一个 Host 的扫描器可以删掉另一个 Host 正在写的慢下载；无 workspace 的
  Lane 会把 staging 当最终输出目录，于是**已完成的有效产物**与在写工件混在同一
  目录里，同样会被按年龄清掉。
- 现在 staging 是 per-exact-Host 独占目录，名字只由受信任的 root 进程身份
  （pid + platform start key）派生；扫描器跳过 active/retained-cancel GUID 且只
  扫本 Host 目录；无受管 workspace 时直接拒绝下载/PDF 输出而不是退化复用 staging。

## 3. 当前资源策略

### 3.1 全局弹性策略

全局不是固定 1 GiB：

| 预设 | Chromium 总 RSS 比例 | 单任务归因 watchdog | 单任务 active op | 单任务 Lane | 单任务 tab |
| --- | ---: | ---: | ---: | ---: | ---: |
| ResourceSaving | 30% | 768 MiB | 1 | 2 | 8 |
| Automatic | 40% | 1 GiB | 2 | 4 | 16 |
| HighConcurrency | 50% | 1 GiB | 2 | 4 | 16 |

当前机器实测约 63.75 GiB RAM、32 logical CPU。Automatic 启动策略可推导到 64 active operations、128 open lanes、约 25.5 GiB Chromium 总 RSS 压力边界，并保留最多 8 GiB 的系统内存 reserve。它不会因为机器更大而提高单任务额度。

注意：1 GiB 是共享 Host RSS 的任务归因 watchdog，不是 Windows Job Object/cgroup 的 per-task 硬 RSS。若需要严格物理隔离，只能每任务独立 Host/profile/process containment；这会增加基线进程与内存，是无法消除的结构性取舍。

### 3.2 其他单任务边界

- Browser MCP capability children：每 task family 最多 16。
- Browser stdio child：每 child 同时 1 请求，输入 256 KiB，输出 20 MiB。
- Browser family blocked-wire 结构包络：约 348,127,376 bytes（不含 allocator/进程固定开销）。
- Gateway stdio child：8 active，输入 8 MiB，输出 32 MiB。
- Gateway HTTP family in-flight：8。
- Remote MCP：普通 session 7 + 1 个保留 DELETE cleanup 槽；headerless initialize 4。
- crawl：页面正文 1 MiB、HTML 8 MiB、schema 64 KiB、深度 32、节点 4096、batch 256 KiB、最终结果 16 MiB。
- StorageState：JSON 16 MiB、cookies 4096、origins 32、每 origin localStorage 4096、localStorage renderer 侧 2 MiB UTF-16。
- CDP wire message：64 MiB。
- 平台结构上限仍包括 64 active operations、128 lanes、256 global queue；它们是防御性结构边界，不是用户要求的 1 GiB 总量限制。

## 4. 已完成并已有验证证据的工作

以下结论在“Primary/download 最后 WIP 增量”之前已通过对应测试；接手者仍需在 WIP 合并后重跑。

1. 硬件自适应 ResourcePolicy、单任务 memory/operation/Lane/tab 配额与 UI 诊断分层。
2. Scheduler admission/promotion ghost 修复；Lane/Host exact cleanup authority 和 panic-safe publication。
3. Windows Job/root PID/start-key descendant proof，Unix 对应 process containment 与恢复扫描。
4. launch cleanup quarantine：spawn 失败可重试、单 live worker、panic 退避、同一 debt 持续收敛。
5. profile footprint scan single-flight：caller abort 不会产生无界 spawn_blocking，也不会阻塞 exact shutdown。
6. cookies-only Managed identity；IndexedDB `getAll()` 生产路径删除；StorageState/Vault/restore 全链有界。
7. target/OOPIF/popup/replacement authority、CDP session/debug strings、可靠队列、crawl/observe/HTML/payload 全链上限。
8. Remote MCP/Gateway slowloris、body、session/inflight、persistent final cleanup retry。
9. stdio request registry、取消/EOF/panic/write deadline、JoinSet lifetime fuse、current-thread runtime、blocking pool=4。
10. 所有平台托管 profile 的 disk/media/shader cache 限制；standalone/external profile 不受误匹配。
11. UI 全量测试（本轮最新）：1666 pass、0 fail；`bun run check` 全部通过。

历史/本轮专项测试记录：

- `nomi-process-runtime`：209 tests 通过。
- `nomi-browser-engine --lib`：历史完整集 751 通过、9 ignored。
- `nomi-browser --lib`：历史完整集 262 通过、16 ignored。
- `nomifun-browser-platform --lib`：历史完整集 251 通过。
- StorageState：19/19；Vault：16/16；oversize 专项：9/9。
- cleanup quarantine：4/4。
- stdio bounds：14/14；runtime/双入口：7/7；Browser capability family：8/8。
- 下载初版专项：engine download 36 通过、1 ignored；platform adapter 36/36；Hub ledger 2/2。注意：这些测试在下述下载对抗审计问题出现前通过，不能作为最终下载实现的验收结论。
- UI：1666/1666，0 fail（2026-08-04 本轮重跑）。
- `bun run check`：通过（TypeScript/i18n/theme/icon/process-runtime-boundary/browser-platform-boundary/agent vocabulary/help）。
- `nomifun-common` 完整测试历史上只有 1 个与本任务无关的 Windows `zip_safe` 既有失败：期望 `a:b.md`，实际 `files/a:b.md`；不要误归因于 Browser Use。

冻结工作树后的最终归档闸门（2026-08-04）也已单独重跑：

- `cargo fmt --all -- --check`：通过。
- `git diff --check`：通过。
- `cargo check -p nomifun-browser-platform -p nomi-browser-engine -p nomi-browser`：通过，仅有检查点内已知的 dead-code 警告。
- `cargo check -p nomifun-app --features browser-use`：通过，仅有已知警告。
- `cargo test -p nomifun-browser-platform --lib task_download_ -- --test-threads=1`：3/3 通过。
- `cargo test -p nomi-browser-engine --lib download_terminal_ttl_and_shutdown_reconciliation_clear_state_and_staging -- --test-threads=1`：1/1 通过。
- `.github/workflows` 中 `.yml` / `.yaml` 数量：0。

本次归档没有在最终 WIP 增量后重跑真实 Chrome ignored 集和全部 Rust/UI 大集；这些仍是接手者完成下述 P1/P2 后的最终验收项，不能用上述编译与定向测试替代。

### 4.1 第二轮（WIP 收口后）重跑结果

- `cargo fmt --all -- --check`：通过。
- `git diff --check`：通过。
- `cargo check -p nomi-browser-engine --all-targets`：0 error。
- `cargo test -p nomi-browser-engine --lib`：**783 passed / 0 failed / 9 ignored**
  （检查点为 764 + 9 ignored；本轮新增 19 项，含 download 全套 52 项）。
- `cargo test -p nomifun-browser-platform --lib`：**263 passed / 0 failed**（原 251 + 8 Primary/下载新测试 + 4）。
- `cargo test -p nomi-process-runtime --lib`：117 passed / 0 failed。
- `bun run check`：全部通过。
- `bun run test:ui`：**1666 pass / 0 fail**（339 文件）。
- `.github/workflows` 中 `.yml` / `.yaml` 数量：0。

顺带修掉一个**先于本轮就存在**的测试脆弱性（在检查点提交上可稳定复现）：
`launch::tests::cleanup_quarantine_worker_panic_retries_the_same_retained_debt`
用「睡一半退避时间后断言无进展」探测退避，而 test-only 退避常量只有 10ms——
低于 Windows 计时器粒度（约 15.6ms），探测本身会睡过整个窗口而误报「在自旋」。
把 test-only 的 `BROWSER_CLEANUP_RETRY_INITIAL/MAX` 提到 80ms 后连跑 8 次全绿
（改前在检查点上 6 次里有 1 次失败，改后 8/8 通过）。这不影响生产退避常量。

**真实 Chrome ignored 集仍未在本轮增量后重跑**——它是唯一能证明「exact
process/profile residue = 0」与「stable profile sentinel 物理存活」的验收，仍
必须在本机（或任一有 Chrome 的机器）执行第 7 节的命令。

### 4.2 第二轮修掉的两处 WIP-A 附带破损

**(1) fake driver 继承 `profile_footprint` 默认值导致 Primary 被误 fence（已修）**

`BrowserHostDriver::profile_footprint` 有一个默认实现返回 `Ok(None)`，而 WIP-A
把 `Ok(None)` 判为 `footprint_measurement_unavailable` 并 **fail-closed 永久
fence Primary**。生产侧安全：`derive_host_config` 恒设
`config.user_data_dir = Some(profile)`（platform_adapter.rs:1507），所以真实
`ManagedEngineHostDriver` 永远走到实测分支，不会返回 `Ok(None)`；fail-closed
方向本身也是安全方向。但 **13 个测试 fake driver 全部继承了这个默认值**，于是
gateway/public/app/ai-agent 的 Primary 相关测试在检查点上成批失败（WIP-A 归档时
未重跑这些 crate，所以没被发现）。

修法：新增 `BrowserProfileFootprint::EMPTY`（一次**完成的**零测量，语义上区别于
`Ok(None)` 的「测不出来」），并在每个测试 fake 上显式覆写 `profile_footprint`
返回它——fake 本来就没有落盘 profile，这个测量是诚实的。生产策略与 fail-closed
语义**一字未改**。`nomifun-public` 的 fake 用的是手写 desugared future 风格，
其覆写也照该风格写。

**(2) `active_owner_bindings_scale_with_distinct_signed_tasks` 编码了前专项预期（已修）**

该测试用 64 个不同 `agent_id` 但**同一个 `CONVERSATION_ID`** 申请 owner lease，
断言 64 个全部成功。但本专项把 conversation 定为 task-resource family 边界
（§2.1），而 `MAX_ACTIVE_OWNER_LEASES_PER_PREBIND_FAMILY = 32`（lease.rs:18），
所以 64 个同 conversation 的 agent 属于**同一个 family**、共享 32 的额度——生产
行为正确，测试预期过时。按测试自己的断言语义（「MCP 不得对**独立**任务施加进程级
上限」）改为每个任务用**独立 conversation id**，即真正独立的 family，64 个即全部
通过。生产上限未改。

### 4.3 两处既有红项（本轮已按正确设计修完，生产语义未放宽）

**(1) `at_most_once_retries_undelivered_connection_failures` — 修测试，不动生产**

该测试 abort 掉本地 server 后立即（0ms 延迟）发起第一次尝试，期望被判为
「**可证明未送达**」从而重试。但 `JoinHandle::abort()` 是异步的——它只在下一个
await 点才真正取消任务，于是旧 listener 可能仍在 accept：连接被接受、请求写出、
随后连接被重置。**这种失败是真正歧义的**（字节可能已到达服务端），at-most-once
必须拒绝重试——生产判断完全正确。

排查过程中先排除了「连接池复用」假说：`build_bridge_http_client` 已设
`pool_max_idle_per_host(0)`（stdio_common.rs:1169），不存在 keep-alive 复用。

修法：新增测试辅助 `wait_until_connection_refused(port)`，在 abort 之后**先证明
端口确实拒连**，再发起调用。这样第一次尝试拿到的是真正的 connect 错误，测试断言
的正是它声称的那条分支。连跑 5 次全绿。`error.is_connect()` 判据与 fail-closed
语义**一字未改**——放宽它会让「可能已执行的浏览器动作」被重投。

**(2) 真实 Chrome 3 项 4-Lane 冲突 — 让测试改用生产模型，不抬高配额**

`managed_host_real_chromium_acceptance_matrix`（一个 Host 上 4 overlap + serial +
primary-a/b 共 7 个 Lane）、`managed_host_sixteen_lane_real_chromium_acceptance`
（16 Lane）、`shared_host_rss_is_materially_below_four_independent_hosts`
（shared 4 Lane，且多 Host/多 round 共用**进程级 compatibility scope**累积）全部
撞 `STANDALONE_MAX_LIVE_LANES_PER_SCOPE = 4`。

关键判断：这个 4 **不是历史债务，而是产品的单任务 Lane 预算**（§3.1 表格里的
「单任务 Lane 4」）。抬高它等于为了让测试变绿而削弱本专项要建立的每任务边界，
**不能做**。真正的错配在于：standalone 路径的语义是「一个 Host 绑一个任务 scope」，
而这三个验收测试要证明的是「**一个受管 Host 为多个独立任务复用 Lane**」——那正是
生产里的 `PlatformManaged` 模型（Lane 容量由 Hub scheduler 拥有，见
`TaskTabReservationAuthority::release_lane` 的 no-op 默认）。

修法：三个测试改用 `ManagedBrowserHost::launch_platform_managed`，并像平台适配器
那样为每条 Lane 传入受信任 task key + tab/download authority；测试内新增
`HubStandInTaskAuthority` 作为 Hub 替身（每 Lane 一个独立任务族）。per-task 的
tab/download 配额本身仍由 engine 的 standalone scope 与 Hub 的真实实现单测覆盖，
这里不重复。**没有改动任何配额常量。**

顺带纠正一个仓库既有误解：`[dev-dependencies]` 里为「外部集成测试 crate 不继承普通
依赖」而重复列出的条目是**不必要**的——Cargo 会把普通依赖的 `--extern` 一并传给
test target。本轮实测确认集成测试无需重复声明 `async-trait` 即可编译，因此没有新增
dev-dependency（`chromiumoxide` 那条既有条目同理是无害的冗余，未在本轮改动）。

### 4.4 真实 Chrome ignored 集（本轮已跑，6/6 全绿）

```
NOMIFUN_CHROME_BINARY='C:\Program Files\Google\Chrome\Application\chrome.exe'
cargo test -p nomi-browser-engine --test integration_managed_host -- --ignored --test-threads=1
```

**6 passed / 0 failed**（73s）：

- `managed_host_drop_reaps_full_tree_and_ephemeral_profile`：
  `residual_pids=[] profile_removed=true provisional_marker_removed=true
  committed_record_removed=true`；
- `managed_host_shutdown_clears_only_exact_runtime_profile_artifacts`：
  **stable profile sentinel 在 exact shutdown 后保留**（第 5.1 节项 7 的物理验收）；
- `create_engine_drop_reaps_hidden_host_runtime_and_allows_stable_relaunch`；
- `managed_host_real_chromium_acceptance_matrix`（跨 Lane 重叠/同 Lane 串行/身份隔离）；
- `managed_host_sixteen_lane_real_chromium_acceptance`（16 Lane 共享一个 Host 进程）；
- `shared_host_rss_is_materially_below_four_independent_hosts`（共享 Host 对 4 独立 Host 的 RSS 对照）。

运行前后按 executable path 精确统计 Chrome：**前 0、后 0**；per-Host 独占 staging
子目录残留 **0**（只剩可复用的空 staging 根目录，符合设计）。

### 4.5 一个环境性间歇（不是代码缺陷）

真实 Chrome 套件刚跑完后紧接着跑 `nomifun-app` 全量时，观察到 2 次单发失败，
每次换一个不同测试（`revoked_binding_sweep_...`、
`oversized_body_is_rejected_before_task_admission`），失败点都是 loopback
`reqwest ... .send().await.unwrap()` 而**不是断言**——即 Chrome 进程刚退出留下的
socket/handle 压力。隔离重跑 **3 次全部 445 passed / 0 failed**。跑 app 全量前先让
Chrome 完全退出即可。

## 5. WIP 收口结果（2026-08-04 第二轮接手）

两条 WIP 均已实现并有测试证据。下面保留原设计说明，并在每条末尾记录实际落地状态。

### 5.1 WIP-A：Primary stable profile sticky fence — 已完成

当前设计已确定，代码已接线并有故障注入/竞态测试：

归档时实际状态：生产接线已经完成到可编译检查点；`cargo check -p nomifun-browser-platform`、`cargo check -p nomifun-app --features browser-use`、platform `--no-run`、既有 Anonymous profile 4 项回归、fmt 和 diff check 均通过。尚缺的是下列 Primary 专项故障注入/竞态测试，因此仍按 WIP 交接，不能标为发布完成。

- `PrimaryProfilePolicy`：默认/硬上限 2 GiB、100k entries，15s 周期，首次实际 operation 必须强制采样。
- sticky fence 与 exact cleanup epoch 集分离：
  - sticky fence 在当前 Hub 生命周期内绝不清除；
  - exact cleanup epoch 只保留尚未证明 process tree 消失的 Host epoch；
  - cleanup 完成后删除 exact epoch，但不解除 sticky fence。
- open 必须在 scheduler admission 前拒绝；`get_or_launch_host` 在 open gate 外/内双检查；restart/visibility/resource emergency 中央 launch 入口也必须检查，防绕过。
- execute 首次操作前复用现有 profile sample flight/mailbox；caller abort 不得取消 native scan 或丢失超限结果。
- periodic sweep 覆盖静默 Primary Host，但必须跳过 driver 尚未发布的 cold launch，不能误永久 fence。
- 达界后：同步 publish fence，drain admission，detach 全部 Primary Lane，停止 exact Host；不 replacement、不删除 profile。
- cleanup failure/panic：保留 exact epoch 和 orphan authority，由 supervisor 有界退避重试。

必须至少通过这些测试：

1. `primary_first_observe_samples_and_fences_before_dispatch`
2. `primary_public_sweep_samples_silent_host`
3. `primary_sticky_fence_blocks_existing_and_new_open`
4. `primary_shutdown_failure_retries_exact_epoch_without_replacement`
5. `primary_worker_panic_rearms_exact_cleanup`
6. visibility/resource restart 不能绕过 sticky fence
7. stable profile sentinel 在 exact shutdown 后保留

**实际落地（2026-08-04 第二轮）**：全部 7 项已写入
`crates/backend/nomifun-browser-platform/src/hub.rs` 的 `mod tests` 并通过。
项 6 的测试名为 `primary_visibility_restart_cannot_bypass_sticky_fence`；项 7 在
Hub 层实现为 `primary_fence_preserves_identity_data_and_never_deletes_the_profile`
（断言 fence 只精确停 Host、不 replacement、错误 metadata 诚实声明
`automatic_profile_deletion=false` / `persistent_identity_preserved=true`）——
**物理 sentinel 存活仍由真实 Chrome `integration_managed_host` ignored 集断言，
Hub 层无 profile 删除权威，不能替代该验收**。额外补了
`primary_cold_launch_is_never_fenced_by_the_sweep`（driver 未发布的 cold launch
不被 sweep 误永久 fence）。

### 5.2 WIP-B：下载 task-family 配额最终闭环 — 已完成

初版目标：单 task family 累计 1 GiB、单文件 512 MiB、256 completed files、4 active downloads；Host/runtime/Lane 轮换不能重置；`save_as_pdf` 同一账本。

归档时（第一轮）实际状态：sticky completed-family ledger、4096 family 上限、两阶段 reservation trait/Hub/standalone/bridge、Lane unregister/TTL retained-cancel、5 秒无终态 poison+shutdown 已实现。下列缺口在第二轮全部补齐：

- ~~`CdpHostRuntime::Drop` 尚未把 router/reservation 所有权挂到 `DurableProcessCleanup` ticket~~ →
  新增 `launch::HostStopReconcile` trait 与 `PostStopReconcileCell`：Host 的下载账本
  （`HostDownloadLedger`：路由表 + 每条 `PendingDownload` 持有的 reservation + 独占
  staging 目录）从 `HostTargetRouter` 中拆出，由 `DurableProcessCleanup` 持 `Arc`。
  reconcile **只在** `clean_dropped_browser_once` / `clean_reclaimable_dropped_browser`
  / 直接 `finish()` 证明 exact 进程树停止（且 profile artifact 清理成功）之后运行一次；
  panic 被 `catch_unwind` 包住，不会把已证明的进程清理反转成未证明。Host Drop 同时把
  download loop 的 JoinHandle 交给 relay 的 `runtime_tasks`，relay 先 settle 任务再
  终止进程，最后 reconcile——Arc 不可能早于停止证明释放 active 配额。
- ~~progress lag/Closed 受同一 Drop guard 缺口影响~~ → 同上修复覆盖。
- ~~`finish_download` / `save_as_pdf` 未接入 prepare/finalize 两阶段与 RAII/no-clobber 发布事务~~ →
  新增 `download::publish_task_output`：`prepare_complete(实际字节)` → 唯一同卷
  `.nomifun-<nonce>.part`（`create_new`，不覆盖）→ hard-link 原子发布（不覆盖既有
  产物，退化文件系统走 exists-guarded rename）→ 无 await `finalize_complete()`。
  失败路径删除 temp 并放弃预留（drop reservation 即退还 active 记账）；temp 删不掉时
  **提交计费**并把路径作为保留清理债返回（`TaskOutputPublishError { charged, residual }`），
  绝不留下未计费产物。`finish_download` 与 `act_save_as_pdf` 都走这一条路径。
- ~~exact Host 唯一 staging、无 workspace 拒绝、scanner 跳过 active/retained GUID~~ →
  staging 从共享 `<data_dir>/downloads` 改为 `<data_dir>/download-staging/host-<pid>-<start_key>`
  （名字只由**受信任的 root 进程身份**派生）；Host 无该身份时 `Browser.setDownloadBehavior`
  设为 `deny` fail-closed，绝不回落用户 Downloads。Lane 最终输出目录仅在有受管
  workspace 时存在，否则 `begin_download` 拒绝准入。mtime 扫描器现在跳过 active 与
  retained-cancel GUID（含 `.crdownload`/`.tmp` 变体），且只扫本 Host 目录；reconcile
  无残债后删除整个独占目录。启动时 `sweep_orphan_host_staging_dirs` 按
  `probe_process_identity` 的**确证死亡**（无进程或 start key 不符=PID 回收）清理硬杀
  残留，探测出错或身份仍存活一律不动。
- ~~fake cancel failure + continuous write、publish rollback/delete failure、双 Host staging 动态测试~~ →
  见下方测试清单。

最终对抗审计的问题状态：

#### P1：zero-owner gap 清空同任务账本

- Gateway、ACP、User login、System browser fetcher、Native renew-failure 都可能先 revoke 旧 owner，再在稍后 bind 新 owner。
- 因此不能只逐调用点做 overlap handoff，也不能在 owners=0 时删除 completed ledger。
- 已定方案：Hub 生命周期内 sticky completed-family ledger；同 family 后续 owner 原子继承；Hub shutdown 才统一清。
- ledger 自身必须有结构上限：最多 4096 inactive completed families；达到上限时新 family fail-closed，绝不 TTL/LRU 驱逐已经消费的 quota。active-only、从未消费的空 family 可回收。

#### P1：取消/TTL 先释放 reservation，Chromium 仍可能写盘

- Lane unregister 和 TTL 旧路径先从 router 删除 route，Drop reservation 释放 active quota，再 best-effort `Browser.cancelDownload`。
- cancel 失败或没有终态时，Chromium 可继续写 staging；反复 reopen 可绕过 active=4。
- 修复要求：进入 retained-cancel state，reservation 一直保留；仅 `downloadProgress` terminal event 或 exact Host/process stop 证明后释放。cancel ack 本身不是终止证明。
- bounded grace 后仍无终态：poison/fence Host 并精确 stop；不能继续复用 Host。

#### P1：所有 Host 共享 staging，scanner 跨 Host 删除有效文件

- 旧 managed template 让所有 Host 使用同一个 `<data_dir>/downloads`。
- 无 workspace Lane 还会把 staging 当最终输出目录；5 分钟 scanner 可删除有效 PDF/下载或另一 Host 的慢下载。
- 修复要求：每 exact Host/epoch 独占 staging；路径只能由可信 host id/epoch 派生；无受管 workspace 时直接拒绝文件/PDF输出；scanner 只处理本 Host staging 并跳过 active/retained-cancel GUID；最终产物永不由 staging scanner 删除。

#### P2：目标发布失败却永久消耗 quota

- 旧 `finish_download` / `save_as_pdf` 先 `complete()`，后 rename/copy/write；目标不可写时零产物但 quota 永久消耗。
- 修复要求：唯一同卷临时文件（create_new，不覆盖）→ 写/复制完成 → 原子发布 → 无 await 地 complete；RAII guard 在 complete 前 error/cancel/panic 删除 temp/已发布文件。
- complete 失败后的删除也可能失败（Windows lock/AV）。必须保留 exact cleanup debt/fence 并阻止 family 继续下载，或实现两阶段 charge；不能静默留下未计费产物。

下载最终验收至少包括：

1. zero-owner → same-family rebind 循环仍继承 1 GiB；第 4097 inactive family fail-closed；
2. cancel command error、ack 无 terminal、TTL、Lane unregister 均保持 reservation，最终 Host stop 后才释放；
3. 双 Host staging 不相同且不能互删；无 workspace 明确拒绝；
4. progress NaN/negative/non-finite/伪 total fail-closed；
5. publish/create/write/rename/complete/delete-cleanup failure 的事务测试；
6. completed metadata 用实际 size，不能相信 CDP total；
7. `save_as_pdf` 使用同一 family ledger，CDP 64 MiB wire cap仍在打印响应前提供上界。

**第二轮新增测试与覆盖对应**（全部通过）：

| 验收项 | 测试 |
| --- | --- |
| 1 | `task_download_rebind_cycles_inherit_cumulative_cap_until_hub_clear`（hub.rs，4 轮真实 zero-owner rebind 后第 5 次被 1 GiB 拒绝，`clear()` 后新生命周期恢复）+ 既有 `task_download_sticky_family_table_fails_closed_at_structural_limit` |
| 2 | `cancel_failure_retains_reservation_until_host_stop_finalization`（fake 让 `Browser.cancelDownload` 返回协议错误：路由/reservation 保留、Host 准入毒化、工件不被 sweep 清、只有 host-stop 终态化才释放）、`cancel_ack_without_terminal_event_poisons_after_bounded_grace`、`handed_off_cleanup_retains_download_reservations_until_exact_stop_proof`（真 relay 证明进程树停止后才释放并删独占 staging）+ 既有 TTL 测试 |
| 3 | `staging_sweep_skips_owned_guids_and_never_crosses_hosts`、`lane_without_workspace_denies_download_admission_fail_closed`、`host_staging_dir_name_round_trips_and_rejects_foreign_names`、`orphan_staging_sweep_requires_exact_dead_process_proof` |
| 4 | 既有 `download_progress_numeric_conversion_fails_closed`；伪 total 由 `finish_download` 改用**实际 metadata 字节**记账后失去意义（见项 6） |
| 5 | `publish_task_output_prepare_denial_produces_no_artifact`、`publish_task_output_never_clobbers_and_rolls_back_on_full_collision`、`publish_task_output_undeletable_residual_keeps_the_charge`、`publish_task_output_moves_staged_file_and_consumes_source`、`finish_download_publish_failure_rolls_back_charge_and_releases_reservation` + 既有 delete-cleanup 保留债测试 |
| 6 | `finish_download_two_phase_charges_actual_size_and_never_clobbers`（断言 `prepare_complete` 收到的是落盘 `metadata().len()`，且既有同名产物不被覆盖） |
| 7 | `standalone_download_*` 5 项（host.rs：4 active 上限与 Drop 归还、512 MiB/1 GiB 边界、prepare/finalize 幂等与不双扣、同 key 幂等与 4 KiB key 界、completed 计费跨 Lane 轮换存活）；`act_save_as_pdf` 现与浏览器下载共用 `publish_task_output` 与同一 family ledger |

## 6. 接手后的第一轮检查

按顺序执行，避免多个 Cargo 进程争抢同一个 target，并先修编译再跑大集：

```powershell
git switch codex/browser-use-task-resource-hardening
git status --short
git config --local user.name
git config --local user.email
git config --local --get core.hooksPath

cargo fmt --all -- --check
git diff --check
cargo check -p nomifun-browser-platform -p nomi-browser-engine -p nomi-browser

cargo test -p nomifun-browser-platform --lib primary_ -- --test-threads=1
cargo test -p nomifun-browser-platform --lib task_download_ -- --test-threads=1
cargo test -p nomi-browser-engine --lib download -- --test-threads=1
cargo test -p nomi-browser --lib platform_adapter -- --test-threads=1
```

然后执行关键完整集：

```powershell
cargo test -p nomi-process-runtime --lib -- --test-threads=1
cargo test -p nomifun-browser-platform --lib -- --test-threads=1
cargo test -p nomi-browser-engine --lib -- --test-threads=1
cargo test -p nomi-browser --lib -- --test-threads=1
cargo test -p nomifun-gateway --features browser-use --lib -- --test-threads=1
cargo test -p nomifun-public --features browser-use --lib -- --test-threads=1
cargo test -p nomifun-app --features browser-use --lib -- --test-threads=1
cargo check -p nomifun-app --features browser-use
bun run check
bun run test:ui
```

不要在多人并行编辑时运行仓库根 `bun run test`。该脚本会先运行 `scripts/prune-build.mjs`；本轮误启动后及时停止，但已经删除 992 个陈旧 incremental entry 和 `build.noindex/tmp`。这些只是可重建编译缓存，没有删除源码或用户数据，但会让下一次 Rust 编译变慢。

静态扫描：

```powershell
rg -n "\.getAll\(" crates/agent/nomi-browser-engine crates/agent/nomi-browser
rg -n "handoff_bound_task_cleanup|handoff_global_cleanup|cleanup_bound_task_lanes|cleanup_drain_scopes" crates
Get-ChildItem -LiteralPath .github\workflows -File -Force -ErrorAction SilentlyContinue |
  Where-Object { $_.Extension -in '.yml', '.yaml' }
```

预期：前两条源码扫描无生产命中；workflow YAML 数量为 0。

## 7. 真实 Chrome 验收（当前检查点尚未重跑）

本机 Chrome：`C:\Program Files\Google\Chrome\Application\chrome.exe`，归档前基线 exact process count=0、working set=0，版本曾观测为 150.0.7871.187。

```powershell
$env:NOMIFUN_CHROME_BINARY = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
cargo test -p nomi-browser-engine --test integration_managed_host -- --ignored --test-threads=1 --nocapture
```

该 ignored 集覆盖：

- stable profile normal shutdown 只清 runtime artifacts，保留 profile sentinel；
- Drop reaps 整棵 Chromium tree 并删除 ephemeral profile；
- hidden `create_engine` Host Drop；
- Primary/Anonymous identity/profile 隔离；
- 16 Lane 共享一个 Host；
- 共享 Host 与 4 个独立 Host 的 RSS 对照。

验收后必须用 executable path 精确统计 Chrome，而不是按进程名误算用户自己的 Chrome/Edge：

```powershell
$chrome = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
Get-CimInstance Win32_Process |
  Where-Object { $_.ExecutablePath -eq $chrome } |
  Select-Object ProcessId, ParentProcessId, CreationDate, CommandLine
```

预期：测试结束后新增 exact Chrome process=0；没有命令行仍引用测试 profile；ephemeral profile 残留=0。

## 8. 当前已知环境残留

- `C:\Users\Developer\AppData\Local\Temp\.tmpOmCP6h`
- 当前测得约 85,709,895 bytes，主要是被强杀的 Chrome-for-Testing 下载 `.staging/.../chrome.part` 和 acquire lock。
- 这是测试进程被硬终止、绕过 Rust Drop 后留下的旧临时目录；新代码已有 staging guard 和 stale sweep。
- 本轮两次按精确绝对路径尝试删除都被执行策略拒绝，没有绕过，也没有声称已经删除。接手者可在允许的终端中先验证路径确实位于 `%LOCALAPPDATA%\Temp`，再手工删除该精确目录。

## 9. 关键代码地图

- 聚合/单任务资源策略：`crates/backend/nomifun-browser-platform/src/resource.rs`
- Scheduler、Hub、profile fence、task download ledger、exact cleanup：`crates/backend/nomifun-browser-platform/src/hub.rs`
- Hub/engine authority traits：`crates/backend/nomifun-browser-platform/src/driver.rs`
- cleanup ledger：`crates/backend/nomifun-browser-platform/src/cleanup_budget.rs`
- managed Host/profile 派生：`crates/agent/nomi-browser/src/platform_adapter.rs`
- process launch/Job/cleanup quarantine/cache flags：`crates/agent/nomi-browser-engine/src/launch.rs`
- CDP router、target/session、download/staging：`crates/agent/nomi-browser-engine/src/backend/cdp.rs`
- standalone task scope：`crates/agent/nomi-browser-engine/src/host.rs`
- PDF/download actions：`crates/agent/nomi-browser-engine/src/actions.rs`
- StorageState/Vault：`crates/agent/nomi-browser-engine/src/storage_state.rs`、`vault.rs`
- stdio bounds：`crates/backend/nomifun-app/src/commands/stdio_common.rs`
- Gateway admission：`crates/backend/nomifun-gateway/src/server.rs`、`browser_registry.rs`
- Remote MCP：`crates/backend/nomifun-public/src/router.rs`、`session.rs`
- Windows/Unix process proof：`crates/shared/nomi-process-runtime/src/platform/`
- UI policy/diagnostics：`ui/src/common/browser/`、`ui/src/renderer/pages/browser/`

## 10. 重要经验与防回归约束

1. 不要把 Explorer 中多个 Chromium helper process 直接判为泄漏；先按 exact root PID/start key/profile/Job membership 证明 ownership，再看生命周期是否收敛。
2. task family 只能计费，不能作为清理 authority；否则一个旧 runtime 可能清掉 replacement 或 sibling。
3. owner 数暂时为 0 不等于用户可见任务结束。续租、child restart、登录重开和知识抓取都有合法 zero-owner gap。
4. `cancelDownload` command ack 不是下载停止证明；只有 terminal event 或 exact process stop 才能释放 active reservation。
5. filesystem publish 与 quota commit 必须是补偿事务；任何 error/cancel/panic 窗口都不能留下未计费产物或无产物永久扣费。
6. staging 必须有 exact owner；共享目录 + mtime scanner 是跨 Host 数据破坏源。
7. profile scan 的 `spawn_blocking` 不能随被取消 waiter 重复创建；保留一份 active/completed mailbox。
8. stable Primary profile 可能包含不可恢复的设备绑定/离线状态。不能把 Cookies、Local Storage、IndexedDB、Service Worker、CacheStorage、OPFS 当普通 cache 自动删除。
9. Chromium flags 只能限制 HTTP/media/shader cache，不能证明整个 profile 或单任务 origin storage 的硬上限。
10. 共享 Host 下 per-task RSS/CPU 只能估算；若产品要求严格物理硬限，应显式选择 per-task Host + Job Object/cgroup，而不是在指标文案中夸大保证。
11. 所有 worker 的“已启动”标志必须只在真实 worker 存活时成立；spawn failure/panic 后必须允许同一 debt re-arm，且避免 Drop 中递归 spawn。
12. 运行完整测试前先跑 targeted compile；多人共享 target 时并行 Cargo 往往更慢且会掩盖真正的编译错误。

## 11. 完成定义

只有同时满足以下条件，才能把专项从 WIP 改为完成：

- ~~Primary sticky fence 的 open/execute/restart/sweep/exact cleanup 全路径测试通过~~ ✅ 第 5.1 节，8 项通过；
- ~~下载的 3 个 P1 + 1 个 P2 对抗用例通过~~ ✅ 第 5.2 节表格，全部通过；final adversarial review 仍应在推送前跑一次；
- ~~Rust 格式、diff、关键完整 crate、UI、边界脚本全部通过~~ ✅ 第 4.1 节 + 第 4.3 节修复后：engine 783 / platform 263 / nomi-browser 252 / ai-agent 867 / gateway 211 / public 41 / **app 445（连跑 3 次 0 失败）** / process-runtime 117 / UI 1666 / `bun run check` / `cargo fmt --all --check` / `git diff --check` 全部通过，**无红项**；
- ~~真实 Chrome normal/error/Drop/16-Lane/RSS 测试结束后 exact process/profile residue 为 0~~ ✅ 第 4.4 节：**6/6 全绿**，exact process residue 前后均为 0、per-Host staging 残留 0；
- `.github/workflows` YAML 为 0（已确认 0）；
- 最终提交作者/提交者是负责人类开发者，无 AI attribution；
- 分支已经推送到 origin，另一台电脑可直接 fetch/switch；
- 发布说明诚实保留共享 Host 物理归因、rmcp 1024 fuse、Primary persistent storage 与 OS filesystem quota 的结构边界；新增：**per-Host 独占 staging 只保证目录归属与清理权威，不构成对 Chromium 写入速率或磁盘总量的 OS 级配额**。

## 12. 本轮新增/改动的关键代码位置

- `crates/agent/nomi-browser-engine/src/launch.rs`
  - `HostStopReconcile` trait、`PostStopReconcileCell`（停止证明后跑且只跑一次的 reconcile 挂点）；
  - `PendingDroppedBrowserCleanup` / `ReclaimableDroppedBrowserCleanup` 新增
    `post_stop_reconcile` 字段；两条 cleanup 路径在证明成功后调用它；
  - test-only 退避常量 10ms → 80ms（Windows 计时器粒度）。
- `crates/agent/nomi-browser-engine/src/backend/cdp.rs`
  - 新增 `HostDownloadLedger`（路由/拒绝表/独占 staging/清理债/毒化位/`downloads_finalized`
    终态围栏），`HostTargetRouter` 改为持 `Arc<HostDownloadLedger>` 并委托；
  - `impl HostStopReconcile for HostDownloadLedger`：终态化 → 释放保留 reservation →
    残债为 0 时删除独占 staging 目录；
  - `DurableProcessCleanup` 新增 `post_stop_reconcile` 槽 + `install_post_stop_reconcile`；
    `hand_off_with_runtime_tasks` 移交它，`finish()` 在成功后运行它；
  - `from_launched` 派生 per-Host staging + 启动孤儿清扫 + 无身份时
    `set_download_behavior_deny` fail-closed；`launch_in_mode` 不再用 `data_dir` 当下载落点；
  - `finish_download` 改为两阶段发布事务；`sweep_stale_staging_files_at` 跳过被持有的 GUID；
  - `CdpHostRuntime::Drop` 把 download loop 句柄交给 relay 的 `runtime_tasks`。
- `crates/agent/nomi-browser-engine/src/download.rs`
  - `download_staging_root` / `host_staging_dir_name` / `parse_host_staging_dir_name` /
    `sweep_orphan_host_staging_dirs`（确证死亡才删）；
  - `publish_task_output` + `TaskOutputPayload` + `TaskOutputPublishError`（两阶段补偿事务）。
- `crates/agent/nomi-browser-engine/src/actions.rs`：`act_save_as_pdf` 走 `publish_task_output`。
- `crates/agent/nomi-browser-engine/src/host.rs`：新增 5 项 standalone 下载配额测试。
- `crates/backend/nomifun-browser-platform/src/hub.rs`：新增 7 项 Primary fence 测试 + 1 项
  rebind 累计配额测试。
- `crates/backend/nomifun-browser-platform/src/driver.rs`：新增
  `BrowserProfileFootprint::EMPTY`（完成的零测量，区别于 `Ok(None)` 的测不出来）。
- 13 个测试 fake host 显式覆写 `profile_footprint`（gateway ×3 含集成测试、app ×6、
  public ×1、ai-agent ×1 等），修掉 WIP-A 的 fail-closed 默认值成批 fence 测试的问题。
- `crates/backend/nomifun-app/src/browser_mcp_server.rs`：
  `active_owner_bindings_scale_with_distinct_signed_tasks` 改用独立 conversation。
- `crates/backend/nomifun-app/src/commands/stdio_common.rs`：新增测试辅助
  `wait_until_connection_refused`，让 at-most-once 重试测试先建立「端口确实拒连」
  的前提，不再与异步 abort 竞态（生产判据未改）。
- `crates/agent/nomi-browser-engine/tests/integration_managed_host.rs`：新增
  `HubStandInTaskAuthority` + `platform_lane_config`，三个多 Lane 验收测试改用
  `launch_platform_managed`（生产的多任务共享 Host 模型），不再受 standalone
  单任务 4-Lane 预算约束（未改动任何配额常量，也未新增 dev-dependency）。
