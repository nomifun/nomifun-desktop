# Linux 命令与会话可靠性进度

更新：2026-10-06（首轮 WSL2 执行）。宿主为 WSL2 Ubuntu 26.04（内核
`6.18.33.2-microsoft-standard-WSL2`），源码与夹具使用原生 ext4 路径（`/home/muri/...`，
`/dev/sdd`），`/mnt/c` 为 9p 挂载仅作对照、不作文件语义证据。

规则见 [实施计划](IMPLEMENTATION-PLAN.zh.md)，公共根因引用
[共享进度](PROGRESS-SHARED.zh.md)，Case 定义见
[HIGH-ORDER-OPERATIONS-CASE-CATALOG.zh.md](HIGH-ORDER-OPERATIONS-CASE-CATALOG.zh.md)。
本页按用户 2026-10-03 要求建立 Linux 走查清单；Linux 结果不代判 Windows/macOS，
反之亦然；单平台不存在的 Case 记 `N/A + 原因`，不记 pass。

## 2026-10-06 首轮执行记录（WSL2 主链路）

- 基线：HEAD `7fe9ec62ef875bbec2d16735d0b53fcb4d708306`；执行前工作树已有
  `IMPLEMENTATION-PLAN.zh.md` 本地修改与本文档（未跟踪），无其他源码改动。
- 环境：本轮前 WSL 无任何工具链；按用户选择在用户态安装 rustup stable `1.99.0`、
  bun `1.3.1`、node `22.21.0`，并由用户执行 `apt` 安装完整编译依赖（含
  `libwebkit2gtk-4.1-dev`、`libayatana-appindicator3-dev`、`libpipewire-0.3-dev` 等，
  清单同 `docs/guides/desktop-app.zh.md` + Dockerfile 构建期依赖）。
- 证据：测试完整输出在仓库外 `/home/muri/agent-reliability-evidence/linux/*.log`。
- GUI 探测：`WAYLAND_DISPLAY=wayland-0` 与 `/mnt/wslg` 存在，WSLg 初步可达；
  D-Bus 会话总线无 `org.kde.StatusNotifierWatcher`/AppIndicator 宿主，
  **托盘 Quit 入口在 WSLg 记 `N/A`**（见 L-OUT-03），不以窗口关闭冒充托盘退出。
- 挂载：`/`=ext4、`/tmp`=tmpfs、`/mnt/c`=9p（对照）。

本轮发现（首败保留，修复只跑最小相关回归）：

| # | 发现 | 范围 | 处置 |
| --- | --- | --- | --- |
| L-F1 | handoff manifest 基线偏移：`--check` exit 1，`base_commit` `296dc899f` 是 HEAD 祖先（上游 12 提交）但非 HEAD/parent | L-P0-01 | 记 FAIL；按脚本纪律不重新生成 manifest，属交接快照超窗的设计内拒绝 |
| L-F2 | 契约清单 `cargo_lock_digest` 过期：checked-in `739c4f93…`，当前 Cargo.lock `79f38c87…`；`agent-v2-contract write` 再生仅差此 digest（3 个联动文件） | L-P0-04 | 记 FAIL；根因为 `2e6a2c7c1` 改 Cargo.lock 未再生清单，跨平台同败、非 Linux 问题；工作树已还原，修复=按提示跑 `write` |
| L-F3 | `architecture_contract::windows_conpty_global_state_tests_share_one_serial_group` 计数过期：`1d20fad57` 新增 serial 测试（`setup_deadline_during_assignment_never_resumes_or_restarts_the_timeout`）未同步 22→23，跨平台同败 | L-P1-06 | 已修（计数 23，新测试本就在组内）；重跑该目标 16/16 通过，随后 `--tests` 全量绿 |
| L-F4 | `watch_service::tests::deleted_office_workspace_can_be_stopped_through_its_original_alias` 竞态 FAIL：被删目录的 inotify wd 由内核摘除后，迟到的 `unwatch` 得 `EINVAL`，`confirm_unwatch` 误报 "cleanup unconfirmed"（单跑 5/5 通过，全量并发下触发） | nomifun-file / LNX-013 | 已修：`#[cfg(target_os = "linux")]` 下 `Io(EINVAL)`（wd 已失效=已摘除的确证）按已清理处理，经 `unwatch_error_is_confirmed_removal` 纯函数承载并有确定性单测；`cargo test -p nomifun-file` 全绿 |
| L-F5 | 文档旧口径过期："Agent Store migration head=6" 已被 canonical clean-cut 取代：现单 baseline `001_canonical_baseline.sql`、`AGENT_STORE_MIGRATION_HEAD=1`；未知/部分 lineage 由 `validate_current_migration_lineage` fail-closed（`database.rs:178-236`） | L-P1-08 | 记录口径更新；旧 005/006/007 保留语义不再适用，不记 pass 也不记 fail |

