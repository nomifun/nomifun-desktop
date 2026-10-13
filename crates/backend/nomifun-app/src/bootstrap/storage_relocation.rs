//! Offline storage coordinator, before runtime/browser/logging initialization.
use std::path::{Path, PathBuf};
use anyhow::{Context, Result, ensure};
use nomifun_agent_contracts::OperationId;
use nomifun_agent_session::AgentSessionStore;
use nomifun_common::storage_relocation::{self as storage, RelocationPhase, RelocationPlan};
use nomifun_db::sqlx::{sqlite::SqliteConnectOptions, sqlite::SqlitePoolOptions};
#[cfg(test)]
use nomifun_db::relocate_storage_domain_paths as relocate_domain_paths;
use super::environment::{ExistingV3DatabaseProbe, probe_existing_v3_database};

#[cfg(test)]
#[path = "app_storage_relocation_tests.rs"]
mod app_storage_relocation_tests;

fn executable(plan: &RelocationPlan) -> bool {
    matches!(plan.phase, RelocationPhase::AwaitingRestart | RelocationPhase::Copying | RelocationPhase::Verifying)
        || (plan.phase == RelocationPhase::Failed && !plan.cancellable)
}

/// Host entry points use this before starting any worker or native profile.
pub fn prepare_startup_storage_root(requested: PathBuf, work_override: Option<&Path>) -> Result<PathBuf> {
    let root = super::resolve_startup_data_root(requested)?;
    let Some(control) = storage::resolve_control_dir(&root)? else { return Ok(root); };
    if !storage::pending_storage_relocation(&control)?.as_ref().is_some_and(executable) { return Ok(root); }
    tokio::runtime::Builder::new_current_thread().enable_all().build()?
        .block_on(prepare_startup_storage_root_async(root, work_override))
}

/// Embedded hosts use the same coordinator on their existing async runtime.
pub async fn prepare_startup_storage_root_async(requested: PathBuf, work_override: Option<&Path>) -> Result<PathBuf> {
    let root = super::resolve_startup_data_root(requested.clone())?;
    let Some(control) = storage::resolve_control_dir(&root)? else { return Ok(root); };
    // The pointer is already authoritative after a publication crash. Finish
    // its receipt without reopening a previous drive or its old work root.
    if storage::finalize_committed_storage_relocation(&control)? {
        return super::resolve_startup_data_root(requested);
    }
    let Some(plan) = storage::pending_storage_relocation(&control)? else { return Ok(root); };
    if !executable(&plan) { return Ok(root); }
    ensure!(work_override.is_none(), "remove --work-dir before executing a storage relocation");
    // The source locks protect the copy, DB transitions and the single commit
    // pointer. Locks are released before normal boot opens the committed root.
    let server_lock = super::acquire_offline_server_lock(&plan.source.data_dir)?;
    let data_lock = super::acquire_work_root_lock(server_lock.protected_data_dir())?;
    let _work_lock = super::acquire_distinct_work_root_lock(&data_lock, &plan.source.work_dir)?;
    let result = execute(&control).await;
    match result {
        Ok(()) => super::resolve_startup_data_root(requested),
        Err(error) => {
            let failed = storage::mark_storage_relocation_failed(&control, &plan.operation_id, &format!("{error:#}"))?;
            if failed.cancellable {
                storage::require_storage_root_not_sealed(&plan.source.data_dir)?;
                super::record_boot_note(super::BootNoteLevel::Warn, format!("Storage relocation failed; original data retained: {error:#}"));
                Ok(plan.source.data_dir)
            } else { Err(error.context("storage publication is pending recovery; refusing to reopen the old root")) }
        }
    }
}

async fn require_current_database(path: &Path) -> Result<()> {
    ensure!(matches!(probe_existing_v3_database(path).await?, ExistingV3DatabaseProbe::Current),
        "storage relocation requires a healthy current database; migration/reset is not authorized");
    Ok(())
}

