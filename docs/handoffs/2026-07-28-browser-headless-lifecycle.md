# Browser Headless Lifecycle Handoff

更新时间：2026-07-28
状态：WIP 检查点，核心修复已实现，大部分测试已通过；真实 Chrome 最后两项验收和隔离 Web smoke 尚未收口。

## 1. Git 检查点

- 仓库：`nomifun/nomifun-tauri`
- 分支：`dev/browser-headless-lifecycle-ubuntu-20260728`
- 实现检查点：`6166c71ee1c09ffdd8836ee8edc2078abf0db794`
- 检查点提交说明：`wip(browser): checkpoint headless lifecycle hardening`
- 检查点基线：`46f5f96471ad9f271b0267012f5a54d8f580ce51`

在另一台电脑上继续：

```powershell
git fetch origin
git switch --track origin/dev/browser-headless-lifecycle-ubuntu-20260728
git status --short
```

如果本地已经存在同名分支：

```powershell
git fetch origin
git switch dev/browser-headless-lifecycle-ubuntu-20260728
git pull --ff-only
git status --short
```

## 2. 用户要求

1. 大多数普通 Browser Search、知识检索和天气查询默认使用静默无头浏览器。
2. 用户可以调整全局默认显示策略。
3. 明确需要人工操作时允许临时切换到前台，但不能永久覆盖用户的默认策略。
4. 浏览器管理页面必须展示真实运行状态，并能在前台、后台之间切换。
5. 管理页面必须能权威关闭浏览器，而不只是移除可见 Lane。
6. Turn、会话或应用生命周期结束后，必须回收 Lane、Target、Host、Chrome 进程树和临时 profile。
7. 未 dispatch、仍在排队或清理失败的 Browser 操作不得包装成成功。

天气查询只是复现用例，不应为天气业务添加特殊判断。策略应对普通 Browser Use 全局生效。

## 3. 原始故障证据

故障会话：

```text
019fa765-f033-7a62-921f-b02835b5d321
```

会话 JSON 取证确认：

- 模型调用了 4 次 `Browser`。
- `dispatched:true` 次数为 0。
- Lane 始终为 `queued`。
- `browser_epoch` 始终为 0。
- 没有创建任何 tab。
- admission 原因是 `system_memory_pressure`。
- 当时 `owner_active=0`、`global_active=0`，首个基本 Lane 仍未获准启动。
- `navigate` 约 8 ms 返回。
- `wait(3000)` 和 `wait(5000)` 均约 1 ms 返回，没有实际等待。
- 所有未执行结果却被包装为 `ok:true`、`is_error:false`。
- 模型随后降级到 shell，最终没有生成正常答复。

同时确认原设备的旧偏好中存在无版本：

```json
{
  "agent.browserUse.displayMode": "external"
}
```

后续日志还显示：

- 前台 Chrome 可以存在于零 Lane Host 中。
- 旧 inventory 不展示零 Lane Host。
- 旧 Close All 主要从 Lane 出发，无法完整覆盖 retained、retiring 或 detached cleanup。
- EndTurn cleanup 失败后可能由全局 sweep 产生 false success。

## 4. 已实现内容

### 4.1 默认无头与用户策略

- 全局默认显示模式改为 `headless`。
- Chromium 无头启动使用 `--headless=new`。
- 增加版本化显示策略。
- 无版本、旧版本、畸形策略 fail-closed 到 `headless`。
- 已经是当前版本的用户选择会被保留。
- `GET/PUT /api/browser/display-mode` 使用严格请求结构。
- live apply 失败时不会错误持久化新策略。
- foreground/background 的单次操作不会改写默认值。
- 设置页面和 Browser 管理页面都可调整默认策略。

### 4.2 Admission 与工具结果

- 系统未达到 critical floor 时，允许首个全局基本 Lane 启动。
- 内存压力仍会阻止额外并发扩张。
- critical pressure 继续 fail closed。
- queued page action 返回明确的 `browser_capacity_queued` 错误。
- queued `wait` 不再作为成功的 no-op 立即返回。
- 工具结果的外层错误状态与内部执行状态一致。

### 4.3 管理页面与 Host 可见性

- overview 展示实际 Host：
  - Host id
  - epoch
  - headful/headless
  - identity mode
  - Lane 数量
  - RSS
- 零 Lane Host 仍会出现在 inventory。
- foreground/background 通过替换 Host 实现。
- 替换时 epoch 增加，旧 handle 失效，Lane 重新绑定。
- 登录打开可以显式前台化，但不改写默认策略。

### 4.4 Close All 与 exact-owner cleanup

- Close All 会冻结新的 open。
- 会 drain：
  - live Lane
  - queued/starting Lane
  - pending Lane cleanup
  - active Host
  - retiring Host
  - orphaned/retained Host
- Close All 的完成条件是：
  - `remaining_lane_count == 0`
  - `remaining_cleanup_count == 0`
  - `remaining_managed_host_count == 0`
