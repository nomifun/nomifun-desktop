# Browser Platform Ubuntu 迁移归档（2026-07-28）

## 1. 归档目的与当前基线

这是一份**迁移快照**，用于把当前 Windows 工作树交给 Ubuntu 环境继续收尾。
它不是新的需求规格，也不替代 `07-browser-platform-handoff.zh.md`；当两者冲突时，
本文件的“现行产品决策”和仓库 `AGENTS.md` 优先。

- 归档分支：`dev/browser-headless-lifecycle-ubuntu-20260728`
- 归档前基线：本地 `main`，`8252b4fc`
- 归档时远程基线：`origin/main`，`377595d1`
- 本次操作：把当前工作树的全部开发修改集中提交到上述本地开发分支
- 远程操作：**没有 push，也不要在迁移前 push**
- Worktree 目标：只保留一个工作树；不要再从旧的 `refactor` 或历史
  `integrate/*` worktree 继续开发

## 2. 现行产品决策（必须保持）

### 2.1 默认静默，而不是按场景猜测

“天气查询”只是例子，不是特殊例外。大多数简单的 Browser Search、知识检索、
公开页面读取都必须走同一个全局默认策略：

- 受管 Browser Use 默认启动 Chromium `--headless=new`；
- 不创建操作系统窗口、不弹窗、不抢焦点；
- 不使用用户个人 Chrome/Edge profile；
- Primary 仍使用应用管理的稳定 profile，但其进程默认无头；
- Anonymous、Authenticated Replica、Isolated Host 也默认无头；
- 用户可在 Settings 调整应用级的默认可见策略：新安装默认“后台静默
  （headless）”，用户可以显式改成“默认前台可见（external/headful）”；该选择
  属于可信用户偏好，**不能被普通 Agent action 或模型参数覆盖**。即使用户选择
  默认前台，也不得恢复嵌入式 Viewer 或用户接管能力。

推荐迁移契约：

- `agent.browserUse.displayMode=headless`：默认无头；
- `agent.browserUse.displayMode=external`：用户明确选择默认前台可见；
- 历史 `embedded`：迁移为 `headless`，因为嵌入式 Viewer 已退出产品；
- 历史 `silent=true`：迁移为 `headless`；
- 历史 `silent=false`：迁移为 `external`；
- 两者都不存在：使用并持久化 `headless`；
- 不再写入 `agent.browserUse.silent`。

### 2.2 前台打开是管理操作，不是 Agent 能力

`/browser` 页面只是状态和生命周期管理页，不提供嵌入式页面、截图轮询、
Viewer WebSocket、接管输入或交还控制。

只有经过应用认证的用户管理操作，才允许对 running Primary Lane 调用：

```text
POST /api/browser/lanes/{lane_id}/foreground
```

该操作必须安全关闭旧 headless Host，再用同一应用 profile 创建 headful Host。
进程替换会递增 browser epoch，旧 target/frame/ref 必须失效并要求 fresh observe。
它不能成为模型可调用的普通 Browser action，也不能绕过 approval、egress 或
其他安全策略。

### 2.3 生命周期必须立即收敛

- Agent turn 正常结束、取消或异常结束：关闭该 turn 的 Lane；
- runtime、attempt、conversation、remote capability 或 lease 结束：关闭其归属
  Lane；
- 关闭最后一个 Lane 后，若没有待完成的 target/start cleanup，立即关闭最后一个
  Host/Chromium；
- 60 秒 warm timer 和周期 sweep 只能作为被动兜底，不能成为显式关闭的前置条件；
- 应用退出必须等待显式 Host shutdown 完成，不能以强杀进程冒充验收通过。

## 3. 本次归档中已经存在的实现

当前提交包含以下方向的开发代码（后续不要重复重写）：

1. `BrowserSessionHub` 作为主进程唯一浏览器所有权；Host/Lane、身份域、资源
   调度、owner lease、inventory 和清理路径已集中到 Hub。
