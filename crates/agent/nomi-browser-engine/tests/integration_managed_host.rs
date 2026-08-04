use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nomi_browser_engine::{
    BrowserEngine, BrowserError, EngineConfig, FirewallConfig, LaneEngineConfig,
    ManagedBrowserHost, TaskDownloadReservation, TaskDownloadReservationAuthority,
    TaskTabReservation, TaskTabReservationAuthority,
};
use nomi_browser_engine::profile::{COMMITTED_OWNERSHIP_RECORD_FILE, OWNERSHIP_MARKER_FILE};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CROSS_LANE_COUNT: usize = 4;
const CLUSTER_ATTEMPT_LANE_COUNT: usize = 16;

/// Stand-in for the platform Hub's cross-Host task authorities.
///
/// One managed Chromium Host serves Lanes belonging to *many* independent task
/// families; the per-task Lane budget lives in the Hub scheduler, not in the
/// engine (hence `TaskTabReservationAuthority::release_lane`'s no-op default
/// for platform authorities). Acceptance tests below assert Host-level
/// multiplexing and process/RSS behavior, so they supply a permissive Hub
/// stand-in here. The task tab/download budgets themselves are unit-tested
/// against the real implementations in `nomi-browser-engine`'s standalone
/// scope and in `nomifun-browser-platform`'s Hub.
struct HubStandInTaskAuthority;

struct HubStandInTabReservation;

impl TaskTabReservation for HubStandInTabReservation {}

#[async_trait::async_trait]
impl TaskTabReservationAuthority for HubStandInTaskAuthority {
    async fn reserve(
        &self,
        _task_resource_key: &str,
        _lane_id: &str,
        _reservation_key: &str,
    ) -> Result<Arc<dyn TaskTabReservation>, BrowserError> {
        Ok(Arc::new(HubStandInTabReservation))
    }
}

struct HubStandInDownloadReservation;

impl TaskDownloadReservation for HubStandInDownloadReservation {
    fn update_progress(
        &self,
        _received_bytes: u64,
        _total_bytes: Option<u64>,
    ) -> Result<(), BrowserError> {
        Ok(())
    }

    fn prepare_complete(&self, _actual_bytes: u64) -> Result<(), BrowserError> {
        Ok(())
    }

    fn finalize_complete(&self) {}
}

#[async_trait::async_trait]
impl TaskDownloadReservationAuthority for HubStandInTaskAuthority {
    async fn reserve(
        &self,
        _task_resource_key: &str,
        _lane_id: &str,
        _download_key: &str,
    ) -> Result<Arc<dyn TaskDownloadReservation>, BrowserError> {
        Ok(Arc::new(HubStandInDownloadReservation))
    }
}

/// Lane config for a platform-managed Host, with one logical task family per
/// Lane.
///
/// The standalone engine path deliberately binds one Host to one task scope and
/// caps it at `STANDALONE_MAX_LIVE_LANES_PER_SCOPE`, which is the product's
/// per-task Lane budget. Acceptance tests that need many Lanes on one Host are
/// modelling *several independent tasks* sharing a Host — the production
/// arrangement — so they use the platform-managed constructor and pass the
/// trusted task key plus authorities the way the platform adapter does.
fn platform_lane_config(lane_id: &str) -> LaneEngineConfig {
    let authority = Arc::new(HubStandInTaskAuthority);
    LaneEngineConfig {
        task_resource_key: Some(format!("acceptance-task:{lane_id}")),
        task_tab_reservation_authority: Some(authority.clone()),
        task_download_reservation_authority: Some(authority),
        // `max_task_tabs` comes from the default (the product's per-task tab
        // budget), which already satisfies the platform-managed guard.
        ..LaneEngineConfig::default()
    }
}

struct FixtureRequest {
    path: String,
    cookie: String,
    received_at: Instant,
    response: oneshot::Sender<FixtureResponse>,
}

#[derive(Debug)]
struct FixtureResponse {
    status: &'static str,
    headers: Vec<(&'static str, String)>,
    body: String,
}

impl FixtureResponse {
    fn html(body: impl Into<String>) -> Self {
        Self {
            status: "200 OK",
            headers: Vec::new(),
            body: body.into(),
        }
    }

    fn html_with_cookie(body: impl Into<String>, cookie: impl Into<String>) -> Self {
        Self {
            status: "200 OK",
            headers: vec![("Set-Cookie", cookie.into())],
            body: body.into(),
        }
    }
}

struct LocalFixture {
    address: std::net::SocketAddr,
    requests: mpsc::Receiver<FixtureRequest>,
    task: tokio::task::JoinHandle<()>,
}

impl LocalFixture {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, requests) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let request_tx = request_tx.clone();
                tokio::spawn(async move {
                    serve_fixture_request(socket, request_tx).await;
                });
            }
        });
        Self {
            address,
            requests,
            task,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.address, path)
    }

    async fn next_request(&mut self) -> FixtureRequest {
        tokio::time::timeout(REQUEST_TIMEOUT, self.requests.recv())
            .await
            .expect("browser request must reach the local fixture")
            .expect("local fixture request channel must remain open")
    }

    fn stop(self) {
        self.task.abort();
    }
}

async fn serve_fixture_request(
    mut socket: tokio::net::TcpStream,
    request_tx: mpsc::Sender<FixtureRequest>,
) {
    let mut request = Vec::with_capacity(4096);
    loop {
        let mut chunk = [0_u8; 4096];
        let read = socket.read(&mut chunk).await.unwrap();
        if read == 0 {
            return;
        }
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        assert!(
            request.len() <= 64 * 1024,
            "fixture request headers exceeded 64 KiB"
        );
    }

    let request = String::from_utf8_lossy(&request);
    let mut lines = request.lines();
    let request_line = lines.next().unwrap_or_default();
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let cookie = lines
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("cookie")
                    .then(|| value.trim().to_string())
            })
        })
        .unwrap_or_default();

    // Chromium may fetch this independently of the top-level navigation.  It
    // is not an acceptance operation and must never consume a test probe.
    let response = if path == "/favicon.ico" {
        FixtureResponse {
            status: "204 No Content",
            headers: Vec::new(),
            body: String::new(),
        }
    } else {
        let (response_tx, response_rx) = oneshot::channel();
        if request_tx
            .send(FixtureRequest {
                path,
                cookie,
                received_at: Instant::now(),
                response: response_tx,
            })
            .await
            .is_err()
        {
            return;
        }
        match response_rx.await {
            Ok(response) => response,
            Err(_) => return,
        }
    };

    let mut headers = String::new();
    for (name, value) in response.headers {
        headers.push_str(name);
        headers.push_str(": ");
        headers.push_str(&value);
        headers.push_str("\r\n");
    }
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        headers,
        response.body.len(),
        response.body
    );
    socket.write_all(response.as_bytes()).await.unwrap();
}

