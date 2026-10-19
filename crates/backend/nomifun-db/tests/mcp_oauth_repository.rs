use nomifun_db::{
    CreateMcpServerParams, DbError, IMcpOAuthRepository, IMcpServerRepository,
    McpOAuthClientParams, McpOAuthGrantParams, McpOAuthGrantRefreshParams,
    SqliteMcpOAuthRepository, SqliteMcpServerRepository, init_database_memory,
};

struct Fixture {
    db: nomifun_db::Database,
    repo: SqliteMcpOAuthRepository,
    owner: String,
    server: String,
    client: McpOAuthClientParams,
}

impl Fixture {
    async fn new() -> Self {
        let db = init_database_memory().await.unwrap();
        Self::with_database(db).await
    }

    async fn with_database(db: nomifun_db::Database) -> Self {
        let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
        let server = SqliteMcpServerRepository::new(db.pool().clone()).create(CreateMcpServerParams {
            name: "oauth-test", description: None, enabled: true, transport_type: "http",
            transport_config: r#"{"url":"https://resource.example/mcp"}"#,
            tools: None, original_json: None, builtin: false,
        }).await.unwrap().mcp_server_id;
        let client = McpOAuthClientParams {
            client_ref: uuid::Uuid::now_v7().to_string(), owner_id: owner.clone(),
            issuer: "https://auth.example".into(), profile_key: "public-loopback".into(),
            registration_kind: "dynamic".into(), client_id: "registered-client".into(),
            token_endpoint_auth_method: "none".into(), redirect_uris_json: r#"["http://127.0.0.1/callback"]"#.into(),
            client_secret_encrypted: None, client_secret_expires_at: None,
        };
        let repo = SqliteMcpOAuthRepository::new(db.pool().clone());
        repo.upsert_client(client.clone()).await.unwrap();
        Self { db, repo, owner, server, client }
    }

    fn grant(&self) -> McpOAuthGrantParams {
        McpOAuthGrantParams {
            grant_id: uuid::Uuid::now_v7().to_string(), owner_id: self.owner.clone(), server_id: self.server.clone(),
            connection_config_ref: "mcp-connection:test".into(), issuer: self.client.issuer.clone(),
            resource: "https://resource.example/mcp".into(), client_ref: self.client.client_ref.clone(),
            scopes_json: r#"["mcp:connect"]"#.into(), token_encrypted: "encrypted-token-envelope".into(),
            expires_at: Some(20_000), obtained_at: 10_000,
        }
    }

    fn refresh(&self, grant_id: &str, revision: i64) -> McpOAuthGrantRefreshParams {
        McpOAuthGrantRefreshParams { owner_id: self.owner.clone(), grant_id: grant_id.into(), expected_revision: revision,
            scopes_json: r#"["mcp:connect"]"#.into(), token_encrypted: "rotated-ciphertext".into(),
            expires_at: Some(30_000), obtained_at: 20_000 }
    }
}

#[tokio::test]
async fn refresh_is_atomic_and_cannot_resurrect_deleted_or_replaced_grants() {
    let f = Fixture::new().await;
    let first = f.repo.replace_grant(f.grant()).await.unwrap();
    let (a, b) = tokio::join!(
        f.repo.compare_and_swap_grant(f.refresh(&first.grant_id, 1)),
        f.repo.compare_and_swap_grant(f.refresh(&first.grant_id, 1)),
    );
    let results = [a.unwrap(), b.unwrap()];
    assert_eq!(results.iter().filter(|row| row.is_some()).count(), 1);
    let refreshed = results.into_iter().flatten().next().unwrap();
    assert_eq!(refreshed.revision, 2);
    assert_eq!(refreshed.grant_id, first.grant_id);
    assert_eq!(refreshed.connection_config_ref, first.connection_config_ref);

    let second = f.repo.replace_grant(f.grant()).await.unwrap();
    assert_ne!(second.grant_id, first.grant_id);
    assert_eq!(second.revision, 1);
    assert!(f.repo.compare_and_swap_grant(f.refresh(&first.grant_id, 2)).await.unwrap().is_none());
    f.repo.delete_grant(&f.owner, &f.server).await.unwrap();
    f.repo.delete_grant(&f.owner, &f.server).await.unwrap();
    assert!(f.repo.compare_and_swap_grant(f.refresh(&second.grant_id, 1)).await.unwrap().is_none());
    assert!(f.repo.get_grant(&f.owner, &f.server).await.unwrap().is_none());
}

