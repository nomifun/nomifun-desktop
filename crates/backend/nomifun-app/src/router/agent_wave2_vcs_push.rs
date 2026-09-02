//! Standalone, fail-closed `vcs.push` production owner.
//!
//! The central Wave 2 host deliberately does not wire this module yet. The
//! owner only pushes to an already configured local/file remote. The workspace
//! build disables git2's SSH/HTTPS features and no application-owned Git
//! credential authority exists, so authenticated transports remain explicitly
//! unavailable instead of reading process credentials or accepting secrets in
//! action input.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use git2::{ErrorCode, Oid, PushOptions, RemoteCallbacks, Repository};
use nomifun_agent_contracts::TypedResourceBinding;
use nomifun_file::{
    WORKSPACE_RESOURCE_KIND, WORKSPACE_ROOT_PARAMETER, WORKSPACE_WRITE_OPERATION,
};
use serde::{Deserialize, Serialize};

const DEFAULT_PUSH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PUSH_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const MAX_REMOTE_NAME_BYTES: usize = 256;
const MAX_REFSPEC_BYTES: usize = 1024;

#[derive(Clone)]
pub(crate) struct VcsPushOwner {
    authority: Arc<VcsPushAuthority>,
    push_lock: Arc<tokio::sync::Mutex<()>>,
    outcome_unknown: Arc<AtomicBool>,
}