2. Engine 增加 `BrowserHostLaunchMode`，普通启动使用 `--headless=new`，并记录
   实际 headful/headless 状态。
3. Native Agent 通过 Hub 签发的 `BrowserLaneBinding` 接入；turn 完成/取消路径
   已开始执行 Lane cleanup，runtime kill/drop 使用 owner lease 回收。
4. 管理页移除嵌入式预览和用户接管入口，显示 Lane/Host 状态、队列、容量、身份，
   只对 running Primary 提供“前台打开”，并提供 Lane/会话/全部浏览器关闭。
5. 旧 `agent.browserUse.silent` 仅作迁移读取，不再写回；display mode 统一收敛
   到管理层的 `external` 语义（routine Agent execution 仍然 headless）。
6. 文档、i18n、Browser 管理页测试和 Engine/Agent 定向测试已同步过一轮。

注意：归档代码中的设置层目前把所有旧 display mode 统一迁移为单一
`external` 展示标签，而后端 `primary_host_is_headful()` 又始终返回 `false`。因此
“默认真正无头”已实现，但“用户可以调整全局默认可见策略”**尚未实现**，见
P0-G；不要把当前单选 UI 当作最终产品契约。

主要改动区域：

- `crates/backend/nomifun-browser-platform/src/hub.rs`
- `crates/agent/nomi-browser-engine/src/{launch.rs,host.rs,backend/cdp.rs,lib.rs}`
- `crates/agent/nomi-browser/src/platform_adapter.rs`
- `crates/backend/nomifun-ai-agent/src/{factory/browser_lane.rs,manager/nomi/agent.rs}`
- `crates/backend/nomifun-app/src/services.rs`
- `ui/src/renderer/pages/browser/` 与 Browser 设置/i18n

## 4. 迁移时必须优先修复的阻塞项

### P0-A：Host finalization single-flight 尚未接线

`hub.rs` 已有 `HostFinalizationFlight` 和 `host_finalizations` 字段脚手架，但当前
还没有由显式 close、target cleanup callback、sweep 统一调用。后台 finalization
和显式 close 可能重复 shutdown 同一个 Host，导致一次 shutdown failure 被后台路径
消费、显式调用却错误返回成功。需要：

- 实现 `finalize_host_once(key)`；
- 同一 Host 的所有调用者 join 同一个 flight；
- 失败结果保留 retirement authority；
- 后续 sweep 才开启明确的新 retry；
- 关闭流程必须能看到真实的第一次失败。

### P0-B：Lane detach 与 pending start 的竞态

`detach_lane_for_close()` 当前在移除 Lane 后才登记 `pending_host_retirements`，
存在 `finalize_empty_host()` 观察到空 Host 并提前 shutdown 的 TOCTOU。应拆为
持有 `open_gate` 的 locked 版本，在同一临界区完成 Lane detach、pending start 登记
和 Host retirement authority 发布。`abandon_unclaimed_lane_start()` 已经围绕
`open_gate` 工作，必须与新版本统一锁语义。

### P0-C：锁序和等待边界

统一所有 retirement 路径的锁序，建议固定为：

```text
open_gate -> retiring_host_keys -> host_slots -> retiring_host_slots
```

不得在持有 `retiring_host_slots` 时再获取 `retiring_host_keys`。等待 pending
start 必须有有限 timeout；超时保留 Hub-owned pending authority，交给 sweep 重试，
不能删除记录或永久等待。

### P0-D：turn cleanup 不能被其他清理阶段短路

审查重点是 `TurnTerminationGuard::Drop`、`kill_and_wait()` 和
`finish_nomi_teardown()`：MCP/process cleanup 失败时仍必须执行 Browser Lane cleanup；
terminal event 不得早于 cleanup 证明。还要处理 `kill()` 先 revoke owner lease、
随后 turn `close_all()` 因 lease 已撤销而失败的竞态，确保 Hub-owned cleanup 仍可完成。

### P0-E：普通 Agent 工具的外部浏览器旁路

