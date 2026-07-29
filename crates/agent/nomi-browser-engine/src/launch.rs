//! 托管启动 Chromium：经 [`nomi_process_runtime::ChildProcessBuilder`] spawn 解析到的 chrome，传随机
//! 调试端口（`--remote-debugging-port=0`，OS 分配）+ **专属 user-data-dir**（红线：永不
//! 碰用户 profile）+ [`crate::switches::chromium_switches`] 全量硬化开关，然后**轮询
//! `<user-data-dir>/DevToolsActivePort`** 拿到实际端口与 browser ws 路径，拼出
//! `ws://127.0.0.1:<port><path>` 交给 [`crate::transport::Connection`] connect。
//!
//! 为何读 DevToolsActivePort 而非 HTTP `/json/version`：免一次 HTTP（无需 `trust_env(false)`
//! 绕代理）、无需解析 JSON、且是 chrome 端口就绪的**权威信号**（文件出现即端口在监听）。
//!
//! 进程托管：`Builder::spawn_with_cleanup` 同时返回 direct-child handle 与三平台整树
//! cleanup proof（Windows Job / Unix watchdog）。生命周期 owner 必须同时持有二者，并且只有
//! direct child 已回收且 cleanup proof 完成后，才能报告 Chromium 已停止。
//!
//! headless 决策：[`crate::display::display_available`] 为 false（无显示器：无头 server /
//! CI / SSH 无 X）→ 强制 `--headless=new`。日常 Agent 工作同样显式使用现代 headless；
//! 只有受信任的“前台打开”入口才会创建带真实窗口的替代 Host。

use std::path::{Path, PathBuf};
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

use crate::engine::BrowserError;

/// Chromium Host 的进程级展示模式。
///
/// 这是不可在存活进程上切换的启动属性。普通 Agent 工作必须使用
/// [`Self::Headless`]；只有 Hub 的受信任前台入口可以在先完整关闭旧 Host 后，
/// 用同一应用托管 profile 创建 [`Self::Headful`] 替代 Host。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserHostLaunchMode {
    Headless,
    Headful,
}

impl BrowserHostLaunchMode {
    pub const fn from_headful(headful: bool) -> Self {
        if headful { Self::Headful } else { Self::Headless }
    }

    pub const fn is_headful(self) -> bool {
        matches!(self, Self::Headful)
    }
}

/// 轮询 DevToolsActivePort 文件的最长等待（chrome 冷启 + 端口监听就绪）。仅 Windows ws 路径用。
#[cfg(windows)]
const PORT_FILE_TIMEOUT: Duration = Duration::from_secs(30);
/// 轮询间隔。仅 Windows ws 路径用。
#[cfg(windows)]
const PORT_FILE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 托管启动配置。`resolve_chrome_path`（Task 6）得到的可执行 + 专属数据目录 + headful。
#[derive(Clone)]
pub struct LaunchConfig {
    /// chrome 可执行绝对路径（来自 [`crate::acquire::resolve_chrome_path`]）。
    pub chrome_path: PathBuf,
    /// **专属** user-data-dir（红线：绝不指向用户真实 profile）。launch 会确保其存在。
    pub user_data_dir: PathBuf,
    /// 是否带可见窗口。注意：`display_available()==false` 时本标志被忽略，强制 headless。
    pub headful: bool,
}

impl std::fmt::Debug for LaunchConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchConfig")
            .field("chrome_path_configured", &true)
            .field("user_data_dir_configured", &true)
            .field("headful", &self.headful)
            .finish()
    }
}

/// 一次成功启动的产物：托管的 child handle + CDP 连接运输 + 精确 profile 所有权。
///
/// 字段故意不公开：拆走 child/transport 而丢失 ownership token 会让进程退出后留下
/// marker/DevToolsActivePort。所有消费路径都必须经过保留同一清理权的 API。
pub struct Launched {
    transport: Option<LaunchTransport>,
    cleanup: CommittedLaunchGuard,
}

impl Launched {
    fn new(transport: LaunchTransport, cleanup: CommittedLaunchGuard) -> Self {
        Self {
            transport: Some(transport),
            cleanup,
        }
    }

    pub(crate) fn into_managed(
        mut self,
    ) -> (
        nomi_process_runtime::ManagedChildProcess,
        LaunchTransport,
        crate::profile::BrowserOwnershipToken,
        Option<PathBuf>,
    ) {
        let transport = self
            .transport
            .take()
            .expect("launched browser still owns its transport");
        let (process, ownership_token, cleanup_user_data_dir) =
            self.cleanup.take_managed();
        (
            process,
            transport,
            ownership_token,
            cleanup_user_data_dir,
        )
    }

    /// Connect a low-level test/diagnostic caller while retaining exact process
    /// and profile cleanup authority. Production runtimes normally consume the
    /// launch through `CdpBackend`/`CdpHostRuntime` instead.
    pub async fn connect(
        self,
    ) -> Result<
        (LaunchedProcessGuard, crate::transport::Connection),
        crate::transport::TransportError,
    > {
        let (process, transport, ownership_token, cleanup_user_data_dir) =
            self.into_managed();
        let process = LaunchedProcessGuard {
            cleanup: CommittedLaunchGuard::new(
                process,
                ownership_token,
                cleanup_user_data_dir,
            ),
        };
        let connection = crate::transport::Connection::connect_launched(transport).await?;
        Ok((process, connection))
    }
}

/// Exact cleanup owner returned by [`Launched::connect`].
///
/// Dropping it proves whole-tree exit before removing this launch's exact
/// marker/port artifacts. It intentionally exposes only the direct child for
/// diagnostics; callers cannot split away the cleanup token.
pub struct LaunchedProcessGuard {
    cleanup: CommittedLaunchGuard,
}

impl LaunchedProcessGuard {
    pub fn child_mut(&mut self) -> &mut tokio::process::Child {
        self.cleanup.process_mut().child_mut()
    }
}

/// Force-stop a launched browser through its single authoritative lifecycle
/// operation. A failed or cancelled attempt leaves the same exact authority
/// available for a later retry.
pub(crate) async fn terminate_launched_process_tree(
    process: &mut nomi_process_runtime::ManagedChildProcess,
) -> Result<(), BrowserError> {
    process.shutdown().await.map_err(|error| {
        tracing::warn!(
            target: "nomi_browser_engine::launch",
            error_kind = ?error.kind(),
            "managed Chromium process-tree cleanup could not be proven"
        );
        BrowserError::Other("managed Chromium process-tree cleanup could not be proven".into())
    })
}

/// Terminate the exact managed Chromium tree and then clear only the ownership
/// artifacts committed for that same launch.
///
/// This is the authoritative cleanup path for failures after marker commit but
/// before a [`Launched`] value is handed to a runtime. Marker cleanup is never
/// attempted unless process-tree exit has first been proven.
pub(crate) async fn terminate_launched_process_tree_and_cleanup_profile(
    process: &mut nomi_process_runtime::ManagedChildProcess,
    ownership_token: &crate::profile::BrowserOwnershipToken,
    cleanup_user_data_dir: Option<&Path>,
) -> Result<(), BrowserError> {
    terminate_launched_process_tree(process).await?;
    match cleanup_user_data_dir {
        Some(profile_dir) => crate::profile::cleanup_ephemeral_profile_after_exact_shutdown(
            ownership_token,
            profile_dir,
        ),
        None => crate::profile::cleanup_browser_ownership_after_exact_shutdown(ownership_token),
    }
    .map_err(|_| ownership_artifact_cleanup_error())
}

async fn terminate_committed_launch_under_claim(
    process: &mut nomi_process_runtime::ManagedChildProcess,
    ownership_token: &crate::profile::BrowserOwnershipToken,
    ownership_claim: &crate::profile::ProfileLaunchClaim,
    cleanup_user_data_dir: Option<&Path>,
) -> Result<(), BrowserError> {
    terminate_launched_process_tree(process).await?;
    match cleanup_user_data_dir {
        Some(profile_dir) => {
            crate::profile::cleanup_ephemeral_profile_after_exact_shutdown_under_launch_claim(
                ownership_token,
                profile_dir,
                ownership_claim,
            )
        }
        None => {
            crate::profile::cleanup_browser_ownership_after_exact_shutdown_under_launch_claim(
                ownership_token,
                ownership_claim,
            )
        }
    }
    .map_err(|_| ownership_artifact_cleanup_error())
}

fn ownership_artifact_cleanup_error() -> BrowserError {
    tracing::warn!(
        target: "nomi_browser_engine::profile",
        reason = "ownership_artifact_cleanup_unverified",
        "managed Chromium exited but ownership artifact cleanup could not be proven"
    );
    BrowserError::Other(
        "managed Chromium profile ownership cleanup could not be proven".into(),
    )
}

fn launch_error_after_cleanup(
    primary: BrowserError,
    cleanup: Result<(), BrowserError>,
) -> BrowserError {
    if cleanup.is_ok() {
        primary
    } else {
        BrowserError::Other(
            "browser launch failed and process-tree cleanup could not be proven".into(),
        )
    }
}

/// Owns a spawned browser before its durable ownership marker is committed.
///
/// Cancellation in process-identity discovery must keep the process-tree proof
/// and exact ephemeral profile authority together. The Drop path therefore
/// hands both to one worker which proves tree exit before deleting the
/// uncommitted profile. Stable profiles carry no whole-directory token and are
/// never removed.
struct UncommittedLaunchGuard<'claim> {
    process: Option<nomi_process_runtime::ManagedChildProcess>,
    ephemeral_cleanup: Option<crate::profile::EphemeralProfileCleanupToken>,
    ownership_claim: &'claim crate::profile::ProfileLaunchClaim,
}

impl<'claim> UncommittedLaunchGuard<'claim> {
    fn new(
        ownership_claim: &'claim crate::profile::ProfileLaunchClaim,
        ephemeral_cleanup: Option<crate::profile::EphemeralProfileCleanupToken>,
    ) -> Self {
        Self {
            process: None,
            ephemeral_cleanup,
            ownership_claim,
        }
    }

