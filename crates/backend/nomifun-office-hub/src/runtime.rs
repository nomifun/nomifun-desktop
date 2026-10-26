//! The host-owned, lazy supplier and supervised launcher for the user-installed CLI.
//!
//! Discovery never runs an installer, changes PATH, or reads credential files.
//! Authentication stays in the official CLI; this module never logs argv/stdin.
use crate::{HubError, types::*};
use async_trait::async_trait;
use fs2::FileExt;
use nomi_process_runtime::ChildProcessBuilder;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

pub const PAYLOAD_FILE: &str = "nomifun-office-payload.json";
const STDOUT_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 512 * 1024;

/// Deliberately has no Debug implementation: auth argv and stdin may be secrets.
#[derive(Clone)]
pub struct CliCommand {
    pub args: Vec<String>,
    pub stdin: Option<Vec<u8>>,
    /// A private, per-launch JSON file, referenced only by the fixed PAYLOAD_FILE.
    pub input_file: Option<Vec<u8>>,
    pub timeout: Duration,
    pub mutating: bool,
}
impl CliCommand {
    pub fn new(args: impl IntoIterator<Item = impl Into<String>>, mutating: bool) -> Self {
        Self { args: args.into_iter().map(Into::into).collect(), stdin: None, input_file: None,
            timeout: Duration::from_secs(60), mutating }
    }
}