#[derive(Clone)]
struct VcsPushAuthority {
    repository: RepositoryIdentity,
    timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepositoryIdentity {
    worktree_root: PathBuf,
    git_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct VcsPushRequest {
    pub principal_id: String,
    pub workspace_root: PathBuf,
    pub binding: TypedResourceBinding,
    pub remote: String,
    pub refspec: String,
    pub force: bool,
}

/// Untrusted action payload. Workspace and binding authority are injected by
/// the host when it constructs [`VcsPushRequest`], never deserialized from the
/// model-controlled payload.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VcsPushActionInput {
    pub remote: String,
    pub refspec: String,
    #[serde(default)]
    pub force: bool,
}

impl VcsPushRequest {
    pub(crate) fn from_action_input(
        principal_id: String,
        workspace_root: PathBuf,
        binding: TypedResourceBinding,
        input: VcsPushActionInput,
    ) -> Self {
        Self {
            principal_id,
            workspace_root,
            binding,
            remote: input.remote,
            refspec: input.refspec,
            force: input.force,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VcsPushReceipt {
    pub remote: String,
    pub refspec: String,
    pub source_commit: String,
    pub destination_ref: String,
    pub remote_commit_before: Option<String>,
    pub remote_commit_after: String,
    pub transferred_objects: usize,
    pub transferred_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VcsPushErrorKind {
    InvalidPayload,
    PresetResourceNotBound,
    ResourceOwnerMismatch,
    ResourceNotFound,
    CredentialAuthorityUnavailable,
    NonFastForward,
    RemoteRejected,
    CapabilityUnavailable,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VcsPushEffectDisposition {
    NotApplied,
    OutcomeUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VcsPushError {
    pub kind: VcsPushErrorKind,
    pub disposition: VcsPushEffectDisposition,
    pub code: &'static str,
    pub message: String,
}

impl VcsPushError {
    fn not_applied(
        kind: VcsPushErrorKind,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            disposition: VcsPushEffectDisposition::NotApplied,
            code,
            message: message.into(),
        }
    }

    fn outcome_unknown(message: impl Into<String>) -> Self {
        Self {
            kind: VcsPushErrorKind::OutcomeUnknown,
            disposition: VcsPushEffectDisposition::OutcomeUnknown,
            code: "CAPABILITY_UNAVAILABLE",
            message: message.into(),
        }
    }
}

impl fmt::Display for VcsPushError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for VcsPushError {}

impl VcsPushOwner {
    pub(crate) fn new(current_repository_root: impl AsRef<Path>) -> Result<Self, VcsPushError> {
        Self::with_timeout(current_repository_root, DEFAULT_PUSH_TIMEOUT)
    }

    pub(crate) fn with_timeout(
        current_repository_root: impl AsRef<Path>,
        timeout: Duration,
    ) -> Result<Self, VcsPushError> {
        if timeout.is_zero() || timeout > MAX_PUSH_TIMEOUT {
            return Err(VcsPushError::not_applied(
                VcsPushErrorKind::InvalidPayload,
                "INVALID_PAYLOAD",
                "vcs.push timeout must be between 1 millisecond and 120 seconds",
            ));
        }
        let repository = exact_repository_identity(current_repository_root.as_ref())?;
        Ok(Self {
            authority: Arc::new(VcsPushAuthority {
                repository,
                timeout,
            }),
            push_lock: Arc::new(tokio::sync::Mutex::new(())),
            outcome_unknown: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(crate) async fn push(
        &self,
        request: VcsPushRequest,
    ) -> Result<VcsPushReceipt, VcsPushError> {
        let _guard = self.push_lock.lock().await;
        if self.outcome_unknown.load(Ordering::Acquire) {
            return Err(VcsPushError::outcome_unknown(
                "vcs.push is blocked because a previous push has an unresolved outcome",
            ));
        }

        let validated = validate_request(&self.authority, request)?;
        let timeout = self.authority.timeout;
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = Arc::clone(&cancellation);
        let mut worker = tokio::task::spawn_blocking(move || {
            push_blocking(
                validated,
                worker_cancellation,
                Instant::now() + timeout,
            )
        });

        match tokio::time::timeout(timeout, &mut worker).await {
            Ok(Ok(Ok(receipt))) => Ok(receipt),
            Ok(Ok(Err(error))) => {
                if error.disposition == VcsPushEffectDisposition::OutcomeUnknown {
                    self.outcome_unknown.store(true, Ordering::Release);
                }
                Err(error)
            }
            Ok(Err(_join_error)) => {
                self.outcome_unknown.store(true, Ordering::Release);
                Err(VcsPushError::outcome_unknown(
                    "vcs.push worker terminated without a proven remote outcome",
                ))
            }
            Err(_) => {
                cancellation.store(true, Ordering::Release);
                self.outcome_unknown.store(true, Ordering::Release);
                Err(VcsPushError::outcome_unknown(format!(
                    "vcs.push exceeded its {} ms deadline; the remote outcome requires reconciliation",
                    timeout.as_millis()
                )))
            }
        }
    }
}

struct ValidatedPush {
    repository: RepositoryIdentity,
    remote: String,
    refspec: ParsedRefspec,
    remote_repository_root: PathBuf,
}

#[derive(Clone)]
struct ParsedRefspec {
    canonical: String,
    source: String,
    destination: String,
}

fn validate_request(
    authority: &VcsPushAuthority,
    request: VcsPushRequest,
) -> Result<ValidatedPush, VcsPushError> {
    if request.principal_id.trim().is_empty()
        || request.principal_id.chars().any(char::is_control)
    {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::InvalidPayload,
            "INVALID_PAYLOAD",
            "vcs.push requires a non-empty authenticated principal",
        ));
    }
    if request.force {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::InvalidPayload,
            "INVALID_PAYLOAD",
            "vcs.push does not allow force pushes",
        ));
    }

    let requested_repository = exact_repository_identity(&request.workspace_root)?;
    if requested_repository != authority.repository {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::PresetResourceNotBound,
            "PRESET_RESOURCE_NOT_BOUND",
            "vcs.push workspace is not the repository owned by the current host binding",
        ));
    }
    validate_binding(&request.principal_id, authority, &request.binding)?;

    let remote_name = request.remote.trim();
    if remote_name != request.remote
        || remote_name.is_empty()
        || remote_name.len() > MAX_REMOTE_NAME_BYTES
        || remote_name.chars().any(char::is_control)
        || !git2::Remote::is_valid_name(remote_name)
    {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::InvalidPayload,
            "INVALID_PAYLOAD",
            "vcs.push remote must be one configured Git remote name",
        ));
    }

    let refspec = parse_refspec(&request.refspec)?;
    let repository = Repository::open(&requested_repository.worktree_root).map_err(|error| {
        repository_error(
            VcsPushErrorKind::ResourceNotFound,
            "RESOURCE_NOT_FOUND",
            "vcs.push could not open the bound repository",
            &error,
        )
    })?;
    ensure_repository_identity(&repository, &requested_repository)?;
    let remote = repository.find_remote(remote_name).map_err(|error| {
        if error.code() == ErrorCode::NotFound {
            VcsPushError::not_applied(
                VcsPushErrorKind::ResourceNotFound,
                "RESOURCE_NOT_FOUND",
                "vcs.push remote is not configured in the bound repository",
            )
        } else {
            repository_error(
                VcsPushErrorKind::CapabilityUnavailable,
                "CAPABILITY_UNAVAILABLE",
                "vcs.push could not load the configured remote",
                &error,
            )
        }
    })?;
    let push_url = remote
        .pushurl_bytes()
        .unwrap_or_else(|| remote.url_bytes());
    let push_url = std::str::from_utf8(push_url).map_err(|_| {
        VcsPushError::not_applied(
            VcsPushErrorKind::CapabilityUnavailable,
            "CAPABILITY_UNAVAILABLE",
            "vcs.push remote URL is not valid UTF-8",
        )
    })?;
    let remote_repository_root = configured_local_remote(push_url)?;

