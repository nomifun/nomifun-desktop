use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomifun_db::{Database, SqlitePool};
use serde_json::{Value as JsonValue, json};
use tokio::sync::Notify;

use super::*;

const PLUGIN_ID: &str = "0199cc00-0000-7000-8000-000000000001";
const OTHER_PLUGIN_ID: &str = "0199cc00-0000-7000-8000-000000000002";
const CROSS_OWNER_PLUGIN_ID: &str = "0199cc00-0000-7000-8000-000000000003";
const PROVIDER_ID: &str = "0199cc00-0000-7000-8000-000000000004";
const CONNECTION_ID: &str = "0199cc00-0000-7000-8000-000000000005";
const GENERATION_ID: &str = "0199cc00-0000-7000-8000-000000000006";
const CROSS_OWNER_ID: &str = "0199cc00-0000-7000-8000-000000000007";
const ARTIFACT_DIGEST: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const KEY: [u8; 32] = [0x42; 32];

#[derive(Clone, Debug, PartialEq, Eq)]
struct DesktopCall {
    plugin_id: String,
    request: DesktopFileOpenRequest,
}

#[derive(Default)]
struct RecordingDesktop {
    calls: Mutex<Vec<DesktopCall>>,
}

#[async_trait]
impl PluginDesktopOwner for RecordingDesktop {
    async fn open_file(
        &self,
        caller_plugin_id: &PluginId,
        request: DesktopFileOpenRequest,
        cancellation: PluginServiceCancellation,
    ) -> Result<DesktopFileOpenResult, PluginServicePortError> {
        if cancellation.is_canceled() {
            return Err(port_error("service_call_canceled"));
        }
        self.calls
            .lock()
            .unwrap()
            .push(DesktopCall {
                plugin_id: caller_plugin_id.as_ref().into(),
                request,
            });
        Ok(DesktopFileOpenResult { opened: true })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ActionCall {
    caller: String,
    action: String,
    input: JsonValue,
    call_chain: Vec<String>,
}

#[derive(Default)]
struct RecordingDispatcher {
    calls: Mutex<Vec<ActionCall>>,
    block_until_canceled: AtomicBool,
    observed_cancel: AtomicBool,
    started: Notify,
}

#[async_trait]
impl UnifiedPluginActionDispatcher for RecordingDispatcher {
    async fn invoke(
        &self,
        caller_plugin_id: &PluginId,
        action: &str,
        input: JsonValue,
        call_chain: Vec<String>,
        _preview: bool,
        cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError> {
        self.calls.lock().unwrap().push(ActionCall {
            caller: caller_plugin_id.as_ref().into(),
            action: action.into(),
            input: input.clone(),
            call_chain,
        });
        if self.block_until_canceled.load(Ordering::Acquire) {
            self.started.notify_one();
            while !cancellation.is_canceled() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            self.observed_cancel.store(true, Ordering::Release);
        }
        Ok(json!({"dispatched": action, "input": input}))
    }
}

struct Fixture {
    _database: Database,
    pool: SqlitePool,
    owner: String,
}

impl Fixture {
    async fn new() -> Self {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let pool = database.pool().clone();
        let owner = nomifun_db::installation_owner_id(&pool).await.unwrap();
        seed_artifact(&pool).await;
        seed_plugin(&pool, &owner, PLUGIN_ID, "local.ports").await;
        seed_provider_credentials(&pool).await;
        grant(&pool, &owner, PLUGIN_ID, DESKTOP_FILES_OPEN).await;
        grant(&pool, &owner, PLUGIN_ID, ACTIONS_INVOKE).await;
        set_binding(
            &pool,
            &owner,
            PLUGIN_ID,
            "api_key",
            &format!("provider:{PROVIDER_ID}"),
        )
        .await;
        Self {
            _database: database,
            pool,
            owner,
        }
    }

    fn ports(
        &self,
        dispatcher: Arc<RecordingDispatcher>,
        desktop: Arc<RecordingDesktop>,
    ) -> PluginServicePorts {
        build_plugin_service_ports(self.pool.clone(), KEY, dispatcher, desktop)
    }
}

async fn seed_artifact(pool: &SqlitePool) {
    let manifest = manifest_json();
    sqlx::query(
        "INSERT INTO plugin_artifacts
         (artifact_digest, package_id, version, manifest_json, files_json,
          artifact_root, has_ui, has_service, data_version, created_at_ms)
         VALUES (?, 'local.ports', '1.0.0', ?, '[]', 'artifacts/aa', 0, 1, 0, 1)",
    )
    .bind(ARTIFACT_DIGEST)
    .bind(manifest.to_string())
    .execute(pool)
    .await
    .unwrap();
}

fn manifest_json() -> JsonValue {
    json!({
        "schema": "nomifun.plugin/v1",
        "id": "local.ports",
        "version": "1.0.0",
        "name": "Ports",
        "description": "Production port fixture",
        "hostApi": ">=1 <2",
        "entrypoints": {"service": "service/main.mjs", "serviceMode": "onDemand"},
        "actions": {},
        "bindings": [],
        "dataVersion": 0,
        "migrations": [],
        "configSchema": {"type":"object"},
        "secrets": ["api_key"],
        "permissions": [DESKTOP_FILES_OPEN, ACTIONS_INVOKE]
    })
}

async fn seed_plugin(pool: &SqlitePool, owner: &str, plugin_id: &str, package_id: &str) {
    sqlx::query(
        "INSERT INTO plugins
         (plugin_id, owner_user_id, package_id, name, description, enabled,
          active_artifact_digest, data_generation, revision, config_json,
          created_at_ms, updated_at_ms)
         VALUES (?, ?, ?, 'Ports', 'Fixture', 1, ?, ?, 1, '{}', 1, 1)",
    )
    .bind(plugin_id)
    .bind(owner)
    .bind(package_id)
    .bind(ARTIFACT_DIGEST)
    .bind(GENERATION_ID)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_provider_credentials(pool: &SqlitePool) {
    let provider_plaintext = json!({"api_keys":["provider-secret"]}).to_string();
    let provider_ciphertext = nomifun_common::encrypt_string(&provider_plaintext, &KEY).unwrap();
    sqlx::query(
        "INSERT INTO providers
         (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted,
          enabled, created_at, updated_at)
         VALUES (?, 'openai', 'Provider', 'https://example.invalid', 'bearer', ?, 1, 1, 1)",
    )
    .bind(PROVIDER_ID)
    .bind(provider_ciphertext)
    .execute(pool)
    .await
    .unwrap();

    let connection_plaintext = json!({"api_keys":["connection-secret"]}).to_string();
    let connection_ciphertext =
        nomifun_common::encrypt_string(&connection_plaintext, &KEY).unwrap();
    sqlx::query(
        "INSERT INTO provider_connections
         (connection_id, provider_id, role, base_url, auth_scheme,
          credentials_encrypted, extra, created_at, updated_at)
         VALUES (?, ?, 'voice', 'https://voice.invalid', 'bearer', ?, '{}', 1, 1)",
    )
    .bind(CONNECTION_ID)
    .bind(PROVIDER_ID)
    .bind(connection_ciphertext)
    .execute(pool)
    .await
    .unwrap();
}

async fn grant(pool: &SqlitePool, owner: &str, plugin_id: &str, permission: &str) {
    sqlx::query(
        "INSERT INTO plugin_grants
         (owner_user_id, plugin_id, permission, granted,
          confirmed_artifact_digest, updated_at_ms)
         VALUES (?, ?, ?, 1, ?, 1)",
    )
    .bind(owner)
    .bind(plugin_id)
    .bind(permission)
    .bind(ARTIFACT_DIGEST)
    .execute(pool)
    .await
    .unwrap();
}

async fn set_binding(
    pool: &SqlitePool,
    owner: &str,
    plugin_id: &str,
    slot: &str,
    credential_id: &str,
) {
    sqlx::query(
        "INSERT INTO plugin_credential_bindings
         (owner_user_id, plugin_id, slot, credential_id, updated_at_ms)
         VALUES (?, ?, ?, ?, 1)
         ON CONFLICT(owner_user_id, plugin_id, slot)
         DO UPDATE SET credential_id = excluded.credential_id, updated_at_ms = 1",
    )
    .bind(owner)
    .bind(plugin_id)
    .bind(slot)
    .bind(credential_id)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn encrypted_provider_and_connection_json_resolve_only_for_declared_bound_slots() {
    let fixture = Fixture::new().await;
    let resolver = SqlitePluginSecretsPort {
        pool: fixture.pool.clone(),
        encryption_key: KEY,
    };
    let plugin_id = PluginId::from(PLUGIN_ID);
    assert_eq!(
        resolver
            .resolve_credential_json(
                &plugin_id,
                "api_key",
                &format!("provider:{PROVIDER_ID}"),
            )
            .await
            .unwrap()
            .as_str(),
        r#"{"api_keys":["provider-secret"]}"#
    );
    let secret = resolver
        .get(
            &plugin_id,
            "api_key",
            &format!("provider:{PROVIDER_ID}"),
            false,
        )
        .await
        .unwrap()
        .unwrap();
    let debug = format!("{secret:?}");
    assert_eq!(debug, "PluginSecret([REDACTED])");
    assert!(!debug.contains("provider-secret"));
    assert_eq!(
        format!(
            "{:?}",
            resolver
                .get(
                    &PluginId::from(OTHER_PLUGIN_ID),
                    "api_key",
                    &format!("provider:{PROVIDER_ID}"),
                    true,
                )
                .await
                .unwrap()
                .unwrap()
        ),
        "PluginSecret([REDACTED])",
        "Preview uses the same Credential port with an explicit ephemeral binding"
    );
    assert_eq!(
        resolver
            .get(
                &plugin_id,
                "api_key",
                &format!("connection:{CONNECTION_ID}"),
                false,
            )
            .await
            .unwrap_err(),
        port_error("credential_denied"),
        "production cannot substitute a different Credential reference"
    );

    set_binding(
        &fixture.pool,
        &fixture.owner,
        PLUGIN_ID,
        "api_key",
        &format!("connection:{CONNECTION_ID}"),
    )
    .await;
    assert_eq!(
        resolver
            .resolve_credential_json(
                &plugin_id,
                "api_key",
                &format!("connection:{CONNECTION_ID}"),
            )
            .await
            .unwrap()
            .as_str(),
        r#"{"api_keys":["connection-secret"]}"#
    );

    set_binding(
        &fixture.pool,
        &fixture.owner,
        PLUGIN_ID,
        "undeclared",
        &format!("provider:{PROVIDER_ID}"),
    )
    .await;
    assert_eq!(
        resolver
            .get(
                &plugin_id,
                "undeclared",
                &format!("provider:{PROVIDER_ID}"),
                false,
            )
            .await
            .unwrap_err(),
        port_error("credential_denied")
    );
}

#[tokio::test]
async fn malformed_missing_wrong_key_and_cross_owner_credentials_fail_closed() {
    let fixture = Fixture::new().await;
    let plugin_id = PluginId::from(PLUGIN_ID);
    let resolver = SqlitePluginSecretsPort {
        pool: fixture.pool.clone(),
        encryption_key: KEY,
    };
    for reference in [
        "provider:not-a-uuid",
        "provider:0199cc00-0000-7000-8000-000000000099:extra",
        "unknown:0199cc00-0000-7000-8000-000000000099",
        "connection:0199cc00-0000-7000-8000-000000000099",
    ] {
        set_binding(
            &fixture.pool,
            &fixture.owner,
            PLUGIN_ID,
            "api_key",
            reference,
        )
        .await;
        assert!(resolver
            .get(&plugin_id, "api_key", reference, false)
            .await
            .is_err());
    }
    set_binding(
        &fixture.pool,
        &fixture.owner,
        PLUGIN_ID,
        "api_key",
        &format!("provider:{PROVIDER_ID}"),
    )
    .await;
    let wrong_key = SqlitePluginSecretsPort {
        pool: fixture.pool.clone(),
        encryption_key: [0x99; 32],
    };
    assert_eq!(
        wrong_key
            .get(
                &plugin_id,
                "api_key",
                &format!("provider:{PROVIDER_ID}"),
                false,
            )
            .await
            .unwrap_err(),
        port_error("credential_invalid")
    );

    sqlx::query(
        "INSERT INTO users
         (user_id, username, password_hash, created_at, updated_at)
         VALUES (?, 'cross-owner', 'hash', 1, 1)",
    )
    .bind(CROSS_OWNER_ID)
    .execute(&fixture.pool)
    .await
    .unwrap();
    seed_plugin(
        &fixture.pool,
        CROSS_OWNER_ID,
        CROSS_OWNER_PLUGIN_ID,
        "local.ports",
    )
    .await;
    set_binding(
        &fixture.pool,
        CROSS_OWNER_ID,
        CROSS_OWNER_PLUGIN_ID,
        "api_key",
        &format!("provider:{PROVIDER_ID}"),
    )
    .await;
    assert_eq!(
        resolver
            .get(
                &PluginId::from(CROSS_OWNER_PLUGIN_ID),
                "api_key",
                &format!("provider:{PROVIDER_ID}"),
                false,
            )
            .await
            .unwrap_err(),
        port_error("credential_denied")
    );
}

#[tokio::test]
async fn desktop_capability_requires_current_manifest_grant_and_never_accepts_a_raw_path() {
    let fixture = Fixture::new().await;
    let desktop = Arc::new(RecordingDesktop::default());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let ports = fixture.ports(dispatcher, desktop.clone());
    let plugin_id = PluginId::from(PLUGIN_ID);
    let result = ports
        .host
        .invoke(
            &plugin_id,
            DESKTOP_FILES_OPEN,
            json!({"fileId":"workspace-file:note-1"}),
            false,
            PluginServiceCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(result, json!({"opened":true}));
    assert_eq!(
        desktop.calls.lock().unwrap().as_slice(),
        [DesktopCall {
            plugin_id: PLUGIN_ID.into(),
            request: DesktopFileOpenRequest {
                file_id: "workspace-file:note-1".into()
            }
        }]
    );
    assert_eq!(
        ports
            .host
            .invoke(
                &plugin_id,
                DESKTOP_FILES_OPEN,
                json!({"path":"C:/secret.txt"}),
                false,
                PluginServiceCancellation::default(),
            )
            .await
            .unwrap_err(),
        port_error("desktop_request_invalid")
    );
    assert_eq!(desktop.calls.lock().unwrap().len(), 1);

    let preview = ports
        .host
        .invoke(
            &PluginId::from(OTHER_PLUGIN_ID),
            DESKTOP_FILES_OPEN,
            json!({"fileId":"workspace-file:preview-note"}),
            true,
            PluginServiceCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(preview, json!({"opened":true}));
    assert_eq!(desktop.calls.lock().unwrap().len(), 2);

    let other_digest = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    sqlx::query(
        "INSERT INTO plugin_artifacts
         (artifact_digest, package_id, version, manifest_json, files_json,
          artifact_root, has_ui, has_service, data_version, created_at_ms)
         VALUES (?, 'local.ports', '1.0.1', ?, '[]', 'artifacts/bb', 0, 1, 0, 2)",
    )
    .bind(other_digest)
    .bind(manifest_json().to_string())
    .execute(&fixture.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE plugin_grants SET confirmed_artifact_digest = ?
         WHERE plugin_id = ? AND permission = ?",
    )
    .bind(other_digest)
    .bind(PLUGIN_ID)
    .bind(DESKTOP_FILES_OPEN)
    .execute(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(
        ports
            .host
            .invoke(
                &plugin_id,
                DESKTOP_FILES_OPEN,
                json!({"fileId":"workspace-file:note-2"}),
                false,
                PluginServiceCancellation::default(),
            )
            .await
            .unwrap_err(),
        port_error("grant_denied")
    );
    assert_eq!(
        ports
            .host
            .invoke(
                &plugin_id,
                "desktop.process.spawn",
                json!({}),
                false,
                PluginServiceCancellation::default(),
            )
            .await
            .unwrap_err(),
        port_error("host_capability_denied")
    );
}

#[tokio::test]
async fn action_dispatch_preserves_caller_input_and_the_exact_cancellation_chain() {
    let fixture = Fixture::new().await;
    let desktop = Arc::new(RecordingDesktop::default());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let ports = fixture.ports(dispatcher.clone(), desktop);
    let plugin_id = PluginId::from(PLUGIN_ID);
    let action = format!("plugin:{OTHER_PLUGIN_ID}/echo");
    let result = ports
        .actions
        .invoke(
            &plugin_id,
            &action,
            json!({"value":1}),
            vec!["plugin:caller/root".into()],
            false,
            PluginServiceCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(result["dispatched"], action);
    assert_eq!(
        dispatcher.calls.lock().unwrap()[0],
        ActionCall {
            caller: PLUGIN_ID.into(),
            action: action.clone(),
            input: json!({"value":1}),
            call_chain: vec!["plugin:caller/root".into()],
        }
    );
    assert_eq!(
        ports
            .actions
            .invoke(
                &plugin_id,
                "not-an-action",
                json!({}),
                Vec::new(),
                false,
                PluginServiceCancellation::default(),
            )
            .await
            .unwrap_err(),
        port_error("action_invalid")
    );
    let preview_result = ports
        .actions
        .invoke(
            &PluginId::from(OTHER_PLUGIN_ID),
            &action,
            json!({"preview":true}),
            Vec::new(),
            true,
            PluginServiceCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(preview_result["dispatched"], action);

    dispatcher.block_until_canceled.store(true, Ordering::Release);
    let cancellation = PluginServiceCancellation::default();
    let pending = {
        let actions = ports.actions.clone();
        let plugin_id = plugin_id.clone();
        let action = action.clone();
        let cancellation_for_call = cancellation.clone();
        tokio::spawn(async move {
            actions
                .invoke(
                    &plugin_id,
                    &action,
                    json!({"wait":true}),
                    vec!["plugin:caller/root".into()],
                    false,
                    cancellation_for_call,
                )
                .await
        })
    };
    dispatcher.started.notified().await;
    cancellation.cancel();
    assert_eq!(
        pending.await.unwrap().unwrap_err(),
        port_error("service_call_canceled")
    );
    assert!(dispatcher.observed_cancel.load(Ordering::Acquire));
}
