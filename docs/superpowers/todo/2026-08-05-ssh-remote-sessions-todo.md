# SSH 远程会话 — 剩余需求 TODO（Phase 2 / 3）

- 日期:2026-08-05
- 状态:**Phase 1 与 Phase 2 的 T1/T2/T3 均已完成并验证**（见 `docs/superpowers/plans/2026-08-05-plan-b-ssh-phase2.md`）
- 分支:`feature/ssh-remote-session-phase2`（基于本地 `main`,未推送）
- 本文用途:记录仍待做项,供下次唤醒继续。文末附**启动 prompt**。

---

## 已完成（勿重做）

**Phase 1**:后端全链路 + UI 主链路 + 文档,对真 sshd 端到端验证（`cargo run -p nomifun-ssh --example demo_ssh` → ✅）。传输层（russh/russh-sftp、sentinel 持久 shell、SFTP 原子写、known_hosts、sudo 注入）、`migration 024_ssh_hosts` + id_schema 四处注册、加密主机簿 CRUD + `/api/ssh-hosts` 路由、agent 的 6 个远程工具顶替 + bootstrap + factory 接线、主机簿设置页 + 四认证表单 + 创建流、i18n、文档。

**Phase 2（2026-08-05）**:

- **T3 连接池**:`SshConnectionPool`（`DashMap` + 每链路 `watch<SshLinkState>` + per-host 拨号闸门 + 倍增退避重连 + cwd 重放 + Degraded 时同传输重开 shell）。池是进程级后端服务,`ConfigurationChanged` 换模型不再掉线。取证关闭:`ShellCloseProof` 看到 channel close **且** exit-status/signal 才算 `Reaped`,否则如实 `Lost`。挂上 `OnConversationDelete` 与 `desktop.rs` 关机路径。**15 个真 sshd 集成测试**。
- **T2 `ssh.status`**:`SshEventEmitter`（user-scoped `/ws`,已登记进 `private_event_boundary`）+ 唯一投影 `SshStatusEvent::from_state` + `GET /api/ssh-hosts/statuses` 快照;`publish` 是 watch 的唯一写入者且只在真实变化时发,故 socket / 快照 / UI 三者不可能漂移。`ui-api-contract-version` 5→6。
- **T1 header 药丸**:`SshHostStatusPill`（七态配色走 `SSH_STATUS_COLOR`,popover 显示 `user@host:port` / 指纹 / sudo 是否已存;`retryable=false` 时给可操作提示）+ `useSshLinkStatus`（快照 + 增量 + 重连补齐）。移动端保留（身份在手机上更要紧）。
- **侧栏分组（Phase 1 遗漏的真实缺口）**:`SshSessionGroup` 按主机二级聚合。此前 SSH 会话被排除出普通列表却没有任何分组承接,**切走即再也找不回**。
- **两个前置 bug**:① 死通道被报成 `exit_code:124` 超时 → 改为 `SshError::Disconnected`（探活与取证的地基）;② PTY Ctrl-C 打不断阻塞的 `read` 时,drain 探针会被当成该 `read` 的输入吞掉 → 超时后 shell 永久失同步。②同时是 `tests/sudo.rs` 长期 flaky 的根因（修前 6 次跑失败 5 次,修后 10/10）。

### 已撤回的判断（勿再当缺口处理）

- ~~「`extra.workspace: ''` 导致 workspace rail 是死按钮」~~ — **错误**。空串是后端**自动分配**的约定信号:`nomifun-conversation/src/service.rs:4249-4275` 判空 → 分配 uuidv7 token → `:4565` `create_dir_all` → `:4592` 把真实路径写回 `extra.workspace`。SSH 会话早已拿到 spec F2 要求的 `{work_dir}/conversations/{uuid}` 本地 scratch 目录,`ChatSlider` 的空值早退不会触发。

---

## 待做（TODO,按优先级）

### T4. 远程输出面板（只读）

**为什么**:让用户看到 agent 在远程跑了什么。