两处源码修复（`architecture_contract.rs` 计数、`watch_service.rs` EINVAL 分支）
留在工作树未提交，与既有本地修改并存。

产物：`target/x86_64-unknown-linux-gnu/release/bundle/{appimage,deb,rpm}` 三件套与
`release/nomifun-desktop`（ELF x86-64 stripped）已产出，SHA-256 记录在仓库外
`/home/muri/agent-reliability-evidence/linux/LNX-016-artifacts.sha256`。

## 0. 宿主能力边界（先决核对，决定可验范围）

| 能力 | WSL2 可达性 | 对走查的影响 |
| --- | --- | --- |
| Cargo/Node 定向测试、ext4 文件与进程语义 | 可验 | P0～P3、LNX 文件/进程/Shell/权限项正常执行 |
| Tauri GUI（经 WSLg） | 已探测：`wayland-0`/`/mnt/wslg` 存在，初步可达 | 正式会话仍需构建产物与凭据；本轮未排正式 UI |
| 托盘/AppIndicator | 已探测：D-Bus 无 `StatusNotifierWatcher`/AppIndicator 宿主 | 退出入口项记 `N/A`（L-OUT-03），不以窗口关闭冒充托盘 Quit |
| 系统 S3 sleep/wake、OS reboot | 不可验 | 列入"范围外"，需真实 Linux 机器，不以 WSL2 结果替代 |
| Computer-use 屏幕/输入 | WSLg Wayland 语义与真机 X11/Wayland 均不同 | 只可标 `WSL2 lane` 结果；真机结论仍开放 |
| 真实凭据/网络模型调用 | 可验 | 沿用加密 StepFun 配置与凭据隔离 runner |

## 1. 走查清单

### L-P0 源码接收、编译与契约（不调模型）

| ID | 内容 | 命令 | 状态 |
| --- | --- | --- | --- |
| L-P0-01 | 源码交接完整性 | `node scripts/validation/agent-reliability-handoff.mjs --check` | FAIL（L-F1 基线偏移，设计内拒绝） |
| L-P0-02 | runtime/session 测试编译 | `cargo check -p nomifun-agent-runtime -p nomifun-agent-session --tests` | 已验 PASS（56s，nomi-process-runtime 2 warnings） |
| L-P0-03 | 恢复测试目标编译 | `cargo check -p nomifun-app --lib --test native_execution_recovery` | 已验 PASS（3m15s，lib 13 warnings） |
| L-P0-04 | 契约清单一致性 | `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check` | FAIL（L-F2 cargo_lock_digest 过期，跨平台） |
| L-P0-05 | 仓库级静态检查 | `bun run check`（desktop-ui-boundary / process-runtime-boundary / uarc-boundary / agent-vocabulary / i18n / help --check 等） | 已验 PASS（typecheck+15 项检查全绿） |

注意：历史上远端提交曾造成 Linux 漏 `Path` 导入编译失败；P0 必须在 Linux 原生跑过，
不得复用 Windows 编译结果。桌面 renderer 相关改动后必须重跑 `check:desktop-ui-boundary`。

### L-P1 状态机与迁移定向测试（不调真实模型）

