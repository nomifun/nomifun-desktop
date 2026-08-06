# SSH 远程会话 Phase 2 — 实施计划(连接池 / 实时状态 / 会话身份)

- 日期:2026-08-05
- 分支:`feature/ssh-remote-session-phase2`,基线 `2b728bfc`(Phase 1 已合并入本地 `main`)
- 上游文档:
  - 设计:`docs/superpowers/specs/2026-08-04-ssh-remote-sessions-design.md`(§10 UI、F1/F2、§11 错误表、§15 分期)
  - 待做清单:`docs/superpowers/todo/2026-08-05-ssh-remote-sessions-todo.md`(T1/T2/T3)
  - Phase 1 计划:`docs/superpowers/plans/2026-08-04-plan-a-ssh-remote-sessions.md`(格式基准;Task D3/D4/G4 是本文 Task C/D/E 的前身)
- 本文覆盖:TODO 的 **T3(连接池)+ T2(`ssh.status` 实时事件)+ T1(header 状态药丸)**,外加一个 Phase 1 遗留的真实缺口(Task A)。
- 不覆盖(留 Phase 3 / 后续):T4 远程输出面板、T5 `~/.ssh/config` 导入、T6 证书/agent 认证、T7 MFA、T8 ProxyJump。

---

## 0. 开工前的事实核对结论(全部对真实代码核实,与上游文档有冲突处以本节为准)

| 编号 | 结论 | 证据 |
|---|---|---|
| V1 | **`SshConnectionHandle` 的 `shell`/`fs`/`_conn` 是模块私有**,`pool.rs` 作为兄弟模块无法访问 → 必须加 `pub(crate)` 访问器 | `nomifun-ssh/src/sink.rs:23-30` |
| V2 | **当前存在两份彼此不可见的 `SshHostService` + `SshConnectionProvider`**:一份给 agent factory(`services.rs:2957-2968`,`service` 是局部变量、构造完即丢),一份给路由(`router/state.rs:788-805`)。任何加在其中一侧的池对另一侧完全不可见 | `services.rs:2957`、`state.rs:788`、`state.rs:562` |
| V3 | **`SshBackendProvider::connect` 只有 3 个 `&str` 参数、返回 `Arc<dyn SshBackend>`**;全仓库仅两个调用点 | trait `nomi-agent/src/ssh_backend.rs:63-73`;调用 `nomifun-ai-agent/src/factory/nomi.rs:748`、`nomifun-ssh/src/routes.rs:133` |
| V4 | **`NomiHostWiring` 是 `pub(crate)` + 手写 `Default`**(非 derive);`ssh_backend` 在 `agent.rs:661-667` 被 **move** 进 bootstrap,manager 不留句柄 → **今天 teardown 根本没有 SSH 可拆**;`NomiTeardownResults` 只有 3+1 个字段,无 SSH 字段 | `agent.rs:331-347`、`:661-667`、`:1571-1577`、`finish_nomi_teardown` 在 `:1637` 按值接收 |
| V5 | **`desktop.rs::perform_shutdown(&self)` 一律走 `self.<field>`**,此处没有 `AppServices` 句柄 → 池必须作为 `DesktopServer` 的新字段在构造时(`:735-738` 附近)克隆进去;`close_database_after_cleanup` 在 `:971`,且 `errors` 非空时会跳过关库 | `desktop.rs:934`、`:946`、`:971`、`:1407-1415` |
| V6 | **`private_event_boundary.rs` 是 substring 扫描**:每个登记源必须含 `UserEventSink`,且**连注释里都不得出现** `EventBroadcaster` / `WebSocketManager` | `nomifun-realtime/tests/private_event_boundary.rs:6-21`、`:23-36` |
| V7 | **`shell.rs` 把「通道已死」当成「命令超时」**:`collect_until_sentinel` 的 `Ok(None) \| Err(_) => return None`(`:260`),上层一律返回 `Ok(ShellOutcome{exit_code:124, timed_out:true})`,死链只能靠 `cwd == ""` 猜。`SshError` 无 `Disconnected` 变体(仅 5 个),且 `From<russh::Error>` 把一切塌缩成 `Protocol` | `shell.rs:159-201`、`:260`;`connection.rs:44-61` |
| V8 | russh **0.62.5**(`Cargo.toml:162` 精确 pin)。`Handle::is_closed()` 存在但**是同步的、只反映本地 mpsc sender 是否关闭**(session task 结束才为真,不是网络探针);`Handle::disconnect(reason, description, language_tag)` **要三个参数**;`send_ping()` 是等 pong 的往返版(`send_keepalive(want_reply)` 是单向版);`ChannelMsg::ExitSignal{signal_name, core_dumped, error_message, lang_tag}` 存在但标注 `(server only)`,且 `shell.rs:242-258` 目前把除 `Data`/`ExtendedData` 外的一切 `continue` 掉 | `russh-0.62.5/src/client/mod.rs:289,878,925,933`;`src/channels/mod.rs:99-104` |
| V9 | **`ssh_hosts.status` 永远绿**:`mark_connected`(`service.rs:176-186`)是 `update_status` 的唯一调用者,自身唯一调用点是 `sink.rs:239`;仓库里没有任何地方写 `disconnected`/`error` | `service.rs:176`、`sink.rs:239`、`nomifun-db/src/repository/sqlite_ssh_host.rs:189` |
| V10 | `ui-api-contract-version.txt` 现值 **5**;断言点:`webui_dist.rs:11,115,342`、`apps/build-support/ui_build_manifest.rs:43,85`、`ui/vite.config.ts:12`、`scripts/ui-build-manifest.ts:21`。**改它必须跟一次 `bun run build:ui`**,否则 boot 校验失败 | 同左 |
| V11 | **侧栏缺口是真的**(Task A):`useConversationListSync.ts:172-185` 把 `isOrdinaryWorkConversation` 过滤后的数组直接赋给 store,**原始 `items` 被丢弃**,快照类型(`:127-131`)没有未过滤字段,也没有任何 selector 能拿到被过滤掉的 SSH 会话;仓库里不存在 SSH 分组组件 | `useConversationListSync.ts:127-131,167-185`;`conversationListFilter.ts:26` |
| V12 | **~~workspace rail 是死按钮~~ — 此前的判断错误,已撤回。** `extra.workspace: ''` 是后端**自动分配**的约定信号:`service.rs:4249-4257` 判空 → `:4265-4275` 分配 uuidv7 token → `:4565` `create_dir_all` → `:4592` 把真实路径写回 `extra.workspace` 并落库。SSH 会话因此**已经**拿到 spec F2 要求的 `{work_dir}/conversations/{uuid}` 本地 scratch 目录,`is_temporary_workspace` 由 `convert.rs:58-64` 每次读时派生为 `true`,`ChatSlider.tsx:21` 的空值早退**不会**触发。故 Phase 1 在这一点上是对的 | `service.rs:4249-4275,4565,4592`;`convert.rs:58-64` |
| V13 | 承 V12:spec F1(`workspaceLocalTabs`)剩下的只是**观感诚实**问题(远程会话里 Files/Changes 展示的是一个空的本地 scratch 目录),不是坏功能。代价却不低:`extraTabs` 有 **body 链 5 跳 + chrome 链 2 跳**,且要同时改 4 处消费点(`WorkspaceToolRail.tsx:106-120` 隐藏两项、`WorkspaceRailBody.tsx:277-283` 兜底改为首个 extra tab、`WorkspaceRailBody.tsx:85-91` **与** `hooks/useWorkspacePanelTabs.ts:20-27` 两份独立的 localStorage 初始 tab、`ChatLayout/index.tsx:136-141` 标题)。→ **本轮不做**,记回 TODO 与 T4 合并处理(T4 本身要用 `extraTabs`,届时一并) | 同左 |

