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

#[cfg(any(windows, test))]
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// 默认 profile 子目录。本引擎单 profile 启动（不传 `--profile-directory`），故恒 `Default`。
pub const DEFAULT_PROFILE_SUBDIR: &str = "Default";
/// profile 偏好文件名。
pub const PREFERENCES_FILE: &str = "Preferences";
/// `profile.exit_type` 的干净值（对应 Chromium `ExitType::kClean`）。
const EXIT_TYPE_NORMAL: &str = "Normal";

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
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()), // 首启，无 profile
        Err(e) => return Err(e),
    };
    match scrub_prefs_json(&text) {
        Ok(Some(new_text)) => {
            // 原子回写：写同目录临时文件再 rename（同卷 rename 是原子替换）。
            let tmp = path.with_extension("nomi-scrub.tmp");
            std::fs::write(&tmp, new_text)?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
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

/// Ownership marker written into every application-managed Chromium
/// `--user-data-dir`.
///
/// The payload deliberately contains only process ownership data. It never
/// stores a CDP endpoint, websocket URL, cookie, lease, token, or other secret.
pub const OWNERSHIP_MARKER_FILE: &str = ".nomifun-browser-owner.json";

const OWNERSHIP_MARKER_VERSION: u32 = 1;
const MAX_MARKER_SCAN_DEPTH: usize = 4;
#[cfg(not(windows))]
const PROCESS_DISCOVERY_RETRIES: usize = 40;
#[cfg(not(windows))]
const PROCESS_DISCOVERY_INTERVAL: Duration = Duration::from_millis(25);
const PROCESS_TREE_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_TREE_CONFIRM_INTERVAL: Duration = Duration::from_millis(25);
const PROCESS_TREE_ABSENCE_CONFIRMATIONS: usize = 2;
const PROFILE_OPERATION_LOCK_PREFIX: &str = ".nomifun-browser-operation";

static MANAGED_APP_INSTANCE_ID: OnceLock<String> = OnceLock::new();

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

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserOwnershipMarker {
    version: u32,
    app_instance_id: String,
    owner_app: ProcessIdentity,
    browser: ProcessIdentity,
    profile_id: String,
}

impl std::fmt::Debug for BrowserOwnershipMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserOwnershipMarker")
            .field("version", &self.version)
            .field("app_instance_id_configured", &!self.app_instance_id.is_empty())
            .field("owner_app", &self.owner_app)
            .field("browser", &self.browser)
            .field("profile_id_configured", &!self.profile_id.is_empty())
            .finish()
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
    fn terminate_tree(&mut self, expected: &ProcessIdentity) -> Result<usize, String>;
    fn confirm_tree_absent(&mut self, expected: &ProcessIdentity) -> Result<bool, String>;
}

struct SystemProcessControl;

struct ProfileOperationClaim {
    file: std::fs::File,
    profile_dir: PathBuf,
}

impl ProfileOperationClaim {
    fn acquire(profile_dir: &Path) -> Result<Self, String> {
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
        Ok(Self {
            file,
            profile_dir: canonical_profile,
        })
    }

    fn validates(&self, profile_dir: &Path) -> Result<(), String> {
        let canonical = std::fs::canonicalize(profile_dir)
            .map_err(|error| format!("canonicalize claimed browser profile: {error}"))?;
        if canonical != self.profile_dir {
            return Err("browser profile operation claim belongs to a different directory".into());
        }
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

    fn terminate_tree(&mut self, expected: &ProcessIdentity) -> Result<usize, String> {
        terminate_process_tree(expected)
    }

    fn confirm_tree_absent(&mut self, expected: &ProcessIdentity) -> Result<bool, String> {
        confirm_process_tree_absent(expected)
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

fn read_marker(profile_dir: &Path) -> Result<BrowserOwnershipMarker, String> {
    let path = ownership_marker_path(profile_dir);
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("read ownership marker metadata: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("ownership marker is not a regular file".into());
    }
    let bytes =
        std::fs::read(&path).map_err(|error| format!("read ownership marker: {error}"))?;
    let marker: BrowserOwnershipMarker = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse ownership marker: {error}"))?;
    validate_marker(&marker, profile_dir)?;
    Ok(marker)
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

fn prepare_ownership_marker_under_claim(
    profile_dir: &Path,
    claim: &ProfileLaunchClaim,
) -> Result<(), String> {
    claim.0.validates(profile_dir)?;
    let path = ownership_marker_path(profile_dir);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("inspect existing ownership marker: {error}")),
        Ok(_) => {}
    }

    let marker = read_marker(profile_dir)?;
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
            std::fs::remove_file(path)
                .map_err(|error| format!("remove completed ownership marker: {error}"))
        }
    }
}

/// Write a secret-free ownership marker immediately after Chromium spawn.
///
/// The observed process executable must resolve to the configured executable;
/// the marker records the observed creation identity, not caller-supplied PID
/// metadata. The caller must kill the newly spawned child if this returns an
/// error.
pub async fn write_browser_ownership_marker(
    claim: &ProfileLaunchClaim,
    profile_dir: &Path,
    expected_executable: &Path,
    child: &tokio::process::Child,
) -> Result<(), String> {
    claim.0.validates(profile_dir)?;
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
        app_instance_id: managed_app_instance_id().to_owned(),
        owner_app,
        browser,
        profile_id,
    };
    commit_ownership_marker(profile_dir, &marker)
}