| ID | 内容 | 命令 | 状态 |
| --- | --- | --- | --- |
| L-P1-01 | Runtime lib 全组（segments/recovery/steering/checkpoint） | `cargo test -p nomifun-agent-runtime --lib` | 已验 PASS（336，1 ignored） |
| L-P1-02 | Session lib（native_execution_tests：租约竞争/旧执行者隔离/取消/零进度 preamble） | `cargo test -p nomifun-agent-session --lib` | 已验 PASS（95） |
| L-P1-03 | DB schema 契约与迁移 | `cargo test -p nomifun-db --test id_schema_contract`、`cargo test -p nomifun-db --lib migration` | 已验 PASS（6+1；migration 过滤器仅命中 `seed_rows_populated_after_migrations`） |
| L-P1-04 | Broker 与流式协议 | `cargo test -p nomifun-chat-model-broker --lib --test conformance` | 已验 PASS（lib 18 + conformance 43） |
| L-P1-05 | Engine core | `cargo test -p nomifun-engine-core --lib` | 已验 PASS（41，1 ignored 为原有 Bun 检查） |
| L-P1-06 | Process runtime 全组 | `cargo test -p nomi-process-runtime --tests` | 首败 FAIL（L-F3 计数漂移）→修复后 PASS（lib 176 + arch 16 + child_builder 10 + io 16 + parent_death 2 + process 13 + pty 10 + request 16 + session_registry 13 + supervisor 11 = 283；darwin_registered 0 为 Darwin-only `N/A`；process_soak 1 ignored 手动 soak） |
| L-P1-07 | 门禁单元测试 | `node --test scripts/validation/agent-reliability-report.test.mjs`、`node --test scripts/validation/probe-stepfun-tool-schema.test.mjs` | 已验 PASS（11+2） |
| L-P1-08 | 迁移覆盖 | 新库初始化、重复打开、schema manifest 一致（口径更新见 L-F5） | 已验 PASS（id_schema_contract 6/6 + session lib reopen/idempotent；旧 005-007/head=6 语义已退役） |

已有组件基础（此前在 Windows 宿主的 WSL2 内取得，仅作引用、不自动沿用为当前 HEAD 结果）：
`nomi-process-runtime` 曾 245/245；artifact 目录 fsync EBADF、删除事件去重时序、snapshot
字面路径均已修并有定向回归。源码如有后续变化须先核对是否影响上述链路再复用。

### L-P2 正式应用链路（脚本 provider，不用真实 key）

| ID | 内容 | 命令 | 状态 |
| --- | --- | --- | --- |
| L-P2-01 | 崩溃/重启自动恢复端到端 | `cargo test -p nomifun-app --test native_execution_recovery` | 已验 PASS（7/7，28s） |
| L-P2-02 | 历史展示 | `cargo test -p nomifun-app --lib history_display_tests` | 已验 PASS（5/5） |
| L-P2-03 | 路由缺口 | `cargo test -p nomifun-app --test nomi_core_route_gap` | 已验 PASS（41/41） |
| L-P2-04 | broker host | `cargo test -p nomifun-app --lib chat_broker_host::tests` | 已验 PASS（19/19） |
| L-P2-05 | execution API 核对 | `GET /api/agent-sessions/{id}/execution`：运行时保留 checkpoint、完成后不再保留、fence 正确递增、无自动 replay 授权 | 已验 PASS（断言由 L-P2-01 逐条覆盖：`checkpoint_retained` running/paused=true、completed/cancelled=false；`execution_generation`/`execution_fence` 单调递增；`automatic_replay_authorized`=false；精确重试 resume 返回 `duplicate=true` 不 replay） |

### L-P3 故障与边界矩阵（TEST-MATRIX P3 全表在 Linux 均需独立结果）

Store/runtime 用例已写的只关闭对应组件断言；应用组合、正式现场与 owner 端核对仍需 Linux
证据。下表按原 P3 行逐项列 Linux 状态：

