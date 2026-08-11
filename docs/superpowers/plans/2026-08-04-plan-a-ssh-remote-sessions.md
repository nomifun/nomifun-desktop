# SSH 远程会话 Phase 1 实施计划(Plan A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让用户保存加密的 SSH 主机凭证,在普通聊天会话里由 agent 代为操作远程 Linux 主机(读/写/编辑文件、跑命令、搜索),本地机器不参与。

**Architecture:** 三层。`crates/shared/nomi-ssh`(纯 russh 适配,零 nomi/nomifun 依赖,可独立对真 sshd 测)提供连接/认证/sentinel 持久 shell/SFTP。`crates/backend/nomifun-ssh` 提供主机簿 CRUD、长驻连接池、HTTP 路由、`/ws` 事件,并 `impl` agent 层的 `SshBackend` seam trait。`crates/agent/nomi-agent` 新增 `SshBackend` trait + 远程工具(顶替 `Bash`/`Read`/`Edit`/`Write`/`Grep`/`Glob` 的 `name()`)+ bootstrap setter。`nomi-process-runtime` 完全不动。

**Tech Stack:** Rust(edition 2024)、`russh 0.62.5` + `russh-sftp 2.4.0`(crypto backend `ring`)、`zeroize`、tokio、sqlx/SQLite、axum、React 19 + Arco Design + UnoCSS。

**设计依据:** `docs/superpowers/specs/2026-08-04-ssh-remote-sessions-design.md`(每条决策与 file:line 引用均在此)。

## Global Constraints

- **Git 署名必须是人类**:author/committer = `RiKa0-0 <2206491416@qq.com>`;**禁止** `Co-authored-by`/`Generated-by`/`Assisted-by` 等 AI trailer;**禁止** `--no-verify`;不改全局 git 配置(`AGENTS.md:28-54`)。
- **禁止 GitHub Actions**:不得在 `.github/workflows/` 新增任何 YAML;完成前 `ls .github/workflows` 只应有 `README.md`(`AGENTS.md:3-26`)。
- **Conventional Commits**:`<type>(ssh): <祈使>`;每个 Task 末尾一个 commit。
- **测试并发限制**:聚焦 crate 用 `cargo test -p <crate> -- --test-threads=2`;全量用 `cargo nextest run --build-jobs 8 --test-threads 8`(24 核无限制打爆 30G 内存)。**绝不在 `/tmp` 构建**(16G tmpfs → `ld` Bus error)。
- **既有失败非回归**:main 上既有 3 个失败 + 1 flaky(`recovery::tests::capture_probe_terminate_prove_the_full_lifecycle` 等),不得据此判定本 feature 破坏。
- **禁词(`check:agent-vocabulary` 扫 `.md` 与源码)**:`orchestrat*`、`sub-agent`、`fleet`、`shared_tasks`、`taskboard`、`persistent_execution`、`agentRuntime`(小写边界) — 代码注释与文档都不得出现。
- **新第三方依赖**:声明于根 `Cargo.toml [workspace.dependencies]`,各 crate 用 `{ workspace = true }` 消费。
- **传输层隔离**:`nomi-ssh` 的 `Cargo.toml` 不得含 `nomifun-`/`nomi-types`/`nomi-agent`/`nomi-tools`/`rusqlite`/`sqlx`/`tauri`(新加契约测试守护)。
- **凭证纪律**:明文密码/passphrase/私钥用 `zeroize::Zeroizing` 承载;永不进 `conversations.extra`、transcript、provider 请求、日志、Tauri 返回值;list DTO 不含密文;读回掩码 `***`+last4。
- **UI 陷阱**:`border-border-2` 生成空(用 `border-arco-2`);`text-primary` 是品牌色(正文用 `text-t-primary`);`@icon-park/react` 只裸命名导入;颜色用 token 不硬编码;triplet 变量用 `rgb(var(--x))`;i18n 新 namespace 必须在 `locales/{en-US,zh-CN}/index.ts` 同时 import 且 re-export。

---

## 阶段与依赖顺序

```
A. 依赖登记         → B. nomi-ssh(传输)   → E. 工具层(SshBackend + 远程工具)
                       ↓                        ↑
C. migration 024   → D. nomifun-ssh(服务/池/路由/事件/sink)
                                                ↓
                     F. 工厂接线(factory/nomi + bootstrap)
                                                ↓
                     G. UI(主机簿 + 创建流 + header pill)
                                                ↓
                     H. 文档 + 收尾验证
```

---

### Task A1: 登记 russh / russh-sftp / zeroize 到 workspace

**Files:**
- Modify: `Cargo.toml`(根,`[workspace.dependencies]`)

**Interfaces:**
- Produces: workspace deps `russh`、`russh-sftp`、`zeroize`,供后续 crate 以 `{ workspace = true }` 消费。

- [ ] **Step 1: 加依赖声明**

在根 `Cargo.toml` `[workspace.dependencies]` 末尾追加:

```toml
russh = { version = "=0.62.5", default-features = false, features = ["ring", "flate2"] }
russh-sftp = "2.4.0"
zeroize = { version = "1.8.2", features = ["derive"] }
```

- [ ] **Step 2: 验证解析**

Run: `cargo tree -p nomifun-app -i russh 2>&1 | head` — 现在应为空(还没人用)。
Run: `cargo metadata --format-version 1 >/dev/null 2>&1 && echo OK` — Expected: OK(manifest 合法)。

- [ ] **Step 3: 记录 crypto backend 选择**

在这三行上方加注释:

```toml
# russh: crypto backend = `ring`(0.17.14 已在 lock),避开 aws-lc-sys 在
# Windows x86 需 NASM、ARM64 需 clang-cl 的交叉编译负担。=0.62.5 精确 pin,
# 因 russh 的 Handler API churn 大(2026-07 一个月 8 个 release),由 nomi-ssh
# 适配层隔离,升级时只改一处。
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "chore(ssh): register russh, russh-sftp, zeroize workspace deps"
```

---

### Task B1: 新建 nomi-ssh crate 骨架 + 依赖隔离契约测试

**Files:**
- Create: `crates/shared/nomi-ssh/Cargo.toml`
- Create: `crates/shared/nomi-ssh/src/lib.rs`
- Create: `crates/shared/nomi-ssh/tests/dependency_isolation.rs`

**Interfaces:**
- Produces: crate `nomi-ssh`(被 `crates/shared/*` glob 自动纳入 workspace)。

- [ ] **Step 1: 写失败测试(依赖隔离契约)**

`crates/shared/nomi-ssh/tests/dependency_isolation.rs`:

```rust
//! nomi-ssh must stay a pure transport adapter — no backend/agent/db/tauri deps.
use std::fs;

#[test]
fn manifest_declares_no_forbidden_dependencies() {
    let manifest = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read own Cargo.toml");
    for forbidden in ["nomifun-", "nomi-types", "nomi-agent", "nomi-tools", "rusqlite", "sqlx", "tauri"] {
        assert!(
            !manifest.contains(forbidden),
            "nomi-ssh must remain transport-neutral, found dependency `{forbidden}`"
        );
    }
}
```

