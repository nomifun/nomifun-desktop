//! Exercises the actual managed process path with deterministic native fixtures.
#![cfg(unix)]
use nomifun_api_types::{
    OfficePreviewDocType as DocType, OfficePreviewError, OfficePreviewResponse, OfficePreviewState,
};
use nomifun_office::{
    OfficeEngine, OfficeEngineSupplier, OfficePreviewService, OfficeRouterState, PreviewLimits,
    SnapshotService, office_artifact_routes,
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

struct Supplier(OfficeEngine);
#[async_trait::async_trait]
impl OfficeEngineSupplier for Supplier {
    async fn resolve(&self) -> Result<OfficeEngine, OfficePreviewError> {
        Ok(self.0.clone())
    }
}
fn fixture(path: &Path, body: &str) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for name in ["[Content_Types].xml", "ppt/presentation.xml"] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}
fn script(directory: &Path, content: &str) -> OfficeEngine {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("renderer");
    std::fs::write(&path, format!("#!/bin/sh\n{content}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    OfficeEngine {
        executable_path: path,
        version: "test-v1".into(),
        digest: "test-engine-digest".into(),
    }
}
fn renderer(directory: &Path, delay: &str) -> OfficeEngine {
    script(
        directory,
        &format!(
            "test \"$OFFICECLI_NO_AUTO_RESIDENT\" = 1 || exit 8\ntest \"$1\" = view || exit 9\ntest \"$3\" = html || exit 10\nprintf x >> '{}'\nsleep {delay}\nprintf '<!DOCTYPE html><html><body>document</body></html>'",
            directory.join("count").display()
        ),
    )
}
fn service(directory: &Path, engine: OfficeEngine, limits: PreviewLimits) -> OfficePreviewService {
    OfficePreviewService::with_supplier(directory, Arc::new(Supplier(engine)), limits)
}
fn start_type(
    service: &OfficePreviewService,
    owner: &str,
    file: &Path,
    doc_type: DocType,
) -> String {
    let id = uuid::Uuid::now_v7().to_string();
    let response = service
        .start(owner, &id, std::fs::canonicalize(file).unwrap(), doc_type)
        .unwrap();
    assert_eq!(response.state, OfficePreviewState::Queued);
    id
}
fn start(service: &OfficePreviewService, owner: &str, file: &Path) -> String {
    start_type(service, owner, file, DocType::Ppt)
}

async fn terminal(service: &OfficePreviewService, owner: &str, id: &str) -> OfficePreviewResponse {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = service.status(owner, id).unwrap();
            if matches!(
                response.state,
                OfficePreviewState::Ready | OfficePreviewState::Error
            ) {
                return response;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("request must settle")
}
fn count(directory: &Path) -> usize {
    std::fs::read(directory.join("count"))
        .unwrap_or_default()
        .len()
}

#[tokio::test]
async fn shared_render_survives_one_cancel_and_owner_grants_are_separate() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("中文.pptx");
    fixture(&file, "<root/>");
    let service = service(
        directory.path(),
        renderer(directory.path(), "1"),
        PreviewLimits::default(),
    );
    let first = start(&service, "first", &file);
    let second = start(&service, "second", &file);
    tokio::time::sleep(Duration::from_millis(550)).await;
    service.stop("first", &first).unwrap();
    assert!(service.status("other", &second).is_err());
    let response = terminal(&service, "second", &second).await;
    assert_eq!(response.state, OfficePreviewState::Ready, "{response:?}");
    assert_eq!(count(directory.path()), 1);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(directory.path().join("office-preview-cache"))
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0
    );
    let capability = response.capability.unwrap();
    let html = service
        .read_artifact(&capability, &second, &uuid::Uuid::now_v7().to_string())
        .await
        .unwrap();
    assert!(
        String::from_utf8(html)
            .unwrap()
            .contains("nomifun-office-preview-ready")
    );
    // Releasing a different owner's same ID cannot revoke this capability.
    service.stop("first", &second).unwrap();
    assert!(
        service
            .read_artifact(&capability, &second, &uuid::Uuid::now_v7().to_string())
            .await
            .is_ok()
    );
    service.stop("second", &second).unwrap();
    assert!(
        service
            .read_artifact(&capability, &second, &uuid::Uuid::now_v7().to_string())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn repeated_opens_reuse_disk_cache_and_content_change_invalidates_it() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("slides.pptx");
    fixture(&file, "<v1/>");
    let engine = renderer(directory.path(), "0");
    let service_a = service(directory.path(), engine.clone(), PreviewLimits::default());
    for _ in 0..50 {
        let id = start(&service_a, "owner", &file);
        assert_eq!(
            terminal(&service_a, "owner", &id).await.state,
            OfficePreviewState::Ready
        );
        service_a.stop("owner", &id).unwrap();
    }
    assert_eq!(count(directory.path()), 1);
    drop(service_a);
    let service_b = service(directory.path(), engine, PreviewLimits::default());
    let id = start(&service_b, "owner", &file);
    assert_eq!(
        terminal(&service_b, "owner", &id).await.state,
        OfficePreviewState::Ready
    );
    assert_eq!(count(directory.path()), 1);
    fixture(&file, "<v2/>");
    let changed = start(&service_b, "owner", &file);
    assert_eq!(
        terminal(&service_b, "owner", &changed).await.state,
        OfficePreviewState::Ready
    );
    assert_eq!(count(directory.path()), 2);
}

#[tokio::test]
async fn stop_before_start_and_id_rebinding_cannot_resurrect_work() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("slides.pptx");
    fixture(&file, "<v1/>");
    let service = service(
        directory.path(),
        renderer(directory.path(), "0"),
        PreviewLimits::default(),
    );
    let id = uuid::Uuid::now_v7().to_string();
    service.stop("owner", &id).unwrap();
    assert_eq!(
        service
            .start("owner", &id, file.clone(), DocType::Ppt)
            .unwrap()
            .error
            .unwrap()
            .code,
        "CANCELLED"
    );
    assert_eq!(count(directory.path()), 0);
    let active = start(&service, "owner", &file);
    assert!(
        service
            .start(
                "owner",
                &active,
                directory.path().join("other.pptx"),
                DocType::Ppt
            )
            .is_err()
    );
    service.stop("owner", &active).unwrap();
    assert!(
        service
            .start("owner", "not-a-uuid", file, DocType::Ppt)
            .is_err()
    );
}