---

## Global Constraints

沿用 Phase 1 计划的全部规矩,外加本轮新增的硬约束:

1. **循环依赖**:`nomifun-ai-agent` 永不依赖 `nomifun-ssh`。跨界只走 `nomi-agent/src/ssh_backend.rs` 的 seam,经 `nomifun-ai-agent/src/lib.rs` 再导出;`nomifun-app` 是唯一同时依赖两者的 crate。
2. **池不归 agent runtime**:`AgentKillReason::ConfigurationChanged` 会在换模型时销毁 runtime。池是进程级后端服务,runtime 只持 lease。
3. **idle 状态不走 `AgentStreamEvent`**(严格 per-turn),只走 user-scoped `/ws` 的 `UserEventSink`,发射器逐字仿 `nomifun-terminal/src/events.rs`。
4. **绝不伪造 `reaped`**:`channel 关闭 && (exit-status || exit-signal)` = Reaped;其余一律 `Lost`,且 `Lost` 是真实 teardown 失败。
5. **edition 2024**:裸 trait object 是硬错误,一律写 `Arc<dyn Trait>`。
6. **`private_event_boundary.rs` 是 substring 扫描**(见 V6):新 `events.rs` 里连注释都不能出现 `EventBroadcaster` / `WebSocketManager`。
7. **署名**:每个 commit 作者与提交者均为 `RiKa0-0 <2206491416@qq.com>`,conventional commits,无 AI trailer,不 `--no-verify`,`.github/workflows/` 下不新增任何文件。
8. **测试**:`cargo test -p <crate> -- --test-threads=2`;全量 `cargo nextest run --build-jobs 8 --test-threads 8`;**不在 /tmp 构建**;集成测试起独立高位端口 sshd + 独立密钥(`crates/shared/nomi-ssh/tests/support/sshd.rs`),**绝不碰真实 `~/.ssh`**(known_hosts 一律用 tempdir);无 sshd 时诚实 SKIP,不假通过。
9. **每个 commit 边界都必须编译通过且测试通过**。凡是拓宽 trait 签名的步骤,必须在**同一个 commit 内**改完所有调用点(V3 的两个点)。

---

## 设计决策(评审胜出方案 + 已修正的致命缺陷)

**核心不变式:每条链路(link)的 `tokio::sync::watch<SshLinkState>` 是唯一真相源。** 重连阶梯、`ssh.status` 事件、REST 快照、header 药丸、teardown 判词,全部读同一个值;`PoolInner::publish` 是唯一写入者,并在同一次调用里把新值投影成 `SshStatusEvent` 交给发射器 —— 于是「socket 状态」与「UI 看到的状态」在结构上无法漂移。

- **归属**:`SshConnectionPool` 进程级单例,在 `services.rs` 于 `event_bus` 之后、`build_agent_factory` **之前**构造一次,`AppServices.ssh_pool` 持有(内部是 `Arc<PoolInner>` newtype,`clone()` 是句柄)。交给恰好三个消费者:`AgentFactoryDeps.ssh_provider`、`build_ssh_host_state`、`conversation_service.with_delete_hook`。这同时消灭 V2 的双实例。
- **键**:`SshLinkKey { conversation_id, ssh_host_id }` —— host 进键,避免会话改绑主机后复用旧 socket。
- **teardown 取证不是补丁而是状态**:`SshLinkState::Closed { teardown: SshTeardown }`,而 `SshTeardown::Reaped` 只能由看到「channel close + exit-status/exit-signal」的 `ShellCloseProof` 构造。
- **前置债**:V7 —— 死通道被报成超时,所以「探活」和「取证」在修掉 `shell.rs` 之前根本无法实现。Task C 的前两步就是还这笔债。

### 评审发现的致命缺陷 → 本计划如何避免

