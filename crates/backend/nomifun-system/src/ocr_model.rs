//! Managed, opt-in OCR artifact control plane.
//!
//! This module deliberately owns only artifact installation. It never starts
//! an OCR runtime and never downloads at application startup. The two official
//! PP-OCRv6 Small ONNX artifacts are pinned by immutable revision, size and
//! SHA-256 and are exposed through a path/URL-free API contract.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use nomifun_api_types::{
    LocalModelErrorKind, LocalModelInstallPhase, OcrModelCatalogEntry, OcrModelComponent,
    OcrModelServiceStatus, OcrModelState, OcrModelTransferProgress,
};
use nomifun_common::AppError;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const OCR_PROTOCOL_VERSION: &str = "1";
const OCR_ROOT_DIR: &str = "local-ai";
const OCR_SUBDIR: &str = "ocr";
pub const PP_OCRV6_SMALL_MODEL_ID: &str = "pp-ocrv6-small-onnx";
const DOWNLOAD_PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy)]
struct OcrArtifact {
    component: OcrModelComponent,
    file_name: &'static str,
    url: &'static str,
    revision: &'static str,
    size: u64,
    sha256: &'static str,
}

// Official PaddlePaddle ONNX exports. Keep these revisions immutable: changing
// any value is a catalog migration, not a transparent update.
const OCR_ARTIFACTS: [OcrArtifact; 4] = [
    OcrArtifact {
        component: OcrModelComponent::Detector,
        file_name: "detector.onnx",
        url: "https://huggingface.co/PaddlePaddle/PP-OCRv6_small_det_onnx/resolve/28fe5895c24fd108c19eb3e8479f4ab385fbfc62/inference.onnx",
        revision: "28fe5895c24fd108c19eb3e8479f4ab385fbfc62",
        size: 9_880_512,
        sha256: "d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e",
    },
    OcrArtifact {
        component: OcrModelComponent::DetectorConfig,
        file_name: "detector.yml",
        url: "https://huggingface.co/PaddlePaddle/PP-OCRv6_small_det_onnx/resolve/28fe5895c24fd108c19eb3e8479f4ab385fbfc62/inference.yml",
        revision: "28fe5895c24fd108c19eb3e8479f4ab385fbfc62",
        size: 885,
        sha256: "193f435274bf9f0b5f71a929bbfbcf148282df7e633b34e7c373e8f44741b516",
    },
    OcrArtifact {
        component: OcrModelComponent::Recognizer,
        file_name: "recognizer.onnx",
        url: "https://huggingface.co/PaddlePaddle/PP-OCRv6_small_rec_onnx/resolve/b8f84f0b80c529de40b4fbb3544b84fa7233a513/inference.onnx",
        revision: "b8f84f0b80c529de40b4fbb3544b84fa7233a513",
        size: 21_159_378,
        sha256: "5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634",
    },
    OcrArtifact {
        component: OcrModelComponent::RecognizerConfig,
        file_name: "recognizer.yml",
        url: "https://huggingface.co/PaddlePaddle/PP-OCRv6_small_rec_onnx/resolve/b8f84f0b80c529de40b4fbb3544b84fa7233a513/inference.yml",
        revision: "b8f84f0b80c529de40b4fbb3544b84fa7233a513",
        size: 150_579,
        sha256: "ab078671bb49f06228eadccd34f1bb501e157f7a047095ffb943ba81512c77d1",
    },
];

const OCR_TOTAL_SIZE: u64 = 9_880_512 + 885 + 21_159_378 + 150_579;

#[derive(Debug)]
struct ActiveDownload {
    generation: u64,
    cancel: CancellationToken,
    done: Arc<Notify>,
}

#[derive(Debug)]
struct MutableState {
    model: OcrModelState,
    download: Option<ActiveDownload>,
    next_generation: u64,
    last_error: Option<String>,
}

#[derive(Debug)]
struct OcrFailure {
    kind: LocalModelErrorKind,
    safe_message: &'static str,
    detail: String,
    cancelled: bool,
}