BrowserTool 主路径已接入 Hub，但 `nomi-computer` 的 `launch`、ACP Computer MCP、
shell/open-external 或用户自定义 MCP 仍可能直接调用 OS opener。需要先界定边界，
然后对 Agent surface 的 `http/https` 外部打开 fail-closed 或引导使用 Hub Browser；
不能影响用户从 Browser 管理页发起的可信“前台打开”。

### P0-F：测试基线需要按新契约重写，而不是回退产品决策

归档前最近运行：

```text
cargo test -p nomifun-browser-platform --lib
126 passed / 7 failed（133 total）
```

失败项及原因线索：

- `crawl_host_waits_for_the_sixty_second_warm_timer`
- `failed_warm_shutdown_keeps_the_host_slot_for_the_next_sweep`

  旧测试把显式 close 后的 Crawl Host 保留 60 秒，当与新契约“最后 Lane 立即关闭
  Host”冲突，应改写为显式关闭立即回收；warm timer 只测试被动 sweep。

- `cancelling_empty_host_sweep_before_queue_handoff_does_not_strand_retiring_key`
- `cancelling_empty_host_sweep_after_queue_handoff_retains_cleanup_authority`

  与 pending retirement 的原子 handoff 和统一锁序相关。

- `failed_shutdown_retains_host_authority_for_explicit_retry`

  主要由重复 finalization/错误消费造成，先修 P0-A 再调整测试。

- `foreground_lane_for_user_uses_trusted_seam_and_publishes_activity`
- `foreground_lane_for_user_requires_running_lane_and_propagates_safe_driver_error`

  FakeHost 默认 `is_headful() == false`，测试现在意外走 headless→headful 替换；
  FakeHost 应记录 `HostLaunchRequest.headful` 并实现准确的 `is_headful()`，再分别
  覆盖 headful seam 和 headless transition。

归档前 `hub.rs` 中临时的 `#[cfg(test)] eprintln!` 已删除。当前代码仍可能有编译
warning（未接线的 finalization 脚手架），不要把 warning 当作完成证明。

### P0-G：补齐用户可调整的全局默认可见策略

当前归档实现将 `BrowserStartupPreferences.display_mode` 和前端
`BROWSER_DISPLAY_MODES` 收敛为 `external`，同时始终以 headless 启动 Primary。
Ubuntu 续作需要按 2.1 的迁移契约重新提供两个可信用户选项：

- 后台静默（默认，`headless`）；
- 默认前台可见（用户显式选择，`external`）。

后端必须在 Host launch policy 中执行该偏好，而不是只改变文案；普通 Agent/模型
不能通过 lane name、tool JSON 或请求参数覆盖它。无论选择哪一项，用户仍可在
管理页手动“前台打开”当前 running Primary。不得重新加入 `embedded`、截图预览、
Viewer token、接管或输入桥接。应补新安装、旧 `displayMode`、旧 `silent`、损坏值
以及“不再写 silent”的前后端迁移测试。

## 5. Ubuntu 继续开发启动步骤

```bash
cd /path/to/nomifun-tauri
git switch dev/browser-headless-lifecycle-ubuntu-20260728
git status --short --branch
git fetch origin main
git rev-list --left-right --count origin/main...HEAD
git diff --check
```

先阅读：

1. `AGENTS.md`
2. 本文件
3. `docs/continuity/07-browser-platform-handoff.zh.md`
4. `docs/architecture/browser-platform.zh.md`
5. `docs/guides/computer-browser-use.zh.md`

然后按顺序处理 P0-A 至 P0-F。不要 `reset --hard`、`git clean -fdx` 或覆盖已有
未提交工作；不要创建额外 worktree；不要 push 任何远程分支。

## 6. 可直接复制的继续工作 Prompt

