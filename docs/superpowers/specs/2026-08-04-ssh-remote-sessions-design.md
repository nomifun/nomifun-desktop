# SSH 远程会话(nomi-ssh + nomifun-ssh + 主机簿 UI)设计

- 日期:2026-08-04
- 状态:待用户复核(方向性决策已逐项确认,见下表;实现分阶段,本设计覆盖 Phase 1)
- 涉及仓库:`nomifun-tauri`(全部实现)
- 分支基线:`feature/ssh-remote-session` @ `ec1fcdf8`(v0.3.8),从 `origin/main` 切出
- 交付物:
  1. `docs/superpowers/specs/2026-08-04-ssh-remote-sessions-design.md` — 本文
  2. `docs/superpowers/plans/2026-08-04-plan-a-ssh-remote-sessions.md` — TDD 实施计划(writing-plans 阶段产出)
  3. Phase 1 代码 + 随码同行文档(`STATUS.md`、架构/参考文档表、`docs/guides/ssh-sessions.{md,zh.md}`、`CHANGELOG.md`、`ui-api-contract-version.txt` 4→5)

## 0. 背景与目标

用户把一台远程 Linux 主机的 SSH 连接信息(host / port / username + 密码 | 私钥 | 证书 | 本机 agent)交给 nomifun,然后在**普通交互式会话页**里聊天,agent 代为在该远程主机上完成一切开发操作:读文件、改文件、跑命令、搜索、装依赖。本地机器完全不参与。

这是一个在 Rust、后端、UI 三处同时全新的能力:`grep -icE 'russh|ssh2|thrussh|libssh' Cargo.lock` = `0`,仓库无任何 SSH 代码。

### 已确认的方向性决策

| 决策点 | 结论 | 备选与否决理由 |
|---|---|---|
| 工作面 | **纯远程** — 一个会话绑定一台主机,agent 一切操作在该主机上 | 否决"双机可达"(每工具加 host 参数、prompt 要教模型选机、极易选错) |
| 会话形态 | **侧栏一等分组**(顶层独立分组,按主机二级聚合),底层仍是 `type='nomi'` conversation + `extra.ssh_host_id` | 否决扩 `SessionKind` union(每个本地 workpath 节点多一个永空 `ssh:[]` 桶,四处连带改动零收益,见 §关键前提 F3);否决扩 `conversations.type` CHECK(SQLite 表级 CHECK 需 12 步建表迁移,`migrations/` 无先例,且触发闭合枚举全仓库级联) |
| 主机簿 | **已保存、可复用**的主机记录(`ssh_hosts` 表),安装主人所有,AES-256-GCM 加密,`***`+last4 掩码回传 | 否决临时凭证(仓库禁止凭证入 `conversations.extra`,"临时"实为"隐藏的一次性凭证行",工程量更大) |
| 远程可见性 | 聊天工具卡 + 只读远程输出面板(仿 `ConversationTerminalPanel`,ANSI 剥离的 tail,不 mount xterm) | 否决可交互终端 v1(见 §远程输出面板);Phase 1 先只做工具卡,输出面板置于 Phase 2 |
| 认证与生态 | 复用本机 SSH 生态:密码 / 私钥(含 passphrase) / 证书 / 本机 ssh-agent + 导入 `~/.ssh/config` + 复用 `~/.ssh/known_hosts` | 否决自成一体(用户已有 Host 别名、IdentityFile、agent key 全用不上);否决跳板机 v1(ProxyJump 明显抬高传输层复杂度) |
| 安全姿态 | **与本地完全一致** — 不加审批闸门、不拦截破坏性命令、不加确认弹窗、不加审计表;sudo 密码由传输层在识别提示符时注入,模型永不可见 | 否决独立红线闸门、否决每主机生产分级 — 用户明示"体验优先于严格安全管控" |
| 主机密钥 | **首连自动接受(写入 `~/.ssh/known_hosts`,等价 `accept-new`)+ 变更手动信任**(阻止连接,输出面板内 inline 按钮) | 否决一律自动接受(静默改写全局共用文件、MITM 不可检测);否决另存独立信任库(与"复用本机生态"冲突) |
| 传输库 | **`russh` 0.62.5 + `russh-sftp` 2.4.0**,crypto backend 用 `ring` | 见 §传输库选型 |
| 传输层归属 | **两个新 crate,不改 `nomi-process-runtime`** | 见 §总体架构;否决改造 runtime(继承满足不了的 teardown 取证契约,且 russh 传递依赖污染全链) |