fn managed_config(
    data_dir: &std::path::Path,
    profile_name: &str,
    ephemeral_profile: bool,
) -> EngineConfig {
    EngineConfig {
        data_dir: data_dir.to_path_buf(),
        user_data_dir: Some(data_dir.join(profile_name)),
        ephemeral_profile,
        firewall: FirewallConfig {
            block_private_ips: false,
            gate_cross_origin_post: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[derive(Clone, Debug, Default)]
struct ProcessTreeSnapshot {
    pids: Vec<u32>,
    rss_bytes: u64,
}

fn sample_process_tree(system: &mut sysinfo::System, root_pid: u32) -> ProcessTreeSnapshot {
    sample_process_forest(system, &[root_pid])
}

fn sample_process_forest(
    system: &mut sysinfo::System,
    root_pids: &[u32],
) -> ProcessTreeSnapshot {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};

    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    let started_at_by_pid = system
        .processes()
        .values()
        .map(|process| (process.pid().as_u32(), process.start_time()))
        .collect::<HashMap<_, _>>();
    let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();
    for process in system.processes().values() {
        if let Some(parent) = process.parent() {
            let parent_pid = parent.as_u32();
            // Windows keeps an orphan's stale parent PID. If that numeric PID
            // is later reused by a test Chrome process, a parent-only walk
            // incorrectly counts the unrelated orphan tree and later reports
            // it as Chrome residue. Reject edges where the child predates the
            // current process occupying its recorded parent PID.
            let parent_is_not_newer = started_at_by_pid
                .get(&parent_pid)
                .is_none_or(|started_at| process.start_time() >= *started_at);
            if parent_is_not_newer {
                children_by_parent
                    .entry(parent_pid)
                    .or_default()
                    .push(process.pid().as_u32());
            }
        }
    }

    let mut pending = root_pids.to_vec();
    let mut visited = HashSet::new();
    let mut snapshot = ProcessTreeSnapshot::default();
    while let Some(pid) = pending.pop() {
        if !visited.insert(pid) {
            continue;
        }
        let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) else {
            continue;
        };
        snapshot.pids.push(pid);
        snapshot.rss_bytes = snapshot.rss_bytes.saturating_add(process.memory());
        if let Some(children) = children_by_parent.get(&pid) {
            pending.extend(children.iter().copied());
        }
    }
    snapshot.pids.sort_unstable();
    snapshot
}

#[derive(Debug, Default)]
struct TelemetrySummary {
    samples: u64,
    peak_rss_by_root: HashMap<u32, u64>,
    peak_processes_by_root: HashMap<u32, usize>,
}

struct TelemetrySampler {
    roots: Arc<Mutex<Vec<u32>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<TelemetrySummary>>,
}

impl TelemetrySampler {
    fn start(initial_root: u32) -> Self {
        let roots = Arc::new(Mutex::new(vec![initial_root]));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_roots = Arc::clone(&roots);
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut system = sysinfo::System::new();
            let mut summary = TelemetrySummary::default();
            while !thread_stop.load(Ordering::Acquire) {
                let roots = thread_roots.lock().unwrap().clone();
                for root in roots {
                    let snapshot = sample_process_tree(&mut system, root);
                    summary
                        .peak_rss_by_root
                        .entry(root)
                        .and_modify(|rss| *rss = (*rss).max(snapshot.rss_bytes))
                        .or_insert(snapshot.rss_bytes);
                    summary
                        .peak_processes_by_root
                        .entry(root)
                        .and_modify(|count| *count = (*count).max(snapshot.pids.len()))
                        .or_insert(snapshot.pids.len());
                }
                summary.samples += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            summary
        });
        Self {
            roots,
            stop,
            thread: Some(thread),
        }
    }

    fn add_root(&self, root: u32) {
        self.roots.lock().unwrap().push(root);
    }

    fn finish(mut self) -> TelemetrySummary {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap()
    }
}

#[derive(Clone, Debug, Default)]
struct AggregateTelemetrySummary {
    samples: u64,
    peak_rss_bytes: u64,
    peak_process_count: usize,
    peak_pids: Vec<u32>,
}

struct AggregateTelemetrySampler {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<AggregateTelemetrySummary>>,
}