```text
你正在 Ubuntu 上继续 NomiFun Browser Platform 的收尾工作。先阅读仓库 AGENTS.md、
docs/continuity/08-browser-platform-ubuntu-migration.zh.md、07-browser-platform-
handoff.zh.md、architecture/browser-platform.zh.md 和 guides/computer-browser-use.zh.md。

当前开发分支是 dev/browser-headless-lifecycle-ubuntu-20260728；它包含 Windows
工作树的完整归档，不要 reset、clean、回退或重写已有提交，不要创建新的 worktree，
不要 push。先输出 git status、HEAD、origin/main...HEAD 和 .github/workflows 下的
文件检查。

产品决策不可改变：普通和大多数简单的 Browser Search/知识检索在新安装上全局默认
使用真正的 Chromium --headless=new，不弹窗、不抢焦点；Primary 也默认无头。
Settings 必须允许可信用户把应用级默认策略改成 external/headful，但普通 Agent/模型
不能覆盖该偏好。无头模式下，认证用户仍可从 /browser 管理页对 running Primary
明确调用 POST /api/browser/lanes/{id}/foreground，关闭旧 headless Host 并用同一
应用 profile 创建 headful Host。没有嵌入式预览、JPEG 轮询、Viewer WebSocket、
用户接管或页面输入能力。关闭最后 Lane/turn/runtime 后必须立即关闭最后 Host，
warm timer/sweep 只能兜底。

先修 P0：
1. 在 crates/backend/nomifun-browser-platform/src/hub.rs 接通 per-Host
   HostFinalizationFlight，消除后台和显式 close 重复 shutdown，并保留第一次失败；
2. 在 open_gate 下原子完成 detach Lane 与 pending start retirement 登记；统一锁序
   open_gate -> retiring_host_keys -> host_slots -> retiring_host_slots；为 pending
   start 等待加 bounded timeout，超时保留权威记录；
3. 修复 TurnTerminationGuard/kill_and_wait，使 MCP/process 失败也不会跳过 Browser
   Lane cleanup，并处理 revoke 与 close_all 竞态；
4. 审查并封堵 nomi-computer、ACP Computer MCP、shell/open-external 的 Agent 外部
   http/https opener 旁路，但保留管理页可信 foreground；
5. 按新契约改写 Hub 测试：显式最后 Lane close 立即 Host shutdown，warm timer 仅测
   被动回收；FakeHost 要记录并反映 request.headful；补跨 Lane/turn/runtime cleanup
   和 Ubuntu bundled/configured Chromium 进程残留测试。
6. 修复设置契约：displayMode 只提供 headless（新安装默认）与 external（用户显式
   默认前台）两项；按迁移表保留用户明确选择，不再写 silent；后端 launch policy
   必须实际执行该偏好，Agent tool JSON 无权覆盖。

每个改动后运行定向测试，最后运行 cargo fmt --all、cargo check/test、bun typecheck
和 git diff --check。禁止 GitHub Actions；.github/workflows 下不得有 yml/yaml。完成
前不要宣称生产可上线，也不要 push。
```

## 7. 归档完成判定

迁移前至少确认：

- 当前唯一工作树已切到归档开发分支；
- 所有开发文件已在本地提交，`git status` 干净；
- 提交 hash 已记录在本文件末尾；
- 没有 push；
- `.github/workflows` 下没有 `.yml`/`.yaml`；
- 已明确记录 7 个当前失败测试和 P0 阻塞项。

归档代码提交：`00823ec3fb470ee4d0dda98176679a0786cfc565`

契约补充提交：本文所在的后续文档提交（可通过 `git log -2` 确认）。

## 8. Ubuntu 续作进展（2026-07-28）

本节记录在 Ubuntu 上对 P0-A 至 P0-G 的实施结果。环境：Linux 7.0.0-28-generic，
rustup 安装的 rustc 1.97.1（发行版 rustc 1.93.1 无法编译锁定的 sysinfo 0.39.6），
bun 1.3.14。

### 已完成