impl OcrFailure {
    fn new(
        kind: LocalModelErrorKind,
        safe_message: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            safe_message,
            detail: detail.into(),
            cancelled: false,
        }
    }

    fn cancelled() -> Self {
        Self {
            kind: LocalModelErrorKind::Unknown,
            safe_message: "OCR model download is paused.",
            detail: "download cancelled by user".into(),
            cancelled: true,
        }
    }
}

/// One-click OCR artifact manager. It is cheap to construct and performs no
/// network requests until `install` or `resume` is called explicitly.
pub struct OcrModelService {
    root: PathBuf,
    http_client: reqwest::Client,
    state: Mutex<MutableState>,
    mutation_lock: Mutex<()>,
}

impl OcrModelService {
    pub async fn new(data_dir: impl AsRef<Path>) -> Result<Arc<Self>, AppError> {
        // Anchor safety checks at `local-ai`, not at its `ocr` child. This
        // rejects a pre-positioned link/reparse point on the shared parent.
        let root = data_dir.as_ref().join(OCR_ROOT_DIR);
        let model_dir = model_dir(&root);
        prepare_managed_directory(&root, &root)
            .and_then(|_| prepare_managed_directory(&root, &root.join(OCR_SUBDIR)))
            .and_then(|_| prepare_managed_directory(&root, &model_dir))
            .map_err(|error| AppError::Internal(format!("prepare OCR model directory: {error}")))?;
        for artifact in &OCR_ARTIFACTS {
            let path = artifact_path(&root, artifact);
            prepare_managed_file(&root, &path)
                .and_then(|_| prepare_managed_file(&root, &partial_path(&path)))
                .map_err(|error| {
                    AppError::Internal(format!("validate OCR artifact path: {error}"))
                })?;
        }

        let model = inspect_model_state(&root).await;
        Ok(Arc::new(Self {
            root,
            http_client: ocr_download_client(),
            state: Mutex::new(MutableState {
                last_error: model
                    .error_kind
                    .is_some()
                    .then(|| "OCR model files need repair before they can be used.".into()),
                model,
                download: None,
                next_generation: 0,
            }),
            mutation_lock: Mutex::new(()),
        }))
    }

    pub async fn catalog(&self) -> Vec<OcrModelCatalogEntry> {
        vec![catalog_entry()]
    }

    pub async fn status(&self) -> OcrModelServiceStatus {
        let state = self.state.lock().await;
        snapshot(&state)
    }

    /// Start a fresh install. A paused transfer must use `resume`, making the
    /// lifecycle explicit to clients and avoiding accidental background work.
    pub async fn install(
        self: &Arc<Self>,
        model_id: &str,
    ) -> Result<OcrModelServiceStatus, AppError> {
        self.start_download(model_id, false).await
    }

    pub async fn resume(
        self: &Arc<Self>,
        model_id: &str,
    ) -> Result<OcrModelServiceStatus, AppError> {
        self.start_download(model_id, true).await
    }

    async fn start_download(
        self: &Arc<Self>,
        model_id: &str,
        resume_only: bool,
    ) -> Result<OcrModelServiceStatus, AppError> {
        validate_model_id(model_id)?;
        let _mutation = self.mutation_lock.lock().await;
        let (generation, cancel, done) = {
            let mut state = self.state.lock().await;
            if state.download.is_some() {
                return Err(AppError::Conflict(
                    "An OCR model download is already running".into(),
                ));
            }
            if state.model.install_phase == LocalModelInstallPhase::Installed {
                return Ok(snapshot(&state));
            }
            if resume_only && state.model.install_phase != LocalModelInstallPhase::Paused {
                return Err(AppError::Conflict(
                    "The OCR model does not have a paused download to resume".into(),
                ));
            }
            if !resume_only && state.model.install_phase == LocalModelInstallPhase::Paused {
                return Err(AppError::Conflict(
                    "Resume the paused OCR model download instead".into(),
                ));
            }

            state.next_generation = state.next_generation.wrapping_add(1).max(1);
            let generation = state.next_generation;
            let cancel = CancellationToken::new();
            let done = Arc::new(Notify::new());
            state.download = Some(ActiveDownload {
                generation,
                cancel: cancel.clone(),
                done: Arc::clone(&done),
            });
            state.model.install_phase = LocalModelInstallPhase::Downloading;
            state.model.error_kind = None;
            state.model.message = None;
            state.last_error = None;
            (generation, cancel, done)
        };

        let service = Arc::clone(self);
        tokio::spawn(async move {
            service.run_install(generation, cancel).await;
            done.notify_one();
        });
        Ok(self.status().await)
    }