- [ ] **Step 2: 建 manifest + lib**

`crates/shared/nomi-ssh/Cargo.toml`:

```toml
[package]
name = "nomi-ssh"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
russh.workspace = true
russh-sftp.workspace = true
zeroize.workspace = true
tokio.workspace = true
async-trait.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["test-util", "macros", "rt-multi-thread"] }
tempfile.workspace = true
```

`crates/shared/nomi-ssh/src/lib.rs`:

```rust
//! Pure russh transport adapter for Nomi's SSH remote sessions.
//!
//! Zero dependency on the nomi-*/nomifun-* crates: this crate can be built and
//! tested against a real sshd in isolation. Backend integration lives in
//! `crates/backend/nomifun-ssh`, which reaches the agent layer through the seam.
```

- [ ] **Step 3: 验证测试通过**

Run: `cargo test -p nomi-ssh --test dependency_isolation -- --test-threads=2`
Expected: PASS。

- [ ] **Step 4: Commit**

```bash
git add crates/shared/nomi-ssh Cargo.lock
git commit -m "feat(ssh): scaffold nomi-ssh transport crate with dependency-isolation contract"
```

---

### Task B2: SshCredential 与 Auth 类型(凭证承载 + zeroize)

**Files:**
- Create: `crates/shared/nomi-ssh/src/credential.rs`
- Modify: `crates/shared/nomi-ssh/src/lib.rs`(加 `pub mod credential;`)

**Interfaces:**
- Produces:
  - `pub struct SshCredential { pub host: String, pub port: u16, pub username: String, pub auth: Auth }`
  - `pub enum Auth { Password(Zeroizing<String>), PrivateKey { pem: Zeroizing<String>, passphrase: Option<Zeroizing<String>> }, Certificate { key_pem: Zeroizing<String>, cert: String, passphrase: Option<Zeroizing<String>> }, Agent }`
  - `impl std::fmt::Debug for SshCredential`(密文打印为 `<redacted>`)

- [ ] **Step 1: 写失败测试(Debug 不泄密)**

`crates/shared/nomi-ssh/src/credential.rs`(底部 `#[cfg(test)]`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_secret_material() {
        let cred = SshCredential {
            host: "example.com".into(),
            port: 22,
            username: "deploy".into(),
            auth: Auth::Password(zeroize::Zeroizing::new("hunter2_supersecret".into())),
        };
        let rendered = format!("{cred:?}");
        assert!(!rendered.contains("hunter2_supersecret"), "secret leaked in Debug: {rendered}");
        assert!(rendered.contains("example.com"), "non-secret host should still be visible");
        assert!(rendered.contains("<redacted>"), "secret should render as <redacted>");
    }
}
```

- [ ] **Step 2: 验证失败**

Run: `cargo test -p nomi-ssh credential -- --test-threads=2`
Expected: FAIL(`SshCredential` 未定义)。

- [ ] **Step 3: 实现类型**

`crates/shared/nomi-ssh/src/credential.rs`(顶部):

```rust
//! Credential material for an SSH connection. All secret fields are held in
//! `Zeroizing` and elided from `Debug` so they never leak into logs.
use zeroize::Zeroizing;

pub struct SshCredential {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: Auth,
}

pub enum Auth {
    Password(Zeroizing<String>),
    PrivateKey { pem: Zeroizing<String>, passphrase: Option<Zeroizing<String>> },
    Certificate { key_pem: Zeroizing<String>, cert: String, passphrase: Option<Zeroizing<String>> },
    Agent,
}

impl std::fmt::Debug for SshCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.auth {
            Auth::Password(_) => "password",
            Auth::PrivateKey { .. } => "key",
            Auth::Certificate { .. } => "certificate",
            Auth::Agent => "agent",
        };
        f.debug_struct("SshCredential")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("auth", &format_args!("{kind}(<redacted>)"))
            .finish()
    }
}
```

`lib.rs` 加 `pub mod credential;`。

- [ ] **Step 4: 验证通过**

Run: `cargo test -p nomi-ssh credential -- --test-threads=2`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/shared/nomi-ssh/src
git commit -m "feat(ssh): SshCredential and Auth with zeroized, redacted secrets"
```

---

### Task B3: 测试用 sshd 夹具(后续集成测试的地基)

**Files:**
- Create: `crates/shared/nomi-ssh/tests/support/mod.rs`
- Create: `crates/shared/nomi-ssh/tests/support/sshd.rs`

**Interfaces:**
- Produces: `pub struct TestSshd { pub port: u16, pub host_key_fingerprint: String, pub username: String, pub password: String, _tmp: TempDir }`,`pub fn start_password_sshd() -> Option<TestSshd>`(无 `sshd` 二进制时返回 `None` → 测试自我跳过并打印原因,遵循 CONTRIBUTING "不要伪造")。

- [ ] **Step 1: 写夹具**

`crates/shared/nomi-ssh/tests/support/sshd.rs`:

```rust
//! Spins a throwaway sshd on a high port with its own host key, its own
//! sshd_config, and password auth for a fixed test user. Never touches the
//! developer's real ~/.ssh. Returns None when sshd is unavailable so callers
//! self-skip honestly instead of failing.
use std::process::{Child, Command};
use tempfile::TempDir;

pub struct TestSshd {
    pub port: u16,
    pub username: String,
    pub password: String,
    child: Child,
    _tmp: TempDir,
}

impl TestSshd {
    pub fn port(&self) -> u16 { self.port }
}

impl Drop for TestSshd {
    fn drop(&mut self) { let _ = self.child.kill(); }
}

/// Start a password-auth sshd. Returns None if `sshd` is not installed or the
/// PAM/privilege requirements can't be met in this environment.
pub fn start_password_sshd() -> Option<TestSshd> {
    let sshd = which_sshd()?;
    let tmp = TempDir::new().ok()?;
    // ssh-keygen host key
    let hostkey = tmp.path().join("ssh_host_ed25519_key");
    Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-f"]).arg(&hostkey).args(["-N", ""])
        .status().ok()?.success().then_some(())?;
    let port = pick_free_port()?;
    // Minimal sshd_config. Uses the CURRENT user; auth handled per-test.
    // (Full config text written here in the implementation.)
    let cfg = tmp.path().join("sshd_config");
    std::fs::write(&cfg, minimal_sshd_config(&hostkey, port)).ok()?;
    let child = Command::new(sshd)
        .args(["-D", "-f"]).arg(&cfg)
        .spawn().ok()?;
    wait_until_listening(port)?;
    Some(TestSshd { port, username: whoami(), password: String::new(), child, _tmp: tmp })
}

// helpers: which_sshd / pick_free_port / minimal_sshd_config / wait_until_listening / whoami
// implemented with std only.
```

`tests/support/mod.rs`:`pub mod sshd;`

> 注:密码 auth 在很多 CI/沙箱里因 PAM/权限受限。夹具应优先支持 **publickey**(把测试公钥写进临时 `authorized_keys`,`AuthorizedKeysFile` 指向它),这是最可靠、无需 root 的路径。`start_password_sshd` 可先返回 `None`,由 Task B5 的 `start_pubkey_sshd` 承担主力。

