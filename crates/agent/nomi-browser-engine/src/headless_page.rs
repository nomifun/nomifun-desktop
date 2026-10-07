//! One-request Headless page owner. No Workspace, vault, user profile, or
//! model-controlled browser commands enter this module.

use crate::{
    launch::{LaunchedProcessGuard, launch_headless_page_chrome},
    transport::{Connection, ROOT_SESSION},
};
use base64::Engine;
use chromiumoxide::{
    cdp::{
        browser_protocol::{browser, fetch, page, target},
        js_protocol::runtime,
    },
    types::{Command, MethodType},
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinSet};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio_util::sync::CancellationToken;
use url::Url;

const DEADLINE: Duration = Duration::from_secs(30);
const MAX_BODY: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BODY: usize = 16 * 1024 * 1024;

// Shared by search, rendering and explicit version probes in this host process.
// No per-model/per-session construction can multiply the browser process limit.
const MAX_HEADLESS_PROCESSES: usize = 2;
const MAX_HEADLESS_REQUESTS: usize = 16;
static ADMISSION: std::sync::LazyLock<Admission> = std::sync::LazyLock::new(||
    Admission::new(MAX_HEADLESS_PROCESSES, MAX_HEADLESS_REQUESTS));

struct Admission {
    active: Arc<Semaphore>,
    outstanding: Arc<Semaphore>,
}
#[derive(Debug)]
struct ProcessCapacity {
    _active: OwnedSemaphorePermit,
    _outstanding: OwnedSemaphorePermit,
}
impl Admission {
    fn new(active: usize, outstanding: usize) -> Self {
        Self { active: Arc::new(Semaphore::new(active)), outstanding: Arc::new(Semaphore::new(outstanding)) }
    }
    async fn acquire(&self, deadline: tokio::time::Instant, cancel: &CancellationToken) -> Result<ProcessCapacity, HeadlessPageError> {
        if cancel.is_cancelled() { return Err(HeadlessPageError::Canceled); }
        if tokio::time::Instant::now() >= deadline { return Err(HeadlessPageError::Timeout); }
        let outstanding = self.outstanding.clone().try_acquire_owned().map_err(|error| match error {
            TryAcquireError::NoPermits => HeadlessPageError::Busy,
            TryAcquireError::Closed => HeadlessPageError::Unavailable,
        })?;
        let active = tokio::select! { biased;
            _=cancel.cancelled()=>return Err(HeadlessPageError::Canceled),
            result=tokio::time::timeout_at(deadline,self.active.clone().acquire_owned())=>
                result.map_err(|_|HeadlessPageError::Timeout)?.map_err(|_|HeadlessPageError::Unavailable)?,
        };
        if cancel.is_cancelled() { return Err(HeadlessPageError::Canceled); }
        if tokio::time::Instant::now() >= deadline { return Err(HeadlessPageError::Timeout); }
        Ok(ProcessCapacity { _active: active, _outstanding: outstanding })
    }
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) { self.0.cancel(); }
}

struct NetworkActivity(Arc<AtomicUsize>);
impl Drop for NetworkActivity {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn reserve_response_bytes(total: &AtomicUsize, bytes: usize) -> bool {
    total
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(bytes)
                .filter(|next| *next <= MAX_TOTAL_BODY)
        })
        .is_ok()
}

pub fn implementation_digest() -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for source in [
        include_bytes!("headless_page.rs").as_slice(),
        include_bytes!("render_content.js").as_slice(),
        include_bytes!("launch.rs").as_slice(),
        include_bytes!("transport.rs").as_slice(),
        include_bytes!("session.rs").as_slice(),
        include_bytes!("profile.rs").as_slice(),
        include_bytes!("switches.rs").as_slice(),
    ] {
        digest.update(source.len().to_le_bytes());
        digest.update(source);
    }
    digest.update(nomifun_net::egress::implementation_digest());
    digest.update(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../Cargo.lock"
    )));
    digest.update(env!("CARGO_PKG_VERSION"));
    digest.update(std::env::consts::OS);
    digest.update(std::env::consts::ARCH);
    format!("{:x}", digest.finalize())
}

/// Constructed by a compiled adapter, never deserialized from Tool input.
pub struct PageRequest {
    pub url: Url,
    /// Exact HTTPS origins for Search; empty for public RenderContent.
    pub allowed_origins: BTreeSet<String>,
    /// Host-selected purpose, never part of model or Knowledge request JSON.
    pub purpose: PagePurpose,
    /// Fixed adapter code returning {state: waiting|ready|empty|challenge|invalid,...}.
    pub extraction: &'static str,
    pub language: String,
    pub expected_browser_product: Option<String>,
    #[cfg(test)]
    pub(crate) fixture: bool,
    #[cfg(test)]
    pub(crate) opened_profile: Option<tokio::sync::oneshot::Sender<PathBuf>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PagePurpose {
    Search,
    RenderContent,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct RenderedContent {
    pub final_url: String,
    pub html: String,
    pub html_truncated: bool,
}

/// Fingerprint host-selected browser bytes without executing the file.
pub async fn binary_digest(path: PathBuf) -> Result<String, HeadlessPageError> {
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        use sha2::{Digest,Sha256};
        let mut file=std::fs::File::open(path).map_err(|_|HeadlessPageError::Unavailable)?;
        let metadata=file.metadata().map_err(|_|HeadlessPageError::Unavailable)?;
        if !metadata.is_file() || metadata.len()>512*1024*1024 {return Err(HeadlessPageError::Unavailable);}
        let mut hash=Sha256::new(); let mut buffer=[0;65536];
        loop {let n=file.read(&mut buffer).map_err(|_|HeadlessPageError::Unavailable)?;if n==0 {break;}hash.update(&buffer[..n]);}
        Ok(format!("{:x}",hash.finalize()))
    }).await.map_err(|_|HeadlessPageError::Unavailable)?
}