| 缺陷(评审指出) | 本计划的处置 |
|---|---|
| pool.rs 访问 `handle.shell` 编译不过(字段私有,V1) | Task C Step 6 显式加 `pub(crate) fn shell()/fs()` 访问器,并在同 commit 内使用 |
| `AppServices.ssh_pool` 类型前后不一致(`Arc<Self>` vs 裸类型) | 统一为:`SshConnectionPool` 本身是 `#[derive(Clone)] pub struct SshConnectionPool(Arc<PoolInner>)`,`new()` 返回 `Self`(**不是** `Arc<Self>`);字段声明为 `pub ssh_pool: nomifun_ssh::SshConnectionPool` |
| `pool.host_service()` 未定义就被调用 | Task C Step 7 在池上显式声明 `pub fn host_service(&self) -> SshHostService` |
| `desktop.rs` 里 `services.ssh_pool.shutdown_all()` 无 `services` 绑定(V5) | 改为给 `DesktopServer` 加字段 `ssh_pool`,构造时克隆,`perform_shutdown` 里走 `self.ssh_pool` |
| `NomiTeardownResults` 用裸 trait object(edition 2024 硬错误) | 字段写全 `Option<Arc<dyn crate::SshSessionLease>>` |
| 重复声明 `SshError` 时丢掉 `#[error]` 属性 | 只**追加** `Disconnected` 一个变体并带 `#[error(...)]`,不重写枚举 |
| 把 `SshLeaseRelease::Retained` 记成成功而 `Lost` 也当成功 | `describe_ssh_release`:`Retained`/`Reaped` → `Ok(())`;`Lost` → `Err(AppError)`,进 teardown 聚合失败 |
| 拓宽 `SshBackendProvider::connect` 却不改 `routes.rs:133`(V3) | Task C Step 8 在同一 commit 内改 trait + 两个调用点 |
| `is_retryable` 测试引用后面步骤才存在的类型 | 分类器与 `SshDialError` 同步落在 Step 6,状态机 Step 3 只测纯函数(退避/上限/取证) |
| 池 events.rs 加入 boundary 测试却含禁词(V6) | Task D Step 2 先写 events.rs 并**自查禁词**,再登记进 `private_event_boundary.rs` |
| `/api/ssh-hosts/status` 与 `{ssh_host_id}` 捕获冲突 | 用复数 `/api/ssh-hosts/statuses`,并加「GET 真实 host id 仍可解析」的回归测试 |

### 采纳的跨方案 graft

| graft(来自落选方案) | 落点 |
|---|---|
| 删掉 `SshConnectionProvider`,让池**就是** `SshBackendProvider`(少一个类型,且强制解决 V2) | Task C Step 8 + Step 9 |
| `Arc::ptr_eq` 断言路由与 agent factory 观察到同一个池 | Task C Step 9 的 `ssh_pool_is_shared_between_routes_and_the_agent_factory` |
| **per-host** 拨号闸门 + 终局错误 poison + 共享冷却(N 个会话打同一台重启中的主机只产生一次拨号) | Task C Step 7(`dial_gate: DashMap<SshHostId, Arc<Mutex<HostGate>>>`) |
| 机器可读的 `reason` 枚举上线,UI 不靠字符串匹配挑颜色 | `SshLinkPhase` + `detail`,颜色只由 phase 决定(`SSH_STATUS_COLOR`) |
| 主机 DELETE / 改认证材料时 `close_for_host` + poison,防止 supervisor 往已删主机重拨 | Task C Step 10 |
| `SshHostService::mark_unreachable`(修 V9) | Task C Step 10 |
| `reaped = (closed \|\| eof) && (exit_status.is_some() \|\| exit_signal.is_some())`,把 `ExitSignal` 一并捕获 | Task C Step 2 |
| `RemoteShell::close` 对 channel 锁加 `tokio::time::timeout`,拿不到锁就诚实报 `errors` 且 `reaped=false` | Task C Step 2 |
| `SentinelEnd { Found, TimedOut, Closed }`,`Closed` 映射到新的 `SshError::Disconnected` | Task C Step 1 |
| 拒绝的 graft:用 `std::sync::Mutex<SshStatusEvent>` 取代 `watch`。**理由**:`tokio::sync::watch` 在本仓库有大量先例(scheduler/idle_scanner/plugin 等),且 watch 的「订阅 + 变更通知」正是 supervisor 与快照都要的语义,Mutex 版要自己再造通知 | — |

---

## 阶段与依赖顺序

```
Task A(侧栏 SSH 分组,纯 UI)        ← 与 C/D/E 完全独立,可先做/并行
Task C(T3 连接池,含 nomi-ssh 前置债)
   └─ Task D(T2 ssh.status 事件 + REST 快照)
         └─ Task E(T1 header 药丸 + 前端 hook + i18n)
```

- Task A 不碰任何 Rust,不与 C/D/E 冲突。
- Task C 内部严格顺序(1→11);Task D 依赖 C 的 `SshLinkState`/`publish`;Task E 依赖 D 的 wire 契约。
- `ui-api-contract-version.txt` 5→6 与 `bun run build:ui` 放在 Task D 的路由步骤(那是新增 HTTP 路由的那一步)。

---

### Task A: 侧栏 SSH 会话分组(补 Phase 1 缺口 V11)

**为什么**:SSH 会话被 `isOrdinaryWorkConversation` 排除出普通列表(`conversationListFilter.ts:26`),而 spec §10 要求的「顶层独立分组」从未实现 → 建完会话切走就再也找不回来,唯一入口是主机簿里再点「新建会话」(每点一次新建一个)。

