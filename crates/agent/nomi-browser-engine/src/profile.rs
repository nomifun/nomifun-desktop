//! 脏 profile 崩溃标记 scrub：每次 spawn chrome **前**，把专属 user-data-dir 的
//! `<profile>/Default/Preferences` 里 chromium 记录的「上次崩溃/被杀」标记改回干净，
//! 根治非优雅退出（TerminateProcess / Job Object 硬杀 / app crash / 断电）后下次启动弹
//! 「Chrome 未正确关闭 / 恢复页面?」气泡 + 跑会话恢复。
//!
//! ## 为什么这是 keystone（唯一在所有退出路径都生效的层）
//! 本引擎的 chrome 进程**永远是被硬杀**的（`launch.rs` 的 `kill_on_drop` + Windows Job
//! Object `KILL_ON_JOB_CLOSE`），且 app 退出时后端线程被同步 `exit(0)` 硬杀、跑不到任何
//! 异步优雅关闭。优雅 `Browser.close`（写回干净退出）在当前架构下**无可达调用点**，故
//! 真正的根治是：承认「下次必是脏 profile」，在**下次 launch 前**把崩溃标记洗干净。
//!
//! ## 权威来源（Chromium 源码核实，见 docs/superpowers/specs/browser-use）
//! - 键 `profile.exit_type`（C++ 常量 `prefs::kSessionExitType`，`chrome/common/pref_names.h`）。
//!   取值 `"Normal"`（干净）/ `"Crashed"`（崩溃被杀）/ `"SessionEnded"`（系统强退），见
//!   `chrome/browser/sessions/exit_type_service.cc`。
//! - **时序**：chromium 启动时**立即把 `exit_type` 写成 `"Crashed"`**，只有走完整干净关闭
//!   才回写 `"Normal"`——故被硬杀必留 `"Crashed"`。下次启动 `HasPendingUncleanExit()`
//!   （`startup_browser_creator.cc`）见 `exit_type==Crashed` 即武装气泡。把它改回 `"Normal"`
//!   → 气泡闸门不触发 + 不跑会话恢复。
//! - **无可靠命令行开关**：`--disable-session-crashed-bubble` 已从源码树删除；
//!   `--hide-crash-restore-bubble` 仅 ChromeOS full-restore 生效（桌面 Windows no-op）。
//!   故唯一权威手段是改 `exit_type`（等价 ChromeDriver `PrepareUserDataDir` 种子化）。
//!
//! ## 红线
//! 只动 `profile.exit_type` 这**一个**键，绝不动其它 pref（cookie / localStorage 等登录态
//! 全保留）。文件不存在 = 首启，跳过；JSON 损坏 = best-effort 不致命（warn 后照常启动）。
//! 必须在 chrome **未运行**时改（launch 前，本引擎专属 dir 同一时刻只一个 chrome）。

use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// 默认 profile 子目录。本引擎单 profile 启动（不传 `--profile-directory`），故恒 `Default`。
pub const DEFAULT_PROFILE_SUBDIR: &str = "Default";
/// profile 偏好文件名。
pub const PREFERENCES_FILE: &str = "Preferences";
/// `profile.exit_type` 的干净值（对应 Chromium `ExitType::kClean`）。
const EXIT_TYPE_NORMAL: &str = "Normal";
/// Crash-marker scrubbing is launch hygiene, not a general Preferences parser.
/// Keep one corrupt or hostile profile from allocating an unbounded buffer on
/// the launching task. Normal Chromium Preferences files are far below this
/// ceiling; oversized files are preserved and skipped under the existing
/// best-effort semantics.
const MAX_PREFERENCES_BYTES: u64 = 16 * 1024 * 1024;

/// `<user-data-dir>/Default/Preferences` 的绝对路径。
pub fn preferences_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join(DEFAULT_PROFILE_SUBDIR).join(PREFERENCES_FILE)
}

/// **纯逻辑**：把一段 Preferences JSON 文本的崩溃标记改成「干净退出」。
///
/// - `Ok(Some(new_text))`：`profile.exit_type` 原本非 `"Normal"`，已改写，需回写。
/// - `Ok(None)`：已是 `"Normal"`（免无谓写盘）。
/// - `Err(msg)`：JSON 解析/结构异常（调用方 best-effort：warn 后照常启动）。
///
/// 只插入/改写 `profile.exit_type`，其它键（含 `profile` 下的兄弟键）原样保留。
pub fn scrub_prefs_json(text: &str) -> Result<Option<String>, String> {
    let mut v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("parse Preferences: {e}"))?;
    let obj = v
        .as_object_mut()
        .ok_or_else(|| "Preferences root not a JSON object".to_string())?;
    // `profile` 不存在则建空对象（边角：极简/损坏 prefs）；存在但非对象 = 结构异常 → Err。
    let profile = obj
        .entry("profile")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| "Preferences `profile` is not a JSON object".to_string())?;

    if profile.get("exit_type").and_then(|x| x.as_str()) == Some(EXIT_TYPE_NORMAL) {
        return Ok(None); // 已干净
    }
    profile.insert("exit_type".into(), serde_json::json!(EXIT_TYPE_NORMAL));
    serde_json::to_string(&v)
        .map(Some)
        .map_err(|e| format!("serialize Preferences: {e}"))
}

/// 薄 I/O 包装：读 `<user-data-dir>/Default/Preferences` → [`scrub_prefs_json`] → 原子回写。
///
/// **best-effort 语义**（绝不阻断启动）：
/// - 文件不存在（首启，chrome 尚未建过 profile）→ `Ok(())`，跳过。
/// - JSON 损坏 / 结构异常 → warn + `Ok(())`（照常启动；最坏情况只是弹一次气泡）。
/// - 仅当真有改动时写盘（temp + rename 原子替换，避免 chrome 读到半截——虽然此刻
///   chrome 必未运行，原子写仍是稳妥习惯）。
///
/// 返回 `Err` 仅限**非 NotFound 的读 I/O 错误**（如权限），交调用方 warn。
pub fn scrub_crash_markers(user_data_dir: &Path) -> std::io::Result<()> {
    let path = preferences_path(user_data_dir);
    let (mut source, source_identity) = match open_verified_preferences(&path) {
        Ok(opened) => opened,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()), // 首启，无 profile
        Err(e) => return Err(e),
    };
    let Some(text) = read_preferences_bounded(&mut source)? else {
        tracing::warn!(
            target: "nomi_browser_engine::profile",
            reason = "preferences_size_limit",
            limit_bytes = MAX_PREFERENCES_BYTES,
            "Preferences crash-marker scrub skipped (best-effort; launch continues)"
        );
        return Ok(());
    };
    match scrub_prefs_json(&text) {
        Ok(Some(new_text)) => {
            // 原子回写：写同目录临时文件再 rename（同卷 rename 是原子替换）。
            drop(source);
            let (tmp_path, mut tmp_file) = create_unique_preferences_temp(&path)?;
            let replace_result = (|| -> std::io::Result<()> {
                tmp_file.write_all(new_text.as_bytes())?;
                tmp_file.sync_all()?;
                drop(tmp_file);
                let (current, current_identity) = open_verified_preferences(&path)?;
                if current_identity != source_identity {
                    return Err(std::io::Error::other(
                        "Preferences changed during crash-marker scrub",
                    ));
                }
                replace_preferences_file(&path, &tmp_path)?;
                drop(current);
                Ok(())
            })();
            if replace_result.is_err() {
                let _ = std::fs::remove_file(&tmp_path);
            }
            replace_result
        }
        Ok(None) => Ok(()), // 已干净
        Err(msg) => {
            tracing::warn!(
                target: "nomi_browser_engine::profile",
                reason = "invalid_preferences_shape",
                "Preferences crash-marker scrub skipped (best-effort; launch continues)"
            );
            let _ = msg;
            Ok(())
        }
    }
}

fn read_preferences_bounded(source: &mut std::fs::File) -> std::io::Result<Option<String>> {
    let declared_len = source.metadata()?.len();
    if declared_len > MAX_PREFERENCES_BYTES {
        return Ok(None);
    }

    let mut text = String::with_capacity(declared_len as usize);
    // The metadata check avoids a needless allocation for an already-large
    // file. `take(limit + 1)` also closes the grow-after-metadata race without
    // ever reading the remainder into memory.
    (&mut *source)
        .take(MAX_PREFERENCES_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_PREFERENCES_BYTES {
        return Ok(None);
    }
    Ok(Some(text))
}

/// macOS/Linux：清理 stale `Singleton*` 三件套（symlink → `hostname-pid`，硬杀残留）。
///
/// Windows **不需要**：其单实例锁是 `lockfile`（`FILE_FLAG_DELETE_ON_CLOSE`），进程被杀时
/// 内核自动删除；单实例发现靠命名互斥量 + 隐藏消息窗口（随进程消失），无 stale 文件锁。
///
/// chrome 通常能自愈 stale lock（检查 pid/hostname 后破链重建），仅跨主机共享 profile 等
/// 边角才阻塞——我们用专属本机 dir，删它纯属兜底。
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn clear_stale_singleton(user_data_dir: &Path) {
    // TODO(verify-macos/linux)：本机仅 Windows；mac/linux 上的 stale lock 行为待实机核对，
    // 见 docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md。
    for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
        let _ = std::fs::remove_file(user_data_dir.join(name));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PreferencesFileIdentity {
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

fn invalid_preferences_file(reason: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, reason)
}

fn open_verified_preferences(
    path: &Path,
) -> std::io::Result<(std::fs::File, PreferencesFileIdentity)> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_DELETE, FILE_SHARE_READ,
        };

        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(invalid_preferences_file(
                "Preferences is a reparse point or non-regular file",
            ));
        }
        let (volume_serial_number, file_index, number_of_links) =
            windows_file_identity(&file)?;
        if number_of_links != 1 {
            return Err(invalid_preferences_file(
                "Preferences has multiple hard links",
            ));
        }
        return Ok((
            file,
            PreferencesFileIdentity {
                volume_serial_number,
                file_index,
            },
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(invalid_preferences_file(
                "Preferences is not a regular file",
            ));
        }
        if metadata.nlink() != 1 {
            return Err(invalid_preferences_file(
                "Preferences has multiple hard links",
            ));
        }
        Ok((
            file,
            PreferencesFileIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
            },
        ))
    }
}

fn create_unique_preferences_temp(
    preferences: &Path,
) -> std::io::Result<(PathBuf, std::fs::File)> {
    let parent = preferences.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Preferences has no parent directory",
        )
    })?;
    for _ in 0..16 {
        let mut random = [0_u8; 16];
        getrandom::getrandom(&mut random)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let path = parent.join(format!(
            ".{PREFERENCES_FILE}.nomi-scrub-{}.tmp",
            hex::encode(random)
        ));
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique Preferences scrub temp file",
    ))
}

#[cfg(windows)]
fn replace_preferences_file(
    preferences: &Path,
    replacement: &Path,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        ReplaceFileW, REPLACEFILE_WRITE_THROUGH,
    };

    let preferences = preferences
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement = replacement
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both buffers are live, NUL-terminated UTF-16 paths; optional
    // backup/exclusion pointers are intentionally null.
    let replaced = unsafe {
        ReplaceFileW(
            preferences.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn replace_preferences_file(
    preferences: &Path,
    replacement: &Path,
) -> std::io::Result<()> {
    std::fs::rename(replacement, preferences)
}

/// Ownership marker written into every application-managed Chromium
/// `--user-data-dir`.
///
/// The payload deliberately contains only process ownership data. It never
/// stores a CDP endpoint, websocket URL, cookie, lease, token, or other secret.
pub const OWNERSHIP_MARKER_FILE: &str = ".nomifun-browser-owner.json";
/// Append-only committed half of an ephemeral ownership transition.
///
/// The provisional record deliberately remains at [`OWNERSHIP_MARKER_FILE`].
/// Publishing the committed browser identity at a distinct path means the
/// transition never needs a compare-then-overwrite rename: a competing writer
/// can make the transition fail closed, but can never be overwritten.
pub const COMMITTED_OWNERSHIP_RECORD_FILE: &str =
    ".nomifun-browser-owner.committed.json";
const DEVTOOLS_ACTIVE_PORT_FILE: &str = "DevToolsActivePort";

const OWNERSHIP_MARKER_VERSION: u32 = 1;
/// Ownership records contain only two process identities and fixed lineage
/// fields. Treat a larger file as corrupt authority and fail closed instead of
/// letting profile recovery allocate attacker-controlled bytes.
const MAX_OWNERSHIP_RECORD_BYTES: u64 = 64 * 1024;
/// Defensive bound for one complete no-follow ownership-marker discovery
/// walk. Hitting it is an explicit incomplete scan. Unresolved trees still
/// fail closed; an already-discovered exact ephemeral profile may instead be
/// removed by the separate bounded marker-last continuation and then rescanned.
const MAX_MARKER_SCAN_ENTRIES: usize = 100_000;
/// One exact ephemeral-profile cleanup attempt is intentionally bounded, but
/// reaching this limit is resumable rather than a permanent quarantine.  The
/// root ownership record stays in place, so the caller retains exact cleanup
/// authority and a later attempt continues over the smaller remaining tree.
const MAX_EPHEMERAL_DELETE_BATCH_ENTRIES: usize = MAX_MARKER_SCAN_ENTRIES;
/// Bound the aggregate directory-name/path bytes copied while one deletion
/// batch is in memory. This is independent from the entry count because one
/// hostile wide directory can otherwise make 100k long names expensive.
const MAX_EPHEMERAL_DELETE_BATCH_PATH_BYTES: usize = 16 * 1024 * 1024;
const EPHEMERAL_DELETE_RETRY_REQUIRED: &str =
    "ephemeral profile cleanup reached its bounded batch limit; retry required";
/// Startup recovery keeps one exact claimed profile moving through bounded
/// batches in the same invocation. The attempt/time ceilings prevent a
/// hostile concurrent writer from turning startup into an unbounded loop.
const MAX_RECOVERY_DELETE_CONTINUATION_ATTEMPTS: usize = 64;
const MAX_RECOVERY_DELETE_CONTINUATION_TIME: Duration = Duration::from_secs(30);
/// Full `PathBuf`s duplicate their parent prefix. Bound those retained copies
/// separately from raw directory-name bytes so a deep common prefix multiplied
/// by many profiles cannot exceed the per-scan memory envelope.
const MAX_MARKER_SCAN_RETAINED_PATH_BYTES: usize = 16 * 1024 * 1024;
/// Windows traversal retains a frontier of child paths rather than Unix
/// directory fds. Its *current* frontier has an independent byte ceiling and
/// releases charges as paths are popped.
#[cfg(windows)]
const MAX_MARKER_SCAN_PENDING_PATH_BYTES: usize = 16 * 1024 * 1024;
/// In addition to the entry count, cap the aggregate bytes retained for Unix
/// directory names. A 100k-entry directory otherwise still permits tens of
/// MiB of attacker-controlled names before callers get a chance to enforce
/// their entry budget.
#[cfg(unix)]
const MAX_MARKER_SCAN_NAME_BYTES: usize = 16 * 1024 * 1024;
/// Marker-last verification only needs to observe the two permitted ownership
/// records plus one unexpected entry. Any wider concurrent mutation is an
/// immediate fail-closed result rather than another large inventory.
#[cfg(unix)]
const MAX_POST_CLEANUP_ROOT_ENTRIES: usize = 3;
#[cfg(unix)]
const MAX_POST_CLEANUP_ROOT_NAME_BYTES: usize = 4 * 1024;
/// Unix marker scans hold one directory fd per *ancestor*, so this bound is
/// also the exact ceiling of concurrently open scan fds; it must stay far
/// below the default macOS RLIMIT_NOFILE soft limit of 256. Exceeding it is
/// an explicit incomplete scan and every destructive caller fails closed.
#[cfg(unix)]
const MAX_MARKER_SCAN_DEPTH: usize = 64;
const MAX_EPHEMERAL_DELETE_DEPTH: usize = 256;
#[cfg(not(windows))]
const PROCESS_DISCOVERY_RETRIES: usize = 40;
#[cfg(not(windows))]
const PROCESS_DISCOVERY_INTERVAL: Duration = Duration::from_millis(25);
const PROCESS_TREE_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_TREE_CONFIRM_INTERVAL: Duration = Duration::from_millis(25);
/// Coarse re-probe interval while a tree is still observably present. The
/// absence loop can run on an async caller's thread, so it must sleep rather
/// than busy-poll process inventory.
const PROCESS_TREE_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const PROCESS_TREE_ABSENCE_CONFIRMATIONS: usize = 2;
const PROFILE_OPERATION_LOCK_PREFIX: &str = ".nomifun-browser-operation";

#[derive(Clone, Copy, Debug)]
struct EphemeralDeleteLimits {
    max_entries: usize,
    max_path_bytes: usize,
}

impl EphemeralDeleteLimits {
    fn production() -> Self {
        #[cfg(test)]
        if let Some(limits) = TEST_EPHEMERAL_DELETE_LIMITS.with(std::cell::Cell::get) {
            return limits;
        }
        Self {
            max_entries: MAX_EPHEMERAL_DELETE_BATCH_ENTRIES,
            max_path_bytes: MAX_EPHEMERAL_DELETE_BATCH_PATH_BYTES,
        }
    }
}

fn marker_scan_entry_limit() -> usize {
    #[cfg(test)]
    if let Some(limits) = TEST_EPHEMERAL_DELETE_LIMITS.with(std::cell::Cell::get) {
        return limits.max_entries;
    }
    MAX_MARKER_SCAN_ENTRIES
}

#[cfg(test)]
thread_local! {
    static TEST_EPHEMERAL_DELETE_LIMITS: std::cell::Cell<Option<EphemeralDeleteLimits>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
struct TestEphemeralDeleteLimitsGuard(Option<EphemeralDeleteLimits>);

#[cfg(test)]
impl TestEphemeralDeleteLimitsGuard {
    fn install(limits: EphemeralDeleteLimits) -> Self {
        Self(TEST_EPHEMERAL_DELETE_LIMITS.with(|slot| slot.replace(Some(limits))))
    }
}

#[cfg(test)]
impl Drop for TestEphemeralDeleteLimitsGuard {
    fn drop(&mut self) {
        TEST_EPHEMERAL_DELETE_LIMITS.with(|slot| slot.set(self.0));
    }
}

#[derive(Debug)]
struct EphemeralDeleteBudget {
    limits: EphemeralDeleteLimits,
    entries: usize,
    path_bytes: usize,
}

impl EphemeralDeleteBudget {
    fn new(limits: EphemeralDeleteLimits) -> Self {
        Self {
            limits,
            entries: 0,
            path_bytes: 0,
        }
    }

    /// Reserve one entry before retaining its name/path. `Ok(false)` is a
    /// normal continuation boundary: no accounting is changed and the caller
    /// must keep the root marker for a later exact retry.
    fn try_charge(&mut self, path_bytes: usize) -> Result<bool, String> {
        let next_entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| "ephemeral profile deletion entry accounting overflowed".to_string())?;
        let next_path_bytes = self
            .path_bytes
            .checked_add(path_bytes)
            .ok_or_else(|| "ephemeral profile deletion path accounting overflowed".to_string())?;
        if next_entries > self.limits.max_entries
            || next_path_bytes > self.limits.max_path_bytes
        {
            return Ok(false);
        }
        self.entries = next_entries;
        self.path_bytes = next_path_bytes;
        Ok(true)
    }

    #[cfg(unix)]
    fn try_charge_unix_name(&mut self, name: &[u8]) -> Result<bool, String> {
        self.try_charge(name.len())
    }

    #[cfg(windows)]
    fn try_charge_windows_path(&mut self, path: &Path) -> Result<bool, String> {
        self.try_charge(path_storage_bytes(path))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EphemeralDeleteProgress {
    Complete,
    MoreWork,
}

#[cfg(unix)]
fn path_storage_bytes(path: &Path) -> usize {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().len()
}

#[cfg(windows)]
fn path_storage_bytes(path: &Path) -> usize {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .count()
        .saturating_mul(std::mem::size_of::<u16>())
}

#[derive(Debug)]
struct RetainedPathBudget {
    limit_bytes: usize,
    used_bytes: usize,
    label: &'static str,
}

impl RetainedPathBudget {
    fn new(limit_bytes: usize, label: &'static str) -> Self {
        Self {
            limit_bytes,
            used_bytes: 0,
            label,
        }
    }

    /// Atomically charge every path before any of them is retained. On error
    /// the previous accounting remains valid and the caller can fail closed.
    fn charge(&mut self, paths: &[&Path]) -> Result<(), String> {
        let additional = paths.iter().try_fold(0_usize, |total, path| {
            total.checked_add(path_storage_bytes(path)).ok_or(())
        });
        let next = additional
            .ok()
            .and_then(|additional| self.used_bytes.checked_add(additional))
            .ok_or_else(|| format!("{} byte accounting overflowed", self.label))?;
        if next > self.limit_bytes {
            return Err(format!(
                "{} exceeded {} bytes",
                self.label, self.limit_bytes
            ));
        }
        self.used_bytes = next;
        Ok(())
    }

    #[cfg(any(windows, test))]
    fn release(&mut self, path: &Path) -> Result<(), String> {
        self.used_bytes = self
            .used_bytes
            .checked_sub(path_storage_bytes(path))
            .ok_or_else(|| format!("{} byte accounting underflowed", self.label))?;
        Ok(())
    }
}

static MANAGED_APP_INSTANCE_ID: OnceLock<String> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static OWNERSHIP_COMMIT_DIRECTORY_BOUND_HOOK:
        std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        std::cell::RefCell::new(None);
    static OWNERSHIP_COMMIT_BARRIER_HOOK:
        std::cell::RefCell<Option<Box<dyn FnOnce(&Path, &Path)>>> =
        std::cell::RefCell::new(None);
    // 消费点（marker-last 最终 rmdir 前的屏障）与设置点都只在 Windows 测试路径。
    #[cfg(windows)]
    static FINAL_PROFILE_RMDIR_BARRIER_HOOK:
        std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn run_ownership_commit_directory_bound_hook(profile_dir: &Path) {
    OWNERSHIP_COMMIT_DIRECTORY_BOUND_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook(profile_dir);
        }
    });
}

#[cfg(test)]
fn run_ownership_commit_barrier_hook(
    profile_dir: &Path,
    committed_path: &Path,
) {
    OWNERSHIP_COMMIT_BARRIER_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook(profile_dir, committed_path);
        }
    });
}

#[cfg(all(test, windows))]
fn run_final_profile_rmdir_barrier_hook(profile_dir: &Path) {
    FINAL_PROFILE_RMDIR_BARRIER_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook(profile_dir);
        }
    });
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessIdentity {
    pid: u32,
    /// Portable diagnostic value. The platform-specific key below is the
    /// identity anchor used for termination.
    start_time_epoch_seconds: u64,
    /// Windows: the full 100 ns creation FILETIME. Linux: `/proc/<pid>/stat`
    /// field 22 (raw boot ticks). Other Unix: the best available start value.
    platform_start_key: u64,
    executable: String,
    /// `ChildProcessBuilder` makes the Chromium root a process-group leader on
    /// Unix. It is absent on Windows, where recovery uses exact handles + Job.
    process_group_id: Option<u32>,
}

impl std::fmt::Debug for ProcessIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessIdentity")
            .field("pid", &self.pid)
            .field("start_time_present", &(self.start_time_epoch_seconds != 0))
            .field("platform_start_key_present", &(self.platform_start_key != 0))
            .field("executable_configured", &!self.executable.is_empty())
            .field("process_group_id", &self.process_group_id)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
enum BrowserOwnershipPhase {
    /// An explicitly ephemeral profile exists and may have spawned Chromium,
    /// but the exact browser identity has not been committed yet.
    Provisional,
    /// The marker contains the exact managed Chromium process identity.
    Committed,
}

impl Default for BrowserOwnershipPhase {
    fn default() -> Self {
        // Markers written before the phase field was introduced always carried
        // an exact browser identity.
        Self::Committed
    }
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserOwnershipMarker {
    version: u32,
    #[serde(default)]
    phase: BrowserOwnershipPhase,
    app_instance_id: String,
    owner_app: ProcessIdentity,
    browser: ProcessIdentity,
    profile_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProfileDirectoryIdentity {
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    mount_key: [u8; 8],
    #[cfg(target_os = "linux")]
    mount_id: u64,
}

impl std::fmt::Debug for BrowserOwnershipMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserOwnershipMarker")
            .field("version", &self.version)
            .field("phase", &self.phase)
            .field("app_instance_id_configured", &!self.app_instance_id.is_empty())
            .field("owner_app", &self.owner_app)
            .field("browser", &self.browser)
            .field("profile_id_configured", &!self.profile_id.is_empty())
            .finish()
    }
}

/// Opaque proof of the exact ownership marker committed for one managed
/// Chromium launch.
///
/// The canonical profile directory is captured at commit time so normal
/// shutdown cannot accidentally clear an identical marker copied into another
/// profile. The marker payload and profile path are deliberately not exposed.
#[derive(Clone)]
pub(crate) struct BrowserOwnershipToken {
    profile_dir: PathBuf,
    profile_identity: ProfileDirectoryIdentity,
    marker_path: PathBuf,
    marker: BrowserOwnershipMarker,
}

impl std::fmt::Debug for BrowserOwnershipToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserOwnershipToken")
            .field("profile_configured", &true)
            .field("marker", &self.marker)
            .finish()
    }
}

impl BrowserOwnershipToken {
    pub(crate) fn browser_start_time_epoch_seconds(&self) -> u64 {
        self.marker.browser.start_time_epoch_seconds
    }

    pub(crate) fn browser_platform_start_key(&self) -> u64 {
        self.marker.browser.platform_start_key
    }
}

/// Opaque authority for deleting one exact, explicitly ephemeral browser
/// profile before an ownership marker has been committed.
///
/// The path is canonicalized while the caller holds the per-profile launch
/// claim. Keeping this separate from [`BrowserOwnershipToken`] makes it
/// impossible for a stable profile to enter the uncommitted whole-directory
/// cleanup path by accident.
#[derive(Clone)]
pub(crate) struct EphemeralProfileCleanupToken {
    profile_dir: PathBuf,
    profile_identity: ProfileDirectoryIdentity,
    provisional_marker: BrowserOwnershipMarker,
}

impl std::fmt::Debug for EphemeralProfileCleanupToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EphemeralProfileCleanupToken")
            .field("profile_configured", &true)
            .finish()
    }
}

impl EphemeralProfileCleanupToken {
    pub(crate) fn into_profile_dir(self) -> PathBuf {
        self.profile_dir
    }
}

/// Whether recovery may delete the profile after proving its process tree is
/// gone. Primary profiles use `PreserveStableProfile`; only explicitly
/// ephemeral roots use `DeleteEphemeralProfile`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileRecoveryMode {
    DeleteEphemeralProfile,
    PreserveStableProfile,
}

/// Display-safe startup recovery totals. No PID, executable path, endpoint, or
/// profile path is included in the summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileRecoveryReport {
    pub markers_scanned: usize,
    pub process_trees_terminated: usize,
    pub ephemeral_profiles_removed: usize,
    pub stable_markers_cleared: usize,
    pub live_owners_preserved: usize,
    pub profiles_preserved: usize,
    pub failures: usize,
}

impl ProfileRecoveryReport {
    pub fn merge(&mut self, other: Self) {
        self.markers_scanned += other.markers_scanned;
        self.process_trees_terminated += other.process_trees_terminated;
        self.ephemeral_profiles_removed += other.ephemeral_profiles_removed;
        self.stable_markers_cleared += other.stable_markers_cleared;
        self.live_owners_preserved += other.live_owners_preserved;
        self.profiles_preserved += other.profiles_preserved;
        self.failures += other.failures;
    }

    pub fn safety_summary(&self) -> String {
        format!(
            "markers={}, terminated_trees={}, removed_ephemeral={}, cleared_stable_markers={}, live_owner_profiles_preserved={}, unverified_profiles_preserved={}, failures={}",
            self.markers_scanned,
            self.process_trees_terminated,
            self.ephemeral_profiles_removed,
            self.stable_markers_cleared,
            self.live_owners_preserved,
            self.profiles_preserved,
            self.failures,
        )
    }
}

#[derive(Clone, Debug)]
enum ProcessLookup {
    Missing,
    Found(ProcessIdentity),
    Unverified(String),
}

trait ProcessControl {
    fn current_process(&mut self) -> Result<ProcessIdentity, String>;
    fn lookup(&mut self, pid: u32) -> ProcessLookup;
    fn confirm_tree_absent(&mut self, expected: &ProcessIdentity) -> Result<bool, String>;
    /// Terminate the exact verified process tree anchored at `expected`.
    ///
    /// Unix-only: Windows startup recovery never signals marker-derived
    /// processes; its Job Object already guarantees tree exit.
    #[cfg(unix)]
    fn terminate_tree(&mut self, expected: &ProcessIdentity) -> Result<(), String>;
}

struct SystemProcessControl;

struct ProfileOperationClaim {
    file: std::fs::File,
    profile_dir: PathBuf,
    profile_identity: std::sync::Mutex<ProfileDirectoryIdentity>,
    #[cfg(windows)]
    profile_guard: std::sync::Mutex<Option<std::fs::File>>,
}

impl ProfileOperationClaim {
    #[cfg(windows)]
    fn acquire(profile_dir: &Path) -> Result<Self, String> {
        Self::acquire_internal(profile_dir, false)
    }

    #[cfg(not(windows))]
    fn acquire(profile_dir: &Path) -> Result<Self, String> {
        Self::acquire_internal(profile_dir)
    }

    #[cfg(windows)]
    fn acquire_pinned(profile_dir: &Path) -> Result<Self, String> {
        Self::acquire_internal(profile_dir, true)
    }