#[tokio::test]
async fn registrations_are_immutable_and_new_registrations_preserve_old_grant_identity() {
    let f = Fixture::new().await;
    let grant = f.repo.replace_grant(f.grant()).await.unwrap();
    let mut next = f.client.clone();
    next.client_ref = uuid::Uuid::now_v7().to_string();
    next.client_id = "new-registration".into();
    let next = f.repo.upsert_client(next).await.unwrap();
    assert_eq!(f.repo.get_client(&f.owner, &f.client.issuer, &f.client.profile_key).await.unwrap().unwrap().client_ref, next.client_ref);
    assert_eq!(f.repo.get_client_by_ref(&f.owner, &grant.client_ref).await.unwrap().unwrap().client_id, "registered-client");
    let mut invalid = f.client.clone();
    invalid.client_id = "overwrite-identity".into();
    assert!(matches!(f.repo.upsert_client(invalid).await, Err(DbError::Conflict(_))));
    assert_eq!(f.repo.get_grant(&f.owner, &f.server).await.unwrap().unwrap().grant_id, grant.grant_id);
}

#[tokio::test]
async fn grant_replacement_prunes_the_old_registration_and_logout_keeps_the_current_cache() {
    let f = Fixture::new().await;
    let old_grant = f.repo.replace_grant(f.grant()).await.unwrap();
    let mut next = f.client.clone();
    next.client_ref = uuid::Uuid::now_v7().to_string();
    next.client_id = "replacement-registration".into();
    let next = f.repo.upsert_client(next).await.unwrap();
    assert!(f.repo.get_client_by_ref(&f.owner, &old_grant.client_ref).await.unwrap().is_some(),
        "a live grant must keep its exact original registration");

    let mut new_grant = f.grant();
    new_grant.client_ref = next.client_ref.clone();
    let new_grant = f.repo.replace_grant(new_grant).await.unwrap();
    assert!(f.repo.get_client_by_ref(&f.owner, &old_grant.client_ref).await.unwrap().is_none(),
        "the superseded registration has no grant or cache role after replacement");
    assert_eq!(f.repo.get_grant(&f.owner, &f.server).await.unwrap().unwrap().client_ref, new_grant.client_ref);

    f.repo.delete_grant(&f.owner, &f.server).await.unwrap();
    assert_eq!(f.repo.get_client(&f.owner, &next.issuer, &next.profile_key).await.unwrap().unwrap(), next,
        "logout retains the current reusable client registration");
}

#[tokio::test]
async fn new_client_prunes_only_unreferenced_history_and_preserves_each_profiles_current_cache() {
    let f = Fixture::new().await;
    let mut other_profile = f.client.clone();
    other_profile.client_ref = uuid::Uuid::now_v7().to_string();
    other_profile.profile_key = "web-callback".into();
    let other_profile = f.repo.upsert_client(other_profile).await.unwrap();
    let mut next = f.client.clone();
    next.client_ref = uuid::Uuid::now_v7().to_string();
    next.client_id = "new-unbound-registration".into();
    let next = f.repo.upsert_client(next).await.unwrap();

    assert!(f.repo.get_client_by_ref(&f.owner, &f.client.client_ref).await.unwrap().is_none());
    assert_eq!(f.repo.get_client(&f.owner, &next.issuer, &next.profile_key).await.unwrap().unwrap(), next);
    assert_eq!(f.repo.get_client(&f.owner, &other_profile.issuer, &other_profile.profile_key).await.unwrap().unwrap(), other_profile,
        "an unbound current cache in another profile is still useful");
}

#[tokio::test]
async fn registration_cache_order_survives_wall_clock_rollback_without_deleting_the_new_client() {
    let f = Fixture::new().await;
    let future = nomifun_common::now_ms() + 60_000;
    sqlx::query("UPDATE mcp_oauth_clients SET created_at=? WHERE client_ref=?")
        .bind(future).bind(&f.client.client_ref).execute(f.db.pool()).await.unwrap();
    let mut next = f.client.clone();
    next.client_ref = uuid::Uuid::now_v7().to_string();
    next.client_id = "registered-after-clock-rollback".into();
    let next = f.repo.upsert_client(next).await.unwrap();
    assert_eq!(next.created_at, future + 1);
    assert_eq!(next.updated_at, next.created_at);
    assert_eq!(f.repo.get_client(&f.owner, &next.issuer, &next.profile_key).await.unwrap().unwrap(), next);
    assert!(f.repo.get_client_by_ref(&f.owner, &f.client.client_ref).await.unwrap().is_none());
    let mut grant = f.grant();
    grant.client_ref = next.client_ref.clone();
    assert_eq!(f.repo.replace_grant(grant).await.unwrap().client_ref, next.client_ref);
}