## 1. 关键前提(代码考察结论,均在 `ec1fcdf8` 上核实)

**执行层完全硬编码在本地,但有一个干净的复用起点。**
- 模型的每次执行走 `BashTool` → `nomi_process_runtime::ProcessSupervisor::start(NormalizedProcessRequest)`;`platform::spawn`(`platform/mod.rs:50`)只在 `Transport::{Pipe, Pty}` 上分派;`normalize_request`(`request.rs:167`)做 `fs::canonicalize(cwd)` + `is_dir` + `cwd_roots` 包含检查 — 远程路径在 spawn 前即失败。
- 文件工具**根本不走 supervisor**:`ReadTool`→`std::fs`、`EditTool`/`WriteTool`→`std::fs`+atomic rename、`GrepTool`→裸 `tokio::process::Command` 跑 `rg`、`GlobTool`→进程内 `glob`。全仓库无 VFS trait。
- `Tool` trait(`nomi-tools/src/lib.rs:147`)方法全集:`name`/`description`/`input_schema`/`is_concurrency_safe`(必填无默认)/`execute`/`execute_with_context`/`category`/`category_for`/`is_polling_invocation`/`max_result_size`。

**F1 — Workspace 面板会对远程路径撒谎。** `WorkspaceToolRail` 把 Files/Changes 两 tab 硬编码,无隐藏开关;未知键自愈默认回 `'files'`。纯远程会话若把 `extra.workspace` 设成远程路径,本地 tree provider 会去读一个不存在的本地目录。→ 处理:新增 `workspaceLocalTabs?: boolean`(默认 `true`),SSH 会话传 `false`。

**F2 — `extra.workspace` 是 nomi 会话必填字段**(`storage.ts` 非可选),且 `ChatSlider` 在 `!extra.workspace` 时直接返回空 div → 面板体消失但 rail 按钮仍在,得到死按钮。→ 处理:`extra.workspace` 写**本地 scratch 目录**(`{work_dir}/conversations/{uuid}`,与既有 nomi 会话一致),远程 cwd 另置于 `extra.ssh_remote_cwd`。**本设计在此刻意偏离早期 UI 稿的"workspace 写远程路径"建议** — 因为该字段流入 `AgentRuntimeBuildOptions.workspace`,下游 skill 符号链接、knowledge 挂载、`mkdir` 全部假定本地路径,写远程路径会静默坏掉。

**F3 — 扩 `SessionKind` union 是净亏损。** `workpathTree.ts:12` 现为 `'interactive' | 'terminal'`;一改,每个本地 workpath 节点都要多一个永远为空的 `ssh:[]` 桶,连带 `SessionKindGroup` label 三元、折叠键等四处改动零收益。→ 处理:走 `CompanionSessionGroup`(`SessionList/CompanionSessionGroup.tsx`)先例,侧栏顶层独立分组;排除入口单点在 `conversationListFilter.ts:14` `isOrdinaryWorkConversation`,**加一行**。

**F4 — 远程工具卡若走 MCP 通道会退化成 generic。** `toolGroupSummaryModel.ts` 的 `allowExactName = !isMcp`。→ 处理:远程工具用**原生名沿用规范工具名**(`Bash`/`Read`/`Edit`/`Grep`/`Glob`),`allowExactName` 对原生名为真,process-trace 自动给出正确图标与 receipt,UI 侧零新代码。这也与"纯远程、同名工具顶替"的工作面决策一致。

**退役的 sentinel shell 已存在但不可直接用。** `crates/agent/nomi-tools/src/persistent_shell.rs`(15KB,6 测试)实现了完整的 sentinel 持久 shell(`__NOMI_END_<nonce>__<rc>__` 由追加的 `printf` 发出,精确匹配;`stty -echo`;cwd/env 跨命令保持;超时 Ctrl-C 重同步否则重生;Drop 杀进程组)。但它是 `#[cfg(test)] pub mod`,契约测试 `retired_pty_modules_and_portable_pty_are_test_only`(`architecture_contract.rs:601`)强制它**不得出现在生产源码**,`portable-pty` 只能在 dev-dependencies。→ 结论:**移植其设计到 SSH channel,不依赖其代码**。

