//! ssh_hosts must satisfy the id-schema contract that runs on every boot, and
//! the repository must enforce owner isolation.
use nomifun_common::SshHostId;
use nomifun_db::{CreateSshHostParams, ISshHostRepository, SqliteSshHostRepository};
use sqlx::{migrate::Migrator, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};
use std::borrow::Cow;

static MIGRATOR: Migrator = sqlx::migrate!();

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

#[tokio::test]
async fn retiring_host_connection_hints_preserves_profiles_credentials_and_fingerprints() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ssh-status-retirement.sqlite");
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(&path).create_if_missing(true).foreign_keys(false),
    ).await.unwrap();
    let previous = Migrator {
        migrations: Cow::Owned(MIGRATOR.iter().take_while(|migration| migration.version < 22).cloned().collect()),
        ..Migrator::DEFAULT
    };
    previous.run(&pool).await.unwrap();
    let owner = nomifun_common::UserId::new();
    sqlx::query("INSERT INTO users(user_id,username,password_hash,jwt_secret,created_at,updated_at) VALUES(?,?,'','',0,0)")
        .bind(owner.as_str()).bind(owner.as_str()).execute(&pool).await.unwrap();
    let repo = SqliteSshHostRepository::new(pool.clone());
    let saved = repo.create(owner.as_str(), CreateSshHostParams {
        name: "preserved-host", host: "127.0.0.1", port: 2222, username: "fixture", auth_type: "password",
        password_encrypted: Some("password-ciphertext"), private_key_encrypted: Some("key-ciphertext"),
        passphrase_encrypted: Some("passphrase-ciphertext"), certificate_encrypted: Some("certificate-ciphertext"),
        sudo_password_encrypted: Some("sudo-ciphertext"),
    }).await.unwrap();
    let id = SshHostId::parse(saved.ssh_host_id).unwrap();
    repo.update_fingerprint(owner.as_str(), &id, "SHA256:preserved").await.unwrap();
    sqlx::query("UPDATE ssh_hosts SET status='connected',last_connected_at=123 WHERE ssh_host_id=?")
        .bind(id.as_str()).execute(&pool).await.unwrap();
    let before = serde_json::to_value(repo.find(owner.as_str(), &id).await.unwrap().unwrap()).unwrap();
    let receipts: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&pool).await.unwrap();
    let schema: Vec<(String, String)> = sqlx::query_as(
        "SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name<>'ssh_hosts' ORDER BY name",
    ).fetch_all(&pool).await.unwrap();

    MIGRATOR.run(&pool).await.unwrap();
    pool.close().await;
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(&path).foreign_keys(false),
    ).await.unwrap();
    let repo = SqliteSshHostRepository::new(pool.clone());

    let after = serde_json::to_value(repo.find(owner.as_str(), &id).await.unwrap().unwrap()).unwrap();
    assert_eq!(after, before, "retirement must preserve every saved profile field and credential byte");
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('ssh_hosts')")
        .fetch_all(&pool).await.unwrap();
    assert!(!columns.iter().any(|column| matches!(column.as_str(), "status" | "last_connected_at")));
    assert_eq!(sqlx::query_as::<_, (String, String)>(
        "SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name<>'ssh_hosts' ORDER BY name",
    ).fetch_all(&pool).await.unwrap(), schema);
    assert_eq!(sqlx::query_as::<_, (i64, Vec<u8>)>("SELECT version,checksum FROM _sqlx_migrations WHERE version<22 ORDER BY version")
        .fetch_all(&pool).await.unwrap(), receipts);
    assert!(repo.update_fingerprint(nomifun_common::UserId::new().as_str(), &id, "SHA256:foreign").await.is_err());
    assert_eq!(repo.find(owner.as_str(), &id).await.unwrap().unwrap().host_fingerprint.as_deref(), Some("SHA256:preserved"));
}
