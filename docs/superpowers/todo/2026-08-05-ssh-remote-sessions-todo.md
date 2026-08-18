# SSH 远程会话 — 剩余需求 TODO（Phase 2 / 3）

- 日期:2026-08-05
- 状态:**Phase 1、Phase 2(T1/T2/T3)、T5、T6 均已完成并验证;已合并远程 main**
- 分支:`feature/ssh-remote-session-phase2`(**未推送**;已合并 `origin/main` 至 `9599ddc1`,落后 0)
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

## 第二轮(2026-08-05 晚):T5 / T6 + 一次减重审查

**T5 `~/.ssh/config` 导入**:手写解析器(不引 `russh-config`,只给本 crate 加了 `dirs`),`GET /api/ssh-hosts/import-candidates` + `POST /api/ssh-hosts/import`。取 Host/HostName/User/Port/IdentityFile,忽略通配符 Host 与 ProxyJump/ProxyCommand 条目(**具名报出**而非静默丢弃),`Include` 不跟随但**计数告知**。**POST 只接 alias 不接 identityFile** —— 否则就是一个任意文件读原语。导入时读私钥(256 KiB 上限,无 `PRIVATE KEY` 标记则不当凭据),候选列表**绝不含**私钥内容(有断言钉住)。

**T6 证书 + ssh-agent 认证**:`authenticate_openssh_cert` 与 `AgentClient` 两条分支打通,四种认证方式全部可用。证书错误按本地可判的原因分流(过期 / principal 不匹配 / CA 不被服务器信任),两种粘贴错误在拨号前就拒掉。
- **`AgentClient::connect_uds` 是 `#[cfg(unix)]`** —— 不加 gate 会破坏 Windows 构建,已加 `not(unix)` 孪生分支。
- **ssh-agent 的真实可用边界**:后端是 Tauri 桌面进程内的 axum 任务,`SSH_AUTH_SOCK` 取决于启动器导出了什么。Linux + GNOME/gnome-keyring 可用;裸 WM 或环境被净化的 `.desktop` **不可用**;macOS 一般可用;**Windows 完全不可用**(agent 是命名管道)。失败信息已明说,**刻意没有**为此做 socket 路径配置 UI。

**减重审查**(5 维度并行 → 43 条结论 → 26 条删减类经对抗验证 → **4 条成立、22 条被驳回**):结论是这份代码**并不过重**,真正的死代码只有约 86 行。已删:`state::reconnect_delay`(活的是 `PoolTuning::delay`)、`state::is_retryable`(活的是 `SshDialError::is_retryable`)、同一张可重试表的三份拷贝、UI 表单校验的两层空壳。

**审查修掉的核心可用性缺陷**(都在核心路径上,模型/用户真会撞到):

| 修的问题 | 原来会怎样 |
|---|---|
| sudo 应答正则过宽(**安全**) | `mysql -p`、嵌套 `ssh`、`git push https` 的密码提示都被判成 sudo 提示,**本机 sudo 密码被写进它们的 stdin** |
| sudo 提示未锚定末尾(**安全**) | `cat /var/log/auth.log` 等只是**打印**该字样的命令也触发注入;此时无人读 stdin,密码成为下一条命令并出现在其输出里 → **进入模型上下文** |
| PTY 未中和分页器 | `git log`/`systemctl status`/`journalctl` 启动 `less` 等按键,一直卡到超时;更糟的是 `cwd: ""` 会被池读作"不可恢复",**一条 git log 就让会话丢掉整个 shell** |
| 远程 Bash 丢弃模型传的 `timeout` | 装包/编译必然超时(它顶替的本地 Bash schema 写着 max 600000,模型会传) |
| Grep 把错误吞成空串 | 路径打错/权限不足/正则不合法,模型全读成"没有匹配"并据此下结论 |
| 拨号无超时 | 防火墙 DROP 时卡约 130s,且卡在 gate 锁内堵住同主机所有会话 |
| 删主机不掐链路 | agent 继续在已删除的主机上执行命令,而药丸直接消失 |
| 重连梯子放弃后无人再拨 | 约 5 分钟后会话永久变砖,药丸提示的"去改凭据"也不生效 |
| 系统提示写本地 scratch 目录 | 而所有工具都在远程 `$HOME`,模型被明确告知一个不存在的路径 |
| Glob 示例 `src/**/*.rs` | dash 无 globstar,深层文件静默缺失,与"真的没有"不可区分 |
| 远程 Read 无大小闸门 | 一次读大文件把整份内容灌进**用户桌面进程**内存 |
| `changed_at` 在投影时打戳 | REST 快照里该字段是"你何时提问",客户端却拿它做乱序判定与倒计时锚点 |
| 认证标签 `as never` 绕过类型 | 新增认证方式会渲染成裸 i18n 键(T6 刚让四种方式全部可达) |
| 主机删除后药丸消失 | 界面上再没有"在操哪台机"的标识 |