**凭证与会话绑定的现成先例。**
- `crypto.rs`:`encrypt_string`/`decrypt_string`(AES-256-GCM,`nomifun-common`)。密钥 `AppServices.encryption_key: [u8;32]`,经 `AgentFactoryDeps.encryption_key`(`factory/mod.rs:123`)threaded。
- `remote_agents` 表(`001_v3_baseline.sql:326`)是列结构模板:`id INTEGER PK AUTOINCREMENT` + UUIDv7 业务 id(带 GLOB/length/lowercase CHECK) + host/port/user 明文 + `*_encrypted` 密文列 + status + last_connected_at;无物理 FK。`factory/remote.rs:17` 是解密→连接的蓝图(`:35/:43/:52` 解密,`:79` connect,`:91` 回写加密)。
- **会话绑定用已有的 `json_text_ref!` 契约**:`id_schema_contract.rs:840` 已有 `conversations.extra.$.remote_agent_id → remote_agents`。SSH 版:`conversations.extra.$.ssh_host_id → ssh_hosts.ssh_host_id`,`idx_conversations_extra_ssh_host_id`,`Restrict, RequireParent`(有会话绑定时不许删主机)。
- **`nomifun-secret` crate 与 `SecretValue` 在 main 上不存在**;解密后的密码/passphrase/私钥用 `zeroize::Zeroizing<String>` 承载(`zeroize 1.8.2` 已在 lock,无 crate 直接依赖,需显式加)。

**长驻连接不能由 agent runtime 持有。** Nomi runtime 无空闲驱逐,但 `AgentKillReason::ConfigurationChanged` 会在换模型等普通 UI 操作时销毁 runtime。若把 SSH 连接放在 runtime 里,用户点一下模型选择器就会掉线、重弹 passphrase、丢远程 cwd/env。→ 连接归**后端连接池服务**(在 `nomifun-ssh`),runtime 只持句柄。

**实时状态不能走 `AgentStreamEvent`(严格 per-turn),要走 user-scoped `/ws` 总线。** `UserEventSink::send_to_user`(`broadcaster.rs:25`)是进程生命周期的;`TerminalEventEmitter`(`nomifun-terminal/src/events.rs:12`,`send_to_user` 在 `:72`)是逐字照抄的发射器模板。SSH 版发 `ssh.status`/`ssh.output` 等,由连接生命周期任务持 `Arc<dyn UserEventSink>` + `owner_id` 发出;再加 REST 快照字段供重连。

**安全姿态基线(与本地一致,如实记录不藏):** 默认 session mode = `yolo`(`factory/nomi.rs`,注释"所有 nomi 会话默认自动批准"),yolo 下全类别自动批准;审批有三条绕过路径;IDMM 因 `"exec"`/`"execute"` 字符串不匹配会额外自动确认 exec 类调用 — 这是**既有 bug**,同时影响本地/远程,本 feature 不修,作为独立发现记此。全仓库无命令黑名单、无执行审计日志。SSH exec 工具声明 `ToolCategory::Exec`,走**同一条**审批管道,**不**加入 `config.tools.allow_list`(那是审批绕过名单)。这套一致性的后果按仓库诚实注释惯例(如 `path_guard.rs` "说清自己不是沙箱")明写入 §威胁模型。

## 2. 总体架构

两个新 crate,严格分层。传输层 crate 依赖隔离:manifest 不得含 `nomifun-`/`nomi-types`/`nomi-agent`/`nomi-tools`/`rusqlite`/`sqlx`/`tauri`(与 `nomi-process-runtime` 同款隔离,新加契约测试守护)。

