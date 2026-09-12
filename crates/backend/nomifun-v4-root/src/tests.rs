use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nomifun_agent_contracts::{
    FreshV4OperationKind, FreshV4ReadyMarker, FRESH_V4_PARENT_MARKER_FILE,
    FRESH_V4_READY_MARKER_FILE,
};

use crate::{
    FreshV4AccessAudit, FreshV4AccessKind, FreshV4Clock, FreshV4Coordinator,
    FreshV4FaultInjector, FreshV4FaultPoint, FreshV4QuiescePort,
    FreshV4RecoveryPhase, FreshV4RootError, NoAccessAudit, NoFaults,
    PreServiceQuiesced, application_build_digest,
};

const BUILD_IDENTITY: &str = "nomifun-v4-root-test-build";
const FIXED_TIMESTAMP: &str = "20260829123456";

#[derive(Clone, Copy)]
struct FixedClock;

impl FreshV4Clock for FixedClock {
    fn utc_timestamp(&self) -> String {
        FIXED_TIMESTAMP.to_owned()
    }
}

#[derive(Clone)]
struct OneShotFault {
    point: Arc<Mutex<Option<FreshV4FaultPoint>>>,
}

impl OneShotFault {
    fn at(point: FreshV4FaultPoint) -> Self {
        Self {
            point: Arc::new(Mutex::new(Some(point))),
        }
    }
}

impl FreshV4FaultInjector for OneShotFault {
    fn check(&self, point: FreshV4FaultPoint) -> Result<(), FreshV4RootError> {
        let mut armed = self.point.lock().unwrap();
        if *armed == Some(point) {
            *armed = None;
            return Err(FreshV4RootError::Fault(format!("{point:?}")));
        }
        Ok(())
    }
}

#[derive(Default)]
struct RecordingAudit {
    archive: Mutex<Option<PathBuf>>,
    accesses: Mutex<Vec<(FreshV4AccessKind, PathBuf)>>,
}

impl RecordingAudit {
    fn trap_archive(&self, archive: PathBuf) {
        *self.archive.lock().unwrap() = Some(archive);
    }

    fn accesses(&self) -> Vec<(FreshV4AccessKind, PathBuf)> {
        self.accesses.lock().unwrap().clone()
    }
}

impl FreshV4AccessAudit for Arc<RecordingAudit> {
    fn record(
        &self,
        kind: FreshV4AccessKind,
        path: &Path,
    ) -> Result<(), FreshV4RootError> {
        {
            let archive = self.archive.lock().unwrap();
            if let Some(archive) = archive.as_ref()
                && path.starts_with(archive)
                && path != archive
            {
                return Err(FreshV4RootError::State(format!(
                    "archive descendant access is forbidden: {}",
                    path.display()
                )));
            }
        }
        self.accesses
            .lock()
            .unwrap()
            .push((kind, path.to_path_buf()));
        Ok(())
    }
}

struct RejectQuiesce;

impl FreshV4QuiescePort for RejectQuiesce {
    fn quiesce_before_root_change(&self) -> Result<(), FreshV4RootError> {
        Err(FreshV4RootError::Quiesce(
            "test holder remains active".into(),
        ))
    }
}

fn production_test_coordinator(
) -> FreshV4Coordinator<PreServiceQuiesced, NoFaults, NoAccessAudit, FixedClock>
{
    FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        NoFaults,
        NoAccessAudit,
        FixedClock,
    )
}

fn archive_for(root: &Path) -> PathBuf {
    root.with_file_name(format!(
        "{}.pre-v4-archive-{FIXED_TIMESTAMP}",
        root.file_name().unwrap().to_string_lossy()
    ))
}