- Close All 可重复调用，完成后 Hub 可以重新打开 Browser。
- EndTurn 使用 exact owner lease revoke。
- pending Lane/Host cleanup 现在保留 owner attribution。
- owner cleanup 只驱动自己的 `(HostKey, epoch)` obligation。
- 不会误杀相同 HostKey 的 replacement epoch。
- 共享 Primary Host 中存在 sibling 时，不会因某个 owner 的结束关闭共享 Host。
- Host shutdown 失败会保留可重试 authority，不会被误报为成功。

### 4.5 CDP、进程树与 profile

- CDP worker 有界：单 worker、有限队列和总时间预算。
- unknown target creation fail closed。
- late popup 使用 tombstone/generation 防止复活旧 target。
- 最后一条 Lane 关闭时会执行 terminal Host cleanup。
- Drop 和显式 shutdown 共享 durable process cleanup authority。
- Windows 使用 Job Object 回收完整进程树。
- Windows ownership marker 使用 pinned handle 和 append-only lineage。
- Unix profile 操作绑定 directory fd。
- Linux 清理使用 `openat2`、`RESOLVE_NO_XDEV`、`NO_SYMLINKS` 和 fd-relative unlink。
- 临时 profile 只有在精确进程树终止后才删除。
- 稳定 profile 只清理当前 runtime artifacts，保留 Cookie、local storage 等持久状态。
- 启动恢复不再根据旧 marker 猜测并终止可能属于其他进程的浏览器。

## 5. 主要代码入口

- `crates/backend/nomifun-browser-platform/src/hub.rs`
- `crates/backend/nomifun-browser-platform/src/model.rs`
- `crates/backend/nomifun-browser-platform/src/resource.rs`
- `crates/backend/nomifun-app/src/browser_lane_provider.rs`
- `crates/backend/nomifun-app/src/router/browser_management.rs`
- `crates/backend/nomifun-app/src/router/browser_login.rs`
- `crates/backend/nomifun-app/src/services.rs`
- `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs`
- `crates/agent/nomi-browser/src/tool.rs`
- `crates/agent/nomi-browser-engine/src/backend/cdp.rs`
- `crates/agent/nomi-browser-engine/src/launch.rs`
- `crates/agent/nomi-browser-engine/src/profile.rs`
- `ui/src/common/browser/browserDisplayModeController.ts`
- `ui/src/renderer/pages/browser/BrowserDisplayModeControl.tsx`
- `ui/src/renderer/pages/browser/index.tsx`

## 6. 已完成验证

下列结果来自实现检查点形成过程。继续修改相关文件后必须重新运行受影响测试。

- `nomifun-app` 显式 Browser 测试：126/126
- `nomifun-ai-agent` Browser 测试：29/29
- Browser Lane Provider：16/16
- Browser Platform 原完整套件：150/150
- 新增的两个 exact-owner 测试单独通过
- Browser Engine：579 passed、0 failed、8 ignored
- `nomi-browser`：209 passed、0 failed、6 ignored
- UI 相关 7 个测试文件：84/84
- UI build：通过
- Gateway `browser-use` feature check：通过
- Windows profile tests：66/66
- Windows launch tests：26/26
- WSL profile tests：44/44
- WSL launch tests：22/22
- `cargo fmt --all -- --check`：通过
- `git diff --check`：通过
- `.github/workflows` 中 `.yml/.yaml`：0

真实 Chrome 已通过：

- `single_tab_headless_has_exactly_one_page_target`
- `standalone_backend_drop_cleans_stable_profile_and_allows_relaunch`
- `managed_host_shutdown_clears_only_exact_runtime_profile_artifacts`

## 7. 当前已知未完成项

### 7.1 真实 Chrome Drop 测试的 marker 契约

当前失败测试：

```text
managed_host_drop_reaps_full_tree_and_ephemeral_profile
```

位置：

```text
crates/agent/nomi-browser-engine/tests/integration_managed_host.rs
```

当前测试把：

```rust
ownership_marker_browser_pid(&profile)
```

与：

```rust
host.process_id()
```

直接比较。

但是 helper 读取的是：

```text
.nomifun-browser-owner.json
```

新的 append-only ownership 设计中，这个文件是 provisional predecessor，其
`browser.pid` 等于 owner app PID。真正 committed browser identity 位于：

```text
.nomifun-browser-owner.committed.json
```

因此该断言目前比较了两个不同语义的 PID。

不要只删除断言。正确收口方式：

1. 让集成测试读取并验证 committed ownership record。
2. 确认 committed browser PID 与 Managed Host root PID 的契约。
3. 在 Drop 前采样实际 Chrome 进程树。
4. 确认 Windows Job 包含并回收实际 browser root 和 descendants。
5. Drop 后确认：
   - 所有采样 PID 消失
   - DevTools endpoint 关闭
   - provisional/committed marker 消失
   - ephemeral profile 消失
   - 没有属于隔离 profile 的 Chrome 进程残留

真实 Chrome：

```text
C:\Program Files\Google\Chrome\Application\chrome.exe
```

建议命令：

```powershell
$env:NOMIFUN_CHROME_BINARY = 'C:\Program Files\Google\Chrome\Application\chrome.exe'

cargo test -p nomi-browser-engine `
  --test integration_managed_host `
  managed_host_drop_reaps_full_tree_and_ephemeral_profile `
  -- --ignored --exact --nocapture

