//! Host-book service: encryption at rest, masked round-trip, and the mandatory
//! negative security assertion (no plaintext in the serialized response).
use std::sync::Arc;

use nomifun_common::SshHostId;
use nomifun_db::SqliteSshHostRepository;
use nomifun_ssh::dto::{CreateSshHostRequest, UpdateSshHostRequest};
use nomifun_ssh::SshHostService;

async fn service_with_owner() -> (SshHostService, String) {
    let db = nomifun_db::init_database_memory().await.expect("db");
    let user_id = nomifun_common::UserId::new().as_str().to_string();
    sqlx::query(
        "INSERT INTO users (user_id, username, password_hash, jwt_secret, created_at, updated_at) \
         VALUES (?, ?, '', '', 0, 0)",
    )
    .bind(&user_id)
    .bind(&user_id)
    .execute(db.pool())
    .await
    .expect("seed user");
    let repo = Arc::new(SqliteSshHostRepository::new(db.pool().clone()));
    // A fixed 32-byte key for the test.
    let key = [7u8; 32];
    (SshHostService::new(repo, key), user_id)
}

fn create_req() -> CreateSshHostRequest {
    serde_json::from_value(serde_json::json!({
        "name": "prod-web",
        "host": "10.0.3.21",
        "port": 22,
        "username": "deploy",
        "authType": "password",
        "password": "hunter2_supersecret",
        "sudoPassword": "sudo_secret_pw",
    }))
    .expect("deserialize create req")
}

#[tokio::test]
async fn response_dto_never_contains_plaintext_secret() {
    let (svc, user) = service_with_owner().await;
    let created = svc.create(&user, create_req()).await.expect("create");
    let json = serde_json::to_string(&created).expect("serialize");
    assert!(
        !json.contains("hunter2_supersecret"),
        "password plaintext leaked into response: {json}"
    );
    assert!(
        !json.contains("sudo_secret_pw"),
        "sudo password plaintext leaked into response: {json}"
    );
    // Presence is masked.
    assert!(json.contains("***"), "stored secret should be masked: {json}");
}

#[tokio::test]
async fn masked_update_leaves_secret_unchanged_and_decrypts_original() {
    let (svc, user) = service_with_owner().await;
    let created = svc.create(&user, create_req()).await.expect("create");
    let id = SshHostId::parse(created.ssh_host_id).unwrap();

    // Update the name only; resend the mask for the password (unchanged).
    let upd: UpdateSshHostRequest = serde_json::from_value(serde_json::json!({
        "name": "prod-web-renamed",
        "password": "***",
    }))
    .unwrap();
    svc.update(&user, &id, upd).await.expect("update");

    // The original password must still decrypt.
    let cred = svc.decrypt_credential(&user, &id).await.expect("decrypt");
    assert_eq!(
        cred.password.as_deref().map(|z| z.as_str()),
        Some("hunter2_supersecret"),
        "masked update must leave the stored password unchanged"
    );
    assert_eq!(
        cred.sudo_password.as_deref().map(|z| z.as_str()),
        Some("sudo_secret_pw")
    );
}

#[tokio::test]
async fn empty_string_clears_a_secret() {
    let (svc, user) = service_with_owner().await;
    let created = svc.create(&user, create_req()).await.expect("create");
    let id = SshHostId::parse(created.ssh_host_id).unwrap();

    let upd: UpdateSshHostRequest = serde_json::from_value(serde_json::json!({
        "sudoPassword": "",
    }))
    .unwrap();
    svc.update(&user, &id, upd).await.expect("update");

    let cred = svc.decrypt_credential(&user, &id).await.expect("decrypt");
    assert!(cred.sudo_password.is_none(), "empty string must clear the secret");
    // Password (not touched) remains.
    assert_eq!(
        cred.password.as_deref().map(|z| z.as_str()),
        Some("hunter2_supersecret")
    );
}

#[tokio::test]
async fn unknown_field_is_rejected() {
    // deny_unknown_fields on the request DTO.
    let parsed: Result<CreateSshHostRequest, _> = serde_json::from_value(serde_json::json!({
        "name": "x", "host": "h", "username": "u", "authType": "password",
        "bogusField": 1,
    }));
    assert!(parsed.is_err(), "unknown field must be rejected");
}