- **P0-A**：`hub.rs` 实现 `finalize_host_once(key)` per-Host single-flight。
  显式 close、`run_pending_lane_cleanup` 的完成回调和 `close_matching` 全部 join
  同一 `HostFinalizationFlight`；attempt 本身由 Hub-owned spawned task 执行，调用
  方取消/超时不会中断进行中的 shutdown。失败结果保留在 flight map 中（sticky），
  后续显式调用者 join 同一个第一次失败；只有 `retry_retiring_host_slots`（sweep/
  shutdown 路径）清除 settled flight 并开启明确的新 retry。调用方等待有
  `HOST_FINALIZATION_WAITER_TIMEOUT`（7s）上界。
- **P0-B**：`detach_lane_for_close` 拆为公开入口（获取 `open_gate`）+
  `detach_lane_for_close_locked`（临界区内完成 Lane 移除、pending start 登记与
  调度器释放）。`abandon_unclaimed_lane_start` 已持有 `open_gate`，改调 locked
  版本，消除与 `finalize_empty_host` 空 Host 判定的 TOCTOU。
- **P0-C**：锁序 `open_gate -> retiring_host_keys -> host_slots ->
  retiring_host_slots` 在 `BrowserSessionHubInner` 字段声明处作为契约注释固化；
  审计确认 `finalize_empty_host`、`sweep_empty_hosts`、`forget_retired_host_slot`
  均已按此顺序。`wait_for_pending_host_starts` 增加
  `PENDING_LANE_START_WAIT_TIMEOUT`（6s）有界等待；超时保留 Hub-owned
  `pending_host_retirements` 记录并交给 sweep 的
  `finalize_hosts_ready_after_cleanup`（已加入 `sweep()` 开头）恢复。
- **P0-D**：`manager/nomi/agent.rs` + `factory/browser_lane.rs`：
  1. `TurnTerminationGuard::Drop` 的 teardown task 重写为无条件尝试全部三个
     fence（MCP 不再嵌套于 `if let Some(supervisor)`、process quiesce、Browser
     `close_turn_lanes`），terminal 仅在聚合证明 exact 时发布（保留非终态
     quarantine 语义）；
  2. `fence_cancelled_processes` 用 `NomiTeardownFailures` 聚合 MCP 与 process
     两阶段，不再短路；
  3. kill 竞态：`close_turn_lanes` 感知 `revocation_requested` 位与
     `OwnerLeaseExpired` 错误码，将"lease 已被 Hub-owned revocation flight 撤销"
     视为 satisfied-by-revocation（Hub 的 `close_owner_lease` 按 lease id 关闭
     Lane，不依赖 lease 记录存在）；
  4. idle-kill 路径 `schedule_nomi_cancelled_terminal_after_process_fence` 接收
     browser binding，terminal 发布前 `revoke_and_wait` 等待有界 Browser cleanup
     证明；无 supervisor 时也执行 MCP shutdown。
  新增测试：`turn_boundary_close_treats_revoked_owner_lease_as_satisfied`、
  `termination_guard_drop_emits_finish_after_revoked_lease_browser_cleanup`、
  `idle_kill_terminal_waits_for_browser_cleanup_proof`、
  `idle_kill_withholds_terminal_when_browser_cleanup_fails`。
  另修复基线上已失败的 `termination_guard_emits_finish_on_armed_drop`
  （terminal 由 spawned task 异步发布，测试原先同步 `try_recv`）。
- **P0-E**：Agent 外部 http/https opener 全链路 fail-closed，错误文本统一引导
  使用受管 Browser（`browser navigate`）：
  1. `nomi-computer/src/launch.rs` 新增 `validate_agent_web_target`（含
     `microsoft-edge:https://…` 等 wrapper 协议），一处封堵 in-process
     ComputerTool、`nomifun-computer` MCP 与 Gateway `nomi_computer_launch`；
  2. `nomifun-shell` `ShellService::launch` 同规则封堵（唯一生产调用方是
     `nomifun-open` MCP）；`open_external` 不变，继续服务 UI 可信链接；
  3. Gateway `nomi_shell_open_external` 收窄为仅 `mailto:`；
  4. `nomi-tools/windows_shell.rs` 的脚本校验扩展为跨平台：OS opener/浏览器
     二进制（`xdg-open`/`open`/`gio`/浏览器名等）+ http(s) URL 参数组合被拒，
     本地文件打开与 `curl`/`wget` 等非 opener egress 不受影响；
  5. `acp_assembler.rs` 提示语与各工具 schema/描述改写为"URL 走受管 Browser"。
  边界说明：`/browser` 管理页 foreground、login-open 与 UI 链接
  （`POST /api/shell/open-external`）不经过以上任何守卫，保持可信路径不变。
