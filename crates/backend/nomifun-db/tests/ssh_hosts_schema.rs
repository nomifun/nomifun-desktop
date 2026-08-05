//! ssh_hosts must satisfy the id-schema contract that runs on every boot, and
//! the repository must enforce owner isolation.
use nomifun_common::SshHostId;
use nomifun_db::{CreateSshHostParams, ISshHostRepository, SqliteSshHostRepository};

#[tokio::test]
async fn migrations_apply_and_id_contract_passes_with_ssh_hosts() {
    // init_database_memory runs all migrations + validate_id_schema_contract.
    let db = nomifun_db::init_database_memory()
        .await
        .expect("init in-memory db with ssh_hosts migration + contract");
    let exists: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name='ssh_hosts'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(exists, 1, "ssh_hosts table must exist after migration");
}

#[tokio::test]
async fn create_list_delete_is_owner_scoped() {
    let db = nomifun_db::init_database_memory().await.expect("db");
    // Real UUIDv7 owner ids (the users.user_id CHECK enforces the format), so
    // the ssh_hosts.user_id logical reference resolves against real rows.
    let user_a = nomifun_common::UserId::new().as_str().to_string();
    let user_b = nomifun_common::UserId::new().as_str().to_string();
    for uid in [&user_a, &user_b] {
        sqlx::query(
            "INSERT INTO users (user_id, username, password_hash, jwt_secret, created_at, updated_at) \
             VALUES (?, ?, '', '', 0, 0)",
        )
        .bind(uid)
        .bind(uid)
        .execute(db.pool())
        .await
        .expect("seed user");
    }

    let repo = SqliteSshHostRepository::new(db.pool().clone());
    let created = repo
        .create(
            &user_a,
            CreateSshHostParams {
                name: "prod",
                host: "10.0.0.1",
                port: 22,
                username: "deploy",
                auth_type: "password",
                password_encrypted: Some("CIPHERTEXT"),
                ..Default::default()
            },
        )
        .await
        .expect("create");
    let id = SshHostId::parse(created.ssh_host_id.clone()).unwrap();

    // Owner A sees it; owner B does not.
    assert_eq!(repo.list(&user_a).await.unwrap().len(), 1);
    assert_eq!(repo.list(&user_b).await.unwrap().len(), 0);
    // Cross-owner find is indistinguishable from absent.
    assert!(repo.find(&user_b, &id).await.unwrap().is_none());
    assert!(repo.find(&user_a, &id).await.unwrap().is_some());

    // Cross-owner delete fails; owner delete succeeds.
    assert!(repo.delete(&user_b, &id).await.is_err());
    assert!(repo.delete(&user_a, &id).await.is_ok());
    assert_eq!(repo.list(&user_a).await.unwrap().len(), 0);
}