| 场景 | Linux 要求 | 状态 |
| --- | --- | --- |
| 两个恢复者同时接管 | 一个 winner；另一个不能取消/写入/终结 winner | 组件已验（`native_recovery_race_has_exactly_one_winner`、`independent_disk_stores_share_one_recovery_winner_and_fence_old_writers`）；应用组合待跑 |
| 旧 producer 迟到 | admission/结果/checkpoint/终态均被 fence；旧取消不影响新任务 | 组件已验（takeover fences old writers；stale checkpoint digest resume 被拒） |
| 接管后加载失败 | 未使用 claim 可释放；已准入不谎称未使用 | 用例已写 |
| claim 后、首个 Runtime 事件前崩溃 | 重用原输入，零副作用恢复，不新增 Turn | 组件已验（`empty_recovery_accepts_only_a_root_preamble_without_model_or_tool_work`）+ 组合覆盖（L-P2-01 crash image 恢复不重放写） |
| TurnStarted/TurnInputScope 后首个 checkpoint 前崩溃 | 只允许该两事件，原子建立零进度 checkpoint | 组件已验（零进度 preamble 用例） |
| 同一 checkpoint 连续崩溃 | fence、模型号、压缩号不复用，旧文本不复活 | fence/CAS 单调组件已验（checkpoint CAS winner、generation 递增）；模型号/压缩号待跑 |
| 写入完成后模型流中断 | 已完成写入一次；未准入批次零次 | 应用组合已验（`startup_resumes_crash_image_without_repeating_the_completed_write`） |
| checkpoint 后出现工具准入/效果 | 不盲重放；核对不能完成则显式隔离并保留进度 | 应用组合已验（`startup_quarantines_a_pending_external_effect_without_replay`、`orphan_quarantine_marks_pending_effect_unknown`） |
| pending managed/external effect | 变为 unknown，不变可重试 failed；继续阻挡新效果 | 应用组合已验（`owner_reconciles_unknown_push_from_independent_remote_ref_before_resume` + orphan quarantine） |
| 用户取消与恢复并发 | 取消优先；不复活已取消 Turn | 应用组合已验（`cancelled_turn_is_never_selected_by_startup_recovery`、`cancel_racing_a_success_receipt_keeps_the_effect_and_cancels_the_turn`） |
| 纠正输入在模型等待时入队 | 保持接受顺序、完整文本、附件与技能选择 | 组件部分已验（session pause/fenced input 用例）；正式待跑 |
| 纠正已 applied 但 checkpoint 未提交 | 不丢输入，不用旧 checkpoint 越过它 | 待跑 |
| Snapshot/能力代次/build 改变 | 不自动兼容、不扩权限 | 待跑 |
| segment 边界上进程仍运行 | 不持久化活句柄为可恢复证明；停止时 SDK 仍清理 | 待跑 |
| Journal soft/hard/累计预算 | 窗口只在 checkpoint 确认后重置；总量不清零 | 组件已验（segments/cumulative limit 用例）；库级长链待跑 |
| Session payload 接近上限 | 提前停止并记录原因；不删历史腾空间 | 组件已验（compaction/byte-limit 用例）；产品链待跑 |
| 无进展/重复读/反复计划 | 不无限续期 | 组件已验（`unchanged_plan_success_is_not_counted_as_task_progress_forever`）；产品循环待跑 |
| 长 Turn 结束后再发新任务 | 历史可读；不被旧 4096 条/8 MiB 假上限卡住 | 压力测试待跑 |
| 恢复前 UI 有半条文本 | 只关闭消息段；新输出新 ID；不宣称旧任务完成 | 桌面待跑（依赖 GUI 探测） |
| native private terminal 缺失 | 只能从 canonical failed/cancelled 派生中断；不合成成功 | 待跑 |
| 本地跨平台一致性 | 文件与进程语义、数据库锁、快照和清理在 Linux 独立成立 | 组件已验（nomifun-file 480 + process-runtime 283 于 ext4/tmpfs 原生路径）；完整一致性矩阵待跑 |

### LNX Linux 平台专项（目录无 Linux 章节，按 WIN-001~018 / MAC-001~018 立等价项）