    Ok(ValidatedPush {
        repository: requested_repository,
        remote: remote_name.to_owned(),
        refspec,
        remote_repository_root,
    })
}

fn validate_binding(
    principal_id: &str,
    authority: &VcsPushAuthority,
    binding: &TypedResourceBinding,
) -> Result<(), VcsPushError> {
    if binding.resource_kind.as_ref() != WORKSPACE_RESOURCE_KIND
        || binding.binding_id.as_ref().trim().is_empty()
        || binding.resource_id.as_ref().trim().is_empty()
    {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::PresetResourceNotBound,
            "PRESET_RESOURCE_NOT_BOUND",
            "vcs.push requires one identified workspace resource binding",
        ));
    }
    if binding.owner_id != principal_id {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::ResourceOwnerMismatch,
            "RESOURCE_OWNER_MISMATCH",
            "vcs.push workspace binding belongs to a different principal",
        ));
    }
    if !binding.operations.contains(WORKSPACE_WRITE_OPERATION) {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::PresetResourceNotBound,
            "PRESET_RESOURCE_NOT_BOUND",
            "vcs.push workspace binding does not grant write",
        ));
    }
    if binding.connection_config_ref.is_some() {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::CredentialAuthorityUnavailable,
            "CAPABILITY_UNAVAILABLE_ON_PLATFORM",
            "vcs.push cannot consume a connection credential until a Git credential authority is wired",
        ));
    }
    if binding
        .typed_parameters
        .keys()
        .any(|key| key != WORKSPACE_ROOT_PARAMETER)
    {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::InvalidPayload,
            "INVALID_PAYLOAD",
            "vcs.push binding accepts only workspace_root; credentials and secret parameters are forbidden",
        ));
    }
    let binding_root = binding
        .typed_parameters
        .get(WORKSPACE_ROOT_PARAMETER)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            VcsPushError::not_applied(
                VcsPushErrorKind::PresetResourceNotBound,
                "PRESET_RESOURCE_NOT_BOUND",
                "vcs.push workspace binding has no host-resolved workspace_root",
            )
        })?;
    let binding_root = canonical_absolute_directory(
        Path::new(binding_root),
        "vcs.push binding workspace_root is unavailable",
    )?;
    if binding_root != authority.repository.worktree_root {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::PresetResourceNotBound,
            "PRESET_RESOURCE_NOT_BOUND",
            "vcs.push binding resolves to a different repository",
        ));
    }
    Ok(())
}

fn parse_refspec(value: &str) -> Result<ParsedRefspec, VcsPushError> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > MAX_REFSPEC_BYTES
        || value.chars().any(char::is_control)
        || value.starts_with('+')
    {
        return Err(invalid_refspec());
    }
    let Some((source, destination)) = value.split_once(':') else {
        return Err(invalid_refspec());
    };
    if source.is_empty()
        || destination.is_empty()
        || destination.contains(':')
        || source.contains('*')
        || destination.contains('*')
        || (source != "HEAD"
            && (!source.starts_with("refs/heads/")
                || !git2::Reference::is_valid_name(source)))
        || !destination.starts_with("refs/heads/")
        || !git2::Reference::is_valid_name(destination)
    {
        return Err(invalid_refspec());
    }
    Ok(ParsedRefspec {
        canonical: value.to_owned(),
        source: source.to_owned(),
        destination: destination.to_owned(),
    })
}

fn invalid_refspec() -> VcsPushError {
    VcsPushError::not_applied(
        VcsPushErrorKind::InvalidPayload,
        "INVALID_PAYLOAD",
        "vcs.push requires one explicit non-force branch refspec source:destination",
    )
}