**做什么**:新组件 `RemoteOutputPanel.tsx`,仿 `ConversationTerminalPanel.tsx`——ANSI 剥离的输出尾巴、**不 mount xterm**（避免 resize 远程 PTY / 抢焦点）。经 `ChatLayout` 的 `workspaceExtraTabs` 挂入右侧工作区。数据走新的 `ssh.output` WS 事件（与 `ssh.status` 同一个 `SshEventEmitter`,加一个 `emit_output`）+ 快照。用 `--color=never` 已保证输出无 ANSI;`stripTerminalControls`/`trimOutput`/`OUTPUT_LIMIT=32K` 可抽成共享 `terminalTail.ts`。

**顺带做掉 spec F1**:远程会话的 Files/Changes 展示的是一个空的本地 scratch 目录,观感上不诚实。`workspaceLocalTabs?: boolean`（默认 true）要穿 **body 链 5 跳 + chrome 链 2 跳**,并改 4 处消费点:`WorkspaceToolRail.tsx:106-120`（隐藏两项）、`WorkspaceRailBody.tsx:277-283`（兜底改首个 extra tab）、`WorkspaceRailBody.tsx:85-91` **与** `hooks/useWorkspacePanelTabs.ts:20-27`（**两份独立**的 localStorage 初始 tab,漏一处则 rail 高亮与面板体不一致）、`ChatLayout/index.tsx:136-141`（标题）。T4 本身就要动 `extraTabs`,合并做才划算。

### T5. `~/.ssh/config` 导入（快车道）

**为什么**:绝大多数用户已有 `~/.ssh/config`,手填是摩擦。

**做什么**:后端读 `~/.ssh/config`,解析 Host/HostName/User/Port/IdentityFile（**忽略 pattern-only host 和 ProxyCommand**,仿 Codex/Termius）。主机簿空态主 CTA 改为「从 ~/.ssh/config 导入」+ 显示「已检测到 N 个 Host」。用 `russh-config 0.58` 解析。

### T6. 证书认证 + ssh-agent 认证

**为什么**:目前只做了 password + 私钥（`SshConnection::authenticate` 里 `Certificate`/`Agent` 返回 `AuthFailed(not yet supported)`）。

**做什么**:`crates/shared/nomi-ssh/src/connection.rs` 的 `authenticate` 补 `Auth::Certificate`（`handle.authenticate_openssh_cert(user, Arc<PrivateKey>, Certificate)`）与 `Auth::Agent`（`russh::keys::agent::client::AgentClient` + `authenticate_publickey_with`）。表单四认证 UI 已就绪,打通后端即可。

### T7. MFA / keyboard-interactive

**做什么**:russh 的 `InfoRequest{prompts}` 多轮循环,经 Tauri IPC 让 UI 中途应答,答案不 touch 日志/模型上下文。spike S9。

### T8. ProxyJump / 跳板机（可选,范围较大）

`russh` 不原生执行 ProxyJump;`russh-config` 只解析成字符串。用 `channel_open_direct_tcpip` 分层实现（参 async-ssh2-tokio `connect_via`）。

### T9. 主机删除 / 改凭据时主动断链（小,Phase 2 遗留）

路由层现已持有池,但 `DELETE /api/ssh-hosts/{id}` 与 `PUT`（改认证材料）尚未调 `pool.close_for_host(...)` + poison 闸门。**当前不是缺陷**:主机行被删后 `decrypt_credential` 失败 → 非重试的 `SshDialError::Credential` 会在一次尝试后停止阶梯。真正待定的是「`UpdateSshHostRequest` 的哪些字段算改了认证材料」——这是设计问题,不是接线问题。

### T10. `nomi-ssh` 的 `INIT_READY_TIMEOUT` 在高负载下过紧

远程 shell 首个 sentinel 的预算硬编码 5s。本机 load 40+ 时,**shell 初始化本身**会超这个预算（与池无关）。池的集成测试因此带一个环境闸门:仅这一种错误打印 `SKIP:` 并返回,其他任何拨号失败照旧 panic。要让这些测试在繁忙机器上稳定,得重新审视这个常量。

---

## 已知遗留 / 独立问题（非本功能引入,勿混入）