| ID | 场景与操作 | 验收标准 | 对应 | 状态 |
| --- | --- | --- | --- | --- |
| LNX-001 | 工作区跨文件系统：ext4 home 与另一挂载点（含 9p 对照） | cwd/path owner 使用真实挂载身份；跨挂载相对路径不误解析；9p 语义差异单独记录，不作 ext4 证据 | WIN-001 | 待验（挂载已核：`/`=ext4、`/mnt/c`=9p、`/tmp`=tmpfs；跨挂载 owner 身份专项未跑） |
| LNX-002 | 路径含空格、中文、emoji、NFC/NFD、非法 UTF-8 字节名 | 合法路径正确往返；不合法字节名不静默改写或错位创建 | WIN-002、MAC-003 | 组件已验（file_read_write/management、path_safety、invalid-UTF8 与隐藏文件用例）；NFC/NFD 与非法字节名专项待跑 |
| LNX-003 | 大小写敏感 ext4：`Foo`/`foo` 为不同文件 | owner/evidence 不合并为同一安全资源（与 WIN-005/MAC-001 断言相反，需独立正向验证） | WIN-005、MAC-001/002 | 待验（现仅有 case-insensitive 过滤器用例，无正向断言） |
| LNX-004 | symlink 指根内/根外、TOCTOU 变化 | 根内按契约操作，根外 fail closed；变化可检测 | WIN-007、MAC-004 | 组件已验（`rejects_symlink_escaping_sandbox`、`validate_path_resolves_symlink_within_sandbox`、`symlink_cwd_cannot_escape_its_capability_root`、recreated-artifact live reader）；TOCTOU 专项待跑 |
| LNX-005 | `/bin/sh -c`（dash）与 `/bin/bash -lc` quoting；直接 executable | shell 差异显式；直接 executable 不发生 expansion；literal argv 含 `$()`、反引号、`|`、`&`、`;`、引号不求值（W275 的 Linux 等价） | WIN-008/009、MAC-006 | 组件已验（request_contract 16 + child_process_builder 10：empty script 拒绝、program 保 os strings）；`$()`/反引号字面 argv 专项待跑 |
| LNX-006 | process group/session 所有权；leader 退出 descendant 存活 | group 清理完成前不发布成功；无 zombie；cancel/host death 清理整棵树 | WIN-011、MAC-007 | 组件已验（process_contract 13 + supervisor 11 + session_registry 13：leader 退出 descendant 存活不发布成功、sigint→sigterm 升级、cancel/abrupt host exit 收整组）；正式链待验 |
| LNX-007 | child `setsid` 逃离可观察 group | 标 authority lost/unknown；不等待伪 EOF 或谎称清理 | MAC-008 | 组件已验（`observable_setsid_escape_is_lost_instead_of_waiting_for_fake_pipe_eof`） |
| LNX-008 | parent death watchdog / subreaper（`linux_watchdog.rs`） | 应用强退后 child/grandchild 被精确回收；不误杀复用 PID | MAC-009、WIN-016 | 组件已验（parent_death 2 + linux_watchdog 全组 + `abrupt_harness_exit_kills_and_reaps_the_owned_process_group`）；正式链待验 |
| LNX-009 | PTY（/dev/ptmx）：初始尺寸、resize 后实际观察、快速退出、反复创建 | 不丢输出、不死锁；owner/cursor/cancel/reaped 核对（等价工作流 D 的 ConPTY 132×43 项） | WIN-010 | 组件已验（pty_contract 10：poll/write/resize/cancel、快速退出、UTF-8 分片、连续会话不丢输出；越界尺寸拒绝） |
| LNX-010 | UTF-8/LF、分块多字节、无效字节、stdout/stderr 混合流 | stream/lifetime encoding metadata 准确，原始 bytes 有界、不互相吞 | WIN-012 | 组件已验（io_contract 16：分块多字节、无效字节上报且 raw bytes 有界、lifetime encoding metadata 存活） |
| LNX-011 | mode/只读/不可执行/无权限目录；普通用户执行需 root 的操作 | 明确 permission denied；不触发 sudo/提权 | WIN-013、MAC-005 | 组件部分已验（`unix_literal_checkout_preserves_executable_and_symlink_modes`、`cwd_outside_capability_root_is_denied`、`guard_refuses_fs_root`）；permission-denied 专项待跑 |
| LNX-012 | 原子发布与清理：ext4 rename/no-replace、staging inode/字节核对、目录 fsync | 外来同名保留且 cleanup 记未确认；不吞 fsync 错误（WSL2 EBADF 已修，正式链复验） | FILE/ART 相关 | 组件已验（artifact_store 全组、`atomic_write_errors_preserve_uncertainty_and_cleanup_observations`、`unix_replacement_rejects_modified_staged_bytes_before_rename`）；正式链待验 |
| LNX-013 | watcher 删除事件时序（Modify(Metadata) → Remove(File)） | 归约按 event_type+path 去重，不同事实不互吞 | FILE-040 | 组件已验（file_watching 14 + watch_service 去重用例；本轮新修 L-F4 的 EINVAL 竞态）；正式链待验 |
| LNX-014 | 原生 opener（xdg-open 类）与相对缺失 app 名、TOCTOU 删除 | 成功/失败如实；不扩大权限 | COMP-006/012 | 待验（进度文档明确 Linux 未验） |
| LNX-015 | 退出链：正常 Quit、活动 Turn 中退出、清理失败 exit code、Runtime 未证 shutdown 的失败退出、kill 后 lease 到期恢复 | exit code 如实传递；活树清理有 owner 证据；不复活旧 Turn（W259/W264/W268/W274/W276 的 Linux 等价） | WIN-016、OBS 终态 | 待验；托盘 Quit 入口 `N/A`（WSLg 无 StatusNotifier 宿主，见 L-OUT-03） |
| LNX-016 | 桌面打包与 WebUI 一致：Linux desktop 构建/deb·rpm·AppImage 产物、updater 对齐、WebKitGTK sandbox；Tauri 与桌面 WebUI 同后端语义一致 | 产物身份可核对；authority/路径/终态不随 surface 改变（本分支有 `fix(linux)`/`fix(build)` 打包提交需回归） | WIN-018 | 构建已验：release 编译 21m58s + 三产物（AppImage/deb/rpm，SHA-256 见 `LNX-016-artifacts.sha256`）；脚本 exit 3 = release-lock 对脏工作树 fail-closed 拒绝 attestation（设计内，干净树重打即可）；产物身份一致性专项待跑 |
| LNX-017 | SSH/MCP/Browser 在 Linux 宿主 | transport/lifecycle 正常；远端 owner OS attestation 与负向正式 UI 如实 | §12/§13 | 待验（进度文档明确未验） |
| LNX-018 | 长会话累积：journal 3200 条/4 MiB、payload 16 MiB 联合预算在 ext4 库上 | 分段/安全点按预算切换；不伪称保存成功 | LONG 系列 | 组件部分已验（runtime segments/cumulative limit 用例）；库级联合预算压力待跑 |