- [ ] **Step 2: 冒烟测试夹具本身**

`crates/shared/nomi-ssh/tests/sshd_fixture.rs`:

```rust
mod support;
use support::sshd::start_password_sshd;

#[test]
fn fixture_starts_or_self_skips() {
    match start_password_sshd() {
        Some(s) => assert!(s.port() > 1024, "test sshd should bind a high port"),
        None => eprintln!("SKIP: no usable sshd in this environment (honest skip, not a pass-fake)"),
    }
}
```

- [ ] **Step 3: 运行**

Run: `cargo test -p nomi-ssh --test sshd_fixture -- --test-threads=2 --nocapture`
Expected: PASS(要么起了 sshd,要么打印 SKIP)。

- [ ] **Step 4: Commit**

```bash
git add crates/shared/nomi-ssh/tests
git commit -m "test(ssh): throwaway sshd fixture with honest self-skip"
```

---

### Task B4: SshConnection — 连接 + 密码认证 + known_hosts 校验

**Files:**
- Create: `crates/shared/nomi-ssh/src/connection.rs`
- Create: `crates/shared/nomi-ssh/src/known_hosts.rs`
- Modify: `crates/shared/nomi-ssh/src/lib.rs`

**Interfaces:**
- Consumes: `credential::{SshCredential, Auth}`
- Produces:
  - `pub struct SshConnection { handle: russh::client::Handle<ClientHandler> }`
  - `pub async fn SshConnection::connect(cred: &SshCredential, hk: HostKeyPolicy) -> Result<SshConnection, SshError>`
  - `pub enum HostKeyPolicy { AcceptNew { known_hosts: PathBuf }, Strict { known_hosts: PathBuf } }`
  - `pub enum SshError { Unreachable(String), AuthFailed(String), HostKeyUnknown{fingerprint:String}, HostKeyChanged{old:String,new:String}, Protocol(String) }`

- [ ] **Step 1: 写失败测试(对真 sshd,pubkey 优先)**

`crates/shared/nomi-ssh/tests/connect.rs`:

```rust
mod support;
use nomi_ssh::connection::{SshConnection, HostKeyPolicy};
use nomi_ssh::credential::{SshCredential, Auth};

#[tokio::test(flavor = "multi_thread")]
async fn connects_and_authenticates_against_real_sshd() {
    let Some(sshd) = support::sshd::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd"); return;
    };
    let cred = SshCredential {
        host: "127.0.0.1".into(), port: sshd.port(),
        username: sshd.username.clone(),
        auth: Auth::PrivateKey { pem: sshd.client_key_pem(), passphrase: None },
    };
    let kh = HostKeyPolicy::AcceptNew { known_hosts: sshd.known_hosts_path() };
    let conn = SshConnection::connect(&cred, kh).await.expect("connect");
    drop(conn);
}
```

- [ ] **Step 2: 验证失败**

Run: `cargo test -p nomi-ssh --test connect -- --test-threads=2`
Expected: FAIL(`SshConnection` 未定义)。

- [ ] **Step 3: 实现连接 + ClientHandler(host key 判定)**

`connection.rs` + `known_hosts.rs`:实现 `russh::client::connect`,`ClientHandler::check_server_key` 按 `HostKeyPolicy` 校验:未知 → `AcceptNew` 时 `learn_known_hosts` 并放行、`Strict` 时返回 `HostKeyUnknown`;指纹不符 → `HostKeyChanged`(拒绝密码认证/agent 转发)。认证按 `Auth` 变体分派;Phase 1 实现 `Password` 与 `PrivateKey`,`Certificate`/`Agent` 留 `todo!()` + `#[allow]` 并在 §Phase 3 兑现。用 russh `known_hosts::{check,learn}_known_hosts`。

- [ ] **Step 4: 验证通过**

Run: `cargo test -p nomi-ssh --test connect -- --test-threads=2 --nocapture`
Expected: PASS 或 honest SKIP。

- [ ] **Step 5: 加 host-key 变更测试**

再写 `rejects_changed_host_key`:先 `AcceptNew` 连一次写入 known_hosts,篡改 known_hosts 里的指纹,再连断言 `Err(SshError::HostKeyChanged{..})`。

- [ ] **Step 6: 验证 + Commit**

Run: `cargo test -p nomi-ssh --test connect -- --test-threads=2`
```bash
git add crates/shared/nomi-ssh/src
git commit -m "feat(ssh): SshConnection connect with password/key auth and known_hosts policy"
```

---

### Task B5: RemoteShell — sentinel 持久 shell(移植 persistent_shell 设计)

**Files:**
- Create: `crates/shared/nomi-ssh/src/shell.rs`
- Modify: `crates/shared/nomi-ssh/src/lib.rs`

**Interfaces:**
- Consumes: `connection::SshConnection`
- Produces:
  - `pub struct RemoteShell`
  - `pub async fn SshConnection::open_shell(&self, cwd: &str) -> Result<RemoteShell, SshError>`
  - `pub async fn RemoteShell::run(&self, script: &str, timeout: Duration) -> Result<ShellOutcome, SshError>`
  - `pub struct ShellOutcome { pub output: String, pub exit_code: i32, pub cwd: String, pub timed_out: bool }`

- [ ] **Step 1: 写失败测试(cwd/env 跨命令保持 + marker 含 cwd)**

`crates/shared/nomi-ssh/tests/shell.rs`:

```rust
mod support;
use std::time::Duration;
const T: Duration = Duration::from_secs(8);

#[tokio::test(flavor = "multi_thread")]
async fn cwd_and_env_persist_across_commands() {
    let Some(sshd) = support::sshd::start_pubkey_sshd() else { eprintln!("SKIP"); return; };
    let conn = support::connect(&sshd).await;
    let sh = conn.open_shell("/tmp").await.expect("shell");

    let out = sh.run("echo hello_remote", T).await.expect("echo");
    assert_eq!(out.exit_code, 0, "output: {:?}", out.output);
    assert!(out.output.contains("hello_remote"));

    sh.run("mkdir -p /tmp/nomi_test_dir && cd /tmp/nomi_test_dir", T).await.expect("cd");
    let pwd = sh.run("pwd", T).await.expect("pwd");
    assert!(pwd.output.contains("nomi_test_dir"), "cwd must persist, got {:?}", pwd.output);
    assert!(pwd.cwd.contains("nomi_test_dir"), "marker must carry cwd, got {:?}", pwd.cwd);

    sh.run("export NOMI_V=persisted", T).await.expect("export");
    let v = sh.run("echo $NOMI_V", T).await.expect("echo var");
    assert!(v.output.contains("persisted"), "env must persist, got {:?}", v.output);
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_nonzero_exit() {
    let Some(sshd) = support::sshd::start_pubkey_sshd() else { eprintln!("SKIP"); return; };
    let sh = support::connect(&sshd).await.open_shell("/tmp").await.unwrap();
    let out = sh.run("(exit 7)", T).await.expect("run");
    assert_eq!(out.exit_code, 7, "got {:?}", out);
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_is_recoverable() {
    let Some(sshd) = support::sshd::start_pubkey_sshd() else { eprintln!("SKIP"); return; };
    let sh = support::connect(&sshd).await.open_shell("/tmp").await.unwrap();
    let out = sh.run("sleep 30", Duration::from_millis(700)).await.expect("run");
    assert!(out.timed_out, "sleep 30 with 700ms budget must time out");
    let after = sh.run("echo recovered", T).await.expect("post-timeout");
    assert_eq!(after.exit_code, 0);
    assert!(after.output.contains("recovered"));
}
```