    fn acquire_internal(
        profile_dir: &Path,
        #[cfg(windows)] pin_profile_directory: bool,
    ) -> Result<Self, String> {
        let metadata = std::fs::symlink_metadata(profile_dir)
            .map_err(|error| format!("inspect browser profile before locking: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("browser profile is not a regular directory".into());
        }
        let canonical_profile = std::fs::canonicalize(profile_dir)
            .map_err(|error| format!("canonicalize browser profile before locking: {error}"))?;
        let profile_id = canonical_profile
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "browser profile has no valid Unicode id".to_string())?;
        // A stable, deliberately conservative hash is sufficient here:
        // collisions only serialize two sibling profiles; they cannot grant
        // access or authorize cleanup.
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in profile_id.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let parent = canonical_profile
            .parent()
            .ok_or_else(|| "browser profile has no parent directory".to_string())?;
        let lock_path = parent.join(format!(
            "{PROFILE_OPERATION_LOCK_PREFIX}-{hash:016x}.lock"
        ));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| format!("open browser profile operation lock: {error}"))?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
            format!("browser profile is already being launched or recovered: {error}")
        })?;
        #[cfg(windows)]
        let (profile_identity, profile_guard) = if pin_profile_directory {
            let guard = open_locked_non_reparse_directory(&canonical_profile)?;
            let identity = windows_directory_identity(&guard)?;
            (
                ProfileDirectoryIdentity {
                    volume_serial_number: identity.volume_serial_number,
                    file_index: identity.file_index,
                },
                Some(guard),
            )
        } else {
            (
                capture_profile_directory_identity(&canonical_profile)?,
                None,
            )
        };
        #[cfg(not(windows))]
        let profile_identity = capture_profile_directory_identity(&canonical_profile)?;
        Ok(Self {
            file,
            profile_dir: canonical_profile,
            profile_identity: std::sync::Mutex::new(profile_identity),
            #[cfg(windows)]
            profile_guard: std::sync::Mutex::new(profile_guard),
        })
    }

    fn validates(&self, profile_dir: &Path) -> Result<(), String> {
        let canonical = std::fs::canonicalize(profile_dir)
            .map_err(|error| format!("canonicalize claimed browser profile: {error}"))?;
        if canonical != self.profile_dir {
            return Err("browser profile operation claim belongs to a different directory".into());
        }
        let expected = self.directory_identity()?;
        if capture_profile_directory_identity(&canonical)? != expected {
            return Err("browser profile directory identity changed under its operation claim".into());
        }
        Ok(())
    }

    /// Recreate the one exact claimed directory after a completed ephemeral
    /// launch attempt removed it.
    ///
    /// The operation lock lives in the canonical parent, so deleting the child
    /// directory does not release or change this claim. Re-creation is allowed
    /// only when the caller supplies the same final component below that exact
    /// canonical parent. Existing symlinks/junctions, files, or a directory
    /// resolving anywhere else fail closed.
    ///
    /// 仅被 Windows 启动重试路径（`restore_ephemeral_profile_for_retry`）与其
    /// 跨平台单测使用，故与之同门：`cfg(any(windows, test))`。
    #[cfg(any(windows, test))]
    fn restore_exact_directory(&self, profile_dir: &Path) -> Result<(), String> {
        match std::fs::symlink_metadata(profile_dir) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(
                        "claimed browser profile retry target is not a regular directory".into(),
                    );
                }
                return self.validates(profile_dir);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "inspect claimed browser profile retry target: {error}"
                ));
            }
        }

        let claimed_parent = self
            .profile_dir
            .parent()
            .ok_or_else(|| "claimed browser profile has no parent directory".to_string())?;
        let supplied_parent = profile_dir
            .parent()
            .ok_or_else(|| "browser profile retry target has no parent directory".to_string())?;
        let supplied_name = profile_dir
            .file_name()
            .ok_or_else(|| "browser profile retry target has no final component".to_string())?;
        let canonical_parent = std::fs::canonicalize(supplied_parent)
            .map_err(|error| format!("canonicalize browser profile retry parent: {error}"))?;
        if canonical_parent != claimed_parent
            || claimed_parent.join(supplied_name) != self.profile_dir
        {
            return Err(
                "browser profile retry target no longer matches the claimed directory".into(),
            );
        }

        match std::fs::create_dir(profile_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "recreate exact ephemeral browser profile for retry: {error}"
                ));
            }
        }
        let metadata = std::fs::symlink_metadata(profile_dir)
            .map_err(|error| format!("inspect recreated browser profile: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("recreated browser profile is not a regular directory".into());
        }
        let canonical = std::fs::canonicalize(profile_dir)
            .map_err(|error| format!("canonicalize recreated browser profile: {error}"))?;
        if canonical != self.profile_dir {
            return Err("recreated browser profile no longer matches its claim".into());
        }
        let identity = capture_profile_directory_identity(&canonical)?;
        *self
            .profile_identity
            .lock()
            .map_err(|_| "browser profile identity lock was poisoned".to_string())? =
            identity;
        Ok(())
    }

    fn directory_identity(&self) -> Result<ProfileDirectoryIdentity, String> {
        #[cfg(windows)]
        {
            let guard = self
                .profile_guard
                .lock()
                .map_err(|_| "browser profile guard lock was poisoned".to_string())?;
            if let Some(directory) = guard.as_ref() {
                let identity = windows_directory_identity(directory)?;
                return Ok(ProfileDirectoryIdentity {
                    volume_serial_number: identity.volume_serial_number,
                    file_index: identity.file_index,
                });
            }
        }
        self.profile_identity
            .lock()
            .map(|identity| *identity)
            .map_err(|_| "browser profile identity lock was poisoned".to_string())
    }

    #[cfg(windows)]
    fn release_profile_guard_for_directory_removal(&self) -> Result<(), String> {
        let guard = self
            .profile_guard
            .lock()
            .map_err(|_| "browser profile guard lock was poisoned".to_string())?
            .take();
        drop(guard);
        Ok(())
    }
}

impl Drop for ProfileOperationClaim {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

/// Exclusive per-profile launch claim. The OS releases it automatically if the
/// process crashes, so a failed recovery cannot permanently quarantine data.
pub struct ProfileLaunchClaim(ProfileOperationClaim);

impl ProcessControl for SystemProcessControl {
    fn current_process(&mut self) -> Result<ProcessIdentity, String> {
        let pid = sysinfo::get_current_pid()
            .map_err(|error| format!("resolve current process id: {error}"))?
            .as_u32();
        match self.lookup(pid) {
            ProcessLookup::Found(identity) => Ok(identity),
            ProcessLookup::Missing => Err("current application process missing from snapshot".into()),
            ProcessLookup::Unverified(error) => Err(error),
        }
    }

    fn lookup(&mut self, pid: u32) -> ProcessLookup {
        if pid == 0 {
            return ProcessLookup::Unverified("PID 0 is not a valid process identity".into());
        }
        let system = match fresh_process_snapshot() {
            Ok(system) => system,
            Err(error) => return ProcessLookup::Unverified(error),
        };
        let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) else {
            return ProcessLookup::Missing;
        };
        match process_identity(process) {
            Ok(identity) => ProcessLookup::Found(identity),
            Err(error) => ProcessLookup::Unverified(error),
        }
    }

    fn confirm_tree_absent(&mut self, expected: &ProcessIdentity) -> Result<bool, String> {
        confirm_process_tree_absent(expected)
    }

    #[cfg(unix)]
    fn terminate_tree(&mut self, expected: &ProcessIdentity) -> Result<(), String> {
        let Some(pgid) = expected.process_group_id else {
            return Err("browser marker has no process group".into());
        };
        // `ChildProcessBuilder` makes the managed Chromium root its own
        // process-group leader; anything else is not a tree this recovery may
        // ever signal.
        if pgid == 0 || pgid != expected.pid {
            return Err(
                "browser marker is not its expected process-group leader".into(),
            );
        }
        // SAFETY: getpgrp only reads the current process-group id.
        let app_group = unsafe { libc::getpgrp() };
        if app_group <= 0 || pgid == app_group as u32 {
            return Err(
                "browser process group overlaps the current application group".into(),
            );
        }
        // Re-verify the exact creation identity immediately before signalling
        // so a PID recycled between inventory and termination is never killed.
        match self.lookup(expected.pid) {
            ProcessLookup::Found(observed) if same_process(expected, &observed) => {}
            ProcessLookup::Missing => return Ok(()),
            ProcessLookup::Found(_) => {
                return Err(
                    "browser PID was reused by a different identity before termination"
                        .into(),
                );
            }
            ProcessLookup::Unverified(error) => {
                return Err(format!(
                    "orphan browser identity could not be re-verified before termination: {error}"
                ));
            }
        }
        // SAFETY: killpg signals only the verified, application-isolated
        // browser process group.
        if unsafe { libc::killpg(pgid as libc::pid_t, libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            return Err(format!(
                "terminate orphan browser process group: {error}"
            ));
        }
        Ok(())
    }
}

fn managed_app_instance_id() -> &'static str {
    MANAGED_APP_INSTANCE_ID
        .get_or_init(nomifun_common::generate_id)
        .as_str()
}

fn ownership_marker_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(OWNERSHIP_MARKER_FILE)
}

fn committed_ownership_record_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(COMMITTED_OWNERSHIP_RECORD_FILE)
}

fn is_ownership_record_name(name: &std::ffi::OsStr) -> bool {
    name == OWNERSHIP_MARKER_FILE || name == COMMITTED_OWNERSHIP_RECORD_FILE
}

fn normalized_executable(path: &Path) -> Result<String, String> {
    if path.as_os_str().is_empty() {
        return Err("process executable path is empty".into());
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("canonicalize process executable: {error}"))?;
    let simplified = dunce::simplified(&canonical);
    let text = simplified
        .to_str()
        .ok_or_else(|| "process executable path is not valid Unicode".to_string())?;
    #[cfg(windows)]
    {
        Ok(text.to_lowercase())
    }
    #[cfg(not(windows))]
    {
        Ok(text.to_owned())
    }
}

fn same_process(expected: &ProcessIdentity, observed: &ProcessIdentity) -> bool {
    expected.pid == observed.pid
        && expected.start_time_epoch_seconds == observed.start_time_epoch_seconds
        && expected.platform_start_key == observed.platform_start_key
        && expected.executable == observed.executable
        && expected.process_group_id == observed.process_group_id
}

fn fresh_process_snapshot() -> Result<sysinfo::System, String> {
    let mut system = sysinfo::System::new();
    let updated = system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_exe(sysinfo::UpdateKind::Always)
            .without_tasks(),
    );
    if updated == 0 {
        return Err("process snapshot returned no updated processes".into());
    }
    let current = sysinfo::get_current_pid()
        .map_err(|error| format!("resolve current PID for snapshot health check: {error}"))?;
    if system.process(current).is_none() {
        return Err("process snapshot did not contain the current application".into());
    }
    Ok(system)
}

fn process_identity(process: &sysinfo::Process) -> Result<ProcessIdentity, String> {
    let pid = process.pid().as_u32();
    if pid == 0 {
        return Err("process snapshot returned PID 0".into());
    }

    #[cfg(windows)]
    {
        let identity = nomi_process_runtime::windows_process_identity(pid)
            .map_err(|error| format!("inspect process {pid}: {error}"))?;
        return process_identity_from_windows(&identity);
    }

    #[cfg(not(windows))]
    {
        let start_time_epoch_seconds = process.start_time();
        if start_time_epoch_seconds == 0 {
            return Err(format!("process {pid} has no usable start time"));
        }
        let executable = process
            .exe()
            .ok_or_else(|| format!("process {pid} executable is unavailable"))
            .and_then(normalized_executable)?;
        let platform_start_key = unix_platform_start_key(pid, start_time_epoch_seconds)?;
        // SAFETY: getpgid is a read-only query for the just-snapshotted PID.
        let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
        if pgid <= 0 {
            return Err(format!(
                "query process group for PID {pid}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(ProcessIdentity {
            pid,
            start_time_epoch_seconds,
            platform_start_key,
            executable,
            process_group_id: Some(pgid as u32),
        })
    }
}

#[cfg(target_os = "linux")]
fn unix_platform_start_key(pid: u32, _fallback: u64) -> Result<u64, String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|error| format!("read raw start time for PID {pid}: {error}"))?;
    let command_end = stat
        .rfind(')')
        .ok_or_else(|| format!("malformed /proc/{pid}/stat"))?;
    let fields = stat
        .get(command_end + 1..)
        .ok_or_else(|| format!("malformed /proc/{pid}/stat suffix"))?
        .split_whitespace()
        .collect::<Vec<_>>();
    // The suffix begins at field 3 (state); index 19 is field 22 (starttime).
    fields
        .get(19)
        .ok_or_else(|| format!("/proc/{pid}/stat has no starttime field"))?
        .parse::<u64>()
        .map_err(|error| format!("parse raw start time for PID {pid}: {error}"))
}

#[cfg(target_os = "macos")]
fn unix_platform_start_key(pid: u32, _fallback: u64) -> Result<u64, String> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    // SAFETY: info is a correctly sized writable proc_bsdinfo buffer.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int,
        )
    };
    if written != std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int
        || info.pbi_pid != pid
    {
        return Err(format!(
            "query exact macOS creation time for PID {pid}: {}",
            std::io::Error::last_os_error()
        ));
    }
    info.pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .filter(|value| *value != 0)
        .ok_or_else(|| format!("macOS process {pid} has an invalid creation time"))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn unix_platform_start_key(_pid: u32, _fallback: u64) -> Result<u64, String> {
    Err("this Unix target has no exact process creation identity backend".into())
}

#[cfg(windows)]
fn process_identity_from_windows(
    identity: &nomi_process_runtime::WindowsProcessIdentity,
) -> Result<ProcessIdentity, String> {
    Ok(ProcessIdentity {
        pid: identity.pid,
        start_time_epoch_seconds: identity.start_time_epoch_seconds,
        platform_start_key: identity.platform_start_key,
        executable: normalized_executable(&identity.executable)?,
        process_group_id: None,
    })
}

#[cfg(windows)]
fn windows_child_process_identity(
    child: &tokio::process::Child,
) -> Result<ProcessIdentity, String> {
    let identity = nomi_process_runtime::windows_child_process_identity(child)
        .map_err(|error| format!("inspect spawned browser process: {error}"))?;
    process_identity_from_windows(&identity)
}

fn validate_marker(marker: &BrowserOwnershipMarker, profile_dir: &Path) -> Result<(), String> {
    if marker.version != OWNERSHIP_MARKER_VERSION {
        return Err(format!("unsupported ownership marker version {}", marker.version));
    }
    nomifun_common::validate_uuidv7(&marker.app_instance_id)
        .map_err(|error| format!("invalid app instance id: {error}"))?;
    let profile_id = profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "profile directory has no valid Unicode id".to_string())?;
    if marker.profile_id != profile_id {
        return Err("ownership marker profile id does not match its directory".into());
    }
    validate_marker_process(&marker.owner_app)?;
    validate_marker_process(&marker.browser)?;
    if marker.phase == BrowserOwnershipPhase::Provisional
        && !same_process(&marker.owner_app, &marker.browser)
    {
        return Err(
            "provisional ownership marker does not match its exact owner identity".into(),
        );
    }
    if marker.phase == BrowserOwnershipPhase::Committed
        && same_process(&marker.owner_app, &marker.browser)
    {
        return Err(
            "committed browser ownership marker aliases the application identity".into(),
        );
    }
    Ok(())
}

fn validate_marker_process(identity: &ProcessIdentity) -> Result<(), String> {
    if identity.pid == 0
        || identity.start_time_epoch_seconds == 0
        || identity.platform_start_key == 0
        || identity.executable.is_empty()
        || !Path::new(&identity.executable).is_absolute()
    {
        return Err("ownership marker contains an incomplete process identity".into());
    }
    #[cfg(unix)]
    if identity.process_group_id.is_none() {
        return Err("ownership marker is missing its Unix process group".into());
    }
    #[cfg(windows)]
    if identity.process_group_id.is_some() {
        return Err("ownership marker contains an unexpected Windows process group".into());
    }
    Ok(())
}

fn read_ownership_record_bytes(
    file: &mut std::fs::File,
    declared_len: u64,
    context: &'static str,
) -> Result<Vec<u8>, String> {
    if declared_len > MAX_OWNERSHIP_RECORD_BYTES {
        return Err(format!("{context} is too large"));
    }
    let mut bytes = Vec::with_capacity(declared_len as usize);
    (&mut *file)
        .take(MAX_OWNERSHIP_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {context}: {error}"))?;
    if bytes.len() as u64 > MAX_OWNERSHIP_RECORD_BYTES {
        return Err(format!("{context} grew beyond its size limit"));
    }
    Ok(bytes)
}

fn read_marker_at(
    profile_dir: &Path,
    path: &Path,
) -> Result<BrowserOwnershipMarker, String> {
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("read ownership marker metadata: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("ownership marker is not a regular file".into());
    }
    if metadata.len() > MAX_OWNERSHIP_RECORD_BYTES {
        return Err("ownership marker is too large".into());
    }
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("open ownership marker: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("inspect opened ownership marker: {error}"))?;
    if !opened_metadata.is_file() {
        return Err("opened ownership marker is not a regular file".into());
    }
    let bytes = read_ownership_record_bytes(
        &mut file,
        opened_metadata.len(),
        "ownership marker",
    )?;
    let marker: BrowserOwnershipMarker = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse ownership marker: {error}"))?;
    validate_marker(&marker, profile_dir)?;
    Ok(marker)
}

fn exact_provisional_to_committed_lineage(
    provisional: &BrowserOwnershipMarker,
    committed: &BrowserOwnershipMarker,
) -> bool {
    committed.phase == BrowserOwnershipPhase::Committed
        && *provisional == provisional_predecessor(committed)
}

fn provisional_predecessor(
    committed: &BrowserOwnershipMarker,
) -> BrowserOwnershipMarker {
    let mut provisional = committed.clone();
    provisional.phase = BrowserOwnershipPhase::Provisional;
    provisional.browser = provisional.owner_app.clone();
    provisional
}

#[derive(Clone)]
struct OwnershipRecordSet {
    active_path: PathBuf,
    active: BrowserOwnershipMarker,
    provisional_predecessor: Option<(PathBuf, BrowserOwnershipMarker)>,
}

#[cfg(windows)]
struct PinnedOwnershipRecord {
    _file: std::fs::File,
    path: PathBuf,
    marker: BrowserOwnershipMarker,
}

#[cfg(windows)]
struct WindowsOwnershipCommitGuards {
    _committed: std::fs::File,
    _provisional_predecessor: Option<PinnedOwnershipRecord>,
}

#[cfg(windows)]
struct PinnedOwnershipRecordSet {
    active: PinnedOwnershipRecord,
    provisional_predecessor: Option<PinnedOwnershipRecord>,
}

#[cfg(windows)]
impl PinnedOwnershipRecordSet {
    fn marker(&self) -> &BrowserOwnershipMarker {
        &self.active.marker
    }

    fn records(&self) -> OwnershipRecordSet {
        OwnershipRecordSet {
            active_path: self.active.path.clone(),
            active: self.active.marker.clone(),
            provisional_predecessor: self
                .provisional_predecessor
                .as_ref()
                .map(|record| (record.path.clone(), record.marker.clone())),
        }
    }
}

#[cfg(windows)]
fn open_pinned_ownership_record(
    profile_dir: &Path,
    path: PathBuf,
) -> Result<Option<PinnedOwnershipRecord>, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ,
    };

    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        // Existing or future write/delete handles are incompatible with this
        // authority handle. Its parsed bytes therefore remain immutable until
        // startup recovery has completed every absence proof.
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "open pinned browser ownership record: {error}"
            ));
        }
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect pinned browser ownership record: {error}"))?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(
            "pinned browser ownership record is a reparse point or non-file".into(),
        );
    }
    let (_, _, links) = windows_file_identity(&file)
        .map_err(|error| format!("identify pinned browser ownership record: {error}"))?;
    if links != 1 {
        return Err("pinned browser ownership record has multiple hard links".into());
    }
    let bytes = read_ownership_record_bytes(
        &mut file,
        metadata.len(),
        "pinned browser ownership record",
    )?;
    let marker: BrowserOwnershipMarker = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse pinned browser ownership record: {error}"))?;
    validate_marker(&marker, profile_dir)?;
    Ok(Some(PinnedOwnershipRecord {
        _file: file,
        path,
        marker,
    }))
}

#[cfg(windows)]
fn read_pinned_ownership_record_set(
    profile_dir: &Path,
) -> Result<PinnedOwnershipRecordSet, String> {
    let marker_path = ownership_marker_path(profile_dir);
    let committed_path = committed_ownership_record_path(profile_dir);
    let committed =
        open_pinned_ownership_record(profile_dir, committed_path)?;
    let provisional =
        open_pinned_ownership_record(profile_dir, marker_path)?;
    match (committed, provisional) {
        (None, None) => Err("browser ownership record disappeared before pinning".into()),
        (None, Some(active)) => Ok(PinnedOwnershipRecordSet {
            active,
            provisional_predecessor: None,
        }),
        (Some(committed), provisional) => {
            if committed.marker.phase != BrowserOwnershipPhase::Committed {
                return Err(
                    "pinned committed ownership sidecar has a non-committed phase".into(),
                );
            }
            if let Some(predecessor) = provisional.as_ref()
                && !exact_provisional_to_committed_lineage(
                    &predecessor.marker,
                    &committed.marker,
                )
            {
                return Err(
                    "pinned committed ownership record does not match its exact provisional predecessor"
                        .into(),
                );
            }
            Ok(PinnedOwnershipRecordSet {
                active: committed,
                provisional_predecessor: provisional,
            })
        }
    }
}

/// Resolve one profile's append-only ownership state.
///
/// A valid pair resolves to its exact committed record. A committed sidecar
/// also remains authoritative by itself: cleanup deletes the provisional
/// predecessor first and the committed record last, so a crash between those
/// unlinks retains the stronger exact browser lineage. A present but
/// mismatched predecessor always quarantines the profile.
fn read_ownership_record_set(
    profile_dir: &Path,
) -> Result<OwnershipRecordSet, String> {
    let marker_path = ownership_marker_path(profile_dir);
    let committed_path = committed_ownership_record_path(profile_dir);
    match std::fs::symlink_metadata(&committed_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let active = read_marker_at(profile_dir, &marker_path)?;
            Ok(OwnershipRecordSet {
                active_path: marker_path,
                active,
                provisional_predecessor: None,
            })
        }
        Err(error) => Err(format!(
            "inspect committed ownership record: {error}"
        )),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("committed ownership record is not a regular file".into())
        }
        Ok(_) => {
            let committed = read_marker_at(profile_dir, &committed_path)?;
            if committed.phase != BrowserOwnershipPhase::Committed {
                return Err("committed ownership sidecar has a non-committed phase".into());
            }
            let provisional_predecessor = match std::fs::symlink_metadata(&marker_path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(format!(
                        "inspect provisional ownership predecessor: {error}"
                    ));
                }
                Ok(_) => {
                    let provisional = read_marker_at(profile_dir, &marker_path)?;
                    if !exact_provisional_to_committed_lineage(&provisional, &committed) {
                        return Err(
                            "committed ownership record does not match its exact provisional predecessor"
                                .into(),
                        );
                    }
                    Some((marker_path, provisional))
                }
            };
            Ok(OwnershipRecordSet {
                active_path: committed_path,
                active: committed,
                provisional_predecessor,
            })
        }
    }
}

fn read_marker(profile_dir: &Path) -> Result<BrowserOwnershipMarker, String> {
    Ok(read_ownership_record_set(profile_dir)?.active)
}

/// Acquire the per-profile OS lock and refuse to launch over a profile which
/// may still belong to a live process.
///
/// A stale marker may be cleared here only when it belongs to this exact
/// process-local app instance and the prior browser tree is proven absent.
/// Markers from another app instance are handled exclusively by startup
/// recovery.
pub fn prepare_ownership_marker_for_launch(
    profile_dir: &Path,
) -> Result<ProfileLaunchClaim, String> {
    let claim = ProfileLaunchClaim(ProfileOperationClaim::acquire(profile_dir)?);
    prepare_ownership_marker_under_claim(profile_dir, &claim)?;
    Ok(claim)
}

/// Revalidate the same claimed profile between bounded launch attempts.
pub fn prepare_ownership_marker_for_retry(
    profile_dir: &Path,
    claim: &ProfileLaunchClaim,
) -> Result<(), String> {
    prepare_ownership_marker_under_claim(profile_dir, claim)
}

/// Bind whole-directory cleanup authority to the exact profile protected by a
/// held launch claim. Callers must invoke this only for an explicitly
/// ephemeral engine configuration.
pub(crate) fn claim_ephemeral_profile_cleanup(
    profile_dir: &Path,
    claim: &ProfileLaunchClaim,
) -> Result<EphemeralProfileCleanupToken, String> {
    claim.0.validates(profile_dir)?;
    let mut control = SystemProcessControl;
    let owner_app = control.current_process()?;
    let profile_id = profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "profile directory has no valid Unicode id".to_string())?
        .to_owned();
    // Until Chromium's identity is committed, the current app identity is a
    // conservative provisional browser identity. Recovery always checks the
    // live owner first, so it never signals the running application. On the
    // next startup both identities refer to the absent prior app and provide
    // durable, fail-closed lineage for this otherwise unmarked profile.
    let provisional_marker = BrowserOwnershipMarker {
        version: OWNERSHIP_MARKER_VERSION,
        phase: BrowserOwnershipPhase::Provisional,
        app_instance_id: managed_app_instance_id().to_owned(),
        owner_app: owner_app.clone(),
        browser: owner_app,
        profile_id,
    };
    if let Err(error) = commit_ownership_marker_under_claim(
        profile_dir,
        &provisional_marker,
        None,
        &claim.0,
    ) {
        // No browser has spawned yet, so an explicitly ephemeral directory can
        // be removed immediately only while a complete scan proves that no
        // ownership marker appeared. A namespace replacement must never be
        // mistaken for that original unmarked directory: only a still-valid
        // exact claim may authorize this best-effort rollback.
        if claim.0.validates(profile_dir).is_ok() {
            let _ = cleanup_unmarked_ephemeral_profile_under_launch_claim(profile_dir, claim);
        }
        return Err(error);
    }
    Ok(EphemeralProfileCleanupToken {
        profile_dir: claim.0.profile_dir.clone(),
        profile_identity: claim.0.directory_identity()?,
        provisional_marker,
    })
}

fn cleanup_unmarked_ephemeral_profile_under_launch_claim(
    profile_dir: &Path,
    claim: &ProfileLaunchClaim,
) -> Result<(), String> {
    claim.0.validates(profile_dir)?;
    let (markers, scan_errors, _) = collect_marker_paths(profile_dir);
    if let Some(error) = scan_errors.into_iter().next() {
        return Err(format!(
            "refusing unmarked ephemeral cleanup after an incomplete marker scan: {error}"
        ));
    }
    if !markers.is_empty() {
        return Err(
            "refusing unmarked ephemeral cleanup after ownership appeared".into(),
        );
    }
    remove_ephemeral_profile_contents_marker_last(
        profile_dir,
        None,
        claim.0.directory_identity()?,
    )
}

/// Restore an ephemeral profile removed by a completed first launch attempt,
/// then re-run the normal ownership preflight under the same still-held OS
/// claim.
///
/// 唯一生产调用点在 `launch.rs` 的 `#[cfg(windows)]` 启动重试分支；跨平台单测
/// 直接调用它，故门为 `cfg(any(windows, test))` 而非删除。
#[cfg(any(windows, test))]
pub(crate) fn restore_ephemeral_profile_for_retry(
    profile_dir: &Path,
    claim: &ProfileLaunchClaim,
) -> Result<EphemeralProfileCleanupToken, String> {
    claim.0.restore_exact_directory(profile_dir)?;
    prepare_ownership_marker_under_claim(profile_dir, claim)?;
    claim_ephemeral_profile_cleanup(profile_dir, claim)
}

/// Remove a stale runtime endpoint before spawning Chromium. Ignoring a
/// deletion failure could make a new process/marker pair appear connected to
/// an endpoint left by a prior process, so every unsafe file type or I/O error
/// fails the launch closed.
pub(crate) fn prepare_runtime_port_for_launch(
    profile_dir: &Path,
    claim: &ProfileLaunchClaim,
) -> Result<(), String> {
    claim.0.validates(profile_dir)?;
    remove_regular_devtools_active_port(profile_dir, "launch preparation")
}

fn prepare_ownership_marker_under_claim(
    profile_dir: &Path,
    claim: &ProfileLaunchClaim,
) -> Result<(), String> {
    claim.0.validates(profile_dir)?;
    let provisional_path = ownership_marker_path(profile_dir);
    let committed_path = committed_ownership_record_path(profile_dir);
    let provisional_exists = match std::fs::symlink_metadata(&provisional_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("inspect existing ownership marker: {error}")),
        Ok(_) => true,
    };
    let committed_exists = match std::fs::symlink_metadata(&committed_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(format!(
                "inspect existing committed ownership record: {error}"
            ));
        }
        Ok(_) => true,
    };
    if !provisional_exists && !committed_exists {
        return Ok(());
    }

    let records = read_ownership_record_set(profile_dir)?;
    let marker = records.active.clone();
    let mut control = SystemProcessControl;
    let current = control.current_process()?;
    let owner_is_current = same_process(&marker.owner_app, &current)
        && marker.app_instance_id == managed_app_instance_id();
    if !owner_is_current {
        return Err(
            "profile ownership belongs to another or unverified app instance; startup recovery must resolve it"
                .into(),
        );
    }
    if marker.phase == BrowserOwnershipPhase::Provisional {
        return Err(
            "ephemeral profile has provisional ownership; startup recovery must quarantine it"
                .into(),
        );
    }

    match control.lookup(marker.browser.pid) {
        ProcessLookup::Found(observed) if same_process(&marker.browser, &observed) => {
            Err("the previous managed browser process is still running".into())
        }
        ProcessLookup::Found(_) => {
            Err("the previous browser PID has been reused; profile ownership remains quarantined".into())
        }
        ProcessLookup::Unverified(error) => Err(format!(
            "could not verify the previous managed browser before launch: {error}"
        )),
        ProcessLookup::Missing => {
            if !control.confirm_tree_absent(&marker.browser)? {
                return Err("the previous managed browser tree is not proven absent".into());
            }
            remove_regular_devtools_active_port(profile_dir, "launch recovery")?;
            let current = read_ownership_record_set(profile_dir)?;
            if current.active != marker
                || current.active_path != records.active_path
                || current.provisional_predecessor != records.provisional_predecessor
            {
                return Err(
                    "ownership records changed before launch recovery cleanup".into(),
                );
            }
            if let Some((path, expected)) = &records.provisional_predecessor {
                if read_marker_at(profile_dir, path)? != *expected {
                    return Err(
                        "provisional ownership predecessor changed before launch recovery".into(),
                    );
                }
                std::fs::remove_file(path).map_err(|error| {
                    format!("remove completed provisional ownership record: {error}")
                })?;
            }
            if read_marker_at(profile_dir, &records.active_path)? != marker {
                return Err(
                    "active ownership record changed before launch recovery".into(),
                );
            }
            std::fs::remove_file(&records.active_path)
                .map_err(|error| format!("remove completed ownership record: {error}"))
        }
    }
}