**Files:**
- Modify: `ui/src/renderer/pages/conversation/SessionList/hooks/useConversationListSync.ts`(快照加 `sshConversations`,同一遍扫描填充,零额外请求)
- Create: `ui/src/renderer/pages/conversation/SessionList/SshSessionGroup.tsx`
- Create: `ui/src/renderer/pages/conversation/SessionList/SshSessionGroup.structure.test.ts`
- Modify: `ui/src/renderer/pages/conversation/SessionList/hooks/useWorkpathUiState.ts`(新增 `SSH_GROUP_STORAGE_KEY` + `sshGroupExpanded` + `toggleSshGroup`,仿 companion 的 6 处)
- Modify: `ui/src/renderer/pages/conversation/SessionList/index.tsx`(collapsed 与 expanded 两个挂载点,仿 `:636-656` 与 `:667-672`)
- Modify: `ui/src/renderer/services/i18n/locales/{en-US,zh-CN}/ssh.json`(分组标题/空态/tooltip)

**Interfaces:**
- Consumes:`useConversationListSync()` 的新 `sshConversations: TChatConversation[]`;`useSWR('ssh-hosts.list', () => ipcBridge.ssh.list.invoke())` 拿主机名(与 `SshHostManagement.tsx:263` 同 key,SWR 去重);`ConversationRow`(`types.ts:22-49` 的 `ConversationRowProps`,无 workpath 依赖)
- Produces:`SshSessionGroup`,props 与 `CompanionSessionGroup`(`:27-38`)同形:`{ activeConversationId, collapsed?, onSessionClick?, expanded?, onToggleExpanded?, renderRow }`。空列表时 `return null`(仿 `:131`)。

- [ ] **Step 1: 写失败测试** — `SshSessionGroup.structure.test.ts`,`bun:test` + `readFileSync`(仿 `CompanionSessionGroup.structure.test.ts` 的 idiom,不 render):断言源码含 `useConversationListSync`、按 `ssh_host_id` 二级聚合的 `Map`、`ipcBridge.ssh.list`、裸 `import { Server } from '@icon-park/react';`、`t('ssh.group.` 前缀、`return null` 空态;负向断言不含 `getUserConversations`(不得二次拉全量)。
- [ ] **Step 2:** Run `bun test --cwd ui src/renderer/pages/conversation/SessionList/SshSessionGroup.structure.test.ts` — Expected: FAIL(文件不存在)。
- [ ] **Step 3:** 实现 `useConversationListSync` 的 `sshConversations`(在 `:172` 同一遍过滤里分流,数组身份稳定:ids+modified_at 未变则复用旧数组,避免 `useSyncExternalStore` 抖动)+ `useWorkpathUiState` 的 ssh 折叠态 + `SshSessionGroup.tsx` + 两个挂载点 + i18n 键。
- [ ] **Step 4:** Run 同上 — Expected: PASS。Run `bun run check` — Expected: PASS(含 `check:i18n`、`check:icons`)。
- [ ] **Step 5: Commit** `feat(ssh): sidebar group for host-bound sessions`

---

### Task C: `SshConnectionPool` — 状态机 / 退避重连 / 取证关闭(TODO T3)

#### Step 组 1:`nomi-ssh` 前置债(V7)

**Files:** Modify `crates/shared/nomi-ssh/src/shell.rs`、`crates/shared/nomi-ssh/src/connection.rs`;Modify `crates/shared/nomi-ssh/tests/shell.rs`

**Interfaces:**
- Produces:`enum SentinelEnd { Found { exit_code: i32, cwd: String }, TimedOut, Closed }`(私有);`SshError::Disconnected(String)`(**追加**变体,带 `#[error("ssh link disconnected: {0}")]`);`pub struct ShellCloseProof { pub eof_sent: bool, pub channel_closed: bool, pub exit_status: Option<u32>, pub exit_signal: Option<String>, pub errors: Vec<String> }` + `pub fn is_reaped(&self) -> bool`;`RemoteShell::close(&self, budget: Duration) -> ShellCloseProof`;`SshConnection::is_closed(&self) -> bool`(委托 `Handle::is_closed()`,V8:同步、只反映本地 session task 是否结束)与 `SshConnection::disconnect(&self) -> Result<(), SshError>`(**三参数**调用 `Handle::disconnect`)

- [ ] **Step 1: 写失败测试** — `crates/shared/nomi-ssh/tests/shell.rs` 追加(真 sshd,无 sshd 则 `let-else` 诚实 SKIP):`run_reports_disconnect_when_the_shell_exits` —— 在持久 shell 里跑 `exit`,断言下一次 `run` 返回 `Err(SshError::Disconnected(_))`,而**不是**今天的 `Ok(ShellOutcome{exit_code:124, cwd:""})`。
- [ ] **Step 2:** Run `cargo test -p nomi-ssh --test shell -- --test-threads=2` — Expected: FAIL。
- [ ] **Step 3:** 实现:`collect_until_sentinel` 返回 `SentinelEnd`(把 `:260` 的 `Ok(None) | Err(_) => return None` 拆成 `Ok(None) => Closed` 与 `Err(e) => Closed`),`Closed` 一路上抛为 `SshError::Disconnected`;`TimedOut` 保持既有 124/Ctrl-C/信号阶梯语义不变。
- [ ] **Step 4:** Run 同上 — Expected: PASS。另跑全 crate `cargo test -p nomi-ssh -- --test-threads=2` 确认 sentinel/超时既有测试未回归。
- [ ] **Step 5: Commit** `fix(ssh): report a dead shell channel as a disconnect instead of a timeout`
- [ ] **Step 6: 写失败测试** — `close_proves_the_shell_was_reaped`(断言 `proof.channel_closed && proof.exit_status.is_some() && proof.is_reaped()`)与 `close_on_a_dead_channel_is_not_reaped`(先杀 shell,断言 `!is_reaped()` 且 `errors` 非空)。
- [ ] **Step 7:** 实现 `ShellCloseProof` + `RemoteShell::close(budget)`:`eof()` → 在 budget 内 drain `wait()` 收集 `ChannelMsg::{ExitStatus, ExitSignal, Eof, Close}` → `close()`;对 channel mutex 用 `tokio::time::timeout`,超时则 `errors.push("shell busy; close proof unavailable")` 且不置 `channel_closed`。同步补 `SshConnection::{is_closed, disconnect}`。
- [ ] **Step 8:** Run `cargo test -p nomi-ssh -- --test-threads=2` — Expected: PASS。
- [ ] **Step 9: Commit** `feat(ssh): close the remote shell with exit evidence`