- **IDMM `"exec"` vs `"execute"` 字符串不匹配**（`nomifun-idmm/src/probe.rs`）:导致 exec 类调用被 IDMM 额外自动确认。既有 bug,本地/远程同受影响。
- **clippy 债**:`-D warnings` 会传播到 path 依赖,`nomifun-common`、`nomi-process-runtime` 各有既有错误,**`nomi-agent` 自身**也有 2 lib + 6 lib-test 错误。故 `cargo clippy -p nomi-agent ... -- -D warnings` 即使加 `--no-deps` 也不可能绿;只有 `-p nomifun-ssh --no-deps` 是干净的。
- **既有测试失败**（非本轮引入）:UI 6 个（`workspaceToolRail`、`knowledgeDetailActionBar` ×3、`MarkdownTypography`、`directorySelectionApi`）;`nomifun-app --lib` 的 `bootstrap::work_dir::tests::ignored_v1_control_replay_cannot_redirect_or_clear_a_later_v3_root`（已在 HEAD 上复现确认）。
- `cargo check --workspace --all-targets` 在 HEAD 上即失败:`nomifun-ai-agent/tests/agent_types_integration.rs` 需要 `--features test-support`。

---

## 待验证 spike（实现前置,来自 spec §附）

- S1:`ring` backend 下 russh 在 Android/iOS/Windows-ARM64 交叉编译（`feature/mobile-bridge` 相关）。
- S3:PTY stderr 交织与 sentinel 对超大输出 / `vim`/`top` 的鲁棒性（普通输出与超时恢复已验证）。
- S4:russh-sftp 2.4.0 是否暴露 `posix-rename@openssh.com`（已用 remove+rename 回退兜底）。

---

## 验证与规矩（每次都要遵守）

- 测试限并发:`cargo test -p <crate> -- --test-threads=2`;全量 `cargo nextest run --build-jobs 8 --test-threads 8`。**不在 /tmp 构建**。
- 测试用独立高位端口 sshd + 独立密钥 + tempdir known_hosts,**绝不碰真实 `~/.ssh`**。夹具在 `crates/shared/nomi-ssh/tests/support/sshd.rs` 与 `crates/backend/nomifun-ssh/tests/support/`（后者能停/重启同端口 sshd,供重连测试）。
- 提交:conventional commits、**人类署名 `RiKa0-0 <2206491416@qq.com>`、无 AI trailer、不 `--no-verify`**。
- 完成前:`bun run check` 全绿;`.github/workflows/` 无 YAML;`git log` 署名审计。改 `ui-api-contract-version.txt` 必须跟一次 `bun run build:ui`。
- 接线避坑:`nomifun-ai-agent` 不能依赖 `nomifun-ssh`（循环）——靠 `SshBackendProvider` seam;`nomifun-app` 是唯一同时依赖两者、构造真池的地方,且**只能构造一次**（`services.rs` 与 `router/state.rs` 曾各建一份互不可见的实例,已由 `ssh_pool_is_shared_between_routes_and_the_agent_factory` 钉住）。

---

## 启动 prompt（下次唤醒直接粘这段）

```
继续 SSH 远程会话功能。先读 docs/superpowers/todo/2026-08-05-ssh-remote-sessions-todo.md
与 docs/superpowers/plans/2026-08-05-plan-b-ssh-phase2.md 了解已完成与待做。Phase 1 与
Phase 2 的 T1/T2/T3（连接池 / ssh.status 实时状态 / header 药丸）+ 侧栏分组均已完成,
对真 sshd 端到端验证通过,在分支 feature/ssh-remote-session-phase2 上（未推送）。

本轮做 T4（远程输出面板,顺带 spec F1 的 workspaceLocalTabs）+ T5（~/.ssh/config 导入）。
沿用既有做法:先读真实组件对齐 API,TDD,对真 sshd 端到端验证,每步一个人类署名 commit,
收尾跑 bun run check。安全姿态保持与本地一致（见记忆 ux-over-strict-security）,
不加审批闸门。不 push,除非我明确要求。
```