    pub async fn pause(&self, model_id: &str) -> Result<OcrModelServiceStatus, AppError> {
        validate_model_id(model_id)?;
        let _mutation = self.mutation_lock.lock().await;
        let done = {
            let mut state = self.state.lock().await;
            let Some(active) = state.download.as_ref() else {
                return Err(AppError::Conflict(
                    "The OCR model is not currently downloading".into(),
                ));
            };
            let done = Arc::clone(&active.done);
            active.cancel.cancel();
            state.model.install_phase = LocalModelInstallPhase::Paused;
            state.model.progress = None;
            state.model.message = Some("OCR model download is paused.".into());
            done
        };
        // Do not return a resumable state until the transfer has flushed and
        // released its active-generation slot. This makes pause -> resume and
        // pause -> delete deterministic for the UI.
        done.notified().await;
        Ok(self.status().await)
    }

    pub async fn delete(&self, model_id: &str) -> Result<OcrModelServiceStatus, AppError> {
        validate_model_id(model_id)?;
        let _mutation = self.mutation_lock.lock().await;
        {
            let state = self.state.lock().await;
            if state.download.is_some() {
                return Err(AppError::Conflict(
                    "Pause the OCR model download before deleting it".into(),
                ));
            }
        }

        for artifact in &OCR_ARTIFACTS {
            let final_path = artifact_path(&self.root, artifact);
            let part_path = partial_path(&final_path);
            remove_managed_file(&self.root, &final_path).await?;
            remove_managed_file(&self.root, &part_path).await?;
        }

        let mut state = self.state.lock().await;
        state.model = empty_model_state();
        state.last_error = None;
        Ok(snapshot(&state))
    }

    async fn run_install(self: Arc<Self>, generation: u64, cancel: CancellationToken) {
        let result = self.install_artifacts(generation, &cancel).await;
        let installed_bytes = current_bytes(&self.root).await;
        let mut state = self.state.lock().await;
        if !state
            .download
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            return;
        }
        state.download = None;
        state.model.progress = None;
        state.model.installed_bytes = installed_bytes;

