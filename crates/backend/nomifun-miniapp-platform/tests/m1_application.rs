use std::sync::Arc;

use nomifun_api_types::{
    CreateMiniAppProjectRequest, MiniAppKindDto,
    MiniAppProjectSourceStateDto,
};
use nomifun_db::{
    IMiniAppM1Repository, SqliteMiniAppM1Repository, init_database_memory,
    installation_owner_id,
};
use nomifun_miniapp_platform::{
    MiniAppM1ApplicationError, MiniAppM1ApplicationService,
};

#[tokio::test]
async fn owner_scoped_library_create_and_workshop_use_the_new_data_root() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let service = MiniAppM1ApplicationService::new(repository);

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
    assert_eq!(created.source_state, MiniAppProjectSourceStateDto::Empty);
    assert_eq!(created.project_revision, 1);
    assert_eq!(created.build_generation, 0);
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
    let service = MiniAppM1ApplicationService::new(repository);
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

    let error = service
        .workshop(another_owner, &created.miniapp.miniapp_id)
        .await
        .unwrap_err();
    assert!(matches!(error, MiniAppM1ApplicationError::NotFound));
}