    fn attach_process(&mut self, process: nomi_process_runtime::ManagedChildProcess) {
        debug_assert!(self.process.is_none());
        self.process = Some(process);
    }

    fn process(&self) -> &nomi_process_runtime::ManagedChildProcess {
        self.process
            .as_ref()
            .expect("uncommitted launch owns its spawned process")
    }

    fn into_committed(
        mut self,
        ownership_token: crate::profile::BrowserOwnershipToken,
    ) -> CommittedLaunchGuard {
        let process = self
            .process
            .take()
            .expect("ownership cannot commit without a spawned process");
        let cleanup_user_data_dir = self
            .ephemeral_cleanup
            .take()
            .map(crate::profile::EphemeralProfileCleanupToken::into_profile_dir);
        CommittedLaunchGuard::new(process, ownership_token, cleanup_user_data_dir)
    }

    async fn cleanup_under_claim(mut self) -> Result<(), BrowserError> {
        if let Some(process) = self.process.as_mut() {
            terminate_launched_process_tree(process).await?;
        }
        if let Some(ephemeral_cleanup) = self.ephemeral_cleanup.as_ref() {
            crate::profile::cleanup_uncommitted_ephemeral_profile_after_exact_shutdown_under_launch_claim(
                ephemeral_cleanup,
                self.ownership_claim,
            )
            .map_err(|_| ownership_artifact_cleanup_error())?;
        }
        self.process.take();
        self.ephemeral_cleanup.take();
        Ok(())
    }
}

impl Drop for UncommittedLaunchGuard<'_> {
    fn drop(&mut self) {
        match (self.process.take(), self.ephemeral_cleanup.take()) {
            (Some(process), Some(ephemeral_cleanup)) => {
                hand_off_uncommitted_browser_cleanup(process, ephemeral_cleanup);
            }
            (Some(process), None) => {
                // ManagedChildProcess owns the stable-profile process proof and
                // delegates it to its durable cleanup relay on drop.
                drop(process);
            }
            (None, Some(ephemeral_cleanup)) => {
                if crate::profile::cleanup_uncommitted_ephemeral_profile_after_exact_shutdown_under_launch_claim(
                    &ephemeral_cleanup,
                    self.ownership_claim,
                )
                .is_err()
                {
                    tracing::warn!(
                        target: "nomi_browser_engine::launch",
                        "unspawned ephemeral browser profile cleanup could not be proven"
                    );
                }
            }
            (None, None) => {}
        }
    }
}

/// Owns a committed launch from the instant its exact marker exists.
///
/// Any cancellation or early return drops this guard and hands both the
/// process-tree proof and exact marker token to one retrying cleanup worker.
/// Successful runtime construction explicitly takes both pieces together.
struct CommittedLaunchGuard {
    process: Option<nomi_process_runtime::ManagedChildProcess>,
    ownership_token: Option<crate::profile::BrowserOwnershipToken>,
    cleanup_user_data_dir: Option<PathBuf>,
}

impl CommittedLaunchGuard {
    fn new(
        process: nomi_process_runtime::ManagedChildProcess,
        ownership_token: crate::profile::BrowserOwnershipToken,
        cleanup_user_data_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            process: Some(process),
            ownership_token: Some(ownership_token),
            cleanup_user_data_dir,
        }
    }

    fn process_mut(&mut self) -> &mut nomi_process_runtime::ManagedChildProcess {
        self.process
            .as_mut()
            .expect("committed launch still owns its process")
    }

    fn take_managed(
        &mut self,
    ) -> (
        nomi_process_runtime::ManagedChildProcess,
        crate::profile::BrowserOwnershipToken,
        Option<PathBuf>,
    ) {
        (
            self.process
                .take()
                .expect("committed launch still owns its process"),
            self.ownership_token
                .take()
                .expect("committed launch still owns its ownership token"),
            self.cleanup_user_data_dir.take(),
        )
    }

    async fn cleanup_under_claim(
        mut self,
        ownership_claim: &crate::profile::ProfileLaunchClaim,
    ) -> Result<(), BrowserError> {
        let ownership_token = self
            .ownership_token
            .as_ref()
            .expect("committed launch still owns its ownership token")
            .clone();
        let cleanup_user_data_dir = self.cleanup_user_data_dir.clone();
        let result = terminate_committed_launch_under_claim(
            self.process_mut(),
            &ownership_token,
            ownership_claim,
            cleanup_user_data_dir.as_deref(),
        )
        .await;
        if result.is_ok() {
            self.process.take();
            self.ownership_token.take();
        }
        result
    }
}

impl Drop for CommittedLaunchGuard {
    fn drop(&mut self) {
        let (Some(process), Some(ownership_token)) =
            (self.process.take(), self.ownership_token.take())
        else {
            return;
        };
        spawn_committed_launch_cleanup(
            process,
            ownership_token,
            self.cleanup_user_data_dir.take(),
        );
    }
}

fn spawn_committed_launch_cleanup(
    process: nomi_process_runtime::ManagedChildProcess,
    ownership_token: crate::profile::BrowserOwnershipToken,
    cleanup_user_data_dir: Option<PathBuf>,
) {
    hand_off_dropped_browser_cleanup(
        std::sync::Arc::new(tokio::sync::Mutex::new(process)),
        ownership_token,
        cleanup_user_data_dir,
    );
}

const DROPPED_BROWSER_CLEANUP_MAX_ATTEMPTS: usize = 20;

#[cfg(test)]
std::thread_local! {
    /// A one-shot failure injection scoped to the current libtest thread.
    ///
    /// The previous process-global AtomicBool could be consumed by an
    /// unrelated parallel test which happened to drop a browser first.
    static FORCE_BROWSER_CLEANUP_THREAD_FAILURE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
struct BrowserCleanupThreadFailureInjection {
    _private: (),
}

#[cfg(test)]
impl BrowserCleanupThreadFailureInjection {
    fn arm() -> Self {
        FORCE_BROWSER_CLEANUP_THREAD_FAILURE.with(|forced| {
            assert!(
                !forced.replace(true),
                "cleanup-thread failure injection is already armed on this test thread"
            );
        });
        Self { _private: () }
    }
}

#[cfg(test)]
impl Drop for BrowserCleanupThreadFailureInjection {
    fn drop(&mut self) {
        FORCE_BROWSER_CLEANUP_THREAD_FAILURE.with(|forced| forced.set(false));
    }
}

#[cfg(test)]
fn take_browser_cleanup_thread_failure_injection() -> bool {
    FORCE_BROWSER_CLEANUP_THREAD_FAILURE.with(|forced| forced.replace(false))
}

struct PendingDroppedBrowserCleanup {
    process: std::sync::Arc<tokio::sync::Mutex<nomi_process_runtime::ManagedChildProcess>>,
    authority: DroppedBrowserCleanupAuthority,
    completion: DroppedBrowserCleanupTicket,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DroppedBrowserCleanupCompletion {
    Complete,
    RetryPending,
}

#[derive(Clone)]
pub(crate) struct DroppedBrowserCleanupTicket {
    inner: std::sync::Arc<DroppedBrowserCleanupTicketInner>,
}

struct DroppedBrowserCleanupTicketInner {
    state: std::sync::atomic::AtomicU8,
    changed: tokio::sync::watch::Sender<u8>,
    recovery: std::sync::Mutex<Option<ReclaimableDroppedBrowserCleanup>>,
}

struct ReclaimableDroppedBrowserCleanup {
    process: std::sync::Arc<
        tokio::sync::Mutex<nomi_process_runtime::ManagedChildProcess>,
    >,
    authority: DroppedBrowserCleanupAuthority,
}

impl DroppedBrowserCleanupTicket {
    fn pending() -> Self {
        let (changed, _) = tokio::sync::watch::channel(0);
        Self {
            inner: std::sync::Arc::new(DroppedBrowserCleanupTicketInner {
                state: std::sync::atomic::AtomicU8::new(0),
                changed,
                recovery: std::sync::Mutex::new(None),
            }),
        }
    }

    fn publish_complete(&self) {
        if self
            .inner
            .state
            .swap(1, std::sync::atomic::Ordering::AcqRel)
            != 1
        {
            self.inner
                .recovery
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            self.inner.changed.send_replace(1);
        }
    }

    fn publish_recoverable(
        &self,
        process: std::sync::Arc<
            tokio::sync::Mutex<nomi_process_runtime::ManagedChildProcess>,
        >,
        authority: DroppedBrowserCleanupAuthority,
    ) {
        if self.inner.state.load(std::sync::atomic::Ordering::Acquire) == 1 {
            return;
        }
        let mut recovery = self
            .inner
            .recovery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.inner.state.load(std::sync::atomic::Ordering::Acquire) == 1 {
            return;
        }
        if recovery.is_none() {
            *recovery = Some(ReclaimableDroppedBrowserCleanup { process, authority });
        }
        drop(recovery);
        self.inner
            .state
            .store(2, std::sync::atomic::Ordering::Release);
        self.inner.changed.send_replace(2);
    }

    fn restore_recovery(&self, cleanup: ReclaimableDroppedBrowserCleanup) {
        *self
            .inner
            .recovery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cleanup);
        self.inner
            .state
            .store(2, std::sync::atomic::Ordering::Release);
        self.inner.changed.send_replace(2);
    }

    pub(crate) async fn wait_or_retry(&self) -> DroppedBrowserCleanupCompletion {
        let mut changed = self.inner.changed.subscribe();
        loop {
            match self.inner.state.load(std::sync::atomic::Ordering::Acquire) {
                1 => return DroppedBrowserCleanupCompletion::Complete,
                2 if self
                    .inner
                    .state
                    .compare_exchange(
                        2,
                        3,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Acquire,
                    )
                    .is_ok() =>
                {
                    let cleanup = self
                        .inner
                        .recovery
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    let Some(cleanup) = cleanup else {
                        self.inner
                            .state
                            .store(2, std::sync::atomic::Ordering::Release);
                        self.inner.changed.send_replace(2);
                        tokio::task::yield_now().await;
                        continue;
                    };
                    let mut lease = DroppedBrowserCleanupRecoveryLease {
                        ticket: self.clone(),
                        cleanup: Some(cleanup),
                    };
                    let complete = clean_reclaimable_dropped_browser(
                        lease
                            .cleanup
                            .as_ref()
                            .expect("recovery lease retains exact authority"),
                    )
                    .await;
                    if complete {
                        lease.cleanup.take();
                        self.publish_complete();
                        return DroppedBrowserCleanupCompletion::Complete;
                    }
                    let cleanup = lease
                        .cleanup
                        .take()
                        .expect("failed recovery returns exact authority");
                    self.restore_recovery(cleanup);
                    return DroppedBrowserCleanupCompletion::RetryPending;
                }
                _ => {
                    // watch retains the latest state even when completion
                    // lands between this load and the async registration.
                    let _ = changed.changed().await;
                }
            }
        }
    }
}

struct DroppedBrowserCleanupRecoveryLease {
    ticket: DroppedBrowserCleanupTicket,
    cleanup: Option<ReclaimableDroppedBrowserCleanup>,
}

impl Drop for DroppedBrowserCleanupRecoveryLease {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            self.ticket.restore_recovery(cleanup);
        }
    }
}