### L-正式会话（真实 Tauri UI + StepFun，对应 A/B/C 与 REAL）

前提：GUI 探测通过；工作区在 ext4 下按 `…/nomifun-agent-reliability/<run_id>/<case_id>`
建独立子目录；用户任务从会话输入框以自然语言发送；选择加密 StepFun Coding Plan /
`step-3.7-flash`（按计划也可用 step-5-preview）；冻结 build/provider/Agent Revision/
Snapshot digest；`NOMIFUN_LIVE_*` 凭据只经既有安全 runner 一次性 stdin 注入。

| ID | 场景 | 覆盖问题簇 | 关键断言（Linux 命令映射后） | 状态 |
| --- | --- | --- | --- | --- |
| L-A | 观察、只读、小测试 | C01/C02/C03/C06/C08 | 首次合法形态；`LIST_TOP_LEVEL_INCLUDING_HIDDEN` 用 `/usr/bin/ls -a` 语义（dotfile 即隐藏项）；中文/空格路径读搜；零匹配如实说明；Git 只读 status/diff；按 AGENTS 跑指定小测试；预期非零如实披露不假报通过 | 待验（WSLg 显示可达，未建正式会话；无 StepFun 凭据注入） |
| L-B | 文件、进程、停止 | C04/C05/C06/C08 | 创建/精确修改/复制/移动/回读/hash/精确删除与 receipt 一致；交互 helper stdin→close→poll；长 helper 含 descendant，停止后无孤儿（process group 核对）、无新副作用；已完成效果不重放 | 待验（同上） |
| L-C | 连续、纠正、恢复 | C07 + 相关 C01～C06 | 多步连续不丢要求/证据、不串结果；追加纠正按接受顺序；一次实际 compaction；取消冷读不复活 cancelled；崩溃/退出后从 checkpoint 恢复不重放已完成副作用 | 待验（同上） |
| L-REAL | REAL-001～024 会话区真实 Case | 全簇 | UI/API/canonical/磁盘四方一致；GEN 与 COD 入口各有正式样本 | 待验（同上） |
| L-ADOPT | 新共享机制的 Linux 正式采用 | C06/C07/C08 | 有界历史正文投影与 closed-turn 引用投递（eb45556ce/fd3305422/3c41d3bf5 等）、typed 输出截断续写公开展示（b78ac77a3）、模型配置变更冻结/下一 Turn 刷新（W276 在 Windows 首败未定位，Linux 等价场景独立取证）、操作结果优先投影与来源去重 | 待验（同上） |