        match result {
            Ok(()) => {
                state.model.install_phase = LocalModelInstallPhase::Installed;
                state.model.installed_bytes = OCR_TOTAL_SIZE;
                state.model.error_kind = None;
                state.model.message = Some(
                    "OCR model files are installed; inference runtime is not connected yet.".into(),
                );
                state.last_error = None;
                info!(model = PP_OCRV6_SMALL_MODEL_ID, "OCR model artifacts installed");
            }
            Err(error) if error.cancelled => {
                state.model.install_phase = LocalModelInstallPhase::Paused;
                state.model.error_kind = None;
                state.model.message = Some(error.safe_message.into());
                state.last_error = None;
            }
            Err(error) => {
                warn!(
                    model = PP_OCRV6_SMALL_MODEL_ID,
                    error = %error.detail,
                    "OCR model artifact install failed"
                );
                state.model.install_phase = LocalModelInstallPhase::Failed;
                state.model.error_kind = Some(error.kind);
                state.model.message = Some(error.safe_message.into());
                state.last_error = Some(error.safe_message.into());
            }
        }
    }

    async fn install_artifacts(
        &self,
        generation: u64,
        cancel: &CancellationToken,
    ) -> Result<(), OcrFailure> {
        for artifact in &OCR_ARTIFACTS {
            debug_assert!(artifact.url.contains(artifact.revision));
            if cancel.is_cancelled() {
                return Err(OcrFailure::cancelled());
            }
            let destination = artifact_path(&self.root, artifact);
            prepare_managed_file(&self.root, &destination).map_err(storage_failure)?;
            if verified_file(&self.root, &destination, artifact, Some(cancel)).await? {
                continue;
            }
            remove_file_if_exists(&self.root, &destination).await?;

            let completed_before = completed_bytes_before(&self.root, artifact.component).await;
            let mut last_error = None;
            for source in download_sources(artifact.url) {
                match self
                    .download_once(
                        &source,
                        artifact,
                        &destination,
                        completed_before,
                        generation,
                        cancel,
                    )
                    .await
                {
                    Ok(()) => {
                        last_error = None;
                        break;
                    }
                    Err(error) if error.cancelled => return Err(error),
                    Err(error) => {
                        warn!(
                            component = ?artifact.component,
                            error = %error.detail,
                            "OCR artifact source failed"
                        );
                        last_error = Some(error);
                    }
                }
            }
            if let Some(error) = last_error {
                return Err(error);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_once(
        &self,
        url: &str,
        artifact: &OcrArtifact,
        destination: &Path,
        completed_before: u64,
        generation: u64,
        cancel: &CancellationToken,
    ) -> Result<(), OcrFailure> {
        prepare_managed_file(&self.root, destination).map_err(storage_failure)?;
        let part = partial_path(destination);
        prepare_managed_file(&self.root, &part).map_err(storage_failure)?;

        let mut offset = file_len(&part).await;
        if offset > artifact.size {
            remove_file_if_exists(&self.root, &part).await?;
            offset = 0;
        }
        if offset == artifact.size {
            self.set_verifying(generation).await;
            if hash_file(&part, cancel).await? == artifact.sha256 {
                commit_partial(&self.root, &part, destination).await?;
                return Ok(());
            }
            remove_file_if_exists(&self.root, &part).await?;
            offset = 0;
        }

        let required = artifact.size.saturating_sub(offset);
        let available = fs2::available_space(model_dir(&self.root)).map_err(|error| {
            OcrFailure::new(
                LocalModelErrorKind::Unknown,
                "Could not inspect available storage for the OCR model.",
                error.to_string(),
            )
        })?;
        if available < required {
            return Err(OcrFailure::new(
                LocalModelErrorKind::InsufficientSpace,
                "There is not enough free space to install the OCR model.",
                format!("required {required}, available {available}"),
            ));
        }
        if cancel.is_cancelled() {
            return Err(OcrFailure::cancelled());
        }

        let mut request = self.http_client.get(url);
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(OcrFailure::cancelled()),
            response = request.send() => response.map_err(|error| OcrFailure::new(
                LocalModelErrorKind::Network,
                "OCR model download failed. Check the network and try again.",
                error.to_string(),
            ))?,
        };
        if !allowed_download_url(response.url()) {
            return Err(OcrFailure::new(
                LocalModelErrorKind::Network,
                "The OCR model download source did not pass safety checks.",
                "redirected to a disallowed host",
            ));
        }

        let status = response.status();
        let mut append = false;
        if offset > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT {
            let range = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range)
                .ok_or_else(|| {
                    OcrFailure::new(
                        LocalModelErrorKind::Network,
                        "The OCR download server returned an invalid resume response.",
                        "missing or invalid Content-Range",
                    )
                })?;
            if range != (offset, artifact.size.saturating_sub(1), artifact.size) {
                return Err(OcrFailure::new(
                    LocalModelErrorKind::Network,
                    "The OCR download server returned a mismatched resume range.",
                    format!("unexpected Content-Range {range:?}"),
                ));
            }
            append = true;
        } else if offset > 0 && status.is_success() {
            // Origin ignored Range: restart rather than append a full response.
            offset = 0;
        } else if !status.is_success() {
            return Err(OcrFailure::new(
                LocalModelErrorKind::Network,
                "The OCR model download service is temporarily unavailable.",
                format!("HTTP status {status}"),
            ));
        }

        if let Some(length) = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
        {
            let expected = artifact.size.saturating_sub(offset);
            if length != expected {
                return Err(OcrFailure::new(
                    LocalModelErrorKind::Network,
                    "The OCR model download has an unexpected size.",
                    format!("Content-Length {length}, expected {expected}"),
                ));
            }
        }

        prepare_managed_file(&self.root, &part).map_err(storage_failure)?;
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).write(true);
        if append {
            options.append(true);
        }
        let mut file = options.open(&part).await.map_err(|error| {
            OcrFailure::new(
                LocalModelErrorKind::Unknown,
                "Could not write the OCR model download.",
                error.to_string(),
            )
        })?;
        prepare_managed_file(&self.root, &part).map_err(storage_failure)?;
        if !file
            .metadata()
            .await
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return Err(storage_failure(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "opened OCR partial is not a regular file",
            )));
        }
        if !append {
            file.set_len(0).await.map_err(|error| {
                OcrFailure::new(
                    LocalModelErrorKind::Unknown,
                    "Could not reset the OCR model download.",
                    error.to_string(),
                )
            })?;
        }

        let initial_offset = offset;
        let started = Instant::now();
        let mut last_report = Instant::now() - DOWNLOAD_PROGRESS_INTERVAL;
        let mut downloaded = offset;
        let mut stream = response.bytes_stream();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => {
                    file.sync_all().await.ok();
                    return Err(OcrFailure::cancelled());
                }
                next = stream.next() => next,
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|error| {
                OcrFailure::new(
                    LocalModelErrorKind::Network,
                    "OCR model download was interrupted and can be resumed.",
                    error.to_string(),
                )
            })?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            if downloaded > artifact.size {
                return Err(OcrFailure::new(
                    LocalModelErrorKind::Network,
                    "The OCR model download exceeded its expected size.",
                    "response exceeded expected size",
                ));
            }
            file.write_all(&chunk).await.map_err(|error| {
                OcrFailure::new(
                    LocalModelErrorKind::Unknown,
                    "Could not write the OCR model download.",
                    error.to_string(),
                )
            })?;
            if last_report.elapsed() >= DOWNLOAD_PROGRESS_INTERVAL {
                let seconds = started.elapsed().as_secs_f64().max(0.001);
                let rate = ((downloaded - initial_offset) as f64 / seconds) as u64;
                self.set_progress(
                    generation,
                    artifact.component,
                    downloaded,
                    artifact.size,
                    completed_before.saturating_add(downloaded),
                    rate,
                )
                .await;
                last_report = Instant::now();
            }
        }
        file.sync_all().await.map_err(|error| {
            OcrFailure::new(
                LocalModelErrorKind::Unknown,
                "Could not commit the OCR model download.",
                error.to_string(),
            )
        })?;
        drop(file);

        if downloaded != artifact.size {
            return Err(OcrFailure::new(
                LocalModelErrorKind::Network,
                "OCR model download was interrupted and can be resumed.",
                format!("downloaded {downloaded} of {}", artifact.size),
            ));
        }
        self.set_verifying(generation).await;
        let actual = hash_file(&part, cancel).await?;
        if actual != artifact.sha256 {
            remove_file_if_exists(&self.root, &part).await?;
            return Err(OcrFailure::new(
                LocalModelErrorKind::ChecksumMismatch,
                "OCR model integrity verification failed. Download it again.",
                format!("SHA-256 mismatch: expected {}, got {actual}", artifact.sha256),
            ));
        }
        commit_partial(&self.root, &part, destination).await
    }

    async fn set_progress(
        &self,
        generation: u64,
        component: OcrModelComponent,
        downloaded_bytes: u64,
        total_bytes: u64,
        overall_downloaded_bytes: u64,
        bytes_per_second: u64,
    ) {
        let mut state = self.state.lock().await;
        if !state
            .download
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            return;
        }
        state.model.install_phase = LocalModelInstallPhase::Downloading;
        state.model.installed_bytes = overall_downloaded_bytes;
        state.model.progress = Some(OcrModelTransferProgress {
            component,
            downloaded_bytes,
            total_bytes,
            overall_downloaded_bytes,
            overall_total_bytes: OCR_TOTAL_SIZE,
            bytes_per_second,
        });
    }

    async fn set_verifying(&self, generation: u64) {
        let mut state = self.state.lock().await;
        if state
            .download
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            state.model.install_phase = LocalModelInstallPhase::Verifying;
            state.model.progress = None;
        }
    }
}

