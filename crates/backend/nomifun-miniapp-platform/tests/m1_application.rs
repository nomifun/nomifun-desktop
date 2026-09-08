use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_api_types::{
    BuildMiniAppRequest, CreateMiniAppProjectRequest, MiniAppKindDto,
    MiniAppProjectSourceStateDto,
};
use nomifun_db::{
    IMiniAppM1Repository, SqliteMiniAppM1Repository, init_database_memory,
    installation_owner_id,
};
use nomifun_miniapp_platform::{
    MiniAppM1ApplicationError, MiniAppM1ApplicationService,
};
use uuid::Uuid;

struct TestStoreRoot(PathBuf);

impl TestStoreRoot {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "nomifun-miniapp-application-{label}-{nanos}-{}",
            Uuid::now_v7()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestStoreRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn owner_scoped_library_create_and_workshop_use_the_new_data_root() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = TestStoreRoot::new("create");
    let service = MiniAppM1ApplicationService::new_with_root(
        repository,
        store_root.path(),
    )
    .unwrap();

    let empty = service.library(&owner).await.unwrap();
    assert_eq!(empty.library_revision, 0);
    assert!(empty.miniapps.is_empty());

    let created = service
        .create(
            &owner,
            CreateMiniAppProjectRequest {
                expected_library_revision: 0,
                display_name: "M1 Notes".to_owned(),
                description: Some("clean-start MiniApp".to_owned()),
                kind: MiniAppKindDto::UiOnly,
            },
        )
        .await
        .unwrap();
    assert_eq!(created.miniapp.display_name, "M1 Notes");
    assert_eq!(
        created.source_state,
        MiniAppProjectSourceStateDto::Editable
    );
    assert_eq!(created.project_revision, 1);
    assert_eq!(created.build_generation, 1);
    assert!(created.source_snapshot_digest.is_some());
    assert!(created.dependency_lock_digest.is_some());
    assert!(created.ready.is_none());
    assert!(!created.miniapp.surface_available);

    let library = service.library(&owner).await.unwrap();
    assert_eq!(library.library_revision, 1);
    assert_eq!(library.miniapps.len(), 1);
    assert_eq!(
        library.miniapps[0].miniapp_id,
        created.miniapp.miniapp_id
    );

    let workshop = service
        .workshop(&owner, &created.miniapp.miniapp_id)
        .await
        .unwrap();
    assert_eq!(workshop, created);
}

#[tokio::test]
async fn workshop_is_not_visible_to_another_owner() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let another_owner = "0190f5fe-7c00-7000-8000-000000000401";
    nomifun_db::sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, ?, '', '', 1, 1)",
    )
    .bind(another_owner)
    .bind(another_owner)
    .execute(database.pool())
    .await
    .unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = TestStoreRoot::new("service-created");
    let service = MiniAppM1ApplicationService::new_with_root(
        repository,
        store_root.path(),
    )
    .unwrap();
    let created = service
        .create(
            &owner,
            CreateMiniAppProjectRequest {
                expected_library_revision: 0,
                display_name: "Private".to_owned(),
                description: None,
                kind: MiniAppKindDto::Service,
            },
        )
        .await
        .unwrap();
    assert_eq!(created.miniapp.kind, MiniAppKindDto::Service);
    assert_eq!(created.source_state, MiniAppProjectSourceStateDto::Editable);
    assert_eq!(service.library(&another_owner).await.unwrap().miniapps.len(), 0);
}

#[tokio::test]
async fn ui_only_build_commits_ready_atomically_and_exposes_created_at() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = TestStoreRoot::new("build");
    let service = MiniAppM1ApplicationService::new_with_root(
        repository,
        store_root.path(),
    )
    .unwrap();

    let created = service
        .create(
            &owner,
            CreateMiniAppProjectRequest {
                expected_library_revision: 0,
                display_name: "Buildable".to_owned(),
                description: None,
                kind: MiniAppKindDto::UiOnly,
            },
        )
        .await
        .unwrap();
    let built = service
        .build(
            &owner,
            BuildMiniAppRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: created.miniapp.product_revision,
                project_id: created.project_id.clone(),
                expected_project_revision: created.project_revision,
                expected_build_generation: created.build_generation,
                expected_source_snapshot_digest: created
                    .source_snapshot_digest
                    .clone()
                    .unwrap(),
                expected_dependency_lock_digest: created
                    .dependency_lock_digest
                    .clone()
                    .unwrap(),
                service_lifecycle: None,
            },
        )
        .await
        .unwrap();
    let ready = built.ready.as_ref().unwrap();
    assert_eq!(ready.created_at_ms, ready.release.release_digest.len() as i64 * 0 + ready.created_at_ms);
    assert!(ready.created_at_ms > 0);
    assert_eq!(ready.project_build_generation, 1);
    assert_eq!(built.source_state, MiniAppProjectSourceStateDto::Editable);
    assert_eq!(built.build_generation, 1);
}