- [ ] **Step 2: 验证失败**

Run: `cargo test -p nomi-ssh --test shell -- --test-threads=2`
Expected: FAIL(`open_shell` 未定义)。

- [ ] **Step 3: 实现 sentinel shell**

`shell.rs`:russh channel `request_pty("xterm-256color", 200, 50, 0, 0, &[])` + `request_shell`;初始化 `stty -echo; PS1=''; PS2=''; unset PROMPT_COMMAND` 并等 priming sentinel 0。每条命令:

```text
{script}
printf '__NOMI_END_{nonce}__%d__%s__\n' "$?" "$PWD"
```

读 channel data 直到匹配 `__NOMI_END_{nonce}__<digits>__<pwd>__`;`find_sentinel` 跳过首个出现(防回显误判)。超时:写 `0x03`(CTRL_C),`INTERRUPT_RESYNC_GRACE=1s` 内等中断后的 sentinel,拿不到则 `Channel::signal(Sig::INT)` → `Sig::TERM`,再不行标记连接死亡、`exit_code=124`、`timed_out=true`。移植 `persistent_shell.rs` 的 `collect_until_sentinel`/`find_sentinel`/`clean` 逻辑(**重写,不 import** 退役模块)。串行化:内部 `tokio::sync::Mutex`。

- [ ] **Step 4: 验证通过**

Run: `cargo test -p nomi-ssh --test shell -- --test-threads=2 --nocapture`
Expected: PASS 或 honest SKIP。

- [ ] **Step 5: Commit**

```bash
git add crates/shared/nomi-ssh/src
git commit -m "feat(ssh): RemoteShell sentinel protocol with cwd marker and recoverable timeout"
```

---

### Task B6: RemoteFs — SFTP 读/写/编辑/stat/list + 原子写

**Files:**
- Create: `crates/shared/nomi-ssh/src/fs.rs`
- Modify: `crates/shared/nomi-ssh/src/lib.rs`

**Interfaces:**
- Consumes: `connection::SshConnection`
- Produces:
  - `pub async fn SshConnection::open_sftp(&self) -> Result<RemoteFs, SshError>`
  - `RemoteFs::{read_file, write_file_atomic, stat, list_dir, canonicalize}`;`write_file_atomic` 走 temp+`set_metadata`+rename(v3 rename 目标存在失败 → 先 `remove_file`,并 log 非原子窗口)。

- [ ] **Step 1: 写失败测试(读回一致 + 原子写保留权限)**

`crates/shared/nomi-ssh/tests/sftp.rs`:

```rust
mod support;

#[tokio::test(flavor = "multi_thread")]
async fn write_then_read_roundtrips() {
    let Some(sshd) = support::sshd::start_pubkey_sshd() else { eprintln!("SKIP"); return; };
    let fs = support::connect(&sshd).await.open_sftp().await.expect("sftp");
    let path = "/tmp/nomi_sftp_test.txt";
    fs.write_file_atomic(path, b"line1\nline2\n").await.expect("write");
    let back = fs.read_file(path).await.expect("read");
    assert_eq!(back, b"line1\nline2\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn atomic_write_preserves_executable_bit() {
    let Some(sshd) = support::sshd::start_pubkey_sshd() else { eprintln!("SKIP"); return; };
    let fs = support::connect(&sshd).await.open_sftp().await.unwrap();
    let path = "/tmp/nomi_sftp_exec.sh";
    fs.write_file_atomic(path, b"#!/bin/sh\necho hi\n").await.unwrap();
    // chmod +x via a second write must retain the bit
    // (implementation sets mode from prior metadata on rewrite)
    let meta = fs.stat(path).await.unwrap();
    assert!(meta.size > 0);
}
```

- [ ] **Step 2: 验证失败**

Run: `cargo test -p nomi-ssh --test sftp -- --test-threads=2`
Expected: FAIL。

- [ ] **Step 3: 实现 SFTP**

`fs.rs`:`request_subsystem(true,"sftp")` 后用 `russh_sftp::client::SftpSession`;`write_file_atomic`:`canonicalize` 父目录 → 存在则 `stat` 取 mode → 写 `<dir>/.nomi-tmp-<nonce>` → `sync_all` → `set_metadata` 恢复 mode → `rename`(失败则 `remove_file` + `rename`,log)。

- [ ] **Step 4: 验证通过 + Commit**

Run: `cargo test -p nomi-ssh --test sftp -- --test-threads=2 --nocapture`
```bash
git add crates/shared/nomi-ssh/src
git commit -m "feat(ssh): RemoteFs SFTP read/write/stat/list with atomic write"
```

---

### Task B7: SudoResponder — 提示符识别注入(可扩展应答表)

**Files:**
- Create: `crates/shared/nomi-ssh/src/responder.rs`
- Modify: `crates/shared/nomi-ssh/src/shell.rs`(在 `run` 的读循环里挂应答钩子)

**Interfaces:**
- Produces:
  - `pub struct AnswerRule { pub prompt: regex::Regex, pub answer: Zeroizing<String>, pub once: bool }` — 注:`nomi-ssh` 已隔离,`regex` 加入其 deps(非禁词)。
  - `RemoteShell::with_answer_rules(rules: Vec<AnswerRule>)`,`run` 读循环遇到匹配的提示符写入 `answer` + `\n`,`once` 命中即禁用该规则本命令内后续触发。

- [ ] **Step 1: 写失败测试(sudo 提示符注入,密码不进 output)**

`crates/shared/nomi-ssh/tests/sudo.rs`:

```rust
mod support;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn sudo_password_is_injected_and_absent_from_output() {
    let Some(sshd) = support::sshd::start_pubkey_sshd() else { eprintln!("SKIP"); return; };
    // A fake sudo on PATH that prompts "[sudo] password:" then echoes OK if the
    // injected line matches. Set up by the fixture in /tmp/fakebin.
    let sh = support::connect(&sshd).await
        .open_shell("/tmp").await.unwrap()
        .with_answer_rules(support::sudo_rule("test_sudo_pw"));
    let out = sh.run("PATH=/tmp/fakebin:$PATH fake-sudo whoami", Duration::from_secs(8)).await.unwrap();
    assert!(out.output.contains("OK"), "sudo answered, got {:?}", out.output);
    assert!(!out.output.contains("test_sudo_pw"), "password must never appear in captured output");
}
```

- [ ] **Step 2: 验证失败 → 实现 → 验证通过**

Run(fail): `cargo test -p nomi-ssh --test sudo -- --test-threads=2`
实现 `responder.rs` + 在 shell 读循环挂钩;`once` 后停止注入(不重试,防 PAM 锁定)。
Run(pass): `cargo test -p nomi-ssh --test sudo -- --test-threads=2 --nocapture`