// ── `~/.ssh/config` import ──────────────────────────────────────────────
//
// Every config below lives in a `tempfile::tempdir()` and every `~` expands
// against that tempdir: no test here reads the developer's real `~/.ssh`.

use nomifun_ssh::dto::SshImportSkipReason;
use nomifun_ssh::ssh_config::{scan_ssh_config, SshConfigScan};

const FAKE_KEY: &str =
    "-----BEGIN OPENSSH PRIVATE KEY-----\nIMPORTED-FAKE-KEY-BODY\n-----END OPENSSH PRIVATE KEY-----\n";

/// Lay out a fake home with a config (and optionally an identity file), then
/// scan it exactly as the route would.
fn scan_fixture(config: &str, keys: &[(&str, &str)]) -> (tempfile::TempDir, SshConfigScan) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ssh_dir = dir.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).expect("mkdir .ssh");
    let config_path = ssh_dir.join("config");
    std::fs::write(&config_path, config).expect("write config");
    for (name, body) in keys {
        std::fs::write(ssh_dir.join(name), body).expect("write key");
    }
    let scan = scan_ssh_config(&config_path, Some(dir.path())).expect("scan");
    (dir, scan)
}

#[tokio::test]
async fn import_stores_the_identity_file_encrypted_under_key_auth() {
    let (svc, user) = service_with_owner().await;
    let (_dir, scan) = scan_fixture(
        "Host prod-web\n  HostName 10.0.3.21\n  User deploy\n  Port 2222\n  IdentityFile ~/.ssh/id_ed25519\n",
        &[("id_ed25519", FAKE_KEY)],
    );

    let result = svc
        .import_hosts(&user, &["prod-web".to_string()], &scan.hosts)
        .await
        .expect("import");

    assert_eq!(result.skipped.len(), 0, "{:?}", result.skipped);
    assert_eq!(result.imported.len(), 1);
    let imported = &result.imported[0];
    assert_eq!(imported.alias, "prod-web");
    assert!(
        !imported.needs_credential,
        "a readable identity file is a usable credential"
    );

    // The row carries the coordinates from the config and the key, encrypted.
    let id = SshHostId::parse(imported.ssh_host_id.clone()).unwrap();
    let row = svc.get(&user, &id).await.expect("get");
    assert_eq!(row.name, "prod-web");
    assert_eq!(row.host, "10.0.3.21");
    assert_eq!(row.port, 2222);
    assert_eq!(row.username, "deploy");
    assert_eq!(row.auth_type, "key");
    assert_eq!(row.private_key.as_deref(), Some("***"));

    let cred = svc.decrypt_credential(&user, &id).await.expect("decrypt");
    assert_eq!(
        cred.private_key.as_deref().map(|z| z.as_str()),
        Some(FAKE_KEY),
        "the identity file's contents should be what a dial uses"
    );

    // The import result is a report, not a credential channel.
    let json = serde_json::to_string(&result).expect("serialize");
    assert!(
        !json.contains("IMPORTED-FAKE-KEY-BODY"),
        "key body leaked into the import result: {json}"
    );
}

#[tokio::test]
async fn import_flags_a_host_whose_key_cannot_be_read() {
    let (svc, user) = service_with_owner().await;
    // Three ways to end up without a usable key: no IdentityFile at all, one
    // that does not exist, and one that is a *public* key.
    let (_dir, scan) = scan_fixture(
        "Host no-key\n  HostName 10.0.3.30\n  User deploy\n\
         Host gone-key\n  HostName 10.0.3.31\n  User deploy\n  IdentityFile ~/.ssh/absent_key\n\
         Host pub-key\n  HostName 10.0.3.32\n  User deploy\n  IdentityFile ~/.ssh/id_ed25519.pub\n",
        &[("id_ed25519.pub", "ssh-ed25519 AAAAC3NzaC1lZDI1 tester@host\n")],
    );

    let requested = ["no-key".to_string(), "gone-key".to_string(), "pub-key".to_string()];
    let result = svc.import_hosts(&user, &requested, &scan.hosts).await.expect("import");

    assert_eq!(result.imported.len(), 3, "coordinates are still worth importing");
    for imported in &result.imported {
        assert!(
            imported.needs_credential,
            "{} has no usable stored credential and must say so",
            imported.alias
        );
        let id = SshHostId::parse(imported.ssh_host_id.clone()).unwrap();
        let cred = svc.decrypt_credential(&user, &id).await.expect("decrypt");
        assert!(
            cred.private_key.is_none(),
            "{}: a public key is not a private key",
            imported.alias
        );
    }
    // A host that names an identity file authenticates by key; one that names
    // none is left on the password default the form opens with.
    let by_alias = |alias: &str| {
        result
            .imported
            .iter()
            .find(|i| i.alias == alias)
            .expect("imported")
            .ssh_host_id
            .clone()
    };
    let no_key = svc
        .get(&user, &SshHostId::parse(by_alias("no-key")).unwrap())
        .await
        .expect("get");
    assert_eq!(no_key.auth_type, "password");
    let gone_key = svc
        .get(&user, &SshHostId::parse(by_alias("gone-key")).unwrap())
        .await
        .expect("get");
    assert_eq!(gone_key.auth_type, "key");
}