async fn execute(control: &Path) -> Result<()> {
    let plan = storage::pending_storage_relocation(control)?.context("storage plan disappeared before offline preparation")?;
    let mut application_ids = None;
    let committed = storage::resolve_storage_location(control)?.is_some_and(|location|
        nomifun_common::paths::paths_equivalent(&location.data_dir, &plan.destination.data_dir));
    // Final publication is governed by its durable verified manifest. A new
    // application version must finish that exact commit before applying its
    // own normal startup schema policy; it must not reopen or rewrite the
    // source database after publication has begun.
    if !committed && plan.cancellable {
        // Inspect stopped application registries under the same offline
        // dataset locks. This never opens the hosting owner or reconciles a
        // recorded process; an unproven process keeps the source untouched.
        let inventory = nomifun_app_hosting::inspect_offline_hosted_apps(&plan.source.data_dir)?;
        if !plan.has_verified_copy() {
            let path = plan.source.data_dir.join("nomifun-backend.db");
            require_current_database(&path).await?;
            let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
                SqliteConnectOptions::new().filename(&path).create_if_missing(false).read_only(true).foreign_keys(true)
            ).await?;
            let validation = async {
                nomifun_db::validate_storage_relocation_paths(&pool, &plan.source.data_dir, &plan.source.work_dir).await?;
                AgentSessionStore::from_pool(pool.clone()).await?
                    .validate_workspace_relocation_with_app_workspaces(&plan.source.data_dir, &plan.source.work_dir, &inventory.app_ids).await?;
                Ok::<(), anyhow::Error>(())
            }.await;
            pool.close().await;
            validation?;
        }
        // Once copied, even a read-only SQLite connection can create or alter
        // source WAL/SHM leaves. Recovery must leave that immutable inventory
        // alone: common preparation still verifies every source byte, and the
        // copied database below still proves schema, canonical idleness and
        // every exact relocation transition before publication.
        application_ids = Some(inventory.app_ids);
    }
    let prepared = match application_ids.as_ref() {
        Some(ids) => storage::prepare_pending_storage_relocation_with_app_registries(control, ids)?,
        None => storage::prepare_pending_storage_relocation(control)?,
    };
    let Some(prepared) = prepared else { return Ok(()); };
    if prepared.preparation_complete {
        storage::commit_storage_relocation(control, &prepared)?;
        return Ok(());
    }
    let application_ids = match application_ids {
        Some(ids) => ids,
        None => nomifun_app_hosting::inspect_offline_hosted_apps(&prepared.source.data_dir)?.app_ids,
    };
    let database_path = prepared.staging_data_dir.join("nomifun-backend.db");
    require_current_database(&database_path).await?;
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(&database_path).create_if_missing(false).foreign_keys(true)
    ).await.context("open copied database without startup migrations")?;
    let transition = async {
        let sessions = AgentSessionStore::from_pool(pool.clone()).await?;
        // Knowledge bindings must still reference the copied source domain
        // while the canonical writer proves its exact resource transition.
        let outcome = sessions.relocate_managed_workspaces_with_app_workspaces(&prepared.source.data_dir, &prepared.source.work_dir,
            &prepared.destination.data_dir, &OperationId::from(prepared.plan.operation_id.clone()), &application_ids).await?;
        nomifun_db::relocate_storage_domain_paths_with_app_workspaces(&pool, &prepared.source.data_dir, &prepared.source.work_dir,
            &prepared.destination.data_dir, &outcome.managed_namespaces, &outcome.app_workspaces, &application_ids).await?;
        let rebased = nomifun_app_hosting::rewrite_offline_hosted_app_registries(&prepared.source.data_dir,
            &prepared.staging_data_dir, &prepared.destination.data_dir)?;
        ensure!(rebased == application_ids, "copied application registry inventory changed during storage relocation");
        sessions.validate_workspace_relocation_with_app_workspaces(&prepared.destination.data_dir,
            &prepared.destination.work_dir, &application_ids).await?;
        nomifun_db::validate_id_data_contract(&pool).await?;
        let check: String = nomifun_db::sqlx::query_scalar("PRAGMA quick_check").fetch_one(&pool).await?;
        ensure!(check == "ok", "copied database failed SQLite integrity validation: {check}");
        nomifun_db::sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&pool).await?;
        Ok::<(), anyhow::Error>(())
    }.await;
    // Close even on failure: no connection or WAL writer survives publication.
    pool.close().await;
    transition?;
    require_current_database(&database_path).await?;
    storage::commit_storage_relocation_with_app_registries(control, &prepared, &application_ids)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn text(path: &Path) -> String { path.to_string_lossy().into_owned() }
    const GENERATION: &str = "0190f5fe-7c00-7a00-8000-0000000000f0";
    const SESSION: &str = "0190f5fe-7c00-7a00-8000-0000000000f1";
    const KB: &str = "0190f5fe-7c00-7a00-8000-0000000000f2";

    async fn checkpoint_fixture(pool: &nomifun_db::sqlx::SqlitePool) {
        let status: (i64, i64, i64) = nomifun_db::sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(pool).await.unwrap();
        assert_eq!(status, (0, 0, 0), "fixture WAL must be fully checkpointed before offline migration");
    }

    fn assert_source_database_unchanged(source: &Path, before: &[u8]) {
        let after = fs::read(source.join("nomifun-backend.db")).unwrap();
        let offsets = before.iter().zip(&after).enumerate().filter(|(_, (a, b))| a != b)
            .map(|(offset, _)| offset).take(8).collect::<Vec<_>>();
        assert!(after == before, "offline relocation changed source DB: sizes {} / {}, first differing offsets {offsets:?}",
            before.len(), after.len());
    }

    fn source_payload_leaves(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn collect(root: &Path, relative: &Path, leaves: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(root.join(relative)).unwrap() {
                let entry = entry.unwrap();
                let relative = relative.join(entry.file_name());
                // Root lifecycle markers and locks belong to the coordinator;
                // application files, registries and all DB leaves remain data.
                if relative.components().count() == 1 && matches!(relative.to_str(),
                    Some(".nomifun-work-root.lock" | "server.lock" | "server.lock.info" | "nomifun-backend.db.migrate.lock"
                        | ".nomifun-storage-location.json" | ".nomifun-storage-relocated.json" | "dir-config.json"
                        | "dataset-v3.json" | ".nomifun-work-root-owner.json" | ".nomifun-work-root-binding.json")) { continue; }
                if entry.file_type().unwrap().is_dir() { collect(root, &relative, leaves); }
                else { leaves.insert(relative.clone(), fs::read(root.join(relative)).unwrap()); }
            }
        }
        let mut leaves = std::collections::BTreeMap::new();
        collect(root, Path::new(""), &mut leaves);
        leaves
    }

    async fn seed(source: &Path, work: &Path) {
        fs::create_dir_all(source).unwrap(); fs::create_dir_all(work).unwrap();
        let database = nomifun_db::init_database(&source.join("nomifun-backend.db")).await.unwrap();
        nomifun_db::sqlx::query("UPDATE system_settings SET language='zh-CN' WHERE singleton_key='system'")
            .execute(database.pool()).await.unwrap();
        // Model a stopped source process: finish the fixture writer's WAL
        // before capturing immutable source bytes for an offline operation.
        checkpoint_fixture(database.pool()).await;
        database.close().await;
        fs::write(source.join("storage-generation"), GENERATION).unwrap();
        nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(source, work, GENERATION).unwrap();
    }

    #[tokio::test]
    async fn storage_relocation_boot_moves_dataset_and_separate_managed_workspaces_once() {
        let fixture = tempfile::tempdir().unwrap();
        let base = nomifun_common::paths::canonicalize_simplified(fixture.path()).unwrap();
        let source = base.join("source"); let work = base.join("work"); let target = base.join("target");
        let control = base.join("control"); let external = base.join("external-project");
        seed(&source, &work).await;
        for (relative, bytes) in [("attachments/upload.bin", b"attachment".as_slice()),
            ("encryption_key", b"secret"), ("browser-v4/agent-sessions/profile/state", b"browser"),
            ("plugin-data/plugin/settings.json", b"settings"), ("config/custom.json", b"config")] {
            let path = source.join(relative); fs::create_dir_all(path.parent().unwrap()).unwrap(); fs::write(path, bytes).unwrap();
        }
        let managed = work.join("conversations").join(SESSION).join("result.txt");
        fs::create_dir_all(managed.parent().unwrap()).unwrap(); fs::write(&managed, b"managed-output").unwrap();
        fs::create_dir(&external).unwrap(); fs::write(external.join("project.txt"), b"external").unwrap();
        let before_database = fs::read(source.join("nomifun-backend.db")).unwrap();
        let plan = storage::request_storage_relocation(&control, &source, &work, &target).unwrap();
        assert_eq!(super::super::resolve_startup_data_root(source.clone()).unwrap(), source);
        storage::start_storage_relocation(&control, &plan.operation_id).unwrap();
        assert_eq!(prepare_startup_storage_root_async(source.clone(), None).await.unwrap(), target);
        assert_eq!(prepare_startup_storage_root_async(source.clone(), None).await.unwrap(), target);
        assert_source_database_unchanged(&source, &before_database);
        assert_eq!(fs::read(target.join("attachments/upload.bin")).unwrap(), b"attachment");
        assert_eq!(fs::read(target.join("encryption_key")).unwrap(), b"secret");
        assert_eq!(fs::read(target.join("plugin-data/plugin/settings.json")).unwrap(), b"settings");
        assert_eq!(fs::read(target.join("browser-v4/agent-sessions/profile/state")).unwrap(), b"browser");
        assert_eq!(fs::read(target.join("conversations").join(SESSION).join("result.txt")).unwrap(), b"managed-output");
        assert_eq!(fs::read(&managed).unwrap(), b"managed-output");
        assert_eq!(fs::read(external.join("project.txt")).unwrap(), b"external");
        assert_eq!(fs::read_to_string(target.join("storage-generation")).unwrap(), GENERATION);
        assert!(storage::require_storage_root_not_sealed(&source).is_err());
        let db = nomifun_db::open_database_for_backup(&target.join("nomifun-backend.db")).await.unwrap();
        assert_eq!(nomifun_db::sqlx::query_scalar::<_,String>("SELECT language FROM system_settings WHERE singleton_key='system'").fetch_one(db.pool()).await.unwrap(), "zh-CN");
        db.close().await;
    }

    #[tokio::test]
    async fn storage_relocation_publication_recovery_uses_verified_target_without_source_database() {
        let fixture = tempfile::tempdir().unwrap();
        let base = nomifun_common::paths::canonicalize_simplified(fixture.path()).unwrap();
        let source = base.join("source");
        let target = base.join("target");
        let control = base.join("control");
        seed(&source, &source).await;
        let plan = storage::request_storage_relocation(&control, &source, &source, &target).unwrap();
        storage::start_storage_relocation(&control, &plan.operation_id).unwrap();
        let prepared = storage::prepare_pending_storage_relocation(&control).unwrap().unwrap();
        // Fail after the final manifest and rename, before the pointer commit.
        let blocked_seal = source.join(".nomifun-storage-relocated.json");
        fs::create_dir(&blocked_seal).unwrap();
        assert!(storage::commit_storage_relocation(&control, &prepared).is_err());
        drop(prepared);
        fs::remove_dir(&blocked_seal).unwrap();
        assert!(!storage::pending_storage_relocation(&control).unwrap().unwrap().cancellable);
        let copied = fs::read(target.join("nomifun-backend.db")).unwrap();
        // Once publication begins, source database I/O or a new schema policy
        // cannot authorize replaying callbacks or block the verified commit.
        fs::write(source.join("nomifun-backend.db"), b"source unavailable after publication").unwrap();
        assert_eq!(prepare_startup_storage_root_async(source.clone(), None).await.unwrap(), target);
        assert_eq!(fs::read(target.join("nomifun-backend.db")).unwrap(), copied);
        assert!(storage::require_storage_root_not_sealed(&source).is_err());
        require_current_database(&target.join("nomifun-backend.db")).await.unwrap();
    }

    #[tokio::test]
    async fn storage_relocation_boot_resumes_copied_app_registry_rebase_before_publication() {
        use nomifun_app_hosting::{AppRecord, AppSettings, AppState, ImportKind};
        let fixture = tempfile::tempdir().unwrap();
        let base = nomifun_common::paths::canonicalize_simplified(fixture.path()).unwrap();
        let source = base.join("source"); let target = base.join("target"); let control = base.join("control");
        seed(&source, &source).await;
        let id = uuid::Uuid::now_v7().to_string();
        let directory = source.join("app-hosting").join("apps").join(&id);
        let registry = source.join("app-hosting").join("registry").join(format!("{id}.json"));
        fs::create_dir_all(&directory).unwrap(); fs::create_dir_all(registry.parent().unwrap()).unwrap();
        fs::write(directory.join("index.html"), b"retained application source").unwrap();
        let app = AppRecord { id: id.clone(), name: "Interrupted application relocation".into(), description: "Preserved configuration".into(),
            tags: Vec::new(), development_session_id: None, directory, import_kind: ImportKind::Source,
            settings: AppSettings::default(), state: AppState::Stopped, operation: None, last_error: None,
            output: String::new(), pid: None, preview: false, prepared: false };
        fs::write(&registry, serde_json::to_vec_pretty(&serde_json::json!({"record":app,"identity":null})).unwrap()).unwrap();
        let before_registry = fs::read(&registry).unwrap();
        let before_database = fs::read(source.join("nomifun-backend.db")).unwrap();
        let plan = storage::request_storage_relocation(&control, &source, &source, &target).unwrap();
        storage::start_storage_relocation(&control, &plan.operation_id).unwrap();
        let ids = nomifun_app_hosting::inspect_offline_hosted_apps(&source).unwrap().app_ids;
        let prepared = storage::prepare_pending_storage_relocation_with_app_registries(&control, &ids).unwrap().unwrap();
        assert_eq!(nomifun_app_hosting::rewrite_offline_hosted_app_registries(&source, &prepared.staging_data_dir, &target).unwrap(), ids);
        // Dropping the prepared token models the caller exiting after its real
        // registry callback and before commit. The next boot owns recovery.
        drop(prepared);
        assert!(!target.exists());
        let source_leaves = source_payload_leaves(&source);
        let resumed = prepare_startup_storage_root_async(source.clone(), None).await.unwrap();
        let status = storage::storage_relocation_status(&control).unwrap();
        assert_eq!(resumed, target, "the interrupted App relocation must publish successfully: {status:?}");
        assert_eq!(source_payload_leaves(&source), source_leaves, "resume must not create source SQLite sidecars or change payload leaves");
        assert_eq!(prepare_startup_storage_root_async(source.clone(), None).await.unwrap(), target);
        assert_eq!(source_payload_leaves(&source), source_leaves, "a repeated committed boot must retain the same source payload leaves");
        assert_source_database_unchanged(&source, &before_database);
        assert_eq!(fs::read(registry).unwrap(), before_registry);
        let relocated: serde_json::Value = serde_json::from_slice(&fs::read(target.join("app-hosting").join("registry").join(format!("{id}.json"))).unwrap()).unwrap();
        assert_eq!(relocated["record"]["directory"], serde_json::json!(target.join("app-hosting").join("apps").join(&id)));
        assert_eq!(fs::read(target.join("app-hosting").join("apps").join(id).join("index.html")).unwrap(), b"retained application source");
    }

    #[tokio::test]
    async fn storage_relocation_invalid_owned_domain_keeps_original_usable() {
        let fixture = tempfile::tempdir().unwrap();
        let base = nomifun_common::paths::canonicalize_simplified(fixture.path()).unwrap();
        let source = base.join("source"); let target = base.join("target"); let control = base.join("control");
        seed(&source, &source).await;
        let db = nomifun_db::init_database(&source.join("nomifun-backend.db")).await.unwrap();
        nomifun_db::sqlx::query("INSERT INTO knowledge_bases(knowledge_base_id,name,root_path,managed,created_at,updated_at) VALUES(?,'unsafe',?,1,1,1)")
            .bind(KB).bind(text(&base.join("external-project"))).execute(db.pool()).await.unwrap();
        checkpoint_fixture(db.pool()).await;
        db.close().await;
        let before = fs::read(source.join("nomifun-backend.db")).unwrap();
        let plan = storage::request_storage_relocation(&control, &source, &source, &target).unwrap();
        storage::start_storage_relocation(&control, &plan.operation_id).unwrap();
        assert_eq!(prepare_startup_storage_root_async(source.clone(), None).await.unwrap(), source);
        assert_source_database_unchanged(&source, &before);
        assert!(!target.exists());
        assert!(storage::require_storage_root_not_sealed(&source).is_ok());
        let status = storage::storage_relocation_status(&control).unwrap().unwrap();
        assert_eq!(status.phase, RelocationPhase::Failed); assert!(status.cancellable); assert!(status.error.is_some());
        assert_eq!(prepare_startup_storage_root_async(source.clone(), None).await.unwrap(), source);
    }

    #[tokio::test]
    async fn storage_relocation_domain_uses_ownership_and_preserves_external_knowledge_paths() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let fixture = tempfile::tempdir().unwrap(); let source = fixture.path().join("source"); let target = fixture.path().join("target");
        let managed_path = source.join("knowledge").join(KB);
        let external_id = "0190f5fe-7c00-7a00-8000-0000000000f3";
        let external_path = source.join("knowledge").join(external_id);
        for (id, path, managed) in [(KB, &managed_path, 1), (external_id, &external_path, 0)] {
            nomifun_db::sqlx::query("INSERT INTO knowledge_bases(knowledge_base_id,name,root_path,managed,created_at,updated_at) VALUES(?,'kb',?,?,1,1)")
                .bind(id).bind(text(path)).bind(managed).execute(db.pool()).await.unwrap();
        }
        relocate_domain_paths(db.pool(), &source, &source, &target, &[]).await.unwrap();
        let paths: Vec<String> = nomifun_db::sqlx::query_scalar("SELECT root_path FROM knowledge_bases ORDER BY knowledge_base_id").fetch_all(db.pool()).await.unwrap();
        assert_eq!(paths, vec![text(&target.join("knowledge").join(KB)), text(&external_path)]);
        db.close().await;
    }

    #[tokio::test]
    async fn storage_relocation_canonical_namespace_updates_owned_caches_and_preserves_external_paths() {
        use std::collections::{BTreeMap, BTreeSet};
        use nomifun_agent_contracts::{AgentBindingValue, AgentHandoffMode, AgentSessionLiveRecord,
            AgentSessionMetadata, PresetRevisionRef, PrincipalRef, ResolvedSnapshotRef,
            SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef, StrictJsonValue,
            TypedResourceBinding, resource_definition_id};
        use nomifun_agent_session::{CreateSessionRequest, ReplaceSessionAgentBinding};
        use nomifun_db::sqlx;
        let db = nomifun_db::init_database_memory().await.unwrap();
        let owner_id: String = sqlx::query_scalar("SELECT owner_user_id FROM installation_identity")
            .fetch_one(db.pool()).await.unwrap();
        let owner = PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.clone() };
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source"); let target = fixture.path().join("target");
        let mut workspace = TypedResourceBinding {
            binding_id: "unassigned".into(), resource_kind: "workspace".into(),
            resource_id: "default-workspace".into(), owner_id: owner_id.clone(),
            operations: BTreeSet::from(["read".into()]), connection_config_ref: None,
            typed_parameters: BTreeMap::from([("workspace_root".into(), text(&source))]),
        };
        workspace.binding_id = resource_definition_id(&workspace).unwrap();
        let binding = AgentBindingValue {
            preset_revision_ref: PresetRevisionRef { preset_id: "coding.codex".into(), revision: 1,
                revision_digest: "a".repeat(64).into() },
            resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: "snapshot-source".into(),
                snapshot_digest: "b".repeat(64).into() },
            typed_resource_bindings: vec![workspace.clone()], binding_version: 1,
        };
        let store = AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
        let created = store.create_session(CreateSessionRequest::new(AgentSessionLiveRecord {
            agent_session_id: SESSION.into(), owner_ref: owner.clone(),
            metadata: AgentSessionMetadata { purpose: Default::default(), app_project_id: None, title: None,
                pinned: false, archived: false, reasoning_effort: None },
            agent_binding: binding.clone(), remote_binding_provenance: None,
            parent_session_id: None, fork_base_payload_id: None, next_seq: 1,
        }, 1_788_000_000_000, "storage-cache-create", "session-api".into(),
            "storage-cache-create".into(), "storage-cache-create".into())).await.unwrap();
        store.append_event(&SessionEventAppend {
            agent_session_id: SESSION.into(), event_id: "storage-cache-ready".into(),
            producer_id: "runtime-supervisor".into(), idempotency_key: "storage-cache-ready".into(),
            semantic_event: SemanticSessionEventDraft { kind: SessionEventKind("session/ready".into()), kind_version: 1,
                correlation_id: "storage-cache-ready".into(), causation_event_id: Some(created.opening_ack.event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::json!({}))) },
        }).await.unwrap();
        // Earlier Agent switches could record this product-owned directory as
        // selected. Only its immutable opening definition proves ownership.
        let owned = source.join("conversations").join(SESSION);
        workspace.resource_id = "selected-workspace:owned".into();
        workspace.typed_parameters.insert("workspace_root".into(), text(&owned));
        workspace.binding_id = resource_definition_id(&workspace).unwrap();
        let mut replacement = binding.clone();
        replacement.preset_revision_ref.preset_id = "coding.replacement".into();
        replacement.preset_revision_ref.revision_digest = "c".repeat(64).into();
        replacement.resolved_snapshot_ref = ResolvedSnapshotRef { snapshot_id: "snapshot-replacement".into(),
            snapshot_digest: "d".repeat(64).into() };
        replacement.typed_resource_bindings = vec![workspace]; replacement.binding_version += 1;
        store.replace_session_agent_binding(&owner, &SESSION.into(), ReplaceSessionAgentBinding {
            expected: binding, replacement, previous_agent_label: "Source".into(), next_agent_label: "Target".into(),
            transition_id: uuid::Uuid::now_v7().to_string().into(), request_digest: "9".repeat(64).into(),
            idempotency_key: "storage-cache-agent-switch".into(), handoff_mode: AgentHandoffMode::ContextOnly,
            handoff: None, initial_active_capability_ids: vec!["coding.workspace".into()],
        }).await.unwrap();
        let external = source.join("conversations").join(uuid::Uuid::now_v7().to_string());
        for path in [&owned, &external] {
            sqlx::query("INSERT INTO terminal_sessions(terminal_id,name,cwd,command,created_at,updated_at,user_id) VALUES(?,'cache',?,'shell',1,1,?)")
                .bind(uuid::Uuid::now_v7().to_string()).bind(text(path)).bind(&owner_id).execute(db.pool()).await.unwrap();
            sqlx::query("INSERT INTO channel_sessions(channel_session_id,channel_user_id,agent_type,conversation_id,workspace,created_at,last_activity) VALUES(?,?,'nomi',?,?,1,1)")
                .bind(uuid::Uuid::now_v7().to_string()).bind(uuid::Uuid::now_v7().to_string())
                .bind(SESSION).bind(text(path)).execute(db.pool()).await.unwrap();
            sqlx::query("INSERT INTO knowledge_bindings(knowledge_binding_id,target_kind,target_workpath,updated_at) VALUES(?,'workpath',?,1)")
                .bind(uuid::Uuid::now_v7().to_string()).bind(text(path)).execute(db.pool()).await.unwrap();
        }
        let outcome = store.relocate_managed_workspaces(&source, &source, &target,
            &OperationId::from(uuid::Uuid::now_v7().to_string())).await.unwrap();
        assert_eq!(outcome.managed_namespaces.len(), 1);
        relocate_domain_paths(db.pool(), &source, &source, &target, &outcome.managed_namespaces).await.unwrap();
        let moved = text(&target.join("conversations").join(SESSION));
        for (table, column) in [("terminal_sessions", "cwd"), ("channel_sessions", "workspace"),
            ("knowledge_bindings", "target_workpath")] {
            let paths: Vec<String> = sqlx::query_scalar(&format!("SELECT {column} FROM {table} ORDER BY id"))
                .fetch_all(db.pool()).await.unwrap();
            assert_eq!(paths, vec![moved.clone(), text(&external)], "{table}");
        }
        store.validate_workspace_relocation(&target, &target).await.unwrap();
        db.close().await;
    }
}