#### Step 组 2:状态机(纯函数,无 I/O)

**Files:** Create `crates/backend/nomifun-ssh/src/state.rs`;Modify `crates/backend/nomifun-ssh/src/lib.rs`

**Interfaces:**
- Produces:常量 `SSH_RECONNECT_INITIAL_BACKOFF_MS=1_000`、`SSH_RECONNECT_MAX_BACKOFF_MS=60_000`、`SSH_RECONNECT_MAX_ATTEMPTS=10`、`SSH_LIVENESS_POLL_INTERVAL=15s`、`SSH_CLOSE_BUDGET=5s`;`pub fn reconnect_delay(attempt: u32) -> Duration`(倍增封顶);`#[serde(rename_all="camelCase")] pub enum SshLinkPhase { Idle, Connecting, Connected, Degraded, Reconnecting, Dropped, Closed }`;`pub enum SshLinkState { Idle, Connecting{attempt}, Connected{fingerprint}, Degraded{detail}, Reconnecting{attempt, next_retry_in_ms}, Dropped{detail, retryable}, Closed{teardown: SshTeardown} }`;`pub enum SshTeardown { Reaped{detail}, Lost{detail}, AlreadyDown{detail} }`,其中 `SshTeardown::from_proof(&ShellCloseProof) -> Self` 是 `Reaped` 的**唯一**构造路径。

- [ ] **Step 10: 写失败测试** — `state.rs` 内 `#[cfg(test)] mod tests`:`reconnect_delay_doubles_and_caps_at_60s`(attempt 1..=12)、`max_attempts_and_backoff_constants_are_pinned`、`teardown_is_reaped_only_with_exit_evidence`(`channel_closed=true` 但无 exit 证据 → `Lost`)、`phase_of_every_state_is_total`(七个变体全覆盖)。
- [ ] **Step 11:** Run `cargo test -p nomifun-ssh state:: -- --test-threads=2` — FAIL → 实现 → PASS。
- [ ] **Step 12: Commit** `feat(ssh): link state machine with pinned reconnect ladder`

#### Step 组 3:wire 投影 + 发射器 → 见 Task D(两者同源,故 `SshStatusEvent` 落在 Task D Step 1)

#### Step 组 4:池本体

**Files:** Create `crates/backend/nomifun-ssh/src/pool.rs`;Modify `crates/backend/nomifun-ssh/src/sink.rs`、`src/service.rs`、`src/lib.rs`;Create `crates/backend/nomifun-ssh/tests/pool_lifecycle.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone)] pub struct SshConnectionPool(Arc<PoolInner>)`,`pub fn new(service: SshHostService, known_hosts: PathBuf, events: SshEventEmitter) -> Self`
  - `pub struct SshLinkKey { pub conversation_id: String, pub ssh_host_id: SshHostId }`(`Eq + Hash`)
  - `pub async fn acquire(&self, user_id, conversation_id, ssh_host_id, remote_cwd) -> Result<SshSessionBinding, SshDialError>`
  - `pub fn host_service(&self) -> SshHostService`;`pub fn subscribe(&self, key: &SshLinkKey) -> Option<watch::Receiver<SshLinkState>>`;`pub fn snapshot(&self, user_id: &str) -> Vec<(SshLinkKey, SshLinkState)>`
  - `pub async fn close_link(&self, key) -> SshTeardown`;`pub async fn close_conversation(&self, conversation_id) -> Vec<SshTeardown>`;`pub async fn close_for_host(&self, ssh_host_id)`;`pub async fn shutdown_all(&self) -> SshShutdownReport`;`pub async fn probe(&self, user_id, ssh_host_id) -> SshProbeOutcome`
  - `sink.rs`:`pub enum SshDialError { Credential, Auth, HostKey, Unreachable, Protocol, ShuttingDown }`(thiserror);`SshConnectionHandle::{pub(crate) fn shell(), pub(crate) fn fs(), pub fn is_transport_closed()}`;`connect` 返回 `Result<Self, SshDialError>`
  - `service.rs`:`pub async fn mark_unreachable(&self, user_id, id, detail) -> Result<(), SshServiceError>`(修 V9)
- 不变式:`PoolInner::publish(&self, link, next_state)` 是 `watch` 的**唯一**写入者,且在同一次调用里投影 + 发射(Task D 接线后)。