**测试夹具的 pid 安全**(曾因此打坏用户 shell,见记忆 `no-blind-pid-kills`):`kill_tree` 现在开火前重读 `/proc/<pid>/stat` 校验 comm+ppid,并**删掉了进程组信号**。

**合并 `origin/main`**(`4d181736`):四处冲突按真值解决 —— `id_schema_contract.rs` 取 main 的具名清单(双方都发现 `ssh_hosts` 没进断言,main 解法更好);contract version 取 **15**(我 6 / main 14 / 合并后又前进一格)。合并暴露并修掉两个冲突标记不会提示的问题:main 删了隐含 `border-style` 的工具类(**必须显式 `border-solid`**),以及 i18n 新增了 locale 键集必须匹配的校验(zh-CN 缺 9 个 `_other`)。

---

## 待做（TODO,按优先级）

### T4. 远程输出面板（只读）

**为什么**:让用户看到 agent 在远程跑了什么。

**做什么**:新组件 `RemoteOutputPanel.tsx`,仿 `ConversationTerminalPanel.tsx`——ANSI 剥离的输出尾巴、**不 mount xterm**（避免 resize 远程 PTY / 抢焦点）。经 `ChatLayout` 的 `workspaceExtraTabs` 挂入右侧工作区。数据走新的 `ssh.output` WS 事件（与 `ssh.status` 同一个 `SshEventEmitter`,加一个 `emit_output`）+ 快照。用 `--color=never` 已保证输出无 ANSI;`stripTerminalControls`/`trimOutput`/`OUTPUT_LIMIT=32K` 可抽成共享 `terminalTail.ts`。

**顺带做掉 spec F1**:远程会话的 Files/Changes 展示的是一个空的本地 scratch 目录,观感上不诚实。`workspaceLocalTabs?: boolean`（默认 true）要穿 **body 链 5 跳 + chrome 链 2 跳**,并改 4 处消费点:`WorkspaceToolRail.tsx:106-120`（隐藏两项）、`WorkspaceRailBody.tsx:277-283`（兜底改首个 extra tab）、`WorkspaceRailBody.tsx:85-91` **与** `hooks/useWorkspacePanelTabs.ts:20-27`（**两份独立**的 localStorage 初始 tab,漏一处则 rail 高亮与面板体不一致）、`ChatLayout/index.tsx:136-141`（标题）。T4 本身就要动 `extraTabs`,合并做才划算。

### T7. MFA / keyboard-interactive

**做什么**:russh 的 `InfoRequest{prompts}` 多轮循环,经 Tauri IPC 让 UI 中途应答,答案不 touch 日志/模型上下文。spike S9。

### T8. ProxyJump / 跳板机（可选,范围较大）

`russh` 不原生执行 ProxyJump;`russh-config` 只解析成字符串。用 `channel_open_direct_tcpip` 分层实现（参 async-ssh2-tokio `connect_via`）。

### T9. 改凭据时主动断链（小,剩一半）

`DELETE /api/ssh-hosts/{id}` **已接线**:删主机会调 `pool.close_for_host(...)` 掐掉活链路。剩下的是 `PUT`(改认证材料)——**当前不是缺陷**,改了凭据后旧链路继续用旧连接跑,直到掉线重拨才会用新凭据。真正待定的是「`UpdateSshHostRequest` 的哪些字段算改了认证材料」,这是设计问题不是接线问题。

### T10. `nomi-ssh` 的 `INIT_READY_TIMEOUT` 在高负载下过紧

远程 shell 首个 sentinel 的预算硬编码 5s。本机 load 40+ 时,**shell 初始化本身**会超这个预算（与池无关）。池的集成测试因此带一个环境闸门:仅这一种错误打印 `SKIP:` 并返回,其他任何拨号失败照旧 panic。要让这些测试在繁忙机器上稳定,得重新审视这个常量。

---

## 已知遗留 / 独立问题（非本功能引入,勿混入）

