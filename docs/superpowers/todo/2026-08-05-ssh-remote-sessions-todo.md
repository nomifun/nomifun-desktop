# SSH 远程会话 — 剩余需求 TODO（Phase 2 / 3）

- 日期:2026-08-05
- 状态:Phase 1 已完成并全面验证（见 `docs/superpowers/specs/2026-08-04-ssh-remote-sessions-design.md` §15 分期、`plans/2026-08-04-plan-a-ssh-remote-sessions.md`）
- 分支:`feature/ssh-remote-session`（已合并入本地 `main`）
- 本文用途:记录 Phase 1 之后待做项，供下次唤醒继续。文末附**启动 prompt**。

---

## 已完成（Phase 1，勿重做）

后端全链路 + UI 主链路 + 文档，全部对真 sshd 端到端验证通过（`cargo run -p nomifun-ssh --example demo_ssh` → ✅）：

- `crates/shared/nomi-ssh`:russh/russh-sftp 传输——连接、密码+私钥认证、known_hosts（首连 accept-new / 变更阻止）、sentinel 持久 shell（cwd/env 保持）、SFTP 原子写、sudo 传输层注入。
- `migration 024_ssh_hosts` + owner-scoped 仓库 + id_schema 四处注册（boot-verified）。
- `crates/backend/nomifun-ssh`:加密主机簿 CRUD（`***` 掩码往返）、`SshBackendProvider` 连接 seam、`/api/ssh-hosts` 路由（实例主人限定）。
- agent:`SshBackend` seam + 6 个远程工具顶替 `Bash/Read/Edit/Write/Grep/Glob`、bootstrap `.ssh_session()`、factory 接线（`extra.ssh_host_id` / `extra.ssh_remote_cwd`）。
- UI:主机簿设置页 `/settings/ssh-hosts`、四认证表单 modal、创建流（每行「新建会话」）、侧栏排除 SSH 会话、i18n `ssh` 命名空间（中英）。
- 文档:guide（中英）、CHANGELOG、api-overview、backend-crates、data-and-storage、STATUS、`ui-api-contract-version` 4→5。

---

## 待做（TODO，按优先级）

### T1. 会话页 header 状态药丸（体验增强，优先）
**为什么**:让用户在会话里随时无需阅读就知道 agent 正在操控哪台远程主机——本地/远程混淆是本功能主要失效模式。当前身份感知靠「会话名=主机名 + 侧栏独立分组」，够用但不够醒目。
**做什么**:在 `ui/src/renderer/pages/conversation/components/ChatConversation.tsx` 的 nomi 分支（约 :677，`companion_session` 分支旁）为 `conversation.extra?.ssh_host_id` 增加处理；经 `ChatLayout` 的 `headerExtra` 注入一个状态 pill（复用 `capabilityHeaderButtonClass/Style` + `CAPABILITY_COLORS`，仿 `SummonPanel` 的 header badge）。pill 显示主机名 + 连接态色标，popover 里显示 `user@host:port` / 指纹 / sudo 是否已存。
**风险点**:prop 要穿过 `NomiConversationPanel`；`headerExtra` 已是既有注入面，避免改 `ChatLayout` 本身。禁用态 tooltip 需裹 `<span className='inline-flex'>`。
**验收**:结构测试断言用 `capabilityHeaderButtonClass` + `CAPABILITY_COLORS` + `<Server>` 裸导入；`bun run check` 绿。

### T2. 连接状态实时事件 `ssh.status`（配合 T1）
**为什么**:pill 要显示 connecting/connected/reconnecting/dropped，需要后端推状态。
**做什么**:`crates/backend/nomifun-ssh/src/events.rs` 新增 `SshEventEmitter`，仿 `nomifun-terminal/src/events.rs`（持 `Arc<dyn UserEventSink>` + `owner_id`，`send_to_user` 发 `ssh.status`）。连接生命周期任务（在连接池里）发事件；**不要**走 `AgentStreamEvent`（严格 per-turn，idle 时不推）。前端 `ipcBridge.ssh.onStatus = wsMappedEmitter('ssh.status', …)`，加 REST 快照字段供重连。
**依赖**:需要先做 T3 的连接池（状态由池持有）。

### T3. 连接池 + 生命周期取证
**为什么**:当前每次 build 新连一个 `SshConnectionHandle`（Phase 1 简化）。真实使用应池化（一 conversation 一连接、复用、退避重连），且连接关闭要能取证（不伪造 reaped）。
**做什么**:`crates/backend/nomifun-ssh` 新增 `SshConnectionPool`（`DashMap<key, Arc<...>>`、`watch<Status>`、指数退避重连仿 bridge `relay_client`）。teardown 语义:channel 关闭+收到 exit-status = 已回收；否则诚实报 Lost。挂到 `NomiTeardownResults` 第四字段 + `OnConversationDelete` hook。
**注意**:连接**不能**由 agent runtime 持有（`ConfigurationChanged` 会在换模型时销毁 runtime → 掉线）。池是独立后端服务。