/// Raw output is internal and intentionally neither Serialize nor Debug.
#[derive(Clone, Default)]
pub struct CliOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub started: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub truncated: bool,
    pub cleanup_error: Option<String>,
}
impl CliOutput {
    pub fn fixture(code: i32, stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Self { exit_code: Some(code), stdout: stdout.into(), stderr: stderr.into(), started: true,
            ..Self::default() }
    }
    pub fn successful(&self) -> bool {
        self.started && self.exit_code == Some(0) && !self.timed_out && !self.cancelled
            && !self.truncated && self.cleanup_error.is_none()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliStream { Stdout, Stderr }
/// Callbacks receive transient chunks. Consumers must project only approved data
/// (e.g. an official verification URL), never persist raw auth output.
pub type CliObserver = Arc<dyn Fn(CliStream, &str) + Send + Sync>;

#[async_trait]
pub trait CliExecutor: Send + Sync {
    async fn execute(&self, executable: &Path, command: CliCommand,
        cancel: CancellationToken, observer: Option<CliObserver>) -> Result<CliOutput, HubError>;
}

pub struct SupervisedCliExecutor { work_root: PathBuf }
impl SupervisedCliExecutor {
    pub fn new(data_dir: PathBuf) -> Self { Self { work_root: data_dir.join("office-cli-inputs") } }
}

struct PrivateLaunchDirectory(PathBuf);
impl PrivateLaunchDirectory {
    async fn create(root: &Path) -> Result<Self, HubError> {
        tokio::fs::create_dir_all(root).await.map_err(|_|HubError::unavailable("无法创建私有办公输入目录；尚未启动业务请求"))?;
        let root = tokio::fs::canonicalize(root).await.map_err(|_|HubError::unavailable("无法确认私有办公输入目录；尚未启动业务请求"))?;
        let path = root.join(uuid::Uuid::now_v7().to_string());
        // A fresh UUID child of this canonical root is the only cleanup target.
        #[cfg(unix)]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(not(unix))]
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
        builder.create(&path).map_err(|_|HubError::unavailable("无法创建本次私有目录；尚未启动业务请求"))?;
        Ok(Self(path))
    }
}
impl Drop for PrivateLaunchDirectory {
    fn drop(&mut self) {
        // Never traverse a caller-supplied location; this is an exact new child.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Captured { bytes: Vec<u8>, incomplete: bool }
async fn capture_stream<R: AsyncRead + Unpin>(
    mut reader: R, stream: CliStream, limit: usize, observer: Option<CliObserver>,
) -> Captured {
    let mut result = Captured { bytes: Vec::new(), incomplete: false };
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                let remaining = limit.saturating_sub(result.bytes.len());
                let accepted = count.min(remaining);
                if accepted != count { result.incomplete = true; }
                if accepted > 0 {
                    result.bytes.extend_from_slice(&buffer[..accepted]);
                    if let Some(callback) = &observer {
                        callback(stream, &String::from_utf8_lossy(&buffer[..accepted]));
                    }
                }
                // Continue draining the pipe after the cap, without retaining or
                // forwarding more bytes, so a child cannot deadlock the owner.
            }
            Err(_) => { result.incomplete = true; break; }
        }
    }
    result
}

#[async_trait]
impl CliExecutor for SupervisedCliExecutor {
    async fn execute(&self, executable: &Path, command: CliCommand,
        cancel: CancellationToken, observer: Option<CliObserver>) -> Result<CliOutput, HubError>
    {
        if cancel.is_cancelled() { return Err(HubError::unavailable("操作已取消，尚未启动飞书 CLI")); }
        if !executable.is_absolute() && executable.as_os_str() != std::ffi::OsStr::new("npm") {
            return Err(HubError::invalid("办公组件必须使用已有程序的绝对路径"));
        }
        let directory = PrivateLaunchDirectory::create(&self.work_root).await?;
        if let Some(input) = &command.input_file {
            if input.len() > STDOUT_LIMIT {
                return Err(HubError::invalid("办公请求体超过允许大小"));
            }
            let input_path = directory.0.join(PAYLOAD_FILE);
            let mut options = tokio::fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            { options.mode(0o600); }
            let mut file = options.open(&input_path).await.map_err(|_|HubError::unavailable("无法准备请求体文件；尚未启动业务请求"))?;
            file.write_all(input).await.map_err(|_|HubError::unavailable("无法写入请求体文件；尚未启动业务请求"))?;
            file.flush().await.map_err(|_|HubError::unavailable("无法完成请求体文件；尚未启动业务请求"))?;
        }
        let configured = HashMap::from([
            ("LARKSUITE_CLI_NO_UPDATE_NOTIFIER".into(), "1".into()),
            ("LARKSUITE_CLI_NO_SKILLS_NOTIFIER".into(), "1".into()),
            ("NO_COLOR".into(), "1".into()),
            ("TERM".into(), "dumb".into()),
        ]);
        let prepared = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(HubError::unavailable("操作已取消，尚未启动飞书 CLI")),
            prepared = async {
                if executable.is_absolute() {
                    // Discovery and business calls only execute an existing
                    // selected file, even inside the shared npm prefix. Its
                    // Go binary does not acquire a Node dependency by location.
                    nomifun_runtime::prepare_existing_process_launch(executable, &command.args, &directory.0, &configured).await
                } else {
                    // This sole relative command is private to explicit
                    // prepare(): it may lazily supply missing Node/npm.
                    nomifun_runtime::prepare_process_launch("npm", &command.args, &directory.0, &configured).await
                }
            } =>
                prepared.map_err(|_| HubError::unavailable("无法准备飞书 CLI 的运行环境；尚未发起业务请求"))?,
        };
        let mut builder = ChildProcessBuilder::clean_cli(&prepared.program);
        builder.args(&prepared.args).env_clear().envs(&prepared.environment)
            .current_dir(&directory.0)
            .stdin(if command.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut process = builder.spawn_managed().map_err(|error| {
            if matches!(error.kind(),std::io::ErrorKind::NotFound|std::io::ErrorKind::InvalidInput){
                HubError::unavailable("飞书 CLI 未能启动；尚未发起业务请求")
            }else{
                HubError::unknown("飞书 CLI 启动未取得完整证明，无法断言业务请求未执行")
            }
        })?;
        let stdout = process.stdout.take().ok_or_else(|| HubError::unknown("无法观察飞书 CLI 输出"))?;
        let stderr = process.stderr.take().ok_or_else(|| HubError::unknown("无法观察飞书 CLI 错误输出"))?;
        let out_task = tokio::spawn(capture_stream(stdout, CliStream::Stdout, STDOUT_LIMIT, observer.clone()));
        let err_task = tokio::spawn(capture_stream(stderr, CliStream::Stderr, STDERR_LIMIT, observer));
        let input_task = command.stdin.map(|input| {
            let stdin = process.stdin.take();
            tokio::spawn(async move {
                let Some(mut stdin) = stdin else { return false };
                stdin.write_all(&input).await.is_ok() && stdin.shutdown().await.is_ok()
            })
        });
        let mut output = CliOutput { started: true, ..CliOutput::default() };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => { output.cancelled = true; }
            _ = tokio::time::sleep(command.timeout) => { output.timed_out = true; }
            result = process.wait() => match result {
                Ok(status) => output.exit_code = status.code(),
                Err(_) => output.cleanup_error = Some("无法取得原进程退出状态".into()),
            },
        }
        // Both normal exit and cancellation must settle this exact registered
        // tree before the executing/auth worker can report completion. Failed
        // shutdown attempts retain the same child and cleanup authority. Keep
        // retrying with bounded backoff; cancellation stops execution, never
        // this cleanup. If proof cannot finish, this owner stays outstanding
        // so application shutdown can report the unsettled work.
        let mut cleanup_backoff = Duration::from_millis(50);
        while process.shutdown().await.is_err() {
            tokio::time::sleep(cleanup_backoff).await;
            cleanup_backoff = (cleanup_backoff * 2).min(Duration::from_secs(2));
        }
        if output.exit_code.is_none() {
            output.exit_code = process.try_wait().ok().flatten().and_then(|status| status.code());
        }
        let captured_out = out_task.await
            .map_err(|_| HubError::unknown("飞书 CLI 输出观察未完成"))?;
        let captured_err = err_task.await
            .map_err(|_| HubError::unknown("飞书 CLI 错误输出观察未完成"))?;
        output.stdout = String::from_utf8_lossy(&captured_out.bytes).into_owned();
        output.stderr = String::from_utf8_lossy(&captured_err.bytes).into_owned();
        output.truncated = captured_out.incomplete || captured_err.incomplete;
        if let Some(writer) = input_task {
            if !writer.await.unwrap_or(false) { output.truncated = true; }
        }
        Ok(output)
    }
}

#[derive(Clone, PartialEq, Eq)]
struct FileStamp { length: u64, modified: Option<SystemTime> }
fn file_stamp(path: &Path) -> Result<FileStamp, HubError> {
    let metadata = std::fs::metadata(path).map_err(|_| HubError::unavailable("选定组件无法读取"))?;
    if !metadata.is_file() { return Err(HubError::invalid("组件目标必须是文件")); }
    Ok(FileStamp { length: metadata.len(), modified: metadata.modified().ok() })
}
struct Verified { stamp: FileStamp, version: String }