- **IDMM `"exec"` vs `"execute"` 字符串不匹配**（`nomifun-idmm/src/probe.rs`）:导致 exec 类调用被 IDMM 额外自动确认。既有 bug,本地/远程同受影响。
- **clippy 债**:`-D warnings` 会传播到 path 依赖,`nomifun-common`、`nomi-process-runtime` 各有既有错误,**`nomi-agent` 自身**也有 2 lib + 6 lib-test 错误。故 `cargo clippy -p nomi-agent ... -- -D warnings` 即使加 `--no-deps` 也不可能绿;只有 `-p nomifun-ssh --no-deps` 是干净的。
- **既有测试失败**（非本轮引入）:全量实测 13416 个测试里 14 个失败,全部集中在 `nomifun-common::factory_reset::*`(一整族)、`nomi-process-runtime`、`nomifun-extension`、`nomifun-knowledge`、`nomifun-app` 的 `bootstrap::work_dir` 与 `agent_integration_e2e`(后者高并发下 flaky,单跑即过),以及 `nomi-agent` 的 openclaw 构造测试(已在基线 commit 上复现确认);UI 侧 6 个(`workspaceToolRail`、`knowledgeDetailActionBar` ×3、`MarkdownTypography`、`directorySelectionApi`)。
- 本轮顺手修掉两个**与 SSH 无关**的仓库问题:`agent_types_integration` 引用已改名字段导致整个 workspace 的 nextest 编译不了(`fa71ecc8`);`id_schema_contract` 的产品表断言漏了 `ssh_hosts`(`2585c2c8`,合并时让位给 main 更好的具名清单解法)。

---

## 待验证 spike（实现前置,来自 spec §附）

- S1:`ring` backend 下 russh 在 Android/iOS/Windows-ARM64 交叉编译（`feature/mobile-bridge` 相关）。
- S3:PTY stderr 交织与 sentinel 对超大输出 / `vim`/`top` 的鲁棒性（普通输出与超时恢复已验证）。
- S4:russh-sftp 2.4.0 是否暴露 `posix-rename@openssh.com`（已用 remove+rename 回退兜底）。

---

## 验证与规矩（每次都要遵守）

- 测试限并发:`cargo test -p <crate> -- --test-threads=2`;全量 `cargo nextest run --build-jobs 8 --test-threads 8 --features nomifun-ai-agent/test-support`（**少了那个 feature,`agent_types_integration` 这个 test 目标编译不过 → 整个 workspace 一个测试都跑不了**;注意 `cargo … | grep` 的退出码是 grep 的,别把 build 失败读成全绿）。**不在 /tmp 构建**。
- 测试用独立高位端口 sshd + 独立密钥 + tempdir known_hosts,**绝不碰真实 `~/.ssh`**。夹具在 `crates/shared/nomi-ssh/tests/support/sshd.rs` 与 `crates/backend/nomifun-ssh/tests/support/`（后者能停/重启同端口 sshd,供重连测试）。
- 提交:conventional commits、**人类署名 `NomiFun Contributor <nomifun@users.noreply.github.com>`、无 AI trailer、不 `--no-verify`**。
- 完成前:`bun run check` 全绿;`.github/workflows/` 无 YAML;`git log` 署名审计。改 `ui-api-contract-version.txt` 必须跟一次 `bun run build:ui`。
- 接线避坑:`nomifun-ai-agent` 不能依赖 `nomifun-ssh`（循环）——靠 `SshBackendProvider` seam;`nomifun-app` 是唯一同时依赖两者、构造真池的地方,且**只能构造一次**（`services.rs` 与 `router/state.rs` 曾各建一份互不可见的实例,已由 `ssh_pool_is_shared_between_routes_and_the_agent_factory` 钉住）。

---

## 启动 prompt（下次唤醒直接粘这段）

```
继续 SSH 远程会话功能。先读 docs/superpowers/todo/2026-08-05-ssh-remote-sessions-todo.md
与 docs/superpowers/plans/2026-08-05-plan-b-ssh-phase2.md 了解已完成与待做。Phase 1 与
Phase 2 的 T1/T2/T3（连接池 / ssh.status 实时状态 / header 药丸）+ 侧栏分组均已完成,
对真 sshd 端到端验证通过,在分支 feature/ssh-remote-session-phase2 上（未推送）。

T5（~/.ssh/config 导入）与 T6（证书 + ssh-agent 认证）已完成,远程 main 也已合并进来。
本轮若还要推进,只剩 T4（远程输出面板,顺带 spec F1 的 workspaceLocalTabs）与 T7-T10 —— 都是可选项,
核心功能已经可用好用,不必为了做而做。
沿用既有做法:先读真实组件对齐 API,TDD,对真 sshd 端到端验证,每步一个人类署名 commit,
收尾跑 bun run check。安全姿态保持与本地一致（见记忆 ux-over-strict-security）,
不加审批闸门。不 push,除非我明确要求。
```