### T4. 远程输出面板（只读）
**为什么**:让用户看到 agent 在远程跑了什么。
**做什么**:新组件 `RemoteOutputPanel.tsx`，仿 `ConversationTerminalPanel.tsx`——ANSI 剥离的输出尾巴、**不 mount xterm**（避免 resize 远程 PTY / 抢焦点）。经 `ChatLayout` 的 `workspaceExtraTabs` 挂入右侧工作区。数据走 `ssh.output` WS 事件（T2 的姊妹事件）+ 快照。用 `--color=never` 已保证输出无 ANSI；`stripTerminalControls`/`trimOutput`/`OUTPUT_LIMIT=32K` 可抽成共享 `terminalTail.ts`。

### T5. `~/.ssh/config` 导入（快车道）
**为什么**:绝大多数用户已有 `~/.ssh/config`，手填是摩擦。
**做什么**:后端读 `~/.ssh/config`，解析 Host/HostName/User/Port/IdentityFile（**忽略 pattern-only host 和 ProxyCommand**，仿 Codex/Termius）。主机簿空态主 CTA 改为「从 ~/.ssh/config 导入」+ 显示「已检测到 N 个 Host」。用 `russh-config 0.58` 解析。

### T6. 证书认证 + ssh-agent 认证
**为什么**:Phase 1 只做了 password + 私钥（`SshConnection::authenticate` 里 `Certificate`/`Agent` 目前返回 `AuthFailed(not yet supported)`）。
**做什么**:`crates/shared/nomi-ssh/src/connection.rs` 的 `authenticate` 补 `Auth::Certificate`（`handle.authenticate_openssh_cert(user, Arc<PrivateKey>, Certificate)`）与 `Auth::Agent`（`russh::keys::agent::client::AgentClient` + `authenticate_publickey_with`）。表单四认证 UI 已就绪，打通后端即可。

### T7. MFA / keyboard-interactive
**为什么**:很多生产主机要 2FA。
**做什么**:russh 的 `InfoRequest{prompts}` 多轮循环，经 Tauri IPC 让 UI 中途应答，答案不 touch 日志/模型上下文。spike S9。

### T8. ProxyJump / 跳板机（可选，范围较大）
`russh` 不原生执行 ProxyJump；`russh-config` 只解析成字符串。用 `channel_open_direct_tcpip` 分层实现（参 async-ssh2-tokio `connect_via`）。

---

## 已知遗留 / 独立问题（非本功能引入，勿混入）

- **IDMM `"exec"` vs `"execute"` 字符串不匹配**（`nomifun-idmm/src/probe.rs`）:导致 exec 类调用被 IDMM 额外自动确认。既有 bug，本地/远程同受影响，作为独立项处理。
- `nomifun-common` / `nomifun-db` 有既有 clippy 债（`-D warnings` 才暴露，正常构建不跑）——非本功能引入。

---

## 待验证 spike（实现前置，来自 spec §附）

- S1:`ring` backend 下 russh 在 Android/iOS/Windows-ARM64 交叉编译（`feature/mobile-bridge` 相关）。
- S3:PTY stderr 交织与 sentinel 对超大输出 / `vim`/`top` 的鲁棒性（Phase 1 已对普通输出验证）。
- S4:russh-sftp 2.4.0 是否暴露 `posix-rename@openssh.com`（Phase 1 已用 remove+rename 回退兜底）。

---

## 验证与规矩（每次都要遵守）

- 测试限并发:`cargo test -p <crate> -- --test-threads=2`；全量 `cargo nextest run --build-jobs 8 --test-threads 8`。**不在 /tmp 构建**。
- 测试用独立高位端口 sshd + 独立密钥（`crates/shared/nomi-ssh/tests/support/sshd.rs`），**绝不碰真实 `~/.ssh`**。
- 提交:conventional commits、**人类署名 `RiKa0-0 <2206491416@qq.com>`、无 AI trailer、不 `--no-verify`**。
- 完成前:`bun run check` 全绿；`.github/workflows/` 无 YAML；`git log` 署名审计。
- 接线避坑:`nomifun-ai-agent` 不能依赖 `nomifun-ssh`（循环）——靠 `SshBackendProvider` seam；`nomifun-app` 是唯一同时依赖两者、构造真 provider 的地方。

---

## 启动 prompt（下次唤醒直接粘这段）

```
继续 SSH 远程会话功能的 Phase 2。先读 docs/superpowers/todo/2026-08-05-ssh-remote-sessions-todo.md
和项目记忆 ssh-remote-session-progress.md 了解已完成与待做。Phase 1（传输/DB/服务/agent 工具/
factory 接线/主机簿 UI/文档）已完成并对真 sshd 验证通过、已合并入 main。

本轮按 TODO 优先级做 T1（会话页 header 状态药丸）+ T2（ssh.status 实时事件）+ T3（连接池），
让用户在会话里能实时看到「正在操控哪台主机、连接是否健康」。沿用 Phase 1 的做法：
先读真实组件对齐 API，TDD，对真 sshd 端到端验证，每步一个人类署名 commit，收尾跑 bun run check。
安全姿态保持与本地一致（见记忆 ux-over-strict-security），不加审批闸门。不 push，除非我明确要求。
```