impl Drop for PendingDroppedBrowserCleanup {
    fn drop(&mut self) {
        // Panic, worker/runtime construction failure, and explicit defer all
        // return the indivisible job to the shared ticket. A caller can drive
        // one bounded retry round; final ticket Drop alone falls back to the
        // ManagedChildProcess relay and startup ownership audit.
        self.completion
            .publish_recoverable(self.process.clone(), self.authority.clone());
    }
}

#[derive(Clone)]
enum DroppedBrowserCleanupAuthority {
    Committed {
        ownership_token: crate::profile::BrowserOwnershipToken,
        cleanup_user_data_dir: Option<PathBuf>,
    },
    UncommittedEphemeral {
        cleanup_token: crate::profile::EphemeralProfileCleanupToken,
    },
}

fn defer_dropped_browser_cleanups(cleanups: Vec<PendingDroppedBrowserCleanup>) {
    let count = cleanups.len();
    tracing::error!(
        count,
        "browser cleanup worker unavailable; exact cleanup jobs returned to their durable tickets"
    );
    // PendingDroppedBrowserCleanup::drop moves a clone of each indivisible job
    // into its ticket. If no caller retained that ticket, dropping its inner
    // job falls through to ManagedChildProcess's relay and startup lineage.
    drop(cleanups);
}

/// Hand one dropped browser's indivisible process/token/profile authority to a
/// process-local relay that does not depend on the caller's Tokio runtime.
///
/// Thread-spawn and independent-runtime construction failures return the exact
/// job to the completion ticket, allowing an authoritative shutdown caller to
/// take over one bounded retry round. If nobody retained the ticket, final
/// Drop releases the process into `nomi-process-runtime` and preserves marker
/// lineage for startup recovery.
pub(crate) fn hand_off_dropped_browser_cleanup(
    process: std::sync::Arc<
        tokio::sync::Mutex<nomi_process_runtime::ManagedChildProcess>,
    >,
    ownership_token: crate::profile::BrowserOwnershipToken,
    cleanup_user_data_dir: Option<PathBuf>,
) -> DroppedBrowserCleanupTicket {
    let completion = DroppedBrowserCleanupTicket::pending();
    let batch = vec![PendingDroppedBrowserCleanup {
        process,
        authority: DroppedBrowserCleanupAuthority::Committed {
            ownership_token,
            cleanup_user_data_dir,
        },
        completion: completion.clone(),
    }];
    hand_off_pending_browser_cleanups(batch);
    completion
}

fn hand_off_uncommitted_browser_cleanup(
    process: nomi_process_runtime::ManagedChildProcess,
    cleanup_token: crate::profile::EphemeralProfileCleanupToken,
) {
    let completion = DroppedBrowserCleanupTicket::pending();
    hand_off_pending_browser_cleanups(vec![PendingDroppedBrowserCleanup {
        process: std::sync::Arc::new(tokio::sync::Mutex::new(process)),
        authority: DroppedBrowserCleanupAuthority::UncommittedEphemeral { cleanup_token },
        completion,
    }]);
}