```
┌─ crates/agent/nomi-agent ──────────────────────────────────────────┐
│  bootstrap.rs: 新 builder setter .ssh_session(SshSession)          │
│    → 选择远程实现顶替 Bash/Read/Edit/Grep/Glob 的 name()           │
│  ssh_tools.rs (新): impl Tool，方法内调 trait SshBackend           │
│  pub trait SshBackend (新): run_command / read_file / write_file  │
│    / edit_file / grep / list_files / stat，async，&self，Result    │
│    <_,String>；不暴露 connect(），连接凭证/池全在实现方背后         │
└────────────────────────────────────────────────────────────────────┘
             ▲ 经 nomifun-ai-agent seam 再导出 SshBackend
┌─ crates/backend/nomifun-ssh (新) ──────────────────────────────────┐
│  SshHostService   主机簿 CRUD（加密/掩码/owner 限定）              │
│  SshConnectionPool 长驻连接：DashMap<key, Arc<SshConnection>>，     │
│    watch<Status>，Arc<dyn UserEventSink>，退避重连，proven close   │
│  SshBackendSink   impl SshBackend（凭证解密 + 池查找，用 into_arc） │
│  routes.rs        /api/ssh-hosts CRUD + test-connection            │
│  events.rs        ssh.status / ssh.output（user-scoped /ws）        │
│  依赖：nomi-ssh, nomifun-common(crypto), nomifun-db, nomifun-realtime│
└────────────────────────────────────────────────────────────────────┘
             ▲ 依赖
┌─ crates/shared/nomi-ssh (新) ──────────────────────────────────────┐
│  纯 russh 适配层，零 nomi-*/nomifun-* 依赖，可独立对真 sshd 测试    │
│  SshCredential { host,port,user, Auth::{Password|Key|Cert|Agent} } │
│  SshConnection: connect/auth/keepalive/reconnect；known_hosts 校验  │
│  RemoteShell: sentinel 持久 shell（移植 persistent_shell.rs 设计） │
│    - cwd 编进 marker：printf '__NOMI_END_%s__%d__%s__\n' n $? $PWD  │
│    - 两级超时；软超时空命令取增量 / is_input 注入按键              │
│    - 命令走 SFTP 上传脚本再单行 bash 执行（消灭引号/注入面）        │
│    - 中断阶梯：Ctrl-C(0x03) → Channel::signal(INT) → signal(TERM)  │
│  RemoteFs: SFTP（read/write/edit/stat/list，原子写 temp+rename）    │
│  SudoResponder: 提示符识别 → 注入密码（模型永不可见）；可扩展应答表 │
│  依赖：russh, russh-sftp, russh-keys, zeroize, tokio               │
└────────────────────────────────────────────────────────────────────┘
```

会话构建:route → `ConversationService::build_runtime_options` → `factory/nomi.rs` 解析 `extra.ssh_host_id` → 经 `AgentFactoryDeps` 拿 `SshBackend` handle → `bootstrap.ssh_session(...)` → 远程工具顶替本地同名工具。连接由 `SshConnectionPool` 持有,runtime 仅持句柄;`ConfigurationChanged` 重建 agent 但池保连接,`ConversationDeleted`/`UserCancelled` 证明关闭(经 `OnConversationDelete` hook)。

## 3. 传输库选型

**`russh` 0.62.5(Apache-2.0,2026-07-31,MSRV 1.85,edition 2024)+ `russh-sftp` 2.4.0**,`default-features = false` + `features = ["ring", ...]`。