#[tokio::test]
async fn exhausted_registration_clock_fails_closed_without_changing_the_existing_cache() {
    let f = Fixture::new().await;
    sqlx::query("UPDATE mcp_oauth_clients SET created_at=? WHERE client_ref=?")
        .bind(i64::MAX).bind(&f.client.client_ref).execute(f.db.pool()).await.unwrap();
    let before = f.repo.get_client_by_ref(&f.owner, &f.client.client_ref).await.unwrap().unwrap();
    let mut next = f.client.clone();
    next.client_ref = uuid::Uuid::now_v7().to_string();
    next.client_id = "cannot-allocate-another-cache-clock".into();
    let next_ref = next.client_ref.clone();
    assert!(matches!(f.repo.upsert_client(next).await, Err(DbError::Conflict(_))));
    assert_eq!(f.repo.get_client(&f.owner, &before.issuer, &before.profile_key).await.unwrap().unwrap(), before);
    assert!(f.repo.get_client_by_ref(&f.owner, &next_ref).await.unwrap().is_none());
}

#[tokio::test]
async fn grant_enforces_owner_issuer_and_live_server_boundaries() {
    let f = Fixture::new().await;
    let other = uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES(?,'other','hash',1,1)")
        .bind(&other).execute(f.db.pool()).await.unwrap();
    let mut wrong_owner = f.grant();
    wrong_owner.owner_id = other.clone();
    assert!(f.repo.replace_grant(wrong_owner).await.is_err());
    let mut wrong_issuer = f.grant();
    wrong_issuer.issuer = "https://different.example".into();
    assert!(f.repo.replace_grant(wrong_issuer).await.is_err());
    let grant = f.repo.replace_grant(f.grant()).await.unwrap();
    assert!(f.repo.get_grant(&other, &f.server).await.unwrap().is_none());
    assert!(f.repo.get_client_by_ref(&other, &f.client.client_ref).await.unwrap().is_none());
    let mut wrong_refresh = f.refresh(&grant.grant_id, 1);
    wrong_refresh.owner_id = other;
    assert!(f.repo.compare_and_swap_grant(wrong_refresh).await.unwrap().is_none());
    SqliteMcpServerRepository::new(f.db.pool().clone()).delete(&f.server).await.unwrap();
    assert!(f.repo.get_grant(&f.owner, &f.server).await.unwrap().is_none());
    assert!(f.repo.compare_and_swap_grant(f.refresh(&grant.grant_id, 1)).await.unwrap().is_none());
    assert!(f.repo.replace_grant(f.grant()).await.is_err());
}

#[tokio::test]
async fn agent_reset_preserves_oauth_configuration_and_debug_redacts_secrets() {
    let f = Fixture::new().await;
    let grant = f.repo.replace_grant(f.grant()).await.unwrap();
    let client = f.repo.get_client_by_ref(&f.owner, &f.client.client_ref).await.unwrap().unwrap();
    let report = nomifun_db::reset_agent_data(f.db.pool()).await.unwrap();
    assert_eq!(report.preserved_rows["mcp_oauth_clients"], 1);
    assert_eq!(report.preserved_rows["mcp_oauth_grants"], 1);
    assert_eq!(f.repo.get_grant(&f.owner, &f.server).await.unwrap().unwrap(), grant);
    assert_eq!(f.repo.get_client_by_ref(&f.owner, &f.client.client_ref).await.unwrap().unwrap(), client);
    assert!(!format!("{grant:?}").contains("encrypted-token-envelope"));
}

#[tokio::test]
async fn oauth_grant_and_registered_client_round_trip_through_backup_restore() {
    use nomifun_db::backup_bundle::{BackupObjectGraph, create_backup_bundle, restore_backup_data_dir};
    let temp = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database(&temp.path().join("source.sqlite")).await.unwrap();
    let f = Fixture::with_database(database).await;
    let grant = f.repo.replace_grant(f.grant()).await.unwrap();
    let client = f.repo.get_client_by_ref(&f.owner, &f.client.client_ref).await.unwrap().unwrap();
    let bundle = temp.path().join("backup");
    let restored_path = temp.path().join("restored");
    create_backup_bundle(&f.db, &bundle, &uuid::Uuid::now_v7().to_string(), BackupObjectGraph::full_database()).await.unwrap();
    restore_backup_data_dir(&bundle, &restored_path).await.unwrap();
    let restored = nomifun_db::init_database(&restored_path.join("nomifun-backend.db")).await.unwrap();
    let repo = SqliteMcpOAuthRepository::new(restored.pool().clone());
    assert_eq!(repo.get_grant(&f.owner, &f.server).await.unwrap().unwrap(), grant);
    assert_eq!(repo.get_client_by_ref(&f.owner, &f.client.client_ref).await.unwrap().unwrap(), client);
}
