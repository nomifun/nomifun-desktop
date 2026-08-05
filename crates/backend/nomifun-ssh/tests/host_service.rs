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