pub struct CliRuntime {
    data_dir: PathBuf,
    shared_root: Option<PathBuf>,
    executor: Arc<dyn CliExecutor>,
    verified: Mutex<HashMap<PathBuf, Verified>>,
}
impl CliRuntime {
    pub fn new(data_dir: PathBuf) -> Self {
        let executor = Arc::new(SupervisedCliExecutor::new(data_dir.clone()));
        Self::with_executor(data_dir, executor)
    }
    pub fn with_executor(data_dir: PathBuf, executor: Arc<dyn CliExecutor>) -> Self {
        Self { data_dir, shared_root: nomifun_runtime::user_cli_root(), executor,
            verified: Mutex::new(HashMap::new()) }
    }
    /// Trusted host/test construction; an HTTP caller cannot choose this root.
    pub fn with_executor_and_root(data_dir: PathBuf, shared_root: PathBuf,
        executor: Arc<dyn CliExecutor>) -> Self
    {
        Self { data_dir, shared_root: Some(shared_root), executor,
            verified: Mutex::new(HashMap::new()) }
    }
    pub fn shared_root(&self) -> Option<&Path> { self.shared_root.as_deref() }

    fn shared_candidate(&self) -> Option<PathBuf> {
        let root = self.shared_root.as_ref()?;
        let native = npm_native(root);
        if native.is_file() { return Some(native); }
        let launcher = npm_launcher(root);
        if launcher.exists() { return Some(launcher); }
        None
    }
    fn source(&self, path: &Path, preferred: RuntimeSource) -> RuntimeSource {
        if self.shared_root.as_ref().and_then(|p| p.canonicalize().ok())
            .is_some_and(|root| path.starts_with(root)) { RuntimeSource::SharedUser }
        else { preferred }
    }
    async fn candidates(&self, selected: Option<&str>, npm_prefix: bool) -> Vec<(PathBuf, RuntimeSource)> {
        if let Some(selected) = selected {
            return vec![(PathBuf::from(selected), RuntimeSource::Selected)];
        }
        if let Some(path) = self.shared_candidate() { return vec![(path, RuntimeSource::SharedUser)]; }
        if let Some(path) = nomifun_runtime::resolve_command_path("lark-cli") {
            return vec![(path, RuntimeSource::External)];
        }
        if npm_prefix {
            // Only inspect an existing npm runtime here. Missing Node/npm is
            // prepared exclusively inside the explicit prepare() operation.
            if let (Some(npm), Some(_)) = (nomifun_runtime::resolve_command_path("npm"),
                nomifun_runtime::resolve_command_path("node"))
            {
                let command = CliCommand { timeout: Duration::from_secs(12),
                    ..CliCommand::new(["prefix", "--global"], false) };
                if let Ok(output) = self.executor.execute(&npm, command, CancellationToken::new(), None).await {
                    if output.successful() {
                        let prefix = PathBuf::from(output.stdout.trim());
                        if prefix.is_absolute() {
                            let path = npm_native(&prefix);
                            if path.is_file() { return vec![(path, RuntimeSource::External)]; }
                            let path = npm_launcher(&prefix);
                            if path.is_file() { return vec![(path, RuntimeSource::External)]; }
                        }
                    }
                }
            }
        }
        Vec::new()
    }

    async fn probe(&self, candidate: &Path, source: RuntimeSource) -> RuntimeInfo {
        let fail = |phase, message: String, path: Option<PathBuf>, version: Option<String>| RuntimeInfo {
            phase, executable: path.map(|p| p.to_string_lossy().into_owned()), version,
            source: Some(source.clone()), message: Some(message),
            official_install_command: OFFICIAL_INSTALL_COMMAND.into(),
        };
        let path = match normalize_executable(candidate) {
            Ok(path) => path,
            Err(error) => return fail(RuntimePhase::Broken, error.message, Some(candidate.to_owned()), None),
        };
        let stamp = match file_stamp(&path) {
            Ok(stamp) => stamp,
            Err(error) => return fail(RuntimePhase::Broken, error.message, Some(path), None),
        };
        if let Some(verified) = self.verified.lock().await.get(&path).filter(|v| v.stamp == stamp) {
            return RuntimeInfo { phase: RuntimePhase::Ready, executable: Some(path.to_string_lossy().into_owned()),
                version: Some(verified.version.clone()), source: Some(self.source(&path, source)),
                message: None, official_install_command: OFFICIAL_INSTALL_COMMAND.into() };
        }
        let command = CliCommand { timeout: Duration::from_secs(15), ..CliCommand::new(["--version"], false) };
        let output = match self.executor.execute(&path, command, CancellationToken::new(), None).await {
            Ok(output) if output.successful() => output,
            _ => return fail(RuntimePhase::Broken, "已有组件无法正常启动；请修复该安装，不会另装一份".into(), Some(path), None),
        };
        let Some(version) = parse_cli_version(&output.stdout) else {
            return fail(RuntimePhase::Incompatible, "无法确认这是受支持的官方飞书 CLI".into(), Some(path), None);
        };
        if version != FEISHU_CLI_VERSION {
            return fail(RuntimePhase::Incompatible,
                format!("已安装版本 {version}；当前已验证的版本为 {FEISHU_CLI_VERSION}，不会静默替换现有安装"),
                Some(path), Some(version));
        }
        for args in [vec!["--help"], vec!["schema", "task.tasks.create"]] {
            let command = CliCommand { timeout: Duration::from_secs(15), ..CliCommand::new(args.clone(), false) };
            let valid = match self.executor.execute(&path, command, CancellationToken::new(), None).await {
                Ok(output) if output.successful() => {
                    args.len() == 1 || serde_json::from_str::<serde_json::Value>(&output.stdout)
                        .ok().is_some_and(|value| value.get("inputSchema").is_some() && value.get("_meta").is_some())
                }
                _ => false,
            };
            if !valid {
                return fail(RuntimePhase::Incompatible, "组件缺少办公工作台需要的命令契约".into(), Some(path), Some(version));
            }
        }
        self.verified.lock().await.insert(path.clone(), Verified { stamp, version: version.clone() });
        RuntimeInfo { phase: RuntimePhase::Ready, executable: Some(path.to_string_lossy().into_owned()),
            version: Some(version), source: Some(self.source(&path, source)), message: None,
            official_install_command: OFFICIAL_INSTALL_COMMAND.into() }
    }