#[tokio::test]
async fn fresh_install_publishes_distinct_ready_contract() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    let audit = Arc::new(RecordingAudit::default());

    let outcome = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        NoFaults,
        Arc::clone(&audit),
        FixedClock,
    )
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap();

    assert_eq!(
        outcome.operation_kind,
        Some(FreshV4OperationKind::Fresh)
    );
    assert_eq!(
        outcome.recovered_from,
        FreshV4RecoveryPhase::IntentDurable
    );
    assert!(root.join(crate::FRESH_V4_DATABASE_FILE).is_file());
    assert!(root.join(FRESH_V4_READY_MARKER_FILE).is_file());
    assert!(!root.join(crate::FRESH_V4_INITIALIZING_MARKER_FILE).exists());
    assert!(!parent.path().join(FRESH_V4_PARENT_MARKER_FILE).exists());
    assert_eq!(
        outcome.ready_marker.application_build_digest,
        application_build_digest(BUILD_IDENTITY).unwrap()
    );
    assert!(
        outcome
            .ready_marker
            .matches_schema_metadata(&outcome.schema_metadata)
    );

    let bytes = std::fs::read(root.join(FRESH_V4_READY_MARKER_FILE)).unwrap();
    let ready: FreshV4ReadyMarker = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(ready, outcome.ready_marker);
    assert!(
        audit.accesses().iter().all(|(kind, _)| !matches!(
            *kind,
            FreshV4AccessKind::RenameSource | FreshV4AccessKind::RenameTarget
        )),
        "fresh initialization must never perform an archive rename"
    );
}

#[tokio::test]
async fn precreated_empty_canonical_directory_is_treated_as_fresh_install() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    std::fs::create_dir(&root).unwrap();

    let outcome = production_test_coordinator()
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap();

    assert_eq!(
        outcome.operation_kind,
        Some(FreshV4OperationKind::Fresh)
    );
    assert!(root.join(crate::FRESH_V4_DATABASE_FILE).is_file());
    assert!(root.join(FRESH_V4_READY_MARKER_FILE).is_file());
    assert!(
        !parent
            .path()
            .join(format!(
                "NomiFun.pre-v4-archive-{FIXED_TIMESTAMP}"
            ))
            .exists()
    );
}

#[tokio::test]
async fn cutover_is_one_whole_root_rename_and_archive_is_opaque() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    let sentinel = root.join("opaque/nested/legacy.bin");
    std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
    std::fs::write(&sentinel, b"legacy-bytes").unwrap();
    let archive = archive_for(&root);
    let audit = Arc::new(RecordingAudit::default());
    audit.trap_archive(archive.clone());
    let coordinator = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        NoFaults,
        Arc::clone(&audit),
        FixedClock,
    );

    let outcome = coordinator
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap();

    assert_eq!(
        outcome.operation_kind,
        Some(FreshV4OperationKind::Cutover)
    );
    assert_eq!(
        std::fs::read(archive.join("opaque/nested/legacy.bin")).unwrap(),
        b"legacy-bytes"
    );
    assert!(root.join(FRESH_V4_READY_MARKER_FILE).is_file());
    let accesses = audit.accesses();
    assert_eq!(
        accesses
            .iter()
            .filter(|(kind, _)| *kind == FreshV4AccessKind::RenameSource)
            .count(),
        1
    );
    assert_eq!(
        accesses
            .iter()
            .filter(|(kind, path)| {
                *kind == FreshV4AccessKind::RenameTarget
                    && std::fs::canonicalize(path).ok()
                        == std::fs::canonicalize(&archive).ok()
            })
            .count(),
        1
    );
}

#[tokio::test]
async fn post_rename_crash_recovers_without_touching_archive_children() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("legacy-sentinel"), b"legacy").unwrap();
    let archive = archive_for(&root);
    let first = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        OneShotFault::at(FreshV4FaultPoint::AfterCutoverRename),
        NoAccessAudit,
        FixedClock,
    );

    assert!(
        first
            .bootstrap(&root, BUILD_IDENTITY, &[])
            .await
            .is_err()
    );
    assert!(!root.exists());
    assert_eq!(
        std::fs::read(archive.join("legacy-sentinel")).unwrap(),
        b"legacy"
    );

    let audit = Arc::new(RecordingAudit::default());
    audit.trap_archive(archive);
    let recovery = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        NoFaults,
        audit,
        FixedClock,
    )
    .bootstrap(&root, BUILD_IDENTITY, &[])
    .await
    .unwrap();
    assert_eq!(
        recovery.recovered_from,
        FreshV4RecoveryPhase::CutoverRenamed
    );
}