- **P0-F**：`hub.rs` 7 个失败测试按新契约重写，全套 135 通过：
  - `explicit_last_lane_close_shuts_down_crawl_host_immediately`（原 warm-timer
    等待测试）：显式关闭立即回收，warm sweep 不得二次 shutdown；
  - `warm_timer_sweep_reclaims_stranded_empty_crawl_host_as_backstop`（新增）：
    用 `strand_lane_record` 构造无 close 路径的 stranded Host，验证被动兜底；
  - `failed_explicit_close_shutdown_keeps_host_authority_for_the_next_sweep`
    （原 failed_warm_shutdown）：失败在 close 调用方可见，权威保留给 sweep 重试；
  - 两个 sweep 取消测试改用 stranded Host 构造，"before handoff"改为持有
    `retiring_host_keys` 写锁（锁序中 handoff 的第一把锁）；
  - `failed_shutdown_retains_host_authority_for_explicit_retry`：首次 shutdown
    返回第一次真实失败、同一调用内 retry 收敛（host_shutdowns==2）、重复
    shutdown 幂等；
  - foreground 两测试拆分为 headful seam（`HubConfig.headful=true`，原地
    `bring_to_front`，发布活动）与新增 headless transition 测试（第二次 launch
    请求 `headful==true`、旧 Host 显式停止、epoch 递增、`BrowserRestarted`
    fresh-observe 语义）；FakeHost 记录 `HostLaunchRequest.headful` 并实现准确
    `is_headful()`。
  产品代码同步修复：foreground 的 headless 分支现在与 headful 分支一致地在
  成功后发布 `last_active_at_ms` 活动戳（带 closing 复验）。
- **P0-G**：headless/external 用户设置与迁移契约（前后端）：
  - 后端 `services.rs`：`BrowserStartupPreferences::default().display_mode =
    "headless"`；`resolve_browser_display_mode` 实现第 2.1 节迁移表（headless/
    external 保留不重写；embedded/无效→headless 持久化；silent=true→headless、
    silent=false→external、缺失→headless 均持久化；读失败不持久化）；
    `primary_host_is_headful` 返回 `display_mode == "external"`，经
    `HubConfig.headful` 由 Host launch policy 执行（仅 Primary 生效，非 Primary
    Host 恒 headless；`open_lane`/`LaneLaunchRequest` 无可见性参数，Agent JSON
    无路径可达）。对应 6 个迁移测试重写/新增；
  - 前端 `browserSettings.ts`：`BROWSER_DISPLAY_MODES = ['headless','external']`，
    `migrateBrowserDisplayMode` 与后端映射一致；设置页
    `BrowserUseSettingsContent.tsx` 提供两选项 RadioGroup（保留挂载时迁移与
    "永不写 silent"约束）；`configKeys.ts`、i18n（en/zh）与 source-scan 测试
    同步更新；
  - 生效语义：与其它启动偏好一致，更改在应用重启后生效（设置描述已注明）。
    Hub 每次 Host 启动都从 `HubConfig`（RwLock）读取 headful，如后续需要热更新
    可加 `set_resource_policy` 同款 setter，本次未引入未接线的 API。

### 测试与门禁证据（本机，2026-07-28）

- `cargo test -p nomifun-browser-platform --lib`：135 passed / 0 failed
  （基线 126/7）；