- [ ] **Step 13: 写失败测试** — `tests/pool_lifecycle.rs`(真 sshd + tempdir known_hosts,诚实 SKIP):`acquire_twice_returns_the_same_link`(指针相等 + `active_link_count()==1`)、`connect_publishes_connecting_then_connected`(drain watch)、`dial_failure_publishes_dropped_with_retryable_flag`。另 `tests/dial_errors.rs`(无需 sshd):`unknown_auth_type_is_a_non_retryable_dial_error`、`missing_credential_maps_to_credential_error`。
- [ ] **Step 14:** Run `cargo test -p nomifun-ssh -- --test-threads=2` — FAIL。
- [ ] **Step 15:** 实现 `SshDialError` + 访问器 + `connect` 签名变更(**同 commit** 修 `sink.rs` 内部与 `examples/demo_ssh.rs` 的 `.map_err(|e| e.to_string())`)+ 池的 happy path/复用/per-host 拨号闸门(`DashMap<SshHostId, Arc<Mutex<HostGate>>>`,含冷却与终局错误 poison)。
- [ ] **Step 16:** Run 同上 — PASS。
- [ ] **Step 17: Commit** `feat(ssh): pooled per-conversation links with a per-host dial gate`
- [ ] **Step 18: 写失败测试(掉线 → 退避 → 恢复)** — `a_dead_transport_publishes_dropped_then_reconnecting_then_connected`(杀掉再在同端口重启夹具 sshd,断言 watch 序列;这正是「idle 时也要推状态」的场景)、`a_changed_host_key_publishes_dropped_and_never_retries`、`reconnect_replays_the_last_proven_cwd`。
- [ ] **Step 19:** 实现 supervisor 任务:`SSH_LIVENESS_POLL_INTERVAL` 轮询 `is_transport_closed()`(V8:廉价、不与长命令抢 channel 锁)+ `reconnect_delay` 阶梯 + `last_cwd` 重放 + `Degraded`(shell 超时且 cwd 空 → 同一 transport 上 `reopen_shell`,不重拨)。
- [ ] **Step 20:** Run 同上 — PASS。
- [ ] **Step 21: Commit** `feat(ssh): supervised reconnect ladder with cwd replay`
- [ ] **Step 22: 写失败测试(取证关闭)** — `close_link_publishes_closed_with_a_reaped_teardown`、`close_link_on_an_already_dropped_link_reports_already_down_not_reaped`、`close_conversation_closes_every_host_link`、`shutdown_all_refuses_new_acquires_and_counts_reaped`、`on_conversation_deleted_closes_the_link`(经 `dyn OnConversationDelete`)、`mark_unreachable_walks_back_the_host_status`。
- [ ] **Step 23:** 实现 `close_*`/`shutdown_all`/`probe` + `impl OnConversationDelete for SshConnectionPool` + `mark_unreachable` + `close_for_host`。
- [ ] **Step 24:** Run 同上 — PASS。
- [ ] **Step 25: Commit** `feat(ssh): close pooled links with forensics and cascade on delete`

#### Step 组 5:seam 拓宽 + 接线(编译面最大的一步,必须一次改完)

**Files:** Modify `crates/agent/nomi-agent/src/ssh_backend.rs`、`crates/backend/nomifun-ai-agent/src/lib.rs`、`.../manager/nomi/agent.rs`、`.../factory/nomi.rs`、`crates/backend/nomifun-ssh/src/{sink.rs,routes.rs}`、`crates/backend/nomifun-app/src/{services.rs,router/state.rs,desktop.rs}`

**Interfaces:**
- Produces(seam,`nomi-agent` 侧,**不含任何 nomifun-ssh 类型**):
  - `#[async_trait] pub trait SshSessionLease: Send + Sync { async fn release(&self) -> SshLeaseRelease; }`
  - `pub enum SshLeaseRelease { Retained{detail}, Reaped{detail}, Lost{detail} }`
  - `pub struct SshSessionBinding { pub backend: Arc<dyn SshBackend>, pub lease: Arc<dyn SshSessionLease> }`
  - `SshBackendProvider::connect(&self, user_id, conversation_id, ssh_host_id, remote_cwd) -> Result<SshSessionBinding, String>`(**新增 `conversation_id`**;V3 的两个调用点同 commit 更新)
- Produces(`nomifun-ai-agent` 侧):`NomiHostWiring` 增 `ssh_lease`;`NomiTeardownResults` 增第四字段 `ssh_lease: Option<Arc<dyn crate::SshSessionLease>>`;`fn describe_ssh_release(SshLeaseRelease) -> Result<(), AppError>`(`Lost` → `Err`)
- 删除:`SshConnectionProvider`(池取而代之实现 `SshBackendProvider`),连带 `lib.rs` 的再导出

- [ ] **Step 26: 写失败测试** — `crates/agent/nomi-agent/tests/ssh_tool_contract.rs` 扩源码扫描到新 seam 文件(仍不得含本地执行原语);`agent.rs` 内联 `#[tokio::test]`(仿既有 `finish_nomi_teardown_awaits_browser_after_*`,手写 `Arc<dyn SshSessionLease>` 假件):`ssh_lease_is_released_even_when_kill_fails`、`a_lost_ssh_link_is_a_teardown_failure`(聚合消息含 `SSH session link`)、`a_retained_link_is_not_a_failure`。
- [ ] **Step 27:** 实现 seam 三类型 + `connect` 签名拓宽 + 两个调用点(`factory/nomi.rs:748` 传 `ctx.conversation_id.as_str()`,注意它在 `:767` 被 move,须先借用/克隆;`routes.rs:133` 改走 `pool.probe`)+ `agent.rs` 保留 lease(`:661` 改为 clone backend、lease 存进 manager)+ 第四字段 + `describe_ssh_release`。
- [ ] **Step 28: 写失败测试(单例)** — `crates/backend/nomifun-app/tests/` 加 `ssh_pool_is_shared_between_routes_and_the_agent_factory`:`Arc::ptr_eq` 断言 `build_ssh_host_state(&services).pool` 与 `AgentFactoryDeps.ssh_provider` 指向同一 `PoolInner`。
- [ ] **Step 29:** 实现 `AppServices.ssh_pool`(在 `event_bus` 后、`build_agent_factory` 前构造)+ `services.rs:2957` 改为复用它 + `router/state.rs:788` 的 `build_ssh_host_state` 改为复用 + 注册 `with_delete_hook` + `DesktopServer` 加 `ssh_pool` 字段并在 `perform_shutdown`(`self.ssh_pool`,5s timeout,`close_database_after_cleanup` 之前)调 `shutdown_all()` + 删 `SshConnectionProvider`。
- [ ] **Step 30:** Run `cargo check --workspace` — PASS;`cargo test -p nomifun-ssh -p nomifun-ai-agent -p nomifun-app --lib -- --test-threads=2` — PASS。
- [ ] **Step 31: Commit** `feat(ssh): one shared connection pool behind the backend seam`

