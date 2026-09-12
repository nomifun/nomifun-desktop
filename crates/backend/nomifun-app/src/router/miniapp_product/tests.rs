use super::*;
use nomifun_db::{
    SqliteMiniAppM1Repository, SqliteProviderConnectionRepository,
    SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
    SqliteProviderRepository, init_database_memory, installation_owner_id,
};

const HTML: &str = "<!doctype html><html><head><title>Tasks</title></head><body><input aria-label='Task'><button>Add task</button><script>document.querySelector('button').onclick=()=>document.body.dataset.clicked='yes';</script></body></html>";

async fn fixture() -> (
    nomifun_db::Database,
    tempfile::TempDir,
    MiniAppProductService,
    String,
) {
    let db = init_database_memory().await.unwrap();
    let owner = installation_owner_id(db.pool()).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        MiniAppM1ApplicationService::new_with_root(
            Arc::new(SqliteMiniAppM1Repository::new(db.pool().clone())),
            root.path().join("apps"),
        )
        .unwrap(),
    );
    let model = Arc::new(nomifun_model_invoke::ModelInvokeService::new(
        Arc::new(SqliteProviderRepository::new(db.pool().clone())),
        Arc::new(SqliteProviderModelRepository::new(db.pool().clone())),
        Arc::new(SqliteProviderModelCapabilityRepository::new(
            db.pool().clone(),
        )),
        Arc::new(SqliteProviderConnectionRepository::new(db.pool().clone())),
        [0; 32],
        nomifun_net::http_client(),
        nomifun_model_invoke::AdapterRegistry::new(nomifun_model_invoke::default_adapters()),
    ));
    let service = MiniAppProductService::new(
        MiniAppProductDocuments::new(db.pool().clone()),
        application,
        model,
        root.path().to_path_buf(),
    );
    (db, root, service, owner)
}

fn draft() -> Draft {
    Draft {
        id: uuid::Uuid::now_v7().to_string(),
        revision: 0,
        name: "Daily tasks".into(),
        description: "Track tasks".into(),
        html: HTML.into(),
        service_source: None,
        messages: vec![ChatMessage {
            role: "user".into(),
            content: "Make a checklist".into(),
        }],
        status: "ready".into(),
        error: None,
        miniapp_id: None,
        base_release_digest: None,
        base_source_digest: None,
        updated_at: 1,
        import: None,
    }
}

#[tokio::test]
async fn draft_is_durable_owner_scoped_and_not_a_published_app() {
    let (_db, _root, service, owner) = fixture().await;
    let mut value = draft();
    service.put_draft(&owner, &mut value).await.unwrap();
    let reread = service.draft(&owner, &value.id).await.unwrap();
    assert_eq!(reread.html, HTML);
    assert_eq!(reread.revision, 1);
    assert!(
        service
            .draft(&uuid::Uuid::now_v7().to_string(), &value.id)
            .await
            .is_err()
    );
    assert!(
        service
            .application
            .library(&owner)
            .await
            .unwrap()
            .miniapps
            .is_empty()
    );
    let mut stale = value.clone();
    service.put_draft(&owner, &mut value).await.unwrap();
    assert!(service.put_draft(&owner, &mut stale).await.is_err());
    assert_eq!(service.draft(&owner, &value.id).await.unwrap().revision, 2);
}

#[tokio::test]
async fn save_creates_builds_publishes_and_enables_without_extra_user_steps() {
    let (_db, _root, service, owner) = fixture().await;
    let mut value = draft();
    service.put_draft(&owner, &mut value).await.unwrap();
    let saved = service.save_draft(&owner, &mut value).await.unwrap();
    assert!(matches!(
        saved.miniapp.lifecycle,
        nomifun_api_types::MiniAppLifecycleDto::Enabled
    ));
    assert!(saved.miniapp.releases.active.is_some());
    assert_eq!(
        service
            .application
            .library(&owner)
            .await
            .unwrap()
            .miniapps
            .len(),
        1
    );
    let source = service
        .application
        .source_file(&owner, &saved.miniapp.miniapp_id, "ui/index.html")
        .await
        .unwrap();
    assert_eq!(source.content, HTML);
    let opened = service
        .application
        .open_surface(&owner, &saved.miniapp.miniapp_id)
        .await
        .unwrap();
    assert_eq!(opened.miniapp_id, saved.miniapp.miniapp_id);
    let active = saved.miniapp.releases.active.unwrap().release_digest;
    value.html = HTML.replace("Add task", "Add another task");
    value.base_release_digest = Some("0".repeat(64));
    assert!(service.save_draft(&owner, &mut value).await.is_err());
    assert_eq!(
        service
            .application
            .workshop(&owner, &opened.miniapp_id)
            .await
            .unwrap()
            .miniapp
            .releases
            .active
            .unwrap()
            .release_digest,
        active
    );
}