    pub async fn inspect(&self, selected: Option<&str>) -> RuntimeInfo {
        let candidates = self.candidates(selected, true).await;
        if let Some((path, source)) = candidates.into_iter().next() { self.probe(&path, source).await }
        else { RuntimeInfo::default() }
    }
    pub async fn select(&self, executable: &str) -> Result<RuntimeInfo, HubError> {
        if !Path::new(executable).is_absolute() { return Err(HubError::invalid("请选择组件的绝对路径")); }
        let info = self.probe(Path::new(executable), RuntimeSource::Selected).await;
        if info.phase == RuntimePhase::Ready { Ok(info) }
        else { Err(HubError::unavailable(info.message.unwrap_or_else(|| "组件不可用".into()))) }
    }

    pub async fn prepare(&self, selected: Option<&str>, cancel: CancellationToken) -> Result<RuntimeInfo, HubError> {
        let existing = self.inspect(selected).await;
        if existing.phase == RuntimePhase::Ready { return Ok(existing); }
        if existing.phase != RuntimePhase::Missing {
            return Err(HubError::unavailable(existing.message.unwrap_or_else(|| "已有组件需修复；不会重复安装".into())));
        }
        let root = self.shared_root.as_ref().ok_or_else(|| HubError::unavailable("无法确定用户共享 CLI 安装目录"))?;
        tokio::fs::create_dir_all(root).await.map_err(HubError::storage)?;
        let lock_path = root.join(".nomifun-lark-cli.install.lock");
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .open(lock_path).map_err(HubError::storage)?;
        let _guard = acquire_install_lock(file, &cancel).await?;
        let existing = self.inspect(selected).await;
        if existing.phase == RuntimePhase::Ready { return Ok(existing); }
        if existing.phase != RuntimePhase::Missing { return Err(HubError::unavailable("并行准备后发现已有组件需要修复")); }

        // Resolving npm here is the first point at which missing Node/npm may
        // be prepared. Keep the user's original npm prefix/config intact.
        let query = CliCommand { timeout: Duration::from_secs(180),
            ..CliCommand::new(["prefix", "--global"], false) };
        let prefix_output = self.executor.execute(Path::new("npm"), query, cancel.clone(), None).await?;
        if !prefix_output.successful() { return Err(HubError::unavailable("无法准备官方 npm 安装方式；尚未安装飞书 CLI")); }
        let prefix = PathBuf::from(prefix_output.stdout.trim());
        if prefix.is_absolute() {
            let candidate = if npm_native(&prefix).is_file() { Some(npm_native(&prefix)) }
                else { Some(npm_launcher(&prefix)).filter(|p| p.is_file()) };
            if let Some(path) = candidate {
                let info = self.probe(&path, RuntimeSource::External).await;
                if info.phase == RuntimePhase::Ready { return Ok(info); }
                return Err(HubError::unavailable(info.message.unwrap_or_else(|| "已有全局安装需要修复".into())));
            }
        }
        let args = vec!["install".into(), "--global".into(), "--prefix".into(),
            root.to_string_lossy().into_owned(), format!("@larksuite/cli@{FEISHU_CLI_VERSION}")];
        let command = CliCommand { timeout: Duration::from_secs(300), ..CliCommand::new(args, false) };
        let installed = self.executor.execute(Path::new("npm"), command, cancel, None).await?;
        if !installed.successful() {
            return Err(HubError::unavailable("飞书 CLI 安装未完成；保留当前目录用于诊断，不改用其他安装位置"));
        }
        let candidate = self.shared_candidate().ok_or_else(|| HubError::unavailable("npm 已返回，但未找到飞书 CLI 启动器"))?;
        let info = self.probe(&candidate, RuntimeSource::SharedUser).await;
        if info.phase == RuntimePhase::Ready { Ok(info) }
        else { Err(HubError::unavailable(info.message.unwrap_or_else(|| "安装后的组件未通过验证".into()))) }
    }

