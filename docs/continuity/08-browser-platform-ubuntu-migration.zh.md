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

归档提交：见下方 `归档提交` 字段（提交后填写）。

<!-- ARCHIVE_COMMIT: fill after local commit -->