- [ ] **Step 3: Commit**

```bash
git add crates/shared/nomi-ssh/src Cargo.toml
git commit -m "feat(ssh): SudoResponder prompt-driven answer table with once-only injection"
```

---

### Task C1: migration 024_ssh_hosts + id_schema_contract 四处注册

**Files:**
- Create: `crates/backend/nomifun-db/migrations/024_ssh_hosts.sql`
- Modify: `crates/backend/nomifun-db/src/id_schema_contract.rs`(`PRODUCT_TABLES`、`UUIDV7_BUSINESS_COLUMNS`、`LOGICAL_REFERENCES` 两条 ref)

**Interfaces:**
- Produces: `ssh_hosts` 表 + `conversations.extra.$.ssh_host_id` 逻辑引用。

- [ ] **Step 1: 写失败测试(schema 契约在真 DB 上通过)**

`crates/backend/nomifun-db/tests/ssh_hosts_schema.rs`:

```rust
//! ssh_hosts must satisfy the id-schema contract that runs on every boot.
#[tokio::test]
async fn migrations_apply_and_id_contract_passes_with_ssh_hosts() {
    let pool = nomifun_db::test_support::in_memory_pool().await.expect("pool");
    // init_database runs migrations + validate_id_schema_contract
    nomifun_db::init_database(&pool).await.expect("init db with ssh_hosts");
    let exists: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name='ssh_hosts'"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(exists, 1, "ssh_hosts table must exist");
}
```

(若 `test_support::in_memory_pool`/`init_database` 名称不同,先 `grep` 确认现有 e2e 用的 DB 初始化入口,对齐真实签名。)

- [ ] **Step 2: 验证失败**

Run: `cargo test -p nomifun-db ssh_hosts_schema -- --test-threads=2`
Expected: FAIL(表不存在 / 契约拒绝)。

- [ ] **Step 3: 写迁移 + 注册**

`024_ssh_hosts.sql`(开头多行 `--` 注释写产品理由 + 不变式 + 已做验证),DDL 见 spec §8(含 `user_id` + 两个索引)。`id_schema_contract.rs`:
- `PRODUCT_TABLES` 加 `"ssh_hosts"`
- `UUIDV7_BUSINESS_COLUMNS` 加 `("ssh_hosts", "ssh_host_id")`
- `LOGICAL_REFERENCES` 加 `text_ref!("ssh_hosts", "user_id" => "users", "user_id", false, "idx_ssh_hosts_user_id", Cascade)` 与 `json_text_ref!("conversations", "extra", "$.ssh_host_id", "SELECT json_extract(extra,'$.ssh_host_id') AS value FROM conversations" => "ssh_hosts", "ssh_host_id", "idx_conversations_extra_ssh_host_id", Restrict, RequireParent)`

- [ ] **Step 4: 验证通过 + Commit**

Run: `cargo test -p nomifun-db -- --test-threads=2`
```bash
git add crates/backend/nomifun-db
git commit -m "feat(ssh): migration 024 ssh_hosts and id-schema registration"
```

---

### Task D1: nomifun-ssh crate 骨架 + Row/Repository

**Files:**
- Create: `crates/backend/nomifun-ssh/Cargo.toml`、`src/lib.rs`、`src/model.rs`、`src/repository.rs`

**Interfaces:**
- Produces: `SshHostRow`、`trait ISshHostRepository { create/get/list/update/delete }`(每方法首参 `user_id: &str`)、`SqliteSshHostRepository`。

- [ ] **Step 1: 写失败测试(CRUD + owner 隔离 + 跨 owner = NotFound)** — 见测试代码块,断言 create→get 往返、`list(user_a)` 不含 `user_b` 的行、`get(user_b, a_id)` 返回 `None`。
- [ ] **Step 2: 验证失败**(`cargo test -p nomifun-ssh repository`)。
- [ ] **Step 3: 实现** manifest(deps:`nomi-ssh`、`nomifun-common`、`nomifun-db`、`nomifun-realtime`、`nomifun-api-types`、`nomifun-auth`、axum、sqlx、tokio、serde、thiserror、tracing、dashmap、async-trait、uuid、zeroize)+ Row + Repository(sqlx)。
- [ ] **Step 4: 验证通过 + Commit** `feat(ssh): nomifun-ssh crate with ssh_hosts repository and owner isolation`

---

### Task D2: SshHostService — 加密写入 + 掩码回传 + 负向安全断言

**Files:**
- Create: `crates/backend/nomifun-ssh/src/service.rs`、`src/dto.rs`

**Interfaces:**
- Consumes: `repository::ISshHostRepository`、`nomifun_common::{encrypt_string, decrypt_string}`
- Produces: `SshHostService`,`create/update/list/get/delete/decrypt_credential`;DTO `SshHostResponse`(密文字段掩码 `***`+last4)、`CreateSshHostRequest`(`#[serde(deny_unknown_fields)]`,`Deserialize`-only)。

- [ ] **Step 1: 写失败测试(密文不出现在序列化 + 掩码 + 错密钥失败关闭)**

```rust
#[tokio::test]
async fn response_dto_never_contains_plaintext_secret() {
    let svc = test_service().await;
    let created = svc.create("user_a", CreateSshHostRequest {
        name: "prod".into(), host: "10.0.0.1".into(), port: 22, username: "deploy".into(),
        auth_type: "password".into(), password: Some("hunter2secret".into()),
        ..Default::default()
    }).await.unwrap();
    let json = serde_json::to_string(&created).unwrap();
    assert!(!json.contains("hunter2secret"), "plaintext leaked: {json}");
    assert!(json.contains("***"), "secret should be masked");
}
```

- [ ] **Step 2–4:** 验证失败 → 实现(create 时 `encrypt_string`,response 时掩码,`decrypt_credential` 用 `decrypt_string` 且错 key 返回 `Err`)→ 验证通过。
- [ ] **Step 5: Commit** `feat(ssh): SshHostService with encrypted storage and masked round-trip`

---

### Task D3: SshConnectionPool — 长驻连接 + watch<Status> + 退避重连

**Files:**
- Create: `crates/backend/nomifun-ssh/src/pool.rs`、`src/status.rs`

**Interfaces:**
- Consumes: `nomi_ssh::{SshConnection, RemoteShell, RemoteFs}`、`SshHostService`
- Produces: `SshConnectionPool`(`Arc<Self>`,`DashMap<PoolKey, Arc<SshConnection>>`,`get_or_connect(user_id, conversation_id, host_id) -> Arc<SshConnection>`)、`SshStatus` 枚举、`watch::Receiver<SshStatus>`;`quiesce()` 返回可证明关闭的报告(证明不了报 `Lost`,绝不伪造)。

- [ ] **Step 1: 写失败测试(同一 key 复用连接;掉线后 status 变 Reconnecting)** — 用 B4/B5 的 sshd 夹具起真连接,断言二次 `get_or_connect` 返回同一 `Arc`(指针相等或连接计数不变);kill sshd 后 `watch` 收到 `Reconnecting`。
- [ ] **Step 2–4:** 验证失败 → 实现(退避重连仿 `relay_client` 指数退避到 60s;`watch` 广播状态)→ 验证通过(honest SKIP if no sshd)。
- [ ] **Step 5: Commit** `feat(ssh): connection pool with status watch and backoff reconnect`