fn validate_model_id(model_id: &str) -> Result<(), AppError> {
    if model_id == PP_OCRV6_SMALL_MODEL_ID {
        Ok(())
    } else {
        Err(AppError::NotFound("OCR model is not in the curated catalog".into()))
    }
}

fn catalog_entry() -> OcrModelCatalogEntry {
    OcrModelCatalogEntry {
        id: PP_OCRV6_SMALL_MODEL_ID.into(),
        name: "PP-OCRv6 Small".into(),
        description: "Lightweight ONNX text detection and recognition for local OCR.".into(),
        format: "ONNX".into(),
        download_size_bytes: OCR_TOTAL_SIZE,
        required_memory_bytes: 512 * 1024 * 1024,
        license: "Apache-2.0".into(),
        source: "PaddlePaddle / PP-OCRv6 official ONNX exports".into(),
        components: vec![
            OcrModelComponent::Detector,
            OcrModelComponent::DetectorConfig,
            OcrModelComponent::Recognizer,
            OcrModelComponent::RecognizerConfig,
        ],
        recommended: true,
    }
}

fn empty_model_state() -> OcrModelState {
    OcrModelState {
        model_id: PP_OCRV6_SMALL_MODEL_ID.into(),
        install_phase: LocalModelInstallPhase::NotInstalled,
        progress: None,
        installed_bytes: 0,
        error_kind: None,
        message: None,
    }
}