#[tokio::test]
async fn immutable_parent_marker_bytes_survive_multi_phase_recovery() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    let marker = parent.path().join(FRESH_V4_PARENT_MARKER_FILE);
    let first = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        OneShotFault::at(FreshV4FaultPoint::AfterParentMarkerDurable),
        NoAccessAudit,
        FixedClock,
    );
    assert!(
        first
            .bootstrap(&root, BUILD_IDENTITY, &[])
            .await
            .is_err()
    );
    assert!(
        !root.exists(),
        "fresh root must not be created before the parent marker is durable"
    );
    let original = std::fs::read(&marker).unwrap();

    let second = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        OneShotFault::at(FreshV4FaultPoint::AfterSchemaMetadataCommitted),
        NoAccessAudit,
        FixedClock,
    );
    assert!(
        second
            .bootstrap(&root, BUILD_IDENTITY, &[])
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&marker).unwrap(), original);

    production_test_coordinator()
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap();
    assert!(!marker.exists());
}

#[tokio::test]
async fn fresh_fault_matrix_converges_to_one_ready_root() {
    let points = [
        FreshV4FaultPoint::AfterParentMarkerDurable,
        FreshV4FaultPoint::AfterCanonicalRootCreated,
        FreshV4FaultPoint::AfterInitializingMarkerDurable,
        FreshV4FaultPoint::AfterSchemaMetadataCommitted,
        FreshV4FaultPoint::AfterMaterializationCommitted,
        FreshV4FaultPoint::AfterSeedCommitted,
        FreshV4FaultPoint::AfterReadyMarkerDurable,
        FreshV4FaultPoint::AfterInitializingMarkerRemoved,
        FreshV4FaultPoint::AfterParentMarkerRemoved,
    ];

    for point in points {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("NomiFun");
        let injected = FreshV4Coordinator::with_ports(
            PreServiceQuiesced,
            OneShotFault::at(point),
            NoAccessAudit,
            FixedClock,
        );
        assert!(
            injected
                .bootstrap(&root, BUILD_IDENTITY, &[])
                .await
                .is_err(),
            "{point:?} did not interrupt the operation"
        );

        let recovered = production_test_coordinator()
            .bootstrap(&root, BUILD_IDENTITY, &[])
            .await
            .unwrap();
        assert!(root.join(FRESH_V4_READY_MARKER_FILE).is_file());
        assert!(
            recovered
                .ready_marker
                .matches_schema_metadata(&recovered.schema_metadata)
        );
    }
}

#[tokio::test]
async fn cutover_failures_never_fall_back_to_copy_or_rollback() {
    for point in [
        FreshV4FaultPoint::BeforeCutoverRename,
        FreshV4FaultPoint::AfterCutoverRename,
    ] {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("NomiFun");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("sentinel"), b"legacy").unwrap();
        let archive = archive_for(&root);
        let injected = FreshV4Coordinator::with_ports(
            PreServiceQuiesced,
            OneShotFault::at(point),
            NoAccessAudit,
            FixedClock,
        );

        assert!(
            injected
                .bootstrap(&root, BUILD_IDENTITY, &[])
                .await
                .is_err()
        );
        match point {
            FreshV4FaultPoint::BeforeCutoverRename => {
                assert_eq!(std::fs::read(root.join("sentinel")).unwrap(), b"legacy");
                assert!(!archive.exists());
            }
            FreshV4FaultPoint::AfterCutoverRename => {
                assert!(!root.exists());
                assert_eq!(
                    std::fs::read(archive.join("sentinel")).unwrap(),
                    b"legacy"
                );
            }
            _ => unreachable!(),
        }
        assert!(
            parent.path().join(FRESH_V4_PARENT_MARKER_FILE).is_file(),
            "the immutable recovery fence must remain after {point:?}"
        );
    }
}