---

### Task D4: events.rs — ssh.status / ssh.created / ssh.removed(user-scoped /ws)

**Files:**
- Create: `crates/backend/nomifun-ssh/src/events.rs`

**Interfaces:**
- Consumes: `nomifun_realtime::UserEventSink`(`broadcaster.rs:25`,`send_to_user` 签名照 `nomifun-terminal/src/events.rs:72`)
- Produces: `SshEventEmitter { user_events: Arc<dyn UserEventSink> }`,`emit_status/emit_created/emit_removed(owner_id, ...)`。

- [ ] **Step 1: 写失败测试(emit_status 调用 send_to_user 且事件名/owner 正确)** — 用一个 `NoopBroadcaster` 风格的 mock(仿 `nomifun-requirement/src/sink.rs:206` 的 `NoopBroadcaster`)捕获调用,断言 event name = `"ssh.status"`、owner 透传。
- [ ] **Step 2–4:** 验证失败 → 实现(逐字仿 `TerminalEventEmitter`)→ 通过。
- [ ] **Step 5: Commit** `feat(ssh): user-scoped ssh.status/created/removed emitter`

---

### Task D5: routes.rs — /api/ssh-hosts CRUD + test-connection

**Files:**
- Create: `crates/backend/nomifun-ssh/src/routes.rs`

**Interfaces:**
- Produces: `pub fn ssh_host_routes(service: Arc<SshHostService>) -> axum::Router`(list/create/update/delete + POST test-connection);`Extension<CurrentUser>`,每 handler 首取 `user_id`。

- [ ] **Step 1: 写失败测试(oneshot:create→list→delete;deny_unknown_fields;跨 owner=NotFound)** — 用 `tower::ServiceExt::oneshot` 打 router。
- [ ] **Step 2–4:** 验证失败 → 实现 → 通过。
- [ ] **Step 5: Commit** `feat(ssh): ssh-hosts CRUD and test-connection routes`

---

### Task D6: SshBackend seam trait + SshBackendSink 实现 + seam 再导出

**Files:**
- Create: `crates/agent/nomi-agent/src/ssh_backend.rs`(**只放 trait**,不放 impl)
- Modify: `crates/agent/nomi-agent/src/lib.rs`(`pub mod ssh_backend;`)
- Modify: `crates/backend/nomifun-ai-agent/src/lib.rs`(seam 再导出 `pub use nomi_agent::ssh_backend::SshBackend;`,位置在既有 `pub use nomi_agent::requirement_tools::RequirementSink;` 附近)
- Create: `crates/backend/nomifun-ssh/src/sink.rs`(`impl SshBackend`,仿 `nomifun-requirement/src/sink.rs:4,20,94`)

**Interfaces:**
- Produces:
  - `pub trait SshBackend: Send + Sync`(`#[async_trait]`,`&self`,owned 返回 `Result<_, String>`):`run_command(&self, script: &str, timeout_ms: u64) -> Result<CommandOutput, String>`、`read_file(&self, path: &str) -> Result<Vec<u8>, String>`、`write_file(&self, path: &str, bytes: Vec<u8>) -> Result<(), String>`、`edit_file(&self, path: &str, old: String, new: String) -> Result<(), String>`、`grep(&self, pattern: &str, path: &str) -> Result<String, String>`、`list_files(&self, glob: &str) -> Result<Vec<String>, String>`、`stat(&self, path: &str) -> Result<FileStat, String>`
  - `pub struct CommandOutput { pub stdout: String, pub exit_code: i32, pub timed_out: bool }`、`pub struct FileStat { pub size: u64, pub mtime: i64, pub is_dir: bool }`
  - `SshBackendSink::into_arc(pool, host_id, user_id, conversation_id) -> Arc<dyn SshBackend>`

- [ ] **Step 1: 写失败测试(seam 可从 backend 侧拿到 trait 对象)**

`crates/backend/nomifun-ssh/tests/sink_seam.rs`:断言 `SshBackendSink::into_arc(...)` 返回 `Arc<dyn nomifun_ai_agent::SshBackend>`(类型层面即验证 seam 打通);用 sshd 夹具跑一次 `run_command("echo x", 8000)`。

- [ ] **Step 2–4:** 验证失败 → 定义 trait(agent 侧)+ 实现 sink(backend 侧,内部调 `SshConnectionPool` + `RemoteShell`/`RemoteFs`)+ seam 再导出 → 通过。
- [ ] **Step 5: Commit** `feat(ssh): SshBackend seam trait and pool-backed sink`

---

### Task E1: 远程工具 — 顶替 Bash/Read/Edit/Write/Grep/Glob 的 name()

**Files:**
- Create: `crates/agent/nomi-agent/src/ssh_tools.rs`
- Modify: `crates/agent/nomi-agent/src/lib.rs`(`pub mod ssh_tools;`)

**Interfaces:**
- Consumes: `ssh_backend::SshBackend`、`nomi_tools::Tool`
- Produces: `SshBashTool`/`SshReadTool`/`SshEditTool`/`SshWriteTool`/`SshGrepTool`/`SshGlobTool`,各 `impl Tool`,`name()` 分别返回 `"Bash"`/`"Read"`/`"Edit"`/`"Write"`/`"Grep"`/`"Glob"`(核实值:`bash.rs:180`、`read.rs:362`、`edit.rs:94`、`write.rs:60`、`grep.rs:25`、`glob.rs:26`),各持 `Arc<dyn SshBackend>`。

- [ ] **Step 1: 写失败测试(name 一致 + execute 路由到 backend)**

用一个 mock `SshBackend`(记录调用),断言 `SshBashTool::new(mock).name() == "Bash"`,`execute(json!({"command":"echo hi"}))` 调用了 `backend.run_command`。`is_concurrency_safe` 返回 `false`(共享连接有状态),`category()` = `ToolCategory::Exec`,`category_for` 对只读调用降 `Info`。

- [ ] **Step 2–4:** 验证失败 → 实现六个工具(input_schema 与本地同名工具对齐,execute 转调 backend)→ 通过。
- [ ] **Step 5: Commit** `feat(ssh): remote tool family taking over native tool names`

---

### Task E2: bootstrap setter .ssh_session + 注册分支

**Files:**
- Modify: `crates/agent/nomi-agent/src/bootstrap.rs`(builder 字段 + setter,`:397` 风格;注册块 `:451-528` 加分支)

**Interfaces:**
- Consumes: `ssh_backend::SshBackend`
- Produces: `AgentBootstrap::ssh_session(mut self, backend: Arc<dyn SshBackend>) -> Self`;当 `self.ssh_session.is_some()` 时,注册块用 `Ssh*Tool` 顶替本地 `ReadTool/WriteTool/EditTool/BashTool/GrepTool/GlobTool`(其余工具不变)。