- `cargo test -p nomifun-ai-agent --features browser-use --lib`：全部通过；
- `cargo test -p nomifun-app --features browser-use --lib`：全部通过；
- `cargo test -p nomi-computer --lib`：77 passed；
- `cargo test -p nomifun-shell --lib`：77 passed；
- `cargo test -p nomifun-gateway --lib`：127 passed；
- `cargo test -p nomi-tools --lib -- windows_shell`：通过。注意：本机存在一组
  **HEAD 基线即失败**的环境性测试（`git stash` 复验，与本次改动无关）：
  `nomi-tools` 22 个（bash/pty/exec_command/write_stdin，沙箱 PTY 限制）、
  `nomifun-ai-agent` 1 个（openclaw construction；openclaw/herman 已计划移除，
  不再投入）、`nomifun-app` 1 个（server lock authority）、
  `nomifun-conversation` 1 个、`nomifun-terminal` 2 个。工作树失败集合与
  HEAD 基线完全一致，本次改动未引入新失败；
- `bun test`（browserSettings + BrowserUseSettingsContent）：25 passed；
- `bun run typecheck`：通过；
- `cargo fmt --all -- --check`：通过；`git diff --check`：干净；
- `.github/workflows/` 仅含 README.md，无任何 `.yml`/`.yaml`。

### 对抗性审查轮（2026-07-28，多 Agent diff review）

对本次 diff 做了三维度（hub 并发 / turn teardown / settings+opener）审查加逐项
反驳验证，13 项原始发现中 6 项确认并已全部修复：

1. **foreground×close 竞态产生零 Lane headful 僵尸 Host**（hub.rs restart
   flight 在 slot 已被显式关闭移除后仍无条件插入新 slot 并启动 headful
   Chromium）：`restart_host_once_with_visibility` 增加 live-lane 前置检查
   （无存活 Lane 直接返回 recovery 错误），并在 rebind 后调用
   `finalize_empty_host`（open_gate 下复查）立即回收窗口期内被清空的 key；
2. **pending-start 有界等待超时导致显式关闭静默假成功**：
   `wait_for_pending_host_starts` 改为返回 `Result`，超时向 close 调用方返回
   `cleanup_pending` 错误（`pending_lane_start_wait_timeout_error`），权威记录
   仍保留给 sweep；
3. **settled-Ok finalization flight 在"结果已发布、尚未从 map 移除"窗口被后续
   close 复用**，跳过真正的最后一 Lane 回收：`finalize_host_once` 不再复用
   settled-Ok flight（仅 join 进行中与 sticky 失败），后续调用开启新 attempt；
4. **`close_turn_lanes` 的 revocation_requested 位可掩盖已失败的 revocation
   flight**：收窄为仅对 `OwnerLeaseExpired` 错误码映射 satisfied-by-revocation；
5. **web 守卫 `http://` 子串可被 scheme-only 形式绕过**（`https:example.com`
   会被浏览器规范化为真实导航）：三处守卫（nomi-computer、nomifun-shell、
   nomi-tools shell 校验）全部改为 `http:`/`https:` scheme 前缀匹配；
6. **`app` 参数绕过**（`{target:"example.com", app:"msedge"}`）：nomi-computer
   与 ShellService 的 launch 对 `app` 增加浏览器/opener 黑名单
   （basename 匹配，含 `.exe` 剥离），拒绝并引导受管 Browser。

其余 7 项经验证反驳（含 idle-kill 死锁、guard-drop 回归、fast-path 语义收窄
等，均为不可达或与既有语义一致）。修复后全部受影响套件复跑通过。

### 遗留事项

- 07 文档的 P0 门禁（workspace 全量 `cargo test`、完整 UI 矩阵、最新 main
  集成、真实 Chromium/发布 smoke）仍未在本机执行完毕；本节不构成发布验收。
- Agent shell 的 URL-opener 校验为已知 opener/浏览器清单 + http(s) 参数组合，
  属纵深防御；任意二进制间接打开 URL 无法在词法层穷尽，Exec 审批与 egress
  策略仍是主约束。
- displayMode 热更新（无需重启）未实现，见 P0-G 记录。
- 未 push；本机 rustup 工具链变更不影响仓库内容。