---

### Task D: `ssh.status` 实时事件 + REST 快照(TODO T2)

**Files:** Modify `crates/backend/nomifun-ssh/src/dto.rs`;Create `crates/backend/nomifun-ssh/src/events.rs`;Modify `src/routes.rs`、`src/pool.rs`、`src/lib.rs`;Modify `crates/backend/nomifun-realtime/tests/private_event_boundary.rs`;Modify `crates/backend/nomifun-app/tests/ssh_host_e2e.rs`;Modify `ui-api-contract-version.txt`

**Interfaces:**
- Produces:`SshStatusEvent { sshHostId, conversationId, state: SshLinkPhase, attempt, nextRetryInMs: Option<u64>, hostFingerprint: Option<String>, detail: Option<String>, reaped: Option<bool>, changedAt }`(`#[serde(rename_all="camelCase")]`,与本 crate 既有 DTO 及 UI 的 `ssh` 命名空间一致 —— **不**用 terminal 的 snake_case,否则 `parseSshHostId(raw.sshHostId)` 断掉);`SshStatusEvent::from_state(&SshLinkKey, &SshLinkState) -> Self` 是唯一构造路径;`SshEventEmitter { user_events: Arc<dyn UserEventSink> }` + `emit_status(&self, owner_id, &SshStatusEvent)` 发 `"ssh.status"`;`GET /api/ssh-hosts/statuses` → `Vec<SshStatusEvent>`(owner 限定)

- [ ] **Step 1: 写失败测试(投影全覆盖)** — `dto.rs` 内 `#[cfg(test)]`:`status_event_projects_every_link_state`(七个 `SshLinkState` 变体各断言 phase/attempt/nextRetryInMs/reaped;`Closed{Lost}` 的 `reaped` 必须是 `Some(false)`,非 Closed 一律 `None`)、`status_event_never_serializes_credential_material`(序列化后不含 password/private key/passphrase 字样)。FAIL → 实现 → PASS。
- [ ] **Step 2: 写失败测试(发射器)** — `events.rs` 内 `#[cfg(test)]`:`RecordingUserEvents { deliveries: Mutex<Vec<(String, WebSocketMessage<Value>)>> }` 实现 `UserEventSink`;`status_event_shape` 断言 `name == "ssh.status"` 且 owner 透传;`emitter_is_owner_scoped` 断言不会广播给他人。实现时**自查**:文件内(含注释)不得出现 `EventBroadcaster` / `WebSocketManager`(V6)。
- [ ] **Step 3:** 在 `private_event_boundary.rs:6-21` 的列表中按字母序插入 `("ssh/events", include_str!("../../nomifun-ssh/src/events.rs")),`。Run `cargo test -p nomifun-realtime --test private_event_boundary` — PASS。
- [ ] **Step 4: Commit** `feat(ssh): user-scoped ssh.status emitter and wire projection`
- [ ] **Step 5:** 把发射接进池:`PoolInner::publish` 在写 watch 后立即 `from_state` + `emit_status`;仅在**状态真变化**时发(keepalive tick 不发)。测试 `connect_emits_ssh_status_to_the_owner_only`(用 recording sink 装配池)。
- [ ] **Step 6: Commit** `feat(ssh): publish link transitions to the realtime bus`
- [ ] **Step 7: 写失败测试(REST 快照)** — `crates/backend/nomifun-app/tests/ssh_host_e2e.rs`:`statuses_snapshot_is_owner_scoped_and_matches_the_link_state`、`statuses_route_does_not_shadow_the_ssh_host_id_capture`(GET 一个真实 host id 仍解析到 `get_one`)。
- [ ] **Step 8:** 实现 `SshHostRouterState { service, pool: Option<SshConnectionPool> }`(替掉 `provider`)+ `/api/ssh-hosts/statuses` + `test_connection` 改走 `pool.probe`。
- [ ] **Step 9:** `ui-api-contract-version.txt` 5→6;Run `bun run build:ui`(V10,否则 boot 校验失败);`CHANGELOG.md` 加条目并注明 contract bump。
- [ ] **Step 10:** Run `cargo test -p nomifun-app --test ssh_host_e2e -- --test-threads=2` — PASS。
- [ ] **Step 11: Commit** `feat(ssh): status snapshot route and pool-backed test-connection`

---

### Task E: 会话 header 状态药丸 + 前端接线(TODO T1)

**Files:** Modify `ui/src/common/adapter/ipcBridge.ts`;Create `ui/src/renderer/pages/conversation/hooks/useSshLinkStatus.ts`;Modify `ui/src/renderer/components/capability/capabilityStatusColors.ts`;Create `ui/src/renderer/pages/conversation/components/SshHostStatusPill.tsx` + `.structure.test.ts`;Modify `ui/src/renderer/pages/conversation/components/ChatConversation.tsx`;Modify `ui/src/renderer/services/i18n/locales/{en-US,zh-CN}/ssh.json`

**Interfaces:**
- Consumes:`capabilityHeaderButtonClass/Style`(`components/CapabilityHeaderButton.ts:10-16`)、`CAPABILITY_COLORS`(`components/capability/CapabilityIcon.tsx:19-27`)、既有 nomi `headerExtra`(`ChatConversation.tsx:507-517`,注意 `ExecutionConversationLayout:30-46` 会再包一层)
- Produces:`ISshLinkPhase`、`IApiSshStatus`、`ssh.statuses`、`ssh.onStatus = wsMappedEmitter('ssh.status', fromApiSshStatus)`、`useSshLinkStatus(conversationId, sshHostId)`、`SSH_STATUS_COLOR: Record<ISshLinkPhase, string>`、`SshHostStatusPill`