fn hand_off_pending_browser_cleanups(batch: Vec<PendingDroppedBrowserCleanup>) {
    #[cfg(test)]
    if take_browser_cleanup_thread_failure_injection() {
        defer_dropped_browser_cleanups(batch);
        return;
    }

    let retained = std::sync::Arc::new(std::sync::Mutex::new(Some(batch)));
    let worker_retained = std::sync::Arc::clone(&retained);
    let worker = move || {
        let batch = worker_retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let Some(batch) = batch else {
            return;
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        match runtime {
            Ok(runtime) => runtime.block_on(clean_dropped_browsers(batch)),
            Err(_) => defer_dropped_browser_cleanups(batch),
        }
    };

    if std::thread::Builder::new()
        .name("nomi-browser-cleanup".into())
        .spawn(worker)
        .is_err()
    {
        let batch = retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(batch) = batch {
            defer_dropped_browser_cleanups(batch);
        }
    }
}

async fn clean_dropped_browsers(cleanups: Vec<PendingDroppedBrowserCleanup>) {
    for cleanup in cleanups {
        let mut process = cleanup.process.lock().await;
        let complete = match &cleanup.authority {
            DroppedBrowserCleanupAuthority::Committed {
                ownership_token,
                cleanup_user_data_dir,
            } => {
                retry_dropped_browser_cleanup(
                    &mut process,
                    ownership_token,
                    cleanup_user_data_dir.as_deref(),
                )
                .await
            }
            DroppedBrowserCleanupAuthority::UncommittedEphemeral { cleanup_token } => {
                retry_dropped_uncommitted_browser_cleanup(&mut process, cleanup_token).await
            }
        };
        if complete {
            cleanup.completion.publish_complete();
        }
        drop(process);
    }
}

async fn clean_reclaimable_dropped_browser(
    cleanup: &ReclaimableDroppedBrowserCleanup,
) -> bool {
    let mut process = cleanup.process.lock().await;
    match &cleanup.authority {
        DroppedBrowserCleanupAuthority::Committed {
            ownership_token,
            cleanup_user_data_dir,
        } => {
            retry_dropped_browser_cleanup(
                &mut process,
                ownership_token,
                cleanup_user_data_dir.as_deref(),
            )
            .await
        }
        DroppedBrowserCleanupAuthority::UncommittedEphemeral { cleanup_token } => {
            retry_dropped_uncommitted_browser_cleanup(&mut process, cleanup_token).await
        }
    }
}

/// Retry a dropped browser's exact cleanup on an OS-thread-owned runtime.
///
/// The retry is deliberately bounded. If process proof or artifact cleanup is
/// permanently fail-closed (for example, the marker was replaced), dropping
/// `process` delegates tree cleanup to `nomi-process-runtime` and leaves the
/// profile artifacts for its startup ownership audit instead of keeping an
/// immortal browser cleanup task.
pub(crate) async fn retry_dropped_browser_cleanup(
    process: &mut nomi_process_runtime::ManagedChildProcess,
    ownership_token: &crate::profile::BrowserOwnershipToken,
    cleanup_user_data_dir: Option<&Path>,
) -> bool {
    for attempt in 0..DROPPED_BROWSER_CLEANUP_MAX_ATTEMPTS {
        if terminate_launched_process_tree_and_cleanup_profile(
            process,
            ownership_token,
            cleanup_user_data_dir,
        )
        .await
        .is_ok()
        {
            return true;
        }
        if attempt + 1 < DROPPED_BROWSER_CLEANUP_MAX_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    tracing::warn!(
        target: "nomi_browser_engine::launch",
        attempts = DROPPED_BROWSER_CLEANUP_MAX_ATTEMPTS,
        "dropped browser exact cleanup deferred to managed-process relay and startup profile audit"
    );
    false
}

async fn retry_dropped_uncommitted_browser_cleanup(
    process: &mut nomi_process_runtime::ManagedChildProcess,
    cleanup_token: &crate::profile::EphemeralProfileCleanupToken,
) -> bool {
    for attempt in 0..DROPPED_BROWSER_CLEANUP_MAX_ATTEMPTS {
        let cleanup = async {
            terminate_launched_process_tree(process).await?;
            crate::profile::cleanup_uncommitted_ephemeral_profile_after_exact_shutdown(
                cleanup_token,
            )
            .map_err(|_| ownership_artifact_cleanup_error())
        }
        .await;
        if cleanup.is_ok() {
            return true;
        }
        if attempt + 1 < DROPPED_BROWSER_CLEANUP_MAX_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    tracing::warn!(
        target: "nomi_browser_engine::launch",
        attempts = DROPPED_BROWSER_CLEANUP_MAX_ATTEMPTS,
        "uncommitted ephemeral browser cleanup could not be proven"
    );
    false
}

/// CDP 连接运输。**Unix 生产用 `--remote-debugging-pipe`**（fd3/fd4；浏览器在本进程死亡——含
/// SIGKILL——时,内核关闭继承的 fd → Chromium 自行退出,跨平台父死自清的最优解,见
/// docs/superpowers/specs/browser-use/2026-06-19-macos-pdeath-pipe-transport-design.md）。
/// **Windows 生产用 ws url**（port + DevToolsActivePort + Job Object 清理；pipe 在 Windows 走继承
/// HANDLE,复杂且 Job Object 已内核级清理,故不转）。
pub enum LaunchTransport {
    /// Unix `--remote-debugging-pipe`：`cmd_writer`=我们写命令的管道写端（chrome 在 fd3 读）,
    /// `resp_reader`=我们读响应的管道读端（chrome 在 fd4 写）。交给 [`crate::transport::Connection::connect_pipe`]。
    #[cfg(unix)]
    Pipe {
        cmd_writer: std::os::fd::OwnedFd,
        resp_reader: std::os::fd::OwnedFd,
    },
    /// `ws://127.0.0.1:<port>/devtools/browser/<uuid>`，交给 [`crate::transport::Connection::connect`]。
    Ws { ws_url: String },
}

/// 构造 chrome 启动参数（纯函数，便于单测）。
///
/// - CDP 运输开关：Unix=`--remote-debugging-pipe`（fd3/fd4,浏览器父死自退）；Windows=
///   `--remote-debugging-port=0`（OS 分配 + DevToolsActivePort）。
/// - `--user-data-dir=<dir>`：专属数据目录（红线：非用户 profile）。
/// - [`crate::switches::chromium_switches`] 全量静态硬化开关。
/// - `--no-first-run` / `--no-default-browser-check`：免首启向导/默认浏览器询问。
/// - `--headless=new`：仅当 `force_headless`（无显示器或显式 headless）。
/// - headful（`!force_headless`）：`--window-position` + `--window-size`，创建真实可见窗口。
/// - `--no-startup-window`：不自动开启动窗口（消除冗余 about:blank；受控页由 backend
///   `Target.createTarget` 单独建）。靠 `--remote-debugging-port` 触发的 REMOTE_DEBUGGING
///   keep-alive 保进程存活、不无窗口自退。
///
/// `force_headless` 由调用方按 `display_available()` 与 `LaunchConfig::headful` 算好后传入，
/// 使本函数保持纯逻辑、无平台/环境探测，单测可在任意宿主断言。
pub fn build_chrome_args(user_data_dir: &Path, force_headless: bool) -> Vec<String> {
    build_chrome_args_for_mode(
        user_data_dir,
        if force_headless {
            BrowserHostLaunchMode::Headless
        } else {
            BrowserHostLaunchMode::Headful
        },
    )
}

/// 按显式 Host 展示模式构造 Chromium 参数。
///
/// 不允许用 `--start-minimized` 模拟静默执行：Headless 必须包含
/// `--headless=new`，Headful 必须创建正常窗口。这样系统托盘/任务栏中不会残留一个
/// 用户未请求的隐藏窗口，且“前台打开”的可见性完全由 Hub 的显式替换流程控制。
pub fn build_chrome_args_for_mode(
    user_data_dir: &Path,
    mode: BrowserHostLaunchMode,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // CDP 运输开关：Unix 用 `--remote-debugging-pipe`（fd3/fd4；浏览器在父死/管道 EOF 时自退,
    // 免疫 SIGKILL,见设计文档）;Windows 用 `--remote-debugging-port=0`（OS 分配 + DevToolsActivePort）。
    #[cfg(unix)]
    args.push("--remote-debugging-pipe".into());
    #[cfg(windows)]
    args.push("--remote-debugging-port=0".into());

    args.push(format!("--user-data-dir={}", user_data_dir.display()));

    // 静态硬化基线（零后台出站 / 容器防崩 / 截图可复现；Linux 含 dev-shm）。
    args.extend(crate::switches::chromium_switches());

    args.push("--no-first-run".into());
    args.push("--no-default-browser-check".into());

    if mode == BrowserHostLaunchMode::Headless {
        // 无显示器强制无头；`=new` 是现代 headless（非旧 --headless），CDP 截图可用。
        args.push("--headless=new".into());
    } else {
        // Headful Host 仅由受信任的显式前台入口创建，因此直接使用正常窗口，
        // 不再通过最小化窗口伪装后台执行。
        args.push("--window-position=80,80".into());
        args.push("--window-size=1280,800".into());
    }

    // Linux 容器内 sandbox 常因缺 user-namespace 而启动失败；回退 --no-sandbox。
    // TODO(verify-linux): 容器 sandbox 探测/回退需实机核对（当前为无条件回退，偏保守），
    // 见 docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md。
    #[cfg(target_os = "linux")]
    args.push("--no-sandbox".into());

    // **不自动开启动窗口/标签**：消除冗余的命令行起始标签——受控页由 backend
    // `Target.createTarget("about:blank")` 单独建（[`crate::backend::cdp`]），命令行再开一个就是
    // 多余的孤儿空白标签。改用 `--no-startup-window` 让 chrome 启动时不开任何窗口/标签。
    //
    // 为何不会因「无窗口」自退、也不影响 launch 轮询：本函数恒传 `--remote-debugging-port`
    // （上面），命中 Chromium 的 keep-alive 受支持组合——`(kNoStartupWindow || kHeadless) &&
    // (kRemoteDebuggingPort || kRemoteDebuggingPipe)` → `ScopedKeepAlive(REMOTE_DEBUGGING)`
    // 拴住进程直到显式 `Browser.close`（见 chrome/browser/devtools/chrome_devtools_manager_
    // delegate.cc）；且 DevToolsActivePort 在 socket bind 成功即写、与有无 window 无关（见
    // content/browser/devtools/devtools_http_handler.cc）→ launch_chrome 的端口轮询不受影响。
    // 平台无关的 Chromium 通用开关（keep-alive 逻辑同源、仅排除 ChromeOS，本仓不支持）。
    // TODO(verify-macos/linux): mac/linux 真机各冒烟一次确认（本机仅 Windows 已验），见
    // docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md。
    args.push("--no-startup-window".into());

    args
}

/// 解析 DevToolsActivePort 文件内容 → `(port, ws_path)`。
///
/// chrome 在 `--remote-debugging-port=0` 下把实际监听信息写进
/// `<user-data-dir>/DevToolsActivePort`：
///   - 第 1 行：端口号（如 `54213`）；
///   - 第 2 行：browser ws 路径（如 `/devtools/browser/4f1c-...`）。
///
/// 返回 `Err(Other)` 给出明确诊断（行数不足 / 端口非数字）；不 panic。
pub fn parse_devtools_active_port(content: &str) -> Result<(u16, String), BrowserError> {
    let mut lines = content.lines();
    let port_line = lines
        .next()
        .ok_or_else(|| BrowserError::Other("DevToolsActivePort empty (no port line)".into()))?;
    let ws_path = lines
        .next()
        .ok_or_else(|| BrowserError::Other("DevToolsActivePort missing ws-path line".into()))?;

    // Never include either line in the error. The second line is a
    // browser-scoped WebSocket path and may contain a secret token; the first
    // line is caller-controlled file content and is not useful to a caller.
    let port: u16 = port_line
        .trim()
        .parse()
        .map_err(|_| BrowserError::Other("DevToolsActivePort contained an invalid port".into()))?;
    if port == 0 {
        return Err(BrowserError::Other(
            "DevToolsActivePort reported port 0 (not yet bound)".into(),
        ));
    }

    let ws_path = ws_path.trim().to_string();
    if !ws_path.starts_with('/') {
        return Err(BrowserError::Other(
            "DevToolsActivePort contained an invalid browser path".into(),
        ));
    }
    Ok((port, ws_path))
}

/// 由端口 + ws 路径拼出 browser ws url（loopback v4）。
pub fn build_ws_url(port: u16, ws_path: &str) -> String {
    format!("ws://127.0.0.1:{port}{ws_path}")
}

fn safe_profile_prepare_error() -> BrowserError {
    BrowserError::Other("browser launch could not prepare its profile".into())
}

fn safe_profile_ownership_error() -> BrowserError {
    BrowserError::Other("browser launch ownership preflight failed".into())
}

fn safe_chromium_spawn_error() -> BrowserError {
    BrowserError::Other("browser launch could not start Chromium".into())
}

fn safe_devtools_timeout_error() -> BrowserError {
    BrowserError::Other("browser launch timed out waiting for DevToolsActivePort".into())
}

/// Keep the Chromium test escape hatch on an exact allowlist.
///
/// Release builds do not read `NOMI_CHROME_EXTRA_ARGS` at all. In debug
/// builds, only the switches required by the OOPIF fixture are accepted.
/// An allowlist is intentional here: Chromium has many aliases and related
/// switches that can change profile ownership, CDP exposure, extensions,
/// sandboxing, or other security-sensitive behavior.
fn filtered_extra_chrome_args(extra: &str) -> Vec<String> {
    #[cfg(debug_assertions)]
    {
        const OOPIF_HOST_RESOLVER_RULES: &str =
            "--host-resolver-rules=MAP *.nomitest 127.0.0.1";

        extra
            .lines()
            .map(str::trim)
            .filter(|arg| !arg.is_empty())
            .filter_map(|arg| {
                if arg == OOPIF_HOST_RESOLVER_RULES || arg == "--site-per-process" {
                    Some(arg.to_owned())
                } else {
                    // Never echo a rejected value: it may contain a path,
                    // endpoint, or another secret supplied by the caller.
                    tracing::warn!(
                        target: "nomi_browser_engine::launch",
                        "ignored unsupported NOMI_CHROME_EXTRA_ARGS entry"
                    );
                    None
                }
            })
            .collect()
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = extra;
        Vec::new()
    }
}

/// 托管启动 chrome 并返回 child + CDP 连接运输。
///
/// 流程：确保 user-data-dir 存在 → scrub 脏 profile → 清 stale Singleton → 起 chrome。
/// **Unix**：`--remote-debugging-pipe`,经 fd3/fd4 即时连（无端口轮询；浏览器在父死/管道 EOF 时
/// 自退）;**Windows**：`--remote-debugging-port=0` + 轮询 DevToolsActivePort 拿端口/ws 路径。
/// `force_headless` 由调用方按 display 算好。
pub async fn launch_chrome(
    config: &LaunchConfig,
    force_headless: bool,
) -> Result<Launched, BrowserError> {
    launch_chrome_with_cleanup_profile(config, force_headless, None).await
}

pub(crate) async fn launch_chrome_with_cleanup_profile(
    config: &LaunchConfig,
    force_headless: bool,
    cleanup_user_data_dir: Option<PathBuf>,
) -> Result<Launched, BrowserError> {
    // user-data-dir 必须存在（专属目录；红线已在 config 构造处保证非用户 profile）。
    std::fs::create_dir_all(&config.user_data_dir).map_err(|_| safe_profile_prepare_error())?;

    // Ownership must be resolved before touching Preferences, Singleton files,
    // or any other profile state. A live owner or in-progress recovery makes
    // the whole launch fail closed.
    let ownership_claim =
        crate::profile::prepare_ownership_marker_for_launch(&config.user_data_dir).map_err(
            |_| safe_profile_ownership_error(),
        )?;
    let first_ephemeral_cleanup = cleanup_user_data_dir
        .as_deref()
        .map(|profile_dir| {
            crate::profile::claim_ephemeral_profile_cleanup(profile_dir, &ownership_claim)
        })
        .transpose()
        .map_err(|_| safe_profile_ownership_error())?;
    // From this point forward the uncommitted guard owns exact ephemeral
    // cleanup authority. Before spawn it can remove the profile synchronously;
    // after spawn it keeps that authority indivisible from the process proof.
    let first_attempt =
        UncommittedLaunchGuard::new(&ownership_claim, first_ephemeral_cleanup);

    // **脏 profile 根治（keystone）**：上次 chrome 必被硬杀（kill_on_drop / Job Object / app 同步
    // exit），profile.exit_type 停在 "Crashed" → 下次启动弹「未正确关闭 / 恢复页面?」气泡 + 跑会话
    // 恢复（异常启动路径更易崩）。spawn 前（chrome 此刻必未运行）best-effort 洗回 "Normal"，是覆盖
    // 所有退出路径（含 crash/断电）的唯一可靠层。见 crate::profile 模块文档。
    if let Err(e) = crate::profile::scrub_crash_markers(&config.user_data_dir) {
        tracing::warn!(
            target: "nomi_browser_engine::launch",
            error_kind = ?e.kind(),
            "profile crash-marker scrub failed (best-effort; launch continues)"
        );
    }
    // mac/linux：顺手清 stale Singleton* 三件套（Windows 因 FILE_FLAG_DELETE_ON_CLOSE 无需）。
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    crate::profile::clear_stale_singleton(&config.user_data_dir);
    crate::profile::prepare_runtime_port_for_launch(
        &config.user_data_dir,
        &ownership_claim,
    )
    .map_err(|_| safe_profile_prepare_error())?;

    let mut args = build_chrome_args(&config.user_data_dir, force_headless);

    // The environment escape hatch is compiled out of release builds.
    // Debug builds still use the exact allowlist above; arbitrary Chromium
    // switches must never be able to replace the managed profile or CDP
    // transport, load extensions, or weaken sandbox/security settings.
    #[cfg(debug_assertions)]
    if let Ok(extra) = std::env::var("NOMI_CHROME_EXTRA_ARGS") {
        args.extend(filtered_extra_chrome_args(&extra));
    }

    #[cfg(unix)]
    {
        launch_chrome_pipe(config, &args, first_attempt).await
    }
    #[cfg(windows)]
    {
        match launch_chrome_ws(config, &args, first_attempt).await {
            Ok(v) => Ok(v),
            Err(first) if should_retry_with_startup_page(&first, &args) => {
                tracing::warn!(
                    target: "nomi_browser_engine::launch",
                    error = %first,
                    "chrome exited before DevTools port was ready; retrying with an explicit startup page"
                );
                let retry_ephemeral_cleanup = match cleanup_user_data_dir.as_deref() {
                    Some(profile_dir) => Some(
                        crate::profile::restore_ephemeral_profile_for_retry(
                            profile_dir,
                            &ownership_claim,
                        )
                        .map_err(|_| {
                            BrowserError::Other(
                                "browser launch ownership preflight failed before retry".into(),
                            )
                        })?,
                    ),
                    None => {
                        crate::profile::prepare_ownership_marker_for_retry(
                            &config.user_data_dir,
                            &ownership_claim,
                        )
                        .map_err(|_| {
                            BrowserError::Other(
                                "browser launch ownership preflight failed before retry".into(),
                            )
                        })?;
                        None
                    }
                };
                let retry_attempt =
                    UncommittedLaunchGuard::new(&ownership_claim, retry_ephemeral_cleanup);
                crate::profile::prepare_runtime_port_for_launch(
                    &config.user_data_dir,
                    &ownership_claim,
                )
                .map_err(|_| safe_profile_prepare_error())?;
                let fallback_args = chrome_args_with_startup_page(&args);
                launch_chrome_ws(config, &fallback_args, retry_attempt)
                .await
                .map_err(|_| {
                        BrowserError::Other(
                            "browser launch retry with startup page failed".into(),
                        )
                    })
            }
            Err(e) => Err(e),
        }
    }
}

/// **Unix**：`--remote-debugging-pipe` 启动。建两条匿名管道,经 [`nomi_process_runtime::ChildProcessBuilder::inherit_fds`]
/// 把 chrome 端装到 fd3（读命令）/fd4（写响应）；我们持另两端交 [`crate::transport::Connection::connect_pipe`]。
/// 无端口轮询——管道即时可用,且浏览器在父死/管道 EOF 时自退（免疫 SIGKILL）。
#[cfg(unix)]
async fn launch_chrome_pipe(
    config: &LaunchConfig,
    args: &[String],
    mut uncommitted: UncommittedLaunchGuard<'_>,
) -> Result<Launched, BrowserError> {
    // pipe_in：父写命令 → chrome 读（fd3）。pipe_out：chrome 写响应（fd4）→ 父读。
    let (chrome_cmd_read, our_cmd_write) = make_pipe()?;
    let (our_resp_read, chrome_resp_write) = make_pipe()?;

    let mut builder = nomi_process_runtime::ChildProcessBuilder::new(&config.chrome_path);
    builder
        .args(args)
        // chrome 的 stdout/stderr 我们不消费；null 掉避免污染父进程控制台。
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // chrome `--remote-debugging-pipe`：fd3 读命令、fd4 写响应。
        .inherit_fds(vec![(3, chrome_cmd_read), (4, chrome_resp_write)]);

    let process = builder.spawn_managed().map_err(|e| {
        tracing::debug!(
            target: "nomi_browser_engine::launch",
            error_kind = ?e.kind(),
            "managed Chromium spawn failed"
        );
        safe_chromium_spawn_error()
    })?;
    uncommitted.attach_process(process);
    let ownership_claim = uncommitted.ownership_claim;
    let ownership_token = match commit_browser_ownership(
        config,
        ownership_claim,
        uncommitted.process(),
        uncommitted.ephemeral_cleanup.as_ref(),
    )
    .await
    {
            Ok(token) => token,
            Err(primary) => {
                let cleanup = uncommitted.cleanup_under_claim().await;
                return Err(launch_error_after_cleanup(primary, cleanup));
            }
    };
    let mut committed = uncommitted.into_committed(ownership_token);

    // 快速失败：给 chrome 一小会儿；若立即退出（坏开关 / 缺依赖）立即报错,不必等首条 CDP 命令超时。
    tokio::time::sleep(Duration::from_millis(120)).await;
    if let Ok(Some(status)) = committed.process_mut().child_mut().try_wait() {
        let primary = BrowserError::Other(format!(
            "chrome exited immediately after spawn (bad flags / missing deps?) status {status}"
        ));
        let cleanup = committed.cleanup_under_claim(ownership_claim).await;
        return Err(launch_error_after_cleanup(primary, cleanup));
    }

    Ok(Launched::new(
        LaunchTransport::Pipe {
            cmd_writer: our_cmd_write,
            resp_reader: our_resp_read,
        },
        committed,
    ))
}

/// (unix) 建一条匿名管道 → `(读端, 写端)`,两端都设 `FD_CLOEXEC`。chrome 端经 Builder 的 dup2
/// shuffle 在 fd3/4 上清掉 CLOEXEC 以 survive exec；我们这端保持 CLOEXEC,绝不漏进 chrome 或其它 spawn。
#[cfg(unix)]
fn make_pipe() -> Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd), BrowserError> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe(2) 成功时向数组写入恰好两个新建 owned fd。
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        return Err(BrowserError::Other(format!(
            "pipe(2): {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: pipe(2) 刚返回两个独占 fd,所有权移交 OwnedFd（drop 即 close）。
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    set_cloexec(&read)?;
    set_cloexec(&write)?;
    Ok((read, write))
}

#[cfg(unix)]
fn set_cloexec(fd: &std::os::fd::OwnedFd) -> Result<(), BrowserError> {
    use std::os::fd::AsRawFd;
    let raw = fd.as_raw_fd();
    // SAFETY: F_GETFD/F_SETFD 在一个 owned fd 上,无前置条件。
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFD) };
    if flags < 0 {
        return Err(BrowserError::Other(format!(
            "fcntl F_GETFD: {}",
            std::io::Error::last_os_error()
        )));
    }
    if unsafe { libc::fcntl(raw, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(BrowserError::Other(format!(
            "fcntl F_SETFD: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// **Windows**：`--remote-debugging-port=0` 启动,轮询 DevToolsActivePort 拿端口 + ws 路径,拼 ws url。
#[cfg(windows)]
async fn launch_chrome_ws(
    config: &LaunchConfig,
    args: &[String],
    mut uncommitted: UncommittedLaunchGuard<'_>,
) -> Result<Launched, BrowserError> {
    // `prepare_runtime_port_for_launch` already removed any prior regular port
    // artifact under the held profile claim. Recompute only the path to poll;
    // deletion errors must never be ignored here.
    let port_file = config.user_data_dir.join("DevToolsActivePort");

    let mut builder = nomi_process_runtime::ChildProcessBuilder::new(&config.chrome_path);
    builder
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let process = builder.spawn_managed().map_err(|e| {
        tracing::debug!(
            target: "nomi_browser_engine::launch",
            error_kind = ?e.kind(),
            "managed Chromium spawn failed"
        );
        safe_chromium_spawn_error()
    })?;
    uncommitted.attach_process(process);
    let ownership_claim = uncommitted.ownership_claim;
    let ownership_token = match commit_browser_ownership(
        config,
        ownership_claim,
        uncommitted.process(),
        uncommitted.ephemeral_cleanup.as_ref(),
    )
    .await
    {
            Ok(token) => token,
            Err(primary) => {
                let cleanup = uncommitted.cleanup_under_claim().await;
                return Err(launch_error_after_cleanup(primary, cleanup));
            }
    };
    let mut committed = uncommitted.into_committed(ownership_token);

    // 轮询 DevToolsActivePort 直到出现且可解析，或 child 提前退出，或超时。
    let deadline = Instant::now() + PORT_FILE_TIMEOUT;
    loop {
        if let Ok(Some(status)) = committed.process_mut().child_mut().try_wait() {
            let primary = BrowserError::Other(format!(
                "chrome exited before DevTools port was ready (status {status})"
            ));
            let cleanup = committed.cleanup_under_claim(ownership_claim).await;
            return Err(launch_error_after_cleanup(primary, cleanup));
        }
        if let Ok(content) = std::fs::read_to_string(&port_file) {
            if let Ok((port, ws_path)) = parse_devtools_active_port(&content) {
                let ws_url = build_ws_url(port, &ws_path);
                return Ok(Launched::new(
                    LaunchTransport::Ws { ws_url },
                    committed,
                ));
            }
        }
        if Instant::now() >= deadline {
            let cleanup = committed.cleanup_under_claim(ownership_claim).await;
            return Err(launch_error_after_cleanup(
                safe_devtools_timeout_error(),
                cleanup,
            ));
        }
        tokio::time::sleep(PORT_FILE_POLL_INTERVAL).await;
    }
}

async fn commit_browser_ownership(
    config: &LaunchConfig,
    ownership_claim: &crate::profile::ProfileLaunchClaim,
    process: &nomi_process_runtime::ManagedChildProcess,
    provisional_cleanup: Option<&crate::profile::EphemeralProfileCleanupToken>,
) -> Result<crate::profile::BrowserOwnershipToken, BrowserError> {
    let Some(_) = process.id() else {
        return Err(BrowserError::Other(
            "spawned browser exited before ownership commit".into(),
        ));
    };
    crate::profile::write_browser_ownership_marker(
        ownership_claim,
        &config.user_data_dir,
        &config.chrome_path,
        process.child(),
        provisional_cleanup,
    )
    .await
    .map_err(|_| {
        tracing::warn!(
            target: "nomi_browser_engine::launch",
            "browser ownership marker commit failed; terminating the unowned process tree"
        );
        BrowserError::Other("browser ownership commit failed".into())
    })
}

#[cfg(windows)]
fn should_retry_with_startup_page(error: &BrowserError, args: &[String]) -> bool {
    args.iter().any(|a| a == "--no-startup-window")
        && matches!(
            error,
            BrowserError::Other(message)
                if message.contains("chrome exited before DevTools port was ready")
        )
}

#[cfg(windows)]
fn chrome_args_with_startup_page(args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = args
        .iter()
        .filter(|a| a.as_str() != "--no-startup-window")
        .cloned()
        .collect();
    out.push("about:blank".into());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dropped_cleanup_ticket_is_persistent_for_racing_multiple_and_late_waiters() {
        let ticket = DroppedBrowserCleanupTicket::pending();
        let waiters = (0..4)
            .map(|_| {
                let ticket = ticket.clone();
                tokio::spawn(async move { ticket.wait_or_retry().await })
            })
            .collect::<Vec<_>>();

        // Completion may happen before any spawned waiter is first polled.
        ticket.publish_complete();
        for waiter in waiters {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), waiter)
                    .await
                    .expect("racing ticket waiter cannot lose completion")
                    .expect("ticket waiter joins"),
                DroppedBrowserCleanupCompletion::Complete
            );
        }
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), ticket.wait_or_retry())
                .await
                .expect("late ticket waiter returns immediately"),
            DroppedBrowserCleanupCompletion::Complete
        );
        ticket.publish_complete();
        assert_eq!(
            ticket.wait_or_retry().await,
            DroppedBrowserCleanupCompletion::Complete,
            "a later Drop/defer cannot downgrade exact cleanup success"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cleanup_thread_failure_returns_reclaimable_ticket_to_shutdown_caller() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("ticket-worker-failure");
        std::fs::create_dir_all(&profile).unwrap();
        let claim = crate::profile::prepare_ownership_marker_for_launch(&profile)
            .expect("exclusive launch claim");
        let (process, executable) = spawn_long_running_fixture();
        let token = crate::profile::write_browser_ownership_marker(
            &claim,
            &profile,
            &executable,
            process.child(),
            None,
        )
        .await
        .expect("commit exact ownership marker");
        let pid = process.id().expect("fixture child pid");
        let process = std::sync::Arc::new(tokio::sync::Mutex::new(process));
        std::fs::write(
            profile.join("DevToolsActivePort"),
            b"9222\n/devtools/browser/ticket-reclaim\n",
        )
        .unwrap();

        let failure_injection = BrowserCleanupThreadFailureInjection::arm();
        let ticket = hand_off_dropped_browser_cleanup(
            process.clone(),
            token,
            Some(profile.clone()),
        );
        drop(failure_injection);
        drop(claim);

        let held_process = process.lock().await;
        let takeover_ticket = ticket.clone();
        let takeover =
            tokio::spawn(async move { takeover_ticket.wait_or_retry().await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !takeover.is_finished(),
            "a pending process cleanup must never be reported as complete"
        );
        drop(held_process);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), takeover)
                .await
                .expect("shutdown caller takes over the retained cleanup job")
                .expect("takeover task joins"),
            DroppedBrowserCleanupCompletion::Complete
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), ticket.wait_or_retry())
                .await
                .expect("ticket publishes sticky completion"),
            DroppedBrowserCleanupCompletion::Complete
        );
        wait_for_process_exit(pid).await;
        assert!(
            !profile.exists(),
            "successful ticket takeover removes exact marker, port, and ephemeral profile"
        );
        assert_eq!(
            ticket.wait_or_retry().await,
            DroppedBrowserCleanupCompletion::Complete,
            "repeated shutdown observes sticky completion"
        );
    }

    fn spawn_long_running_fixture(
    ) -> (
        nomi_process_runtime::ManagedChildProcess,
        PathBuf,
    ) {
        #[cfg(windows)]
        {
            let shell = PathBuf::from(
                std::env::var_os("COMSPEC").expect("Windows COMSPEC identifies cmd.exe"),
            );
            let mut builder = nomi_process_runtime::ChildProcessBuilder::new(&shell);
            builder
                .args([
                    "/D",
                    "/S",
                    "/C",
                    "ping -n 60 127.0.0.1 >NUL",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            return (
                builder.spawn_managed().expect("spawn Windows process fixture"),
                shell,
            );
        }

        #[cfg(unix)]
        {
            // Spawn the sleeper directly: a `/bin/sh -c` wrapper may exec into
            // sleep, so the committed ownership marker would name a different
            // executable than the live process image (darwin rejects that).
            let executable = PathBuf::from("/bin/sleep");
            let mut builder = nomi_process_runtime::ChildProcessBuilder::new(&executable);
            builder
                .arg("60")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            (
                builder.spawn_managed().expect("spawn Unix process fixture"),
                executable,
            )
        }
    }

    async fn wait_for_process_exit(pid: u32) {
        tokio::time::timeout(Duration::from_secs(10), async move {
            loop {
                use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
                let mut system = sysinfo::System::new();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing(),
                );
                if system.process(sysinfo::Pid::from_u32(pid)).is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("managed fixture process exits within cleanup deadline");
    }

    #[test]
    fn args_include_port_user_data_dir_and_hardening() {
        let dir = Path::new("/tmp/nomi-udd");
        let args = build_chrome_args(dir, false);

        // 运输开关随平台：Unix=--remote-debugging-pipe（fd3/fd4 自死），Windows=--remote-debugging-port=0。
        #[cfg(unix)]
        assert!(
            args.iter().any(|a| a == "--remote-debugging-pipe"),
            "missing --remote-debugging-pipe flag: {args:?}"
        );
        #[cfg(windows)]
        assert!(
            args.iter().any(|a| a == "--remote-debugging-port=0"),
            "missing --remote-debugging-port=0 flag: {args:?}"
        );
        assert!(
            args.iter().any(|a| a == "--user-data-dir=/tmp/nomi-udd"),
            "missing user-data-dir: {args:?}"
        );
        // 硬化基线关键项必须透传。
        assert!(args.iter().any(|a| a == "--disable-background-networking"));
        assert!(args.iter().any(|a| a == "--disable-component-update"));
        assert!(args.iter().any(|a| a.starts_with("--disable-features=")));
        assert!(args.iter().any(|a| a == "--no-first-run"));
        assert!(args.iter().any(|a| a == "--no-default-browser-check"));
        // 不自动开启动窗口（消除冗余命令行 about:blank；受控页由 backend createTarget 建）。
        assert!(args.iter().any(|a| a == "--no-startup-window"));
        assert!(
            !args.iter().any(|a| a == "about:blank"),
            "命令行不应再带 about:blank 起始页（受控页由 createTarget 建）: {args:?}"
        );
    }

    #[test]
    fn extra_chrome_args_are_fail_closed_for_security_sensitive_switches() {
        let extra = [
            "--user-data-dir=C:\\attacker\\profile",
            "--profile-directory=Default",
            "--remote-debugging-port=9222",
            "--remote-debugging-address=0.0.0.0",
            "--remote-debugging-pipe",
            "--load-extension=C:\\attacker\\extension",
            "--disable-extensions-except=C:\\attacker\\extension",
            "--no-sandbox",
            "--disable-setuid-sandbox",
            "--disable-web-security",
            "--allow-running-insecure-content",
            "--disable-features=IsolateOrigins",
            "--enable-features=NetworkServiceInProcess",
            "--proxy-server=http://attacker.invalid:8080",
            "--host-resolver-rules=MAP * 0.0.0.0",
            "https://attacker.invalid",
            "--site-per-process",
            "--host-resolver-rules=MAP *.nomitest 127.0.0.1",
        ]
        .join("\n");

        let filtered = filtered_extra_chrome_args(&extra);

        #[cfg(debug_assertions)]
        assert_eq!(
            filtered,
            vec![
                "--site-per-process".to_string(),
                "--host-resolver-rules=MAP *.nomitest 127.0.0.1".to_string(),
            ]
        );

        #[cfg(not(debug_assertions))]
        assert!(
            filtered.is_empty(),
            "release builds must not accept ambient Chromium switches: {filtered:?}"
        );

        for rejected in [
            "--user-data-dir=",
            "--profile-directory=",
            "--remote-debugging-port=",
            "--remote-debugging-address=",
            "--remote-debugging-pipe",
            "--load-extension=",
            "--disable-extensions-except=",
            "--no-sandbox",
            "--disable-setuid-sandbox",
            "--disable-web-security",
            "--allow-running-insecure-content",
            "--disable-features=",
            "--enable-features=",
            "--proxy-server=",
            "--host-resolver-rules=MAP * 0.0.0.0",
            "https://attacker.invalid",
        ] {
            assert!(
                !filtered.iter().any(|arg| arg == rejected),
                "sensitive or unapproved switch was accepted: {rejected}; got {filtered:?}"
            );
        }
    }

    #[test]
    fn extra_chrome_args_cannot_override_managed_profile_or_transport() {
        let managed_profile = Path::new("/managed/profile");
        let mut args = build_chrome_args(managed_profile, true);
        args.extend(filtered_extra_chrome_args(
            "--user-data-dir=/attacker/profile\n\
             --profile-directory=Attacker\n\
             --remote-debugging-port=9222\n\
             --remote-debugging-address=0.0.0.0\n\
             --load-extension=/attacker/extension\n\
             --no-sandbox\n\
             --disable-web-security\n\
             --site-per-process",
        ));

        assert_eq!(
            args.iter()
                .filter(|arg| arg.starts_with("--user-data-dir="))
                .count(),
            1,
            "managed profile must remain unique: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--user-data-dir=/attacker/")),
            "untrusted profile override must be absent: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--profile-directory=")),
            "profile-directory must not be caller-controlled: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--remote-debugging-port=9222")),
            "remote debugging port must remain managed: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--remote-debugging-address=")),
            "remote debugging bind address must remain managed: {args:?}"
        );
        assert!(
            args.iter()
                .all(|arg| !arg.starts_with("--load-extension=")),
            "extensions must not be caller-controlled: {args:?}"
        );
        assert!(
            args.iter().all(|arg| arg != "--disable-web-security"),
            "security weakening switch must be absent: {args:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_retry_args_replace_no_startup_window_with_startup_page() {
        let args = vec![
            "--remote-debugging-port=0".to_string(),
            "--no-startup-window".to_string(),
            "--disable-background-networking".to_string(),
        ];
        let fallback = chrome_args_with_startup_page(&args);
        assert!(!fallback.iter().any(|a| a == "--no-startup-window"));
        assert!(fallback.iter().any(|a| a == "about:blank"));
        assert!(should_retry_with_startup_page(
            &BrowserError::Other(
                "chrome exited before DevTools port was ready (status exit code: 0)".into()
            ),
            &args
        ));
    }

    #[tokio::test]
    async fn ownership_commit_failure_cleans_exact_uncommitted_ephemeral_profile() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("uncommitted-commit-failure");
        std::fs::create_dir_all(&profile).unwrap();
        let claim = crate::profile::prepare_ownership_marker_for_launch(&profile)
            .expect("exclusive launch claim");
        let cleanup_token =
            crate::profile::claim_ephemeral_profile_cleanup(&profile, &claim)
                .expect("durable provisional cleanup marker");
        let (process, _) = spawn_long_running_fixture();
        let pid = process.id().expect("fixture child pid");
        let mut uncommitted =
            UncommittedLaunchGuard::new(&claim, Some(cleanup_token));
        uncommitted.attach_process(process);
        let config = LaunchConfig {
            chrome_path: std::env::current_exe().expect("test executable path"),
            user_data_dir: profile.clone(),
            headful: false,
        };

        let error = commit_browser_ownership(
            &config,
            &claim,
            uncommitted.process(),
            uncommitted.ephemeral_cleanup.as_ref(),
        )
        .await
        .expect_err("mismatched executable forces a pre-commit failure");
        assert!(
            error.to_string().contains("ownership commit failed"),
            "{error}"
        );
        uncommitted
            .cleanup_under_claim()
            .await
            .expect("commit error proves tree exit before deleting exact profile");

        wait_for_process_exit(pid).await;
        assert!(
            !profile.exists(),
            "uncommitted ephemeral profile must not survive commit failure"
        );
    }

    #[tokio::test]
    async fn ownership_commit_failure_never_deletes_a_stable_profile() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("stable-commit-failure");
        let stable_data = profile.join("Default").join("Cookies");
        std::fs::create_dir_all(stable_data.parent().unwrap()).unwrap();
        std::fs::write(&stable_data, b"keep").unwrap();
        let claim = crate::profile::prepare_ownership_marker_for_launch(&profile)
            .expect("exclusive launch claim");
        let (process, _) = spawn_long_running_fixture();
        let pid = process.id().expect("fixture child pid");
        let mut uncommitted = UncommittedLaunchGuard::new(&claim, None);
        uncommitted.attach_process(process);
        let config = LaunchConfig {
            chrome_path: std::env::current_exe().expect("test executable path"),
            user_data_dir: profile.clone(),
            headful: false,
        };

        commit_browser_ownership(&config, &claim, uncommitted.process(), None)
            .await
            .expect_err("mismatched executable forces a pre-commit failure");
        uncommitted
            .cleanup_under_claim()
            .await
            .expect("stable commit error still proves process-tree exit");

        wait_for_process_exit(pid).await;
        assert_eq!(std::fs::read(&stable_data).unwrap(), b"keep");
        assert!(
            profile.is_dir(),
            "stable profile must never enter whole-directory cleanup"
        );
    }

    #[tokio::test]
    async fn abort_before_marker_commit_keeps_process_and_ephemeral_cleanup_indivisible() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("cancelled-uncommitted-launch");
        std::fs::create_dir_all(&profile).unwrap();
        let task_profile = profile.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let claim = crate::profile::prepare_ownership_marker_for_launch(&task_profile)
                .expect("exclusive launch claim");
            let cleanup_token =
                crate::profile::claim_ephemeral_profile_cleanup(&task_profile, &claim)
                    .expect("durable provisional cleanup marker");
            let (process, _) = spawn_long_running_fixture();
            let pid = process.id().expect("fixture child pid");
            let mut uncommitted =
                UncommittedLaunchGuard::new(&claim, Some(cleanup_token));
            uncommitted.attach_process(process);
            ready_tx.send(pid).unwrap();
            std::future::pending::<()>().await;
        });

        let pid = ready_rx.await.expect("guard reached pre-commit state");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
                let mut system = sysinfo::System::new();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing(),
                );
                if system.process(sysinfo::Pid::from_u32(pid)).is_none()
                    && !profile.exists()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("cancelled pre-commit launch reaps tree before deleting profile");
    }

    #[tokio::test]
    async fn uncommitted_worker_failure_preserves_durable_startup_lineage() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("uncommitted-worker-failure");
        std::fs::create_dir_all(&profile).unwrap();
        let claim = crate::profile::prepare_ownership_marker_for_launch(&profile)
            .expect("exclusive launch claim");
        let cleanup_token =
            crate::profile::claim_ephemeral_profile_cleanup(&profile, &claim)
                .expect("durable provisional cleanup marker");
        let (process, _) = spawn_long_running_fixture();
        let pid = process.id().expect("fixture child pid");
        let mut uncommitted =
            UncommittedLaunchGuard::new(&claim, Some(cleanup_token));
        uncommitted.attach_process(process);

        let _failure_injection = BrowserCleanupThreadFailureInjection::arm();
        drop(uncommitted);
        drop(claim);
        wait_for_process_exit(pid).await;

        assert!(
            profile
                .join(crate::profile::OWNERSHIP_MARKER_FILE)
                .is_file(),
            "worker failure must leave startup-visible provisional lineage"
        );
        assert!(
            crate::profile::prepare_ownership_marker_for_launch(&profile).is_err(),
            "the live owner must fail closed instead of silently reusing provisional state"
        );
    }

    #[test]
    fn cleanup_thread_failure_injection_cannot_be_stolen_by_another_test_thread() {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let armed = std::thread::spawn(move || {
            let _injection = BrowserCleanupThreadFailureInjection::arm();
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            assert!(take_browser_cleanup_thread_failure_injection());
        });

        ready_rx.recv().unwrap();
        assert!(
            !take_browser_cleanup_thread_failure_injection(),
            "another OS thread must not consume the scoped injection"
        );
        release_tx.send(()).unwrap();
        armed.join().unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn abort_after_marker_commit_retains_exact_cleanup_authority() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("cancelled-committed-launch");
        std::fs::create_dir_all(&profile).unwrap();
        let command_shell = PathBuf::from(
            std::env::var_os("COMSPEC").expect("Windows COMSPEC identifies cmd.exe"),
        );
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let task_profile = profile.clone();
        let task_shell = command_shell.clone();
        let task = tokio::spawn(async move {
            // Declaration order is intentional: cancellation drops the guard
            // before releasing the launch claim, covering the initial lock
            // collision and the independent worker's retry.
            let claim = crate::profile::prepare_ownership_marker_for_launch(&task_profile)
                .expect("exclusive launch claim");
            let mut builder =
                nomi_process_runtime::ChildProcessBuilder::new(&task_shell);
            builder
                .args([
                    "/D",
                    "/S",
                    "/C",
                    "ping -n 60 127.0.0.1 >NUL",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let process = builder.spawn_managed().expect("spawn cancellable fixture");
            let token = crate::profile::write_browser_ownership_marker(
                &claim,
                &task_profile,
                &task_shell,
                process.child(),
                None,
            )
            .await
            .expect("commit exact ownership marker");
            std::fs::write(
                task_profile.join("DevToolsActivePort"),
                b"9222\n/devtools/browser/cancelled\n",
            )
            .unwrap();
            let pid = process.id().expect("fixture child pid");
            let _guard =
                CommittedLaunchGuard::new(process, token, Some(task_profile.clone()));
            ready_tx.send(pid).unwrap();
            std::future::pending::<()>().await;
        });

        let pid = ready_rx.await.expect("guard reached committed state");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
                let mut system = sysinfo::System::new();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing(),
                );
                let process_gone =
                    system.process(sysinfo::Pid::from_u32(pid)).is_none();
                if process_gone && !profile.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("cancelled committed launch cleans process and whole ephemeral profile");

        std::fs::create_dir_all(&profile).unwrap();
        crate::profile::prepare_ownership_marker_for_launch(&profile)
            .expect("same profile path is reusable after cancelled launch cleanup");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cleanup_thread_failure_releases_process_to_durable_relay_and_startup_audit() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("cleanup-thread-failure");
        std::fs::create_dir_all(&profile).unwrap();
        let command_shell = PathBuf::from(
            std::env::var_os("COMSPEC").expect("Windows COMSPEC identifies cmd.exe"),
        );
        let claim = crate::profile::prepare_ownership_marker_for_launch(&profile)
            .expect("exclusive launch claim");
        let mut builder = nomi_process_runtime::ChildProcessBuilder::new(&command_shell);
        builder
            .args([
                "/D",
                "/S",
                "/C",
                "ping -n 60 127.0.0.1 >NUL",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let process = builder.spawn_managed().expect("spawn relay fixture");
        let token = crate::profile::write_browser_ownership_marker(
            &claim,
            &profile,
            &command_shell,
            process.child(),
            None,
        )
        .await
        .expect("commit exact ownership marker");
        let pid = process.id().expect("fixture child pid");
        std::fs::write(
            profile.join("DevToolsActivePort"),
            b"9222\n/devtools/browser/deferred\n",
        )
        .unwrap();

        let _failure_injection = BrowserCleanupThreadFailureInjection::arm();
        drop(CommittedLaunchGuard::new(process, token, None));
        drop(claim);

        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
                let mut system = sysinfo::System::new();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing(),
                );
                if system.process(sysinfo::Pid::from_u32(pid)).is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("thread failure must still release the child to process-runtime cleanup");

        assert!(
            profile
                .join(crate::profile::OWNERSHIP_MARKER_FILE)
                .is_file(),
            "artifact lineage stays fail-closed when exact browser worker cannot start"
        );
        assert!(profile.join("DevToolsActivePort").is_file());
        let recovery_claim =
            crate::profile::prepare_ownership_marker_for_launch(&profile)
                .expect("startup preflight recovers the now-absent exact browser");
        drop(recovery_claim);
        assert!(
            !profile
                .join(crate::profile::OWNERSHIP_MARKER_FILE)
                .exists()
        );
        assert!(!profile.join("DevToolsActivePort").exists());
    }

    #[test]
    fn headless_flag_only_when_forced() {
        let dir = Path::new("/tmp/x");
        let headless = build_chrome_args(dir, true);
        assert!(
            headless.iter().any(|a| a == "--headless=new"),
            "force_headless must add --headless=new: {headless:?}"
        );
        // headless 时不该有 headful 的窗口摆位开关。
        assert!(!headless.iter().any(|a| a.starts_with("--window-position")));
        assert!(
            !headless.iter().any(|a| a == "--start-minimized"),
            "headless must not receive a window-only minimized flag: {headless:?}"
        );

        let headful = build_chrome_args(dir, false);
        assert!(
            !headful.iter().any(|a| a == "--headless=new"),
            "headful must NOT add --headless=new: {headful:?}"
        );
        // headful 时给窗口摆位/尺寸。
        assert!(headful.iter().any(|a| a.starts_with("--window-position")));
        assert!(headful.iter().any(|a| a.starts_with("--window-size")));
        assert!(
            !headful.iter().any(|a| a == "--start-minimized"),
            "headful must be a normal explicitly requested window: {headful:?}"
        );
    }

    #[test]
    fn typed_launch_modes_never_use_a_minimized_window_as_headless() {
        let dir = Path::new("/tmp/x");
        let headless = build_chrome_args_for_mode(dir, BrowserHostLaunchMode::Headless);
        let headful = build_chrome_args_for_mode(dir, BrowserHostLaunchMode::Headful);

        assert!(headless.iter().any(|arg| arg == "--headless=new"));
        assert!(!headful.iter().any(|arg| arg == "--headless=new"));
        assert!(!headless.iter().any(|arg| arg == "--start-minimized"));
        assert!(!headful.iter().any(|arg| arg == "--start-minimized"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_container_falls_back_to_no_sandbox() {
        // TODO(verify-linux): 当前无条件回退 --no-sandbox（偏保守）；容器探测见
        // docs/superpowers/specs/browser-use/PLATFORM-VERIFICATION.md。
        let args = build_chrome_args(Path::new("/tmp/x"), true);
        assert!(
            args.iter().any(|a| a == "--no-sandbox"),
            "linux must add --no-sandbox: {args:?}"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_has_no_no_sandbox() {
        let args = build_chrome_args(Path::new("/tmp/x"), true);
        assert!(!args.iter().any(|a| a == "--no-sandbox"));
    }

    #[test]
    fn parse_active_port_two_lines() {
        let content = "54213\n/devtools/browser/4f1c0a2b-aaaa-bbbb-cccc-ddddeeeeffff\n";
        let (port, path) = parse_devtools_active_port(content).unwrap();
        assert_eq!(port, 54213);
        assert_eq!(
            path,
            "/devtools/browser/4f1c0a2b-aaaa-bbbb-cccc-ddddeeeeffff"
        );
        assert_eq!(
            build_ws_url(port, &path),
            "ws://127.0.0.1:54213/devtools/browser/4f1c0a2b-aaaa-bbbb-cccc-ddddeeeeffff"
        );
    }

    #[test]
    fn parse_active_port_trims_whitespace() {
        // chrome 可能不带末尾换行；也容忍行内多余空白。
        let content = "  9333  \n  /devtools/browser/x  ";
        let (port, path) = parse_devtools_active_port(content).unwrap();
        assert_eq!(port, 9333);
        assert_eq!(path, "/devtools/browser/x");
    }

    #[test]
    fn parse_active_port_rejects_missing_lines() {
        assert!(parse_devtools_active_port("").is_err());
        assert!(parse_devtools_active_port("54213").is_err()); // 缺第二行
    }

    #[test]
    fn parse_active_port_rejects_bad_port() {
        assert!(parse_devtools_active_port("notaport\n/devtools/browser/x").is_err());
        assert!(parse_devtools_active_port("0\n/devtools/browser/x").is_err()); // 0=未绑定
    }

    #[test]
    fn active_port_parse_errors_do_not_echo_endpoint_material() {
        let private_port_line = "12345-private-port-sentinel";
        let error = parse_devtools_active_port(&format!(
            "{private_port_line}\n/devtools/browser/private-token"
        ))
        .unwrap_err()
        .to_string();
        assert!(!error.contains(private_port_line));
        assert!(!error.contains("private-token"));
        assert!(!error.contains("12345"));

        let private_ws_path = "ws://127.0.0.1:12345/devtools/browser/private-token";
        let error = parse_devtools_active_port(&format!("9333\n{private_ws_path}"))
            .unwrap_err()
            .to_string();
        assert!(!error.contains(private_ws_path));
        assert!(!error.contains("private-token"));
        assert!(!error.contains("12345"));
    }

    #[test]
    fn parse_active_port_rejects_non_absolute_ws_path() {
        assert!(parse_devtools_active_port("9333\ndevtools/browser/x").is_err());
    }

    #[test]
    fn launch_boundary_errors_do_not_echo_private_paths_or_endpoints() {
        let profile_path = r"C:\secret\profile";
        let chrome_path = r"C:\secret\Chrome\chrome.exe";
        let ws_endpoint = "ws://127.0.0.1:12345/devtools/browser/private-token";
        let errors = [
            safe_profile_prepare_error(),
            safe_profile_ownership_error(),
            safe_chromium_spawn_error(),
            safe_devtools_timeout_error(),
        ];

        for error in errors {
            let display = error.to_string();
            assert!(!display.contains(profile_path));
            assert!(!display.contains(chrome_path));
            assert!(!display.contains(ws_endpoint));
            assert!(!display.contains("private-token"));
            assert!(!display.contains("12345"));
        }
    }

    #[test]
    fn launch_config_debug_does_not_echo_private_paths() {
        let chrome_path = "LAUNCH-CHROMIUM-PATH-SENTINEL";
        let profile_path = "LAUNCH-PROFILE-PATH-SENTINEL";
        let config = LaunchConfig {
            chrome_path: PathBuf::from(chrome_path),
            user_data_dir: PathBuf::from(profile_path),
            headful: true,
        };

        let debug = format!("{config:?}");
        assert!(!debug.contains(chrome_path), "{debug}");
        assert!(!debug.contains(profile_path), "{debug}");
        assert!(debug.contains("chrome_path_configured"));
        assert!(debug.contains("user_data_dir_configured"));
        assert!(debug.contains("headful: true"));
    }

    #[tokio::test]
    async fn profile_prepare_failure_does_not_echo_the_profile_or_executable_path() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("private-profile-sentinel");
        std::fs::write(&profile, b"not a directory").unwrap();
        let chrome = temp.path().join("private-chrome-sentinel");
        let config = LaunchConfig {
            chrome_path: chrome.clone(),
            user_data_dir: profile.clone(),
            headful: false,
        };

        let error = match launch_chrome(&config, true).await {
            Ok(_) => panic!("a regular file cannot be used as a browser profile"),
            Err(error) => error.to_string(),
        };
        assert!(!error.contains(&profile.display().to_string()));
        assert!(!error.contains(&chrome.display().to_string()));
        assert!(!error.contains("private-profile-sentinel"));
        assert!(!error.contains("private-chrome-sentinel"));
    }

    #[tokio::test]
    async fn spawn_failure_does_not_echo_the_profile_or_executable_path() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("private-profile-sentinel");
        let chrome = temp.path().join("private-chrome-sentinel");
        let config = LaunchConfig {
            chrome_path: chrome.clone(),
            user_data_dir: profile.clone(),
            headful: false,
        };

        let error = match launch_chrome(&config, true).await {
            Ok(_) => panic!("a nonexistent Chromium executable cannot launch"),
            Err(error) => error.to_string(),
        };
        assert!(!error.contains(&profile.display().to_string()));
        assert!(!error.contains(&chrome.display().to_string()));
        assert!(!error.contains("private-profile-sentinel"));
        assert!(!error.contains("private-chrome-sentinel"));
    }
}