pub async fn installed_release_digest(path: &std::path::Path, product: &str) -> Result<String, HeadlessPageError> {
    if !path.is_absolute() || product.len()>64 {return Err(HeadlessPageError::Unavailable);}
    let version=product.strip_prefix("Chrome/").ok_or(HeadlessPageError::Unavailable)?;
    let parts:Vec<_>=version.split('.').collect();
    if parts.len()!=4 || parts.iter().any(|part|part.is_empty() || !part.bytes().all(|b|b.is_ascii_digit()) || part.parse::<u16>().is_err())
        || parts[0].parse::<u16>().map_err(|_|HeadlessPageError::Unavailable)?<120 {return Err(HeadlessPageError::Unavailable);}
    binary_digest(path.to_path_buf()).await
}

fn render_request(url: Url, expected_product: String, language: String) -> PageRequest {
    PageRequest {
        url,
        allowed_origins: BTreeSet::new(),
        purpose: PagePurpose::RenderContent,
        extraction: include_str!("render_content.js"),
        language,
        expected_browser_product: Some(expected_product),
        #[cfg(test)]
        fixture: false,
        #[cfg(test)]
        opened_profile: None,
    }
}

/// Browser-executed HTML in an anonymous, one-operation process/context.
/// This is an engine port, not provider selection or Knowledge admission.
pub async fn render_content(
    chrome: PathBuf,
    url: Url,
    expected_product: String,
    language: String,
    cancel: CancellationToken,
) -> Result<RenderedContent, HeadlessPageError> {
    let output = extract_page(
        chrome,
        render_request(url, expected_product, language),
        cancel,
    )
    .await?;
    serde_json::from_value(output).map_err(|_| HeadlessPageError::InvalidResult)
}