fn snapshot(state: &MutableState) -> OcrModelServiceStatus {
    let ready = state.model.install_phase == LocalModelInstallPhase::Installed;
    OcrModelServiceStatus {
        protocol_version: OCR_PROTOCOL_VERSION.into(),
        artifacts_ready: ready,
        // Do not advertise runtime readiness until preprocessing, ONNX
        // execution and postprocessing are integrated end to end.
        inference_ready: false,
        models: vec![state.model.clone()],
        last_error: state.last_error.clone(),
    }
}

async fn inspect_model_state(root: &Path) -> OcrModelState {
    let mut installed_bytes = 0;
    let mut valid_count = 0;
    let mut has_partial = false;
    let mut corrupt = false;

    for artifact in &OCR_ARTIFACTS {
        let path = artifact_path(root, artifact);
        match verified_file(root, &path, artifact, None).await {
            Ok(true) => {
                valid_count += 1;
                installed_bytes += artifact.size;
            }
            Ok(false) => {
                if file_len(&path).await > 0 {
                    corrupt = true;
                }
            }
            Err(_) => corrupt = true,
        }
        let partial_path = partial_path(&path);
        if prepare_managed_file(root, &partial_path).is_err() {
            corrupt = true;
            continue;
        }
        let partial = file_len(&partial_path).await;
        if partial > 0 {
            has_partial = true;
            installed_bytes = installed_bytes.saturating_add(partial.min(artifact.size));
        }
    }

    let install_phase = if corrupt {
        LocalModelInstallPhase::Failed
    } else if valid_count == OCR_ARTIFACTS.len() {
        LocalModelInstallPhase::Installed
    } else if has_partial || valid_count > 0 {
        LocalModelInstallPhase::Paused
    } else {
        LocalModelInstallPhase::NotInstalled
    };
    OcrModelState {
        model_id: PP_OCRV6_SMALL_MODEL_ID.into(),
        install_phase,
        progress: None,
        installed_bytes,
        error_kind: corrupt.then_some(LocalModelErrorKind::ChecksumMismatch),
        message: match install_phase {
            LocalModelInstallPhase::Installed => Some(
                "OCR model files are installed; inference runtime is not connected yet.".into(),
            ),
            LocalModelInstallPhase::Paused => Some("OCR model download can be resumed.".into()),
            LocalModelInstallPhase::Failed => {
                Some("OCR model files failed integrity checks and need repair.".into())
            }
            _ => None,
        },
    }
}

async fn verified_file(
    root: &Path,
    path: &Path,
    artifact: &OcrArtifact,
    cancel: Option<&CancellationToken>,
) -> Result<bool, OcrFailure> {
    prepare_managed_file(root, path).map_err(storage_failure)?;
    if file_len(path).await != artifact.size {
        return Ok(false);
    }
    let owned_cancel;
    let cancel = match cancel {
        Some(cancel) => cancel,
        None => {
            owned_cancel = CancellationToken::new();
            &owned_cancel
        }
    };
    Ok(hash_file(path, cancel).await? == artifact.sha256)
}