- 唯一在单 crate 内原生 tokio 覆盖全部所需能力者:密码/私钥/**证书**(`authenticate_openssh_cert`)/agent 认证、known_hosts 校验、多 channel、PTY + `window_change` + **`Channel::signal`**、SFTP、端口转发、MFA(`partial_success`/`remaining_methods` + keyboard-interactive 多轮)。
- crypto backend 选 `ring`(0.17.14 已在 lock)而非默认 `aws-lc-rs`:避开 `aws-lc-sys` 在 Windows x86 要 NASM、ARM64 要 clang-cl 的交叉编译负担。
- **否决 `ssh2`**:不支持证书认证、发不出远程信号(只能读 `exit_signal`),libssh2 自 2024-10-16 无发布,ssh2 0.9.6 为 Critical CVE-2026-55200 引用**个人 backport fork**;仅阻塞式。
- **否决 `libssh-rs`**:**LGPL-2.1 静态链接**与本 workspace 的 Apache-2.0 不兼容(单此一条即否决);vendored libssh 0.11.4 早于 2026-07-21 那批含 6 个客户端侧的 CVE;单人维护、edition 2018、无 keepalive API。
- **否决 `openssh` crate**(包系统 ssh 二进制):明文拒绝密码认证、无 PTY API、无 resize、无信号;且依赖用户机装了 `ssh`(Windows 常无)。
- **成本诚实标注**:russh 引入数十个 RustCrypto crate,含 RC 精确 pin(`rsa =0.10.0-rc.18`、`ssh-key =0.7.0-rc.11`)与 vendored `internal-russh-num-bigint`;`ed25519-dalek`/`curve25519-dalek` 各多一个大版本。仓库无 CI(`.github/workflows` 禁用),故把 `cargo audit`/`cargo deny` 加入本地 hook 阶梯;russh API churn 大(仅 2026-07 就 8 个 release),用 `nomi-ssh` 适配层 + `=0.62.x` pin 包裹隔离。

## 4. RemoteShell 完成协议(sentinel)

移植 `persistent_shell.rs` 设计到 russh PTY channel(`request_pty("xterm-256color",cols,rows,...)` + `request_shell`),`Pty`→`Channel`,中断额外用 `Channel::signal`。三处升级:

```text
每条命令（脚本已由 SFTP 上传到 ~/.cache/nomi/cmd-<nonce>.sh）:
  bash ~/.cache/nomi/cmd-<nonce>.sh
  printf '__NOMI_END_<nonce>__%d__%s__\n' "$?" "$PWD"
读到: __NOMI_END_<nonce>__<digits>__<pwd>__   → 退出码 + 远程 cwd
```

- **cwd 编进 marker**:重连后可恢复 cwd,不必客户端跟踪。
- **两级超时**:软"无变化"超时(默认 10s 静默)+ 可选硬超时;软超时期间,空命令拉取增量输出、`is_input=true` 注入按键回答交互提问,新的非空命令被拒并给指引。区分"能答交互提示"与"杀掉一切"。
- **绝不发多行命令文本**:SFTP 上传脚本再单行执行,消灭引号/注入面与 `bashlex` 类解析 bug,且支持任意 heredoc/换行。
- 保留常量:`INIT_READY_TIMEOUT=5s`、`CTRL_C=0x03`、`INTERRUPT_RESYNC_GRACE=1s`、`find_sentinel` 跳过首个出现(防命令回显被误判)。
- 初始化:`stty -echo; PS1=''; PS2=''; unset PROMPT_COMMAND`;非 POSIX 默认 shell(fish/csh)`exec /bin/bash -l` 回落 `/bin/sh`;`printf`/`stty` 缺失则降级报错。
- teardown 取证语义(诚实,不伪造):`channel 关闭 + 收到 exit-status` = 已回收;`channel 关闭无 exit-status` = `Lost`;`连接死亡远程状态未知` = 新增诚实态。绝不伪造 `reaped:true`。

## 5. 文件操作(SFTP,不用 shell 拼)

第一天即用 `russh-sftp` 做 read/write/edit/stat/list。依据:Claude Code 已公开 28 个 GHSA 绝大多数是 `sed`/`echo`/`find`/`rg`/重定向的校验绕过;OpenHands CVE-2026-33718 是路径拼进 shell。

原子写(镜像本地 `write.rs` 的 temp+rename 契约):
1. `canonicalize` 父目录,包含检查在 canonicalize **之后**(防 symlink 逃逸)。
2. `metadata(target)` 捕获 mode/mtime/size 供保留与陈旧检查。
3. 写 `<同目录>/.nomi-tmp-<nonce>`(同目录 = 同文件系统 ⇒ rename 原子)。
4. `sync_all`。
5. `set_metadata` 恢复原权限位(SFTP 创建 mode ≠ 目标 mode,否则可执行位丢失)。
6. `rename` — **SFTP v3 `rename` 目标存在即失败**;覆盖语义需 `posix-rename@openssh.com` 扩展(russh-sftp 2.4.0 是否暴露 = 待验 spike S4);不可用则 `remove_file` 后 `rename`(有非原子窗口)并如实告知。
7. 更新陈旧基线:远程无 inotify,基线用 SFTP `metadata` 的 `(size, mtime)`,rename 前重查。`EditTool` 的陈旧守护(本地比 mtime)改用远程 `(size,mtime)` 或 hash,否则守护要么虚设要么永久阻塞。

`grep`:远程有 `rg` 则用,否则 `grep -rn`;pattern **经文件传递**(`rg -f <上传的 pattern 文件>`),正则不过 shell 命令行。

## 6. sudo 与交互提示应答

- 每主机可选 `sudo_password_encrypted` 列(加密姿态同 `remote_agents.auth_token_encrypted`);留空 = 视远程为 NOPASSWD。
- agent 只写它本来会写的(`sudo systemctl restart nginx`);`SudoResponder` 在 PTY 上识别 sudo 提示符,**自己**把密码字节写进 channel stdin。
- 密码因此不在命令字符串 → 不进 `ps`、不进 transcript、不进 provider 请求。关键:`nomi_redact` 只脱敏工具**结果**、从不碰**输入**(其测试断言 `token=abc` 不被脱敏),`nomi_providers` 在 debug 级打整个请求体。让模型压根接触不到,是唯一可靠且体验最好的做法。
- sudo 15 分钟时间戳缓存 → 持久连接下一个会话只注入一次。
- **注入一次,第二次提示即停止注入并让输出直通模型,绝不重试**(远程 sudo 连败触发 PAM 锁定)。
- 机制泛化为可扩展**应答规则表**(sudo、`apt` y/n、git 凭证提示、二次 ssh 密码),不硬编码单一 sudo。

## 7. 主机密钥(host key)

- **首连自动接受**:`check_server_key` 通过后 `learn_known_hosts`,写入用户既有 `~/.ssh/known_hosts`(等价 OpenSSH `StrictHostKeyChecking=accept-new`),UI 只做事后可关闭的 info 披露 + pill popover 常驻指纹行。**接受的风险明示:首连 MITM 不可检测** — 这是"体验优先"取舍的直接后果。
- **密钥变更**:阻止连接,拒绝密码认证与 agent 转发(仿 ssh(1)),在已打开的输出面板内给一个 **inline text 按钮**(非 modal、非门,正常路径永不遇到):显示旧/新 SHA256 指纹 + `known_hosts` file:line + "信任新密钥并重连"。拒绝自动接受变更的理由不是安全洁癖:`known_hosts` 是用户与整个系统共用的文件,静默改写会让用户在 Nomi 之外的 `ssh` 也失去这道检测。
- 支持粘贴期望指纹做对照接受;支持 hashed known_hosts 两种形态;host 密钥列表裁掉末尾裸 `ssh-rsa`(SHA-1)。

## 8. 数据模型与持久化

**migration `024_ssh_hosts.sql`**(现有最高 `023`),逐列镜像 `remote_agents` 的姿态:

```sql
CREATE TABLE ssh_hosts (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    ssh_host_id            TEXT NOT NULL UNIQUE
                           CHECK ( length(ssh_host_id)=36 AND lower(ssh_host_id)=ssh_host_id
                             AND ssh_host_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(ssh_host_id,'-','') NOT GLOB '*[^0-9a-f]*' ),
    user_id                TEXT NOT NULL,          -- owner 限定（remote_agents 无此列，靠 protect_instance_owner；
                                                    -- 我们保留 type='nomi'，故显式加 user_id + 索引 + 服务层过滤）
    name                   TEXT NOT NULL,
    host                   TEXT NOT NULL,
    port                   INTEGER NOT NULL DEFAULT 22,
    username               TEXT NOT NULL,
    auth_type              TEXT NOT NULL,          -- password | key | certificate | agent
    password_encrypted     TEXT,
    private_key_encrypted  TEXT,
    passphrase_encrypted   TEXT,
    certificate_encrypted  TEXT,
    sudo_password_encrypted TEXT,
    host_fingerprint       TEXT,                   -- 首连记录的 SHA256，用于展示
    status                 TEXT NOT NULL DEFAULT 'unknown',
    last_connected_at      INTEGER,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL
);
CREATE INDEX idx_ssh_hosts_user_id ON ssh_hosts(user_id);
CREATE INDEX idx_ssh_hosts_status  ON ssh_hosts(status);
```

四处 runtime-asserted 注册(否则 app 拒绝启动,`database.rs` 每次 boot 校验):`PRODUCT_TABLES` 加 `ssh_hosts`;`UUIDV7_BUSINESS_COLUMNS` 加 `("ssh_hosts","ssh_host_id")`;`NON_REFERENCE_ID_COLUMNS` 视需要;`LOGICAL_REFERENCES` 加 `text_ref!("ssh_hosts","user_id" => "users","user_id",false,"idx_ssh_hosts_user_id",Cascade)` + `json_text_ref!("conversations","extra","$.ssh_host_id", ... => "ssh_hosts","ssh_host_id","idx_conversations_extra_ssh_host_id",Restrict,RequireParent)`。迁移文件开头带多行 `--` 注释(产品理由 + 不变式 + 已做验证),forward-only,shipped 后不改(boot 校验 checksum)。

## 9. HTTP + 实时

- `ssh_host_routes(state) -> Router`:list / create / update / delete / test-connection,镜像 `remote_agent_routes`。挂载经 `protect_instance_owner(ssh_host_routes(states.ssh), ...)`(`routes.rs:719` 先例),安装主人限定。
- 服务方法每个首参 `user_id` 并按其过滤,跨 owner 访问与 NotFound 不可区分。
- 请求 DTO `Deserialize`-only + `#[serde(deny_unknown_fields)]`;list DTO 不含任何密文;掩码 `***`+last4 回传;加密在服务层不在 repository。
- 实时:`ssh.status`(连接态)、`ssh.created`/`ssh.removed`,走 user-scoped `/ws`,由连接生命周期任务发。加 REST 快照字段供重连(仿 `ConversationRuntimeSummary` 骑在 `ConversationResponse.runtime`)。新 `/ws` 事件名无需 contract bump,但新增 HTTP 路由 + DTO → `ui-api-contract-version.txt` **4→5**。

## 10. UI

设计主张:**SSH 会话不是"开了远程开关的会话",而是"有地址的会话" — 地址在每一像素上被动可见,永不索取一次点击。** 三原则:身份恒在(侧栏行 / header leading+pill / 输出面板标题,统一走 `--brand` 色轴)、零阻断(状态用 pill 配色 + banner + 一次性 toast,无 modal 为状态而开)、不撒谎(隐藏本地 Files/Changes,远程输出只读只 tail)。零新增 theme var。

- **主机簿**:新内置 settings tab `/settings/ssh-hosts`,落 `settings.groupApp` 组。导航项**三处**登记(`SettingsSider.tsx:23` `BUILTIN_TAB_IDS`、`:63` `builtinMap`、`SettingsPageWrapper.tsx:26` 移动端第二份 `builtinMap`,漏第三处 = 移动端缺失)。空态 import-first(静默 parse `~/.ssh/config` 报"已检测到 N 个 Host");列表用行不用卡片网格(身份是文本 `user@host:port`,无头像可放)。
- **表单**(`SshHostFormModal`,复用于三入口):Arco `Form` + `NomiModal`;四认证方式用 `<Form.Item shouldUpdate noStyle>` 条件字段;密文 `<Input.Password>`;私钥兼有粘贴 textarea 与文件选择;掩码回传规则(值以 `***` 开头则从 update payload 删除,并阻止 Test Connection)。Test Connection 给真实诊断(errno + 定向提示),非阻塞。
- **侧栏**:顶层独立分组(仿 `CompanionSessionGroup`),按主机二级聚合;排除入口 `conversationListFilter.ts:14` 加一行。
- **创建流** `/ssh-new`:选/填主机 → 建会话。
- **会话页**:`headerLeading` 换 `<Server>` 方块(最强单点"不在本机"信号);`headerExtra` 挂状态 pill(复用 `capabilityHeaderButtonClass/Style` + `CAPABILITY_COLORS`);标题**不写主机名**(可改名会脱钩);工具卡零新类型(F4 靠原生名)。
- 关键 UI 陷阱规避(来自 design-system 事实核查):`border-border-2` 生成空(用 `border-arco-2`);`text-primary` 是品牌色非正文(正文 `text-t-primary`);`border-2` 是颜色非宽度;`@icon-park/react` 只裸命名导入;默认 preset 是 `rhythm-dark`,颜色用 token 不硬编码;triplet 变量(`--primary-6` 等)必须 `rgb(var(...))`。

远程输出面板(Phase 2)与九个连接状态的完整非模态处理详见 §12 与附录 UI 稿。

## 11. 错误处理

| 场景 | 行为 |
|---|---|
| 连接被拒 / 超时 | `Status::Failed(errno)`;pill danger + banner 含原始 errno + 定向提示;一次性 toast |
| 认证失败 | banner danger + "编辑主机凭据" inline 按钮;不重试 |
| 首连未知主机密钥 | 自动接受写 known_hosts;pill 仍 active;可关闭 info 披露 |
| 主机密钥变更 | 阻止连接;pill danger;输出面板内 inline "信任新密钥并重连"(唯一需用户动手处,非 modal) |
| 连接在本轮中断 | 取消本轮;pill danger;"重新连接"只重建 transport,**绝不自动重发 prompt**(避免命令跑两遍) |
| 命令超时 | 软超时可注入按键;硬超时 Ctrl-C→signal(INT)→signal(TERM);`exit_code=124`,`timed_out=true` |
| sudo 密码被拒 | 停止注入、输出直通模型;pill 保持 active(transport 健康);banner warn + "更新 sudo 密码" |
| SFTP 子系统禁用 | 连接时探测,降级为 shell base64 整文件写 + UI badge;in-place 编辑不降级 |
| teardown 无法证明远程进程树空 | 诚实报 `Lost` / "远程状态未知",绝不伪造 reaped |
| 连接掉线(非本轮) | 退避重连(仿 `relay_client` 指数退避到 60s);重连后重建 cwd/env;pill reconnecting |

## 12. 测试策略

- **`nomi-ssh` 单元 + 对真 sshd 集成**(该 crate 零 nomi 依赖,可独立测):sentinel 解析(含 cwd 段)、`find_sentinel` 跳首个出现、两级超时、中断阶梯、SFTP 原子写回环、known_hosts 首连/变更、sudo 应答识别。本机有 `sshd`(OpenSSH_10.2p1)与 docker,起独立测试 sshd(独立主机密钥 + 独立测试用户,绝不碰真实 `~/.ssh`)。
- **`nomifun-ssh`**:主机簿 CRUD 的密文永不出现在序列化(`assert!(!json.contains(plaintext))`)、错密钥失败关闭、owner 隔离、掩码回传、连接池 quiesce 取证。
- **HTTP e2e**(`nomifun-app/tests/ssh_e2e.rs`):`create_router` + `oneshot`,create/list/delete/test-connection,deny_unknown_fields,跨 owner = NotFound。
- **架构契约(新增)**:一条同 `command_adapters_delegate_to_the_process_supervisor` 形状的测试,守护远程工具文件不出现本地执行原语,补上 `:505` 那条按文件的契约对新 SSH 路径的保护空白;一条守护 `nomi-ssh` manifest 依赖隔离。
- **UI**:表单校验逻辑抽成纯 `sshHostForm.validation.ts` + `.validation.test.ts`;组件 `*.structure.test.ts` 断言关键 className/结构。
- **手动验收**:仿 `demo_desktop.rs` 思路写 `examples/` 脚本,对真 sshd 跑通连接→认证→执行→sudo 注入→断线重连,人可无 GUI 验证。
- 全量测试须限并发:`cargo nextest run --build-jobs 8 --test-threads 8`(24 核无限制打爆 30G 内存);聚焦 crate `cargo test -p <crate> -- --test-threads=2`;**不在 `/tmp` 构建**(16G tmpfs → `ld` Bus error)。main 上既有 4 个失败(3 + 1 flaky)非本 feature 回归。

## 13. 威胁模型(如实,不藏)

- 安全姿态**与本地一致**是用户明示决策:无新审批闸门、无破坏性命令拦截、无审计表;默认 `yolo` 下 SSH exec 自动批准。后果:agent 在生产机上跑 `rm -rf /var/www` 是静默执行的。这与产品 local-first、"用户自己的机器用户负责"的定位一致。
- 凭证明文永不落 `conversations.extra`、`terminal_sessions.env`、transcript、provider 请求、日志。sudo/密码经传输层注入,模型不可见。
- `known_hosts` 首连自动接受 = 首连 MITM 不可检测(明示取舍);变更手动信任,拒绝静默改写全局共用文件。
- ssh-agent 转发**默认关闭**、不继承(移除横向移动类风险;参 s1ngularity 事件)。
- 解密材料用 `Zeroizing`;私钥 `Arc` 只活在连接 actor,不进工具结构/协议事件/Tauri 返回值。
- IDMM `"exec"`/`"execute"` 不匹配是既有 bug,本 feature 不引入亦不修复,记录待独立处理。
- `SECURITY.md` 加一条部署指引:该文档当前对沙箱、host key、审计均无承诺。

## 14. 明确不做(v1 / Phase 1)

- Phase 1 不做:`~/.ssh/config` 导入、证书认证、ssh-agent 认证、MFA/keyboard-interactive、远程输出面板(置于 Phase 2)。
- v1 全程不做:ProxyJump/跳板机、可交互远程终端(v2 且需重开安全讨论)、Tier-1 上传远程 helper 二进制、Windows 远程目标(远程恒为 POSIX Linux)、独立命令黑名单/审批闸门/审计表(用户明示)、修 IDMM exec/execute bug、agent 转发默认开启。

## 15. 分期

1. **Phase 1(本设计核心,先跑通主链路)**:`nomi-ssh`(连接+密码/私钥认证+sentinel shell+SFTP)、`migration 024`、`nomifun-ssh` 服务与路由与事件、工具层接入(`SshBackend` + bootstrap setter + 远程工具顶替同名)、主机簿 UI + 创建流 + header pill、随码文档。
2. **Phase 2**:远程输出面板、`~/.ssh/config` 导入、九状态完整非模态处理。
3. **Phase 3**:证书认证、ssh-agent 认证、MFA/keyboard-interactive、连接池 quiesce 精细化。

## 附:待验证 spike(计划阶段前置)

- S1:`ring` backend 下 russh 在 Android/iOS/Windows-ARM64 交叉编译(`feature/mobile-bridge` 相关)。
- S3:PTY stderr 交织与 sentinel 对真实输出(`find /`、大输出、`vim`/`top`)的鲁棒性。
- S4:russh-sftp 2.4.0 是否暴露 `posix-rename@openssh.com`(决定原子覆盖 rename 是否可用)。
- S9:MFA/keyboard-interactive 经 Tauri IPC 的阻塞多轮往返(答案不touch 日志/模型上下文)。