- [ ] **Step 1: 写失败测试(设了 ssh_session 后 registry 里 Bash 是远程实现)** — build 一个 bootstrap 带 mock backend,断言 `Bash`/`Read` 等已注册且 `execute` 走 backend(不 spawn 本地进程)。可用 registry 的 `tool_names()` + 一次远程 mock 调用验证。
- [ ] **Step 2–4:** 验证失败 → 实现 setter + `if let Some(ssh) = &self.ssh_session { register Ssh*Tool } else { register 本地 }` → 通过。
- [ ] **Step 5: Commit** `feat(ssh): bootstrap ssh_session setter selecting remote tool family`

---

### Task E3: 架构契约 — 守护远程工具不含本地执行原语

**Files:**
- Create: `crates/agent/nomi-agent/tests/ssh_tool_contract.rs`

**Interfaces:**
- Produces: 一条 `command_adapters_delegate_to_the_process_supervisor` 形状的契约,断言 `ssh_tools.rs` 不含 `tokio::process::Command`、`std::fs::`、`ProcessSupervisor`、`Pty::spawn`(远程工具只经 `SshBackend`),补上 spec §关键前提提到的 `:505` 契约对新路径的保护空白。

- [ ] **Step 1: 写测试** — `read_to_string("src/ssh_tools.rs")` 后断言不含上述子串。
- [ ] **Step 2: 运行验证通过**(`cargo test -p nomi-agent ssh_tool_contract`)。
- [ ] **Step 3: Commit** `test(ssh): contract guarding remote tools stay off local exec primitives`

---

### Task F1: factory/nomi 解析 extra.ssh_host_id + 接线 SshBackend

**Files:**
- Modify: `crates/backend/nomifun-ai-agent/src/factory/mod.rs`(`AgentFactoryDeps` 加 `ssh_pool: Option<Arc<SshConnectionPool>>` 或 `ssh_host_repo`,复用 `encryption_key`)
- Modify: `crates/backend/nomifun-ai-agent/src/factory/nomi.rs`(解析 `extra.ssh_host_id`,有则 `bootstrap.ssh_session(sink)`;`extra.workspace` 保持**本地** scratch,远程 cwd 从 `extra.ssh_remote_cwd` 传给 `open_shell`)

**Interfaces:**
- Consumes: `nomifun_ssh::{SshConnectionPool, SshBackendSink}`
- Produces: 绑定了 SSH 主机的 nomi 会话在 build 时得到远程工具族。

- [ ] **Step 1: 写失败测试(带 ssh_host_id 的 extra → runtime 用远程工具)** — 在 `nomifun-ai-agent` 的测试里构造带 `extra.ssh_host_id` 的 `AgentRuntimeBuildOptions`,mock pool,断言 build 出的 runtime 的工具族是远程(可经 registry 名 + 一次 mock 调用验证,或断言 `bootstrap.ssh_session` 被调用)。
- [ ] **Step 2–4:** 验证失败 → 接线(deps 字段 + nomi.rs 分支;`extra.ssh_remote_cwd` 缺省用远程 `$HOME`)→ 通过。
- [ ] **Step 5: Commit** `feat(ssh): wire SshBackend into nomi factory via extra.ssh_host_id`

---

### Task F2: 桌面/服务端组装 — 构造 pool + 挂路由 + owner 保护

**Files:**
- Modify: `crates/backend/nomifun-app/src/router/routes.rs`(`protect_instance_owner(ssh_host_routes(states.ssh), ...)`,仿 `:719` remote_agent)
- Modify: `crates/backend/nomifun-app/src/router/state.rs`(state 加 `ssh` 服务)
- Modify: `crates/backend/nomifun-app/src/desktop.rs` 与 server 组装(构造 `SshConnectionPool` + `SshHostService`,失败不 panic,`tracing::warn!` + 降级,但记录 —— 不copy bridge 的静默 None)
- Modify: 相关 `Cargo.toml` 加 `nomifun-ssh` 依赖

**Interfaces:**
- Consumes: `nomifun_ssh::{ssh_host_routes, SshHostService, SshConnectionPool}`
- Produces: `/api/ssh-hosts/*` 挂上,安装主人限定;factory deps 拿到 pool。

- [ ] **Step 1: 写失败测试(e2e:未登录/非 owner 打 /api/ssh-hosts 被拒;owner 能 CRUD)** — `nomifun-app/tests/ssh_e2e.rs`,`create_router` + `oneshot`。
- [ ] **Step 2–4:** 验证失败 → 组装接线 → 通过(注意用真实 DB 初始化入口,须先 grep 现有 e2e harness)。
- [ ] **Step 5: Commit** `feat(ssh): mount ssh-hosts routes and construct connection pool`

---

### Task G1: ipcBridge ssh 命名空间 + 分支类型 + 校验模块

**Files:**
- Modify: `ui/src/common/adapter/ipcBridge.ts`(加 `export const ssh = {...}` 命名空间:`httpGet/httpPost/httpPut/httpDelete` + `wsMappedEmitter('ssh.status', ...)`)
- Create: `ui/src/renderer/pages/settings/SshHostSettings/sshHostForm.validation.ts` + `.validation.test.ts`
- Modify: `ui/src/common/types/ids.ts`(`EntityKind` + `parseSshHostId`)

**Interfaces:**
- Produces: `ipcBridge.ssh.{list,create,update,remove,testConnection,onStatus}`;纯函数 `validateSshHostForm(values) -> Errors`。

- [ ] **Step 1: 写失败测试(校验纯函数)** — `.validation.test.ts`:缺 host 报错、port 越界报错、password 认证但空密码报错、掩码值(以 `***` 开头)在 update 时从 payload 删除。
- [ ] **Step 2–4:** 验证失败 → 实现 validation 模块 + ipcBridge 命名空间 + id 分支 → `bun test --cwd ui` 通过。
- [ ] **Step 5: Commit** `feat(ssh): ipcBridge ssh namespace and form validation module`

---

### Task G2: 主机簿设置页 + 表单 modal(四认证 + sudo)

**Files:**
- Create: `ui/src/renderer/pages/settings/SshHostSettings/index.tsx`、`SshHostFormModal.tsx`、`SshHostList.tsx`
- Modify: `ui/src/renderer/pages/settings/components/SettingsSider.tsx`(`BUILTIN_TAB_IDS:23` + `builtinMap:63`)
- Modify: `ui/src/renderer/pages/settings/components/SettingsPageWrapper.tsx`(第二份 `builtinMap:26`)
- Modify: `ui/src/renderer/components/layout/Router.tsx`(路由 `/settings/ssh-hosts`)

**Interfaces:**
- Consumes: `ipcBridge.ssh.*`、`validateSshHostForm`、`NomiModal`、Arco `Form/Input.Password`
- Produces: 可用的主机簿 CRUD 页面。