cargo test -p nomi-browser-engine `
  --test integration_managed_host `
  create_engine_drop_reaps_hidden_host_runtime_and_allows_stable_relaunch `
  -- --ignored --exact --nocapture
```

### 7.2 Browser Platform 完整回归

在新增两个 exact-owner 测试之后，尚未重新运行完整 Platform suite。

```powershell
cargo test -p nomifun-browser-platform
```

预期测试总数约为 152，但应以当前源码实际发现数量为准。

### 7.3 隔离 Web 端到端 smoke

尚未运行最终 `nomifun-web` 真实 smoke。

构建：

```powershell
cargo build --locked -p nomifun-web --features 'nomifun-app/browser-use'
```

必须使用全新的临时目录，例如：

```text
%TEMP%\nomifun-browser-smoke-<uuid>\data
```

启动 `target\debug\nomifun-web.exe` 时使用：

```text
--host 127.0.0.1
--port 0
--data-dir <temp>\data
--api-only
--insecure-no-auth
```

子进程环境：

```text
NOMIFUN_WORK_DIR=<temp>\data
NOMIFUN_CHROME_BINARY=C:\Program Files\Google\Chrome\Application\chrome.exe
```

必须验证：

1. fresh install 的 `GET /api/browser/display-mode` 返回 `headless`。
2. 普通 Browser Agent 首次启动包含 `--headless=new`。
3. `/api/browser/login/open` 可以临时 foreground。
4. background/foreground 后 epoch 增加，旧 browser PID 消失。
5. `PUT /api/browser/display-mode` 可持久化 headless/external。
6. `POST /api/browser/close-all` 的三个 remaining 字段全部为 0。
7. overview 中 Lane、pending cleanup、Host 全为 0。
8. 临时 data-dir 所属 Chrome 进程和 ownership artifacts 全部消失。
9. 同一临时 data-dir 重启后默认策略仍为 headless。
10. close-all 后可以重新打开并再次关闭。

注意：

- `/api/browser/login/open` 会主动 foreground，不能用它证明“普通 Agent 默认无头”。
- `nomifun-web` 当前没有完整 graceful signal shutdown；smoke 中必须先调用
  `/api/browser/close-all` 并验证完全 drain，再停止精确的隔离 Web PID。
- 不要运行 `bun run dev` 或 `bun run dev:web`。
- 不要使用或修改真实 `Nomi-dev` data-dir。
- 不要终止 Edge 或任何不属于隔离 data-dir 的浏览器进程。

## 8. 仓库硬性规则

GitHub Actions 在此仓库绝对禁止：

- 不得在 `.github/workflows/` 创建、恢复、重命名、提交任何 `.yml/.yaml`。
- 不得通过 GitHub API、CLI 或设置启用 Actions。
- 每次提交前都要确认 workflow YAML 数量为 0。

检查命令：

```powershell
$workflows = @(
  Get-ChildItem -LiteralPath '.github\workflows' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Extension -in @('.yml', '.yaml') }
)
"WORKFLOW_YAML_COUNT=$($workflows.Count)"
```

## 9. 新 Coding Agent 启动 Prompt

将以下内容作为新任务的首条消息：

```text
继续处理 NomiFun Browser Use 默认无头与完整生命周期回收。

请先完整阅读：
docs/handoffs/2026-07-28-browser-headless-lifecycle.md

仓库：
nomifun/nomifun-tauri

远程分支：
origin/dev/browser-headless-lifecycle-ubuntu-20260728

核心用户要求：
- 普通知识检索和 Browser Search 全局默认静默无头。
- 用户可以调整默认显示策略。
- 登录等人工交互允许临时前台，但不能覆盖默认策略。
- 管理页必须能前台/后台切换和权威关闭。
- Turn/会话结束后 Lane、Target、Host、Chrome 进程树和临时 profile 必须完全回收。
- 未 dispatch、排队和 cleanup 失败不得包装成成功。

不要从头重写。检查点已实现大部分功能并通过大部分测试。

优先任务：
1. 修复真实 Chrome 测试对 provisional/committed ownership record 的错误读取。
2. 验证 Windows Job 确实回收 committed browser PID 和全部 descendants。
3. 重跑两个真实 Chrome Drop/relaunch ignored tests。
4. 重跑 cargo test -p nomifun-browser-platform。
5. 构建带 nomifun-app/browser-use feature 的 nomifun-web。
6. 使用全新 TEMP data-dir 完成 headless、foreground/background、close-all、
   restart persistence 和零残留真实 Chrome smoke。
7. 最后运行 cargo fmt、git diff --check，并确认 .github/workflows YAML 数量为 0。

禁止：
- 不要运行 bun run dev 或 bun run dev:web。
- 不要启动、修改或清理真实 Nomi-dev。
- 不要杀 Edge 或不属于隔离 profile 的浏览器。
- 不要创建 GitHub Actions workflow。
- 不要仅删除失败断言；必须保留进程树和 profile 清理证明。

保持已验证事实与尚未验证项分开。没有完成真实 Chrome 与隔离 Web smoke 前，
不要声称问题已经完全交付。
```