impl AggregateTelemetrySampler {
    /// Sample all roots from one refreshed process table, then sum the union of
    /// their trees.  This avoids the invalid "sum of per-host peaks observed at
    /// different times" comparison.
    fn start(root_pids: Vec<u32>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut system = sysinfo::System::new();
            let mut summary = AggregateTelemetrySummary::default();
            while !thread_stop.load(Ordering::Acquire) {
                let snapshot = sample_process_forest(&mut system, &root_pids);
                if snapshot.rss_bytes > summary.peak_rss_bytes {
                    summary.peak_rss_bytes = snapshot.rss_bytes;
                    summary.peak_pids = snapshot.pids.clone();
                }
                summary.peak_process_count =
                    summary.peak_process_count.max(snapshot.pids.len());
                summary.samples += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            summary
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> AggregateTelemetrySummary {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap()
    }
}

async fn wait_for_processes_gone(pids: &[u32], deadline: Duration) -> Vec<u32> {
    let started = Instant::now();
    let mut system = sysinfo::System::new();
    loop {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let remaining = pids
            .iter()
            .copied()
            .filter(|pid| system.process(sysinfo::Pid::from_u32(*pid)).is_some())
            .collect::<Vec<_>>();
        if remaining.is_empty() || started.elapsed() >= deadline {
            return remaining;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_runtime_artifacts_gone(
    profile: &std::path::Path,
    deadline: Duration,
) -> bool {
    let started = Instant::now();
    loop {
        if !profile.join(OWNERSHIP_MARKER_FILE).exists()
            && !profile.join(COMMITTED_OWNERSHIP_RECORD_FILE).exists()
            && !profile.join("DevToolsActivePort").exists()
        {
            return true;
        }
        if started.elapsed() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn read_ownership_record(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(path).expect("read browser ownership record"),
    )
    .expect("parse browser ownership record")
}

/// Resolve the exact committed browser root PID for a managed profile.
///
/// Ephemeral profiles publish an append-only record pair: the provisional
/// record at [`OWNERSHIP_MARKER_FILE`] retains the owner-app identity, while
/// the exact browser identity is committed at
/// [`COMMITTED_OWNERSHIP_RECORD_FILE`]. Stable profiles have no provisional
/// predecessor and commit the browser identity directly at
/// [`OWNERSHIP_MARKER_FILE`].
fn committed_browser_pid(profile: &std::path::Path) -> u32 {
    let committed_path = profile.join(COMMITTED_OWNERSHIP_RECORD_FILE);
    let record = if committed_path.is_file() {
        read_ownership_record(&committed_path)
    } else {
        read_ownership_record(&profile.join(OWNERSHIP_MARKER_FILE))
    };
    assert_eq!(
        record["phase"].as_str(),
        Some("committed"),
        "browser identity must come from a committed ownership record"
    );
    record["browser"]["pid"]
        .as_u64()
        .expect("committed ownership record contains browser pid") as u32
}

/// Sample the managed process tree until Chromium's descendant processes are
/// visible, so post-Drop assertions cover the root and its children.
async fn sample_tree_with_descendants(
    root_pid: u32,
    deadline: Duration,
) -> ProcessTreeSnapshot {
    let started = Instant::now();
    let mut system = sysinfo::System::new();
    loop {
        let snapshot = sample_process_tree(&mut system, root_pid);
        if snapshot.pids.len() > 1 || started.elapsed() >= deadline {
            return snapshot;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Every live process whose command line references `path`. After cleanup this
/// proves no Chromium process still belongs to the isolated profile, without
/// touching browsers owned by anything else on the machine.
fn processes_referencing_path(path: &std::path::Path) -> Vec<u32> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};

    let needle = path.to_string_lossy().to_ascii_lowercase();
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    system
        .processes()
        .values()
        .filter(|process| {
            process.cmd().iter().any(|argument| {
                argument
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains(&needle)
            })
        })
        .map(|process| process.pid().as_u32())
        .collect()
}

#[cfg(windows)]
fn read_windows_debug_port(profile: &std::path::Path) -> u16 {
    let contents = std::fs::read_to_string(profile.join("DevToolsActivePort"))
        .expect("Windows managed Chromium must publish DevToolsActivePort");
    contents
        .lines()
        .next()
        .expect("DevToolsActivePort must contain a port")
        .parse()
        .expect("DevToolsActivePort first line must be a TCP port")
}

#[cfg(windows)]
async fn wait_for_debug_endpoint_closed(port: u16, deadline: Duration) -> bool {
    let started = Instant::now();
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
        {
            return true;
        }
        if started.elapsed() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Real-Chromium acceptance for exact stable-profile cleanup on normal Host
/// shutdown. The stable browsing state survives, while only the ownership
/// marker and Windows debugging endpoint artifact are removed after the
/// managed process tree is proven gone.
#[tokio::test]
#[ignore = "requires configured/bundled Chromium; set NOMIFUN_CHROME_BINARY and run with --ignored"]
async fn managed_host_shutdown_clears_only_exact_runtime_profile_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let profile_name = "stable-shutdown-cleanup";
    let profile = temp.path().join(profile_name);
    let stable_state = profile.join("Default").join("stable-state.json");
    std::fs::create_dir_all(stable_state.parent().unwrap()).unwrap();
    std::fs::write(&stable_state, br#"{"keep":true}"#).unwrap();

    let host = ManagedBrowserHost::launch(managed_config(
        temp.path(),
        profile_name,
        false,
    ))
    .await
    .expect("launch managed Host for shutdown cleanup");
    let root_pid = host.process_id().expect("managed Host root pid");
    let process_tree = sample_process_tree(&mut sysinfo::System::new(), root_pid);
    assert!(
        process_tree.pids.contains(&root_pid),
        "process-tree sample must include the managed root"
    );
    assert!(profile.join(OWNERSHIP_MARKER_FILE).is_file());
    #[cfg(windows)]
    let debug_port = {
        assert!(profile.join("DevToolsActivePort").is_file());
        read_windows_debug_port(&profile)
    };

    host.shutdown()
        .await
        .expect("normal Host shutdown proves process and profile cleanup");

    assert!(host.process_id().is_none());
    assert!(!profile.join(OWNERSHIP_MARKER_FILE).exists());
    assert!(!profile.join("DevToolsActivePort").exists());
    assert_eq!(std::fs::read(&stable_state).unwrap(), br#"{"keep":true}"#);
    #[cfg(windows)]
    assert!(
        wait_for_debug_endpoint_closed(debug_port, Duration::from_secs(5)).await,
        "stable-profile debugging endpoint remained reachable after shutdown"
    );
    assert!(
        wait_for_processes_gone(&process_tree.pids, Duration::from_secs(5))
            .await
            .is_empty(),
        "a sampled Chromium descendant remained after exact shutdown"
    );
}

#[tokio::test]
#[ignore = "requires configured/bundled Chromium; validates final Runtime Drop without explicit shutdown"]
async fn managed_host_drop_reaps_full_tree_and_ephemeral_profile() {
    let temp = tempfile::tempdir().unwrap();
    let profile_name = "ephemeral-drop-cleanup";
    let profile = temp.path().join(profile_name);
    let host =
        ManagedBrowserHost::launch(managed_config(temp.path(), profile_name, true))
            .await
            .expect("launch ephemeral managed Host");
    let lane = host
        .open_default_lane("drop-lane")
        .await
        .expect("open Lane retained by product caller");
    let root_pid = host.process_id().expect("managed Host root pid");

    // The ephemeral profile carries an append-only ownership record pair. The
    // provisional predecessor conservatively records the owner app as browser
    // identity; only the committed record names the actual Chromium root.
    let owner_app_pid = u64::from(std::process::id());
    let provisional = read_ownership_record(&profile.join(OWNERSHIP_MARKER_FILE));
    assert_eq!(provisional["phase"].as_str(), Some("provisional"));
    assert_eq!(provisional["owner_app"]["pid"].as_u64(), Some(owner_app_pid));
    assert_eq!(
        provisional["browser"]["pid"].as_u64(),
        Some(owner_app_pid),
        "provisional record must keep the owner app as its conservative browser identity"
    );
    let committed =
        read_ownership_record(&profile.join(COMMITTED_OWNERSHIP_RECORD_FILE));
    assert_eq!(committed["phase"].as_str(), Some("committed"));
    assert_eq!(committed["owner_app"]["pid"].as_u64(), Some(owner_app_pid));
    assert_eq!(
        committed["browser"]["pid"].as_u64(),
        Some(u64::from(root_pid)),
        "committed browser identity must match the managed Host root pid"
    );
    assert_eq!(committed_browser_pid(&profile), root_pid);

    // Sample the real Chromium tree rooted at the committed browser PID. The
    // descendants requirement makes the post-Drop assertion prove that the
    // Windows Job (and Unix process group) reaped more than just the root.
    let process_tree =
        sample_tree_with_descendants(root_pid, Duration::from_secs(5)).await;
    assert!(process_tree.pids.contains(&root_pid));
    assert!(
        process_tree.pids.len() > 1,
        "sampled Chromium tree must include descendants beyond the committed root: {:?}",
        process_tree.pids
    );
    #[cfg(windows)]
    let debug_port = read_windows_debug_port(&profile);

    // No explicit close_lane/shutdown: dropping the last Lane and Host must
    // reach CdpHostRuntime's exact Drop relay.
    drop(lane);
    drop(host);

    let residual = wait_for_processes_gone(&process_tree.pids, Duration::from_secs(10)).await;
    assert!(
        residual.is_empty(),
        "sampled Chromium processes survived final Runtime Drop: {residual:?}"
    );
    #[cfg(windows)]
    assert!(
        wait_for_debug_endpoint_closed(debug_port, Duration::from_secs(10)).await,
        "ephemeral Host debugging endpoint {debug_port} remained reachable after Drop"
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while profile.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("exact Drop cleanup removes the whole ephemeral profile");
    assert!(!profile.join(OWNERSHIP_MARKER_FILE).exists());
    assert!(!profile.join(COMMITTED_OWNERSHIP_RECORD_FILE).exists());
    let stragglers = processes_referencing_path(&profile);
    assert!(
        stragglers.is_empty(),
        "processes still reference the reaped ephemeral profile: {stragglers:?}"
    );
    println!(
        "ephemeral-drop acceptance: committed_browser_pid={} owner_app_pid={} \
         sampled_tree_pids={:?} residual_pids={:?} profile_removed=true \
         provisional_marker_removed=true committed_record_removed=true",
        root_pid, owner_app_pid, process_tree.pids, residual,
    );
}

#[tokio::test]
#[ignore = "requires configured/bundled Chromium; validates create_engine's hidden Host Drop path"]
async fn create_engine_drop_reaps_hidden_host_runtime_and_allows_stable_relaunch() {
    let temp = tempfile::tempdir().unwrap();
    let profile_name = "create-engine-stable-drop";
    let profile = temp.path().join(profile_name);
    let sentinel = profile.join("Default").join("stable-state.json");
    std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
    std::fs::write(&sentinel, br#"{"keep":true}"#).unwrap();

    for round in 0..2 {
        let engine = nomi_browser_engine::create_engine(managed_config(
            temp.path(),
            profile_name,
            false,
        ))
        .await
        .expect("create_engine returns a Lane after dropping its local Host coordinator");
        let root_pid = committed_browser_pid(&profile);
        let process_tree =
            sample_tree_with_descendants(root_pid, Duration::from_secs(5)).await;
        assert!(process_tree.pids.contains(&root_pid));

        drop(engine);

        let residual =
            wait_for_processes_gone(&process_tree.pids, Duration::from_secs(10)).await;
        assert!(
            residual.is_empty(),
            "create_engine round {round} left Chromium processes: {residual:?}"
        );
        assert!(
            wait_for_runtime_artifacts_gone(&profile, Duration::from_secs(10)).await,
            "create_engine round {round} left marker/port artifacts"
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), br#"{"keep":true}"#);
    }
}

/// Real-Chromium acceptance for the production Host/Lane path.
///
/// This deliberately does not claim all of specification section 9.  It proves
/// the engine-level pieces which require a real browser, using only a loopback
/// fixture:
///
/// - four independently owned lanes overlap on one Chromium host;
/// - two operations submitted to the same lane are serialized;
/// - target/DOM state does not cross lanes;
/// - Primary lanes on one stable-profile host share live cookie identity;
/// - an Anonymous host with a separate ephemeral profile neither reads nor
///   writes Primary identity;
/// - closing one lane leaves siblings alive;
/// - explicit host shutdown reaps the sampled Chromium process trees.
#[tokio::test]
#[ignore = "requires configured/bundled Chromium; set NOMIFUN_CHROME_BINARY and run with --ignored"]
async fn managed_host_real_chromium_acceptance_matrix() {
    let mut fixture = LocalFixture::start().await;
    let temp = tempfile::tempdir().unwrap();
    #[cfg_attr(not(windows), allow(unused_variables))]
    let primary_profile = temp.path().join("primary-profile");
    let primary_host = Arc::new(
        ManagedBrowserHost::launch_platform_managed(managed_config(temp.path(), "primary-profile", false))
            .await
            .unwrap(),
    );
    assert!(primary_host.epoch() > 0);
    let primary_pid = primary_host
        .process_id()
        .expect("managed Primary host must report its root pid");
    let telemetry = TelemetrySampler::start(primary_pid);
    #[cfg(windows)]
    let primary_debug_port = {
        let port = read_windows_debug_port(&primary_profile);
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("Primary DevTools endpoint must be live before shutdown");
        port
    };

    // Four operations must all reach the fixture while every response is held.
    // That is deterministic overlap evidence, independent of machine speed.
    let mut lanes: Vec<Arc<dyn BrowserEngine>> = Vec::new();
    for index in 0..CROSS_LANE_COUNT {
        lanes.push(
            primary_host
                .open_lane(
                    format!("overlap-{index}"),
                    platform_lane_config(&format!("overlap-{index}")),
                )
                .await
                .unwrap(),
        );
    }
    let overlap_started = Instant::now();
    let mut navigations = Vec::new();
    for (index, lane) in lanes.iter().enumerate() {
        let lane = Arc::clone(lane);
        let url = fixture.url(&format!("/lane-{index}"));
        navigations.push(tokio::spawn(async move {
            lane.navigate(&url, false).await
        }));
    }
    let mut probes = Vec::new();
    let mut paths = HashSet::new();
    for _ in 0..CROSS_LANE_COUNT {
        let probe = fixture.next_request().await;
        paths.insert(probe.path.clone());
        probes.push(probe);
    }
    assert_eq!(
        paths,
        (0..CROSS_LANE_COUNT)
            .map(|index| format!("/lane-{index}"))
            .collect(),
        "all independently owned lanes must reach the fixture"
    );
    let last_overlap_arrival = probes
        .iter()
        .map(|probe| probe.received_at.duration_since(overlap_started))
        .max()
        .unwrap();
    for probe in probes {
        let sentinel = probe.path.trim_start_matches('/').to_ascii_uppercase();
        probe
            .response
            .send(FixtureResponse::html(format!(
                "<html><body>{sentinel}_ONLY</body></html>"
            )))
            .unwrap();
    }
    for navigation in navigations {
        navigation.await.unwrap().unwrap();
    }
    for (index, lane) in lanes.iter().enumerate() {
        let html = lane.rendered_html().await.unwrap();
        assert!(html.contains(&format!("LANE-{index}_ONLY")));
        for other in 0..CROSS_LANE_COUNT {
            if other != index {
                assert!(
                    !html.contains(&format!("LANE-{other}_ONLY")),
                    "lane {index} must not render lane {other}'s target"
                );
            }
        }
    }

    // Same-lane serialization: the second request cannot reach the server
    // while the first navigation holds the lane operation gate.
    let serial_lane = primary_host
        .open_lane("serial", platform_lane_config("serial"))
        .await
        .unwrap();
    let serial_started = Instant::now();
    let first_url = fixture.url("/serial-first");
    let first_navigation = {
        let lane = Arc::clone(&serial_lane);
        tokio::spawn(async move { lane.navigate(&first_url, false).await })
    };
    let first_probe = fixture.next_request().await;
    assert_eq!(first_probe.path, "/serial-first");
    let second_url = fixture.url("/serial-second");
    let second_navigation = {
        let lane = Arc::clone(&serial_lane);
        tokio::spawn(async move { lane.navigate(&second_url, false).await })
    };
    let premature_second =
        tokio::time::timeout(Duration::from_millis(750), fixture.requests.recv()).await;
    assert!(
        premature_second.is_err(),
        "the same lane issued its second navigation before the first response"
    );
    first_probe
        .response
        .send(FixtureResponse::html("<html><body>SERIAL_FIRST</body></html>"))
        .unwrap();
    first_navigation.await.unwrap().unwrap();
    let second_probe = fixture.next_request().await;
    assert_eq!(second_probe.path, "/serial-second");
    let second_arrived_after = second_probe.received_at.duration_since(serial_started);
    second_probe
        .response
        .send(FixtureResponse::html(
            "<html><body>SERIAL_SECOND</body></html>",
        ))
        .unwrap();
    second_navigation.await.unwrap().unwrap();
    assert!(
        serial_lane
            .rendered_html()
            .await
            .unwrap()
            .contains("SERIAL_SECOND")
    );

    // Two Primary lanes use the exact same host and stable profile, so a
    // cookie written by one must be visible immediately in the other.
    let primary_a = primary_host
        .open_lane("primary-a", platform_lane_config("primary-a"))
        .await
        .unwrap();
    let primary_b = primary_host
        .open_lane("primary-b", platform_lane_config("primary-b"))
        .await
        .unwrap();
    let set_primary_url = fixture.url("/set-primary");
    let set_primary = {
        let lane = Arc::clone(&primary_a);
        tokio::spawn(async move { lane.navigate(&set_primary_url, false).await })
    };
    let set_primary_probe = fixture.next_request().await;
    assert_eq!(set_primary_probe.path, "/set-primary");
    set_primary_probe
        .response
        .send(FixtureResponse::html_with_cookie(
            "<html><body>PRIMARY_SET</body></html>",
            "nomifun_primary=shared; Path=/; SameSite=Lax",
        ))
        .unwrap();
    set_primary.await.unwrap().unwrap();
    let primary_echo_url = fixture.url("/primary-echo");
    let primary_echo = {
        let lane = Arc::clone(&primary_b);
        tokio::spawn(async move { lane.navigate(&primary_echo_url, false).await })
    };
    let primary_echo_probe = fixture.next_request().await;
    assert_eq!(primary_echo_probe.path, "/primary-echo");
    assert!(
        primary_echo_probe.cookie.contains("nomifun_primary=shared"),
        "the second Primary lane must observe the first lane's live cookie"
    );
    primary_echo_probe
        .response
        .send(FixtureResponse::html("<html><body>PRIMARY_SHARED</body></html>"))
        .unwrap();
    primary_echo.await.unwrap().unwrap();

    // Anonymous is a separate ephemeral host/profile.  Prove both directions:
    // it cannot read Primary, and a cookie it writes never flows to Primary.
    #[cfg_attr(not(windows), allow(unused_variables))]
    let anonymous_profile = temp.path().join("anonymous-profile");
    let anonymous_host = Arc::new(
        ManagedBrowserHost::launch_platform_managed(managed_config(temp.path(), "anonymous-profile", true))
            .await
            .unwrap(),
    );
    let anonymous_pid = anonymous_host
        .process_id()
        .expect("managed Anonymous host must report its root pid");
    telemetry.add_root(anonymous_pid);
    #[cfg(windows)]
    let anonymous_debug_port = {
        let port = read_windows_debug_port(&anonymous_profile);
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("Anonymous DevTools endpoint must be live before shutdown");
        port
    };
    assert_ne!(
        primary_pid, anonymous_pid,
        "Primary and Anonymous identity domains require separate host processes"
    );
    let anonymous = anonymous_host
        .open_lane("anonymous", platform_lane_config("anonymous"))
        .await
        .unwrap();
    let anonymous_echo_url = fixture.url("/anonymous-echo");
    let anonymous_echo = {
        let lane = Arc::clone(&anonymous);
        tokio::spawn(async move { lane.navigate(&anonymous_echo_url, false).await })
    };
    let anonymous_echo_probe = fixture.next_request().await;
    assert_eq!(anonymous_echo_probe.path, "/anonymous-echo");
    assert!(
        !anonymous_echo_probe.cookie.contains("nomifun_primary=shared"),
        "Anonymous must not read Primary identity"
    );
    anonymous_echo_probe
        .response
        .send(FixtureResponse::html_with_cookie(
            "<html><body>ANONYMOUS_SET</body></html>",
            "nomifun_anonymous=isolated; Path=/; SameSite=Lax",
        ))
        .unwrap();
    anonymous_echo.await.unwrap().unwrap();

    let primary_after_anonymous_url = fixture.url("/primary-after-anonymous");
    let primary_after_anonymous = {
        let lane = Arc::clone(&primary_b);
        tokio::spawn(async move { lane.navigate(&primary_after_anonymous_url, false).await })
    };
    let primary_after_anonymous_probe = fixture.next_request().await;
    assert_eq!(
        primary_after_anonymous_probe.path,
        "/primary-after-anonymous"
    );
    assert!(
        primary_after_anonymous_probe
            .cookie
            .contains("nomifun_primary=shared")
    );
    assert!(
        !primary_after_anonymous_probe
            .cookie
            .contains("nomifun_anonymous=isolated"),
        "Anonymous writes must not flow into Primary identity"
    );
    primary_after_anonymous_probe
        .response
        .send(FixtureResponse::html(
            "<html><body>PRIMARY_STILL_ISOLATED</body></html>",
        ))
        .unwrap();
    primary_after_anonymous.await.unwrap().unwrap();

    // Closing exactly one lane is bounded and leaves its siblings operational.
    let close_lane_started = Instant::now();
    primary_host.close_lane("overlap-0").await.unwrap();
    let close_lane_elapsed = close_lane_started.elapsed();
    assert!(close_lane_elapsed < Duration::from_secs(5));
    assert!(lanes[0].rendered_html().await.is_err());
    assert!(
        lanes[1]
            .rendered_html()
            .await
            .unwrap()
            .contains("LANE-1_ONLY")
    );

    let mut system = sysinfo::System::new();
    let primary_before_shutdown = sample_process_tree(&mut system, primary_pid);
    let anonymous_before_shutdown = sample_process_tree(&mut system, anonymous_pid);
    assert!(primary_before_shutdown.pids.contains(&primary_pid));
    assert!(anonymous_before_shutdown.pids.contains(&anonymous_pid));

    let shutdown_started = Instant::now();
    let (primary_shutdown, anonymous_shutdown) =
        tokio::join!(primary_host.shutdown(), anonymous_host.shutdown());
    primary_shutdown.unwrap();
    anonymous_shutdown.unwrap();
    let shutdown_elapsed = shutdown_started.elapsed();
    assert!(
        shutdown_elapsed < Duration::from_secs(5),
        "both managed hosts must shut down within five seconds; elapsed={shutdown_elapsed:?}"
    );
    assert!(primary_host.process_id().is_none());
    assert!(anonymous_host.process_id().is_none());
    #[cfg(windows)]
    {
        assert!(
            wait_for_debug_endpoint_closed(primary_debug_port, Duration::from_secs(5)).await,
            "Primary debugging endpoint {primary_debug_port} remained reachable"
        );
        assert!(
            wait_for_debug_endpoint_closed(anonymous_debug_port, Duration::from_secs(5)).await,
            "Anonymous debugging endpoint {anonymous_debug_port} remained reachable"
        );
    }

    let managed_pids = primary_before_shutdown
        .pids
        .iter()
        .chain(&anonymous_before_shutdown.pids)
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let residual = wait_for_processes_gone(&managed_pids, Duration::from_secs(5)).await;
    assert!(
        residual.is_empty(),
        "managed Chromium process residue after shutdown: {residual:?}"
    );
    let telemetry = telemetry.finish();
    #[cfg(windows)]
    let debug_endpoints = format!(
        "127.0.0.1:{primary_debug_port},127.0.0.1:{anonymous_debug_port} (both closed)"
    );
    #[cfg(not(windows))]
    let debug_endpoints = "remote-debugging-pipe (closed)".to_string();

    println!(
        "managed-host acceptance: lanes={} primary_pid={} anonymous_pid={} \
         cross_lane_last_arrival_ms={} same_lane_second_arrival_ms={} \
         close_lane_ms={} shutdown_ms={} process_tree_before_shutdown=primary:{} anonymous:{} \
         peak_rss_bytes=primary:{} anonymous:{} peak_processes=primary:{} anonymous:{} \
         telemetry_samples={} debug_endpoints={} primary_tree_pids={:?} \
         anonymous_tree_pids={:?} residual_pids={:?}",
        CROSS_LANE_COUNT,
        primary_pid,
        anonymous_pid,
        last_overlap_arrival.as_millis(),
        second_arrived_after.as_millis(),
        close_lane_elapsed.as_millis(),
        shutdown_elapsed.as_millis(),
        primary_before_shutdown.pids.len(),
        anonymous_before_shutdown.pids.len(),
        telemetry
            .peak_rss_by_root
            .get(&primary_pid)
            .copied()
            .unwrap_or_default(),
        telemetry
            .peak_rss_by_root
            .get(&anonymous_pid)
            .copied()
            .unwrap_or_default(),
        telemetry
            .peak_processes_by_root
            .get(&primary_pid)
            .copied()
            .unwrap_or_default(),
        telemetry
            .peak_processes_by_root
            .get(&anonymous_pid)
            .copied()
            .unwrap_or_default(),
        telemetry.samples,
        debug_endpoints,
        primary_before_shutdown.pids,
        anonymous_before_shutdown.pids,
        residual,
    );

    fixture.stop();
}

/// Real-Chromium acceptance for AC-CON-001's sixteen concurrent attempts.
///
/// Each lane navigates to a separate loopback origin so Chromium's per-origin
/// connection limit cannot serialize the fixture.  Every response remains
/// blocked until all sixteen requests have arrived, proving that one managed
/// Host can keep all Lane operations in flight at once.  Unique sentinels then
/// prove that each Lane retains its own target/DOM state.  Finally every Lane
/// is explicitly closed before Host shutdown, and all sampled process-tree
/// PIDs must be gone.
#[tokio::test]
#[ignore = "requires configured/bundled Chromium; set NOMIFUN_CHROME_BINARY and run alone with --ignored"]
async fn managed_host_sixteen_lane_real_chromium_acceptance() {
    let mut fixtures = Vec::with_capacity(CLUSTER_ATTEMPT_LANE_COUNT);
    for _ in 0..CLUSTER_ATTEMPT_LANE_COUNT {
        fixtures.push(LocalFixture::start().await);
    }

    let temp = tempfile::tempdir().unwrap();
    let profile_name = "sixteen-lane-profile";
    #[cfg_attr(not(windows), allow(unused_variables))]
    let profile = temp.path().join(profile_name);
    let host = Arc::new(
        ManagedBrowserHost::launch_platform_managed(managed_config(
            temp.path(),
            profile_name,
            false,
        ))
        .await
        .unwrap(),
    );
    assert!(host.epoch() > 0);
    let host_pid = host
        .process_id()
        .expect("managed sixteen-Lane host must report its root pid");
    #[cfg(windows)]
    let debug_port = {
        let port = read_windows_debug_port(&profile);
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("sixteen-Lane DevTools endpoint must be live before shutdown");
        port
    };

    let mut lanes: Vec<Arc<dyn BrowserEngine>> =
        Vec::with_capacity(CLUSTER_ATTEMPT_LANE_COUNT);
    for index in 0..CLUSTER_ATTEMPT_LANE_COUNT {
        lanes.push(
            host.open_lane(
                format!("cluster-attempt-{index}"),
                platform_lane_config(&format!("cluster-attempt-{index}")),
            )
            .await
            .unwrap(),
        );
    }
    assert_eq!(
        host.process_id(),
        Some(host_pid),
        "all sixteen Lanes must remain on the original managed Host"
    );

    let overlap_started = Instant::now();
    let mut navigations = Vec::with_capacity(CLUSTER_ATTEMPT_LANE_COUNT);
    for (index, lane) in lanes.iter().enumerate() {
        let lane = Arc::clone(lane);
        let url = fixtures[index].url(&format!("/cluster-attempt-{index}"));
        navigations.push(tokio::spawn(async move {
            lane.navigate(&url, false).await
        }));
    }

    // No response is released until all sixteen requests have arrived.
    let mut probes = Vec::with_capacity(CLUSTER_ATTEMPT_LANE_COUNT);
    for (index, fixture) in fixtures.iter_mut().enumerate() {
        let probe = fixture.next_request().await;
        assert_eq!(probe.path, format!("/cluster-attempt-{index}"));
        probes.push((index, probe));
    }
    let last_overlap_arrival = probes
        .iter()
        .map(|(_, probe)| probe.received_at.duration_since(overlap_started))
        .max()
        .unwrap();

    for (index, probe) in probes {
        probe
            .response
            .send(FixtureResponse::html(format!(
                "<html><body>NOMIFUN_LANE_SENTINEL_{index:02}_END</body></html>"
            )))
            .unwrap();
    }
    for navigation in navigations {
        navigation.await.unwrap().unwrap();
    }

    for (index, lane) in lanes.iter().enumerate() {
        let html = lane.rendered_html().await.unwrap();
        assert!(
            html.contains(&format!("NOMIFUN_LANE_SENTINEL_{index:02}_END")),
            "lane {index} must retain its own target content"
        );
        for other in 0..CLUSTER_ATTEMPT_LANE_COUNT {
            if other != index {
                assert!(
                    !html.contains(&format!("NOMIFUN_LANE_SENTINEL_{other:02}_END")),
                    "lane {index} must not render lane {other}'s target content"
                );
            }
        }
    }

    let mut system = sysinfo::System::new();
    let before_lane_close = sample_process_tree(&mut system, host_pid);
    assert!(before_lane_close.pids.contains(&host_pid));

    let close_started = Instant::now();
    for index in 0..CLUSTER_ATTEMPT_LANE_COUNT {
        host.close_lane(&format!("cluster-attempt-{index}"))
            .await
            .unwrap();
    }
    let close_elapsed = close_started.elapsed();
    for (index, lane) in lanes.iter().enumerate() {
        assert!(
            lane.rendered_html().await.is_err(),
            "closed lane {index} must reject subsequent operations"
        );
    }
    assert_eq!(
        host.process_id(),
        Some(host_pid),
        "closing all Lanes must not bypass explicit Host shutdown"
    );

    let before_host_shutdown = sample_process_tree(&mut system, host_pid);
    assert!(before_host_shutdown.pids.contains(&host_pid));
    let shutdown_started = Instant::now();
    host.shutdown().await.unwrap();
    let shutdown_elapsed = shutdown_started.elapsed();
    assert!(
        shutdown_elapsed < Duration::from_secs(5),
        "managed sixteen-Lane Host must shut down within five seconds; \
         elapsed={shutdown_elapsed:?}"
    );
    assert!(host.process_id().is_none());
    #[cfg(windows)]
    assert!(
        wait_for_debug_endpoint_closed(debug_port, Duration::from_secs(5)).await,
        "sixteen-Lane debugging endpoint {debug_port} remained reachable"
    );

    let managed_pids = before_lane_close
        .pids
        .iter()
        .chain(&before_host_shutdown.pids)
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let residual = wait_for_processes_gone(&managed_pids, Duration::from_secs(5)).await;
    assert!(
        residual.is_empty(),
        "sixteen-Lane managed Chromium process residue after shutdown: {residual:?}"
    );

    #[cfg(windows)]
    let debug_endpoint = format!("127.0.0.1:{debug_port} (closed)");
    #[cfg(not(windows))]
    let debug_endpoint = "remote-debugging-pipe (closed)".to_string();
    println!(
        "managed-host sixteen-lane acceptance: lanes={} host_pid={} \
         cross_lane_last_arrival_ms={} close_all_lanes_ms={} shutdown_ms={} \
         process_tree_before_close={} process_tree_before_shutdown={} \
         debug_endpoint={} sampled_pids={:?} residual_pids={:?}",
        CLUSTER_ATTEMPT_LANE_COUNT,
        host_pid,
        last_overlap_arrival.as_millis(),
        close_elapsed.as_millis(),
        shutdown_elapsed.as_millis(),
        before_lane_close.pids.len(),
        before_host_shutdown.pids.len(),
        debug_endpoint,
        managed_pids,
        residual,
    );

    for fixture in fixtures {
        fixture.stop();
    }
}

#[derive(Clone, Copy, Debug)]
enum RssHostLayout {
    Shared,
    Independent,
}

impl RssHostLayout {
    fn label(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Independent => "independent",
        }
    }

    fn host_count(self) -> usize {
        match self {
            Self::Shared => 1,
            Self::Independent => CROSS_LANE_COUNT,
        }
    }
}

#[derive(Debug)]
struct RssWorkloadResult {
    peak_rss_bytes: u64,
    peak_process_count: usize,
    samples: u64,
    root_pids: Vec<u32>,
    peak_pids: Vec<u32>,
    shutdown_elapsed: Duration,
    debug_endpoint_count: usize,
}

fn rss_fixture_html() -> String {
    // A fixed, external-network-free DOM workload.  Unique top-level URLs
    // avoid navigation cache hits, while every lane receives byte-identical
    // content and performs the same rendered-HTML readback.
    let row = "<article class=\"fixture-row\"><h2>NOMIFUN_RSS_FIXTURE</h2><p>0123456789abcdef0123456789abcdef0123456789abcdef</p></article>";
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>NomiFun RSS fixture</title></head><body>{}</body></html>",
        row.repeat(4_096)
    )
}

async fn run_rss_workload(
    fixture: &mut LocalFixture,
    temp_root: &std::path::Path,
    round: usize,
    layout: RssHostLayout,
) -> RssWorkloadResult {
    let mut hosts = Vec::new();
    let mut profiles = Vec::new();
    for host_index in 0..layout.host_count() {
        let profile_name = format!("rss-round-{round}-{}-{host_index}", layout.label());
        profiles.push(temp_root.join(&profile_name));
        hosts.push(Arc::new(
            ManagedBrowserHost::launch_platform_managed(managed_config(
                temp_root,
                &profile_name,
                false,
            ))
            .await
            .unwrap(),
        ));
    }
    let root_pids = hosts
        .iter()
        .map(|host| {
            host.process_id()
                .expect("RSS workload host must expose a root pid")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        root_pids.iter().copied().collect::<HashSet<_>>().len(),
        layout.host_count(),
        "every independent baseline host must have a distinct root process"
    );

    #[cfg(windows)]
    let debug_ports = {
        let ports = profiles
            .iter()
            .map(|profile| read_windows_debug_port(profile))
            .collect::<Vec<_>>();
        for port in &ports {
            tokio::net::TcpStream::connect(("127.0.0.1", *port))
                .await
                .expect("RSS workload debugging endpoint must be live before shutdown");
        }
        ports
    };
    #[cfg(not(windows))]
    let debug_ports: Vec<u16> = Vec::new();

    let mut lanes: Vec<Arc<dyn BrowserEngine>> = Vec::new();
    match layout {
        RssHostLayout::Shared => {
            for lane_index in 0..CROSS_LANE_COUNT {
                let lane_id = format!("rss-shared-{round}-{lane_index}");
                lanes.push(
                    hosts[0]
                        .open_lane(lane_id.clone(), platform_lane_config(&lane_id))
                        .await
                        .unwrap(),
                );
            }
        }
        RssHostLayout::Independent => {
            for (lane_index, host) in hosts.iter().enumerate() {
                let lane_id = format!("rss-independent-{round}-{lane_index}");
                lanes.push(
                    host.open_lane(lane_id.clone(), platform_lane_config(&lane_id))
                        .await
                        .unwrap(),
                );
            }
        }
    }

    // Do not count sequential launch transients.  Both variants first reach
    // the same ready state: all host roots alive and all four lanes open.
    tokio::time::sleep(Duration::from_millis(750)).await;
    let sampler = AggregateTelemetrySampler::start(root_pids.clone());
    let expected_paths = (0..CROSS_LANE_COUNT)
        .map(|lane_index| {
            format!(
                "/rss/{}/{round}/{lane_index}",
                layout.label()
            )
        })
        .collect::<HashSet<_>>();
    let mut navigations = Vec::new();
    for (lane_index, lane) in lanes.iter().enumerate() {
        let lane = Arc::clone(lane);
        let url = fixture.url(&format!(
            "/rss/{}/{round}/{lane_index}",
            layout.label()
        ));
        navigations.push(tokio::spawn(async move {
            lane.navigate(&url, false).await
        }));
    }
    let mut probes = Vec::new();
    let mut arrived_paths = HashSet::new();
    for _ in 0..CROSS_LANE_COUNT {
        let probe = fixture.next_request().await;
        arrived_paths.insert(probe.path.clone());
        probes.push(probe);
    }
    assert_eq!(
        arrived_paths, expected_paths,
        "all four RSS fixture navigations must overlap before responses release"
    );

    let body = rss_fixture_html();
    for probe in probes {
        probe
            .response
            .send(FixtureResponse::html(body.clone()))
            .unwrap();
    }
    for navigation in navigations {
        navigation.await.unwrap().unwrap();
    }
    for lane in &lanes {
        assert!(
            lane.rendered_html()
                .await
                .unwrap()
                .contains("NOMIFUN_RSS_FIXTURE")
        );
    }
    tokio::time::sleep(Duration::from_millis(750)).await;
    let telemetry = sampler.finish();
    assert!(telemetry.samples > 0);
    assert!(telemetry.peak_rss_bytes > 0);
    assert!(telemetry.peak_process_count > 0);

    let mut system = sysinfo::System::new();
    let before_shutdown = sample_process_forest(&mut system, &root_pids);
    for root_pid in &root_pids {
        assert!(before_shutdown.pids.contains(root_pid));
    }
    let shutdown_started = Instant::now();
    let shutdown_results =
        futures_util::future::join_all(hosts.iter().map(|host| host.shutdown())).await;
    for result in shutdown_results {
        result.unwrap();
    }
    let shutdown_elapsed = shutdown_started.elapsed();
    assert!(
        shutdown_elapsed < Duration::from_secs(5),
        "{} RSS workload hosts must shut down within five seconds; elapsed={shutdown_elapsed:?}",
        layout.label()
    );
    for host in &hosts {
        assert!(host.process_id().is_none());
    }
    #[cfg(windows)]
    for port in &debug_ports {
        assert!(
            wait_for_debug_endpoint_closed(*port, Duration::from_secs(5)).await,
            "{} RSS workload debugging endpoint {port} remained reachable",
            layout.label()
        );
    }

    let sampled_pids = before_shutdown
        .pids
        .iter()
        .chain(&telemetry.peak_pids)
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let residual = wait_for_processes_gone(&sampled_pids, Duration::from_secs(5)).await;
    assert!(
        residual.is_empty(),
        "{} RSS workload left managed Chromium residue: {residual:?}",
        layout.label()
    );

    RssWorkloadResult {
        peak_rss_bytes: telemetry.peak_rss_bytes,
        peak_process_count: telemetry.peak_process_count,
        samples: telemetry.samples,
        root_pids,
        peak_pids: telemetry.peak_pids,
        shutdown_elapsed,
        debug_endpoint_count: debug_ports.len(),
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Real-Chromium RSS baseline for AC-RES-005.
///
/// The paired experiment runs three rounds and alternates variant order to
/// reduce warm-cache/order bias.  Each variant starts sampling only after all
/// roots and exactly four lanes are ready, then executes byte-identical
/// loopback navigations and rendered-HTML readback.  RSS is the peak sum of the
/// union of all managed process trees from a single 50 ms process-table
/// refresh; it never sums per-host peaks captured at different times.
///
/// The material-reduction threshold below is intentionally calibrated from the
/// fixed fixture's repeated measurements rather than inferred from a single
/// run.  On the designated Windows/Chrome 150 acceptance machine the initial
/// three alternating-order ratios were 0.4081, 0.4090 and 0.4091 (0.10
/// percentage-point spread).  Requiring every round to remain at or below 0.50
/// means at least 50% lower peak RSS while retaining roughly 9.1 percentage
/// points of headroom over the worst observation.  If Chrome, the fixture, or
/// the designated hardware changes, rerun the three paired rounds and record
/// the new ratios before changing this threshold.
#[tokio::test]
#[ignore = "requires configured/bundled Chromium; RSS acceptance must run alone with --test-threads=1"]
async fn shared_host_rss_is_materially_below_four_independent_hosts() {
    const PAIRED_ROUNDS: usize = 3;
    const MAX_SHARED_TO_INDEPENDENT_RATIO: f64 = 0.50;

    let mut fixture = LocalFixture::start().await;
    let temp = tempfile::tempdir().unwrap();
    let mut shared_results = Vec::new();
    let mut independent_results = Vec::new();

    for round in 0..PAIRED_ROUNDS {
        let (shared, independent) = if round % 2 == 0 {
            let shared =
                run_rss_workload(&mut fixture, temp.path(), round, RssHostLayout::Shared).await;
            let independent = run_rss_workload(
                &mut fixture,
                temp.path(),
                round,
                RssHostLayout::Independent,
            )
            .await;
            (shared, independent)
        } else {
            let independent = run_rss_workload(
                &mut fixture,
                temp.path(),
                round,
                RssHostLayout::Independent,
            )
            .await;
            let shared =
                run_rss_workload(&mut fixture, temp.path(), round, RssHostLayout::Shared).await;
            (shared, independent)
        };
        let ratio = shared.peak_rss_bytes as f64 / independent.peak_rss_bytes as f64;
        println!(
            "rss paired round {}: order={} shared_peak_bytes={} independent_peak_bytes={} \
             shared_to_independent_ratio={:.4} reduction_percent={:.2} \
             shared_peak_processes={} independent_peak_processes={} \
             shared_samples={} independent_samples={} shared_roots={:?} independent_roots={:?} \
             shared_peak_pids={:?} independent_peak_pids={:?} \
             shared_shutdown_ms={} independent_shutdown_ms={} \
             shared_debug_endpoints={} independent_debug_endpoints={} residual_pids=[]",
            round + 1,
            if round % 2 == 0 {
                "shared-first"
            } else {
                "independent-first"
            },
            shared.peak_rss_bytes,
            independent.peak_rss_bytes,
            ratio,
            (1.0 - ratio) * 100.0,
            shared.peak_process_count,
            independent.peak_process_count,
            shared.samples,
            independent.samples,
            shared.root_pids,
            independent.root_pids,
            shared.peak_pids,
            independent.peak_pids,
            shared.shutdown_elapsed.as_millis(),
            independent.shutdown_elapsed.as_millis(),
            shared.debug_endpoint_count,
            independent.debug_endpoint_count,
        );
        shared_results.push(shared);
        independent_results.push(independent);
    }

    let ratios = shared_results
        .iter()
        .zip(&independent_results)
        .map(|(shared, independent)| {
            shared.peak_rss_bytes as f64 / independent.peak_rss_bytes as f64
        })
        .collect::<Vec<_>>();
    let median_ratio = median(ratios.clone());
    let worst_ratio = ratios.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let best_ratio = ratios.iter().copied().fold(f64::INFINITY, f64::min);
    println!(
        "rss paired summary: rounds={} ratios={:?} median_ratio={:.4} \
         median_reduction_percent={:.2} best_ratio={:.4} worst_ratio={:.4} \
         ratio_spread_percentage_points={:.2} threshold_ratio={:.2}",
        PAIRED_ROUNDS,
        ratios,
        median_ratio,
        (1.0 - median_ratio) * 100.0,
        best_ratio,
        worst_ratio,
        (worst_ratio - best_ratio) * 100.0,
        MAX_SHARED_TO_INDEPENDENT_RATIO,
    );

    assert!(
        ratios
            .iter()
            .all(|ratio| *ratio <= MAX_SHARED_TO_INDEPENDENT_RATIO),
        "every paired shared/independent RSS ratio must remain at or below the calibrated \
         {MAX_SHARED_TO_INDEPENDENT_RATIO:.2} threshold; median={median_ratio:.4}, \
         paired ratios={ratios:?}"
    );
    fixture.stop();
}