#[tokio::test]
async fn archive_target_collision_aborts_before_parent_marker_and_rename() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("sentinel"), b"legacy").unwrap();
    std::fs::create_dir(archive_for(&root)).unwrap();

    let error = production_test_coordinator()
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap_err();

    assert!(matches!(error, FreshV4RootError::State(_)));
    assert_eq!(std::fs::read(root.join("sentinel")).unwrap(), b"legacy");
    assert!(!parent.path().join(FRESH_V4_PARENT_MARKER_FILE).exists());
    assert!(!root.join(FRESH_V4_READY_MARKER_FILE).exists());
}

#[tokio::test]
async fn missing_archive_never_causes_a_ready_v4_root_to_be_renamed() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("legacy-sentinel"), b"legacy").unwrap();
    let archive = archive_for(&root);
    let first = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        OneShotFault::at(FreshV4FaultPoint::AfterReadyMarkerDurable),
        NoAccessAudit,
        FixedClock,
    );
    assert!(
        first
            .bootstrap(&root, BUILD_IDENTITY, &[])
            .await
            .is_err()
    );
    assert!(root.join(FRESH_V4_READY_MARKER_FILE).is_file());
    std::fs::remove_dir_all(&archive).unwrap();

    let error = production_test_coordinator()
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap_err();

    assert!(matches!(error, FreshV4RootError::State(_)));
    assert!(root.join(FRESH_V4_READY_MARKER_FILE).is_file());
    assert!(!archive.exists());
}

#[tokio::test]
async fn deterministic_ready_staging_is_recovered_without_scanning() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    let first = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        OneShotFault::at(FreshV4FaultPoint::AfterSeedCommitted),
        NoAccessAudit,
        FixedClock,
    );
    assert!(
        first
            .bootstrap(&root, BUILD_IDENTITY, &[])
            .await
            .is_err()
    );
    let staging =
        root.join(format!("{FRESH_V4_READY_MARKER_FILE}.staging"));
    std::fs::write(&staging, b"partial-ready").unwrap();

    production_test_coordinator()
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap();

    assert!(!staging.exists());
    assert!(root.join(FRESH_V4_READY_MARKER_FILE).is_file());
}

#[tokio::test]
async fn quiesce_failure_performs_no_filesystem_access() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    let audit = Arc::new(RecordingAudit::default());
    let coordinator = FreshV4Coordinator::with_ports(
        RejectQuiesce,
        NoFaults,
        Arc::clone(&audit),
        FixedClock,
    );

    assert!(
        coordinator
            .bootstrap(&root, BUILD_IDENTITY, &[])
            .await
            .is_err()
    );
    assert!(audit.accesses().is_empty());
    assert!(!root.exists());
}

#[tokio::test]
async fn missing_bundled_package_is_reported_before_foreign_key_validation() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    let coordinator = production_test_coordinator();
    coordinator
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap();
    // Simulate externally corrupted storage, not a supported package deletion.
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(root.join(crate::FRESH_V4_DATABASE_FILE))
                .foreign_keys(false),
        )
        .await
        .unwrap();
    let deleted = sqlx::query("DELETE FROM plugin_packages WHERE package_id = 'nomifun.chat'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(deleted.rows_affected(), 1);
    pool.close().await;

    let error = coordinator
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap_err();
    assert!(
        matches!(error, FreshV4RootError::State(ref message)
        if message.contains("bundled package materialization mismatch for nomifun.chat@")
            && message.contains("package row is missing")),
        "{error}"
    );
}