#[tokio::test]
async fn early_exit_reports_real_failure_without_port_wait() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("slides.pptx");
    fixture(&file, "<v1/>");
    let service = service(
        directory.path(),
        script(directory.path(), "echo 'requires TTY' >&2; exit 13"),
        PreviewLimits::default(),
    );
    let started = Instant::now();
    let id = start(&service, "owner", &file);
    let response = terminal(&service, "owner", &id).await;
    assert_eq!(response.error.unwrap().code, "RENDER_FAILED");
    // Includes the native authority's launch/cleanup handshake, which can
    // take seconds on a busy CI host. Still rejects the retired fixed15s wait.
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn cancel_and_timeout_seal_actual_process_tree() {
    use nomifun_office::artifact::render_html;
    for cancel in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let engine = script(directory.path(), "sleep 120 &\nwait");
        let source = directory.path().join("source.pptx");
        std::fs::write(&source, "fixture").unwrap();
        let token = CancellationToken::new();
        if cancel {
            let token = token.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                token.cancel();
            });
        }
        let started = Instant::now();
        let error = render_html(
            &engine,
            &source,
            &directory.path().join("index.html"),
            &token,
            Duration::from_millis(150),
            1024,
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.code,
            if cancel {
                "CANCELLED"
            } else {
                "RENDER_TIMEOUT"
            }
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

#[tokio::test]
async fn bounded_output_invalid_documents_and_formats_fail_precisely() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("slides.pptx");
    fixture(&file, "<v1/>");
    let service = service(
        directory.path(),
        renderer(directory.path(), "0"),
        PreviewLimits {
            max_html_bytes: 8,
            ..PreviewLimits::default()
        },
    );
    let id = start(&service, "owner", &file);
    assert_eq!(
        terminal(&service, "owner", &id).await.error.unwrap().code,
        "RESOURCE_LIMIT"
    );
    let invalid = directory.path().join("broken.pptx");
    std::fs::write(&invalid, "incomplete zip").unwrap();
    let id = start(&service, "owner", &invalid);
    assert_eq!(
        terminal(&service, "owner", &id).await.error.unwrap().code,
        "DOCUMENT_INVALID"
    );
    let unsupported = directory.path().join("legacy.ppt");
    std::fs::write(&unsupported, "legacy").unwrap();
    let id = start(&service, "owner", &unsupported);
    assert_eq!(
        terminal(&service, "owner", &id).await.error.unwrap().code,
        "FORMAT_UNSUPPORTED"
    );
}