fn exact_repository_identity(root: &Path) -> Result<RepositoryIdentity, VcsPushError> {
    let worktree_root = canonical_absolute_directory(
        root,
        "vcs.push workspace repository is unavailable",
    )?;
    let repository = Repository::discover(&worktree_root).map_err(|error| {
        repository_error(
            VcsPushErrorKind::ResourceNotFound,
            "RESOURCE_NOT_FOUND",
            "vcs.push workspace is not a Git repository",
            &error,
        )
    })?;
    let identity = repository_identity(&repository)?;
    if identity.worktree_root != worktree_root {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::PresetResourceNotBound,
            "PRESET_RESOURCE_NOT_BOUND",
            "vcs.push requires the binding to identify the exact repository root",
        ));
    }
    Ok(identity)
}

fn repository_identity(repository: &Repository) -> Result<RepositoryIdentity, VcsPushError> {
    let worktree = repository.workdir().ok_or_else(|| {
        VcsPushError::not_applied(
            VcsPushErrorKind::ResourceNotFound,
            "RESOURCE_NOT_FOUND",
            "vcs.push requires a non-bare bound workspace repository",
        )
    })?;
    let worktree_root = canonical_absolute_directory(
        worktree,
        "vcs.push repository worktree is unavailable",
    )?;
    let git_dir = canonical_absolute_directory(
        repository.path(),
        "vcs.push repository metadata directory is unavailable",
    )?;
    Ok(RepositoryIdentity {
        worktree_root,
        git_dir,
    })
}

fn ensure_repository_identity(
    repository: &Repository,
    expected: &RepositoryIdentity,
) -> Result<(), VcsPushError> {
    if repository_identity(repository)? != *expected {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::PresetResourceNotBound,
            "PRESET_RESOURCE_NOT_BOUND",
            "vcs.push repository identity changed after authority resolution",
        ));
    }
    Ok(())
}

fn canonical_absolute_directory(
    path: &Path,
    message: &'static str,
) -> Result<PathBuf, VcsPushError> {
    if !path.is_absolute() {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::PresetResourceNotBound,
            "PRESET_RESOURCE_NOT_BOUND",
            "vcs.push workspace paths must be absolute host-resolved paths",
        ));
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| {
        VcsPushError::not_applied(
            VcsPushErrorKind::ResourceNotFound,
            "RESOURCE_NOT_FOUND",
            message,
        )
    })?;
    if !canonical.is_dir() {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::ResourceNotFound,
            "RESOURCE_NOT_FOUND",
            message,
        ));
    }
    Ok(canonical)
}