impl PageRequest {
    fn permits(&self, url: &Url) -> bool {
        permits(self.purpose, &self.allowed_origins, url)
    }
    fn output_limit(&self) -> usize {
        match self.purpose {
            PagePurpose::Search => 128 * 1024,
            PagePurpose::RenderContent => 2 * 1024 * 1024,
        }
    }
}
fn permits(purpose: PagePurpose, origins: &BTreeSet<String>, url: &Url) -> bool {
    match purpose {
        PagePurpose::Search => allows(origins, url),
        // Routing permission is not network authorization: SafeHttpClient
        // validates and pins public DNS/IP addresses for every actual fetch.
        PagePurpose::RenderContent => {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HeadlessPageError {
    #[error("The isolated browser capacity is busy; try again later.")]
    Busy,
    #[error("Headless browser differs from the pinned runtime.")]
    BindingChanged,
    #[error("Headless browser is unavailable.")]
    Unavailable,
    #[error("Headless browser networking was blocked.")]
    Blocked,
    #[error("Headless browser deadline elapsed.")]
    Timeout,
    #[error("Headless browser operation was canceled.")]
    Canceled,
    #[error("Headless page requires human verification or consent.")]
    Challenge,
    #[error("Headless page no longer matches the installed adapter.")]
    InvalidResult,
    #[error("Headless browser cleanup could not be proven.")]
    Cleanup,
}

async fn command<C: Command + MethodType + DeserializeOwned>(
    connection: &Connection,
    session: &str,
    params: Value,
) -> Result<Value, HeadlessPageError> {
    let params: C = serde_json::from_value(params).map_err(|_| HeadlessPageError::Unavailable)?;
    connection.send(session, &params).await.map_err(|error| {
        #[cfg(test)]
        eprintln!(
            "headless CDP {} failed: {error}",
            std::any::type_name::<C>()
        );
        #[cfg(not(test))]
        let _ = error;
        HeadlessPageError::Unavailable
    })
}

/// A dropped caller cancels admission but never drops the in-flight cleanup
/// owner. Its worker settles network activity and obtains process exit proof.
pub async fn extract_page(
    chrome: PathBuf,
    request: PageRequest,
    cancel: CancellationToken,
) -> Result<Value, HeadlessPageError> {
    let owned_cancel = cancel.child_token();
    let _cancel_on_drop = CancelOnDrop(owned_cancel.clone());
    tokio::spawn(async move { run(chrome, request, owned_cancel).await })
        .await
        .map_err(|_| HeadlessPageError::Cleanup)?
}

struct Owner {
    process: LaunchedProcessGuard,
    connection: Connection,
    workers: JoinSet<()>,
}

struct RejectingProxy {
    // Retain the socket reservation even if the Tokio runtime is shut down.
    // The exact process/profile cleanup lease owns this value until exit proof.
    _reservation: std::net::TcpListener,
    task: tokio::task::JoinHandle<()>,
    // Transfer capacity with the existing physical process/Profile cleanup
    // lease, including launch cancellation and cleanup after caller Drop.
    _capacity: ProcessCapacity,
}
impl Drop for RejectingProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Owner {
    async fn close(&mut self) -> Result<(), HeadlessPageError> {
        self.workers.abort_all();
        while self.workers.join_next().await.is_some() {}
        self.connection.shutdown().await;
        let result = self
            .process
            .shutdown()
            .await
            .map_err(|_| HeadlessPageError::Cleanup);
        result
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.workers.abort_all();
    }
}

async fn run(
    chrome: PathBuf,
    #[allow(unused_mut)] mut request: PageRequest,
    cancel: CancellationToken,
) -> Result<Value, HeadlessPageError> {
    if cancel.is_cancelled() {
        return Err(HeadlessPageError::Canceled);
    }
    validate_request(&request)?;
    let deadline = tokio::time::Instant::now() + DEADLINE;
    let mut owner = open_owner(chrome, deadline, &cancel).await?;
    #[cfg(test)]
    if let Some(observer) = request.opened_profile.take() {
        let _ = observer.send(
            owner
                .process
                .ephemeral_profile_path()
                .expect("headless profile ownership")
                .to_path_buf(),
        );
    }
    let result = tokio::select! {
        biased;
        _=cancel.cancelled()=>Err(HeadlessPageError::Canceled),
        result=tokio::time::timeout_at(deadline,extract(&mut owner,request))=>result.unwrap_or(Err(HeadlessPageError::Timeout)),
    };
    owner.close().await?;
    result
}

async fn open_owner(
    chrome: PathBuf,
    deadline: tokio::time::Instant,
    cancel: &CancellationToken,
) -> Result<Owner, HeadlessPageError> {
    let capacity = ADMISSION.acquire(deadline, cancel).await?;
    let reservation =
        std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| HeadlessPageError::Unavailable)?;
    reservation
        .set_nonblocking(true)
        .map_err(|_| HeadlessPageError::Unavailable)?;
    let proxy_address = reservation
        .local_addr()
        .map_err(|_| HeadlessPageError::Unavailable)?;
    let proxy = TcpListener::from_std(
        reservation
            .try_clone()
            .map_err(|_| HeadlessPageError::Unavailable)?,
    )
    .map_err(|_| HeadlessPageError::Unavailable)?;
    let proxy_task = tokio::spawn(async move {
        while let Ok((socket, _)) = proxy.accept().await {
            drop(socket);
        }
    });
    let boundary = crate::cleanup::HostCleanupLease::new(RejectingProxy {
        _reservation: reservation,
        task: proxy_task,
        _capacity: capacity,
    });
    let launched = tokio::time::timeout_at(
        deadline,
        launch_headless_page_chrome(chrome, proxy_address, boundary),
    )
    .await;
    let launched = match launched {
        Ok(Ok(launched)) => launched,
        Ok(Err(error)) => {
            #[cfg(test)]
            eprintln!("headless launch failed: {error}");
            #[cfg(not(test))]
            let _ = error;
            return Err(HeadlessPageError::Unavailable);
        }
        Err(_) => {
            return Err(HeadlessPageError::Timeout);
        }
    };
    let (process, connection) = match tokio::time::timeout_at(deadline, launched.connect()).await {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => {
            return Err(HeadlessPageError::Unavailable);
        }
        Err(_) => {
            return Err(HeadlessPageError::Timeout);
        }
    };
    Ok(Owner {
        process,
        connection,
        workers: JoinSet::new(),
    })
}

/// Local-only startup/protocol probe; creates no page and sends no web query.
pub async fn probe_runtime(chrome: PathBuf) -> Result<String, HeadlessPageError> {
    let cancel = CancellationToken::new();
    let _cancel_on_drop = CancelOnDrop(cancel.clone());
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + DEADLINE;
        let mut owner = open_owner(chrome, deadline, &cancel).await?;
        let result = tokio::select! { biased;
            _=cancel.cancelled()=>Err(HeadlessPageError::Canceled),
            result=tokio::time::timeout_at(
            deadline,
            command::<browser::GetVersionParams>(&owner.connection, ROOT_SESSION, json!({})),
            )=>result.unwrap_or(Err(HeadlessPageError::Timeout)),
        };
        owner.close().await?;
        let version = result?;
        version["product"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(String::from)
            .ok_or(HeadlessPageError::Unavailable)
    })
    .await
    .map_err(|_| HeadlessPageError::Cleanup)?
}

fn validate_request(request: &PageRequest) -> Result<(), HeadlessPageError> {
    #[cfg(test)]
    let fixture = request.fixture;
    #[cfg(not(test))]
    let fixture = false;
    if (request.purpose == PagePurpose::Search && request.allowed_origins.is_empty())
        || (!fixture
            && request.purpose == PagePurpose::RenderContent
            && !request.allowed_origins.is_empty())
        || (!fixture
            && request
                .expected_browser_product
                .as_ref()
                .is_none_or(|value| value.is_empty() || value.len() > 256))
        || request.url.as_str().len() > 8192
        || request.allowed_origins.len() > 8
        || request.extraction.len() > 32 * 1024
        || request.language.len() > 64
        || !request
            .language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-,;= .".contains(&byte))
    {
        return Err(HeadlessPageError::Blocked);
    }
    for origin in &request.allowed_origins {
        let url = Url::parse(origin).map_err(|_| HeadlessPageError::Blocked)?;
        if (!fixture && url.scheme() != "https")
            || url.origin().ascii_serialization() != *origin
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(HeadlessPageError::Blocked);
        }
    }
    if !request.permits(&request.url) {
        return Err(HeadlessPageError::Blocked);
    }
    Ok(())
}