/// Write a secret-free ownership marker immediately after Chromium spawn.
///
/// The observed process executable must resolve to the configured executable;
/// the marker records the observed creation identity, not caller-supplied PID
/// metadata. The caller must kill the newly spawned child if this returns an
/// error.
pub(crate) async fn write_browser_ownership_marker(
    claim: &ProfileLaunchClaim,
    profile_dir: &Path,
    expected_executable: &Path,
    child: &tokio::process::Child,
    provisional_cleanup: Option<&EphemeralProfileCleanupToken>,
) -> Result<BrowserOwnershipToken, String> {
    claim.0.validates(profile_dir)?;
    if let Some(provisional_cleanup) = provisional_cleanup {
        if provisional_cleanup.profile_dir != claim.0.profile_dir {
            return Err(
                "provisional ephemeral marker belongs to a different launch claim".into(),
            );
        }
        if provisional_cleanup.profile_identity != claim.0.directory_identity()? {
            return Err(
                "provisional ephemeral marker directory identity changed".into(),
            );
        }
    }
    let canonical_profile_dir = claim.0.profile_dir.clone();
    let expected_executable = normalized_executable(expected_executable)?;
    let profile_id = profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "profile directory has no valid Unicode id".to_string())?
        .to_owned();
    let mut control = SystemProcessControl;
    let owner_app = control.current_process()?;

    #[cfg(windows)]
    let browser = windows_child_process_identity(child)?;

    #[cfg(not(windows))]
    let browser = {
        let browser_pid = child
            .id()
            .ok_or_else(|| "spawned browser exited before identity capture".to_string())?;
    let mut browser = None;
    let mut last_error = None;
    for _ in 0..PROCESS_DISCOVERY_RETRIES {
        match control.lookup(browser_pid) {
            ProcessLookup::Found(observed) => {
                browser = Some(observed);
                break;
            }
            ProcessLookup::Missing => {
                last_error = Some("spawned browser is not present in the process snapshot".into());
            }
            ProcessLookup::Unverified(error) => last_error = Some(error),
        }
        tokio::time::sleep(PROCESS_DISCOVERY_INTERVAL).await;
    }
        browser.ok_or_else(|| {
        format!(
            "could not observe spawned browser ownership: {}",
            last_error.unwrap_or_else(|| "unknown process lookup failure".into())
        )
    })?
    };
    if browser.executable != expected_executable {
        return Err(format!(
            "spawned process executable did not match configured browser (pid {})",
            browser.pid
        ));
    }
    #[cfg(unix)]
    if browser.process_group_id != Some(browser.pid) {
        return Err("spawned browser is not its expected process-group leader".into());
    }
    #[cfg(unix)]
    {
        // SAFETY: getpgrp only reads the current process-group id.
        let app_group = unsafe { libc::getpgrp() };
        if app_group <= 0 || browser.process_group_id == Some(app_group as u32) {
            return Err(
                "spawned browser process group is not isolated from the application".into(),
            );
        }
    }

    let marker = BrowserOwnershipMarker {
        version: OWNERSHIP_MARKER_VERSION,
        phase: BrowserOwnershipPhase::Committed,
        app_instance_id: managed_app_instance_id().to_owned(),
        owner_app,
        browser,
        profile_id,
    };
    let expected_profile_identity = claim.0.directory_identity()?;
    // Process discovery may await and refresh several OS snapshots. Commit
    // through the original launch claim so Unix binds every namespace
    // operation to one exact directory fd; the helper also proves that exact
    // directory still occupies the public path before a token is published.
    let marker_path = commit_ownership_marker_under_claim(
        &canonical_profile_dir,
        &marker,
        provisional_cleanup.map(|cleanup| &cleanup.provisional_marker),
        &claim.0,
    )?;
    if claim.0.directory_identity()? != expected_profile_identity {
        return Err(
            "browser profile directory identity changed across ownership commit".into(),
        );
    }
    Ok(BrowserOwnershipToken {
        profile_dir: canonical_profile_dir,
        profile_identity: expected_profile_identity,
        marker_path,
        marker,
    })
}

fn commit_ownership_marker_under_claim(
    profile_dir: &Path,
    marker: &BrowserOwnershipMarker,
    expected_existing: Option<&BrowserOwnershipMarker>,
    claim: &ProfileOperationClaim,
) -> Result<PathBuf, String> {
    claim.validates(profile_dir)?;
    let expected_identity = claim.directory_identity()?;

    #[cfg(windows)]
    let profile_guard = {
        let guard = open_locked_non_reparse_directory(profile_dir)?;
        let identity = windows_directory_identity(&guard)?;
        if (ProfileDirectoryIdentity {
            volume_serial_number: identity.volume_serial_number,
            file_index: identity.file_index,
        }) != expected_identity
        {
            return Err(
                "browser profile directory changed before pinned ownership commit".into(),
            );
        }
        guard
    };
    #[cfg(unix)]
    let marker_path = {
        let directory = UnixDirectory::open_path(profile_dir)?;
        if directory.identity() != expected_identity {
            return Err(
                "browser profile directory changed before fd-bound ownership commit".into(),
            );
        }
        #[cfg(test)]
        run_ownership_commit_directory_bound_hook(profile_dir);
        commit_ownership_marker_unix(
            profile_dir,
            &directory,
            marker,
            expected_existing,
        )?
    };

    #[cfg(windows)]
    let (marker_path, _keep_committed_record_pinned) = {
        let _keep_exact_profile_pinned = &profile_guard;
        commit_ownership_marker_path(
            profile_dir,
            marker,
            expected_existing,
            Some(expected_identity),
        )?
    };

    // A successful append into an unlinked/replaced Unix directory remains
    // durable lineage for that exact inode, but it must never publish a token
    // for the replacement now occupying the configured path.
    claim.validates(profile_dir)?;
    if claim.directory_identity()? != expected_identity {
        return Err(
            "browser profile directory identity changed across ownership commit".into(),
        );
    }
    Ok(marker_path)
}

#[cfg(test)]
fn commit_ownership_marker(
    profile_dir: &Path,
    marker: &BrowserOwnershipMarker,
    expected_existing: Option<&BrowserOwnershipMarker>,
) -> Result<PathBuf, String> {
    #[cfg(unix)]
    {
        let directory = UnixDirectory::open_path(profile_dir)?;
        #[cfg(test)]
        run_ownership_commit_directory_bound_hook(profile_dir);
        return commit_ownership_marker_unix(
            profile_dir,
            &directory,
            marker,
            expected_existing,
        );
    }

    #[cfg(not(unix))]
    {
        #[cfg(windows)]
        {
            let (path, _keep_committed_record_pinned) =
                commit_ownership_marker_path(
                    profile_dir,
                    marker,
                    expected_existing,
                    None,
                )?;
            Ok(path)
        }
    }
}

#[cfg(not(unix))]
fn commit_ownership_marker_path(
    profile_dir: &Path,
    marker: &BrowserOwnershipMarker,
    expected_existing: Option<&BrowserOwnershipMarker>,
    expected_profile_identity: Option<ProfileDirectoryIdentity>,
) -> Result<(PathBuf, WindowsOwnershipCommitGuards), String> {
    use std::io::{Seek, SeekFrom};
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ,
    };

    validate_marker(marker, profile_dir)?;
    let provisional_path = ownership_marker_path(profile_dir);
    let committed_path = committed_ownership_record_path(profile_dir);
    let marker_path = if expected_existing.is_some() {
        committed_path.clone()
    } else {
        provisional_path.clone()
    };

    let pinned_predecessor = match expected_existing {
        Some(expected) => {
            validate_marker(expected, profile_dir)?;
            if *expected != provisional_predecessor(marker) {
                return Err(
                    "ownership record is not an exact provisional-to-committed transition"
                        .into(),
                );
            }
            let predecessor = open_pinned_ownership_record(
                profile_dir,
                provisional_path.clone(),
            )?
            .ok_or_else(|| {
                "provisional ownership marker disappeared during browser spawn"
                    .to_string()
            })?;
            if predecessor.marker != *expected {
                return Err(
                    "provisional ownership marker changed during browser spawn".into(),
                );
            }
            match std::fs::symlink_metadata(&committed_path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "inspect committed ownership record before commit: {error}"
                    ));
                }
                Ok(_) => {
                    return Err(
                        "ownership record unexpectedly appeared during browser spawn".into(),
                    );
                }
            }
            Some(predecessor)
        }
        None => {
            for path in [
                provisional_path.as_path(),
                committed_path.as_path(),
            ] {
                match std::fs::symlink_metadata(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(format!(
                            "inspect ownership record before commit: {error}"
                        ));
                    }
                    Ok(_) => {
                        return Err(
                            "ownership record unexpectedly appeared during browser spawn".into(),
                        );
                    }
                }
            }
            None
        }
    };

    // On Windows an open directory handle without FILE_SHARE_DELETE does not
    // by itself prevent that directory entry from being renamed. Creating the
    // append-only final record first and retaining its no-share-write/delete
    // child handle does. If the process crashes mid-write, the visible
    // malformed record quarantines the profile instead of granting ownership.
    let mut marker_guard = std::fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&marker_path)
        .map_err(|error| format!("create pinned ownership record: {error}"))?;
    let metadata = marker_guard
        .metadata()
        .map_err(|error| format!("inspect pinned new ownership record: {error}"))?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err("new pinned ownership record is a reparse point or non-file".into());
    }
    let (_, _, links) = windows_file_identity(&marker_guard)
        .map_err(|error| format!("identify pinned new ownership record: {error}"))?;
    if links != 1 {
        return Err("new pinned ownership record has multiple hard links".into());
    }

    let root_guard = open_locked_non_reparse_directory(profile_dir)?;
    if let Some(expected_identity) = expected_profile_identity {
        let identity = windows_directory_identity(&root_guard)?;
        if (ProfileDirectoryIdentity {
            volume_serial_number: identity.volume_serial_number,
            file_index: identity.file_index,
        }) != expected_identity
        {
            return Err(
                "browser profile directory changed before anchored ownership commit"
                    .into(),
            );
        }
    }
    #[cfg(test)]
    run_ownership_commit_directory_bound_hook(profile_dir);

    // The pinned child record prevents a profile A/B namespace swap from this
    // point onward. Recheck the append-only inventory under that anchor before
    // writing any bytes.
    match expected_existing {
        Some(expected) => {
            let predecessor = pinned_predecessor
                .as_ref()
                .ok_or_else(|| "missing pinned provisional predecessor".to_string())?;
            if predecessor.marker != *expected {
                return Err(
                    "provisional ownership marker changed before append-only commit".into(),
                );
            }
        }
        None => match std::fs::symlink_metadata(&committed_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "reinspect committed ownership record before commit: {error}"
                ));
            }
            Ok(_) => {
                return Err(
                    "ownership record appeared before atomic commit".into(),
                );
            }
        },
    }

    let bytes = serde_json::to_vec_pretty(&marker)
        .map_err(|error| format!("serialize ownership marker: {error}"))?;
    marker_guard
        .write_all(&bytes)
        .map_err(|error| format!("write pinned ownership record: {error}"))?;
    marker_guard
        .sync_all()
        .map_err(|error| format!("flush pinned ownership record: {error}"))?;

    #[cfg(test)]
    run_ownership_commit_barrier_hook(profile_dir, &marker_path);

    marker_guard
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind pinned ownership record: {error}"))?;
    let readback = read_ownership_record_bytes(
        &mut marker_guard,
        bytes.len() as u64,
        "pinned ownership record readback",
    )?;
    let observed: BrowserOwnershipMarker = serde_json::from_slice(&readback)
        .map_err(|error| format!("parse pinned ownership record readback: {error}"))?;
    validate_marker(&observed, profile_dir)?;
    if observed != *marker {
        return Err("ownership record changed across pinned append-only commit".into());
    }
    if let (Some(expected), Some(predecessor)) =
        (expected_existing, pinned_predecessor.as_ref())
        && predecessor.marker != *expected
    {
        return Err(
            "committed ownership record lost its exact pinned provisional predecessor"
                .into(),
        );
    }
    drop(root_guard);
    Ok((
        marker_path,
        WindowsOwnershipCommitGuards {
            _committed: marker_guard,
            _provisional_predecessor: pinned_predecessor,
        },
    ))
}

#[cfg(windows)]
fn create_exact_ownership_record_no_overwrite(
    profile_dir: &Path,
    record_path: &Path,
    marker: &BrowserOwnershipMarker,
) -> Result<(), String> {
    validate_marker(marker, profile_dir)?;
    let mut random = [0_u8; 16];
    getrandom::getrandom(&mut random)
        .map_err(|error| format!("generate ownership record restore nonce: {error}"))?;
    let temp_path = profile_dir.join(format!(
        "{OWNERSHIP_MARKER_FILE}.restore-{}.tmp",
        hex::encode(random)
    ));
    let bytes = serde_json::to_vec_pretty(marker)
        .map_err(|error| format!("serialize ownership record restore: {error}"))?;
    let restore = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|error| format!("create ownership record restore temp: {error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("write ownership record restore temp: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("flush ownership record restore temp: {error}"))?;
        drop(file);
        if let Err(link_error) = std::fs::hard_link(&temp_path, record_path) {
            // A Windows no-share-delete directory handle intentionally blocks
            // namespace link/rename operations while it pins the exact root.
            // Creating the absent final name is still identity-safe under that
            // guard. O_EXCL preserves the no-overwrite barrier; an interrupted
            // direct write leaves a visible malformed record and recovery
            // therefore remains fail closed.
            let direct_restore = (|| -> Result<(), String> {
                let mut record = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(record_path)
                    .map_err(|error| {
                        format!(
                            "create direct ownership restore after hard-link failure ({link_error}): {error}"
                        )
                    })?;
                if let Err(error) = record
                    .write_all(&bytes)
                    .and_then(|()| record.sync_all())
                {
                    drop(record);
                    let _ = std::fs::remove_file(record_path);
                    return Err(format!(
                        "write direct ownership restore after hard-link failure ({link_error}): {error}"
                    ));
                }
                Ok(())
            })();
            direct_restore?;
        }
        let _ = std::fs::remove_file(&temp_path);
        Ok(())
    })();
    if restore.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    restore
}

/// Clear the launch ownership artifacts after the caller has authoritatively
/// proven that the exact managed Chromium process tree has exited.
///
/// This function intentionally does not infer process-tree exit from a PID.
/// Its caller owns the exact process handle and cleanup proof. Under the same
/// per-profile OS lock used by launch and recovery, it revalidates the current
/// application identity and requires the on-disk marker to exactly equal the
/// opaque launch token. Any mismatch or unverifiable state preserves both
/// artifacts. A missing marker is an idempotent success and never authorizes
/// deletion of `DevToolsActivePort`.
pub(crate) fn cleanup_browser_ownership_after_exact_shutdown(
    token: &BrowserOwnershipToken,
) -> Result<(), String> {
    let operation_claim = ProfileOperationClaim::acquire(&token.profile_dir)?;
    cleanup_browser_ownership_under_claim(token, &operation_claim)
}

/// Remove one exact uncommitted ephemeral profile after the caller has proven
/// that its spawned process tree is absent.
///
/// No ownership marker exists in this phase, so authority comes from the
/// opaque canonical token captured under the original launch claim. Any marker
/// which appeared meanwhile makes the operation fail closed.
pub(crate) fn cleanup_uncommitted_ephemeral_profile_after_exact_shutdown(
    token: &EphemeralProfileCleanupToken,
) -> Result<(), String> {
    match std::fs::symlink_metadata(&token.profile_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "inspect uncommitted ephemeral browser profile: {error}"
            ));
        }
        Ok(_) => {}
    }
    let operation_claim = ProfileOperationClaim::acquire(&token.profile_dir)?;
    cleanup_uncommitted_ephemeral_profile_under_claim(token, &operation_claim)
}

pub(crate) fn cleanup_uncommitted_ephemeral_profile_after_exact_shutdown_under_launch_claim(
    token: &EphemeralProfileCleanupToken,
    launch_claim: &ProfileLaunchClaim,
) -> Result<(), String> {
    if token.profile_dir != launch_claim.0.profile_dir {
        return Err(
            "uncommitted ephemeral cleanup token belongs to a different launch claim".into(),
        );
    }
    match std::fs::symlink_metadata(&token.profile_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "inspect claimed uncommitted ephemeral browser profile: {error}"
            ));
        }
        Ok(_) => {}
    }
    launch_claim.0.validates(&token.profile_dir)?;
    cleanup_uncommitted_ephemeral_profile_under_claim(token, &launch_claim.0)
}

fn cleanup_uncommitted_ephemeral_profile_under_claim(
    token: &EphemeralProfileCleanupToken,
    operation_claim: &ProfileOperationClaim,
) -> Result<(), String> {
    operation_claim.validates(&token.profile_dir)?;
    if operation_claim.directory_identity()? != token.profile_identity {
        return Err(
            "uncommitted ephemeral cleanup directory identity changed".into(),
        );
    }
    let metadata = std::fs::symlink_metadata(&token.profile_dir)
        .map_err(|error| format!("inspect exact uncommitted browser profile: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("exact uncommitted browser profile is not a regular directory".into());
    }

    let current_records = read_ownership_record_set(&token.profile_dir)?;
    if current_records.active != token.provisional_marker
        || current_records.active_path != ownership_marker_path(&token.profile_dir)
        || current_records.provisional_predecessor.is_some()
    {
        return Err(
            "provisional ownership inventory changed before uncommitted cleanup".into(),
        );
    }
    // Do not pre-scan the complete profile here. An exact provisional record,
    // exact directory identity, held operation claim, and proven process-tree
    // exit are already the deletion authority. The bounded no-follow walker
    // below checks every child directory for nested ownership records before
    // deleting anything in that directory and keeps this root record across
    // continuation boundaries.
    remove_ephemeral_profile_contents_marker_last(
        &token.profile_dir,
        Some(&token.provisional_marker),
        token.profile_identity,
    )
}

/// Remove an explicitly ephemeral profile after the caller has proven exact
/// process-tree exit.
///
/// Unlike stable cleanup, this keeps the ownership marker inside the directory
/// until the same operation removes the whole profile. Therefore a cancelled
/// or failed removal remains discoverable by startup recovery.
pub(crate) fn cleanup_ephemeral_profile_after_exact_shutdown(
    token: &BrowserOwnershipToken,
    profile_dir: &Path,
) -> Result<(), String> {
    cleanup_ephemeral_profile_after_exact_shutdown_with_limits(
        token,
        profile_dir,
        EphemeralDeleteLimits::production(),
    )
}

fn cleanup_ephemeral_profile_after_exact_shutdown_with_limits(
    token: &BrowserOwnershipToken,
    profile_dir: &Path,
    limits: EphemeralDeleteLimits,
) -> Result<(), String> {
    match std::fs::symlink_metadata(profile_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "inspect ephemeral profile before exact cleanup: {error}"
            ));
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("ephemeral profile cleanup target is not a regular directory".into());
        }
        Ok(_) => {}
    }
    let operation_claim = ProfileOperationClaim::acquire(profile_dir)?;
    cleanup_ephemeral_profile_under_claim_with_limits(
        token,
        profile_dir,
        &operation_claim,
        limits,
    )
}

pub(crate) fn cleanup_ephemeral_profile_after_exact_shutdown_under_launch_claim(
    token: &BrowserOwnershipToken,
    profile_dir: &Path,
    launch_claim: &ProfileLaunchClaim,
) -> Result<(), String> {
    launch_claim.0.validates(profile_dir)?;
    cleanup_ephemeral_profile_under_claim(token, profile_dir, &launch_claim.0)
}

fn cleanup_ephemeral_profile_under_claim(
    token: &BrowserOwnershipToken,
    profile_dir: &Path,
    operation_claim: &ProfileOperationClaim,
) -> Result<(), String> {
    cleanup_ephemeral_profile_under_claim_with_limits(
        token,
        profile_dir,
        operation_claim,
        EphemeralDeleteLimits::production(),
    )
}

fn cleanup_ephemeral_profile_under_claim_with_limits(
    token: &BrowserOwnershipToken,
    profile_dir: &Path,
    operation_claim: &ProfileOperationClaim,
    limits: EphemeralDeleteLimits,
) -> Result<(), String> {
    operation_claim.validates(&token.profile_dir)?;
    if operation_claim.directory_identity()? != token.profile_identity {
        return Err("ephemeral cleanup directory identity changed".into());
    }
    validate_marker(&token.marker, &token.profile_dir)?;

    let current_marker = read_marker(profile_dir)?;
    if current_marker != token.marker {
        return Err(
            "ownership marker changed before ephemeral profile cleanup; profile preserved"
                .into(),
        );
    }
    let mut control = SystemProcessControl;
    let current_app = control.current_process()?;
    if current_marker.app_instance_id != managed_app_instance_id()
        || !same_process(&current_marker.owner_app, &current_app)
    {
        return Err(
            "ephemeral profile marker no longer belongs to this exact application instance"
                .into(),
        );
    }

    let canonical_profile = std::fs::canonicalize(profile_dir)
        .map_err(|error| format!("canonicalize ephemeral profile cleanup target: {error}"))?;
    if canonical_profile != token.profile_dir {
        return Err("ephemeral profile cleanup target changed before removal".into());
    }
    let expected_records = read_ownership_record_set(&canonical_profile)?;
    if expected_records.active != token.marker
        || expected_records.active_path != token.marker_path
    {
        return Err(
            "ephemeral ownership record paths changed before cleanup".into(),
        );
    }
    // A complete pre-scan used to make profiles with more than 100k entries
    // permanently undeletable. The deletion walker performs the same nested
    // ownership check directory-by-directory before mutation, while its root
    // record remains durable across bounded retries.
    remove_ephemeral_profile_contents_marker_last_with_limits(
        &canonical_profile,
        Some(&token.marker),
        token.profile_identity,
        limits,
    )
}

/// Delete browser profile contents while preserving the ownership marker until
/// every other entry is gone.
///
/// If deleting any browser artifact fails, the marker remains in place and
/// startup recovery can retry. Once the marker is removed, only deletion of the
/// now-empty directory remains; a failure at that final step cannot strand
/// browser state or an endpoint behind missing lineage.
#[cfg(not(windows))]
fn remove_ephemeral_profile_contents_marker_last(
    canonical_profile: &Path,
    expected_marker: Option<&BrowserOwnershipMarker>,
    expected_profile_identity: ProfileDirectoryIdentity,
) -> Result<(), String> {
    remove_ephemeral_profile_contents_marker_last_with_limits(
        canonical_profile,
        expected_marker,
        expected_profile_identity,
        EphemeralDeleteLimits::production(),
    )
}

#[cfg(not(windows))]
fn remove_ephemeral_profile_contents_marker_last_with_limits(
    canonical_profile: &Path,
    expected_marker: Option<&BrowserOwnershipMarker>,
    expected_profile_identity: ProfileDirectoryIdentity,
    limits: EphemeralDeleteLimits,
) -> Result<(), String> {
    remove_ephemeral_profile_contents_marker_last_unix(
        canonical_profile,
        expected_marker,
        expected_profile_identity,
        limits,
    )
}

#[cfg(windows)]
fn remove_ephemeral_profile_contents_marker_last(
    canonical_profile: &Path,
    expected_marker: Option<&BrowserOwnershipMarker>,
    expected_profile_identity: ProfileDirectoryIdentity,
) -> Result<(), String> {
    remove_ephemeral_profile_contents_marker_last_with_limits(
        canonical_profile,
        expected_marker,
        expected_profile_identity,
        EphemeralDeleteLimits::production(),
    )
}

#[cfg(windows)]
fn remove_ephemeral_profile_contents_marker_last_with_limits(
    canonical_profile: &Path,
    expected_marker: Option<&BrowserOwnershipMarker>,
    expected_profile_identity: ProfileDirectoryIdentity,
    limits: EphemeralDeleteLimits,
) -> Result<(), String> {
    let root_directory_guard =
        open_locked_non_reparse_directory_for_deletion(canonical_profile)?;
    let root_identity = windows_directory_identity(&root_directory_guard)?;
    if expected_profile_identity
        != (ProfileDirectoryIdentity {
            volume_serial_number: root_identity.volume_serial_number,
            file_index: root_identity.file_index,
        })
    {
        return Err(
            "Windows profile directory identity changed before deletion".into(),
        );
    }
    let expected_records = match expected_marker {
        Some(expected) => {
            let records = read_ownership_record_set(canonical_profile)?;
            if records.active != *expected {
                return Err(
                    "ownership marker changed before ephemeral profile deletion began".into(),
                );
            }
            Some(records)
        }
        None => {
            let (records, errors, _) = collect_marker_paths(canonical_profile);
            if let Some(error) = errors.into_iter().next() {
                return Err(format!(
                    "inspect unmarked ephemeral profile records: {error}"
                ));
            }
            if !records.is_empty() {
                return Err(
                    "ownership appeared before unmarked ephemeral profile deletion".into(),
                );
            }
            None
        }
    };
    let mut preserved_record_paths = expected_records
        .as_ref()
        .map(|records| {
            let mut paths = vec![records.active_path.clone()];
            if let Some((path, _)) = &records.provisional_predecessor {
                paths.push(path.clone());
            }
            paths
        })
        .unwrap_or_default();
    preserved_record_paths.sort();

    let mut delete_budget = EphemeralDeleteBudget::new(limits);
    for entry in std::fs::read_dir(canonical_profile)
        .map_err(|error| format!("read exact ephemeral browser profile: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("read exact ephemeral profile entry: {error}"))?;
        let entry_path = entry.path();
        if preserved_record_paths
            .iter()
            .any(|record| entry_path == *record)
        {
            continue;
        }
        if is_ownership_record_name(&entry.file_name()) {
            return Err(
                "unexpected ownership record appeared during ephemeral profile cleanup".into(),
            );
        }
        if !delete_budget.try_charge_windows_path(&entry_path)? {
            return Err(EPHEMERAL_DELETE_RETRY_REQUIRED.into());
        }
        if remove_windows_profile_entry_tree_no_follow(
            &entry_path,
            0,
            &mut delete_budget,
        )? == EphemeralDeleteProgress::MoreWork
        {
            return Err(EPHEMERAL_DELETE_RETRY_REQUIRED.into());
        }
    }

    // A non-cooperating writer may have added a nested marker while ordinary
    // browser data was being removed. Complete the no-follow scan again before
    // unlinking any durable lineage.
    let (mut remaining_records, scan_errors, _) =
        collect_marker_paths(canonical_profile);
    if let Some(error) = scan_errors.into_iter().next() {
        return Err(format!(
            "refusing marker-last cleanup after an incomplete final scan: {error}"
        ));
    }
    remaining_records.sort();
    if remaining_records != preserved_record_paths {
        return Err(
            "ownership record inventory changed before marker-last profile deletion".into(),
        );
    }

    if let (Some(expected_marker), Some(expected_records)) =
        (expected_marker, expected_records.as_ref())
    {
        let current = read_ownership_record_set(canonical_profile)?;
        if current.active != *expected_marker
            || current.active_path != expected_records.active_path
            || current.provisional_predecessor
                != expected_records.provisional_predecessor
        {
            return Err(
                "ownership marker changed before marker-last profile deletion".into(),
            );
        }

        // The committed sidecar is the last record removed. If cleanup is
        // interrupted after unlinking its provisional predecessor, exact
        // browser recovery remains possible from the sidecar alone.
        if let Some((provisional_path, provisional)) =
            &expected_records.provisional_predecessor
        {
            if read_marker_at(canonical_profile, provisional_path)? != *provisional {
                return Err(
                    "provisional ownership predecessor changed before removal".into(),
                );
            }
            std::fs::remove_file(provisional_path).map_err(|error| {
                format!("remove exact provisional ownership predecessor: {error}")
            })?;
        }
        if read_marker_at(canonical_profile, &expected_records.active_path)?
            != *expected_marker
        {
            return Err(
                "active ownership record changed before marker-last removal".into(),
            );
        }
        let metadata = std::fs::symlink_metadata(&expected_records.active_path)
            .map_err(|error| format!("inspect exact ephemeral ownership marker: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("exact ephemeral ownership marker is not a regular file".into());
        }
        std::fs::remove_file(&expected_records.active_path)
            .map_err(|error| format!("remove exact ephemeral ownership marker: {error}"))?;
    }
    #[cfg(test)]
    run_final_profile_rmdir_barrier_hook(canonical_profile);
    match delete_locked_empty_directory(&root_directory_guard) {
        Ok(()) => {
            // FileDispositionInfo binds deletion to the exact guarded file
            // identity. Closing the final handle completes removal without a
            // path-based rename/replacement window.
            drop(root_directory_guard);
            Ok(())
        }
        Err(error) => {
            // A concurrent new entry can make the final rmdir fail after the
            // last record was unlinked. The no-share-delete handle still pins
            // this exact directory and its ancestor namespace, so restoring
            // the exact active record cannot write through a replacement.
            let restore_error = if let (Some(expected), Some(records)) =
                (expected_marker, expected_records.as_ref())
            {
                if windows_directory_identity(&root_directory_guard).ok()
                    == Some(root_identity)
                {
                    create_exact_ownership_record_no_overwrite(
                        canonical_profile,
                        &records.active_path,
                        expected,
                    )
                    .err()
                } else {
                    Some(
                        "guarded browser profile identity changed before ownership restore"
                            .into(),
                    )
                }
            } else {
                None
            };
            match restore_error {
                Some(restore_error) => Err(format!(
                    "delete exact ephemeral browser profile by handle: {error}; ownership restore failed: {restore_error}"
                )),
                None => Err(format!(
                    "delete exact ephemeral browser profile by handle: {error}"
                )),
            }
        }
    }
}

#[cfg(windows)]
fn open_locked_non_reparse_directory(path: &Path) -> Result<std::fs::File, String> {
    use windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES;
    open_locked_non_reparse_directory_with_access(path, FILE_READ_ATTRIBUTES)
}

#[cfg(windows)]
fn open_locked_non_reparse_directory_for_deletion(
    path: &Path,
) -> Result<std::fs::File, String> {
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_READ_ATTRIBUTES,
    };
    open_locked_non_reparse_directory_with_access(
        path,
        FILE_READ_ATTRIBUTES | DELETE,
    )
}

#[cfg(windows)]
fn open_locked_non_reparse_directory_with_access(
    path: &Path,
    access: u32,
) -> Result<std::fs::File, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
    };

    let file = std::fs::OpenOptions::new()
        .access_mode(access)
        // Deliberately omit FILE_SHARE_DELETE: while this handle is alive the
        // directory path cannot be renamed, removed, or swapped for a junction.
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| format!("open exact browser profile directory handle: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect exact browser profile directory handle: {error}"))?;
    if !metadata.is_dir()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(
            "exact browser profile directory handle is a reparse point or non-directory".into(),
        );
    }
    Ok(file)
}