    pub async fn run(&self, executable: &str, command: CliCommand,
        cancel: CancellationToken, observer: Option<CliObserver>) -> Result<CliOutput, HubError>
    {
        let info = self.select(executable).await?;
        let path = info.executable.ok_or_else(|| HubError::unavailable("未选择飞书 CLI"))?;
        self.executor.execute(Path::new(&path), command, cancel, observer).await
    }
    pub fn data_dir(&self) -> &Path { &self.data_dir }
}

struct InstallLock(File);
impl Drop for InstallLock { fn drop(&mut self) { let _ = FileExt::unlock(&self.0); } }
async fn acquire_install_lock(file: File, cancel: &CancellationToken) -> Result<InstallLock, HubError> {
    loop {
        if cancel.is_cancelled() { return Err(HubError::unavailable("已取消等待共享组件安装")); }
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(InstallLock(file)),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
                || matches!(error.raw_os_error(),Some(code) if Some(code)==fs2::lock_contended_error().raw_os_error()) => {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(HubError::unavailable("已取消等待共享组件安装")),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                }
            }
            Err(_) => return Err(HubError::unavailable("无法锁定用户共享 CLI 安装目录")),
        }
    }
}
fn npm_native(prefix: &Path) -> PathBuf {
    #[cfg(windows)]
    let package = prefix.join("node_modules").join("@larksuite").join("cli");
    #[cfg(not(windows))]
    let package = prefix.join("lib").join("node_modules").join("@larksuite").join("cli");
    package.join("bin").join(if cfg!(windows) { "lark-cli.exe" } else { "lark-cli" })
}
fn npm_launcher(prefix: &Path) -> PathBuf {
    if cfg!(windows) { prefix.join("lark-cli.cmd") } else { prefix.join("bin").join("lark-cli") }
}
fn normalize_executable(candidate: &Path) -> Result<PathBuf, HubError> {
    if !candidate.is_absolute() { return Err(HubError::invalid("飞书组件路径必须是绝对路径")); }
    let path = candidate.canonicalize().map_err(|_| HubError::unavailable("选定组件不存在或无法读取"))?;
    let mut packages = Vec::new();
    if let Some(parent) = candidate.parent() {
        packages.push(parent.join("node_modules").join("@larksuite").join("cli"));
        if let Some(prefix) = parent.parent() {
            packages.push(prefix.join("lib").join("node_modules").join("@larksuite").join("cli"));
        }
    }
    if path.file_name().is_some_and(|s| s == "run.js") {
        if let Some(package) = path.parent().and_then(Path::parent) { packages.push(package.to_owned()); }
    }
    for package in packages {
        let manifest = package.join("package.json");
        let official = std::fs::metadata(&manifest).ok().is_some_and(|m| m.len() <= 128 * 1024)
            && std::fs::read(&manifest).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .is_some_and(|v| v.get("name").and_then(|v| v.as_str()) == Some("@larksuite/cli"));
        if official {
            let native = package.join("bin").join(if cfg!(windows) { "lark-cli.exe" } else { "lark-cli" });
            return native.canonicalize().map_err(|_| HubError::unavailable("已有 npm 安装缺少原生程序；请原地修复，诊断不会自动下载"));
        }
    }
    Ok(path)
}
fn parse_cli_version(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|word| {
        let candidate = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-')
            .trim_start_matches('v');
        semver::Version::parse(candidate).ok().map(|v| v.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ProbeFixture { calls: AtomicUsize }
    #[async_trait]
    impl CliExecutor for ProbeFixture {
        async fn execute(&self, _executable: &Path, command: CliCommand,
            _cancel: CancellationToken, _observer: Option<CliObserver>) -> Result<CliOutput, HubError>
        {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert!(!command.mutating);
            match command.args.first().map(String::as_str) {
                Some("--version") => Ok(CliOutput::fixture(0, "lark-cli 1.0.98", "")),
                Some("--help") => Ok(CliOutput::fixture(0, "auth docs task base", "")),
                Some("schema") => Ok(CliOutput::fixture(0, r#"{"inputSchema":{},"_meta":{}}"#, "")),
                _ => panic!("unexpected installer during discovery"),
            }
        }
    }
    #[tokio::test]
    async fn selected_discovery_is_cached_and_never_installs() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("fixture-lark.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        let fixture = Arc::new(ProbeFixture { calls: AtomicUsize::new(0) });
        let runtime = CliRuntime::with_executor_and_root(temp.path().into(),
            temp.path().join("user-tools"), fixture.clone());
        let path = executable.to_str().unwrap();
        assert_eq!(runtime.inspect(Some(path)).await.phase, RuntimePhase::Ready);
        assert_eq!(fixture.calls.load(Ordering::Relaxed), 3);
        assert_eq!(runtime.inspect(Some(path)).await.phase, RuntimePhase::Ready);
        assert_eq!(fixture.calls.load(Ordering::Relaxed), 3);
        assert!(!temp.path().join("user-tools").exists());
    }
    #[tokio::test]
    async fn supervised_executor_rejects_unowned_relative_programs_before_creating_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let executor = SupervisedCliExecutor::new(temp.path().into());
        for executable in ["node", "npx", "lark-cli", "./npm", "npm/"] {
            let result = executor.execute(Path::new(executable), CliCommand::new(["--version"], false),
                CancellationToken::new(), None).await;
            let error = result.err().expect("relative tools must not enter managed preparation");
            assert!(error.not_executed);
        }
        assert!(!temp.path().join("office-cli-inputs").exists());
    }
    #[tokio::test]
    async fn cancelled_prepare_lock_does_not_start_an_install() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lock");
        let file = OpenOptions::new().create(true).read(true).write(true).truncate(false).open(&path).unwrap();
        file.lock_exclusive().unwrap();
        let waiter = OpenOptions::new().read(true).write(true).open(path).unwrap();
        let cancel = CancellationToken::new(); cancel.cancel();
        assert!(acquire_install_lock(waiter, &cancel).await.is_err());
        FileExt::unlock(&file).unwrap();
    }

    #[derive(Clone, Copy)]
    enum PrepareProbe { Ready, Broken, OldVersion, MissingContract }
    struct PrepareFixture {
        root: PathBuf,
        prefix: PathBuf,
        npm: PathBuf,
        probe: PrepareProbe,
        install_fails: bool,
        hold_install: bool,
        calls: std::sync::Mutex<Vec<(PathBuf, Vec<String>)>>,
        installs: AtomicUsize,
        prefix_entered: tokio::sync::Semaphore,
        install_entered: tokio::sync::Semaphore,
        install_release: tokio::sync::Semaphore,
    }
    impl PrepareFixture {
        fn new(root:PathBuf,prefix:PathBuf,npm:PathBuf,probe:PrepareProbe)->Self {
            Self { root, prefix, npm, probe, install_fails:false, hold_install:false,
                calls:std::sync::Mutex::new(Vec::new()),installs:AtomicUsize::new(0),
                prefix_entered:tokio::sync::Semaphore::new(0),install_entered:tokio::sync::Semaphore::new(0),
                install_release:tokio::sync::Semaphore::new(0) }
        }
    }
    fn publish_prepare_package(root:&Path)->PathBuf {
        let native=npm_native(root);
        let package=native.parent().unwrap().parent().unwrap();
        std::fs::create_dir_all(native.parent().unwrap()).unwrap();
        std::fs::write(package.join("package.json"),r#"{"name":"@larksuite/cli"}"#).unwrap();
        std::fs::write(&native,b"offline native fixture; executor never launches this file").unwrap();
        native
    }
    #[async_trait]
    impl CliExecutor for PrepareFixture {
        async fn execute(&self, executable:&Path, command:CliCommand,
            _cancel:CancellationToken, _observer:Option<CliObserver>)->Result<CliOutput,HubError> {
            self.calls.lock().unwrap().push((executable.into(),command.args.clone()));
            assert!(!command.mutating);
            assert!(command.stdin.is_none()&&command.input_file.is_none());
            if command.args==["prefix","--global"] {
                assert!(executable==Path::new("npm")||executable==self.npm);
                self.prefix_entered.add_permits(1);
                return Ok(CliOutput::fixture(0,self.prefix.to_string_lossy(),""));
            }
            if command.args.first().map(String::as_str)==Some("install") {
                assert_eq!(executable,Path::new("npm"));
                assert_eq!(command.args,vec!["install".to_owned(),"--global".into(),"--prefix".into(),
                    self.root.to_string_lossy().into_owned(),format!("@larksuite/cli@{FEISHU_CLI_VERSION}")]);
                self.installs.fetch_add(1,Ordering::SeqCst);
                self.install_entered.add_permits(1);
                if self.hold_install { self.install_release.acquire().await.unwrap().forget(); }
                publish_prepare_package(&self.root);
                if self.install_fails {
                    std::fs::write(self.root.join("partial-install-diagnostic"),b"keep this failed install").unwrap();
                    return Ok(CliOutput::fixture(1,"","fixture install failed"));
                }
                return Ok(CliOutput::fixture(0,"offline fixture installed",""));
            }
            assert_eq!(executable,npm_native(&self.root).canonicalize().unwrap());
            match command.args.first().map(String::as_str) {
                Some("--version")=>Ok(match self.probe {
                    PrepareProbe::Broken=>CliOutput::fixture(1,"","fixture cannot start"),
                    PrepareProbe::OldVersion=>CliOutput::fixture(0,"lark-cli 1.0.97",""),
                    _=>CliOutput::fixture(0,format!("lark-cli {FEISHU_CLI_VERSION}"),""),
                }),
                Some("--help")=>Ok(CliOutput::fixture(0,"auth docs task base","")),
                Some("schema")=>Ok(CliOutput::fixture(0,
                    if matches!(self.probe,PrepareProbe::MissingContract){r#"{"unrecognized_contract":true}"#}
                    else{r#"{"inputSchema":{},"_meta":{}}"#},"")),
                _=>panic!("Unexpected offline prepare command"),
            }
        }
    }

    // Discovery reads process PATH. Each case therefore runs in its own test
    // child with a fixture-only PATH; no test mutates the multithreaded host's
    // environment or observes a developer's npm prefix/managed Node cache.
    fn run_prepare_case(case:&str) {
        let temp=tempfile::tempdir().unwrap();
        let tools=temp.path().join("path");std::fs::create_dir(&tools).unwrap();
        for name in ["npm","node"] {
            let program=tools.join(if cfg!(windows){format!("{name}.exe")}else{name.into()});
            std::fs::write(&program,b"offline discovery fixture; never execute").unwrap();
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(program,std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let output=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored","--exact","runtime::tests::process_prepare_fixture","--nocapture"])
            .env("PATH",&tools).env("NOMIFUN_OFFICE_PREPARE_CASE",case)
            .env("NOMIFUN_OFFICE_PREPARE_ROOT",temp.path()).output().unwrap();
        assert!(output.status.success(),"prepare child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    }
    #[test]
    fn concurrent_prepare_installs_once_and_the_waiter_reuses_the_verified_shared_install() {
        run_prepare_case("concurrent");
    }
    #[test]
    fn broken_or_incompatible_prepare_preserves_the_existing_install_without_a_fallback_copy() {
        run_prepare_case("existing-unusable");
    }
    #[test]
    fn failed_prepare_preserves_its_partial_install_and_does_not_reinstall_elsewhere() {
        run_prepare_case("install-failure");
    }
    #[test]
    #[ignore]
    fn process_prepare_fixture() {
        let root=PathBuf::from(std::env::var_os("NOMIFUN_OFFICE_PREPARE_ROOT").expect("test-owned root"));
        let case=std::env::var("NOMIFUN_OFFICE_PREPARE_CASE").expect("test-owned case");
        let tools=root.join("path");
        assert_eq!(std::env::var_os("PATH").unwrap(),tools.as_os_str());
        assert!(nomifun_runtime::resolve_command_path("lark-cli").is_none());
        let npm=nomifun_runtime::resolve_command_path("npm").expect("fixture npm dominates managed fallback");
        let node=nomifun_runtime::resolve_command_path("node").expect("fixture node dominates managed fallback");
        assert!(npm.starts_with(&tools)&&node.starts_with(&tools));
        let runtime=tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10),async {
                match case.as_str() {
                    "concurrent"=>{
                        let shared=root.join("shared-user-tools");let prefix=root.join("external-prefix");
                        let mut executor=PrepareFixture::new(shared.clone(),prefix.clone(),npm,PrepareProbe::Ready);
                        executor.hold_install=true;let executor=Arc::new(executor);
                        let first=Arc::new(CliRuntime::with_executor_and_root(root.join("first-data"),shared.clone(),executor.clone()));
                        let second=Arc::new(CliRuntime::with_executor_and_root(root.join("second-data"),shared.clone(),executor.clone()));
                        let first=tokio::spawn(async move{first.prepare(None,CancellationToken::new()).await});
                        executor.install_entered.acquire().await.unwrap().forget();
                        while let Ok(permit)=executor.prefix_entered.try_acquire(){permit.forget();}
                        let second=tokio::spawn(async move{second.prepare(None,CancellationToken::new()).await});
                        executor.prefix_entered.acquire().await.unwrap().forget();
                        assert_eq!(executor.installs.load(Ordering::SeqCst),1,"second prepare has entered while the original installer owns the lock");
                        assert!(!npm_native(&shared).exists(),"the installation is still incomplete");
                        executor.install_release.add_permits(1);
                        let first=first.await.unwrap().unwrap();let second=second.await.unwrap().unwrap();
                        assert_eq!(first.phase,RuntimePhase::Ready);assert_eq!(second.phase,RuntimePhase::Ready);
                        assert_eq!(first.executable,second.executable);
                        assert_eq!(first.source,Some(RuntimeSource::SharedUser));
                        assert_eq!(second.source,Some(RuntimeSource::SharedUser));
                        assert_eq!(executor.installs.load(Ordering::SeqCst),1);
                        assert_eq!(first.executable,Some(npm_native(&shared).canonicalize().unwrap().to_string_lossy().into_owned()));
                        assert!(!prefix.exists(),"npm's original prefix is never changed or installed into");
                    },
                    "existing-unusable"=>{
                        for (label,probe,phase) in [("broken",PrepareProbe::Broken,RuntimePhase::Broken),
                            ("version",PrepareProbe::OldVersion,RuntimePhase::Incompatible),
                            ("contract",PrepareProbe::MissingContract,RuntimePhase::Incompatible)] {
                            let shared=root.join(label);let native=publish_prepare_package(&shared);
                            let before=std::fs::read(&native).unwrap();
                            let prefix=root.join(format!("external-{label}"));
                            let executor=Arc::new(PrepareFixture::new(shared.clone(),prefix.clone(),npm.clone(),probe));
                            let runtime=CliRuntime::with_executor_and_root(root.join(format!("data-{label}")),shared.clone(),executor.clone());
                            assert_eq!(runtime.inspect(None).await.phase,phase);
                            assert!(runtime.prepare(None,CancellationToken::new()).await.is_err());
                            assert_eq!(executor.installs.load(Ordering::SeqCst),0);
                            assert!(executor.calls.lock().unwrap().iter().all(|(path,args)|
                                path==&native.canonicalize().unwrap()&&args.first().is_some_and(|arg|matches!(arg.as_str(),"--version"|"--help"|"schema"))));
                            assert_eq!(std::fs::read(&native).unwrap(),before);
                            assert!(!shared.join(".nomifun-lark-cli.install.lock").exists(),"known unusable installs never enter installation");
                            assert!(!prefix.exists());
                        }
                    },
                    "install-failure"=>{
                        let shared=root.join("failed-shared");let prefix=root.join("external-prefix");
                        let mut executor=PrepareFixture::new(shared.clone(),prefix.clone(),npm,PrepareProbe::Broken);
                        executor.install_fails=true;let executor=Arc::new(executor);
                        let runtime=CliRuntime::with_executor_and_root(root.join("data"),shared.clone(),executor.clone());
                        assert!(runtime.prepare(None,CancellationToken::new()).await.is_err());
                        let diagnostic=std::fs::read(shared.join("partial-install-diagnostic")).unwrap();
                        assert_eq!(runtime.inspect(None).await.phase,RuntimePhase::Broken);
                        assert!(runtime.prepare(None,CancellationToken::new()).await.is_err());
                        assert_eq!(executor.installs.load(Ordering::SeqCst),1);
                        assert_eq!(std::fs::read(shared.join("partial-install-diagnostic")).unwrap(),diagnostic);
                        assert!(!prefix.exists());
                        assert_eq!(std::fs::read_dir(&root).unwrap().map(|entry|entry.unwrap().file_name())
                            .filter(|name|name.to_string_lossy().contains("shared")).count(),1,"no second installation directory");
                    },
                    _=>panic!("Unknown prepare fixture case"),
                }
            }).await.expect("offline prepare case completed within its deadline");
        });
    }
    #[test]
    fn official_npm_launcher_is_resolved_to_the_existing_native_binary() {
        let temp = tempfile::tempdir().unwrap();
        let package = if cfg!(windows) { temp.path().join("node_modules/@larksuite/cli") }
            else { temp.path().join("lib/node_modules/@larksuite/cli") };
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("package.json"), r#"{"name":"@larksuite/cli"}"#).unwrap();
        let native = npm_native(temp.path());std::fs::write(&native, b"native fixture").unwrap();
        let launcher = npm_launcher(temp.path());
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::write(&launcher, b"launcher fixture").unwrap();
        assert_eq!(normalize_executable(&launcher).unwrap(), native.canonicalize().unwrap());
    }
    #[test]
    fn input_directory_cleanup_is_exact_and_keeps_unrelated_files() {
        let temp = tempfile::tempdir().unwrap();
        let other = temp.path().join("unrelated");std::fs::write(&other, b"keep").unwrap();
        let input = temp.path().join("exact-private-job");std::fs::create_dir(&input).unwrap();
        std::fs::write(input.join(PAYLOAD_FILE), b"secret payload").unwrap();
        drop(PrivateLaunchDirectory(input.clone()));
        assert!(!input.exists());assert!(other.exists());
    }
    // These exact test-owned children are invoked by the tests below. A normal
    // run must not read stdin or enter their sleep loop.
    #[test]
    #[ignore]
    fn process_input_fixture() {
        use std::io::Read;
        let mut input=String::new();std::io::stdin().read_to_string(&mut input).unwrap();
        let payload=std::fs::read_to_string(PAYLOAD_FILE).unwrap();
        assert_eq!(input,"test-only-stdin-中文");
        assert_eq!(payload,r#"{"value":"test-only-file"}"#);
        println!("office-private-inputs-observed");
    }
    #[test]
    #[ignore]
    fn process_wait_fixture() {
        use std::io::Write;
        println!("office-fixture-started");std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_secs(30));
    }
    #[tokio::test]
    async fn supervised_executor_delivers_stdin_and_private_file_then_cleans_them() {
        let temp=tempfile::tempdir().unwrap();
        let executor=SupervisedCliExecutor::new(temp.path().into());
        let mut command=CliCommand::new(["--ignored","--exact","runtime::tests::process_input_fixture","--nocapture"],false);
        command.stdin=Some("test-only-stdin-中文".as_bytes().to_vec());
        command.input_file=Some(br#"{"value":"test-only-file"}"#.to_vec());
        command.timeout=Duration::from_secs(15);
        let output=executor.execute(&std::env::current_exe().unwrap(),command,CancellationToken::new(),None).await.unwrap();
        assert!(output.successful());assert!(output.stdout.contains("office-private-inputs-observed"));
        assert!(std::fs::read_dir(temp.path().join("office-cli-inputs")).unwrap().next().is_none());
    }
    #[tokio::test]
    async fn supervised_executor_cancels_only_its_exact_child_and_returns_unknown_evidence() {
        let temp=tempfile::tempdir().unwrap();
        let executor=SupervisedCliExecutor::new(temp.path().into());
        let cancel=CancellationToken::new();let cancel_on_output=cancel.clone();
        let seen=Arc::new(std::sync::Mutex::new(String::new()));
        let observer:CliObserver=Arc::new(move|stream,text|{
            if stream==CliStream::Stdout{
                let mut seen=seen.lock().unwrap();seen.push_str(text);
                if seen.contains("office-fixture-started"){cancel_on_output.cancel();}
            }
        });
        let command=CliCommand{timeout:Duration::from_secs(10),
            ..CliCommand::new(["--ignored","--exact","runtime::tests::process_wait_fixture","--nocapture"],true)};
        let output=executor.execute(&std::env::current_exe().unwrap(),command,cancel,Some(observer)).await.unwrap();
        assert!(output.started);assert!(output.cancelled);assert!(!output.successful());
        assert!(output.cleanup_error.is_none());
        assert!(std::fs::read_dir(temp.path().join("office-cli-inputs")).unwrap().next().is_none());
    }
}