- [ ] **Step 1: 写失败测试(wire 契约)** — `ipcBridge.wire-contract.test.ts`:断言字面量 `'ssh.status'` 与 `/api/ssh-hosts/statuses` 存在,`fromApiSshStatus` 对原始 payload 品牌化 `sshHostId`,且源码**不**用 `IApiSshHost.status`(那是每主机的陈旧提示列,不是活状态)。
- [ ] **Step 2: 写失败测试(颜色表 + i18n)** — 纯测试断言 `SSH_STATUS_COLOR` 覆盖七个 phase 且 `connected → CAPABILITY_COLORS.active`、`dropped → CAPABILITY_COLORS.danger`;`sshLocales.test.ts` 断言每个 `ssh.status.*` / `ssh.pill.*` 键在两个 locale 都存在。
- [ ] **Step 3: 写失败测试(结构)** — `SshHostStatusPill.structure.test.ts`:含 `capabilityHeaderButtonClass(`、`capabilityHeaderButtonStyle(dotColor)`、`SSH_STATUS_COLOR`、`useSshLinkStatus`、裸 `import { Server } from '@icon-park/react';`、`data-testid='ssh-host-status-pill'`、禁用态 `<span className='inline-flex'>` 包裹;负向:不含裸 `rgb(` 字面量、不含 `IApiSshHost.status`。另断言 `ChatLayout/index.tsx` 未被本任务修改(读取该文件,断言不含 `ssh`)。
- [ ] **Step 4:** Run `bun test --cwd ui` 上述三处 — Expected: FAIL。
- [ ] **Step 5:** 实现 bridge + hook(快照 `ssh.statuses` → 增量 `ssh.onStatus` 按 `(conversationId, sshHostId)` 过滤 → `ws.reconnected` 重取)+ 颜色表 + 药丸 + `ChatConversation.tsx` 注入(在既有 headerExtra `<div>` 内、`<CronJobManager>` 之前;`sshHostIdOf(conversation)` 读 `extra.ssh_host_id`)+ i18n 双语键 + `bun run gen:i18n`。
- [ ] **Step 6:** Run `bun test --cwd ui` — PASS;Run `bun run check` — PASS。
- [ ] **Step 7: Commit** `feat(ssh): conversation header host status pill`

---

## 验证阶梯(收尾,命令跑不了就如实写 Not run + 原因)

```bash
# 单元 / 集成(限并发)
cargo test -p nomi-ssh -p nomifun-ssh -- --test-threads=2
cargo test -p nomifun-ai-agent -p nomifun-realtime -- --test-threads=2
cargo test -p nomifun-app --test ssh_host_e2e -- --test-threads=2
cargo check --workspace
# --no-deps 是必须的:`-D warnings` 会传播到 path 依赖,而 nomifun-common 有 9 个
# 既有 clippy 错误(非本轮引入),不加 --no-deps 时任何 crate 的 lint 都会因它失败。
cargo clippy -p nomi-ssh -p nomifun-ssh -p nomi-agent --all-targets --no-deps -- -D warnings

# 真 sshd 端到端(手工验收):连接 → 复用 → 杀 sshd → 退避重连 → 状态事件 → 取证关闭
cargo run -p nomifun-ssh --example demo_ssh

# 前端
bun run check          # typecheck + i18n + theme + icons + 三个 boundary + help
bun test --cwd ui
bun run build:ui       # contract 5→6 之后必须跑

# 规矩审计
ls .github/workflows   # 只应有 README.md
git log --format='%an <%ae>' -40 | sort -u   # 只应出现 RiKa0-0 <2206491416@qq.com>
```

---

## Self-Review

- **Spec 覆盖**:§15 Phase 2/3 中的「连接池 quiesce」「九状态非模态处理」由 Task C 的 `SshLinkState` 七态 + Task E 的颜色表覆盖(`Idle/Connecting/Connected/Degraded/Reconnecting/Dropped/Closed`;spec §11 的「认证失败」「主机密钥变更」落在 `Dropped{retryable:false}` + `detail`);`ssh.status` 实时通道落 Task D;§10 的「顶层独立分组」落 Task A。T4/T5/T6/T7/T8 明确不在本轮。
- **跨界类型一致性**:Rust `SshStatusEvent`(camelCase serde)↔ TS `IApiSshStatus` 字段逐一对应(`sshHostId/conversationId/state/attempt/nextRetryInMs/hostFingerprint/detail/reaped/changedAt`);Rust `SshLinkPhase` 的 camelCase 变体名 ↔ TS `ISshLinkPhase` 七个字面量 ↔ `SSH_STATUS_COLOR` 的七个键 ↔ `ssh.status.*` 七个 i18n 键。四处必须同增同减,Task D Step 1 与 Task E Step 2 的测试各自钉住一端。
- **seam 纯净**:`nomi-agent/src/ssh_backend.rs` 只引 `std`/`async_trait`,新增三类型不含任何 `nomifun-*` 类型;`nomifun-ssh` 经 `nomifun-ai-agent` 的再导出反向消费 seam,方向不变,循环依赖不成立。
- **每个 commit 可编译**:唯一的宽签名改动(`SshBackendProvider::connect`)与其两个调用点、`SshConnectionProvider` 的删除、`AppServices`/`DesktopServer` 的字段新增全部压在 Task C Step 27/29 的同一对 commit 内;`SshStatusEvent` 先于池的发射接线落地(Task D Step 1-4 → Step 5)。
- **风险前置**:V7(死通道被当超时)是整个取证与探活的地基,故排在 Task C 第一步;V2(双 provider 实例)若不先解决,池对路由层不可见 —— 由 `Arc::ptr_eq` 测试永久钉住;V10(contract bump 必须跟 `build:ui`)写进 Task D Step 9 而不是留到收尾。
- **无占位符**:本文不含 TBD;Phase 3 的证书/agent 认证仍是 `connection.rs::authenticate` 里既有的显式返回,不在本轮触碰。