#[cfg(windows)]
fn delete_locked_empty_directory(
    directory: &std::fs::File,
) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle,
        FILE_DISPOSITION_INFO,
    };

    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: `directory` keeps the exact DELETE-capable handle alive and the
    // disposition buffer has the class-required layout and size.
    let succeeded = unsafe {
        SetFileInformationByHandle(
            directory.as_raw_handle(),
            FileDispositionInfo,
            std::ptr::addr_of!(disposition).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowsDirectoryIdentity {
    volume_serial_number: u32,
    file_index: u64,
}

#[cfg(windows)]
fn windows_directory_identity(
    directory: &std::fs::File,
) -> Result<WindowsDirectoryIdentity, String> {
    let (volume_serial_number, file_index, _) = windows_file_identity(directory)
        .map_err(|error| format!("inspect browser profile directory identity: {error}"))?;
    Ok(WindowsDirectoryIdentity {
        volume_serial_number,
        file_index,
    })
}

#[cfg(windows)]
fn windows_file_identity(
    file: &std::fs::File,
) -> std::io::Result<(u32, u64, u32)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` keeps the valid handle alive and `information` is a
    // correctly sized writable output buffer.
    let succeeded = unsafe {
        GetFileInformationByHandle(
            file.as_raw_handle(),
            &mut information,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow);
    Ok((
        information.dwVolumeSerialNumber,
        file_index,
        information.nNumberOfLinks,
    ))
}

fn capture_profile_directory_identity(
    path: &Path,
) -> Result<ProfileDirectoryIdentity, String> {
    #[cfg(windows)]
    {
        let directory = open_locked_non_reparse_directory(path)?;
        let identity = windows_directory_identity(&directory)?;
        return Ok(ProfileDirectoryIdentity {
            volume_serial_number: identity.volume_serial_number,
            file_index: identity.file_index,
        });
    }

    #[cfg(unix)]
    {
        let directory = UnixDirectory::open_path(path)?;
        Ok(ProfileDirectoryIdentity {
            device: directory.device,
            inode: directory.inode,
            mount_key: directory.mount_key,
            #[cfg(target_os = "linux")]
            mount_id: directory.mount_id,
        })
    }
}

#[cfg(unix)]
struct UnixDirectory {
    file: std::fs::File,
    device: u64,
    inode: u64,
    mount_key: [u8; 8],
    #[cfg(target_os = "linux")]
    mount_id: u64,
}

#[cfg(unix)]
struct UnixDirectoryStream(*mut libc::DIR);

#[cfg(unix)]
impl Drop for UnixDirectoryStream {
    fn drop(&mut self) {
        // SAFETY: this wrapper is only constructed from a successful
        // `fdopendir` and owns that DIR pointer exactly once.
        unsafe {
            libc::closedir(self.0);
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct UnixDirectoryEnumerationBudget {
    max_entries: usize,
    max_name_bytes: usize,
    entries: usize,
    name_bytes: usize,
}

#[cfg(unix)]
impl UnixDirectoryEnumerationBudget {
    fn new(max_entries: usize, max_name_bytes: usize) -> Self {
        Self {
            max_entries,
            max_name_bytes,
            entries: 0,
            name_bytes: 0,
        }
    }

    /// Reserve one name before it is copied out of libc's transient dirent.
    /// A failed charge leaves the budget unchanged so a caller can report the
    /// limit without accounting bytes it never retained.
    fn charge(&mut self, name: &[u8]) -> Result<(), String> {
        let next_entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| "Unix directory entry budget overflowed".to_string())?;
        if next_entries > self.max_entries {
            return Err(format!(
                "Unix directory enumeration exceeded {} entries",
                self.max_entries
            ));
        }
        let next_name_bytes = self
            .name_bytes
            .checked_add(name.len())
            .ok_or_else(|| "Unix directory name-byte budget overflowed".to_string())?;
        if next_name_bytes > self.max_name_bytes {
            return Err(format!(
                "Unix directory enumeration exceeded {} name bytes",
                self.max_name_bytes
            ));
        }
        self.entries = next_entries;
        self.name_bytes = next_name_bytes;
        Ok(())
    }
}

#[cfg(unix)]
impl UnixDirectory {
    fn open_path(path: &Path) -> Result<Self, String> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| format!("open exact Unix profile directory: {error}"))?;
        Self::from_file(file)
    }

    fn open_child(&self, name: &std::ffi::OsStr) -> Result<Self, String> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let name = unix_component_cstring(name)?;
        #[cfg(target_os = "linux")]
        let fd = openat2_unix_directory(self.file.as_raw_fd(), &name)?;
        #[cfg(not(target_os = "linux"))]
        // SAFETY: the parent fd and NUL-terminated component are live for this
        // call. O_NOFOLLOW and O_DIRECTORY bind the result to the exact child
        // directory entry without traversing a final symlink.
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY
                    | libc::O_CLOEXEC
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW,
            )
        };
        #[cfg(not(target_os = "linux"))]
        if fd < 0 {
            return Err(format!(
                "open exact Unix profile child: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: openat returned a new owned fd.
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Self::from_file(file)
    }

    fn from_file(file: std::fs::File) -> Result<Self, String> {
        use std::os::unix::fs::MetadataExt;
        let metadata = file
            .metadata()
            .map_err(|error| format!("inspect exact Unix profile directory: {error}"))?;
        if !metadata.is_dir() {
            return Err("exact Unix profile handle is not a directory".into());
        }
        Ok(Self {
            mount_key: unix_mount_key(&file)?,
            #[cfg(target_os = "linux")]
            mount_id: unix_mount_id(&file)?,
            file,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn identity(&self) -> ProfileDirectoryIdentity {
        ProfileDirectoryIdentity {
            device: self.device,
            inode: self.inode,
            mount_key: self.mount_key,
            #[cfg(target_os = "linux")]
            mount_id: self.mount_id,
        }
    }

    fn visit_entries(
        &self,
        mut visitor: impl FnMut(&[u8]) -> Result<bool, String>,
    ) -> Result<(), String> {
        use std::os::fd::AsRawFd;

        // fdopendir owns and closes the supplied descriptor. Opening "." via
        // openat creates a new open file description with an independent
        // directory offset; dup/fcntl would share the authoritative handle's
        // offset and make the second enumeration start at EOF.
        let dot = b".\0";
        // SAFETY: the parent fd and static NUL-terminated "." component live.
        let duplicate = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                dot.as_ptr().cast(),
                libc::O_RDONLY
                    | libc::O_CLOEXEC
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW,
            )
        };
        if duplicate < 0 {
            return Err(format!(
                "duplicate Unix profile directory handle: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: duplicate is a fresh directory fd owned by fdopendir.
        let directory = unsafe { libc::fdopendir(duplicate) };
        if directory.is_null() {
            // SAFETY: fdopendir did not take ownership on failure.
            unsafe {
                libc::close(duplicate);
            }
            return Err(format!(
                "enumerate Unix profile directory: {}",
                std::io::Error::last_os_error()
            ));
        }
        let directory = UnixDirectoryStream(directory);
        loop {
            // errno must be cleared to distinguish end-of-directory from an
            // enumeration error.
            set_unix_errno_zero();
            // SAFETY: directory remains live until closed below.
            let entry = unsafe { libc::readdir(directory.0) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error().unwrap_or(0) == 0 {
                    return Ok(());
                }
                return Err(format!("read Unix profile directory: {error}"));
            }
            // SAFETY: d_name is NUL-terminated for the live dirent.
            let bytes = unsafe {
                std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()).to_bytes()
            };
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if visitor(bytes)? {
                return Ok(());
            }
        }
    }

    fn entries_bounded(
        &self,
        budget: &mut UnixDirectoryEnumerationBudget,
    ) -> Result<Vec<std::ffi::OsString>, String> {
        use std::os::unix::ffi::OsStringExt;

        let mut entries = Vec::new();
        self.visit_entries(|bytes| {
            // Enforce both budgets while `bytes` still points at libc's
            // transient dirent and before allocating an owned name.
            budget.charge(bytes)?;
            entries.push(std::ffi::OsString::from_vec(bytes.to_vec()));
            Ok(false)
        })?;
        Ok(entries)
    }

    /// Copy one deletion candidate only after charging it to the current work
    /// budget. Enumerating and recursing one entry at a time prevents a wide
    /// row of non-empty sibling directories from consuming the whole budget
    /// before the first child can make progress.
    ///
    /// Ownership-record names are skipped for the exact profile root and fail
    /// closed in children. `exhausted=true` means the next ordinary entry could
    /// not be charged, so the root marker must remain for a later retry.
    fn next_deletion_entry(
        &self,
        budget: &mut EphemeralDeleteBudget,
        allow_root_ownership_records: bool,
    ) -> Result<(Option<std::ffi::OsString>, bool), String> {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let mut entry = None;
        let mut exhausted = false;
        self.visit_entries(|bytes| {
            let name = std::ffi::OsStr::from_bytes(bytes);
            if is_ownership_record_name(name) {
                if allow_root_ownership_records {
                    return Ok(false);
                }
                return Err(
                    "unexpected nested ownership record appeared during Unix profile cleanup"
                        .into(),
                );
            }
            if !budget.try_charge_unix_name(bytes)? {
                exhausted = true;
                return Ok(true);
            }
            entry = Some(std::ffi::OsString::from_vec(bytes.to_vec()));
            Ok(true)
        })?;
        Ok((entry, exhausted))
    }

    fn has_entries(&self) -> Result<bool, String> {
        let mut found = false;
        self.visit_entries(|_| {
            found = true;
            Ok(true)
        })?;
        Ok(found)
    }
}

#[cfg(target_os = "linux")]
fn unix_mount_id(file: &std::fs::File) -> Result<u64, String> {
    use std::os::fd::AsRawFd;
    // SAFETY: statx is a correctly sized writable output structure and an
    // empty pathname with AT_EMPTY_PATH queries the supplied live fd.
    let mut stat: libc::statx = unsafe { std::mem::zeroed() };
    let empty = b"\0";
    let result = unsafe {
        libc::statx(
            file.as_raw_fd(),
            empty.as_ptr().cast(),
            libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            &mut stat,
        )
    };
    if result != 0 {
        return Err(format!(
            "inspect Linux profile mount id: {}",
            std::io::Error::last_os_error()
        ));
    }
    if stat.stx_mask & libc::STATX_MNT_ID == 0 || stat.stx_mnt_id == 0 {
        return Err("Linux profile mount id is unavailable".into());
    }
    Ok(stat.stx_mnt_id)
}

#[cfg(unix)]
fn unix_mount_key(file: &std::fs::File) -> Result<[u8; 8], String> {
    use std::os::fd::AsRawFd;
    // SAFETY: statfs is a correctly sized writable output structure.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::fstatfs(file.as_raw_fd(), &mut stat) };
    if result != 0 {
        return Err(format!(
            "inspect Unix profile mount identity: {}",
            std::io::Error::last_os_error()
        ));
    }
    if std::mem::size_of_val(&stat.f_fsid) != 8 {
        return Err("unsupported Unix mount identity width".into());
    }
    let mut key = [0_u8; 8];
    // SAFETY: the size check above proves f_fsid fills the 8-byte output.
    unsafe {
        std::ptr::copy_nonoverlapping(
            std::ptr::addr_of!(stat.f_fsid).cast::<u8>(),
            key.as_mut_ptr(),
            key.len(),
        );
    }
    Ok(key)
}

#[cfg(all(unix, target_os = "linux"))]
fn set_unix_errno_zero() {
    // SAFETY: __errno_location returns this thread's errno slot.
    unsafe {
        *libc::__errno_location() = 0;
    }
}

#[cfg(all(unix, target_os = "macos"))]
fn set_unix_errno_zero() {
    // SAFETY: __error returns this thread's errno slot.
    unsafe {
        *libc::__error() = 0;
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn set_unix_errno_zero() {}

#[cfg(unix)]
fn unix_component_cstring(
    component: &std::ffi::OsStr,
) -> Result<std::ffi::CString, String> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = component.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err("invalid Unix profile path component".into());
    }
    std::ffi::CString::new(bytes)
        .map_err(|_| "Unix profile path component contains NUL".into())
}

#[cfg(target_os = "linux")]
fn openat2_unix_directory(
    parent_fd: std::os::fd::RawFd,
    name: &std::ffi::CStr,
) -> Result<std::os::fd::RawFd, String> {
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    const RESOLVE_NO_XDEV: u64 = 0x01;
    const RESOLVE_NO_SYMLINKS: u64 = 0x04;
    const RESOLVE_BENEATH: u64 = 0x08;
    let how = OpenHow {
        flags: (libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_DIRECTORY
            | libc::O_NOFOLLOW) as u64,
        mode: 0,
        resolve: RESOLVE_NO_XDEV | RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH,
    };
    // SAFETY: the parent fd/name are live and `how` has Linux open_how's
    // stable ABI layout. Unsupported kernels fail closed with ENOSYS.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent_fd,
            name.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd < 0 {
        Err(format!(
            "open exact Unix profile child without crossing mounts: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(fd as std::os::fd::RawFd)
    }
}

#[cfg(unix)]
fn unix_entry_stat(
    directory: &UnixDirectory,
    name: &std::ffi::OsStr,
) -> Result<libc::stat, String> {
    use std::os::fd::AsRawFd;
    let name = unix_component_cstring(name)?;
    // SAFETY: stat is a correctly sized output buffer and both fd/name live.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::fstatat(
            directory.file.as_raw_fd(),
            name.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        Ok(stat)
    } else {
        Err(format!(
            "inspect exact Unix profile entry: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(unix)]
fn same_unix_entry(
    stat: &libc::stat,
    directory: &UnixDirectory,
) -> bool {
    stat.st_dev as u64 == directory.device
        && stat.st_ino as u64 == directory.inode
}

#[cfg(unix)]
fn unlink_unix_entry(
    directory: &UnixDirectory,
    name: &std::ffi::OsStr,
    flags: libc::c_int,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let name = unix_component_cstring(name)?;
    // SAFETY: fd/name live and flags is either zero or AT_REMOVEDIR.
    let result = unsafe {
        libc::unlinkat(directory.file.as_raw_fd(), name.as_ptr(), flags)
    };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "remove exact Unix profile entry: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(unix)]
fn read_unix_marker_record(
    directory: &UnixDirectory,
    profile_dir: &Path,
    name: &std::ffi::OsStr,
) -> Result<BrowserOwnershipMarker, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;
    let name = unix_component_cstring(name)?;
    // SAFETY: parent fd/name live. O_NOFOLLOW rejects a marker symlink.
    let fd = unsafe {
        libc::openat(
            directory.file.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(format!(
            "open exact Unix ownership record: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: openat returned a new owned descriptor.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect exact Unix ownership record: {error}"))?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(
            "Unix ownership record is not a single-link regular file".into(),
        );
    }
    let bytes = read_ownership_record_bytes(
        &mut file,
        metadata.len(),
        "exact Unix ownership record",
    )?;
    let marker: BrowserOwnershipMarker = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse exact Unix ownership record: {error}"))?;
    validate_marker(&marker, profile_dir)?;
    Ok(marker)
}

#[cfg(unix)]
fn unix_entry_exists(
    directory: &UnixDirectory,
    name: &std::ffi::OsStr,
) -> Result<bool, String> {
    use std::os::fd::AsRawFd;
    let name = unix_component_cstring(name)?;
    // SAFETY: stat is a correctly sized output buffer and both fd/name live.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::fstatat(
            directory.file.as_raw_fd(),
            name.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(false)
    } else {
        Err(format!("inspect exact Unix ownership entry: {error}"))
    }
}

#[cfg(unix)]
fn create_unix_ownership_temp(
    directory: &UnixDirectory,
    name: &std::ffi::OsStr,
    bytes: &[u8],
) -> Result<(), String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = unix_component_cstring(name)?;
    // SAFETY: the directory fd/name live. O_EXCL provides the no-overwrite
    // barrier and O_NOFOLLOW rejects a racing symbolic link.
    let fd = unsafe {
        libc::openat(
            directory.file.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY
                | libc::O_CREAT
                | libc::O_EXCL
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(format!(
            "create exact Unix ownership temp: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: openat returned a new owned descriptor.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(bytes)
        .map_err(|error| format!("write exact Unix ownership temp: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("flush exact Unix ownership temp: {error}"))
}

#[cfg(unix)]
fn link_unix_entry_no_overwrite(
    directory: &UnixDirectory,
    source: &std::ffi::OsStr,
    destination: &std::ffi::OsStr,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let source = unix_component_cstring(source)?;
    let destination = unix_component_cstring(destination)?;
    // SAFETY: both names and the common exact directory fd remain live.
    // linkat never overwrites an existing destination.
    let result = unsafe {
        libc::linkat(
            directory.file.as_raw_fd(),
            source.as_ptr(),
            directory.file.as_raw_fd(),
            destination.as_ptr(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "commit exact Unix ownership record: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(unix)]
fn commit_ownership_marker_unix(
    profile_dir: &Path,
    directory: &UnixDirectory,
    marker: &BrowserOwnershipMarker,
    expected_existing: Option<&BrowserOwnershipMarker>,
) -> Result<PathBuf, String> {
    validate_marker(marker, profile_dir)?;
    let provisional_name = std::ffi::OsStr::new(OWNERSHIP_MARKER_FILE);
    let committed_name =
        std::ffi::OsStr::new(COMMITTED_OWNERSHIP_RECORD_FILE);
    let marker_name = if expected_existing.is_some() {
        committed_name
    } else {
        provisional_name
    };

    match expected_existing {
        Some(expected) => {
            validate_marker(expected, profile_dir)?;
            if *expected != provisional_predecessor(marker) {
                return Err(
                    "ownership record is not an exact provisional-to-committed transition"
                        .into(),
                );
            }
            if !unix_entry_exists(directory, provisional_name)?
                || read_unix_marker_record(
                    directory,
                    profile_dir,
                    provisional_name,
                )? != *expected
            {
                return Err(
                    "provisional ownership marker changed during browser spawn".into(),
                );
            }
            if unix_entry_exists(directory, committed_name)? {
                return Err(
                    "ownership record unexpectedly appeared during browser spawn".into(),
                );
            }
        }
        None => {
            if unix_entry_exists(directory, provisional_name)?
                || unix_entry_exists(directory, committed_name)?
            {
                return Err(
                    "ownership record unexpectedly appeared during browser spawn".into(),
                );
            }
        }
    }

    let temp_name = std::ffi::OsString::from(format!(
        "{OWNERSHIP_MARKER_FILE}.{}.{}.tmp",
        marker.app_instance_id, marker.browser.pid
    ));
    let marker_path = profile_dir.join(marker_name);
    let bytes = serde_json::to_vec_pretty(marker)
        .map_err(|error| format!("serialize ownership marker: {error}"))?;
    let write_result = (|| -> Result<PathBuf, String> {
        create_unix_ownership_temp(directory, &temp_name, &bytes)?;

        match expected_existing {
            Some(expected)
                if unix_entry_exists(directory, provisional_name)?
                    && read_unix_marker_record(
                        directory,
                        profile_dir,
                        provisional_name,
                    )? == *expected => {}
            Some(_) => {
                return Err(
                    "provisional ownership marker changed before append-only commit".into(),
                );
            }
            None => {
                if unix_entry_exists(directory, provisional_name)?
                    || unix_entry_exists(directory, committed_name)?
                {
                    return Err(
                        "ownership record appeared before atomic commit".into(),
                    );
                }
            }
        }

        link_unix_entry_no_overwrite(directory, &temp_name, marker_name)?;
        unlink_unix_entry(directory, &temp_name, 0)
            .map_err(|error| format!("remove exact Unix ownership temp: {error}"))?;
        directory
            .file
            .sync_all()
            .map_err(|error| format!("flush exact Unix ownership namespace: {error}"))?;

        #[cfg(test)]
        run_ownership_commit_barrier_hook(profile_dir, &marker_path);

        if read_unix_marker_record(directory, profile_dir, marker_name)?
            != *marker
        {
            return Err(
                "ownership record changed across fd-bound append-only commit".into(),
            );
        }
        match expected_existing {
            Some(expected)
                if read_unix_marker_record(
                    directory,
                    profile_dir,
                    provisional_name,
                )? == *expected => {}
            Some(_) => {
                return Err(
                    "committed ownership record lost its exact provisional predecessor"
                        .into(),
                );
            }
            None if unix_entry_exists(directory, committed_name)? => {
                return Err(
                    "unexpected committed ownership sidecar appeared after initial commit"
                        .into(),
                );
            }
            None => {}
        }
        Ok(marker_path.clone())
    })();
    if write_result.is_err() {
        let _ = unlink_unix_entry(directory, &temp_name, 0);
        let _ = directory.file.sync_all();
    }
    write_result
}

#[cfg(unix)]
fn remove_unix_profile_entry_tree(
    parent: &UnixDirectory,
    name: &std::ffi::OsStr,
    root_device: u64,
    depth: usize,
    budget: &mut EphemeralDeleteBudget,
) -> Result<EphemeralDeleteProgress, String> {
    if depth > MAX_EPHEMERAL_DELETE_DEPTH {
        return Err("Unix profile deletion exceeded the safe directory depth".into());
    }
    if is_ownership_record_name(name) {
        return Err(
            "unexpected nested ownership record appeared during Unix profile cleanup"
                .into(),
        );
    }
    let stat = unix_entry_stat(parent, name)?;
    let is_directory =
        stat.st_mode & libc::S_IFMT == libc::S_IFDIR;
    if !is_directory {
        unlink_unix_entry(parent, name, 0)?;
        return Ok(EphemeralDeleteProgress::Complete);
    }
    let child = parent.open_child(name)?;
    if child.device != root_device
        || child.mount_key != parent.mount_key
        || {
            #[cfg(target_os = "linux")]
            {
                child.mount_id != parent.mount_id
            }
            #[cfg(not(target_os = "linux"))]
            {
                false
            }
        }
        || !same_unix_entry(&stat, &child)
    {
        return Err(
            "Unix profile child crossed a mount or changed identity".into(),
        );
    }
    // Detect a nested managed profile before mutating any of its contents.
    // `fstatat(..., AT_SYMLINK_NOFOLLOW)` backs these exact-name checks, so a
    // marker symlink/directory also quarantines the subtree.
    for marker_name in [OWNERSHIP_MARKER_FILE, COMMITTED_OWNERSHIP_RECORD_FILE] {
        if unix_entry_exists(&child, std::ffi::OsStr::new(marker_name))? {
            return Err(
                "unexpected nested ownership record appeared during Unix profile cleanup"
                    .into(),
            );
        }
    }

    loop {
        let (child_name, exhausted) = child.next_deletion_entry(budget, false)?;
        if exhausted {
            return Ok(EphemeralDeleteProgress::MoreWork);
        }
        let Some(child_name) = child_name else {
            break;
        };
        if remove_unix_profile_entry_tree(
            &child,
            &child_name,
            root_device,
            depth.saturating_add(1),
            budget,
        )? == EphemeralDeleteProgress::MoreWork
        {
            return Ok(EphemeralDeleteProgress::MoreWork);
        }
    }
    if child.has_entries()? {
        return Err("Unix profile child changed before directory removal".into());
    }
    let current = unix_entry_stat(parent, name)?;
    if !same_unix_entry(&current, &child) {
        return Err("Unix profile child changed before unlinkat".into());
    }
    unlink_unix_entry(parent, name, libc::AT_REMOVEDIR)?;
    Ok(EphemeralDeleteProgress::Complete)
}

#[cfg(unix)]
fn restore_unix_record_relative(
    root: &UnixDirectory,
    name: &std::ffi::OsStr,
    marker: &BrowserOwnershipMarker,
) -> Result<(), String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = unix_component_cstring(name)?;
    let bytes = serde_json::to_vec_pretty(marker)
        .map_err(|error| format!("serialize Unix ownership restore: {error}"))?;
    // SAFETY: parent fd/name live. O_EXCL is the no-overwrite commit barrier.
    let fd = unsafe {
        libc::openat(
            root.file.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY
                | libc::O_CREAT
                | libc::O_EXCL
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(format!(
            "restore Unix ownership record: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: openat returned a new owned fd.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(&bytes)
        .map_err(|error| format!("write Unix ownership restore: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("flush Unix ownership restore: {error}"))
}

#[cfg(unix)]
fn remove_ephemeral_profile_contents_marker_last_unix(
    canonical_profile: &Path,
    expected_marker: Option<&BrowserOwnershipMarker>,
    expected_profile_identity: ProfileDirectoryIdentity,
    limits: EphemeralDeleteLimits,
) -> Result<(), String> {
    let parent_path = canonical_profile
        .parent()
        .ok_or_else(|| "Unix profile has no parent directory".to_string())?;
    let leaf = canonical_profile
        .file_name()
        .ok_or_else(|| "Unix profile has no final component".to_string())?;
    let parent = UnixDirectory::open_path(parent_path)?;
    let root = parent.open_child(leaf)?;
    let root_identity = ProfileDirectoryIdentity {
        device: root.device,
        inode: root.inode,
        mount_key: root.mount_key,
        #[cfg(target_os = "linux")]
        mount_id: root.mount_id,
    };
    if root_identity != expected_profile_identity {
        return Err("Unix profile directory identity changed before deletion".into());
    }
    if root.device != parent.device
        || root.mount_key != parent.mount_key
        || {
            #[cfg(target_os = "linux")]
            {
                root.mount_id != parent.mount_id
            }
            #[cfg(not(target_os = "linux"))]
            {
                false
            }
        }
    {
        return Err("Unix profile root is a mount point; cleanup refused".into());
    }
    let initial_root_stat = unix_entry_stat(&parent, leaf)?;
    if !same_unix_entry(&initial_root_stat, &root) {
        return Err("Unix profile identity changed before fd-relative cleanup".into());
    }

    let fixed_name = std::ffi::OsStr::new(OWNERSHIP_MARKER_FILE);
    let committed_name =
        std::ffi::OsStr::new(COMMITTED_OWNERSHIP_RECORD_FILE);
    // Ownership record names are fixed, so resolve them directly through the
    // pinned root fd. A wide root directory must not require a complete 100k
    // inventory before the first resumable deletion batch can make progress.
    let has_fixed = unix_entry_exists(&root, fixed_name)?;
    let has_committed = unix_entry_exists(&root, committed_name)?;

    let (active_name, predecessor) = match expected_marker {
        None => {
            if has_fixed || has_committed {
                return Err(
                    "ownership appeared before Unix unmarked profile cleanup".into(),
                );
            }
            (None, None)
        }
        Some(expected) if expected.phase == BrowserOwnershipPhase::Provisional => {
            if !has_fixed || has_committed {
                return Err("Unix provisional ownership inventory changed".into());
            }
            if read_unix_marker_record(&root, canonical_profile, fixed_name)?
                != *expected
            {
                return Err("Unix provisional ownership record changed".into());
            }
            (Some(fixed_name), None)
        }
        Some(expected) => {
            if has_committed {
                if read_unix_marker_record(
                    &root,
                    canonical_profile,
                    committed_name,
                )? != *expected
                {
                    return Err("Unix committed ownership sidecar changed".into());
                }
                let predecessor = if has_fixed {
                    let provisional =
                        read_unix_marker_record(&root, canonical_profile, fixed_name)?;
                    if provisional != provisional_predecessor(expected) {
                        return Err(
                            "Unix committed sidecar predecessor mismatched".into(),
                        );
                    }
                    Some((fixed_name, provisional))
                } else {
                    None
                };
                (Some(committed_name), predecessor)
            } else {
                if !has_fixed
                    || read_unix_marker_record(
                        &root,
                        canonical_profile,
                        fixed_name,
                    )? != *expected
                {
                    return Err("Unix stable ownership record changed".into());
                }
                (Some(fixed_name), None)
            }
        }
    };

    let mut delete_budget = EphemeralDeleteBudget::new(limits);
    loop {
        let (name, exhausted) = root.next_deletion_entry(&mut delete_budget, true)?;
        if exhausted {
            return Err(EPHEMERAL_DELETE_RETRY_REQUIRED.into());
        }
        let Some(name) = name else {
            break;
        };
        if remove_unix_profile_entry_tree(
            &root,
            &name,
            root.device,
            0,
            &mut delete_budget,
        )? == EphemeralDeleteProgress::MoreWork
        {
            return Err(EPHEMERAL_DELETE_RETRY_REQUIRED.into());
        }
    }

    let mut verification_budget = UnixDirectoryEnumerationBudget::new(
        MAX_POST_CLEANUP_ROOT_ENTRIES,
        MAX_POST_CLEANUP_ROOT_NAME_BYTES,
    );
    let remaining = root.entries_bounded(&mut verification_budget)?;
    if remaining.iter().any(|name| {
        name != fixed_name && name != committed_name
    }) {
        return Err(
            "Unix profile changed before marker-last cleanup".into(),
        );
    }
    if remaining.iter().any(|name| name == fixed_name) != has_fixed
        || remaining.iter().any(|name| name == committed_name) != has_committed
    {
        return Err("Unix ownership inventory changed before marker-last cleanup".into());
    }
    let current_root = unix_entry_stat(&parent, leaf)?;
    if !same_unix_entry(&current_root, &root) {
        return Err("Unix profile leaf changed before marker-last cleanup".into());
    }

    if let Some((name, expected)) = predecessor {
        if read_unix_marker_record(&root, canonical_profile, name)? != expected {
            return Err("Unix provisional predecessor changed before unlink".into());
        }
        unlink_unix_entry(&root, name, 0)?;
    }
    if let (Some(name), Some(expected)) = (active_name, expected_marker) {
        if read_unix_marker_record(&root, canonical_profile, name)? != *expected {
            return Err("Unix active ownership record changed before unlink".into());
        }
        unlink_unix_entry(&root, name, 0)?;
    }

    let restore_active = |error: String| -> Result<(), String> {
        if let (Some(name), Some(expected)) = (active_name, expected_marker) {
            if let Err(restore_error) = restore_unix_record_relative(&root, name, expected) {
                return Err(format!(
                    "{error}; ownership restore also failed: {restore_error}"
                ));
            }
        }
        Err(error)
    };

    let root_has_entries = match root.has_entries() {
        Ok(has_entries) => has_entries,
        Err(error) => {
            return restore_active(format!(
                "verify empty Unix profile before final unlinkat: {error}"
            ));
        }
    };
    if root_has_entries {
        return restore_active("Unix profile changed before final unlinkat".into());
    }
    let current_root = match unix_entry_stat(&parent, leaf) {
        Ok(stat) => stat,
        Err(error) => {
            return restore_active(format!(
                "verify Unix profile leaf before final unlinkat: {error}"
            ));
        }
    };
    if !same_unix_entry(&current_root, &root) {
        return restore_active("Unix profile leaf changed before final unlinkat".into());
    }
    match unlink_unix_entry(&parent, leaf, libc::AT_REMOVEDIR) {
        Ok(()) => Ok(()),
        Err(error) => restore_active(error),
    }
}

#[cfg(windows)]
fn windows_directory_contains_ownership_record(path: &Path) -> Result<bool, String> {
    for name in [OWNERSHIP_MARKER_FILE, COMMITTED_OWNERSHIP_RECORD_FILE] {
        match std::fs::symlink_metadata(path.join(name)) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "inspect nested Windows ownership record before deletion: {error}"
                ));
            }
        }
    }
    Ok(false)
}

/// Delete one Windows subtree in bounded, resumable steps. Every regular
/// directory is pinned with a non-reparse DELETE handle before traversal;
/// reparse points are removed as links and are never followed. A direct
/// ownership record quarantines the complete directory before any of its
/// children are touched.
#[cfg(windows)]
fn remove_windows_profile_entry_tree_no_follow(
    path: &Path,
    depth: usize,
    budget: &mut EphemeralDeleteBudget,
) -> Result<EphemeralDeleteProgress, String> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    if depth > MAX_EPHEMERAL_DELETE_DEPTH {
        return Err("ephemeral profile deletion exceeded the safe directory depth".into());
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect exact ephemeral profile entry: {error}"))?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        // Use single-entry unlink operations only. A path that races from a
        // directory reparse point to a non-empty regular directory therefore
        // fails instead of making a recursive path-based deletion cross the
        // no-follow boundary.
        if metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
            std::fs::remove_dir(path).map_err(|error| {
                format!("remove exact ephemeral profile directory reparse point: {error}")
            })?;
        } else {
            std::fs::remove_file(path).map_err(|error| {
                format!("remove exact ephemeral profile file reparse point: {error}")
            })?;
        }
        return Ok(EphemeralDeleteProgress::Complete);
    }
    if !metadata.is_dir() {
        std::fs::remove_file(path)
            .map_err(|error| format!("remove exact ephemeral profile file: {error}"))?;
        return Ok(EphemeralDeleteProgress::Complete);
    }

    let directory_guard = open_locked_non_reparse_directory_for_deletion(path)?;
    if windows_directory_contains_ownership_record(path)? {
        return Err(
            "unexpected nested ownership record appeared during Windows profile cleanup"
                .into(),
        );
    }

    for entry in std::fs::read_dir(path)
        .map_err(|error| format!("read exact Windows profile directory: {error}"))?
    {
        let entry = entry
            .map_err(|error| format!("read exact Windows profile directory entry: {error}"))?;
        if is_ownership_record_name(&entry.file_name()) {
            return Err(
                "unexpected nested ownership record appeared during Windows profile cleanup"
                    .into(),
            );
        }
        let child = entry.path();
        if !budget.try_charge_windows_path(&child)? {
            return Ok(EphemeralDeleteProgress::MoreWork);
        }
        if remove_windows_profile_entry_tree_no_follow(
            &child,
            depth.saturating_add(1),
            budget,
        )? == EphemeralDeleteProgress::MoreWork
        {
            return Ok(EphemeralDeleteProgress::MoreWork);
        }
    }

    delete_locked_empty_directory(&directory_guard)
        .map_err(|error| format!("delete exact empty Windows profile directory: {error}"))?;
    drop(directory_guard);
    Ok(EphemeralDeleteProgress::Complete)
}

/// Variant used by launch failures which occur while the original exclusive
/// launch claim is still held. Reusing that exact claim avoids a non-reentrant
/// attempt to lock the same profile while retaining the same validation and
/// fail-closed artifact rules as normal shutdown.
pub(crate) fn cleanup_browser_ownership_after_exact_shutdown_under_launch_claim(
    token: &BrowserOwnershipToken,
    launch_claim: &ProfileLaunchClaim,
) -> Result<(), String> {
    launch_claim.0.validates(&token.profile_dir)?;
    cleanup_browser_ownership_under_claim(token, &launch_claim.0)
}

fn cleanup_browser_ownership_under_claim(
    token: &BrowserOwnershipToken,
    operation_claim: &ProfileOperationClaim,
) -> Result<(), String> {
    operation_claim.validates(&token.profile_dir)?;
    if operation_claim.directory_identity()? != token.profile_identity {
        return Err("normal shutdown profile directory identity changed".into());
    }
    validate_marker(&token.marker, &token.profile_dir)?;

    match std::fs::symlink_metadata(&token.marker_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A prior idempotent cleanup may already have removed the exact
            // final record. Never use a different path as replacement
            // authority.
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "inspect exact ownership record during normal shutdown: {error}"
            ));
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(
                "exact ownership record is not a regular file during normal shutdown".into(),
            );
        }
        Ok(_) => {}
    }

    let current_records = read_ownership_record_set(&token.profile_dir)?;
    let current_marker = &current_records.active;
    if *current_marker != token.marker
        || current_records.active_path != token.marker_path
    {
        return Err(
            "ownership marker changed before normal shutdown cleanup; artifacts preserved".into(),
        );
    }

    let mut control = SystemProcessControl;
    let current_app = control.current_process()?;
    if current_marker.app_instance_id != managed_app_instance_id()
        || !same_process(&current_marker.owner_app, &current_app)
    {
        return Err(
            "ownership marker no longer belongs to this exact application instance".into(),
        );
    }

    remove_regular_devtools_active_port(&token.profile_dir, "normal shutdown")?;

    // Remove a provisional predecessor first. The exact committed sidecar
    // remains authoritative if the process crashes between the two unlinks.
    if let Some((path, expected)) = &current_records.provisional_predecessor {
        if read_marker_at(&token.profile_dir, path)? != *expected {
            return Err(
                "provisional predecessor changed during normal shutdown cleanup".into(),
            );
        }
        std::fs::remove_file(path)
            .map_err(|error| format!("remove completed provisional record: {error}"))?;
    }
    if read_marker_at(&token.profile_dir, &token.marker_path)? != token.marker {
        return Err("active ownership record changed during normal shutdown cleanup".into());
    }
    std::fs::remove_file(&token.marker_path)
        .map_err(|error| format!("remove completed ownership record: {error}"))
}

fn remove_regular_devtools_active_port(
    profile_dir: &Path,
    operation: &str,
) -> Result<(), String> {
    let port_path = profile_dir.join(DEVTOOLS_ACTIVE_PORT_FILE);
    match std::fs::symlink_metadata(&port_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "inspect DevToolsActivePort during {operation}: {error}"
            ));
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(
                format!("DevToolsActivePort is not a regular file during {operation}"),
            );
        }
        Ok(_) => {
            std::fs::remove_file(&port_path)
                .map_err(|error| format!("remove DevToolsActivePort: {error}"))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn collect_direct_unix_ownership_records(
    directory: &UnixDirectory,
    display_path: &Path,
    markers: &mut Vec<PathBuf>,
    errors: &mut Vec<String>,
    identities: &mut HashMap<PathBuf, ProfileDirectoryIdentity>,
    retained_path_budget: &mut RetainedPathBudget,
    directly_examined_paths: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    for marker_name in [OWNERSHIP_MARKER_FILE, COMMITTED_OWNERSHIP_RECORD_FILE] {
        let marker_name = std::ffi::OsStr::new(marker_name);
        if !unix_entry_exists(directory, marker_name)? {
            continue;
        }
        let marker_path = display_path.join(marker_name);
        if directly_examined_paths.contains(&marker_path) {
            continue;
        }
        let stat = unix_entry_stat(directory, marker_name)?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
            errors.push("ownership record name is occupied by a non-regular file".into());
            continue;
        }
        // Account separately for the marker vector, identity-map key, and the
        // duplicate path retained by the direct-probe de-duplication set.
        retained_path_budget.charge(&[
            marker_path.as_path(),
            display_path,
            marker_path.as_path(),
        ])?;
        directly_examined_paths.insert(marker_path.clone());
        markers.push(marker_path);
        identities
            .entry(display_path.to_path_buf())
            .or_insert(directory.identity());
    }
    Ok(())
}

#[cfg(unix)]
fn collect_marker_paths(
    root: &Path,
) -> (
    Vec<PathBuf>,
    Vec<String>,
    HashMap<PathBuf, ProfileDirectoryIdentity>,
) {
    let root_directory = match UnixDirectory::open_path(root) {
        Ok(directory) => directory,
        Err(error) => {
            if std::fs::symlink_metadata(root)
                .is_err_and(|io| io.kind() == std::io::ErrorKind::NotFound)
            {
                return (Vec::new(), Vec::new(), HashMap::new());
            }
            return (Vec::new(), vec![error], HashMap::new());
        }
    };
    let root_device = root_directory.device;
    let root_mount_key = root_directory.mount_key;
    #[cfg(target_os = "linux")]
    let root_mount_id = root_directory.mount_id;
    let mut markers: Vec<PathBuf> = Vec::new();
    let mut errors = Vec::new();
    let mut identities = HashMap::new();
    let mut retained_path_budget = RetainedPathBudget::new(
        MAX_MARKER_SCAN_RETAINED_PATH_BYTES,
        "browser profile marker retained paths",
    );
    let mut directly_examined_paths = HashSet::new();
    // Probe the two fixed authority names before copying a potentially huge
    // directory listing. This lets an already-open, marker-owned profile enter
    // bounded cleanup even when its root itself exceeds the scan ceiling.
    if let Err(error) = collect_direct_unix_ownership_records(
        &root_directory,
        root,
        &mut markers,
        &mut errors,
        &mut identities,
        &mut retained_path_budget,
        &mut directly_examined_paths,
    ) {
        errors.push(error);
        return (markers, errors, identities);
    }
    // Depth-first with one open fd per *ancestor* only. A frontier of sibling
    // fds over a wide Chromium profile tree could exhaust the default macOS
    // RLIMIT_NOFILE of 256 and turn every cleanup fail-closed.
    struct ScanFrame {
        directory: UnixDirectory,
        display_path: PathBuf,
        entries: Vec<std::ffi::OsString>,
        next: usize,
    }
    let scan_entry_limit = marker_scan_entry_limit();
    let mut enumeration_budget =
        UnixDirectoryEnumerationBudget::new(scan_entry_limit, MAX_MARKER_SCAN_NAME_BYTES);
    let root_entries = match root_directory.entries_bounded(&mut enumeration_budget) {
        Ok(entries) => entries,
        Err(error) => return (markers, vec![error], identities),
    };
    let mut stack = vec![ScanFrame {
        directory: root_directory,
        display_path: root.to_path_buf(),
        entries: root_entries,
        next: 0,
    }];
    let mut scanned_entries = 0_usize;
    while let Some(frame) = stack.last_mut() {
        if frame.next >= frame.entries.len() {
            stack.pop();
            continue;
        }
        let name = std::mem::take(&mut frame.entries[frame.next]);
        frame.next += 1;
        scanned_entries = scanned_entries.saturating_add(1);
        if scanned_entries > scan_entry_limit {
            errors.push(format!(
                "browser profile marker scan exceeded {scan_entry_limit} entries"
            ));
            markers.sort_by_key(|path| {
                std::cmp::Reverse(path.components().count())
            });
            return (markers, errors, identities);
        }
        let stat = match unix_entry_stat(&frame.directory, &name) {
            Ok(stat) => stat,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        let path = frame.display_path.join(&name);
        let kind = stat.st_mode & libc::S_IFMT;
        if is_ownership_record_name(&name) {
            if directly_examined_paths.contains(&path) {
                continue;
            }
            if kind == libc::S_IFREG {
                if let Err(error) = retained_path_budget
                    .charge(&[path.as_path(), frame.display_path.as_path()])
                {
                    errors.push(error);
                    markers.sort_by_key(|path| {
                        std::cmp::Reverse(path.components().count())
                    });
                    return (markers, errors, identities);
                }
                markers.push(path);
                identities
                    .entry(frame.display_path.clone())
                    .or_insert(frame.directory.identity());
            } else {
                errors.push(
                    "ownership record name is occupied by a non-regular file".into(),
                );
            }
            continue;
        }
        if kind != libc::S_IFDIR {
            continue;
        }
        if stack.len() >= MAX_MARKER_SCAN_DEPTH {
            errors.push(format!(
                "browser profile marker scan exceeded depth {MAX_MARKER_SCAN_DEPTH}"
            ));
            continue;
        }
        let frame_directory = &stack
            .last()
            .expect("scan frame stays on the stack while its entry is processed")
            .directory;
        match frame_directory.open_child(&name) {
            Ok(child)
                if child.device == root_device
                    && child.mount_key == root_mount_key
                    && {
                        #[cfg(target_os = "linux")]
                        {
                            child.mount_id == root_mount_id
                        }
                        #[cfg(not(target_os = "linux"))]
                        {
                            true
                        }
                    } =>
            {
                if let Err(error) = collect_direct_unix_ownership_records(
                    &child,
                    &path,
                    &mut markers,
                    &mut errors,
                    &mut identities,
                    &mut retained_path_budget,
                    &mut directly_examined_paths,
                ) {
                    errors.push(error);
                    markers.sort_by_key(|path| {
                        std::cmp::Reverse(path.components().count())
                    });
                    return (markers, errors, identities);
                }
                match child.entries_bounded(&mut enumeration_budget) {
                    Ok(entries) => stack.push(ScanFrame {
                        directory: child,
                        display_path: path,
                        entries,
                        next: 0,
                    }),
                    Err(error) => errors.push(error),
                }
            }
            Ok(_) | Err(_) => {
                errors.push(
                    "browser profile scan refused a mount boundary or changed directory"
                        .into(),
                );
            }
        }
    }
    markers.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    (markers, errors, identities)
}

#[cfg(windows)]
fn collect_direct_windows_ownership_records(
    directory: &Path,
    markers: &mut Vec<PathBuf>,
    errors: &mut Vec<String>,
    identities: &mut HashMap<PathBuf, ProfileDirectoryIdentity>,
    retained_path_budget: &mut RetainedPathBudget,
    directly_examined_paths: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    for marker_name in [OWNERSHIP_MARKER_FILE, COMMITTED_OWNERSHIP_RECORD_FILE] {
        let marker_path = directory.join(marker_name);
        let metadata = match std::fs::symlink_metadata(&marker_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "inspect direct Windows ownership record: {error}"
                ));
            }
        };
        if directly_examined_paths.contains(&marker_path) {
            continue;
        }
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            errors.push("ownership record name is occupied by a non-regular file".into());
            continue;
        }
        // Account separately for the marker vector, identity-map key, and the
        // duplicate path retained by the direct-probe de-duplication set.
        retained_path_budget.charge(&[
            marker_path.as_path(),
            directory,
            marker_path.as_path(),
        ])?;
        directly_examined_paths.insert(marker_path.clone());
        markers.push(marker_path);
        match capture_profile_directory_identity(directory) {
            Ok(identity) => {
                identities.entry(directory.to_path_buf()).or_insert(identity);
            }
            Err(error) => errors.push(error),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn collect_marker_paths(
    root: &Path,
) -> (
    Vec<PathBuf>,
    Vec<String>,
    HashMap<PathBuf, ProfileDirectoryIdentity>,
) {
    let mut markers = Vec::new();
    let mut errors = Vec::new();
    let mut identities = HashMap::new();
    let mut retained_path_budget = RetainedPathBudget::new(
        MAX_MARKER_SCAN_RETAINED_PATH_BYTES,
        "browser profile marker retained paths",
    );
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (markers, errors, identities);
        }
        Err(error) => {
            errors.push(format!("inspect browser profile recovery root: {error}"));
            return (markers, errors, identities);
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            errors.push("browser profile recovery root is not a regular directory".into());
            return (markers, errors, identities);
        }
        Ok(_) => {}
    }
    let mut pending_path_budget = RetainedPathBudget::new(
        MAX_MARKER_SCAN_PENDING_PATH_BYTES,
        "browser profile marker pending frontier",
    );
    if let Err(error) = pending_path_budget.charge(&[root]) {
        errors.push(error);
        return (markers, errors, identities);
    }
    let mut pending = vec![root.to_path_buf()];
    let mut directly_examined_paths = HashSet::new();
    let scan_entry_limit = marker_scan_entry_limit();
    let mut scanned_entries = 0_usize;
    while let Some(directory) = pending.pop() {
        if let Err(error) = pending_path_budget.release(&directory) {
            errors.push(error);
            markers.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
            return (markers, errors, identities);
        }
        // Marker names are fixed and cheap to probe. Discover exact authority
        // before a wide directory consumes the traversal budget, while the
        // later pinned operation claim still verifies path and file identity.
        if let Err(error) = collect_direct_windows_ownership_records(
            &directory,
            &mut markers,
            &mut errors,
            &mut identities,
            &mut retained_path_budget,
            &mut directly_examined_paths,
        ) {
            errors.push(error);
            markers.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
            return (markers, errors, identities);
        }
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                errors.push(format!("scan browser profile directory: {error}"));
                continue;
            }
        };
        for entry in entries {
            scanned_entries = scanned_entries.saturating_add(1);
            if scanned_entries > scan_entry_limit {
                errors.push(format!(
                    "browser profile marker scan exceeded {scan_entry_limit} entries"
                ));
                markers.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
                return (markers, errors, identities);
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(format!("read browser profile directory entry: {error}"));
                    continue;
                }
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    errors.push(format!("read browser profile entry type: {error}"));
                    continue;
                }
            };
            if is_ownership_record_name(&entry.file_name()) {
                if directly_examined_paths.contains(&entry.path()) {
                    continue;
                }
                if file_type.is_file() && !file_type.is_symlink() {
                    let marker_path = entry.path();
                    if let Err(error) = retained_path_budget
                        .charge(&[marker_path.as_path(), directory.as_path()])
                    {
                        errors.push(error);
                        markers.sort_by_key(|path| {
                            std::cmp::Reverse(path.components().count())
                        });
                        return (markers, errors, identities);
                    }
                    markers.push(marker_path);
                    match capture_profile_directory_identity(&directory) {
                        Ok(identity) => {
                            identities
                                .entry(directory.clone())
                                .or_insert(identity);
                        }
                        Err(error) => errors.push(error),
                    }
                } else {
                    errors.push(
                        "ownership record name is occupied by a non-regular file".into(),
                    );
                }
                continue;
            }
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                let child = entry.path();
                if let Err(error) = pending_path_budget.charge(&[child.as_path()]) {
                    errors.push(error);
                    markers.sort_by_key(|path| {
                        std::cmp::Reverse(path.components().count())
                    });
                    return (markers, errors, identities);
                }
                pending.push(child);
            }
        }
    }
    markers.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    (markers, errors, identities)
}

fn cleanup_recovered_profile(
    canonical_recovery_root: &Path,
    operation_claim: &ProfileOperationClaim,
    mode: ProfileRecoveryMode,
    expected: &BrowserOwnershipMarker,
) -> Result<(), String> {
    operation_claim.validates(&operation_claim.profile_dir)?;
    let canonical_profile = operation_claim.profile_dir.clone();
    if !canonical_profile.starts_with(canonical_recovery_root) {
        return Err("refusing to clean a profile outside the canonical recovery root".into());
    }
    match mode {
        ProfileRecoveryMode::DeleteEphemeralProfile => {
            if canonical_profile == canonical_recovery_root {
                return Err("refusing to remove a profile outside the ephemeral recovery root".into());
            }
            let metadata = std::fs::symlink_metadata(&canonical_profile)
                .map_err(|error| format!("inspect recovered ephemeral profile: {error}"))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("recovered ephemeral profile is not a regular directory".into());
            }
            let records = read_ownership_record_set(&canonical_profile)?;
            if records.active != *expected {
                return Err(
                    "recovered ownership record changed before recursive cleanup".into(),
                );
            }
            // Exact recovery has already pinned the ownership record and
            // proven the browser tree absent. Let the bounded no-follow
            // walker discover nested ownership directory-by-directory so an
            // oversized, otherwise valid profile can make resumable progress.
            let expected_profile_identity =
                operation_claim.directory_identity()?;
            #[cfg(windows)]
            operation_claim.release_profile_guard_for_directory_removal()?;
            let continuation_started = std::time::Instant::now();
            for attempt in 1..=MAX_RECOVERY_DELETE_CONTINUATION_ATTEMPTS {
                match remove_ephemeral_profile_contents_marker_last(
                    &canonical_profile,
                    Some(expected),
                    expected_profile_identity,
                ) {
                    Ok(()) => return Ok(()),
                    Err(error) if error == EPHEMERAL_DELETE_RETRY_REQUIRED => {
                        if attempt == MAX_RECOVERY_DELETE_CONTINUATION_ATTEMPTS
                            || continuation_started.elapsed()
                                >= MAX_RECOVERY_DELETE_CONTINUATION_TIME
                        {
                            return Err(format!(
                                "{EPHEMERAL_DELETE_RETRY_REQUIRED}; startup recovery continuation budget exhausted"
                            ));
                        }
                        // Each pass re-acquires the exact root handle inside
                        // the marker-last remover. Revalidate the canonical
                        // path and file identity before the next bounded pass.
                        operation_claim.validates(&canonical_profile)?;
                        if operation_claim.directory_identity()?
                            != expected_profile_identity
                        {
                            return Err(
                                "recovered ephemeral profile identity changed between deletion batches"
                                    .into(),
                            );
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            unreachable!("the bounded startup recovery loop always returns")
        }
        ProfileRecoveryMode::PreserveStableProfile => {
            let records = read_ownership_record_set(&canonical_profile)?;
            if records.active != *expected {
                return Err("stable ownership changed before recovery cleanup".into());
            }
            remove_regular_devtools_active_port(&canonical_profile, "startup recovery")?;
            let current = read_ownership_record_set(&canonical_profile)?;
            if current.active != *expected
                || current.active_path != records.active_path
                || current.provisional_predecessor != records.provisional_predecessor
            {
                return Err("stable ownership changed before marker-last cleanup".into());
            }
            if let Some((path, predecessor)) = &records.provisional_predecessor {
                if read_marker_at(&canonical_profile, path)? != *predecessor {
                    return Err(
                        "stable provisional predecessor changed before cleanup".into(),
                    );
                }
                std::fs::remove_file(path).map_err(|error| {
                    format!("clear recovered stable provisional record: {error}")
                })?;
            }
            if read_marker_at(&canonical_profile, &records.active_path)? != *expected {
                return Err(
                    "stable active ownership record changed before cleanup".into(),
                );
            }
            std::fs::remove_file(&records.active_path)
                .map_err(|error| format!("clear recovered stable ownership record: {error}"))
        }
    }
}

/// An initial marker discovery may hit the scan ceiling inside a very large
/// marker-owned profile after already discovering its exact root record. Give
/// bounded cleanup a chance to remove that profile, then count only scan
/// errors which still reproduce. This keeps the startup report fail-closed for
/// genuinely unresolved trees without permanently degrading on stale errors
/// from a profile that was safely removed in the same invocation.
fn record_unresolved_recovery_scan_errors(
    recovery_root: &Path,
    initial_scan_errors: &[String],
    report: &mut ProfileRecoveryReport,
) {
    if initial_scan_errors.is_empty() {
        return;
    }
    let (_, unresolved, _) = collect_marker_paths(recovery_root);
    for error in unresolved {
        report.failures += 1;
        tracing::warn!(
            target: "nomi_browser_engine::profile",
            reason = "recovery_scan_incomplete",
            "browser orphan recovery scan remained incomplete after bounded cleanup; affected profiles were preserved"
        );
        let _ = error;
    }
}

/// Recover marker-owned browser processes below one application-owned root.
///
/// This function never uses directory mtime. Unmarked, malformed, live-owner,
/// PID-reused, permission-denied, and unconfirmed profiles are preserved.
pub fn recover_owned_profiles(
    recovery_root: &Path,
    mode: ProfileRecoveryMode,
) -> ProfileRecoveryReport {
    let mut control = SystemProcessControl;
    recover_owned_profiles_with(recovery_root, mode, &mut control)
}

fn recover_provisional_profile(
    recovery_root: &Path,
    profile_dir: &Path,
    marker: &BrowserOwnershipMarker,
    operation_claim: &ProfileOperationClaim,
    #[cfg(windows)] authority: PinnedOwnershipRecordSet,
    mode: ProfileRecoveryMode,
    control: &mut dyn ProcessControl,
    report: &mut ProfileRecoveryReport,
) {
    #[cfg(not(windows))]
    {
        if mode != ProfileRecoveryMode::DeleteEphemeralProfile {
            report.failures += 1;
            report.profiles_preserved += 1;
            tracing::warn!(
                target: "nomi_browser_engine::profile",
                reason = "provisional_stable_profile_preserved",
                "provisional ownership can never authorize stable profile deletion"
            );
            return;
        }
        if operation_claim.validates(profile_dir).is_err() {
            report.failures += 1;
            report.profiles_preserved += 1;
            return;
        }
        // Unix has no pinned authority handles; re-resolve the records under
        // the held claim and fail closed on any change.
        match read_ownership_record_set(profile_dir) {
            Ok(records) if records.active == *marker => {}
            Ok(_) | Err(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                return;
            }
        }
        // Recheck after acquiring the profile claim. A Missing -> Found race,
        // PID reuse, or unverifiable snapshot always preserves the profile.
        match control.lookup(marker.owner_app.pid) {
            ProcessLookup::Missing => {}
            ProcessLookup::Found(_) | ProcessLookup::Unverified(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                return;
            }
        }
        match control.confirm_tree_absent(&marker.owner_app) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                return;
            }
        }
        // Cleanup re-resolves the exact record set under the still-claimed
        // profile directory and fails closed on change.
        match cleanup_recovered_profile(
            recovery_root,
            operation_claim,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            marker,
        ) {
            Ok(()) => report.ephemeral_profiles_removed += 1,
            Err(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
            }
        }
    }

    #[cfg(windows)]
    {
        if mode != ProfileRecoveryMode::DeleteEphemeralProfile {
            report.failures += 1;
            report.profiles_preserved += 1;
            tracing::warn!(
                target: "nomi_browser_engine::profile",
                reason = "provisional_stable_profile_preserved",
                "provisional ownership can never authorize stable profile deletion"
            );
            return;
        }
        if operation_claim.validates(profile_dir).is_err() {
            report.failures += 1;
            report.profiles_preserved += 1;
            return;
        }
        if authority.marker() != marker {
            report.failures += 1;
            report.profiles_preserved += 1;
            return;
        }
        // Recheck after acquiring the profile claim. A Missing -> Found race,
        // PID reuse, or unverifiable snapshot always preserves the profile.
        match control.lookup(marker.owner_app.pid) {
            ProcessLookup::Missing => {}
            ProcessLookup::Found(_) | ProcessLookup::Unverified(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                return;
            }
        }
        match control.confirm_tree_absent(&marker.owner_app) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                return;
            }
        }
        // Release the immutable authority file only after every process-tree
        // absence proof. Cleanup immediately re-resolves the exact record set
        // under the still-pinned profile directory and fails closed on change.
        drop(authority);
        match cleanup_recovered_profile(
            recovery_root,
            operation_claim,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            marker,
        ) {
            Ok(()) => report.ephemeral_profiles_removed += 1,
            Err(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
            }
        }
    }
}

#[cfg(not(windows))]
fn recover_owned_profiles_with(
    recovery_root: &Path,
    mode: ProfileRecoveryMode,
    control: &mut dyn ProcessControl,
) -> ProfileRecoveryReport {
    let mut report = ProfileRecoveryReport::default();
    let canonical_recovery_root = match std::fs::canonicalize(recovery_root) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return report,
        Err(_) => {
            report.failures += 1;
            return report;
        }
    };
    let (markers, scan_errors, scanned_profile_identities) =
        collect_marker_paths(recovery_root);
    report.markers_scanned = markers.len();

    let mut processed_profiles = HashSet::new();
    for marker_path in markers {
        let Some(profile_dir) = marker_path.parent() else {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        };
        if !processed_profiles.insert(profile_dir.to_path_buf()) {
            continue;
        }
        let Some(scanned_profile_identity) =
            scanned_profile_identities.get(profile_dir).copied()
        else {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        };
        let operation_claim = match ProfileOperationClaim::acquire(profile_dir) {
            Ok(claim) => claim,
            Err(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                let _ = error;
                continue;
            }
        };
        if operation_claim.profile_dir == canonical_recovery_root
            || !operation_claim
                .profile_dir
                .starts_with(&canonical_recovery_root)
        {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        }
        let claimed_profile_dir = operation_claim.profile_dir.clone();
        // The fd-relative scan identity must match the claimed directory:
        // a directory swapped since traversal never authorizes any process
        // action or cleanup.
        if operation_claim.directory_identity().ok()
            != Some(scanned_profile_identity)
        {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        }
        let records = match read_ownership_record_set(&claimed_profile_dir) {
            Ok(records) => records,
            Err(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    reason = "invalid_ownership_marker",
                    "invalid browser ownership marker; profile preserved"
                );
                let _ = error;
                continue;
            }
        };
        if operation_claim.validates(&claimed_profile_dir).is_err() {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        }
        let marker = records.active.clone();

        let owner_was_missing = match control.lookup(marker.owner_app.pid) {
            ProcessLookup::Found(observed) if same_process(&marker.owner_app, &observed) => {
                report.live_owners_preserved += 1;
                tracing::info!(
                    target: "nomi_browser_engine::profile",
                    owner_pid = marker.owner_app.pid,
                    reason = "live_verified_owner",
                    "browser profile belongs to a live verified app instance; recovery skipped"
                );
                continue;
            }
            ProcessLookup::Unverified(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    owner_pid = marker.owner_app.pid,
                    reason = "owner_identity_unverified",
                    "browser owner identity could not be verified; profile preserved"
                );
                let _ = error;
                continue;
            }
            ProcessLookup::Missing => true,
            ProcessLookup::Found(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    reason = "owner_pid_reused",
                    "browser owner PID was reused by a different identity; profile preserved"
                );
                continue;
            }
        };
        if marker.phase == BrowserOwnershipPhase::Provisional {
            if owner_was_missing {
                recover_provisional_profile(
                    &canonical_recovery_root,
                    &claimed_profile_dir,
                    &marker,
                    &operation_claim,
                    mode,
                    control,
                    &mut report,
                );
            } else {
                unreachable!("only a missing owner reaches provisional recovery");
            }
            continue;
        }

        let tree_absent = match control.lookup(marker.browser.pid) {
            ProcessLookup::Found(observed) if same_process(&marker.browser, &observed) => {
                // The verified owner is dead but its exact managed browser
                // tree survived. Unix has no Job Object tree-exit guarantee,
                // so startup recovery terminates the verified process group
                // and then requires the standard absence proof.
                match control
                    .terminate_tree(&marker.browser)
                    .and_then(|()| control.confirm_tree_absent(&marker.browser))
                {
                    Ok(true) => {
                        report.process_trees_terminated += 1;
                        tracing::info!(
                            target: "nomi_browser_engine::profile",
                            browser_pid = marker.browser.pid,
                            reason = "dead_owner_tree_terminated",
                            "terminated a verified orphan browser tree whose owning app instance is gone"
                        );
                        true
                    }
                    Ok(false) => {
                        report.failures += 1;
                        report.profiles_preserved += 1;
                        tracing::warn!(
                            target: "nomi_browser_engine::profile",
                            browser_pid = marker.browser.pid,
                            reason = "terminated_tree_absence_unconfirmed",
                            "orphan browser tree was signalled but its exit is not proven; profile preserved"
                        );
                        false
                    }
                    Err(error) => {
                        report.failures += 1;
                        report.profiles_preserved += 1;
                        tracing::warn!(
                            target: "nomi_browser_engine::profile",
                            browser_pid = marker.browser.pid,
                            reason = "orphan_tree_termination_failed",
                            "orphan browser tree could not be terminated safely; profile preserved"
                        );
                        let _ = error;
                        false
                    }
                }
            }
            ProcessLookup::Found(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    browser_pid = marker.browser.pid,
                    reason = "browser_pid_reused",
                    "browser PID was reused by a different process identity; startup recovery preserved the profile"
                );
                false
            }
            ProcessLookup::Missing => {
                match control.confirm_tree_absent(&marker.browser) {
                    Ok(true) => true,
                    Ok(false) => {
                        report.failures += 1;
                        report.profiles_preserved += 1;
                        tracing::warn!(
                            target: "nomi_browser_engine::profile",
                            browser_pid = marker.browser.pid,
                            reason = "tree_absence_unconfirmed",
                            "browser root changed or disappeared but its tree is not proven absent; profile preserved"
                        );
                        false
                    }
                    Err(error) => {
                        report.failures += 1;
                        report.profiles_preserved += 1;
                        tracing::warn!(
                            target: "nomi_browser_engine::profile",
                            browser_pid = marker.browser.pid,
                            reason = "tree_absence_check_failed",
                            "browser tree absence could not be verified; profile preserved"
                        );
                        let _ = error;
                        false
                    }
                }
            }
            ProcessLookup::Unverified(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    browser_pid = marker.browser.pid,
                    reason = "browser_identity_unverified",
                    "orphan browser identity could not be verified; profile preserved"
                );
                let _ = error;
                false
            }
        };
        if !tree_absent {
            continue;
        }

        // Unix has no deny-write record handles; re-resolve the exact record
        // set after every process action instead and fail closed on change.
        match read_ownership_record_set(&claimed_profile_dir) {
            Ok(current)
                if current.active == marker
                    && current.active_path == records.active_path
                    && current.provisional_predecessor
                        == records.provisional_predecessor => {}
            Ok(_) | Err(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                continue;
            }
        }

        if let Err(error) = cleanup_recovered_profile(
            &canonical_recovery_root,
            &operation_claim,
            mode,
            &marker,
        )
        {
            report.failures += 1;
            report.profiles_preserved += 1;
            tracing::warn!(
                target: "nomi_browser_engine::profile",
                recovery_mode = ?mode,
                reason = "profile_cleanup_failed",
                "browser process tree is absent but profile cleanup failed; profile preserved"
            );
            let _ = error;
            continue;
        }
        match mode {
            ProfileRecoveryMode::DeleteEphemeralProfile => {
                report.ephemeral_profiles_removed += 1;
            }
            ProfileRecoveryMode::PreserveStableProfile => {
                report.stable_markers_cleared += 1;
            }
        }
    }
    record_unresolved_recovery_scan_errors(recovery_root, &scan_errors, &mut report);
    report
}

#[cfg(windows)]
fn recover_owned_profiles_with(
    recovery_root: &Path,
    mode: ProfileRecoveryMode,
    control: &mut dyn ProcessControl,
) -> ProfileRecoveryReport {
    let mut report = ProfileRecoveryReport::default();
    let canonical_recovery_root = match std::fs::canonicalize(recovery_root) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return report,
        Err(_) => {
            report.failures += 1;
            return report;
        }
    };
    let (markers, scan_errors, scanned_profile_identities) =
        collect_marker_paths(recovery_root);
    report.markers_scanned = markers.len();

    let mut processed_profiles = HashSet::new();
    for marker_path in markers {
        let Some(profile_dir) = marker_path.parent() else {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        };
        if !processed_profiles.insert(profile_dir.to_path_buf()) {
            continue;
        }
        let Some(scanned_profile_identity) =
            scanned_profile_identities.get(profile_dir).copied()
        else {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        };
        // Establish the filesystem capability boundary before *any* process
        // lookup, absence proof, or termination. A scanned ancestor may have
        // been swapped for a junction/symlink since traversal; the held claim's
        // canonical final directory must remain a strict descendant of the
        // fixed canonical recovery root.
        let operation_claim = match ProfileOperationClaim::acquire_pinned(profile_dir) {
            Ok(claim) => claim,
            Err(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                let _ = error;
                continue;
            }
        };
        if operation_claim.profile_dir == canonical_recovery_root
            || !operation_claim
                .profile_dir
                .starts_with(&canonical_recovery_root)
        {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        }
        let claimed_profile_dir = operation_claim.profile_dir.clone();
        if operation_claim.directory_identity().ok()
            != Some(scanned_profile_identity)
        {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        }
        let authority = match read_pinned_ownership_record_set(&claimed_profile_dir) {
            Ok(authority) => authority,
            Err(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    reason = "invalid_ownership_marker",
                    "invalid browser ownership marker; profile preserved"
                );
                let _ = error;
                continue;
            }
        };
        if operation_claim.validates(&claimed_profile_dir).is_err() {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        }
        let marker = authority.marker().clone();

        let owner_was_missing = match control.lookup(marker.owner_app.pid) {
            ProcessLookup::Found(observed) if same_process(&marker.owner_app, &observed) => {
                report.live_owners_preserved += 1;
                tracing::info!(
                    target: "nomi_browser_engine::profile",
                    owner_pid = marker.owner_app.pid,
                    reason = "live_verified_owner",
                    "browser profile belongs to a live verified app instance; recovery skipped"
                );
                continue;
            }
            ProcessLookup::Unverified(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    owner_pid = marker.owner_app.pid,
                    reason = "owner_identity_unverified",
                    "browser owner identity could not be verified; profile preserved"
                );
                let _ = error;
                continue;
            }
            ProcessLookup::Missing => true,
            ProcessLookup::Found(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    reason = "owner_pid_reused",
                    "browser owner PID was reused by a different identity; profile preserved"
                );
                continue;
            }
        };
        if marker.phase == BrowserOwnershipPhase::Provisional {
            if owner_was_missing {
                recover_provisional_profile(
                    &canonical_recovery_root,
                    &claimed_profile_dir,
                    &marker,
                    &operation_claim,
                    authority,
                    mode,
                    control,
                    &mut report,
                );
            } else {
                unreachable!("only a missing owner reaches provisional recovery");
            }
            continue;
        }

        let tree_absent = match control.lookup(marker.browser.pid) {
            ProcessLookup::Found(observed) if same_process(&marker.browser, &observed) => {
                let _ = observed;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    browser_pid = marker.browser.pid,
                    reason = "startup_recovery_never_signals_live_browser",
                    "verified orphan browser remains live; startup recovery preserves its profile and never signals marker-derived processes"
                );
                false
            }
            ProcessLookup::Found(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    browser_pid = marker.browser.pid,
                    reason = "browser_pid_reused",
                    "browser PID was reused by a different process identity; startup recovery preserved the profile"
                );
                false
            }
            ProcessLookup::Missing => {
                match control.confirm_tree_absent(&marker.browser) {
                    Ok(true) => true,
                    Ok(false) => {
                        report.failures += 1;
                        report.profiles_preserved += 1;
                        tracing::warn!(
                            target: "nomi_browser_engine::profile",
                            browser_pid = marker.browser.pid,
                            reason = "tree_absence_unconfirmed",
                            "browser root changed or disappeared but its tree is not proven absent; profile preserved"
                        );
                        false
                    }
                    Err(error) => {
                        report.failures += 1;
                        report.profiles_preserved += 1;
                        tracing::warn!(
                            target: "nomi_browser_engine::profile",
                            browser_pid = marker.browser.pid,
                            reason = "tree_absence_check_failed",
                            "browser tree absence could not be verified; profile preserved"
                        );
                        let _ = error;
                        false
                    }
                }
            }
            ProcessLookup::Unverified(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    browser_pid = marker.browser.pid,
                    reason = "browser_identity_unverified",
                    "orphan browser identity could not be verified; profile preserved"
                );
                let _ = error;
                false
            }
        };
        if !tree_absent {
            continue;
        }

        if authority.records().active != marker {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        }
        // The deny-write/delete record handles remain the sole authority
        // throughout every absence proof. Release them only when cleanup is
        // ready to re-resolve and remove the exact records under the still
        // pinned profile directory.
        drop(authority);

        if let Err(error) = cleanup_recovered_profile(
            &canonical_recovery_root,
            &operation_claim,
            mode,
            &marker,
        )
        {
            report.failures += 1;
            report.profiles_preserved += 1;
            tracing::warn!(
                target: "nomi_browser_engine::profile",
                recovery_mode = ?mode,
                reason = "profile_cleanup_failed",
                "browser process tree is absent but profile cleanup failed; profile preserved"
            );
            let _ = error;
            continue;
        }
        match mode {
            ProfileRecoveryMode::DeleteEphemeralProfile => {
                report.ephemeral_profiles_removed += 1;
            }
            ProfileRecoveryMode::PreserveStableProfile => {
                report.stable_markers_cleared += 1;
            }
        }
    }
    record_unresolved_recovery_scan_errors(recovery_root, &scan_errors, &mut report);
    report
}

#[cfg(windows)]
fn has_verified_descendant(
    system: &sysinfo::System,
    root: &ProcessIdentity,
) -> Result<bool, String> {
    for process in system.processes().values() {
        if process.parent().map(sysinfo::Pid::as_u32) != Some(root.pid) {
            continue;
        }
        let identity = process_identity(process)?;
        if identity.platform_start_key < root.platform_start_key {
            return Err(format!(
                "browser child {} predates its expected root {}",
                identity.pid, root.pid
            ));
        }
        return Ok(true);
    }
    Ok(false)
}

#[cfg(windows)]
fn one_process_tree_absence_check(expected: &ProcessIdentity) -> Result<bool, String> {
    let system = fresh_process_snapshot()?;
    if let Some(root) = system.process(sysinfo::Pid::from_u32(expected.pid)) {
        let observed = process_identity(root)?;
        if same_process(expected, &observed) {
            return Ok(false);
        }
        // A reused PID is an explicit quarantine condition. Do not treat the
        // unrelated process as proof that the original tree is absent.
        return Ok(false);
    }
    Ok(!has_verified_descendant(&system, expected)?)
}

#[cfg(unix)]
fn one_process_tree_absence_check(expected: &ProcessIdentity) -> Result<bool, String> {
    let Some(pgid) = expected.process_group_id else {
        return Err("browser marker has no process group".into());
    };
    if pgid == 0 {
        return Err("browser marker has invalid process group 0".into());
    }
    // SAFETY: getpgrp only reads the current process-group id.
    let app_group = unsafe { libc::getpgrp() };
    if app_group <= 0 || pgid == app_group as u32 {
        return Err("browser process group overlaps the current application group".into());
    }
    // Signal 0 probes group membership without a full process-table snapshot
    // (the snapshot blocked async callers for hundreds of milliseconds).
    // ESRCH proves the group is empty; success or EPERM means at least one
    // member is still alive, so absence is not proven.
    // SAFETY: killpg with signal 0 performs existence/permission checks only.
    if unsafe { libc::killpg(pgid as libc::pid_t, 0) } == 0 {
        return Ok(false);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(true),
        Some(libc::EPERM) => Ok(false),
        _ => Err(format!("probe browser process group: {error}")),
    }
}

fn confirm_process_tree_absent(expected: &ProcessIdentity) -> Result<bool, String> {
    let deadline = Instant::now() + PROCESS_TREE_CONFIRM_TIMEOUT;
    let mut confirmations = 0;
    while Instant::now() < deadline {
        if one_process_tree_absence_check(expected)? {
            confirmations += 1;
            if confirmations >= PROCESS_TREE_ABSENCE_CONFIRMATIONS {
                return Ok(true);
            }
        } else {
            confirmations = 0;
        }
        // Confirmations poll tightly; a not-yet-absent tree is re-probed on
        // the coarse interval so a blocked synchronous caller mostly sleeps
        // instead of hammering process inventory until the deadline.
        let interval = if confirmations > 0 {
            PROCESS_TREE_CONFIRM_INTERVAL
        } else {
            PROCESS_TREE_RETRY_INTERVAL
        };
        std::thread::sleep(interval);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FakeProcessControl {
        current: ProcessIdentity,
        processes: HashMap<u32, ProcessLookup>,
        absence_result: Result<bool, String>,
        #[cfg(unix)]
        terminate_result: Result<(), String>,
        terminate_calls: usize,
        absence_calls: usize,
        terminate_identities: Vec<ProcessIdentity>,
        absence_identities: Vec<ProcessIdentity>,
    }

    impl ProcessControl for FakeProcessControl {
        fn current_process(&mut self) -> Result<ProcessIdentity, String> {
            Ok(self.current.clone())
        }

        fn lookup(&mut self, pid: u32) -> ProcessLookup {
            self.processes
                .get(&pid)
                .cloned()
                .unwrap_or(ProcessLookup::Missing)
        }

        fn confirm_tree_absent(&mut self, expected: &ProcessIdentity) -> Result<bool, String> {
            self.absence_calls += 1;
            self.absence_identities.push(expected.clone());
            self.absence_result.clone()
        }

        #[cfg(unix)]
        fn terminate_tree(&mut self, expected: &ProcessIdentity) -> Result<(), String> {
            self.terminate_calls += 1;
            self.terminate_identities.push(expected.clone());
            if self.terminate_result.is_ok() {
                // A successfully terminated tree disappears from later lookups
                // and absence probes, mirroring the real SIGKILL.
                self.processes.remove(&expected.pid);
            }
            self.terminate_result.clone()
        }
    }

    struct SequencedLookupControl {
        current: ProcessIdentity,
        lookups: std::collections::VecDeque<ProcessLookup>,
        terminate_calls: usize,
        absence_calls: usize,
        terminate_identities: Vec<ProcessIdentity>,
        absence_identities: Vec<ProcessIdentity>,
        absence_result: Result<bool, String>,
        replace_marker_on_absence: Option<(PathBuf, BrowserOwnershipMarker)>,
    }

    impl ProcessControl for SequencedLookupControl {
        fn current_process(&mut self) -> Result<ProcessIdentity, String> {
            Ok(self.current.clone())
        }

        fn lookup(&mut self, _pid: u32) -> ProcessLookup {
            self.lookups
                .pop_front()
                .unwrap_or(ProcessLookup::Missing)
        }

        fn confirm_tree_absent(&mut self, expected: &ProcessIdentity) -> Result<bool, String> {
            self.absence_calls += 1;
            self.absence_identities.push(expected.clone());
            if let Some((profile, replacement)) = self.replace_marker_on_absence.take() {
                let _ = std::fs::write(
                    ownership_marker_path(&profile),
                    serde_json::to_vec_pretty(&replacement).unwrap(),
                );
            }
            self.absence_result.clone()
        }

        #[cfg(unix)]
        fn terminate_tree(&mut self, expected: &ProcessIdentity) -> Result<(), String> {
            self.terminate_calls += 1;
            self.terminate_identities.push(expected.clone());
            Ok(())
        }
    }

    fn test_executable(name: &str) -> String {
        #[cfg(windows)]
        {
            format!("c:\\nomifun-test\\{name}.exe")
        }
        #[cfg(not(windows))]
        {
            format!("/opt/nomifun-test/{name}")
        }
    }

    fn identity(pid: u32, name: &str) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            start_time_epoch_seconds: 1_700_000_000 + u64::from(pid),
            platform_start_key: 9_000_000_000 + u64::from(pid),
            executable: test_executable(name),
            #[cfg(windows)]
            process_group_id: None,
            #[cfg(unix)]
            process_group_id: Some(pid),
        }
    }

    fn write_test_marker(
        profile_dir: &Path,
        owner_app: ProcessIdentity,
        browser: ProcessIdentity,
    ) -> BrowserOwnershipMarker {
        std::fs::create_dir_all(profile_dir).unwrap();
        let marker = BrowserOwnershipMarker {
            version: OWNERSHIP_MARKER_VERSION,
            phase: BrowserOwnershipPhase::Committed,
            app_instance_id: nomifun_common::generate_id(),
            owner_app,
            browser,
            profile_id: profile_dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        std::fs::write(
            ownership_marker_path(profile_dir),
            serde_json::to_vec_pretty(&marker).unwrap(),
        )
        .unwrap();
        marker
    }

    fn write_provisional_test_marker(
        profile_dir: &Path,
        owner_app: ProcessIdentity,
    ) -> BrowserOwnershipMarker {
        std::fs::create_dir_all(profile_dir).unwrap();
        let marker = BrowserOwnershipMarker {
            version: OWNERSHIP_MARKER_VERSION,
            phase: BrowserOwnershipPhase::Provisional,
            app_instance_id: nomifun_common::generate_id(),
            owner_app: owner_app.clone(),
            browser: owner_app,
            profile_id: profile_dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        std::fs::write(
            ownership_marker_path(profile_dir),
            serde_json::to_vec_pretty(&marker).unwrap(),
        )
        .unwrap();
        marker
    }

    fn write_current_app_test_token(profile_dir: &Path) -> BrowserOwnershipToken {
        std::fs::create_dir_all(profile_dir).unwrap();
        let mut control = SystemProcessControl;
        let marker = BrowserOwnershipMarker {
            version: OWNERSHIP_MARKER_VERSION,
            phase: BrowserOwnershipPhase::Committed,
            app_instance_id: managed_app_instance_id().to_owned(),
            owner_app: control
                .current_process()
                .expect("resolve exact current application identity"),
            browser: identity(8_282, "chrome"),
            profile_id: profile_dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let canonical_profile = std::fs::canonicalize(profile_dir)
            .expect("canonicalize current-app test profile");
        let marker_path = commit_ownership_marker(&canonical_profile, &marker, None)
            .expect("commit current-app test marker");
        BrowserOwnershipToken {
            profile_identity: capture_profile_directory_identity(&canonical_profile)
                .expect("capture current-app test profile identity"),
            profile_dir: canonical_profile,
            marker_path,
            marker,
        }
    }

    fn write_current_app_ephemeral_pair_token(
        profile_dir: &Path,
    ) -> BrowserOwnershipToken {
        std::fs::create_dir_all(profile_dir).unwrap();
        let claim =
            prepare_ownership_marker_for_launch(profile_dir).expect("claim pair profile");
        let provisional =
            claim_ephemeral_profile_cleanup(profile_dir, &claim)
                .expect("write provisional ownership record");
        let mut committed = provisional.provisional_marker.clone();
        committed.phase = BrowserOwnershipPhase::Committed;
        committed.browser = identity(8_283, "chrome-pair");
        let marker_path = commit_ownership_marker(
            &claim.0.profile_dir,
            &committed,
            Some(&provisional.provisional_marker),
        )
        .expect("append committed ownership sidecar");
        BrowserOwnershipToken {
            profile_dir: claim.0.profile_dir.clone(),
            profile_identity: claim
                .0
                .directory_identity()
                .expect("capture pair profile identity"),
            marker_path,
            marker: committed,
        }
    }

    fn fake_control(
        current: ProcessIdentity,
        processes: HashMap<u32, ProcessLookup>,
    ) -> FakeProcessControl {
        FakeProcessControl {
            current,
            processes,
            absence_result: Ok(true),
            #[cfg(unix)]
            terminate_result: Ok(()),
            terminate_calls: 0,
            absence_calls: 0,
            terminate_identities: Vec::new(),
            absence_identities: Vec::new(),
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_startup_recovery_treats_an_empty_complete_root_as_safe() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        std::fs::create_dir_all(&root).unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert_eq!(report.markers_scanned, 0);
        assert_eq!(report.failures, 0);
        assert_eq!(report.profiles_preserved, 0);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_dead_owner_live_browser_tree_is_terminated_and_profile_removed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("dead-owner-live-browser");
        let browser = identity(201, "chrome-survivor");
        write_test_marker(&profile, identity(101, "nomifun-gone"), browser.clone());
        std::fs::write(profile.join("cache.bin"), b"ephemeral").unwrap();
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(browser.clone()))]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(!profile.exists());
        assert_eq!(report.markers_scanned, 1);
        assert_eq!(report.failures, 0);
        assert_eq!(report.profiles_preserved, 0);
        assert_eq!(report.process_trees_terminated, 1);
        assert_eq!(report.ephemeral_profiles_removed, 1);
        assert_eq!(control.terminate_calls, 1);
        assert_eq!(control.terminate_identities, vec![browser.clone()]);
        assert_eq!(control.absence_identities, vec![browser]);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_dead_owner_stable_profile_clears_marker_after_terminating_browser() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("primary");
        let profile = root.join("generation-3");
        let browser = identity(202, "chrome-survivor");
        write_test_marker(&profile, identity(102, "nomifun-gone"), browser.clone());
        std::fs::write(profile.join("Cookies"), b"persistent-login").unwrap();
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(browser.clone()))]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::PreserveStableProfile,
            &mut control,
        );

        assert!(profile.join("Cookies").exists());
        assert!(!ownership_marker_path(&profile).exists());
        assert_eq!(report.failures, 0);
        assert_eq!(report.process_trees_terminated, 1);
        assert_eq!(report.stable_markers_cleared, 1);
        assert_eq!(control.terminate_identities, vec![browser]);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_termination_failure_fails_closed_and_preserves_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("terminate-failed");
        let browser = identity(203, "chrome-stuck");
        write_test_marker(&profile, identity(103, "nomifun-gone"), browser.clone());
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(browser.clone()))]),
        );
        control.terminate_result = Err("signal refused".into());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert!(report.failures > 0);
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(report.process_trees_terminated, 0);
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert_eq!(control.terminate_calls, 1);
        assert_eq!(control.absence_calls, 0);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_terminated_tree_without_absence_proof_is_preserved() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("terminate-unproven");
        let browser = identity(204, "chrome-lingering");
        write_test_marker(&profile, identity(104, "nomifun-gone"), browser.clone());
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(browser.clone()))]),
        );
        control.absence_result = Ok(false);

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert!(report.failures > 0);
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(report.process_trees_terminated, 0);
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert_eq!(control.terminate_calls, 1);
        assert_eq!(control.absence_calls, 1);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_genuine_scan_error_still_fails_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("occupied-marker-name");
        std::fs::create_dir_all(profile.join(OWNERSHIP_MARKER_FILE)).unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(report.markers_scanned, 0);
        assert!(report.failures > 0);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_provisional_marker_replaced_during_absence_proof_fails_closed() {
        // Unlike Windows there is no deny-write authority handle: a marker
        // rewritten during the absence proof must preserve the profile.
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("provisional-proof-race");
        let original =
            write_provisional_test_marker(&profile, identity(109, "nomifun-gone"));
        let replacement = BrowserOwnershipMarker {
            version: OWNERSHIP_MARKER_VERSION,
            phase: BrowserOwnershipPhase::Committed,
            app_instance_id: nomifun_common::generate_id(),
            owner_app: identity(110, "replacement-owner"),
            browser: identity(111, "replacement-browser"),
            profile_id: original.profile_id.clone(),
        };
        let state = profile.join("Default").join("Cookies");
        std::fs::create_dir_all(state.parent().unwrap()).unwrap();
        std::fs::write(&state, b"preserve").unwrap();
        let mut control = SequencedLookupControl {
            current: identity(999, "current"),
            lookups: std::collections::VecDeque::from([
                ProcessLookup::Missing,
                ProcessLookup::Missing,
            ]),
            terminate_calls: 0,
            absence_calls: 0,
            terminate_identities: Vec::new(),
            absence_identities: Vec::new(),
            absence_result: Ok(true),
            replace_marker_on_absence: Some((profile.clone(), replacement)),
        };

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(state.exists());
        assert!(report.failures > 0);
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert_eq!(control.terminate_calls, 0);
    }

    #[cfg(unix)]
    #[test]
    fn unix_marker_scan_depth_overflow_is_resolved_by_exact_cleanup() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("too-deep");
        let browser = identity(206, "chrome-gone");
        write_test_marker(&profile, identity(106, "nomifun-gone"), browser);
        let mut deep = profile.clone();
        for level in 0..MAX_MARKER_SCAN_DEPTH {
            deep = deep.join(format!("d{level}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("entry.bin"), b"beyond-depth").unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(
            !profile.exists(),
            "the exact marker-owned deletion walker safely resolves a stale discovery-depth error"
        );
        assert_eq!(report.failures, 0);
        assert_eq!(report.ephemeral_profiles_removed, 1);
    }

    #[cfg(unix)]
    #[test]
    fn unix_ephemeral_delete_depth_overflow_fails_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("beyond-delete-depth");
        write_test_marker(
            &profile,
            identity(107, "nomifun-gone"),
            identity(207, "chrome-gone"),
        );
        let mut deep = profile.clone();
        for level in 0..=MAX_EPHEMERAL_DELETE_DEPTH {
            deep = deep.join(format!("d{level}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("entry.bin"), b"beyond-delete-depth").unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists(), "the deletion depth ceiling fails closed");
        assert!(ownership_marker_path(&profile).is_file());
        assert!(report.failures > 0);
        assert_eq!(report.ephemeral_profiles_removed, 0);
    }

    #[test]
    fn live_verified_owner_is_never_taken_over() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("host-live");
        let owner = identity(101, "nomifun");
        let browser = identity(202, "chrome");
        write_test_marker(&profile, owner.clone(), browser.clone());
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([
                (owner.pid, ProcessLookup::Found(owner)),
                (browser.pid, ProcessLookup::Found(browser)),
            ]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(report.live_owners_preserved, 1);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
    }

    #[test]
    fn live_provisional_owner_is_preserved_without_process_tree_actions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("provisional-live");
        let owner = identity(103, "nomifun-live");
        write_provisional_test_marker(&profile, owner.clone());
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(owner.pid, ProcessLookup::Found(owner))]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(report.live_owners_preserved, 1);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
    }

    #[test]
    fn missing_provisional_owner_is_rechecked_and_then_marker_last_removed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("provisional-missing");
        let owner = identity(104, "nomifun-gone");
        write_provisional_test_marker(&profile, owner.clone());
        std::fs::write(profile.join("cache.bin"), b"ephemeral").unwrap();
        let mut control = SequencedLookupControl {
            current: identity(999, "current"),
            lookups: std::collections::VecDeque::from([
                ProcessLookup::Missing,
                ProcessLookup::Missing,
            ]),
            terminate_calls: 0,
            absence_calls: 0,
            terminate_identities: Vec::new(),
            absence_identities: Vec::new(),
            absence_result: Ok(true),
            replace_marker_on_absence: None,
        };

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(!profile.exists());
        assert_eq!(report.ephemeral_profiles_removed, 1);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 1);
        assert_eq!(control.terminate_identities, Vec::<ProcessIdentity>::new());
        assert_eq!(control.absence_identities, vec![owner]);
    }

    #[test]
    fn provisional_missing_to_found_race_never_signals_the_application_identity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("provisional-race");
        let owner = identity(105, "nomifun-race");
        write_provisional_test_marker(&profile, owner.clone());
        let mut control = SequencedLookupControl {
            current: identity(999, "current"),
            lookups: std::collections::VecDeque::from([
                ProcessLookup::Missing,
                ProcessLookup::Found(owner),
            ]),
            terminate_calls: 0,
            absence_calls: 0,
            terminate_identities: Vec::new(),
            absence_identities: Vec::new(),
            absence_result: Ok(true),
            replace_marker_on_absence: None,
        };

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
    }

    #[cfg(windows)]
    #[test]
    fn pinned_provisional_marker_rejects_change_during_absence_proof() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("provisional-proof-race");
        let original =
            write_provisional_test_marker(&profile, identity(109, "nomifun-gone"));
        let replacement = BrowserOwnershipMarker {
            version: OWNERSHIP_MARKER_VERSION,
            phase: BrowserOwnershipPhase::Committed,
            app_instance_id: nomifun_common::generate_id(),
            owner_app: identity(110, "replacement-owner"),
            browser: identity(111, "replacement-browser"),
            profile_id: original.profile_id.clone(),
        };
        let state = profile.join("Default").join("Cookies");
        std::fs::create_dir_all(state.parent().unwrap()).unwrap();
        std::fs::write(&state, b"preserve").unwrap();
        let mut control = SequencedLookupControl {
            current: identity(999, "current"),
            lookups: std::collections::VecDeque::from([
                ProcessLookup::Missing,
                ProcessLookup::Missing,
            ]),
            terminate_calls: 0,
            absence_calls: 0,
            terminate_identities: Vec::new(),
            absence_identities: Vec::new(),
            absence_result: Ok(true),
            replace_marker_on_absence: Some((profile.clone(), replacement.clone())),
        };

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(!profile.exists());
        assert_eq!(report.ephemeral_profiles_removed, 1);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 1);
        assert_eq!(
            control.absence_identities,
            vec![original.owner_app.clone()]
        );
    }

    #[test]
    fn provisional_pid_reuse_and_unverified_owner_are_fail_closed() {
        for (name, lookup) in [
            (
                "pid-reused",
                ProcessLookup::Found(identity(106, "unrelated")),
            ),
            (
                "unverified",
                ProcessLookup::Unverified("snapshot unavailable".into()),
            ),
        ] {
            let tmp = tempfile::TempDir::new().unwrap();
            let root = tmp.path().join("ephemeral");
            let profile = root.join(name);
            let owner = identity(106, "nomifun-gone");
            write_provisional_test_marker(&profile, owner.clone());
            let mut control = fake_control(
                identity(999, "current"),
                HashMap::from([(owner.pid, lookup)]),
            );

            let report = recover_owned_profiles_with(
                &root,
                ProfileRecoveryMode::DeleteEphemeralProfile,
                &mut control,
            );

            assert!(profile.exists(), "{name}");
            assert_eq!(report.profiles_preserved, 1, "{name}");
            assert_eq!(control.terminate_calls, 0, "{name}");
            assert_eq!(control.absence_calls, 0, "{name}");
        }
    }

    #[test]
    fn provisional_marker_never_authorizes_stable_profile_deletion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("primary");
        let profile = root.join("provisional-stable");
        let owner = identity(107, "nomifun-gone");
        write_provisional_test_marker(&profile, owner);
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::PreserveStableProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
    }

    #[test]
    fn malformed_committed_owner_alias_is_preserved_without_process_actions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("committed-owner-alias");
        let owner = identity(108, "nomifun-alias");
        let mut marker = write_provisional_test_marker(&profile, owner);
        marker.phase = BrowserOwnershipPhase::Committed;
        std::fs::write(
            ownership_marker_path(&profile),
            serde_json::to_vec_pretty(&marker).unwrap(),
        )
        .unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
    }

    #[test]
    fn pid_reuse_or_browser_identity_mismatch_preserves_without_kill() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("host-reused");
        let owner = identity(111, "nomifun");
        let browser = identity(222, "chrome");
        write_test_marker(&profile, owner, browser.clone());
        let mut reused = browser.clone();
        reused.platform_start_key += 1;
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(reused))]),
        );
        control.absence_result = Ok(false);

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
        assert_eq!(report.profiles_preserved, 1);
    }

    #[test]
    fn owner_executable_only_mismatch_preserves_without_process_actions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("owner-executable-mismatch");
        let owner = identity(112, "nomifun");
        let browser = identity(223, "chrome");
        write_test_marker(&profile, owner.clone(), browser);
        let mut observed_owner = owner.clone();
        observed_owner.executable = test_executable("different-owner");
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(
                owner.pid,
                ProcessLookup::Found(observed_owner),
            )]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
        assert!(control.terminate_identities.is_empty());
        assert!(control.absence_identities.is_empty());
    }

    #[test]
    fn browser_executable_only_mismatch_preserves_without_process_actions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("browser-executable-mismatch");
        let browser = identity(224, "chrome");
        write_test_marker(
            &profile,
            identity(113, "nomifun-gone"),
            browser.clone(),
        );
        let mut observed_browser = browser.clone();
        observed_browser.executable = test_executable("different-browser");
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(
                browser.pid,
                ProcessLookup::Found(observed_browser),
            )]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
        assert!(control.terminate_identities.is_empty());
        assert!(control.absence_identities.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn exact_live_orphan_is_preserved_without_startup_signal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("host-orphan");
        let browser = identity(232, "chrome");
        write_test_marker(&profile, identity(121, "nomifun"), browser.clone());
        std::fs::write(profile.join("cache.bin"), b"cache").unwrap();
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(browser.clone()))]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(control.terminate_calls, 0);
        assert!(control.terminate_identities.is_empty());
        assert_eq!(report.process_trees_terminated, 0);
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert_eq!(report.profiles_preserved, 1);
    }

    #[test]
    fn outer_ephemeral_marker_never_deletes_a_preserved_nested_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let outer = root.join("outer-orphan");
        let inner = outer.join("inner-live");
        let outer_browser = identity(233, "chrome-outer");
        let inner_owner = identity(123, "nomifun-live");
        write_test_marker(
            &outer,
            identity(122, "nomifun-gone"),
            outer_browser.clone(),
        );
        write_test_marker(
            &inner,
            inner_owner.clone(),
            identity(234, "chrome-inner"),
        );
        std::fs::write(inner.join("Cookies"), b"must survive").unwrap();
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(inner_owner.pid, ProcessLookup::Found(inner_owner))]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(outer.exists());
        assert!(inner.join("Cookies").exists());
        assert!(ownership_marker_path(&inner).exists());
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(report.live_owners_preserved, 1);
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(report.failures, 1);
    }

    #[test]
    fn deeply_nested_ownership_marker_is_found_and_preserved() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let outer = root.join("outer-deep-marker");
        let outer_browser = identity(235, "chrome-outer");
        write_test_marker(
            &outer,
            identity(124, "nomifun-gone"),
            outer_browser.clone(),
        );
        let mut inner = outer.clone();
        for level in 0..12 {
            inner = inner.join(format!("level-{level}"));
        }
        let inner_owner = identity(125, "nomifun-deep-live");
        write_test_marker(
            &inner,
            inner_owner.clone(),
            identity(236, "chrome-inner"),
        );
        std::fs::write(inner.join("Cookies"), b"must survive").unwrap();
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(inner_owner.pid, ProcessLookup::Found(inner_owner))]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(outer.exists());
        assert!(inner.join("Cookies").exists());
        assert_eq!(report.ephemeral_profiles_removed, 0);
    }

    #[test]
    fn deep_profile_without_nested_marker_remains_cleanable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("deep-cache-only");
        let browser = identity(237, "chrome");
        write_test_marker(&profile, identity(126, "nomifun-gone"), browser.clone());
        let mut cache = profile.clone();
        for level in 0..12 {
            cache = cache.join(format!("cache-{level}"));
        }
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("entry.bin"), b"cache").unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(!profile.exists());
        assert_eq!(report.ephemeral_profiles_removed, 1);
    }

    #[test]
    fn marker_named_directory_makes_recursive_cleanup_fail_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("marker-directory");
        let browser = identity(238, "chrome");
        write_test_marker(&profile, identity(127, "nomifun-gone"), browser.clone());
        let malformed = profile.join("nested").join(OWNERSHIP_MARKER_FILE);
        std::fs::create_dir_all(&malformed).unwrap();
        std::fs::write(malformed.join("state"), b"preserve").unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert!(malformed.join("state").exists());
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert!(report.failures > 0);
    }

    #[cfg(unix)]
    #[test]
    fn marker_named_symlink_makes_recursive_cleanup_fail_closed() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("marker-symlink");
        let browser = identity(239, "chrome");
        write_test_marker(&profile, identity(128, "nomifun-gone"), browser.clone());
        let nested = profile.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        symlink(
            profile.join("outside-target"),
            nested.join(OWNERSHIP_MARKER_FILE),
        )
        .unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert!(report.failures > 0);
    }

    #[cfg(windows)]
    #[test]
    fn live_browser_is_preserved_even_when_legacy_termination_result_would_fail() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("host-stuck");
        let browser = identity(242, "chrome");
        write_test_marker(&profile, identity(131, "nomifun"), browser.clone());
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(browser))]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert_eq!(report.failures, 0);
    }

    #[test]
    fn already_absent_tree_can_release_ephemeral_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("host-gone");
        let browser = identity(252, "chrome");
        write_test_marker(
            &profile,
            identity(141, "nomifun"),
            browser.clone(),
        );
        let mut control = fake_control(identity(999, "current"), HashMap::new());
        control.absence_result = Ok(true);

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(!profile.exists());
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 1);
        assert_eq!(control.absence_identities, vec![browser]);
        assert_eq!(report.ephemeral_profiles_removed, 1);
    }

    #[test]
    fn stable_primary_clears_absent_orphan_artifacts_but_keeps_profile_data() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("primary");
        let profile = root.join("generation-7");
        let browser = identity(262, "chrome");
        write_test_marker(&profile, identity(151, "nomifun"), browser.clone());
        std::fs::write(profile.join("Cookies"), b"persistent").unwrap();
        let port_path = profile.join(DEVTOOLS_ACTIVE_PORT_FILE);
        std::fs::write(&port_path, b"9222\n/devtools/browser/recovered\n").unwrap();
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::new(),
        );
        control.absence_result = Ok(true);

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::PreserveStableProfile,
            &mut control,
        );

        assert!(profile.join("Cookies").exists());
        assert!(!ownership_marker_path(&profile).exists());
        assert!(
            !port_path.exists(),
            "startup recovery must remove the exact runtime port artifact before its marker"
        );
        assert_eq!(report.process_trees_terminated, 0);
        assert!(control.terminate_identities.is_empty());
        assert_eq!(control.absence_identities, vec![browser]);
        assert_eq!(report.stable_markers_cleared, 1);
        assert_eq!(report.ephemeral_profiles_removed, 0);
    }

    #[test]
    fn stable_recovery_preserves_marker_when_port_artifact_is_unsafe() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("primary");
        let profile = root.join("generation-unsafe-port");
        let browser = identity(263, "chrome");
        write_test_marker(&profile, identity(152, "nomifun"), browser.clone());
        std::fs::create_dir(profile.join(DEVTOOLS_ACTIVE_PORT_FILE)).unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::PreserveStableProfile,
            &mut control,
        );

        assert!(ownership_marker_path(&profile).is_file());
        assert!(profile.join(DEVTOOLS_ACTIVE_PORT_FILE).is_dir());
        assert_eq!(report.stable_markers_cleared, 0);
        assert_eq!(report.failures, 1);
    }

    #[test]
    fn malformed_and_unmarked_profiles_are_never_removed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let malformed = root.join("malformed");
        let unmarked = root.join("unmarked");
        std::fs::create_dir_all(&malformed).unwrap();
        std::fs::create_dir_all(&unmarked).unwrap();
        std::fs::write(ownership_marker_path(&malformed), b"{bad json").unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(malformed.exists());
        assert!(unmarked.exists());
        assert_eq!(report.markers_scanned, 1);
        assert_eq!(report.failures, 1);
        assert_eq!(control.terminate_calls, 0);
    }

    #[test]
    fn marker_payload_contains_ownership_only_and_no_endpoint_or_secret_fields() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("profile-safe");
        let marker = write_test_marker(
            &profile,
            identity(161, "nomifun"),
            identity(272, "chrome"),
        );
        let serialized = serde_json::to_string(&marker).unwrap().to_lowercase();
        for forbidden in [
            "endpoint",
            "websocket",
            "cdp",
            "secret",
            "token",
            "cookie",
            "lease",
            "password",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "marker leaked forbidden field/value class {forbidden}: {serialized}"
            );
        }
    }

    #[test]
    fn ownership_debug_and_direct_errors_do_not_echo_paths_or_marker_payload() {
        let profile_sentinel = "PROFILE-PATH-SENTINEL";
        let executable_sentinel = "EXECUTABLE-PATH-SENTINEL";
        let marker_payload_sentinel = "MARKER-PAYLOAD-SENTINEL";
        let process = ProcessIdentity {
            pid: 4242,
            start_time_epoch_seconds: 1,
            platform_start_key: 2,
            executable: format!("/private/{executable_sentinel}"),
            #[cfg(windows)]
            process_group_id: None,
            #[cfg(unix)]
            process_group_id: Some(4242),
        };
        let marker = BrowserOwnershipMarker {
            version: OWNERSHIP_MARKER_VERSION,
            phase: BrowserOwnershipPhase::Committed,
            app_instance_id: marker_payload_sentinel.into(),
            owner_app: process.clone(),
            browser: process,
            profile_id: profile_sentinel.into(),
        };

        let debug = format!("{marker:?}");
        for sentinel in [
            profile_sentinel,
            executable_sentinel,
            marker_payload_sentinel,
        ] {
            assert!(
                !debug.contains(sentinel),
                "ownership Debug leaked sentinel {sentinel}: {debug}"
            );
        }

        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join(profile_sentinel);
        std::fs::create_dir_all(&profile).unwrap();
        let first = ProfileOperationClaim::acquire(&profile).unwrap();
        let error = match ProfileOperationClaim::acquire(&profile) {
            Ok(_) => panic!("second profile operation claim must fail"),
            Err(error) => error,
        };
        drop(first);
        assert!(!error.contains(profile_sentinel), "{error}");
        assert!(
            !error.contains(&profile.display().to_string()),
            "{error}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn orphan_recovery_logs_do_not_echo_profile_paths_or_marker_payload() {
        #[derive(Default)]
        struct CapturedEvents(std::sync::Mutex<Vec<String>>);

        struct FieldVisitor<'a>(&'a mut String);

        impl tracing::field::Visit for FieldVisitor<'_> {
            fn record_debug(
                &mut self,
                field: &tracing::field::Field,
                value: &dyn std::fmt::Debug,
            ) {
                use std::fmt::Write as _;
                let _ = write!(self.0, "{}={value:?};", field.name());
            }
        }

        impl tracing::Subscriber for CapturedEvents {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
                true
            }

            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }

            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

            fn event(&self, event: &tracing::Event<'_>) {
                let mut rendered = String::new();
                event.record(&mut FieldVisitor(&mut rendered));
                self.0.lock().unwrap().push(rendered);
            }

            fn enter(&self, _: &tracing::span::Id) {}

            fn exit(&self, _: &tracing::span::Id) {}
        }

        let profile_sentinel = "RECOVERY-PROFILE-PATH-SENTINEL";
        let executable_sentinel = "RECOVERY-EXECUTABLE-PATH-SENTINEL";
        let marker_payload_sentinel = "RECOVERY-MARKER-PAYLOAD-SENTINEL";
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join(profile_sentinel);
        let mut marker = write_test_marker(
            &profile,
            identity(301, "owner"),
            identity(302, "browser"),
        );
        marker.app_instance_id = marker_payload_sentinel.into();
        marker.browser.executable = test_executable(executable_sentinel);
        std::fs::write(
            ownership_marker_path(&profile),
            serde_json::to_vec_pretty(&marker).unwrap(),
        )
        .unwrap();
        let mut control = fake_control(identity(999, "current"), HashMap::new());
        let subscriber = std::sync::Arc::new(CapturedEvents::default());

        tracing::subscriber::with_default(subscriber.clone(), || {
            let report = recover_owned_profiles_with(
                &root,
                ProfileRecoveryMode::DeleteEphemeralProfile,
                &mut control,
            );
            assert_eq!(report.failures, 1);
            assert_eq!(report.profiles_preserved, 1);
        });

        let captured = subscriber.0.lock().unwrap().join("\n");
        for sentinel in [
            profile_sentinel,
            executable_sentinel,
            marker_payload_sentinel,
            &profile.display().to_string(),
        ] {
            assert!(
                !captured.contains(sentinel),
                "orphan recovery log leaked sentinel {sentinel}: {captured}"
            );
        }
        assert!(captured.contains("invalid_ownership_marker"), "{captured}");
    }

    #[test]
    fn ownership_marker_commit_writes_once_and_never_overwrites() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("profile-commit");
        std::fs::create_dir_all(&profile).unwrap();
        let marker = BrowserOwnershipMarker {
            version: OWNERSHIP_MARKER_VERSION,
            phase: BrowserOwnershipPhase::Committed,
            app_instance_id: nomifun_common::generate_id(),
            owner_app: identity(171, "nomifun"),
            browser: identity(282, "chrome"),
            profile_id: "profile-commit".into(),
        };

        commit_ownership_marker(&profile, &marker, None).expect("first marker commit");
        assert_eq!(read_marker(&profile).unwrap(), marker);
        assert!(
            commit_ownership_marker(&profile, &marker, None).is_err(),
            "an existing ownership marker must never be overwritten"
        );
        let temp_prefix = format!("{OWNERSHIP_MARKER_FILE}.");
        assert!(
            std::fs::read_dir(&profile)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    name == OWNERSHIP_MARKER_FILE || !name.starts_with(&temp_prefix)
                }),
            "marker commit must not leave a temp file"
        );
    }

    #[test]
    fn oversized_ownership_record_is_rejected_before_body_allocation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("oversized-marker");
        std::fs::create_dir_all(&profile).unwrap();
        let marker_path = ownership_marker_path(&profile);
        let marker = std::fs::File::create(&marker_path).unwrap();
        marker
            .set_len(MAX_OWNERSHIP_RECORD_BYTES + 1)
            .unwrap();
        drop(marker);

        let error = read_marker_at(&profile, &marker_path)
            .expect_err("oversized ownership authority must fail closed");
        assert!(error.contains("too large"), "{error}");
        assert_eq!(
            std::fs::metadata(marker_path).unwrap().len(),
            MAX_OWNERSHIP_RECORD_BYTES + 1
        );
    }

    #[test]
    fn retained_path_budget_counts_full_prefixes_atomically_and_releases_them() {
        let parent = PathBuf::from("shared-prefix").join("x".repeat(4096));
        let marker = parent.join(OWNERSHIP_MARKER_FILE);
        let combined = path_storage_bytes(&parent)
            .checked_add(path_storage_bytes(&marker))
            .unwrap();
        let mut too_small = RetainedPathBudget::new(combined - 1, "test retained paths");

        assert!(
            too_small
                .charge(&[parent.as_path(), marker.as_path()])
                .is_err()
        );
        assert_eq!(
            too_small.used_bytes, 0,
            "a rejected multi-path charge must not retain partial accounting"
        );

        let mut exact = RetainedPathBudget::new(combined, "test pending paths");
        exact
            .charge(&[parent.as_path(), marker.as_path()])
            .unwrap();
        assert_eq!(exact.used_bytes, combined);
        exact.release(&marker).unwrap();
        exact.release(&parent).unwrap();
        assert_eq!(exact.used_bytes, 0);
    }

    #[test]
    fn provisional_marker_is_atomically_replaced_by_exact_committed_identity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("provisional-transition");
        std::fs::create_dir_all(&profile).unwrap();
        let claim = prepare_ownership_marker_for_launch(&profile).unwrap();
        let cleanup = claim_ephemeral_profile_cleanup(&profile, &claim).unwrap();
        assert_eq!(
            read_marker(&profile).unwrap().phase,
            BrowserOwnershipPhase::Provisional
        );
        let mut committed = cleanup.provisional_marker.clone();
        committed.phase = BrowserOwnershipPhase::Committed;
        committed.browser = identity(8_401, "chrome");

        commit_ownership_marker(
            &profile,
            &committed,
            Some(&cleanup.provisional_marker),
        )
        .expect("exact provisional marker is atomically replaced");

        assert_eq!(read_marker(&profile).unwrap(), committed);
    }

    #[test]
    fn changed_provisional_marker_is_never_replaced() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("provisional-changed");
        std::fs::create_dir_all(&profile).unwrap();
        let claim = prepare_ownership_marker_for_launch(&profile).unwrap();
        let cleanup = claim_ephemeral_profile_cleanup(&profile, &claim).unwrap();
        let replacement = write_test_marker(
            &profile,
            identity(8_402, "foreign-owner"),
            identity(8_403, "foreign-chrome"),
        );
        let mut committed = cleanup.provisional_marker.clone();
        committed.phase = BrowserOwnershipPhase::Committed;
        committed.browser = identity(8_404, "chrome");

        let error = commit_ownership_marker(
            &profile,
            &committed,
            Some(&cleanup.provisional_marker),
        )
        .expect_err("changed provisional marker must fail closed");

        assert!(error.contains("changed"), "{error}");
        assert_eq!(read_marker(&profile).unwrap(), replacement);
    }

    #[test]
    fn append_only_commit_never_overwrites_a_barrier_race_replacement() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("provisional-barrier-race");
        std::fs::create_dir_all(&profile).unwrap();
        let claim = prepare_ownership_marker_for_launch(&profile).unwrap();
        let cleanup = claim_ephemeral_profile_cleanup(&profile, &claim).unwrap();
        let mut committed = cleanup.provisional_marker.clone();
        committed.phase = BrowserOwnershipPhase::Committed;
        committed.browser = identity(8_407, "chrome");
        let foreign = BrowserOwnershipMarker {
            version: OWNERSHIP_MARKER_VERSION,
            phase: BrowserOwnershipPhase::Provisional,
            app_instance_id: nomifun_common::generate_id(),
            owner_app: identity(8_408, "foreign-owner"),
            browser: identity(8_408, "foreign-owner"),
            profile_id: profile.file_name().unwrap().to_string_lossy().into_owned(),
        };
        let foreign_bytes = serde_json::to_vec_pretty(&foreign).unwrap();
        let replacement_succeeded = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&replacement_succeeded);
        OWNERSHIP_COMMIT_BARRIER_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |profile, _committed_path| {
                if std::fs::write(ownership_marker_path(profile), &foreign_bytes).is_ok() {
                    observed.store(true, Ordering::SeqCst);
                }
            }));
        });

        let result = commit_ownership_marker(
            &profile,
            &committed,
            Some(&cleanup.provisional_marker),
        );
        #[cfg(unix)]
        {
            let error =
                result.expect_err("successful barrier replacement must quarantine lineage");
            assert!(
                error.contains("does not match")
                    || error.contains("provisional predecessor"),
                "{error}"
            );
            assert!(replacement_succeeded.load(Ordering::SeqCst));
        }
        #[cfg(windows)]
        {
            result.expect(
                "the pinned predecessor must reject a racing in-place replacement",
            );
            assert!(
                !replacement_succeeded.load(Ordering::SeqCst),
                "the predecessor handle must deny a racing writer"
            );
        }
        assert_eq!(
            read_marker_at(&profile, &ownership_marker_path(&profile)).unwrap(),
            if replacement_succeeded.load(Ordering::SeqCst) {
                foreign
            } else {
                cleanup.provisional_marker
            },
            "the racing record is never overwritten"
        );
        assert_eq!(
            read_marker_at(
                &profile,
                &committed_ownership_record_path(&profile)
            )
            .unwrap(),
            committed,
            "the exact browser lineage remains durable in its append-only sidecar"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_initial_child_anchor_blocks_profile_namespace_replacement() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("windows-pinned-commit");
        let replacement = tmp.path().join("windows-pinned-replacement");
        let displaced = tmp.path().join("windows-pinned-displaced");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::write(replacement.join("sentinel"), b"replacement").unwrap();
        let claim = prepare_ownership_marker_for_launch(&profile).unwrap();
        let rename_succeeded = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&rename_succeeded);
        let profile_for_hook = profile.clone();
        let replacement_for_hook = replacement.clone();
        let displaced_for_hook = displaced.clone();
        OWNERSHIP_COMMIT_DIRECTORY_BOUND_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                if std::fs::rename(&profile_for_hook, &displaced_for_hook).is_ok() {
                    observed.store(true, Ordering::SeqCst);
                    std::fs::rename(&replacement_for_hook, &profile_for_hook)
                        .expect("install unexpected test replacement");
                }
            }));
        });

        let cleanup = claim_ephemeral_profile_cleanup(&profile, &claim)
            .expect("pinned exact profile accepts its provisional marker");

        assert!(
            !rename_succeeded.load(Ordering::SeqCst),
            "the no-share-delete root handle must reject a namespace rename"
        );
        assert_eq!(
            read_marker(&profile).unwrap(),
            cleanup.provisional_marker
        );
        assert_eq!(
            std::fs::read(replacement.join("sentinel")).unwrap(),
            b"replacement"
        );
        assert!(!ownership_marker_path(&replacement).exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_transition_pins_predecessor_and_committed_child_against_replacement() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("windows-transition-commit");
        let replacement = tmp.path().join("windows-transition-replacement");
        let displaced = tmp.path().join("windows-transition-displaced");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::write(replacement.join("sentinel"), b"replacement").unwrap();
        let claim = prepare_ownership_marker_for_launch(&profile).unwrap();
        let cleanup = claim_ephemeral_profile_cleanup(&profile, &claim).unwrap();
        let mut committed = cleanup.provisional_marker.clone();
        committed.phase = BrowserOwnershipPhase::Committed;
        committed.browser = identity(8_411, "chrome");

        let rename_succeeded = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&rename_succeeded);
        let profile_for_hook = profile.clone();
        let replacement_for_hook = replacement.clone();
        let displaced_for_hook = displaced.clone();
        OWNERSHIP_COMMIT_DIRECTORY_BOUND_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                if std::fs::rename(&profile_for_hook, &displaced_for_hook).is_ok() {
                    observed.store(true, Ordering::SeqCst);
                    std::fs::rename(&replacement_for_hook, &profile_for_hook)
                        .expect("install unexpected test replacement");
                }
            }));
        });

        let marker_path = commit_ownership_marker_under_claim(
            &profile,
            &committed,
            Some(&cleanup.provisional_marker),
            &claim.0,
        )
        .expect("both exact child handles keep the transition namespace stable");

        assert!(
            !rename_succeeded.load(Ordering::SeqCst),
            "the predecessor/final child handles must reject profile replacement"
        );
        assert_eq!(marker_path, committed_ownership_record_path(&profile));
        assert_eq!(read_marker(&profile).unwrap(), committed);
        assert_eq!(
            std::fs::read(replacement.join("sentinel")).unwrap(),
            b"replacement"
        );
        assert!(
            !ownership_marker_path(&replacement).exists()
                && !committed_ownership_record_path(&replacement).exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_initial_commit_never_writes_or_cleans_namespace_replacement() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("unix-initial-a");
        let replacement = tmp.path().join("unix-initial-b");
        let displaced = tmp.path().join("unix-initial-displaced");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::write(replacement.join("sentinel"), b"replacement").unwrap();
        let claim = prepare_ownership_marker_for_launch(&profile).unwrap();

        let profile_for_hook = profile.clone();
        let replacement_for_hook = replacement.clone();
        let displaced_for_hook = displaced.clone();
        OWNERSHIP_COMMIT_DIRECTORY_BOUND_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                std::fs::rename(&profile_for_hook, &displaced_for_hook).unwrap();
                std::fs::rename(&replacement_for_hook, &profile_for_hook).unwrap();
            }));
        });

        let error = claim_ephemeral_profile_cleanup(&profile, &claim)
            .expect_err("namespace replacement must block provisional publication");

        assert!(error.contains("directory"), "{error}");
        assert_eq!(
            std::fs::read(profile.join("sentinel")).unwrap(),
            b"replacement"
        );
        assert!(
            !ownership_marker_path(&profile).exists(),
            "the replacement must remain marker-free"
        );
        let displaced_marker: BrowserOwnershipMarker = serde_json::from_slice(
            &std::fs::read(ownership_marker_path(&displaced)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            displaced_marker.phase,
            BrowserOwnershipPhase::Provisional,
            "fd-relative commit stays with the displaced original inode"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_transition_postcommit_mismatch_never_publishes_replacement_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("unix-transition-a");
        let replacement = tmp.path().join("unix-transition-b");
        let displaced = tmp.path().join("unix-transition-displaced");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::write(replacement.join("sentinel"), b"replacement").unwrap();
        let claim = prepare_ownership_marker_for_launch(&profile).unwrap();
        let cleanup = claim_ephemeral_profile_cleanup(&profile, &claim).unwrap();
        let mut committed = cleanup.provisional_marker.clone();
        committed.phase = BrowserOwnershipPhase::Committed;
        committed.browser = identity(8_409, "chrome");

        let profile_for_hook = profile.clone();
        let replacement_for_hook = replacement.clone();
        let displaced_for_hook = displaced.clone();
        OWNERSHIP_COMMIT_BARRIER_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_, _| {
                std::fs::rename(&profile_for_hook, &displaced_for_hook).unwrap();
                std::fs::rename(&replacement_for_hook, &profile_for_hook).unwrap();
            }));
        });

        let error = commit_ownership_marker_under_claim(
            &profile,
            &committed,
            Some(&cleanup.provisional_marker),
            &claim.0,
        )
        .expect_err("postcommit namespace mismatch must withhold the token path");

        assert!(error.contains("directory"), "{error}");
        assert_eq!(
            std::fs::read(profile.join("sentinel")).unwrap(),
            b"replacement"
        );
        assert!(
            !ownership_marker_path(&profile).exists()
                && !committed_ownership_record_path(&profile).exists(),
            "the replacement must not receive ownership lineage"
        );
        let displaced_marker: BrowserOwnershipMarker = serde_json::from_slice(
            &std::fs::read(committed_ownership_record_path(&displaced)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            displaced_marker,
            committed,
            "the committed record remains bound to the displaced original"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_transition_aba_swap_keeps_every_operation_on_original_fd() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("unix-aba-a");
        let replacement = tmp.path().join("unix-aba-b");
        let displaced = tmp.path().join("unix-aba-displaced");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::write(replacement.join("sentinel"), b"replacement").unwrap();
        let claim = prepare_ownership_marker_for_launch(&profile).unwrap();
        let cleanup = claim_ephemeral_profile_cleanup(&profile, &claim).unwrap();
        let mut committed = cleanup.provisional_marker.clone();
        committed.phase = BrowserOwnershipPhase::Committed;
        committed.browser = identity(8_410, "chrome");

        let profile_for_bind = profile.clone();
        let replacement_for_bind = replacement.clone();
        let displaced_for_bind = displaced.clone();
        OWNERSHIP_COMMIT_DIRECTORY_BOUND_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                std::fs::rename(&profile_for_bind, &displaced_for_bind).unwrap();
                std::fs::rename(&replacement_for_bind, &profile_for_bind).unwrap();
            }));
        });
        let profile_for_restore = profile.clone();
        let replacement_for_restore = replacement.clone();
        let displaced_for_restore = displaced.clone();
        OWNERSHIP_COMMIT_BARRIER_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_, _| {
                std::fs::rename(&profile_for_restore, &replacement_for_restore).unwrap();
                std::fs::rename(&displaced_for_restore, &profile_for_restore).unwrap();
            }));
        });

        let marker_path = commit_ownership_marker_under_claim(
            &profile,
            &committed,
            Some(&cleanup.provisional_marker),
            &claim.0,
        )
        .expect("restored original namespace may publish its exact lineage");

        assert_eq!(marker_path, committed_ownership_record_path(&profile));
        assert_eq!(read_marker(&profile).unwrap(), committed);
        assert_eq!(
            std::fs::read(replacement.join("sentinel")).unwrap(),
            b"replacement",
            "the temporary namespace occupant is never read, written, or cleaned"
        );
        assert!(
            !ownership_marker_path(&replacement).exists()
                && !committed_ownership_record_path(&replacement).exists()
        );
    }

    #[test]
    fn provisional_commit_error_preserves_appeared_ownership_and_profile_data() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("provisional-appeared-marker");
        std::fs::create_dir_all(&profile).unwrap();
        let claim = prepare_ownership_marker_for_launch(&profile).unwrap();
        let foreign = write_test_marker(
            &profile,
            identity(8_405, "foreign-owner"),
            identity(8_406, "foreign-chrome"),
        );
        let state = profile.join("Default").join("Cookies");
        std::fs::create_dir_all(state.parent().unwrap()).unwrap();
        std::fs::write(&state, b"preserve").unwrap();

        let error = claim_ephemeral_profile_cleanup(&profile, &claim)
            .expect_err("appeared ownership prevents provisional commit");

        assert!(error.contains("unexpectedly appeared"), "{error}");
        assert_eq!(read_marker(&profile).unwrap(), foreign);
        assert_eq!(std::fs::read(&state).unwrap(), b"preserve");
    }

    #[test]
    fn exact_shutdown_cleanup_removes_only_runtime_artifacts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("stable-profile-cleanup");
        let stable_data = profile.join("Default").join("stable-state.json");
        std::fs::create_dir_all(stable_data.parent().unwrap()).unwrap();
        std::fs::write(&stable_data, br#"{"keep":true}"#).unwrap();
        let token = write_current_app_test_token(&profile);
        let port_path = profile.join(DEVTOOLS_ACTIVE_PORT_FILE);
        std::fs::write(&port_path, b"9222\n/devtools/browser/test\n").unwrap();

        cleanup_browser_ownership_after_exact_shutdown(&token)
            .expect("matching exact token authorizes runtime-artifact cleanup");

        assert!(!ownership_marker_path(&profile).exists());
        assert!(!port_path.exists());
        assert_eq!(
            std::fs::read(&stable_data).unwrap(),
            br#"{"keep":true}"#,
            "normal shutdown must preserve stable profile data"
        );
    }

    #[test]
    fn exact_ephemeral_shutdown_removes_profile_with_marker_still_authoritative() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("ephemeral-profile-cleanup");
        let token = write_current_app_test_token(&profile);
        std::fs::write(
            profile.join(DEVTOOLS_ACTIVE_PORT_FILE),
            b"9222\n/devtools/browser/test\n",
        )
        .unwrap();
        std::fs::create_dir_all(profile.join("Default")).unwrap();
        std::fs::write(profile.join("Default").join("Cookies"), b"ephemeral").unwrap();

        cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile)
            .expect("exact token authorizes whole ephemeral profile cleanup");

        assert!(!profile.exists());
    }

    #[test]
    fn exact_ephemeral_cleanup_continues_across_small_bounded_batches() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("ephemeral-bounded-continuation");
        let token = write_current_app_test_token(&profile);
        let data = profile.join("Default").join("Cache");
        std::fs::create_dir_all(&data).unwrap();
        for index in 0..11 {
            std::fs::write(data.join(format!("entry-{index:02}.bin")), b"cache").unwrap();
        }
        let _limits = TestEphemeralDeleteLimitsGuard::install(EphemeralDeleteLimits {
            max_entries: 4,
            max_path_bytes: 4 * 1024,
        });

        let mut previous_files = 11_usize;
        let mut continuation_failures = 0_usize;
        for _ in 0..16 {
            match cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile) {
                Ok(()) => break,
                Err(error) if error == EPHEMERAL_DELETE_RETRY_REQUIRED => {
                    continuation_failures += 1;
                    assert!(
                        ownership_marker_path(&profile).is_file(),
                        "every partial batch must retain exact cleanup authority"
                    );
                    let remaining = std::fs::read_dir(&data)
                        .map(|entries| entries.count())
                        .unwrap_or(0);
                    assert!(
                        remaining < previous_files,
                        "each bounded retry must make monotonic progress: {remaining} !< {previous_files}"
                    );
                    previous_files = remaining;
                }
                Err(error) => panic!("unexpected bounded cleanup failure: {error}"),
            }
        }

        assert!(
            continuation_failures >= 2,
            "the small test limit must exercise multiple fresh-claim retries"
        );
        assert!(
            !profile.exists(),
            "the final batch removes the marker last and then the exact root"
        );
    }

    #[test]
    fn startup_recovery_finishes_all_bounded_batches_in_one_invocation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("oversized-owned-profile");
        let browser = identity(8_490, "chrome-gone");
        write_test_marker(
            &profile,
            identity(8_491, "nomifun-gone"),
            browser.clone(),
        );
        for index in 0..13 {
            std::fs::write(profile.join(format!("entry-{index:02}.bin")), b"cache").unwrap();
        }
        let _limits = TestEphemeralDeleteLimitsGuard::install(EphemeralDeleteLimits {
            max_entries: 4,
            max_path_bytes: 4 * 1024,
        });
        let mut control = fake_control(identity(999, "current"), HashMap::new());
        control.absence_result = Ok(true);

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(!profile.exists());
        assert_eq!(report.ephemeral_profiles_removed, 1);
        assert_eq!(report.failures, 0);
        assert_eq!(control.absence_identities, vec![browser]);
    }

    #[cfg(unix)]
    #[test]
    fn bounded_ephemeral_cleanup_unlinks_symlink_without_touching_external_tree() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::TempDir::new().unwrap();
        let outside = tmp.path().join("outside-profile-tree");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"preserve").unwrap();
        let profile = tmp.path().join("ephemeral-symlink-cleanup");
        let token = write_current_app_test_token(&profile);
        symlink(&outside, profile.join("linked-cache")).unwrap();

        cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile).unwrap();

        assert!(!profile.exists());
        assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }

    #[cfg(unix)]
    #[test]
    fn bounded_ephemeral_cleanup_progresses_across_nonempty_root_siblings() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("ephemeral-wide-directory-row");
        let token = write_current_app_test_token(&profile);
        for index in 0..4 {
            let directory = profile.join(format!("cache-{index}"));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("entry.bin"), b"cache").unwrap();
        }
        let _limits = TestEphemeralDeleteLimitsGuard::install(EphemeralDeleteLimits {
            max_entries: 4,
            max_path_bytes: 4 * 1024,
        });

        let first = cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile);
        assert_eq!(first.unwrap_err(), EPHEMERAL_DELETE_RETRY_REQUIRED);
        assert!(ownership_marker_path(&profile).is_file());
        let remaining_after_first = std::fs::read_dir(&profile)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| !is_ownership_record_name(&entry.file_name()))
            .count();
        assert!(
            remaining_after_first < 4,
            "the first bounded pass must delete at least one non-empty sibling"
        );

        for _ in 0..4 {
            match cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile) {
                Ok(()) => break,
                Err(error) if error == EPHEMERAL_DELETE_RETRY_REQUIRED => {}
                Err(error) => panic!("unexpected bounded sibling cleanup failure: {error}"),
            }
        }
        assert!(!profile.exists());
    }

    #[test]
    fn exact_ephemeral_shutdown_removes_append_only_record_pair() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("ephemeral-pair-cleanup");
        let token = write_current_app_ephemeral_pair_token(&profile);
        assert_eq!(
            token.marker_path,
            committed_ownership_record_path(&token.profile_dir),
            "token paths must stay in the held canonical profile namespace"
        );
        assert!(ownership_marker_path(&token.profile_dir).is_file());
        assert!(token.marker_path.is_file());
        std::fs::create_dir_all(profile.join("Default")).unwrap();
        std::fs::write(profile.join("Default").join("Cookies"), b"ephemeral").unwrap();

        cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile)
            .expect("the exact append-only pair authorizes whole-profile cleanup");

        assert!(!profile.exists());
    }

    #[test]
    fn committed_sidecar_alone_remains_authoritative_for_cleanup() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("ephemeral-sidecar-only");
        let token = write_current_app_ephemeral_pair_token(&profile);
        std::fs::remove_file(ownership_marker_path(&token.profile_dir))
            .expect("simulate crash after predecessor unlink");
        assert_eq!(read_marker(&token.profile_dir).unwrap(), token.marker);

        cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile)
            .expect("committed sidecar alone retains exact cleanup authority");

        assert!(!profile.exists());
    }

    #[cfg(windows)]
    #[test]
    fn append_only_pair_is_scanned_once_per_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("pair-live-owner");
        let token = write_current_app_ephemeral_pair_token(&profile);
        let owner = token.marker.owner_app.clone();
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(owner.pid, ProcessLookup::Found(owner))]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert_eq!(report.markers_scanned, 2);
        assert_eq!(report.live_owners_preserved, 1);
        assert_eq!(control.terminate_calls, 0);
        assert!(profile.exists());
    }

    #[cfg(windows)]
    #[test]
    fn late_root_entry_restores_committed_lineage_after_final_rmdir_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ancestor = tmp.path().join("managed-root");
        let profile = ancestor.join("late-root-entry");
        let moved_ancestor = tmp.path().join("moved-root");
        let moved_profile = ancestor.join("moved-profile");
        let token = write_current_app_ephemeral_pair_token(&profile);
        let hook_ancestor = ancestor.clone();
        let hook_moved_ancestor = moved_ancestor.clone();
        let hook_moved_profile = moved_profile.clone();
        FINAL_PROFILE_RMDIR_BARRIER_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |profile| {
                assert!(
                    std::fs::rename(&hook_ancestor, &hook_moved_ancestor).is_err(),
                    "held DELETE-capable profile handle must block ancestor replacement"
                );
                assert!(
                    std::fs::rename(profile, &hook_moved_profile).is_err(),
                    "held DELETE-capable profile handle must block leaf replacement"
                );
                std::fs::write(profile.join("late-entry"), b"late").unwrap();
            }));
        });

        let error = cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile)
            .expect_err("late entry must make the final rmdir fail closed");

        assert!(error.contains("delete exact"), "{error}");
        assert!(!error.contains("restore failed"), "{error}");
        assert_eq!(read_marker(&token.profile_dir).unwrap(), token.marker);
        assert!(profile.join("late-entry").is_file());
        cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile)
            .expect("restored lineage authorizes a later exact retry");
        assert!(!profile.exists());
    }

    #[test]
    fn exact_ephemeral_cleanup_rejects_a_replaced_non_directory_target() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("ephemeral-replaced-before-cleanup");
        let token = write_current_app_test_token(&profile);
        std::fs::remove_dir_all(&profile).unwrap();
        std::fs::write(&profile, b"replacement").unwrap();

        let error = cleanup_ephemeral_profile_after_exact_shutdown(&token, &profile)
            .expect_err("replaced non-directory target must fail closed");

        assert!(error.contains("not a regular directory"), "{error}");
        assert_eq!(std::fs::read(&profile).unwrap(), b"replacement");
    }

    #[test]
    fn exact_ephemeral_shutdown_reuses_held_launch_claim() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("ephemeral-held-launch-claim");
        std::fs::create_dir_all(&profile).unwrap();
        let launch_claim =
            prepare_ownership_marker_for_launch(&profile).expect("hold launch claim");
        let token = write_current_app_test_token(&profile);
        std::fs::write(
            profile.join(DEVTOOLS_ACTIVE_PORT_FILE),
            b"9222\n/devtools/browser/test\n",
        )
        .unwrap();

        cleanup_ephemeral_profile_after_exact_shutdown_under_launch_claim(
            &token,
            &profile,
            &launch_claim,
        )
        .expect("failed launch cleans its ephemeral profile under the existing claim");

        assert!(!profile.exists());
    }

    #[test]
    fn ephemeral_retry_recreates_only_the_exact_claimed_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("ephemeral-retry");
        std::fs::create_dir_all(&profile).unwrap();
        let launch_claim =
            prepare_ownership_marker_for_launch(&profile).expect("hold exact launch claim");
        let cleanup_token = claim_ephemeral_profile_cleanup(&profile, &launch_claim)
            .expect("bind exact ephemeral cleanup authority");

        cleanup_uncommitted_ephemeral_profile_after_exact_shutdown_under_launch_claim(
            &cleanup_token,
            &launch_claim,
        )
        .expect("completed first attempt removes its exact ephemeral profile");
        assert!(!profile.exists());

        let sibling = tmp.path().join("different-profile");
        assert!(
            restore_ephemeral_profile_for_retry(&sibling, &launch_claim).is_err(),
            "the held claim must not authorize a sibling directory"
        );
        assert!(!sibling.exists());

        let restored = restore_ephemeral_profile_for_retry(&profile, &launch_claim)
            .expect("same exact profile path can be restored under the held claim");
        assert!(profile.is_dir());
        assert_eq!(
            std::fs::canonicalize(&profile).unwrap(),
            launch_claim.0.profile_dir
        );
        cleanup_uncommitted_ephemeral_profile_after_exact_shutdown_under_launch_claim(
            &restored,
            &launch_claim,
        )
        .expect("restored cleanup authority remains exact");
        assert!(!profile.exists());
    }

    #[test]
    fn ephemeral_retry_rejects_a_replaced_non_directory_target() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("ephemeral-replaced-retry");
        std::fs::create_dir_all(&profile).unwrap();
        let launch_claim =
            prepare_ownership_marker_for_launch(&profile).expect("hold exact launch claim");
        let cleanup_token = claim_ephemeral_profile_cleanup(&profile, &launch_claim).unwrap();
        cleanup_uncommitted_ephemeral_profile_after_exact_shutdown_under_launch_claim(
            &cleanup_token,
            &launch_claim,
        )
        .unwrap();
        std::fs::write(&profile, b"replacement").unwrap();

        let error = restore_ephemeral_profile_for_retry(&profile, &launch_claim)
            .expect_err("a replaced file must fail closed");

        assert!(error.contains("not a regular directory"), "{error}");
        assert!(profile.is_file());
    }

    #[test]
    fn launch_port_preparation_removes_only_a_regular_runtime_artifact() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("launch-port-preparation");
        std::fs::create_dir_all(&profile).unwrap();
        let launch_claim = prepare_ownership_marker_for_launch(&profile).unwrap();
        let port = profile.join(DEVTOOLS_ACTIVE_PORT_FILE);
        std::fs::write(&port, b"9222\n/devtools/browser/stale\n").unwrap();

        prepare_runtime_port_for_launch(&profile, &launch_claim)
            .expect("regular stale port artifact is removed before spawn");
        assert!(!port.exists());

        std::fs::create_dir(&port).unwrap();
        let error = prepare_runtime_port_for_launch(&profile, &launch_claim)
            .expect_err("unsafe port artifact type must fail launch closed");
        assert!(error.contains("not a regular file"), "{error}");
        assert!(port.is_dir());
    }

    #[test]
    fn exact_shutdown_cleanup_reuses_the_held_launch_claim() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("held-launch-claim-cleanup");
        std::fs::create_dir_all(&profile).unwrap();
        let launch_claim =
            prepare_ownership_marker_for_launch(&profile).expect("hold launch claim");
        let token = write_current_app_test_token(&profile);
        let port_path = profile.join(DEVTOOLS_ACTIVE_PORT_FILE);
        std::fs::write(&port_path, b"9222\n/devtools/browser/test\n").unwrap();

        cleanup_browser_ownership_after_exact_shutdown_under_launch_claim(
            &token,
            &launch_claim,
        )
        .expect("committed launch failure must clean under its existing claim");

        assert!(!ownership_marker_path(&profile).exists());
        assert!(!port_path.exists());
    }

    #[test]
    fn exact_shutdown_cleanup_preserves_replaced_marker_and_port() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("stable-profile-replaced");
        let token = write_current_app_test_token(&profile);
        let port_path = profile.join(DEVTOOLS_ACTIVE_PORT_FILE);
        std::fs::write(&port_path, b"9222\n/devtools/browser/replaced\n").unwrap();
        let mut replacement = token.marker.clone();
        replacement.browser = identity(8_383, "replacement-chrome");
        std::fs::write(
            ownership_marker_path(&profile),
            serde_json::to_vec_pretty(&replacement).unwrap(),
        )
        .unwrap();

        let error = cleanup_browser_ownership_after_exact_shutdown(&token)
            .expect_err("a replaced marker must fail closed");

        assert!(error.contains("changed"), "{error}");
        assert_eq!(read_marker(&profile).unwrap(), replacement);
        assert!(port_path.is_file());
    }

    #[test]
    fn exact_shutdown_cleanup_missing_marker_never_deletes_port() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("stable-profile-missing-marker");
        let token = write_current_app_test_token(&profile);
        std::fs::remove_file(ownership_marker_path(&profile)).unwrap();
        let port_path = profile.join(DEVTOOLS_ACTIVE_PORT_FILE);
        std::fs::write(&port_path, b"9222\n/devtools/browser/unowned\n").unwrap();

        cleanup_browser_ownership_after_exact_shutdown(&token)
            .expect("missing marker is idempotent");

        assert!(
            port_path.is_file(),
            "a missing marker must never authorize deleting DevToolsActivePort"
        );
    }

    #[test]
    fn exact_shutdown_cleanup_rejects_unsafe_port_type_and_preserves_marker() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("stable-profile-unsafe-port");
        let token = write_current_app_test_token(&profile);
        let port_path = profile.join(DEVTOOLS_ACTIVE_PORT_FILE);
        std::fs::create_dir(&port_path).unwrap();

        let error = cleanup_browser_ownership_after_exact_shutdown(&token)
            .expect_err("a non-file port artifact must fail closed");

        assert!(error.contains("not a regular file"), "{error}");
        assert!(ownership_marker_path(&profile).is_file());
        assert!(port_path.is_dir());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_marker_writer_captures_the_exact_spawned_child_handle() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("profile-exact-child");
        std::fs::create_dir_all(&profile).unwrap();
        let claim =
            prepare_ownership_marker_for_launch(&profile).expect("exclusive launch claim");
        let command_shell = PathBuf::from(
            std::env::var_os("COMSPEC").expect("Windows COMSPEC must identify cmd.exe"),
        );
        let mut command = tokio::process::Command::new(&command_shell);
        command.args(["/D", "/S", "/C", "ping -n 6 127.0.0.1 >NUL"]);
        let mut child = command.spawn().expect("spawn exact marker test child");
        let expected_pid = child.id().expect("child pid");

        write_browser_ownership_marker(&claim, &profile, &command_shell, &child, None)
            .await
            .expect("write marker from exact child handle");
        let marker = read_marker(&profile).expect("read committed ownership marker");
        assert_eq!(marker.browser.pid, expected_pid);
        assert_eq!(
            marker.browser,
            windows_child_process_identity(&child).expect("same exact child identity")
        );

        let _ = child.kill().await;
        let _ = child.wait().await;
    }

    /// The executable-lineage guard must fail closed: when the spawned
    /// process's live image does not resolve to the configured browser
    /// executable (wrong binary, or a wrapper that exec'd into something
    /// else), no ownership marker may be committed for the impostor process.
    #[cfg(unix)]
    #[tokio::test]
    async fn marker_writer_rejects_spawned_executable_that_is_not_the_configured_browser() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("profile-lineage-mismatch");
        std::fs::create_dir_all(&profile).unwrap();
        let claim =
            prepare_ownership_marker_for_launch(&profile).expect("exclusive launch claim");
        // A real, observable process whose image is NOT the configured
        // browser below (the same directly-spawned sleeper the darwin
        // fixtures use, so the observed identity is the live image itself).
        let mut child = tokio::process::Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("spawn lineage-mismatch test child");
        let configured_browser = PathBuf::from("/bin/ls");

        let error = write_browser_ownership_marker(
            &claim,
            &profile,
            &configured_browser,
            &child,
            None,
        )
        .await
        .expect_err("a mismatched executable lineage must fail closed");

        assert!(
            error.contains("did not match configured browser"),
            "rejection must name the lineage mismatch: {error}"
        );
        assert!(
            !ownership_marker_path(&profile).exists(),
            "no ownership marker may be committed for an impostor process"
        );

        let _ = child.kill().await;
        let _ = child.wait().await;
    }

    #[test]
    fn profile_operation_lock_is_exclusive_and_released_on_drop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("profile-lock");
        std::fs::create_dir_all(&profile).unwrap();

        let first = ProfileOperationClaim::acquire(&profile).expect("first lock");
        assert!(
            ProfileOperationClaim::acquire(&profile).is_err(),
            "a second launch/recovery must not enter the same profile"
        );
        drop(first);
        ProfileOperationClaim::acquire(&profile).expect("OS releases lock with guard handle");
    }

    #[cfg(windows)]
    #[test]
    fn launch_claim_blocks_recovery_until_marker_commit_window_closes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("host-contended");
        write_test_marker(
            &profile,
            identity(181, "nomifun"),
            identity(292, "chrome"),
        );
        let launch_claim = ProfileOperationClaim::acquire(&profile).expect("launch claim");
        let mut control = fake_control(identity(999, "current"), HashMap::new());

        let blocked = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );
        assert!(profile.exists());
        assert_eq!(blocked.failures, 1);
        assert_eq!(blocked.ephemeral_profiles_removed, 0);

        drop(launch_claim);
        let recovered = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );
        assert!(!profile.exists());
        assert_eq!(recovered.ephemeral_profiles_removed, 1);
    }

    #[test]
    fn preferences_path_is_default_subdir() {
        let p = preferences_path(Path::new("/data/profile"));
        assert!(p.ends_with("Default/Preferences") || p.ends_with("Default\\Preferences"));
    }

    #[test]
    fn scrub_rewrites_crashed_to_normal() {
        let dirty = r#"{"profile":{"exit_type":"Crashed","name":"Person 1"},"other":42}"#;
        let out = scrub_prefs_json(dirty).unwrap().expect("changed → Some");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["profile"]["exit_type"], "Normal");
        // 红线：只动 exit_type，兄弟键 + 顶层键原样保留。
        assert_eq!(v["profile"]["name"], "Person 1");
        assert_eq!(v["other"], 42);
    }

    #[test]
    fn scrub_rewrites_session_ended_to_normal() {
        let dirty = r#"{"profile":{"exit_type":"SessionEnded"}}"#;
        let out = scrub_prefs_json(dirty).unwrap().expect("changed → Some");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["profile"]["exit_type"], "Normal");
    }

    #[test]
    fn scrub_noop_when_already_normal() {
        let clean = r#"{"profile":{"exit_type":"Normal"}}"#;
        assert!(scrub_prefs_json(clean).unwrap().is_none(), "already clean → None (no write)");
    }

    #[test]
    fn scrub_inserts_exit_type_when_profile_lacks_it() {
        let no_key = r#"{"profile":{"name":"x"}}"#;
        let out = scrub_prefs_json(no_key).unwrap().expect("inserted → Some");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["profile"]["exit_type"], "Normal");
        assert_eq!(v["profile"]["name"], "x");
    }

    #[test]
    fn scrub_creates_profile_object_when_missing() {
        let no_profile = r#"{"some":"thing"}"#;
        let out = scrub_prefs_json(no_profile).unwrap().expect("created profile → Some");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["profile"]["exit_type"], "Normal");
        assert_eq!(v["some"], "thing");
    }

    #[test]
    fn scrub_errs_on_bad_json() {
        assert!(scrub_prefs_json("not json").is_err());
    }

    #[test]
    fn scrub_errs_when_profile_is_not_object() {
        // profile 存在但是字符串 → 结构异常 → Err（调用方 best-effort 吞掉）。
        assert!(scrub_prefs_json(r#"{"profile":"oops"}"#).is_err());
    }

    #[test]
    fn scrub_crash_markers_roundtrips_on_disk() {
        let tmp = tempfile::TempDir::new().unwrap();
        let udd = tmp.path();
        let prefs = preferences_path(udd);
        std::fs::create_dir_all(prefs.parent().unwrap()).unwrap();
        std::fs::write(&prefs, r#"{"profile":{"exit_type":"Crashed"}}"#).unwrap();

        scrub_crash_markers(udd).expect("scrub ok");

        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&prefs).unwrap()).unwrap();
        assert_eq!(after["profile"]["exit_type"], "Normal");
        // 临时文件不残留。
        assert!(!prefs.with_extension("nomi-scrub.tmp").exists());
    }

    #[test]
    fn scrub_crash_markers_skips_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        // 无 Default/Preferences（首启）→ Ok，不报错、不建文件。
        scrub_crash_markers(tmp.path()).expect("missing file is benign");
        assert!(!preferences_path(tmp.path()).exists());
    }

    #[test]
    fn scrub_crash_markers_tolerates_corrupt_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prefs = preferences_path(tmp.path());
        std::fs::create_dir_all(prefs.parent().unwrap()).unwrap();
        std::fs::write(&prefs, "{ corrupt").unwrap();
        // 损坏 JSON → best-effort Ok（warn），不阻断启动；原文件不被破坏成空。
        scrub_crash_markers(tmp.path()).expect("corrupt json is best-effort benign");
        assert_eq!(std::fs::read_to_string(&prefs).unwrap(), "{ corrupt");
    }

    #[test]
    fn scrub_crash_markers_skips_oversized_preferences_without_a_temp_copy() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prefs = preferences_path(tmp.path());
        let preferences_dir = prefs.parent().unwrap();
        std::fs::create_dir_all(preferences_dir).unwrap();
        let source = std::fs::File::create(&prefs).unwrap();
        source.set_len(MAX_PREFERENCES_BYTES + 1).unwrap();
        drop(source);

        scrub_crash_markers(tmp.path()).expect("oversized Preferences is best-effort benign");

        assert_eq!(
            std::fs::metadata(&prefs).unwrap().len(),
            MAX_PREFERENCES_BYTES + 1,
            "the oversized source must be preserved verbatim"
        );
        let entries = std::fs::read_dir(preferences_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            vec![std::ffi::OsString::from(PREFERENCES_FILE)]
        );
    }

    #[test]
    fn scrub_ignores_a_precreated_legacy_fixed_temp_hardlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prefs = preferences_path(tmp.path());
        std::fs::create_dir_all(prefs.parent().unwrap()).unwrap();
        std::fs::write(&prefs, r#"{"profile":{"exit_type":"Crashed"}}"#).unwrap();
        let sentinel = tmp.path().join("outside-sentinel");
        std::fs::write(&sentinel, b"outside-must-not-change").unwrap();
        let legacy_temp = prefs.with_extension("nomi-scrub.tmp");
        std::fs::hard_link(&sentinel, &legacy_temp).unwrap();

        scrub_crash_markers(tmp.path()).expect("random create-new temp bypasses fixed alias");

        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"outside-must-not-change"
        );
        assert_eq!(
            std::fs::read(&legacy_temp).unwrap(),
            b"outside-must-not-change"
        );
    }

    #[test]
    fn scrub_rejects_a_hardlinked_preferences_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prefs = preferences_path(tmp.path());
        std::fs::create_dir_all(prefs.parent().unwrap()).unwrap();
        let outside = tmp.path().join("outside-preferences");
        let original = r#"{"profile":{"exit_type":"Crashed"},"outside":true}"#;
        std::fs::write(&outside, original).unwrap();
        std::fs::hard_link(&outside, &prefs).unwrap();

        scrub_crash_markers(tmp.path())
            .expect_err("multi-link Preferences must fail closed");

        assert_eq!(std::fs::read_to_string(&outside).unwrap(), original);
    }

    #[test]
    fn scrub_rejects_a_symlinked_preferences_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prefs = preferences_path(tmp.path());
        std::fs::create_dir_all(prefs.parent().unwrap()).unwrap();
        let outside = tmp.path().join("outside-symlink-preferences");
        let original = r#"{"profile":{"exit_type":"Crashed"},"outside":true}"#;
        std::fs::write(&outside, original).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(&outside, &prefs).is_err() {
            // Creating symlinks can require Developer Mode on Windows. The
            // hardlink test above still exercises the no-external-write rule.
            return;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &prefs).unwrap();

        scrub_crash_markers(tmp.path())
            .expect_err("symlinked Preferences must fail closed");

        assert_eq!(std::fs::read_to_string(&outside).unwrap(), original);
    }

    #[cfg(windows)]
    #[test]
    fn guarded_profile_handle_blocks_ancestor_namespace_rename() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ancestor = tmp.path().join("managed-root");
        let profile = ancestor.join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let guard =
            open_locked_non_reparse_directory(&profile).expect("open root profile guard");
        let moved = tmp.path().join("moved-root");

        assert!(
            std::fs::rename(&ancestor, &moved).is_err(),
            "an ancestor containing the guarded profile cannot be renamed"
        );
        drop(guard);
        std::fs::rename(&ancestor, &moved)
            .expect("ancestor rename succeeds after guard release");
    }

    #[cfg(windows)]
    #[test]
    fn pinned_recovery_authority_blocks_namespace_replacement_without_process_actions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ancestor = tmp.path().join("managed-root");
        let profile = ancestor.join("profile");
        write_test_marker(
            &profile,
            identity(301, "nomifun-gone"),
            identity(302, "chrome-gone"),
        );
        let claim =
            ProfileOperationClaim::acquire_pinned(&profile).expect("pin recovery profile");
        let authority =
            read_pinned_ownership_record_set(&profile).expect("pin recovery authority");
        claim
            .validates(&profile)
            .expect("pinned marker still occupies claimed profile");
        let moved_ancestor = tmp.path().join("moved-root");
        let moved_profile = ancestor.join("replacement");
        let control = fake_control(identity(999, "current"), HashMap::new());

        assert!(std::fs::rename(&ancestor, &moved_ancestor).is_err());
        assert!(std::fs::rename(&profile, &moved_profile).is_err());
        assert_eq!(control.terminate_calls, 0);
        assert_eq!(control.absence_calls, 0);
        assert!(control.terminate_identities.is_empty());
        assert!(control.absence_identities.is_empty());

        drop(authority);
        drop(claim);
        std::fs::rename(&ancestor, &moved_ancestor)
            .expect("namespace rename succeeds after pinned claim drops");
    }

    #[cfg(unix)]
    #[test]
    fn unix_directory_enumerations_have_independent_offsets() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("one"), b"1").unwrap();
        std::fs::write(tmp.path().join("two"), b"2").unwrap();
        let directory = UnixDirectory::open_path(tmp.path()).unwrap();
        let mut first_budget = UnixDirectoryEnumerationBudget::new(2, 6);
        let mut second_budget = UnixDirectoryEnumerationBudget::new(2, 6);
        let mut first = directory.entries_bounded(&mut first_budget).unwrap();
        let mut second = directory.entries_bounded(&mut second_budget).unwrap();
        first.sort();
        second.sort();

        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn unix_wide_directory_limit_closes_cursor_and_allows_complete_retry() {
        let tmp = tempfile::TempDir::new().unwrap();
        for index in 0..65 {
            std::fs::write(tmp.path().join(format!("entry-{index:03}")), b"x").unwrap();
        }
        let directory = UnixDirectory::open_path(tmp.path()).unwrap();
        let mut tight_budget = UnixDirectoryEnumerationBudget::new(64, 4 * 1024);

        assert!(
            directory.entries_bounded(&mut tight_budget).is_err(),
            "a wide directory must fail before retaining entry 65"
        );

        let mut retry_budget = UnixDirectoryEnumerationBudget::new(65, 4 * 1024);
        let retry = directory.entries_bounded(&mut retry_budget).unwrap();
        assert_eq!(retry.len(), 65);
        assert_eq!(retry_budget.entries, 65);
    }

    #[cfg(unix)]
    #[test]
    fn unix_long_name_is_rejected_before_owned_name_allocation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let long_name = "x".repeat(240);
        std::fs::write(tmp.path().join(&long_name), b"x").unwrap();
        let directory = UnixDirectory::open_path(tmp.path()).unwrap();
        let mut budget = UnixDirectoryEnumerationBudget::new(1, 239);

        assert!(directory.entries_bounded(&mut budget).is_err());
        assert_eq!(budget.entries, 0);
        assert_eq!(budget.name_bytes, 0);
    }

    #[cfg(unix)]
    #[test]
    fn unix_cleanup_budget_failure_preserves_marker_and_retry_authority() {
        let tmp = tempfile::TempDir::new().unwrap();
        let marker = tmp.path().join(OWNERSHIP_MARKER_FILE);
        let subtree = tmp.path().join("browser-data");
        std::fs::write(&marker, b"cleanup-authority").unwrap();
        std::fs::create_dir(&subtree).unwrap();
        std::fs::write(subtree.join("one"), b"1").unwrap();
        std::fs::write(subtree.join("two"), b"2").unwrap();
        let root = UnixDirectory::open_path(tmp.path()).unwrap();
        let mut tight_budget = EphemeralDeleteBudget::new(EphemeralDeleteLimits {
            max_entries: 2,
            max_path_bytes: 1024,
        });
        assert!(tight_budget.try_charge_unix_name(b"browser-data").unwrap());

        assert_eq!(
            remove_unix_profile_entry_tree(
                &root,
                std::ffi::OsStr::new("browser-data"),
                root.device,
                0,
                &mut tight_budget,
            )
            .unwrap(),
            EphemeralDeleteProgress::MoreWork
        );
        assert_eq!(std::fs::read(&marker).unwrap(), b"cleanup-authority");
        assert_eq!(
            std::fs::read_dir(&subtree).unwrap().count(),
            1,
            "the bounded pass commits safe progress before yielding"
        );

        let mut retry_budget = EphemeralDeleteBudget::new(EphemeralDeleteLimits {
            max_entries: 2,
            max_path_bytes: 1024,
        });
        assert!(retry_budget.try_charge_unix_name(b"browser-data").unwrap());
        assert_eq!(
            remove_unix_profile_entry_tree(
                &root,
                std::ffi::OsStr::new("browser-data"),
                root.device,
                0,
                &mut retry_budget,
            )
            .unwrap(),
            EphemeralDeleteProgress::Complete
        );

        assert!(!subtree.exists());
        assert_eq!(std::fs::read(&marker).unwrap(), b"cleanup-authority");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_openat2_rejects_same_filesystem_bind_mounts() {
        struct BindMountGuard(PathBuf);
        impl Drop for BindMountGuard {
            fn drop(&mut self) {
                let _ = std::process::Command::new("umount")
                    .arg(&self.0)
                    .status();
            }
        }

        let tmp = tempfile::TempDir::new().unwrap();
        let profile = tmp.path().join("profile");
        let mounted = profile.join("mounted");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&mounted).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"must-survive").unwrap();
        let status = std::process::Command::new("mount")
            .args(["--bind"])
            .arg(&outside)
            .arg(&mounted)
            .status();
        let Ok(status) = status else {
            return;
        };
        if !status.success() {
            // Unprivileged developer environments cannot create bind mounts.
            // Privileged Linux CI executes the actual boundary assertion.
            return;
        }
        let _guard = BindMountGuard(mounted.clone());
        let directory = UnixDirectory::open_path(&profile).unwrap();

        assert!(
            directory
                .open_child(std::ffi::OsStr::new("mounted"))
                .is_err(),
            "RESOLVE_NO_XDEV must reject even a same-filesystem bind mount"
        );
        assert_eq!(
            std::fs::read(outside.join("sentinel")).unwrap(),
            b"must-survive"
        );
    }
}