#[tokio::test]
async fn ready_root_rejects_runtime_capability_sql_owner_drift() {
    use nomifun_agent_contracts::{CapabilityManifest, canonical_json_bytes, digest_payload};

    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    let coordinator = production_test_coordinator();
    coordinator
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap();
    let inputs = crate::inputs::FrozenRootInputs::load().unwrap();
    let package = inputs
        .target_inventory
        .packages
        .iter()
        .find(|package| package.package.id.as_ref() == "nomifun.chat")
        .unwrap();
    let capability = &package.capabilities[0];
    let manifest: CapabilityManifest = serde_json::from_value(serde_json::json!({
        "id": capability.capability.id,
        "version": capability.capability.version,
        "kind": capability.kind,
        "package": package.package,
        "contribution_id": "r82-runtime-capability",
        "display": {"name": "R82 fixture", "description": "", "localized_names": {}, "localized_descriptions": {}},
        "requires": [], "conflicts": [], "supported_surfaces": [],
        "requires_runtime_features": [], "supported_platforms": [],
        "config_schema": {"type": "object"},
        "contributions": {"actions": [], "context_schema_refs": [], "event_schema_refs": [], "resource_kinds": [], "host_ports": []}
    })).unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(root.join(crate::FRESH_V4_DATABASE_FILE))
        .foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    let updated = sqlx::query(
        "UPDATE capability_definitions SET manifest_json = ?, manifest_digest = ? WHERE capability_id = ?",
    )
    .bind(String::from_utf8(canonical_json_bytes(&manifest).unwrap()).unwrap())
    .bind(digest_payload(&manifest).unwrap().as_ref())
    .bind(manifest.id.as_ref())
    .execute(&pool).await.unwrap();
    assert_eq!(updated.rows_affected(), 1);
    pool.close().await;
    coordinator
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .expect("matching runtime representation must remain restartable");

    // Both alternate owners exist, so SQLite foreign keys remain satisfied.
    // The second variant also keeps package_id but changes package_version.
    for (package_id, package_version) in [("nomifun.skills", "1.0.0"), ("nomifun.chat", "9.9.9")] {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO plugin_packages (package_id, package_version, manifest_json, manifest_digest, display_json) \
             SELECT package_id, '9.9.9', manifest_json, manifest_digest, display_json FROM plugin_packages \
             WHERE package_id = 'nomifun.chat' AND package_version = '1.0.0' ON CONFLICT DO NOTHING",
        ).execute(&pool).await.unwrap();
        let updated = sqlx::query(
            "UPDATE capability_definitions SET package_id = ?, package_version = ? WHERE capability_id = ?",
        )
        .bind(package_id).bind(package_version).bind(manifest.id.as_ref())
        .execute(&pool).await.unwrap();
        assert_eq!(updated.rows_affected(), 1);
        pool.close().await;

        let error = coordinator
            .bootstrap(&root, BUILD_IDENTITY, &[])
            .await
            .unwrap_err();
        assert!(
            matches!(error, FreshV4RootError::State(ref message)
            if message.contains("capability materialization mismatch")),
            "{error}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn recovery_rejects_replaced_root_before_inspecting_descendants() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("NomiFun");
    let interrupted = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        OneShotFault::at(FreshV4FaultPoint::AfterInitializingMarkerDurable),
        NoAccessAudit,
        FixedClock,
    );
    assert!(matches!(
        interrupted.bootstrap(&root, BUILD_IDENTITY, &[]).await,
        Err(FreshV4RootError::Fault(_))
    ));
    let displaced = parent.path().join("displaced");
    std::fs::rename(&root, &displaced).unwrap();
    std::os::unix::fs::symlink(&displaced, &root).unwrap();
    let audit = Arc::new(RecordingAudit::default());
    let normalized_root = std::fs::canonicalize(parent.path())
        .unwrap()
        .join("NomiFun");
    let coordinator = FreshV4Coordinator::with_ports(
        PreServiceQuiesced,
        NoFaults,
        Arc::clone(&audit),
        FixedClock,
    );

    let error = coordinator
        .bootstrap(&root, BUILD_IDENTITY, &[])
        .await
        .unwrap_err();
    assert!(matches!(error, FreshV4RootError::State(_)), "{error}");
    assert!(
        audit
            .accesses()
            .iter()
            .all(|(_, path)| path == &normalized_root || !path.starts_with(&normalized_root)),
        "recovery must reject a linked root before probing its descendants"
    );
    assert!(parent.path().join(FRESH_V4_PARENT_MARKER_FILE).is_file());
    assert!(
        displaced
            .join(crate::FRESH_V4_INITIALIZING_MARKER_FILE)
            .is_file()
    );
}