fn configured_local_remote(push_url: &str) -> Result<PathBuf, VcsPushError> {
    if push_url.is_empty() || push_url.chars().any(char::is_control) {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::CapabilityUnavailable,
            "CAPABILITY_UNAVAILABLE",
            "vcs.push remote has no usable push URL",
        ));
    }

    let direct_path = Path::new(push_url);
    let path = if direct_path.is_absolute() {
        direct_path.to_path_buf()
    } else {
        let parsed = url::Url::parse(push_url).map_err(|_| {
            VcsPushError::not_applied(
                VcsPushErrorKind::CapabilityUnavailable,
                "CAPABILITY_UNAVAILABLE",
                "vcs.push local remote path must be absolute",
            )
        })?;
        if parsed.scheme() != "file" {
            return Err(VcsPushError::not_applied(
                VcsPushErrorKind::CredentialAuthorityUnavailable,
                "CAPABILITY_UNAVAILABLE_ON_PLATFORM",
                "vcs.push SSH/HTTPS transports require an application-owned Git credential authority",
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(VcsPushError::not_applied(
                VcsPushErrorKind::InvalidPayload,
                "INVALID_PAYLOAD",
                "vcs.push forbids credentials embedded in remote URLs",
            ));
        }
        parsed.to_file_path().map_err(|_| {
            VcsPushError::not_applied(
                VcsPushErrorKind::CapabilityUnavailable,
                "CAPABILITY_UNAVAILABLE",
                "vcs.push file remote URL cannot be resolved on this host",
            )
        })?
    };

    let canonical = std::fs::canonicalize(path).map_err(|_| {
        VcsPushError::not_applied(
            VcsPushErrorKind::ResourceNotFound,
            "RESOURCE_NOT_FOUND",
            "vcs.push configured remote repository is unavailable",
        )
    })?;
    Repository::open(&canonical).map_err(|error| {
        repository_error(
            VcsPushErrorKind::ResourceNotFound,
            "RESOURCE_NOT_FOUND",
            "vcs.push configured local remote is not a Git repository",
            &error,
        )
    })?;
    Ok(canonical)
}

fn push_blocking(
    request: ValidatedPush,
    cancellation: Arc<AtomicBool>,
    deadline: Instant,
) -> Result<VcsPushReceipt, VcsPushError> {
    if cancellation.load(Ordering::Acquire) || Instant::now() >= deadline {
        return Err(VcsPushError::not_applied(
            VcsPushErrorKind::CapabilityUnavailable,
            "CAPABILITY_UNAVAILABLE",
            "vcs.push deadline elapsed before remote execution",
        ));
    }

    let repository = Repository::open(&request.repository.worktree_root).map_err(|error| {
        repository_error(
            VcsPushErrorKind::ResourceNotFound,
            "RESOURCE_NOT_FOUND",
            "vcs.push could not reopen the bound repository",
            &error,
        )
    })?;
    ensure_repository_identity(&repository, &request.repository)?;
    let source_commit = repository
        .revparse_single(&request.refspec.source)
        .and_then(|object| object.peel_to_commit())
        .map_err(|error| {
            repository_error(
                VcsPushErrorKind::ResourceNotFound,
                "RESOURCE_NOT_FOUND",
                "vcs.push source branch does not resolve to a commit",
                &error,
            )
        })?
        .id();

    let remote_repository = Repository::open(&request.remote_repository_root).map_err(|error| {
        repository_error(
            VcsPushErrorKind::ResourceNotFound,
            "RESOURCE_NOT_FOUND",
            "vcs.push configured local remote is unavailable",
            &error,
        )
    })?;
    let remote_before = remote_reference_commit(&remote_repository, &request.refspec.destination)?;
    if let Some(remote_before) = remote_before {
        if remote_before != source_commit {
            match repository.graph_descendant_of(source_commit, remote_before) {
                Ok(true) => {}
                Ok(false) => return Err(non_fast_forward()),
                Err(error) if error.code() == ErrorCode::NotFound => {}
                Err(error) => {
                    return Err(repository_error(
                        VcsPushErrorKind::CapabilityUnavailable,
                        "CAPABILITY_UNAVAILABLE",
                        "vcs.push could not verify fast-forward ancestry",
                        &error,
                    ));
                }
            }
        }
    }
    drop(remote_repository);

    let observation = Arc::new(Mutex::new(PushObservation::default()));
    let mut callbacks = RemoteCallbacks::new();
    {
        let observation = Arc::clone(&observation);
        let destination = request.refspec.destination.clone();
        callbacks.push_update_reference(move |reference, status| {
            let mut observation = observation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if reference == destination {
                observation.destination_seen = true;
                observation.rejection = status.map(classify_rejection);
            } else {
                observation.unexpected_reference = true;
            }
            Ok(())
        });
    }
    {
        let observation = Arc::clone(&observation);
        callbacks.push_transfer_progress(move |current, _total, bytes| {
            let mut observation = observation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            observation.transferred_objects = current;
            observation.transferred_bytes = bytes;
        });
    }
    {
        let cancellation = Arc::clone(&cancellation);
        callbacks.push_negotiation(move |_updates| {
            if cancellation.load(Ordering::Acquire) || Instant::now() >= deadline {
                Err(git2::Error::from_str(
                    "vcs.push cancelled before the remote update",
                ))
            } else {
                Ok(())
            }
        });
    }
    {
        let cancellation = Arc::clone(&cancellation);
        callbacks.sideband_progress(move |_message| !cancellation.load(Ordering::Acquire));
    }

    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);
    let mut remote = repository.find_remote(&request.remote).map_err(|error| {
        if error.code() == ErrorCode::NotFound {
            VcsPushError::not_applied(
                VcsPushErrorKind::ResourceNotFound,
                "RESOURCE_NOT_FOUND",
                "vcs.push remote disappeared before execution",
            )
        } else {
            repository_error(
                VcsPushErrorKind::CapabilityUnavailable,
                "CAPABILITY_UNAVAILABLE",
                "vcs.push could not reopen the configured remote",
                &error,
            )
        }
    })?;
    if let Err(error) = remote.push(&[request.refspec.canonical.as_str()], Some(&mut options)) {
        if error.code() == ErrorCode::NotFastForward {
            return Err(non_fast_forward());
        }
        return Err(VcsPushError::outcome_unknown(format!(
            "vcs.push transport failed without a proven remote outcome ({:?}/{:?})",
            error.class(),
            error.code()
        )));
    }
    drop(remote);
    drop(options);

    let observation = observation
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(rejection) = observation.rejection {
        return Err(match rejection {
            PushRejection::NonFastForward => non_fast_forward(),
            PushRejection::Other => VcsPushError::not_applied(
                VcsPushErrorKind::RemoteRejected,
                "CAPABILITY_UNAVAILABLE",
                "vcs.push was rejected by the configured remote",
            ),
        });
    }
    if !observation.destination_seen || observation.unexpected_reference {
        return Err(VcsPushError::outcome_unknown(
            "vcs.push completed without one unambiguous destination acknowledgement",
        ));
    }

    let remote_repository = Repository::open(&request.remote_repository_root).map_err(|_| {
        VcsPushError::outcome_unknown(
            "vcs.push remote acknowledged the update but post-push verification is unavailable",
        )
    })?;
    let remote_after =
        remote_reference_commit(&remote_repository, &request.refspec.destination).map_err(
            |_| {
                VcsPushError::outcome_unknown(
                    "vcs.push remote acknowledged the update but its destination ref cannot be verified",
                )
            },
        )?;
    if remote_after != Some(source_commit) {
        return Err(VcsPushError::outcome_unknown(
            "vcs.push remote acknowledgement does not match the observed destination ref",
        ));
    }

    Ok(VcsPushReceipt {
        remote: request.remote,
        refspec: request.refspec.canonical,
        source_commit: source_commit.to_string(),
        destination_ref: request.refspec.destination,
        remote_commit_before: remote_before.map(|oid| oid.to_string()),
        remote_commit_after: source_commit.to_string(),
        transferred_objects: observation.transferred_objects,
        transferred_bytes: observation.transferred_bytes,
    })
}