#[tokio::test]
async fn disk_bounds_evict_closed_previews_and_never_invalidate_open_ones() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.pptx");
    fixture(&a, "<a/>");
    let b = directory.path().join("b.pptx");
    fixture(&b, "<b/>");
    let service = service(
        directory.path(),
        renderer(directory.path(), "0"),
        PreviewLimits {
            cache_entries: 1,
            ..PreviewLimits::default()
        },
    );
    let first = start(&service, "owner", &a);
    let response = terminal(&service, "owner", &first).await;
    let capability = response.capability.unwrap();
    let second = start(&service, "owner", &b);
    assert_eq!(
        terminal(&service, "owner", &second)
            .await
            .error
            .unwrap()
            .code,
        "CACHE_BUSY"
    );
    assert!(
        service
            .read_artifact(&capability, &first, &uuid::Uuid::now_v7().to_string())
            .await
            .is_ok()
    );
    service.stop("owner", &first).unwrap();
    let retry = start(&service, "owner", &b);
    assert_eq!(
        terminal(&service, "owner", &retry).await.state,
        OfficePreviewState::Ready
    );
    assert_eq!(
        std::fs::read_dir(directory.path().join("office-preview-cache/artifacts"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn source_access_and_cache_integrity_are_rechecked() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("slides.pptx");
    fixture(&file, "<v1/>");
    let service = service(
        directory.path(),
        renderer(directory.path(), "0"),
        PreviewLimits::default(),
    );
    let first = start(&service, "owner", &file);
    let response = terminal(&service, "owner", &first).await;
    let cap = response.capability.unwrap();
    let artifact = std::fs::read_dir(directory.path().join("office-preview-cache/artifacts"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("index.html");
    std::fs::write(&artifact, "corrupt").unwrap();
    service.stop("owner", &first).unwrap();
    let second = start(&service, "owner", &file);
    let response = terminal(&service, "owner", &second).await;
    assert_eq!(response.state, OfficePreviewState::Ready);
    assert_eq!(count(directory.path()), 2);
    std::fs::remove_file(&file).unwrap();
    assert!(
        service
            .read_artifact(
                &response.capability.unwrap(),
                &second,
                &uuid::Uuid::now_v7().to_string()
            )
            .await
            .is_err()
    );
    assert!(
        service
            .read_artifact(&cap, &first, &uuid::Uuid::now_v7().to_string())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn leases_have_bounds_and_expire_proactively() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("slides.pptx");
    fixture(&file, "<v1/>");
    let service = service(
        directory.path(),
        renderer(directory.path(), "0"),
        PreviewLimits {
            max_requests: 1,
            lease_ttl: Duration::from_millis(40),
            ..PreviewLimits::default()
        },
    );
    // Cancellation history does not consume active admission slots.
    for _ in 0..600 {
        service
            .stop("owner", &uuid::Uuid::now_v7().to_string())
            .unwrap();
    }
    let first = start(&service, "owner", &file);
    assert!(
        service
            .start(
                "owner",
                &uuid::Uuid::now_v7().to_string(),
                file.clone(),
                DocType::Ppt
            )
            .is_err()
    );
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        count(directory.path()),
        0,
        "expiry must cancel preparation without another API call"
    );
    assert!(service.status("owner", &first).is_err());
    let second = start(&service, "owner", &file);
    service.stop("owner", &second).unwrap();
}

#[tokio::test]
async fn artifact_http_policy_denies_network_and_unsafe_embedding() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("slides.pptx");
    fixture(&file, "<v1/>");
    let service = Arc::new(service(
        directory.path(),
        renderer(directory.path(), "0"),
        PreviewLimits::default(),
    ));
    let id = start(&service, "owner", &file);
    let response = terminal(&service, "owner", &id).await;
    let router = office_artifact_routes(OfficeRouterState {
        preview_service: service.clone(),
        snapshot_service: Arc::new(SnapshotService::new(directory.path())),
        allowed_roots: vec![directory.path().to_owned()],
    });
    use tower::ServiceExt;
    let request = axum::http::Request::builder()
        .uri(format!(
            "{}?request_id={id}&nonce={}",
            response.url.unwrap(),
            uuid::Uuid::now_v7()
        ))
        .body(axum::body::Body::empty())
        .unwrap();
    let http = router.oneshot(request).await.unwrap();
    assert_eq!(http.status(), 200);
    let policy = http.headers()["content-security-policy"].to_str().unwrap();
    assert!(policy.contains("connect-src 'none'"));
    assert!(policy.contains("sandbox allow-scripts"));
    assert!(policy.contains("default-src 'none'"));
    assert_eq!(http.headers()["cache-control"], "no-store");
    let html = axum::body::to_bytes(http.into_body(), 100_000)
        .await
        .unwrap();
    assert!(
        String::from_utf8(html.to_vec())
            .unwrap()
            .contains("nomifun-office-preview-ready")
    );
    service.stop("owner", &id).unwrap();
}

#[tokio::test]
async fn queued_documents_keep_disposable_snapshot_storage_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let service = service(
        directory.path(),
        renderer(directory.path(), "1"),
        PreviewLimits::default(),
    );
    let mut ids = Vec::new();
    for index in 0..12 {
        let file = directory.path().join(format!("{index}.pptx"));
        fixture(&file, &format!("<v{index}/>"));
        ids.push(start(&service, "owner", &file));
    }
    tokio::time::sleep(Duration::from_millis(650)).await;
    assert!(
        std::fs::read_dir(directory.path().join("office-preview-cache/staging"))
            .unwrap()
            .count()
            <= 2
    );
    for id in ids {
        service.stop("owner", &id).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if std::fs::read_dir(directory.path().join("office-preview-cache/staging"))
                .unwrap()
                .count()
                == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn unchanged_length_and_restored_mtime_do_not_reuse_replaced_content() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("slides.pptx");
    fixture(&file, "<v1/>");
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    let service = service(
        directory.path(),
        renderer(directory.path(), "0"),
        PreviewLimits::default(),
    );
    let first = start(&service, "owner", &file);
    assert_eq!(
        terminal(&service, "owner", &first).await.state,
        OfficePreviewState::Ready
    );
    let replacement = directory.path().join("replacement.pptx");
    fixture(&replacement, "<v2/>");
    std::fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(
        std::fs::metadata(&replacement).unwrap().len(),
        std::fs::metadata(&file).unwrap().len()
    );
    std::fs::rename(replacement, &file).unwrap();
    let second = start(&service, "owner", &file);
    assert_eq!(
        terminal(&service, "owner", &second).await.state,
        OfficePreviewState::Ready
    );
    assert_eq!(count(directory.path()), 2);
}

#[tokio::test]
async fn artifact_reader_rejects_wrong_request_and_oversize_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("slides.pptx");
    fixture(&file, "<v1/>");
    let service = service(
        directory.path(),
        renderer(directory.path(), "0"),
        PreviewLimits {
            max_html_bytes: 128,
            ..PreviewLimits::default()
        },
    );
    let id = start(&service, "owner", &file);
    let response = terminal(&service, "owner", &id).await;
    let cap = response.capability.unwrap();
    assert!(
        service
            .read_artifact(
                &cap,
                &uuid::Uuid::now_v7().to_string(),
                &uuid::Uuid::now_v7().to_string()
            )
            .await
            .is_err()
    );
    let path = std::fs::read_dir(directory.path().join("office-preview-cache/artifacts"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("index.html");
    std::fs::write(path, "x".repeat(129)).unwrap();
    assert!(
        service
            .read_artifact(&cap, &id, &uuid::Uuid::now_v7().to_string())
            .await
            .is_err()
    );
}

/// Optional real-engine smoke: independent of model API credentials.
#[tokio::test]
#[ignore = "requires a verified native OfficeCLI and an actual Office fixture"]
async fn real_managed_engine_generates_and_reuses_html() {
    let file =
        PathBuf::from(std::env::var("NOMIFUN_OFFICE_PREVIEW_FIXTURE").expect("fixture path"));
    let directory = tempfile::tempdir().unwrap();
    let service = OfficePreviewService::new(directory.path());
    let source = std::fs::canonicalize(file).unwrap();
    let doc_type = match source.extension().unwrap().to_str().unwrap() {
        "docx" => DocType::Word,
        "xlsx" => DocType::Excel,
        "pptx" => DocType::Ppt,
        _ => panic!("supported fixture"),
    };
    let id = start_type(&service, "smoke-owner", &source, doc_type);
    let response = terminal(&service, "smoke-owner", &id).await;
    assert_eq!(response.state, OfficePreviewState::Ready, "{response:?}");
    let html = service
        .read_artifact(
            &response.capability.unwrap(),
            &id,
            &uuid::Uuid::now_v7().to_string(),
        )
        .await
        .unwrap();
    assert!(html.len() > 100);
    service.stop("smoke-owner", &id).unwrap();
    let second = start_type(&service, "smoke-owner", &source, doc_type);
    assert_eq!(
        terminal(&service, "smoke-owner", &second).await.state,
        OfficePreviewState::Ready
    );
    assert_eq!(
        std::fs::read_dir(directory.path().join("office-preview-cache/artifacts"))
            .unwrap()
            .count(),
        1
    );
}