async fn hash_file(path: &Path, cancel: &CancellationToken) -> Result<String, OcrFailure> {
    let mut file = tokio::fs::File::open(path).await.map_err(|error| {
        OcrFailure::new(
            LocalModelErrorKind::Unknown,
            "Could not verify the OCR model file.",
            error.to_string(),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = tokio::select! {
            _ = cancel.cancelled() => return Err(OcrFailure::cancelled()),
            result = file.read(&mut buffer) => result.map_err(|error| OcrFailure::new(
                LocalModelErrorKind::Unknown,
                "Could not verify the OCR model file.",
                error.to_string(),
            ))?,
        };
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

async fn current_bytes(root: &Path) -> u64 {
    let mut total = 0_u64;
    for artifact in &OCR_ARTIFACTS {
        let final_path = artifact_path(root, artifact);
        let final_len = file_len(&final_path).await;
        if final_len == artifact.size {
            total = total.saturating_add(final_len);
        }
        total = total.saturating_add(
            file_len(&partial_path(&final_path))
                .await
                .min(artifact.size),
        );
    }
    total
}

async fn completed_bytes_before(root: &Path, component: OcrModelComponent) -> u64 {
    let mut total = 0_u64;
    for artifact in &OCR_ARTIFACTS {
        if artifact.component == component {
            break;
        }
        if file_len(&artifact_path(root, artifact)).await == artifact.size {
            total = total.saturating_add(artifact.size);
        }
    }
    total
}

fn model_dir(root: &Path) -> PathBuf {
    root.join(OCR_SUBDIR).join(PP_OCRV6_SMALL_MODEL_ID)
}

fn artifact_path(root: &Path, artifact: &OcrArtifact) -> PathBuf {
    model_dir(root).join(artifact.file_name)
}

fn partial_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(".part");
    PathBuf::from(name)
}

async fn file_len(path: &Path) -> u64 {
    tokio::fs::metadata(path)
        .await
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

async fn commit_partial(root: &Path, part: &Path, destination: &Path) -> Result<(), OcrFailure> {
    prepare_managed_file(root, part).map_err(storage_failure)?;
    prepare_managed_file(root, destination).map_err(storage_failure)?;
    remove_file_if_exists(root, destination).await?;
    tokio::fs::rename(part, destination).await.map_err(|error| {
        OcrFailure::new(
            LocalModelErrorKind::Unknown,
            "Could not complete the OCR model installation.",
            error.to_string(),
        )
    })
}

async fn remove_file_if_exists(root: &Path, path: &Path) -> Result<(), OcrFailure> {
    prepare_managed_file(root, path).map_err(storage_failure)?;
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(OcrFailure::new(
            LocalModelErrorKind::Unknown,
            "Could not update the OCR model files.",
            error.to_string(),
        )),
    }
}

async fn remove_managed_file(root: &Path, path: &Path) -> Result<(), AppError> {
    prepare_managed_file(root, path)
        .map_err(|error| AppError::Internal(format!("validate OCR deletion path: {error}")))?;
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Internal(format!("remove OCR model file: {error}"))),
    }
}

fn storage_failure(error: std::io::Error) -> OcrFailure {
    OcrFailure::new(
        LocalModelErrorKind::Unknown,
        "OCR model storage did not pass safety checks.",
        error.to_string(),
    )
}

fn ocr_download_client() -> reqwest::Client {
    let build = || {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 || !allowed_download_url(attempt.url()) {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
    };
    nomifun_net::proxy::apply_detected_proxy(build())
        .build()
        .unwrap_or_else(|error| {
            warn!(error = %error, "Could not apply system proxy to OCR downloader");
            build()
                .build()
                .expect("OCR download HTTP client configuration is valid")
        })
}

fn allowed_download_url(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };
    host == "huggingface.co"
        || host.ends_with(".huggingface.co")
        || host == "hf-mirror.com"
        || host.ends_with(".hf-mirror.com")
        || host == "hf.co"
        || host.ends_with(".hf.co")
        || host == "xethub.hf.co"
        || host.ends_with(".xethub.hf.co")
}

fn download_sources(url: &str) -> Vec<String> {
    let mut sources = vec![url.to_owned()];
    let Ok(mut mirror) = reqwest::Url::parse(url) else {
        return sources;
    };
    if mirror.host_str() == Some("huggingface.co")
        && mirror.set_host(Some("hf-mirror.com")).is_ok()
    {
        sources.push(mirror.into());
    }
    sources
}

fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse().ok()?;
    let end = end.parse().ok()?;
    let total = total.parse().ok()?;
    (start <= end && end < total).then_some((start, end, total))
}