#[derive(Clone, Default)]
struct PushObservation {
    destination_seen: bool,
    unexpected_reference: bool,
    rejection: Option<PushRejection>,
    transferred_objects: usize,
    transferred_bytes: usize,
}

#[derive(Clone, Copy)]
enum PushRejection {
    NonFastForward,
    Other,
}

fn classify_rejection(status: &str) -> PushRejection {
    let status = status.to_ascii_lowercase();
    if status.contains("non-fast-forward")
        || status.contains("non fast forward")
        || status.contains("fetch first")
        || status.contains("stale info")
    {
        PushRejection::NonFastForward
    } else {
        PushRejection::Other
    }
}

fn remote_reference_commit(
    repository: &Repository,
    reference: &str,
) -> Result<Option<Oid>, VcsPushError> {
    match repository.find_reference(reference) {
        Ok(reference) => reference
            .peel_to_commit()
            .map(|commit| Some(commit.id()))
            .map_err(|error| {
                repository_error(
                    VcsPushErrorKind::CapabilityUnavailable,
                    "CAPABILITY_UNAVAILABLE",
                    "vcs.push destination ref is not a commit",
                    &error,
                )
            }),
        Err(error) if error.code() == ErrorCode::NotFound => Ok(None),
        Err(error) => Err(repository_error(
            VcsPushErrorKind::CapabilityUnavailable,
            "CAPABILITY_UNAVAILABLE",
            "vcs.push could not inspect the destination ref",
            &error,
        )),
    }
}

fn non_fast_forward() -> VcsPushError {
    VcsPushError::not_applied(
        VcsPushErrorKind::NonFastForward,
        "CAPABILITY_UNAVAILABLE",
        "vcs.push rejected a non-fast-forward update",
    )
}