命令语料记录沿用 8.4.1A：每条记 `semantic_id, normalized_expectation, linux_invocation`；
Snapshot/tool description 的 host OS 必须与实际 owner 一致，否则记
`HOST_OS_COMMAND_MAPPING_ERROR`。

### L-P4 真实模型小样本（StepFun）

| ID | 内容 | 状态 |
| --- | --- | --- |
| L-P4-01 | `nomi_core_live_provider_smoke` opt-in/ignored 用例在 Linux 实际执行 | 待验（`--compile-only` 门禁已验 PASS：live_smoke_compile_status=pass；实际执行需凭据） |
| L-P4-02 | coding-smoke / 协作链 / long-coding 小样本：先确认错误分类、凭据隔离与实际成本 | 待验 |
| L-P4-03 | Step5（step-5-preview）接入及精确来源反馈的 Linux 真实采用（W273 等价） | 待验 |

### L-范围外（WSL2 不可验或本轮明确不完成）

以下条目**不在本次完成目标内**，原因分别标注；有真实 Linux 机器后按原 Case 定义补验，
不得以 WSL2 结果替代：

| ID | 场景 | 排除原因 |
| --- | --- | --- |
| L-OUT-01 | 系统 S3 sleep/wake + process group/PTY active 的 deadline/lease 归约（WIN-015/MAC-014 等价） | WSL2 无真实电源管理；需真机 |
| L-OUT-02 | OS reboot 前的 in-flight effect 恢复（WIN-016/MAC-018 等价） | 整机独占任务；WSL2 重启不等价系统重启 |
| L-OUT-03 | 托盘/AppIndicator Quit 入口（无托盘环境时） | 已探测：WSLg 会话 D-Bus 无 `StatusNotifierWatcher`/AppIndicator 宿主，`N/A` 成立（2026-10-06） |
| L-OUT-04 | Computer-use 真机像素/坐标/a11y/cancel/crash/soak | WSLg Wayland 只可作 `WSL2 lane`；真机 X11/Wayland 仍开放 |
| L-OUT-05 | Linux x86_64/arm64 真机 lane、真实发行版安装包安装/升级 | 当前仅 WSL2；打包构建可验，安装与真机运行不可验 |
| L-OUT-06 | N3/100 seed/LONG 全套/4h·8h soak/408 样本 99% 发布认证 | 按实施计划属独立发布认证，本轮不排 |
| L-OUT-07 | 五角色全分片 × 全业务 Action 矩阵、DOM/MGMT/APAL/AMUL/ACSR 完整队列 | 按计划移出活动队列；仅命令链涉及的 GEN/COD 样本在范围内 |

## 2. 记录纪律

- 条目按 L### 编号；每条记录基线 commit、构建/二进制身份、故障注入点、canonical event
  cursor、实际文件/进程结果；外部证据放仓库外 `…/linux/<case>/`，Git 只收进度摘要。
- 首败保留不覆写；修复只跑最小相关回归；正式 UI 验证每次对应具体新修复或明确覆盖缺口。
- Cargo 与正式 UI 各最多一个活动运行；每个 DB 只有一个写者；夹具失败不记产品结果。
- 任何检查失败记录原始任务、构建哈希、故障点、实际结果；不靠 sleep、放宽断言或删记录
  "修好"测试。未验、生成失败、缺证、已验证分别保留，不混记 PASS。
- 业务命令非零是合法结果；区分预检拒绝/未执行、预期非零、调度/执行/清理故障，不统称
  "操作异常"，不归因无证据的供应商/宿主责任。