fn safe_relative_path(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn unsafe_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.is_file() && metadata.nlink() > 1 {
            return true;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

fn prepare_managed_directory(root: &Path, directory: &Path) -> std::io::Result<()> {
    let relative = directory.strip_prefix(root).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "managed directory escaped OCR root",
        )
    })?;
    if !safe_relative_path(relative) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "managed OCR directory has an unsafe relative path",
        ));
    }

    std::fs::create_dir_all(root)?;
    let root_metadata = std::fs::symlink_metadata(root)?;
    if unsafe_link_or_reparse(&root_metadata) || !root_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "OCR root is a link or not a directory",
        ));
    }
    let canonical_root = std::fs::canonicalize(root)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if unsafe_link_or_reparse(&metadata) || !metadata.is_dir() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "managed OCR ancestor is a link or not a directory",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current)?;
                let metadata = std::fs::symlink_metadata(&current)?;
                if unsafe_link_or_reparse(&metadata) || !metadata.is_dir() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "managed OCR directory creation was redirected",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
    let canonical_directory = std::fs::canonicalize(&current)?;
    if !canonical_directory.starts_with(canonical_root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "managed OCR directory resolved outside its root",
        ));
    }
    Ok(())
}

fn prepare_managed_file(root: &Path, path: &Path) -> std::io::Result<()> {
    let relative = path.strip_prefix(root).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "managed file escaped OCR root",
        )
    })?;
    if relative.as_os_str().is_empty() || !safe_relative_path(relative) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "managed OCR file has an unsafe relative path",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "managed OCR file has no parent",
        )
    })?;
    prepare_managed_directory(root, parent)?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if unsafe_link_or_reparse(&metadata) || !metadata.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "managed OCR target is a link or not a regular file",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn catalog_artifacts_are_immutable_and_small() {
        assert_eq!(OCR_ARTIFACTS[0].revision, "28fe5895c24fd108c19eb3e8479f4ab385fbfc62");
        assert_eq!(OCR_ARTIFACTS[0].size, 9_880_512);
        assert_eq!(OCR_ARTIFACTS[1].size, 885);
        assert_eq!(OCR_ARTIFACTS[2].revision, "b8f84f0b80c529de40b4fbb3544b84fa7233a513");
        assert_eq!(OCR_ARTIFACTS[2].size, 21_159_378);
        assert_eq!(OCR_ARTIFACTS[3].size, 150_579);
        assert_eq!(catalog_entry().download_size_bytes, 31_191_354);
        assert!(OCR_ARTIFACTS.iter().all(|artifact| artifact.url.contains(artifact.revision)));
    }

    #[tokio::test]
    async fn construction_never_downloads_and_reports_runtime_honestly() {
        let temp = TempDir::new().unwrap();
        let service = OcrModelService::new(temp.path()).await.unwrap();
        let status = service.status().await;
        assert!(!status.artifacts_ready);
        assert!(!status.inference_ready);
        assert_eq!(status.models[0].install_phase, LocalModelInstallPhase::NotInstalled);
        assert_eq!(current_bytes(&service.root).await, 0);
    }

    #[test]
    fn downloader_allows_only_pinned_ecosystem_hosts() {
        assert!(allowed_download_url(&reqwest::Url::parse("https://huggingface.co/a/b").unwrap()));
        assert!(allowed_download_url(&reqwest::Url::parse("https://cdn-lfs.hf.co/a").unwrap()));
        assert!(!allowed_download_url(&reqwest::Url::parse("http://huggingface.co/a").unwrap()));
        assert!(!allowed_download_url(&reqwest::Url::parse("https://huggingface.co.evil.test/a").unwrap()));
    }

    #[test]
    fn managed_paths_reject_escape() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("ocr");
        prepare_managed_directory(&root, &root).unwrap();
        assert!(prepare_managed_file(&root, &root.join("model/file.onnx")).is_ok());
        assert!(prepare_managed_file(&root, &temp.path().join("outside.onnx")).is_err());
    }
}