fn repository_error(
    kind: VcsPushErrorKind,
    code: &'static str,
    message: &'static str,
    error: &git2::Error,
) -> VcsPushError {
    VcsPushError::not_applied(
        kind,
        code,
        format!("{message} ({:?}/{:?})", error.class(), error.code()),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        ConnectionConfigRef, ResourceBindingId, ResourceId, ResourceKind,
    };
    use serde_json::json;

    use super::*;

    struct PushFixture {
        _directory: tempfile::TempDir,
        worktree: PathBuf,
        remote: PathBuf,
        repository: Repository,
        initial_commit: Oid,
    }

    impl PushFixture {
        fn new(with_remote: bool) -> Self {
            let directory = tempfile::tempdir().expect("fixture root");
            let worktree = directory.path().join("worktree");
            let remote = directory.path().join("remote.git");
            std::fs::create_dir(&worktree).expect("worktree directory");
            Repository::init_bare(&remote).expect("bare remote");
            let repository = Repository::init(&worktree).expect("worktree repository");
            let initial_commit = commit(&repository, None, "initial", "initial\n");
            repository
                .set_head("refs/heads/main")
                .expect("select main branch");
            if with_remote {
                repository
                    .remote(
                        "origin",
                        remote.to_str().expect("UTF-8 fixture remote path"),
                    )
                    .expect("configure origin");
            }
            Self {
                _directory: directory,
                worktree,
                remote,
                repository,
                initial_commit,
            }
        }

        fn owner(&self) -> VcsPushOwner {
            VcsPushOwner::new(&self.worktree).expect("push owner")
        }

        fn request(&self) -> VcsPushRequest {
            VcsPushRequest::from_action_input(
                "owner-1".to_owned(),
                self.worktree.clone(),
                workspace_binding(&self.worktree),
                VcsPushActionInput {
                    remote: "origin".to_owned(),
                    refspec: "refs/heads/main:refs/heads/main".to_owned(),
                    force: false,
                },
            )
        }

        fn remote_main(&self) -> Option<Oid> {
            Repository::open_bare(&self.remote)
                .expect("open bare remote")
                .find_reference("refs/heads/main")
                .ok()
                .and_then(|reference| reference.target())
        }
    }

    fn workspace_binding(root: &Path) -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: ResourceBindingId::from("workspace-binding"),
            resource_kind: ResourceKind::from(WORKSPACE_RESOURCE_KIND),
            resource_id: ResourceId::from("workspace-resource"),
            owner_id: "owner-1".to_owned(),
            operations: BTreeSet::from([
                "read".to_owned(),
                WORKSPACE_WRITE_OPERATION.to_owned(),
            ]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([(
                WORKSPACE_ROOT_PARAMETER.to_owned(),
                root.to_string_lossy().into_owned(),
            )]),
        }
    }

    fn commit(
        repository: &Repository,
        parent: Option<Oid>,
        message: &str,
        content: &str,
    ) -> Oid {
        let worktree = repository.workdir().expect("worktree");
        std::fs::write(worktree.join("tracked.txt"), content).expect("write fixture file");
        let mut index = repository.index().expect("index");
        index
            .add_path(Path::new("tracked.txt"))
            .expect("stage fixture file");
        index.write().expect("persist fixture index");
        let tree_id = index.write_tree().expect("write fixture tree");
        let tree = repository.find_tree(tree_id).expect("fixture tree");
        let signature =
            git2::Signature::now("NomiFun test", "test@nomifun.invalid").expect("signature");
        let parent = parent.map(|oid| repository.find_commit(oid).expect("parent commit"));
        let parents = parent.iter().collect::<Vec<_>>();
        repository
            .commit(
                Some("refs/heads/main"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .expect("fixture commit")
    }

    #[tokio::test]
    async fn pushes_to_a_real_local_bare_remote() {
        let fixture = PushFixture::new(true);

        let receipt = fixture
            .owner()
            .push(fixture.request())
            .await
            .expect("real local push");

        assert_eq!(fixture.remote_main(), Some(fixture.initial_commit));
        assert_eq!(receipt.remote, "origin");
        assert_eq!(
            receipt.refspec,
            "refs/heads/main:refs/heads/main"
        );
        assert_eq!(receipt.source_commit, fixture.initial_commit.to_string());
        assert_eq!(receipt.remote_commit_before, None);
        assert_eq!(
            receipt.remote_commit_after,
            fixture.initial_commit.to_string()
        );
    }

    #[tokio::test]
    async fn rejects_a_binding_for_another_repository() {
        let fixture = PushFixture::new(true);
        let other = tempfile::tempdir().expect("other repository root");
        Repository::init(other.path()).expect("other repository");
        let mut request = fixture.request();
        request.binding = workspace_binding(other.path());

        let error = fixture
            .owner()
            .push(request)
            .await
            .expect_err("wrong binding must fail");

        assert_eq!(error.kind, VcsPushErrorKind::PresetResourceNotBound);
        assert_eq!(error.disposition, VcsPushEffectDisposition::NotApplied);
        assert_eq!(fixture.remote_main(), None);
    }

    #[tokio::test]
    async fn rejects_force_before_contacting_the_remote() {
        let fixture = PushFixture::new(true);
        let mut request = fixture.request();
        request.force = true;

        let error = fixture
            .owner()
            .push(request)
            .await
            .expect_err("force must fail");

        assert_eq!(error.kind, VcsPushErrorKind::InvalidPayload);
        assert_eq!(error.disposition, VcsPushEffectDisposition::NotApplied);
        assert_eq!(fixture.remote_main(), None);
    }

    #[tokio::test]
    async fn reports_a_missing_configured_remote_without_push() {
        let fixture = PushFixture::new(false);

        let error = fixture
            .owner()
            .push(fixture.request())
            .await
            .expect_err("missing remote must fail");

        assert_eq!(error.kind, VcsPushErrorKind::ResourceNotFound);
        assert_eq!(error.disposition, VcsPushEffectDisposition::NotApplied);
        assert_eq!(fixture.remote_main(), None);
    }

    #[tokio::test]
    async fn rejects_a_real_non_fast_forward_update() {
        let fixture = PushFixture::new(true);
        let owner = fixture.owner();
        owner
            .push(fixture.request())
            .await
            .expect("initial push");

        let remote_head = commit(
            &fixture.repository,
            Some(fixture.initial_commit),
            "remote head",
            "remote head\n",
        );
        owner
            .push(fixture.request())
            .await
            .expect("fast-forward push");
        assert_eq!(fixture.remote_main(), Some(remote_head));

        fixture
            .repository
            .reference(
                "refs/heads/main",
                fixture.initial_commit,
                true,
                "rewind test branch before creating a divergence",
            )
            .expect("rewind local branch");
        let divergent = commit(
            &fixture.repository,
            Some(fixture.initial_commit),
            "divergent",
            "divergent\n",
        );
        assert_ne!(divergent, remote_head);
        let error = owner
            .push(fixture.request())
            .await
            .expect_err("non-fast-forward must fail");

        assert_eq!(error.kind, VcsPushErrorKind::NonFastForward);
        assert_eq!(error.disposition, VcsPushEffectDisposition::NotApplied);
        assert_eq!(fixture.remote_main(), Some(remote_head));
    }

    #[test]
    fn request_shape_rejects_secret_fields() {
        let value = json!({
            "remote": "origin",
            "refspec": "refs/heads/main:refs/heads/main",
            "force": false,
            "password": "must-not-be-consumed"
        });
        let error = serde_json::from_value::<VcsPushActionInput>(value)
            .expect_err("secret field must fail");

        assert!(error.to_string().contains("unknown field"));
        assert!(!error.to_string().contains("must-not-be-consumed"));
    }

    #[tokio::test]
    async fn credential_handles_remain_fail_closed() {
        let fixture = PushFixture::new(true);
        let mut request = fixture.request();
        request.binding.connection_config_ref =
            Some(ConnectionConfigRef::from("credential-handle"));

        let error = fixture
            .owner()
            .push(request)
            .await
            .expect_err("credential authority is not wired");

        assert_eq!(
            error.kind,
            VcsPushErrorKind::CredentialAuthorityUnavailable
        );
        assert_eq!(error.disposition, VcsPushEffectDisposition::NotApplied);
        assert_eq!(fixture.remote_main(), None);
    }

    #[tokio::test]
    async fn request_workspace_cannot_escape_the_current_repository() {
        let fixture = PushFixture::new(true);
        let other = PushFixture::new(true);
        let mut request = other.request();
        request.binding = workspace_binding(&other.worktree);

        let error = fixture
            .owner()
            .push(request)
            .await
            .expect_err("another repository must fail");

        assert_eq!(error.kind, VcsPushErrorKind::PresetResourceNotBound);
        assert_eq!(error.disposition, VcsPushEffectDisposition::NotApplied);
        assert_eq!(fixture.remote_main(), None);
        assert_eq!(other.remote_main(), None);
    }

    #[tokio::test]
    async fn network_remote_reports_the_missing_credential_authority_without_a_request() {
        let fixture = PushFixture::new(false);
        fixture
            .repository
            .remote("origin", "https://user:secret@example.invalid/repository.git")
            .expect("configure network remote");

        let error = fixture
            .owner()
            .push(fixture.request())
            .await
            .expect_err("network credentials are unavailable");

        assert_eq!(
            error.kind,
            VcsPushErrorKind::CredentialAuthorityUnavailable
        );
        assert_eq!(error.disposition, VcsPushEffectDisposition::NotApplied);
        assert!(!error.message.contains("user"));
        assert!(!error.message.contains("secret"));
    }
}