fn commit_ownership_marker(
    profile_dir: &Path,
    marker: &BrowserOwnershipMarker,
) -> Result<(), String> {
    validate_marker(marker, profile_dir)?;
    let marker_path = ownership_marker_path(profile_dir);
    if marker_path.exists() {
        return Err("ownership marker unexpectedly appeared during browser spawn".into());
    }
    let temp_path = profile_dir.join(format!(
        "{OWNERSHIP_MARKER_FILE}.{}.{}.tmp",
        marker.app_instance_id, marker.browser.pid
    ));
    let bytes = serde_json::to_vec_pretty(&marker)
        .map_err(|error| format!("serialize ownership marker: {error}"))?;
    let write_result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|error| format!("create ownership marker temp file: {error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("write ownership marker temp file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("flush ownership marker temp file: {error}"))?;
        std::fs::rename(&temp_path, &marker_path)
            .map_err(|error| format!("commit ownership marker: {error}"))
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    write_result
}

fn collect_marker_paths(root: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut markers = Vec::new();
    let mut errors = Vec::new();
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (markers, errors);
        }
        Err(error) => {
            errors.push(format!("inspect browser profile recovery root: {error}"));
            return (markers, errors);
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            errors.push("browser profile recovery root is not a regular directory".into());
            return (markers, errors);
        }
        Ok(_) => {}
    }
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    while let Some((directory, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                errors.push(format!("scan browser profile directory: {error}"));
                continue;
            }
        };
        for entry in entries {
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
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_file() && entry.file_name() == OWNERSHIP_MARKER_FILE {
                markers.push(entry.path());
            } else if file_type.is_dir() && depth < MAX_MARKER_SCAN_DEPTH {
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    markers.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    (markers, errors)
}

fn cleanup_recovered_profile(
    recovery_root: &Path,
    profile_dir: &Path,
    mode: ProfileRecoveryMode,
) -> Result<(), String> {
    let canonical_root = std::fs::canonicalize(recovery_root)
        .map_err(|error| format!("canonicalize browser recovery root: {error}"))?;
    let canonical_profile = std::fs::canonicalize(profile_dir)
        .map_err(|error| format!("canonicalize recovered profile: {error}"))?;
    if !canonical_profile.starts_with(&canonical_root) {
        return Err("refusing to clean a profile outside the canonical recovery root".into());
    }
    match mode {
        ProfileRecoveryMode::DeleteEphemeralProfile => {
            if canonical_profile == canonical_root {
                return Err("refusing to remove a profile outside the ephemeral recovery root".into());
            }
            let metadata = std::fs::symlink_metadata(&canonical_profile)
                .map_err(|error| format!("inspect recovered ephemeral profile: {error}"))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("recovered ephemeral profile is not a regular directory".into());
            }
            let (nested_markers, scan_errors) = collect_marker_paths(&canonical_profile);
            if let Some(error) = scan_errors.into_iter().next() {
                return Err(format!(
                    "refusing recursive cleanup after an incomplete nested marker scan: {error}"
                ));
            }
            let expected_marker = ownership_marker_path(&canonical_profile);
            if nested_markers.len() != 1 || nested_markers[0] != expected_marker {
                return Err(
                    "refusing recursive cleanup while a descendant ownership marker remains"
                        .into(),
                );
            }
            std::fs::remove_dir_all(&canonical_profile)
                .map_err(|error| format!("remove recovered ephemeral profile: {error}"))
        }
        ProfileRecoveryMode::PreserveStableProfile => {
            std::fs::remove_file(ownership_marker_path(&canonical_profile))
                .map_err(|error| format!("clear recovered stable profile marker: {error}"))
        }
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

fn recover_owned_profiles_with(
    recovery_root: &Path,
    mode: ProfileRecoveryMode,
    control: &mut dyn ProcessControl,
) -> ProfileRecoveryReport {
    let (markers, scan_errors) = collect_marker_paths(recovery_root);
    let mut report = ProfileRecoveryReport::default();
    for error in scan_errors {
        report.failures += 1;
        tracing::warn!(
            target: "nomi_browser_engine::profile",
            reason = "recovery_scan_incomplete",
            "browser orphan recovery scan was incomplete; affected profiles were preserved"
        );
        let _ = error;
    }

    for marker_path in markers {
        report.markers_scanned += 1;
        let Some(profile_dir) = marker_path.parent() else {
            report.failures += 1;
            report.profiles_preserved += 1;
            continue;
        };
        let marker = match read_marker(profile_dir) {
            Ok(marker) => marker,
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

        match control.lookup(marker.owner_app.pid) {
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
            ProcessLookup::Missing | ProcessLookup::Found(_) => {}
        }

        let _operation_claim = match ProfileOperationClaim::acquire(profile_dir) {
            Ok(claim) => claim,
            Err(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    reason = "profile_operation_claim_unavailable",
                    "browser profile is being launched or recovered elsewhere; profile preserved"
                );
                let _ = error;
                continue;
            }
        };
        match read_marker(profile_dir) {
            Ok(current) if current == marker => {}
            Ok(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    reason = "ownership_changed_before_claim",
                    "browser ownership changed before recovery claim; profile preserved"
                );
                continue;
            }
            Err(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    reason = "marker_revalidation_failed_before_recovery",
                    "browser ownership marker disappeared before recovery claim; profile preserved"
                );
                let _ = error;
                continue;
            }
        }

        let mut terminated = false;
        let tree_absent = match control.lookup(marker.browser.pid) {
            ProcessLookup::Found(observed) if same_process(&marker.browser, &observed) => {
                match control.terminate_tree(&marker.browser) {
                    Ok(terminated_count) => {
                        terminated = true;
                        report.process_trees_terminated += 1;
                        tracing::warn!(
                            target: "nomi_browser_engine::profile",
                            terminated_processes = terminated_count,
                            browser_pid = marker.browser.pid,
                            reason = "verified_orphan",
                            "terminated a verified orphan browser process tree"
                        );
                        true
                    }
                    Err(error) => {
                        report.failures += 1;
                        report.profiles_preserved += 1;
                        tracing::warn!(
                            target: "nomi_browser_engine::profile",
                            browser_pid = marker.browser.pid,
                            reason = "termination_unconfirmed",
                            "orphan browser termination was not confirmed; profile preserved"
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
                    "browser PID was reused by a different process identity; reused process was not signaled and profile was preserved"
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

        match read_marker(profile_dir) {
            Ok(current) if current == marker => {}
            Ok(_) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    reason = "ownership_changed_after_recovery",
                    "browser ownership changed after process recovery; profile preserved"
                );
                continue;
            }
            Err(error) => {
                report.failures += 1;
                report.profiles_preserved += 1;
                tracing::warn!(
                    target: "nomi_browser_engine::profile",
                    reason = "marker_revalidation_failed_after_recovery",
                    "browser ownership marker could not be revalidated after process recovery; profile preserved"
                );
                let _ = error;
                continue;
            }
        }

        if let Err(error) = cleanup_recovered_profile(recovery_root, profile_dir, mode) {
            report.failures += 1;
            report.profiles_preserved += 1;
            tracing::warn!(
                target: "nomi_browser_engine::profile",
                terminated,
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
    report
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct DescendantIdentity {
    parent: ProcessIdentity,
    identity: ProcessIdentity,
}

#[cfg(windows)]
fn snapshot_descendants(
    system: &sysinfo::System,
    root: &ProcessIdentity,
) -> Result<Vec<DescendantIdentity>, String> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for process in system.processes().values() {
        if let Some(parent) = process.parent() {
            children
                .entry(parent.as_u32())
                .or_default()
                .push(process.pid().as_u32());
        }
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = vec![(root.clone(), 0_usize)];
    while let Some((parent, depth)) = pending.pop() {
        let Some(child_pids) = children.get(&parent.pid) else {
            continue;
        };
        for child_pid in child_pids {
            if !seen.insert(*child_pid) {
                return Err("cycle detected in browser process tree".into());
            }
            let process = system
                .process(sysinfo::Pid::from_u32(*child_pid))
                .ok_or_else(|| format!("process {child_pid} disappeared from its own snapshot"))?;
            let identity = process_identity(process)?;
            if identity.platform_start_key < parent.platform_start_key {
                return Err(format!(
                    "browser child {} predates immediate parent {}",
                    identity.pid, parent.pid
                ));
            }
            pending.push((identity.clone(), depth + 1));
            result.push(DescendantIdentity {
                parent: parent.clone(),
                identity,
            });
        }
    }
    Ok(result)
}

#[cfg(windows)]
struct AnchoredWindowsProcess {
    edge: Option<DescendantIdentity>,
    identity: ProcessIdentity,
    handle: nomi_process_runtime::WindowsExactProcess,
}

#[cfg(windows)]
fn validate_windows_anchor_snapshot(
    system: &sysinfo::System,
    expected_root: &ProcessIdentity,
    anchored: &[AnchoredWindowsProcess],
) -> Result<(), String> {
    let root = system
        .process(sysinfo::Pid::from_u32(expected_root.pid))
        .ok_or_else(|| "browser root disappeared during Job collection".to_string())?;
    let observed = process_identity(root)?;
    if !same_process(expected_root, &observed) {
        return Err("browser root PID changed identity during Job collection".into());
    }
    for anchor in anchored {
        let Some(edge) = &anchor.edge else {
            continue;
        };
        let process = system
            .process(sysinfo::Pid::from_u32(anchor.identity.pid))
            .ok_or_else(|| {
                format!(
                    "browser descendant {} disappeared during Job collection",
                    anchor.identity.pid
                )
            })?;
        let observed = process_identity(process)?;
        if !same_process(&anchor.identity, &observed) {
            return Err(format!(
                "browser descendant {} changed identity during Job collection",
                anchor.identity.pid
            ));
        }
        if process.parent().map(sysinfo::Pid::as_u32) != Some(edge.parent.pid) {
            return Err(format!(
                "browser descendant {} changed its recorded parent edge",
                anchor.identity.pid
            ));
        }
        let parent = system
            .process(sysinfo::Pid::from_u32(edge.parent.pid))
            .ok_or_else(|| {
                format!(
                    "browser descendant {} lost its verified parent during Job collection",
                    anchor.identity.pid
                )
            })?;
        let observed_parent = process_identity(parent)?;
        if !same_process(&edge.parent, &observed_parent) {
            return Err(format!(
                "browser descendant {} refers to a reused parent PID",
                anchor.identity.pid
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn validate_windows_candidate_snapshot(
    system: &sysinfo::System,
    candidate: &DescendantIdentity,
) -> Result<(), String> {
    let process = system
        .process(sysinfo::Pid::from_u32(candidate.identity.pid))
        .ok_or_else(|| {
            format!(
                "new browser descendant {} disappeared before Job assignment",
                candidate.identity.pid
            )
        })?;
    let observed = process_identity(process)?;
    if !same_process(&candidate.identity, &observed) {
        return Err(format!(
            "new browser descendant {} changed identity before Job assignment",
            candidate.identity.pid
        ));
    }
    if process.parent().map(sysinfo::Pid::as_u32) != Some(candidate.parent.pid) {
        return Err(format!(
            "new browser descendant {} changed parent before Job assignment",
            candidate.identity.pid
        ));
    }
    let parent = system
        .process(sysinfo::Pid::from_u32(candidate.parent.pid))
        .ok_or_else(|| {
            format!(
                "new browser descendant {} lost its verified parent before Job assignment",
                candidate.identity.pid
            )
        })?;
    let observed_parent = process_identity(parent)?;
    if !same_process(&candidate.parent, &observed_parent) {
        return Err(format!(
            "new browser descendant {} refers to a reused parent PID",
            candidate.identity.pid
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn terminate_process_tree(expected: &ProcessIdentity) -> Result<usize, String> {
    let system = fresh_process_snapshot()?;
    let root_process = system
        .process(sysinfo::Pid::from_u32(expected.pid))
        .ok_or_else(|| "browser root disappeared before termination".to_string())?;
    let observed_root = process_identity(root_process)?;
    if !same_process(expected, &observed_root) {
        return Err("browser root identity changed before termination".into());
    }
    let descendants = snapshot_descendants(&system, &observed_root)?;

    // Open and validate every exact handle before any destructive call. A PID
    // reuse after this point cannot retarget these handles.
    let root_handle = nomi_process_runtime::WindowsExactProcess::open_for_recovery(expected.pid)
        .map_err(|error| format!("open verified browser root for recovery: {error}"))?;
    if !same_process(
        expected,
        &process_identity_from_windows(root_handle.identity())?,
    ) {
        return Err("browser root handle identity changed before termination".into());
    }
    let mut job = nomi_process_runtime::WindowsRecoveryJob::new_unarmed()
        .map_err(|error| format!("create recovery Job: {error}"))?;
    job.assign(&root_handle)
        .map_err(|error| format!("assign verified browser root to recovery Job: {error}"))?;
    let mut anchored = vec![AnchoredWindowsProcess {
        edge: None,
        identity: expected.clone(),
        handle: root_handle,
    }];
    for descendant in descendants {
        if descendant.identity.platform_start_key < expected.platform_start_key {
            return Err("browser descendant predates its verified root".into());
        }
        let handle = nomi_process_runtime::WindowsExactProcess::open_for_recovery(
            descendant.identity.pid,
        )
        .map_err(|error| {
            format!(
                "open browser descendant {} for recovery: {error}",
                descendant.identity.pid
            )
        })?;
        if !same_process(
            &descendant.identity,
            &process_identity_from_windows(handle.identity())?,
        ) {
            return Err(format!(
                "browser descendant {} changed identity before termination",
                descendant.identity.pid
            ));
        }
        job.assign(&handle).map_err(|error| {
            format!(
                "assign verified browser descendant {} to recovery Job: {error}",
                descendant.identity.pid
            )
        })?;
        anchored.push(AnchoredWindowsProcess {
            identity: descendant.identity.clone(),
            edge: Some(descendant),
            handle,
        });
    }
    validate_windows_anchor_snapshot(&fresh_process_snapshot()?, expected, &anchored)?;

    // Root is now in the Job, so its future children inherit membership. Scan
    // twice for descendants which raced before their parent was assigned.
    let mut stable_rounds = 0;
    while stable_rounds < PROCESS_TREE_ABSENCE_CONFIRMATIONS {
        let system = fresh_process_snapshot()?;
        validate_windows_anchor_snapshot(&system, expected, &anchored)?;
        let mut candidates = Vec::new();
        for descendant in snapshot_descendants(&system, expected)? {
            if anchored
                .iter()
                .any(|known| known.identity.pid == descendant.identity.pid)
            {
                continue;
            }
            if descendant.identity.platform_start_key < expected.platform_start_key {
                return Err("new browser descendant predates its verified root".into());
            }
            let handle = nomi_process_runtime::WindowsExactProcess::open_for_recovery(
                descendant.identity.pid,
            )
            .map_err(|error| {
                format!(
                    "open new browser descendant {} for recovery: {error}",
                    descendant.identity.pid
                )
            })?;
            if !same_process(
                &descendant.identity,
                &process_identity_from_windows(handle.identity())?,
            ) {
                return Err(format!(
                    "new browser descendant {} changed identity before Job assignment",
                    descendant.identity.pid
                ));
            }
            candidates.push((descendant, handle));
        }
        if candidates.is_empty() {
            stable_rounds += 1;
        } else {
            let validation = fresh_process_snapshot()?;
            validate_windows_anchor_snapshot(&validation, expected, &anchored)?;
            for (candidate, handle) in candidates {
                validate_windows_candidate_snapshot(&validation, &candidate)?;
                job.assign(&handle).map_err(|error| {
                    format!(
                        "assign new browser descendant {} to recovery Job: {error}",
                        candidate.identity.pid
                    )
                })?;
                anchored.push(AnchoredWindowsProcess {
                    identity: candidate.identity.clone(),
                    edge: Some(candidate),
                    handle,
                });
            }
            stable_rounds = 0;
        }
        std::thread::sleep(PROCESS_TREE_CONFIRM_INTERVAL);
    }

    job.arm_kill_on_close()
        .map_err(|error| format!("arm recovery Job kill-on-close: {error}"))?;
    job.terminate_and_wait(
        anchored.iter().map(|process| &process.handle),
        PROCESS_TREE_CONFIRM_TIMEOUT,
    )
    .map_err(|error| format!("wait for recovery Job and verified process exit: {error}"))?;

    if !confirm_process_tree_absent(expected)? {
        return Err("a browser process-tree survivor remained outside the recovery Job".into());
    }
    Ok(anchored.len())
}

#[cfg(unix)]
fn snapshot_process_group(
    system: &sysinfo::System,
    process_group_id: u32,
) -> Result<Vec<ProcessIdentity>, String> {
    if process_group_id == 0 {
        return Err("cannot inspect process group 0".into());
    }
    let mut members = Vec::new();
    for process in system.processes().values() {
        let pid = process.pid().as_u32();
        // SAFETY: getpgid is a read-only query for a PID from this fresh
        // snapshot.
        let observed_group = unsafe { libc::getpgid(pid as libc::pid_t) };
        if observed_group < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            return Err(format!("query process group for PID {pid}: {error}"));
        }
        if observed_group as u32 == process_group_id {
            members.push(process_identity(process)?);
        }
    }
    Ok(members)
}

#[cfg(target_os = "linux")]
fn terminate_process_tree(expected: &ProcessIdentity) -> Result<usize, String> {
    if expected.process_group_id != Some(expected.pid) {
        return Err("browser marker does not identify a process-group leader".into());
    }
    // SAFETY: getpgrp only reads the current process-group id.
    let app_group = unsafe { libc::getpgrp() };
    if app_group <= 0 || expected.process_group_id == Some(app_group as u32) {
        return Err("browser process group overlaps the current application group".into());
    }
    let system = fresh_process_snapshot()?;
    let root_process = system
        .process(sysinfo::Pid::from_u32(expected.pid))
        .ok_or_else(|| "browser root disappeared before termination".to_string())?;
    let observed_root = process_identity(root_process)?;
    if !same_process(expected, &observed_root) {
        return Err("browser root identity changed before termination".into());
    }
    let members = snapshot_process_group(&system, expected.pid)?;
    if !members
        .iter()
        .any(|identity| same_process(expected, identity))
    {
        return Err("verified browser root is absent from its process group".into());
    }
    for identity in &members {
        if identity.platform_start_key < expected.platform_start_key {
            return Err("browser process-group member predates its verified root".into());
        }
    }

    let mut root_anchor = nomi_process_runtime::LinuxProcessGroupAnchor::open(expected.pid)
        .map_err(|error| format!("open exact browser pidfd for recovery: {error}"))?;

    // Opening a pidfd is itself PID-based, so bind it back to the full marker
    // identity before any signal. A PID reuse between snapshot and pidfd_open
    // is therefore quarantined.
    let system = fresh_process_snapshot()?;
    let root_process = system
        .process(sysinfo::Pid::from_u32(expected.pid))
        .ok_or_else(|| "browser root disappeared after opening its pidfd".to_string())?;
    let observed_root = process_identity(root_process)?;
    if !same_process(expected, &observed_root) {
        return Err("browser root identity changed after opening its pidfd".into());
    }

    // Stop the exact anchored root before using a numeric group signal. While
    // the pidfd is held, the kernel PID object remains allocated; while the
    // root is stopped it cannot voluntarily exit or create a reuse window.
    // Drop resumes it on every failure path before the destructive group kill.
    root_anchor
        .stop()
        .map_err(|error| format!("stop exact browser process before recovery: {error}"))?;
    let system = fresh_process_snapshot()?;
    let root_process = system
        .process(sysinfo::Pid::from_u32(expected.pid))
        .ok_or_else(|| "browser root disappeared after exact pidfd stop".to_string())?;
    let observed_root = process_identity(root_process)?;
    if !same_process(expected, &observed_root) {
        return Err("browser root identity changed after exact pidfd stop".into());
    }
    // SAFETY: getpgid is a read-only query for the exact, anchored root.
    let observed_group = root_anchor
        .process_group_id()
        .map_err(|error| format!("query anchored browser process group: {error}"))?;
    if observed_group != expected.pid {
        return Err("anchored browser root changed process group before termination".into());
    }

    // SAFETY: PID 0 and non-leader markers were rejected, the exact root was
    // revalidated after SIGSTOP, and its pidfd prevents this numeric PGID from
    // being recycled until after the signal.
    root_anchor
        .terminate_group()
        .map_err(|error| format!("signal pidfd-anchored browser process group: {error}"))?;
    if !confirm_process_tree_absent(expected)? {
        return Err("verified browser process group did not become absent".into());
    }
    Ok(members.len())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn terminate_process_tree(_expected: &ProcessIdentity) -> Result<usize, String> {
    Err(
        "safe orphan process-tree termination requires an exact process handle; this Unix platform preserves the profile fail-closed"
            .into(),
    )
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
    let descendants = snapshot_descendants(&system, expected)?;
    Ok(descendants.is_empty())
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
    let system = fresh_process_snapshot()?;
    Ok(snapshot_process_group(&system, pgid)?.is_empty())
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
        std::thread::sleep(PROCESS_TREE_CONFIRM_INTERVAL);
    }
    Ok(false)
}

/// Compatibility shim for older callers. `max_age` is intentionally ignored:
/// recovery is ownership-marker based and never deletes by mtime.
pub fn gc_stale_profiles(profiles_root: &Path, _max_age: Duration) {
    let report = recover_owned_profiles(
        profiles_root,
        ProfileRecoveryMode::DeleteEphemeralProfile,
    );
    if report.failures > 0 || report.profiles_preserved > 0 {
        tracing::warn!(
            target: "nomi_browser_engine::profile",
            summary = %report.safety_summary(),
            "browser profile compatibility recovery preserved unverified data"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FakeProcessControl {
        current: ProcessIdentity,
        processes: HashMap<u32, ProcessLookup>,
        terminate_result: Result<usize, String>,
        absence_result: Result<bool, String>,
        terminate_calls: usize,
        absence_calls: usize,
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

        fn terminate_tree(&mut self, _expected: &ProcessIdentity) -> Result<usize, String> {
            self.terminate_calls += 1;
            self.terminate_result.clone()
        }

        fn confirm_tree_absent(&mut self, _expected: &ProcessIdentity) -> Result<bool, String> {
            self.absence_calls += 1;
            self.absence_result.clone()
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

    fn fake_control(
        current: ProcessIdentity,
        processes: HashMap<u32, ProcessLookup>,
    ) -> FakeProcessControl {
        FakeProcessControl {
            current,
            processes,
            terminate_result: Ok(1),
            absence_result: Ok(true),
            terminate_calls: 0,
            absence_calls: 0,
        }
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
    fn exact_orphan_is_terminated_and_ephemeral_profile_removed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("host-orphan");
        let browser = identity(232, "chrome");
        write_test_marker(&profile, identity(121, "nomifun"), browser.clone());
        std::fs::write(profile.join("cache.bin"), b"cache").unwrap();
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(browser))]),
        );
        control.terminate_result = Ok(4);

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(!profile.exists());
        assert_eq!(control.terminate_calls, 1);
        assert_eq!(report.process_trees_terminated, 1);
        assert_eq!(report.ephemeral_profiles_removed, 1);
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
            HashMap::from([
                (
                    outer_browser.pid,
                    ProcessLookup::Found(outer_browser.clone()),
                ),
                (inner_owner.pid, ProcessLookup::Found(inner_owner)),
            ]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(outer.exists());
        assert!(inner.join("Cookies").exists());
        assert!(ownership_marker_path(&inner).exists());
        assert_eq!(control.terminate_calls, 1);
        assert_eq!(report.live_owners_preserved, 1);
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert_eq!(report.profiles_preserved, 1);
        assert_eq!(report.failures, 1);
    }

    #[test]
    fn unconfirmed_termination_preserves_ephemeral_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("host-stuck");
        let browser = identity(242, "chrome");
        write_test_marker(&profile, identity(131, "nomifun"), browser.clone());
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(browser))]),
        );
        control.terminate_result = Err("wait deadline expired".into());

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::DeleteEphemeralProfile,
            &mut control,
        );

        assert!(profile.exists());
        assert_eq!(control.terminate_calls, 1);
        assert_eq!(report.ephemeral_profiles_removed, 0);
        assert_eq!(report.failures, 1);
    }

    #[test]
    fn already_absent_tree_can_release_ephemeral_profile() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("ephemeral");
        let profile = root.join("host-gone");
        write_test_marker(
            &profile,
            identity(141, "nomifun"),
            identity(252, "chrome"),
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
        assert_eq!(report.ephemeral_profiles_removed, 1);
    }

    #[test]
    fn stable_primary_terminates_orphan_but_keeps_profile_data() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("primary");
        let profile = root.join("generation-7");
        let browser = identity(262, "chrome");
        write_test_marker(&profile, identity(151, "nomifun"), browser.clone());
        std::fs::write(profile.join("Cookies"), b"persistent").unwrap();
        let mut control = fake_control(
            identity(999, "current"),
            HashMap::from([(browser.pid, ProcessLookup::Found(browser))]),
        );

        let report = recover_owned_profiles_with(
            &root,
            ProfileRecoveryMode::PreserveStableProfile,
            &mut control,
        );

        assert!(profile.join("Cookies").exists());
        assert!(!ownership_marker_path(&profile).exists());
        assert_eq!(report.process_trees_terminated, 1);
        assert_eq!(report.stable_markers_cleared, 1);
        assert_eq!(report.ephemeral_profiles_removed, 0);
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
    fn compatibility_gc_ignores_mtime_and_preserves_unmarked_directories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("profiles");
        let unmarked = root.join("old-but-unowned");
        std::fs::create_dir_all(&unmarked).unwrap();

        gc_stale_profiles(&root, Duration::ZERO);

        assert!(unmarked.exists(), "mtime must never authorize deletion");
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
            app_instance_id: nomifun_common::generate_id(),
            owner_app: identity(171, "nomifun"),
            browser: identity(282, "chrome"),
            profile_id: "profile-commit".into(),
        };

        commit_ownership_marker(&profile, &marker).expect("first marker commit");
        assert_eq!(read_marker(&profile).unwrap(), marker);
        assert!(
            commit_ownership_marker(&profile, &marker).is_err(),
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

        write_browser_ownership_marker(&claim, &profile, &command_shell, &child)
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
}