fn allows(origins: &BTreeSet<String>, url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && origins.contains(&url.origin().ascii_serialization())
}

async fn extract(owner: &mut Owner, request: PageRequest) -> Result<Value, HeadlessPageError> {
    let connection = &owner.connection;
    if let Some(expected) = &request.expected_browser_product {
        let version =
            command::<browser::GetVersionParams>(connection, ROOT_SESSION, json!({})).await?;
        if version["product"].as_str() != Some(expected.as_str()) {
            return Err(HeadlessPageError::BindingChanged);
        }
    }
    let context = command::<target::CreateBrowserContextParams>(
        connection,
        ROOT_SESSION,
        json!({"disposeOnDetach":true}),
    )
    .await?;
    let context_id = context["browserContextId"]
        .as_str()
        .ok_or(HeadlessPageError::Unavailable)?
        .to_owned();
    for permission in [
        "geolocation",
        "notifications",
        "camera",
        "microphone",
        "clipboard-read",
        "clipboard-write",
        "midi",
        "persistent-storage",
    ] {
        command::<browser::SetPermissionParams>(connection,ROOT_SESSION,json!({"browserContextId":context_id,"permission":{"name":permission},"setting":"denied"})).await?;
    }
    command::<browser::SetDownloadBehaviorParams>(
        connection,
        ROOT_SESSION,
        json!({"browserContextId":context_id,"behavior":"deny"}),
    )
    .await?;

    let blocked = Arc::new(AtomicBool::new(false));
    let network_activity = Arc::new(AtomicUsize::new(0));
    let active_fetches = network_activity.clone();
    let main_frame = Arc::new(std::sync::Mutex::new(String::new()));
    let network_main_frame = main_frame.clone();
    let mut fetches = connection.subscribe_reliable("Fetch.requestPaused", None);
    let network_connection = connection.clone();
    let origins = request.allowed_origins.clone();
    let purpose = request.purpose;
    let network_blocked = blocked.clone();
    let language = request.language.clone();
    #[cfg(test)]
    let fixture = request.fixture;
    #[cfg(not(test))]
    let fixture = false;
    owner.workers.spawn(async move {
        let mut client=nomifun_net::egress::SafeHttpClient::new(Duration::from_secs(10),MAX_BODY);
        if fixture {client=client.allow_private_for_tests();}
        else if purpose==PagePurpose::Search {client=client.with_public_dns_for_hosts(origins.iter().filter_map(|origin|Url::parse(origin).ok()).filter_map(|url|url.host_str().map(str::to_owned)));}
        let total=Arc::new(AtomicUsize::new(0));
        // Child futures are owned directly, not detached tasks. Aborting this
        // worker synchronously drops every pending HTTP fetch.
        let mut in_flight=FuturesUnordered::new();
        loop {
            let event=tokio::select! {
                event=fetches.recv(), if in_flight.len()<4=>event,
                _=in_flight.next(), if !in_flight.is_empty()=>continue,
            };
            let Some(event)=event else {break};
            let network_connection=network_connection.clone();
            let network_main_frame=network_main_frame.clone();
            let network_blocked=network_blocked.clone();
            let origins=origins.clone();
            let language=language.clone();
            let total=total.clone();
            let client=client.clone();
            active_fetches.fetch_add(1,Ordering::AcqRel);
            let activity=NetworkActivity(active_fetches.clone());
            in_flight.push(async move {
            let _activity=activity;
            let session=event.session_id.as_str();
            let params=&event.params;
            let id=match params["requestId"].as_str(){Some(id)=>id,None=>{network_blocked.store(true,Ordering::Release);return}};
            let main_document=params["resourceType"]=="Document" && params["frameId"].as_str()==Some(network_main_frame.lock().unwrap_or_else(|error|error.into_inner()).as_str());
            let url=params["request"]["url"].as_str().and_then(|raw|Url::parse(raw).ok());
            let allowed=params["request"]["method"]=="GET" && url.as_ref().is_some_and(|url|
                if fixture {allows(&origins,url)} else {permits(purpose,&origins,url)});
            if !allowed || total.load(Ordering::Acquire)>=MAX_TOTAL_BODY {
                if main_document {network_blocked.store(true,Ordering::Release);}
                let _=command::<fetch::FailRequestParams>(&network_connection,session,json!({"requestId":id,"errorReason":"BlockedByClient"})).await;
                return;
            }
            let mut headers=reqwest::header::HeaderMap::new();
            if let Ok(value)=language.parse(){headers.insert(reqwest::header::ACCEPT_LANGUAGE,value);}
            // Cookies can only originate from this fresh anonymous context.
            for name in ["accept","user-agent","cookie","origin","referer"] {
                if purpose==PagePurpose::Search && matches!(name,"origin"|"referer") {continue;}
                if let Some(value)=params["request"]["headers"].as_object().and_then(|headers|headers.iter().find(|(key,_)|key.eq_ignore_ascii_case(name))).and_then(|(_,value)|value.as_str())
                    && let Ok(value)=value.parse() {headers.insert(reqwest::header::HeaderName::from_static(name),value);}
            }
            let hop_started=std::time::Instant::now();
            let response=client.get_once(url.as_ref().expect("validated URL").as_str(),headers).await;
            tracing::debug!(main_document,resource=params["resourceType"].as_str().unwrap_or("unknown"),elapsed_ms=hop_started.elapsed().as_millis(),status=response.as_ref().ok().map(|response|response.status.as_u16()),error=?response.as_ref().err().map(|error|error.kind()),"headless page network hop");
            match response {
                Ok(response) if reserve_response_bytes(&total,response.body.len()) => {
                    if main_document && response.status.as_u16()>=400 {network_blocked.store(true,Ordering::Release);}
                    let headers:Vec<_>=response.headers.iter().filter(|(name,_)|!matches!(name.as_str(),"content-length"|"transfer-encoding"|"connection"|"proxy-authenticate"))
                        .filter_map(|(name,value)|value.to_str().ok().map(|value|json!({"name":name.as_str(),"value":value}))).collect();
                    if command::<fetch::FulfillRequestParams>(&network_connection,session,json!({"requestId":id,"responseCode":response.status.as_u16(),"responseHeaders":headers,"body":base64::engine::general_purpose::STANDARD.encode(&response.body)})).await.is_err(){network_blocked.store(true,Ordering::Release);}
                }
                _ => {
                    if main_document {network_blocked.store(true,Ordering::Release);}
                    let _=command::<fetch::FailRequestParams>(&network_connection,session,json!({"requestId":id,"errorReason":"BlockedByClient"})).await;
                }
            }
            });
        }
        network_blocked.store(true,Ordering::Release);
    });

    let mut attached = connection.subscribe_reliable("Target.attachedToTarget", None);
    let attach_connection = connection.clone();
    let attach_blocked = blocked.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let target_count = Arc::new(AtomicUsize::new(0));
    owner.workers.spawn(async move {
        let mut ready = Some(ready_tx);
        while let Some(event) = attached.recv().await {
            let Ok(event) = serde_json::from_value::<target::EventAttachedToTarget>(event.params)
            else {
                break;
            };
            let page = event.target_info.r#type == "page";
            let own_context = event
                .target_info
                .browser_context_id
                .as_ref()
                .is_some_and(|id| id.as_ref() == context_id);
            if target_count.fetch_add(1, Ordering::AcqRel) >= 8
                || (page && (ready.is_none() || !own_context))
            {
                let _ = command::<target::CloseTargetParams>(
                    &attach_connection,
                    ROOT_SESSION,
                    json!({"targetId":event.target_info.target_id}),
                )
                .await;
                if !page {
                    attach_blocked.store(true, Ordering::Release);
                }
                continue;
            }
            if attach_connection.handle_attached(&event).await.is_err() {
                break;
            }
            if page && let Some(sender) = ready.take() {
                let _ = sender.send(String::from(event.session_id));
            }
        }
        attach_blocked.store(true, Ordering::Release);
    });
    connection
        .enable_auto_attach()
        .await
        .map_err(|_| HeadlessPageError::Unavailable)?;
    command::<target::CreateTargetParams>(
        connection,
        ROOT_SESSION,
        json!({"url":"about:blank","browserContextId":context["browserContextId"]}),
    )
    .await?;
    let session = ready_rx.await.map_err(|_| HeadlessPageError::Unavailable)?;
    let initial_tree = command::<page::GetFrameTreeParams>(connection, &session, json!({})).await?;
    *main_frame.lock().unwrap_or_else(|error| error.into_inner()) =
        initial_tree["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or(HeadlessPageError::Unavailable)?
            .to_owned();
    command::<page::NavigateParams>(connection, &session, json!({"url":request.url.as_str()}))
        .await?;
    tracing::debug!("headless page navigation submitted");
    let mut fatal = connection.subscribe_fatal();
    let mut last_extraction_state = String::new();
    loop {
        if fatal.borrow().is_some() {
            return Err(HeadlessPageError::Blocked);
        }
        let tree = command::<page::GetFrameTreeParams>(connection, &session, json!({})).await?;
        let current = tree["frameTree"]["frame"]["url"].as_str().unwrap_or("");
        if current.len()>8192 {return Err(HeadlessPageError::InvalidResult);}
        if current == "about:blank" {
            if blocked.load(Ordering::Acquire) {
                return Err(HeadlessPageError::Blocked);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        }
        if !Url::parse(current)
            .ok()
            .as_ref()
            .is_some_and(|url| request.permits(url))
        {
            return Err(HeadlessPageError::Blocked);
        }
        let frame = tree["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or(HeadlessPageError::Unavailable)?;
        let world = command::<page::CreateIsolatedWorldParams>(
            connection,
            &session,
            json!({"frameId":frame,"worldName":"nomifun-headless-extraction"}),
        )
        .await?;
        let value=command::<runtime::EvaluateParams>(connection,&session,json!({"expression":request.extraction,"contextId":world["executionContextId"],"returnByValue":true,"awaitPromise":false,"userGesture":false})).await;
        if let Ok(value) = value
            && value.get("exceptionDetails").is_none()
        {
            let output = &value["result"]["value"];
            let state = output["state"].as_str().unwrap_or("unknown");
            if state != last_extraction_state {
                tracing::debug!(state, "headless page extraction state");
                last_extraction_state = state.to_owned();
            }
            if output.to_string().len() > request.output_limit() {
                return Err(HeadlessPageError::InvalidResult);
            }
            match output["state"].as_str() {
                Some("ready" | "empty") => {
                    if blocked.load(Ordering::Acquire) {
                        return Err(HeadlessPageError::Blocked);
                    }
                    if request.purpose == PagePurpose::RenderContent
                        && network_activity.load(Ordering::Acquire) > 0
                    {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    let targets =
                        command::<target::GetTargetsParams>(connection, ROOT_SESSION, json!({}))
                            .await?;
                    let pages = targets["targetInfos"]
                        .as_array()
                        .ok_or(HeadlessPageError::Unavailable)?
                        .iter()
                        .filter(|target| target["type"] == "page")
                        .count();
                    if pages != 1 {
                        return Err(HeadlessPageError::InvalidResult);
                    }
                    if request.purpose == PagePurpose::RenderContent {
                        let actual =
                            command::<page::GetFrameTreeParams>(connection, &session, json!({}))
                                .await?;
                        if output["final_url"] != actual["frameTree"]["frame"]["url"] {
                            return Err(HeadlessPageError::InvalidResult);
                        }
                    }
                    return Ok(output.clone());
                }
                Some("challenge") => return Err(HeadlessPageError::Challenge),
                Some("invalid") => {
                    return Err(if blocked.load(Ordering::Acquire) {
                        HeadlessPageError::Blocked
                    } else {
                        HeadlessPageError::InvalidResult
                    });
                }
                _ => {}
            }
        }
        if blocked.load(Ordering::Acquire) {
            return Err(HeadlessPageError::Blocked);
        }
        tokio::select! {
            _=fatal.changed()=>{},
            _=tokio::time::sleep(Duration::from_millis(100))=>{},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn admission_bounds_active_processes_and_waiting_requests() {
        let admission = Admission::new(1, 2);
        let cancel = CancellationToken::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let active = admission.acquire(deadline, &cancel).await.unwrap();
        let mut waiting = Box::pin(admission.acquire(deadline, &cancel));
        assert!(futures_util::poll!(waiting.as_mut()).is_pending());
        assert_eq!(admission.acquire(deadline, &cancel).await.unwrap_err(), HeadlessPageError::Busy);
        assert_eq!(admission.active.available_permits(), 0);
        drop(active);
        let admitted = waiting.await.unwrap();
        assert_eq!(admission.active.available_permits(), 0);
        drop(admitted);
        assert_eq!(admission.active.available_permits(), 1);
        assert_eq!(admission.outstanding.available_permits(), 2);
    }

    #[tokio::test]
    async fn canceled_and_expired_admission_never_consumes_capacity() {
        let admission = Admission::new(1, 2);
        let cancel = CancellationToken::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let active = admission.acquire(deadline, &cancel).await.unwrap();
        let stopped = CancellationToken::new();
        let mut waiting = Box::pin(admission.acquire(deadline, &stopped));
        assert!(futures_util::poll!(waiting.as_mut()).is_pending());
        stopped.cancel();
        assert_eq!(waiting.await.unwrap_err(), HeadlessPageError::Canceled);
        assert_eq!(admission.outstanding.available_permits(), 1);
        assert_eq!(admission.acquire(tokio::time::Instant::now(), &cancel).await.unwrap_err(), HeadlessPageError::Timeout);
        assert_eq!(admission.outstanding.available_permits(), 1);
        drop(active);
        assert_eq!(admission.acquire(deadline, &stopped).await.unwrap_err(), HeadlessPageError::Canceled);
        assert_eq!(admission.active.available_permits(), 1);
        assert_eq!(admission.outstanding.available_permits(), 2);
    }

    #[tokio::test]
    async fn process_capacity_survives_until_the_final_cleanup_lease_is_released() {
        let admission = Admission::new(1, 1);
        let cancel = CancellationToken::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let capacity = admission.acquire(deadline, &cancel).await.unwrap();
        let lease = crate::cleanup::HostCleanupLease::new(capacity);
        let process_cleanup = lease.clone();
        drop(lease);
        assert_eq!(admission.acquire(deadline, &cancel).await.unwrap_err(), HeadlessPageError::Busy);
        drop(process_cleanup);
        assert!(admission.acquire(deadline, &cancel).await.is_ok());
    }

    #[test]
    fn rendering_policy_does_not_expand_the_search_allowlist() {
        let origins = BTreeSet::from(["https://www.bing.com".into()]);
        let external = Url::parse("https://static.example.org/app.js").unwrap();
        assert!(!permits(PagePurpose::Search, &origins, &external));
        assert!(permits(
            PagePurpose::RenderContent,
            &BTreeSet::new(),
            &external
        ));
        for url in [
            "file:///secret",
            "https://user:pass@example.org/",
            "data:text/html,hello",
            "ws://example.org/",
        ] {
            assert!(!permits(
                PagePurpose::RenderContent,
                &BTreeSet::new(),
                &Url::parse(url).unwrap()
            ));
        }
        let mut request = render_request(
            Url::parse("https://example.org/").unwrap(),
            "Chrome/pinned".into(),
            "en-US".into(),
        );
        assert!(validate_request(&request).is_ok());
        request.allowed_origins = origins;
        assert_eq!(validate_request(&request), Err(HeadlessPageError::Blocked));
        request.allowed_origins.clear();
        request.expected_browser_product = None;
        assert_eq!(validate_request(&request), Err(HeadlessPageError::Blocked));
    }

    #[tokio::test]
    #[ignore = "requires NOMIFUN_SEARCH_CHROME pointing to an installed Chromium binary"]
    async fn public_rendering_blocks_loopback_before_network_transmission() {
        let chrome = PathBuf::from(std::env::var_os("NOMIFUN_SEARCH_CHROME").unwrap());
        let product = probe_runtime(chrome.clone()).await.unwrap();
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/private", server.local_addr().unwrap())).unwrap();
        let result = render_content(
            chrome,
            url,
            product,
            "en-US".into(),
            CancellationToken::new(),
        )
        .await;
        assert!(matches!(result, Err(HeadlessPageError::Blocked)));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), server.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    #[ignore = "requires NOMIFUN_SEARCH_CHROME and public network access"]
    async fn public_render_content_uses_the_production_network_path() {
        let chrome=PathBuf::from(std::env::var_os("NOMIFUN_SEARCH_CHROME").unwrap());
        let product=probe_runtime(chrome.clone()).await.unwrap();
        let result=render_content(chrome,Url::parse("https://example.com/").unwrap(),product,"en-US".into(),CancellationToken::new()).await.unwrap();
        assert_eq!(result.final_url,"https://example.com/");
        assert!(result.html.contains("Example Domain"));
        assert!(!result.html_truncated);
    }

    #[tokio::test]
    #[ignore = "requires NOMIFUN_SEARCH_CHROME pointing to an installed Chromium binary"]
    async fn rendered_html_contains_delayed_cross_origin_content_and_cleans_profile() {
        let chrome = PathBuf::from(std::env::var_os("NOMIFUN_SEARCH_CHROME").unwrap());
        let product = probe_runtime(chrome.clone()).await.unwrap();
        let main = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let asset = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", main.local_addr().unwrap());
        let asset_origin = format!("http://{}", asset.local_addr().unwrap());
        let html = format!(
            "<!doctype html><title>Rendered fixture</title><body><div id=result>waiting</div><script>fetch('{asset_origin}/data').then(r=>r.text()).then(t=>document.getElementById('result').textContent=t)</script></body>"
        );
        let main_worker = tokio::spawn(async move {
            while let Ok((mut socket, _)) = main.accept().await {
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(
                    !String::from_utf8_lossy(&buffer[..count])
                        .to_ascii_lowercase()
                        .contains("authorization:")
                );
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                    html.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        let asset_worker = tokio::spawn(async move {
            let (mut socket, _) = asset.accept().await.unwrap();
            let mut buffer = [0; 8192];
            let count = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..count]).to_ascii_lowercase();
            assert!(
                request.contains("origin:"),
                "browser CORS origin must reach the resource server"
            );
            tokio::time::sleep(Duration::from_millis(700)).await;
            let body = "动态渲染 中文";
            let reply = format!(
                "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
        });
        let (opened, profile) = tokio::sync::oneshot::channel();
        let mut request = render_request(
            Url::parse(&format!("{origin}/")).unwrap(),
            product,
            "zh-CN".into(),
        );
        request.fixture = true;
        request.allowed_origins = BTreeSet::from([origin.clone(), asset_origin]);
        request.opened_profile = Some(opened);
        let result = extract_page(chrome, request, CancellationToken::new()).await;
        main_worker.abort();
        let _ = main_worker.await;
        if !asset_worker.is_finished() {
            asset_worker.abort();
        }
        asset_worker
            .await
            .expect("asset handler should have completed");
        let output: RenderedContent = serde_json::from_value(result.unwrap()).unwrap();
        assert_eq!(output.final_url, format!("{origin}/"));
        assert!(
            output.html.contains("动态渲染 中文"),
            "must capture the JavaScript-updated DOM, not the HTTP source"
        );
        assert!(!output.html_truncated);
        assert!(
            !profile.await.unwrap().exists(),
            "exact profile must be removed before returning HTML"
        );
    }

    #[test]
    fn concurrent_fetches_share_one_non_overflowing_byte_budget() {
        let total = Arc::new(AtomicUsize::new(0));
        let successes = std::thread::scope(|scope| {
            let tasks: Vec<_> = (0..16)
                .map(|_| {
                    let total = total.clone();
                    scope.spawn(move || reserve_response_bytes(&total, MAX_BODY))
                })
                .collect();
            tasks
                .into_iter()
                .map(|task| usize::from(task.join().unwrap()))
                .sum::<usize>()
        });
        assert_eq!(successes, MAX_TOTAL_BODY / MAX_BODY);
        assert_eq!(total.load(Ordering::Acquire), MAX_TOTAL_BODY);
        assert!(!reserve_response_bytes(&total, usize::MAX));
    }

    #[tokio::test]
    #[ignore = "requires NOMIFUN_SEARCH_CHROME pointing to an installed Chromium binary"]
    async fn product_binding_is_checked_before_any_page_request() {
        let chrome =
            PathBuf::from(std::env::var_os("NOMIFUN_SEARCH_CHROME").expect("Chromium test binary"));
        let product = probe_runtime(chrome.clone()).await.unwrap();
        assert!(!product.is_empty());
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", server.local_addr().unwrap());
        let result = extract_page(
            chrome,
            PageRequest {
                url: Url::parse(&format!("{origin}/")).unwrap(),
                allowed_origins: BTreeSet::from([origin]),
                purpose: PagePurpose::Search,
                extraction: "({state:'ready'})",
                language: "en-US".into(),
                expected_browser_product: Some(format!("{product}-changed")),
                fixture: true,
                opened_profile: None,
            },
            CancellationToken::new(),
        )
        .await;
        assert_eq!(result, Err(HeadlessPageError::BindingChanged));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), server.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn origin_policy_is_exact_and_rejects_credentials_or_scheme_changes() {
        let origins = BTreeSet::from(["https://www.bing.com".into()]);
        for denied in [
            "http://www.bing.com/",
            "https://www.bing.com:444/",
            "https://bing.com/",
            "https://www.bing.com.evil.test/",
            "https://user@www.bing.com/",
        ] {
            assert!(!allows(&origins, &Url::parse(denied).unwrap()));
        }
        assert!(allows(
            &origins,
            &Url::parse("https://www.bing.com/search?q=test").unwrap()
        ));
    }

    #[tokio::test]
    #[ignore = "requires NOMIFUN_SEARCH_CHROME pointing to an installed Chromium binary"]
    async fn isolated_headless_renders_and_cannot_connect_outside_adapter_origins() {
        let chrome =
            PathBuf::from(std::env::var_os("NOMIFUN_SEARCH_CHROME").expect("Chromium test binary"));
        let trap = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let trap_address = trap.local_addr().unwrap();
        let udp_trap = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let udp_address = udp_trap.local_addr().unwrap();
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", server.local_addr().unwrap());
        let html = format!(
            r#"<!doctype html><body><div id='result'></div><script>
            fetch('/asset').then(r=>r.text()).then(t=>document.getElementById('result').textContent=t);
            fetch('http://{trap_address}/private').catch(()=>{{}});
            new WebSocket('ws://{trap_address}/escape');
            window.open('http://{trap_address}/popup');
            const pc=new RTCPeerConnection({{iceServers:[{{urls:'stun:{udp_address}'}},{{urls:'turn:{trap_address}?transport=tcp',username:'fixture',credential:'fixture'}}]}});
            pc.createDataChannel('probe');
            pc.createOffer().then(offer=>pc.setLocalDescription(offer)).catch(()=>{{}});
            setTimeout(()=>{{pc.close();document.body.dataset.rtcProbed='true';}},1500);
            </script>"#
        );
        let server_task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = server.accept().await {
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                let body = if String::from_utf8_lossy(&buffer[..count]).starts_with("GET /asset ") {
                    "browser-rendered"
                } else {
                    &html
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let (opened, profile) = tokio::sync::oneshot::channel();
        let output=extract_page(chrome,PageRequest {
            url:Url::parse(&format!("{origin}/")).unwrap(),allowed_origins:BTreeSet::from([origin]),language:"en-US".into(),expected_browser_product:None,fixture:true,opened_profile:Some(opened),
            purpose: PagePurpose::Search,
            extraction:"(()=>document.getElementById('result')?.textContent==='browser-rendered'&&document.body.dataset.rtcProbed==='true'?{state:'ready',rendered:true,cookies:document.cookie}:{state:'waiting'})()",
        },CancellationToken::new()).await;
        server_task.abort();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), trap.accept())
                .await
                .is_err(),
            "HTTP/WebSocket/popup escaped to another loopback origin"
        );
        let mut datagram = [0; 1024];
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                udp_trap.recv_from(&mut datagram)
            )
            .await
            .is_err(),
            "WebRTC sent a direct private-network datagram"
        );
        let output = output.expect("restricted Headless result and cleanup proof");
        assert_eq!(output["rendered"], true);
        assert_eq!(output["cookies"], "");
        assert!(
            !profile.await.unwrap().exists(),
            "search returned before deleting its exact temporary profile"
        );
    }

    #[tokio::test]
    #[ignore = "requires NOMIFUN_SEARCH_CHROME pointing to an installed Chromium binary"]
    async fn cancellation_and_caller_abort_preserve_cleanup_owner() {
        let chrome =
            PathBuf::from(std::env::var_os("NOMIFUN_SEARCH_CHROME").expect("Chromium test binary"));
        for abort_caller in [false, true] {
            let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", server.local_addr().unwrap());
            let (opened, profile) = tokio::sync::oneshot::channel();
            let cancel = CancellationToken::new();
            let caller = tokio::spawn(extract_page(
                chrome.clone(),
                PageRequest {
                    url: Url::parse(&format!("{origin}/")).unwrap(),
                    allowed_origins: BTreeSet::from([origin]),
                    purpose: PagePurpose::Search,
                    language: "en-US".into(),
                    fixture: true,
                    expected_browser_product: None,
                    opened_profile: Some(opened),
                    extraction: "({state:'waiting'})",
                },
                cancel.clone(),
            ));
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), server.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            let profile = profile.await.unwrap();
            assert!(profile.exists());
            if abort_caller {
                caller.abort();
                assert!(caller.await.unwrap_err().is_cancelled());
            } else {
                cancel.cancel();
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(5), caller)
                        .await
                        .unwrap()
                        .unwrap(),
                    Err(HeadlessPageError::Canceled)
                );
            }
            tokio::time::timeout(Duration::from_secs(5), async {
                while profile.exists() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("owned worker must settle and clean after caller abort");
        }
    }
}