#[tokio::test]
async fn collection_document_compare_and_swap_does_not_overwrite_other_writes() {
    let (_db, _root, service, owner) = fixture().await;
    assert_eq!(
        service
            .documents
            .put(
                &owner,
                "library",
                0,
                "{\"revision\":1,\"collections\":[],\"items\":{}}"
            )
            .await
            .unwrap(),
        1
    );
    assert!(
        service
            .documents
            .put(&owner, "library", 0, "{}")
            .await
            .is_err()
    );
    assert!(
        service
            .documents
            .put(&owner, "library", 5, "{}")
            .await
            .is_err()
    );
    assert!(
        service
            .documents
            .get(&uuid::Uuid::now_v7().to_string(), "library")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(service.workspace(&owner).await.unwrap().revision, 1);
}

#[tokio::test]
async fn saving_a_draft_does_not_overwrite_source_edited_elsewhere() {
    let (_db, _root, service, owner) = fixture().await;
    let mut value = draft();
    service.put_draft(&owner, &mut value).await.unwrap();
    let saved = service.save_draft(&owner, &mut value).await.unwrap();
    let edited = HTML.replace("Add task", "Edited elsewhere");
    service
        .application
        .replace_source_file(
            &owner,
            ReplaceMiniAppSourceFileRequest {
                miniapp_id: saved.miniapp.miniapp_id.clone(),
                expected_product_revision: saved.miniapp.product_revision,
                project_id: saved.project_id,
                expected_project_revision: saved.project_revision,
                expected_build_generation: saved.build_generation,
                expected_source_snapshot_digest: saved.source_snapshot_digest.unwrap(),
                path: "ui/index.html".into(),
                content: edited.clone(),
            },
        )
        .await
        .unwrap();
    value.html = HTML.replace("Add task", "Outdated draft");
    assert!(matches!(
        service.save_draft(&owner, &mut value).await,
        Err(AppError::RevisionConflict(_))
    ));
    assert_eq!(
        service
            .application
            .source_file(&owner, &saved.miniapp.miniapp_id, "ui/index.html")
            .await
            .unwrap()
            .content,
        edited
    );
}

#[tokio::test]
async fn storage_survives_surface_close_and_reopen() {
    let (_db, _root, service, owner) = fixture().await;
    let mut value = draft();
    service.put_draft(&owner, &mut value).await.unwrap();
    let saved = service.save_draft(&owner, &mut value).await.unwrap();
    let id = &saved.miniapp.miniapp_id;
    let first = service.application.open_surface(&owner, id).await.unwrap();
    let request=serde_json::from_value(serde_json::json!({"call_id":"write-task","target":{"target":"host_kv","request":{"operation":"set","key":"tasks","value":[{"title":"Remember this","done":false}]}}})).unwrap();
    service
        .application
        .surface_bridge_request(
            &owner,
            id,
            &first.surface_capability,
            first.active_release_epoch,
            &first.expected_release_digest,
            request,
        )
        .await
        .unwrap();
    service
        .application
        .close_surface(
            &owner,
            id,
            &first.surface_session_id,
            &first.surface_capability,
        )
        .await
        .unwrap();
    let second = service.application.open_surface(&owner, id).await.unwrap();
    let request=serde_json::from_value(serde_json::json!({"call_id":"read-task","target":{"target":"host_kv","request":{"operation":"get","key":"tasks"}}})).unwrap();
    let result = service
        .application
        .surface_bridge_request(
            &owner,
            id,
            &second.surface_capability,
            second.active_release_epoch,
            &second.expected_release_digest,
            request,
        )
        .await
        .unwrap();
    let json = serde_json::to_value(result).unwrap();
    assert_eq!(json["value"][0]["title"], "Remember this");
}

#[tokio::test]
async fn cancel_retains_the_previous_preview_and_revokes_the_job() {
    let (_db, _root, service, owner) = fixture().await;
    let mut value = draft();
    value.status = "generating".into();
    service.put_draft(&owner, &mut value).await.unwrap();
    let token = CancellationToken::new();
    service
        .jobs
        .lock()
        .await
        .insert(format!("{owner}:{}", value.id), token.clone());
    let state =
        MiniAppM1RouterState::new(service.application.clone()).with_product(service.clone());
    let user = CurrentUser {
        id: nomifun_common::UserId::parse(owner.clone()).unwrap(),
        username: "Owner".into(),
    };
    let _ = cancel(
        State(state),
        Extension(user),
        Path(value.id.clone()),
        Json(ExpectedRevision {
            expected_revision: value.revision,
        }),
    )
    .await
    .unwrap();
    assert!(token.is_cancelled());
    let retained = service.draft(&owner, &value.id).await.unwrap();
    assert_eq!(retained.status, "stopped");
    assert_eq!(retained.html, HTML);
    assert!(
        service
            .documents
            .put(&owner, &format!("draft:{}", value.id), value.revision, "{}")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn sharing_and_backup_are_single_file_importable_and_do_not_replace_the_original() {
    let (_db, root, service, owner) = fixture().await;
    let mut value = draft();
    service.put_draft(&owner, &mut value).await.unwrap();
    let saved = service.save_draft(&owner, &mut value).await.unwrap();
    let state =
        MiniAppM1RouterState::new(service.application.clone()).with_product(service.clone());
    let user = || CurrentUser {
        id: nomifun_common::UserId::parse(owner.clone()).unwrap(),
        username: "Owner".into(),
    };
    for backup in [false, true] {
        let path = root.path().join(if backup {
            "backup.nomiapp"
        } else {
            "share.nomiapp"
        });
        let request =
            serde_json::from_value(serde_json::json!({"destination_path":path,"backup":backup}))
                .unwrap();
        let response = transfer::export_file(
            State(state.clone()),
            Extension(user()),
            Path(saved.miniapp.miniapp_id.clone()),
            Json(request),
        )
        .await
        .unwrap();
        assert!(response.0.data.unwrap().resumed);
        assert!(path.is_file());
        let request = serde_json::from_value(serde_json::json!({"source_path":path})).unwrap();
        let response = transfer::inspect(State(state.clone()), Extension(user()), Json(request))
            .await
            .unwrap();
        let mut imported = response.0.data.unwrap();
        assert_eq!(imported.import.as_ref().unwrap().includes_data, backup);
        let copied = service.save_draft(&owner, &mut imported).await.unwrap();
        assert_ne!(copied.miniapp.miniapp_id, saved.miniapp.miniapp_id);
        assert!(matches!(
            copied.miniapp.lifecycle,
            nomifun_api_types::MiniAppLifecycleDto::Enabled
        ));
    }
    assert_eq!(
        service
            .application
            .library(&owner)
            .await
            .unwrap()
            .miniapps
            .len(),
        3
    );
}

#[tokio::test]
async fn rejected_import_removes_only_its_temporary_copy() {
    use std::io::Write;
    let (_db, root, service, owner) = fixture().await;
    let source = root.path().join("damaged.nomiapp");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&source).unwrap());
    archive
        .start_file("bundle.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"not valid JSON").unwrap();
    archive.finish().unwrap();
    let state =
        MiniAppM1RouterState::new(service.application.clone()).with_product(service.clone());
    let user = CurrentUser {
        id: nomifun_common::UserId::parse(owner.clone()).unwrap(),
        username: "Owner".into(),
    };
    let request = serde_json::from_value(serde_json::json!({"source_path":source})).unwrap();
    assert!(
        transfer::inspect(State(state), Extension(user), Json(request))
            .await
            .is_err()
    );
    assert!(source.is_file());
    assert_eq!(
        std::fs::read_dir(root.path().join("miniapp-imports"))
            .unwrap()
            .count(),
        0
    );
    assert!(
        service
            .documents
            .list(&owner, "draft:")
            .await
            .unwrap()
            .is_empty()
    );
}