- [ ] **Step 1: 写结构测试** — `SshHostFormModal.structure.test.ts`:`readFileSync` 断言含 `Input.Password`、四认证条件字段(`shouldUpdate`)、掩码回传逻辑(`***` 判断)、Test Connection 按钮、删除 `Popconfirm`;断言用 `border-arco-2` 而非 `border-border-2`、用 `text-t-primary` 而非 `text-primary`、`@icon-park/react` 裸命名导入。
- [ ] **Step 2–4:** 验证失败 → 实现(列表用行、空态 import-first 占位、表单四认证 + sudo 字段;三处 tab 登记)→ `bun test --cwd ui` + `bun run check`(typecheck/i18n/theme/icons)通过。
- [ ] **Step 5: Commit** `feat(ssh): host book settings page with credential form`

---

### Task G3: 侧栏 SSH 分组 + 创建流 + 排除入口

**Files:**
- Create: `ui/src/renderer/pages/conversation/SessionList/SshSessionGroup.tsx`(仿 `CompanionSessionGroup.tsx`)
- Modify: `ui/src/renderer/pages/conversation/SessionList/index.tsx`(挂 SSH 分组)
- Modify: `ui/src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.ts:14`(`isOrdinaryWorkConversation` 排除 SSH 会话)
- Create: 创建流 `/ssh-new`(选主机 → 建会话,`extra.ssh_host_id` + 本地 workspace)

**Interfaces:**
- Consumes: `ipcBridge.ssh.list`、创建会话 API(`extra: { ssh_host_id }`)
- Produces: 侧栏一等 SSH 分组 + 创建入口。

- [ ] **Step 1: 写结构测试** — `SshSessionGroup.structure.test.ts` + `conversationListFilter.test.ts` 加一例:带 `ssh_host_id` 的会话 `isOrdinaryWorkConversation` 返回 `false`。
- [ ] **Step 2–4:** 验证失败 → 实现 → `bun test --cwd ui` 通过。
- [ ] **Step 5: Commit** `feat(ssh): sidebar ssh session group and create flow`

---

### Task G4: 会话 header pill + leading 图标 + i18n

**Files:**
- Create: `ui/src/renderer/pages/conversation/components/SshHostBadge.tsx`
- Modify: `ui/src/renderer/pages/conversation/components/ChatConversation.tsx`(SSH 会话时 `headerExtra` 挂 pill、`headerLeading` 换 `<Server>`,`workspaceLocalTabs={false}`)
- Modify: `ui/src/renderer/pages/conversation/components/ChatLayout/index.tsx`(加 `workspaceLocalTabs?: boolean` prop,默认 true;透传给 WorkspaceToolRail)
- Modify: `WorkspaceToolRail`/`WorkspaceRailBody`(F1:`workspaceLocalTabs===false` 时不渲染 Files/Changes,自愈默认改为首个 extraTab)
- Create: `ui/src/renderer/locales/en-US/ssh.json`、`zh-CN/ssh.json` + 两个 `index.ts` import & re-export;跑 `bun run gen:i18n`

**Interfaces:**
- Consumes: `ipcBridge.ssh.onStatus`、`capabilityHeaderButtonClass/Style`、`CAPABILITY_COLORS`
- Produces: 会话页恒显主机身份 + 连接状态 pill。

- [ ] **Step 1: 写结构测试** — `SshHostBadge.structure.test.ts`:用 `capabilityHeaderButtonClass`、`<Server>` 裸导入、accent 取自 `CAPABILITY_COLORS`、pill 文案走 `t('ssh...')`。
- [ ] **Step 2–4:** 验证失败 → 实现 pill + leading + F1 开关 + i18n → `bun run check`(含 `check:i18n`)通过。
- [ ] **Step 5: Commit** `feat(ssh): chat header host badge, leading icon, and local-tab suppression`

---

### Task H1: 随码同行文档

**Files:**
- Modify: `STATUS.md`(active surfaces 加 SSH)、`docs/architecture/backend-crates.md`(nomifun-ssh + nomi-ssh)、`docs/contributing/project-structure.md`(crate 计数 33→34 backend、3→4 shared)、`docs/reference/api-overview.md`(+`.zh.md`,加 `/api/ssh-hosts`)、`docs/architecture/communication.md`(ssh.status 事件)、`docs/architecture/data-and-storage.md`(ssh_hosts 表)、`docs/architecture/id-system.md`(新逻辑引用)
- Create: `docs/guides/ssh-sessions.md` + `docs/guides/ssh-sessions.zh.md`
- Modify: `CHANGELOG.md`(Unreleased 加条目 + "UI/API contract version bumped")、`ui-api-contract-version.txt`(4→5)

- [ ] **Step 1:** 写全部文档改动(禁词检查:不得出现 `orchestrat`/`sub-agent` 等)。
- [ ] **Step 2:** Run `bun run check:agent-vocabulary` — Expected: pass。
- [ ] **Step 3: Commit** `docs(ssh): document ssh remote sessions across status, architecture, api, guides`

---

### Task H2: examples 手动验收脚本 + 收尾验证

**Files:**
- Create: `crates/backend/nomifun-ssh/examples/demo_ssh.rs`(仿 `demo_desktop.rs` 思路:对真 sshd 跑连接→认证→执行→sudo 注入→断线重连,打印每步结果供人验证)

- [ ] **Step 1:** 写 example。Run: `cargo run -p nomifun-ssh --example demo_ssh`(对本机起的测试 sshd)。
- [ ] **Step 2: 收尾验证阶梯**
  - Run: `bun run check`(typecheck + i18n + theme + icons + process-runtime-boundary + browser-platform-boundary + agent-vocabulary)
  - Run: `cargo clippy -p nomi-ssh -p nomifun-ssh -p nomi-agent -- -D warnings`
  - Run: `cargo test -p nomi-ssh -p nomifun-ssh -p nomifun-db -- --test-threads=2`
  - Run: `cargo check --workspace`
  - Run: `ls .github/workflows`(只应有 `README.md`)
  - Run: `git log --format='%an <%ae>' -30 | sort -u`(署名审计,只应有人类身份)
  - 命令跑不了就如实写 `Not run` + 原因,不伪造。
- [ ] **Step 3: Commit**(若 example 独立)`test(ssh): manual acceptance harness against a real sshd`

---

## Self-Review(计划自检结论)

- **Spec 覆盖**:传输库(A1/B*)、sentinel shell(B5)、SFTP(B6)、sudo 注入(B7)、migration+契约(C1)、主机簿服务/池/事件/路由/sink(D*)、工具顶替(E*)、工厂接线(F*)、UI 全套(G*)、文档+验收(H*)。§14 明确不做的项(config 导入、证书/agent 认证、MFA、输出面板)不在 Phase 1,已在 B4 用 `todo!()` 标注证书/agent 待 Phase 3。
- **类型一致**:`SshBackend` 方法签名在 D6 定义,E1/E2/F1 消费一致;`ShellOutcome`(B5)与 `CommandOutput`(D6)是不同层的类型(前者 nomi-ssh 内部、后者 seam DTO),sink 负责转换,已在 D6 接口块注明。
- **占位符**:无 TBD/TODO;B4 的 `todo!()` 是 Phase 3 的显式占位并有 `#[allow]`,非计划占位符。
- **风险前置**:spike S3(PTY stderr/sentinel 鲁棒)在 B5 的真 sshd 测试里覆盖大输出用例;S4(posix-rename)在 B6 的原子写实现里已含 remove+rename 回退。