#[tokio::test]
async fn import_skips_hosts_already_in_the_book() {
    let (svc, user) = service_with_owner().await;
    // `prod-web` @ 10.0.3.21:22 as deploy already exists (from `create_req`).
    svc.create(&user, create_req()).await.expect("create");

    let (_dir, scan) = scan_fixture(
        // Same display name, different endpoint.
        "Host prod-web\n  HostName 10.9.9.9\n  User other\n\
         # the next alias resolves to the endpoint that already exists\n\
         Host prod-alias\n  HostName 10.0.3.21\n  User deploy\n\
         Host genuinely-new\n  HostName 10.0.3.99\n  User deploy\n",
        &[],
    );

    let requested = [
        "prod-web".to_string(),
        "prod-alias".to_string(),
        "genuinely-new".to_string(),
    ];
    let result = svc.import_hosts(&user, &requested, &scan.hosts).await.expect("import");

    assert_eq!(
        result.imported.iter().map(|i| i.alias.as_str()).collect::<Vec<_>>(),
        vec!["genuinely-new"]
    );
    assert_eq!(
        result
            .skipped
            .iter()
            .map(|s| (s.alias.as_str(), s.reason))
            .collect::<Vec<_>>(),
        vec![
            ("prod-web", SshImportSkipReason::DuplicateName),
            ("prod-alias", SshImportSkipReason::DuplicateEndpoint),
        ]
    );
    // Exactly one new row.
    assert_eq!(svc.list(&user).await.expect("list").len(), 2);
}

#[tokio::test]
async fn import_reports_an_alias_the_config_no_longer_has() {
    // The config can change between the scan the user confirmed and the import.
    // Inventing a host for a vanished alias would be a guess; saying so is not.
    let (svc, user) = service_with_owner().await;
    let (_dir, scan) = scan_fixture("Host still-there\n  HostName 10.0.3.40\n", &[]);

    let requested = ["still-there".to_string(), "vanished".to_string()];
    let result = svc.import_hosts(&user, &requested, &scan.hosts).await.expect("import");

    assert_eq!(result.imported.len(), 1);
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(result.skipped[0].alias, "vanished");
    assert_eq!(result.skipped[0].reason, SshImportSkipReason::NotInConfig);
}

#[tokio::test]
async fn import_asked_for_the_same_alias_twice_creates_one_host() {
    let (svc, user) = service_with_owner().await;
    let (_dir, scan) = scan_fixture("Host once\n  HostName 10.0.3.50\n  User deploy\n", &[]);

    let requested = ["once".to_string(), "once".to_string()];
    let result = svc.import_hosts(&user, &requested, &scan.hosts).await.expect("import");

    assert_eq!(result.imported.len(), 1);
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(result.skipped[0].reason, SshImportSkipReason::DuplicateName);
    assert_eq!(svc.list(&user).await.expect("list").len(), 1);
}

#[tokio::test]
async fn the_host_book_router_builds_with_every_route() {
    // axum panics when two routes conflict, and it does so while *building* the
    // router — i.e. at boot, not on a request. The import routes sit on the same
    // prefix as the `{ssh_host_id}` capture, so this asserts the whole router
    // still assembles rather than discovering it when the app fails to start.
    let (service, _user) = service_with_owner().await;
    let _router = nomifun_ssh::ssh_host_routes(nomifun_ssh::SshHostRouterState {
        service,
        pool: None,
    });
}
