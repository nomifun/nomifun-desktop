//! End-to-end evidence for the production Unified Plugin Core HTTP boundary.

mod common;

use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::http::StatusCode;
use base64::Engine as _;
use http_body_util::BodyExt as _;
use nomifun_agent_contracts::PluginId;
use nomifun_plugin_platform::PluginServiceObservation;
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

use common::{body_json, get_with_token, json_with_token, setup_and_login};

const LOCAL_TRUST: &str = "plugin-e2e-local-desktop";

struct Harness {
    app: axum::Router,
    services: nomifun_app::compatibility::AppServices,
    token: String,
    csrf: String,
    files: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Self {
        let (mut app, services) = common::build_local_trust_app(LOCAL_TRUST).await;
        let (token, csrf) =
            setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
        Self {
            app,
            services,
            token,
            csrf,
            files: tempfile::tempdir().unwrap(),
        }
    }

    async fn json(&self, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let mut request = json_with_token(
            method,
            uri,
            body,
            &self.token,
            &self.csrf,
        );
        request
            .headers_mut()
            .insert("x-nomi-local-trust", LOCAL_TRUST.parse().unwrap());
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        (status, body_json(response).await)
    }

    async fn get_json(&self, uri: &str) -> (StatusCode, Value) {
        let mut request = get_with_token(uri, &self.token);
        request
            .headers_mut()
            .insert("x-nomi-local-trust", LOCAL_TRUST.parse().unwrap());
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        (status, body_json(response).await)
    }

    async fn get_public_bytes(&self, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        self.get_public_bytes_with_origin(uri, "null").await
    }

    async fn get_public_bytes_with_origin(
        &self,
        uri: &str,
        origin: &str,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let response = self
            .app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .header("host", "127.0.0.1:5197")
                    .header("origin", origin)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, bytes.to_vec())
    }

    fn package_path(&self, name: &str) -> PathBuf {
        self.files.path().join(name)
    }

    async fn install_directory(&self, source: &Path) -> Value {
        let source_path = source.to_string_lossy().into_owned();
        let (status, inspection) = self
            .json(
                "POST",
                "/api/plugins/import/inspect",
                json!({"source_path":source_path,"kind":"directory"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "inspection failed: {inspection}");
        let confirmation = inspection["data"]["permission_expansion"]["confirmation_id"]
            .as_str()
            .map(str::to_owned);
        let (status, installed) = self
            .json(
                "POST",
                "/api/plugins/import",
                json!({
                    "source_path": source.to_string_lossy(),
                    "kind":"directory",
                    "create_copy":false,
                    "permission_confirmation_id":confirmation,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "install failed: {installed}");
        assert_eq!(installed["data"]["result"]["outcome"], "installed");
        installed["data"]["result"]["plugin"].clone()
    }

    async fn open_surface(&self, plugin_id: &str, revision: u64) -> Value {
        let (status, response) = self
            .json(
                "POST",
                &format!("/api/plugins/{plugin_id}/surface/open"),
                json!({"expected_revision":revision}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "surface open failed: {response}");
        response["data"].clone()
    }

    async fn close_surface(&self, route: &str, descriptor: &Value) {
        let (status, response) = self
            .json(
                "POST",
                route,
                json!({
                    "surface_session_id":descriptor["surface_session_id"],
                    "surface_generation":descriptor["surface_generation"],
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "surface close failed: {response}");
    }

    async fn bridge(&self, route: &str, descriptor: &Value, target: Value) -> (StatusCode, Value) {
        self.json(
            "POST",
            route,
            json!({
                "plugin_id":descriptor["plugin_id"],
                "draft_id":descriptor["draft_id"],
                "artifact_digest":descriptor["artifact_digest"],
                "surface_session_id":descriptor["surface_session_id"],
                "surface_generation":descriptor["surface_generation"],
                "is_preview":descriptor["is_preview"],
                "request":{
                    "call_id":Uuid::now_v7().to_string(),
                    "target":target,
                }
            }),
        )
        .await
    }
}

async fn create_chat_provider(
    harness: &Harness,
    base_url: &str,
    name: &str,
    model: &str,
) -> String {
    let (status, provider) = harness
        .json(
            "POST",
            "/api/providers",
            json!({
                "platform":"openai",
                "name":name,
                "base_url":base_url,
                "auth_scheme":"bearer",
                "credentials":{"api_keys":["test-only"]},
                "enabled":true,
                "initial_model":{
                    "model":model,
                    "enabled":true,
                    "capabilities":[{
                        "task":"chat",
                        "traits":[],
                        "protocol":"openai.chat_text",
                        "connection_role":"default",
                        "provider_params":{}
                    }]
                }
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "provider create failed: {provider}");
    provider["data"]["provider_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn openai_sse_text(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({
            "id":"draft-generation",
            "choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]
        }),
        json!({
            "id":"draft-generation",
            "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]
        })
    )
}

fn write_ui_package(
    root: &Path,
    package_id: &str,
    secret_slots: &[&str],
    permissions: &[&str],
) {
    write_package_files(
        root,
        json!({
            "schema":"nomifun.plugin/v1",
            "id":package_id,
            "version":"1.0.0",
            "name":"Unified UI fixture",
            "description":"A UI-only Unified Plugin fixture.",
            "hostApi":">=1 <2",
            "entrypoints":{"ui":"ui/index.html"},
            "actions":{},
            "bindings":[],
            "dataVersion":0,
            "migrations":[],
            "configSchema":{"type":"object"},
            "secrets":secret_slots,
            "permissions":permissions,
        }),
        true,
        None,
    );
}

fn write_service_package(root: &Path, package_id: &str, mode: &str, mixed: bool) {
    let action_name = format!("{mode} echo");
    let service_mode = if mode == "headless" { "continuous" } else { "onDemand" };
    write_package_files(
        root,
        json!({
            "schema":"nomifun.plugin/v1",
            "id":package_id,
            "version":"1.0.0",
            "name":action_name,
            "description":"A dedicated Service Action fixture.",
            "hostApi":">=1 <2",
            "entrypoints": if mixed {
                json!({"ui":"ui/index.html","service":"service/main.mjs","serviceMode":service_mode})
            } else {
                json!({"service":"service/main.mjs","serviceMode":service_mode})
            },
            "actions":{
                "echo":{
                    "name":action_name,
                    "description":"Returns its input from the isolated Plugin Service.",
                    "input":{"type":"object"},
                    "output":{"type":"object"},
                    "effect":"read"
                }
            },
            "bindings":[{"point":"desktop.command","action":"echo"}],
            "dataVersion":0,
            "migrations":[],
            "configSchema":{"type":"object"},
            "secrets":[],
            "permissions":[],
        }),
        mixed,
        Some(format!(
            r#"export async function activate(ctx) {{
  return {{
    async invoke(action, input) {{
      if (action !== "echo") throw new Error("unsupported action");
      await ctx.storage.kv.set("last-input", input);
      return {{ mode: {mode:?}, input, pluginId: ctx.pluginId }};
    }}
  }};
}}
"#
        )),
    );
}

fn write_package_files(root: &Path, manifest: Value, with_ui: bool, service: Option<String>) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("nomifun.plugin.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    if with_ui {
        fs::create_dir_all(root.join("ui")).unwrap();
        fs::write(
            root.join("ui/index.html"),
            b"<!doctype html><html><head><title>Unified fixture</title><link rel=\"stylesheet\" href=\"./style.css\"><script type=\"module\" src=\"./app.js\"></script></head><body><main>fixture</main></body></html>",
        )
        .unwrap();
        fs::write(root.join("ui/app.js"), b"document.body.dataset.plugin = 'ready';\n").unwrap();
        fs::write(root.join("ui/style.css"), b"main { display: block; }\n").unwrap();
    }
    if let Some(service) = service {
        fs::create_dir_all(root.join("service")).unwrap();
        fs::write(root.join("service/main.mjs"), service).unwrap();
    }
}

fn installed_identity(detail: &Value) -> (String, u64) {
    (
        detail["summary"]["plugin_id"].as_str().unwrap().to_owned(),
        detail["summary"]["revision"].as_u64().unwrap(),
    )
}

fn assert_bridge_success(response: &Value) -> &Value {
    assert_eq!(response["data"]["outcome"], "success", "bridge failed: {response}");
    &response["data"]["result"]
}

fn surface_asset_path(descriptor: &Value) -> String {
    surface_asset_path_for(descriptor, descriptor["entrypoint"].as_str().unwrap())
}

fn surface_asset_path_for(descriptor: &Value, asset: &str) -> String {
    let owner = if descriptor["is_preview"] == true {
        format!(
            "/api/plugin-drafts/{}",
            descriptor["draft_id"].as_str().unwrap()
        )
    } else {
        format!(
            "/api/plugins/{}",
            descriptor["plugin_id"].as_str().unwrap()
        )
    };
    format!(
        "{owner}/surface/assets/{}/{}/{}/{}",
        descriptor["surface_session_id"].as_str().unwrap(),
        descriptor["surface_generation"].as_u64().unwrap(),
        descriptor["artifact_digest"].as_str().unwrap(),
        asset,
    )
}

async fn seed_surface_data(harness: &Harness, plugin_id: &str, descriptor: &Value) {
    let route = format!("/api/plugins/{plugin_id}/surface/bridge");
    for target in [
        json!({"target":"kv","request":{"operation":"set","key":"counter","value":7}}),
        json!({"target":"db","request":{"operation":"execute","sql":"CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)","parameters":[]}}),
        json!({"target":"db","request":{"operation":"execute","sql":"INSERT INTO notes (body) VALUES (?1)","parameters":["persisted"]}}),
        json!({"target":"files","request":{"operation":"write","path":"notes/state.txt","content_base64":base64::engine::general_purpose::STANDARD.encode(b"persisted-file"),"overwrite":false}}),
    ] {
        let (status, response) = harness.bridge(&route, descriptor, target).await;
        assert_eq!(status, StatusCode::OK);
        assert_bridge_success(&response);
    }
}

async fn assert_surface_data(harness: &Harness, plugin_id: &str, descriptor: &Value) {
    let route = format!("/api/plugins/{plugin_id}/surface/bridge");
    let (status, response) = harness
        .bridge(
            &route,
            descriptor,
            json!({"target":"kv","request":{"operation":"get","key":"counter"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assert_bridge_success(&response)["result"]["value"], 7);

    let (status, response) = harness
        .bridge(
            &route,
            descriptor,
            json!({"target":"db","request":{"operation":"query","sql":"SELECT body FROM notes ORDER BY id","parameters":[]}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        assert_bridge_success(&response)["result"]["rows"][0]["body"],
        "persisted"
    );

    let (status, response) = harness
        .bridge(
            &route,
            descriptor,
            json!({"target":"files","request":{"operation":"read","path":"notes/state.txt"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let encoded = assert_bridge_success(&response)["result"]["content_base64"]
        .as_str()
        .unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
        b"persisted-file"
    );
}

#[tokio::test]
async fn ui_only_surface_uses_one_sdk_persists_storage_and_library_pin_is_cas_guarded() {
    let harness = Harness::new().await;
    let package = harness.package_path("ui-only");
    write_ui_package(&package, "e2e.ui-only", &[], &[]);
    fs::create_dir_all(package.join("source")).unwrap();
    fs::write(package.join("source/original.ts"), "export const privateSource = true;").unwrap();
    let detail = harness.install_directory(&package).await;
    let (plugin_id, revision) = installed_identity(&detail);
    assert_eq!(detail["summary"]["has_service"], false);
    assert_eq!(detail["summary"]["runtime"]["state"], "stopped");

    let descriptor = harness.open_surface(&plugin_id, revision).await;
    let (status, headers, html) = harness
        .get_public_bytes(&surface_asset_path(&descriptor))
        .await;
    assert_eq!(status, StatusCode::OK);
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("http://127.0.0.1:5197"));
    assert_eq!(headers["access-control-allow-origin"], "null");
    let (_, hostile_headers, _) = harness
        .get_public_bytes_with_origin(
            &surface_asset_path(&descriptor),
            "https://attacker.invalid",
        )
        .await;
    assert_ne!(
        hostile_headers.get("access-control-allow-origin"),
        Some(&axum::http::HeaderValue::from_static("*"))
    );
    assert_ne!(
        hostile_headers.get("access-control-allow-origin"),
        Some(&axum::http::HeaderValue::from_static(
            "https://attacker.invalid"
        ))
    );
    let html = String::from_utf8(html).unwrap();
    assert!(html.contains("Object.defineProperty(window, 'nomi'"));
    let (status, _, script) = harness
        .get_public_bytes(&surface_asset_path_for(&descriptor, "ui/app.js"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(script).unwrap().contains("plugin = 'ready'"));
    assert_eq!(
        harness
            .get_public_bytes(&surface_asset_path_for(&descriptor, "source/original.ts"))
            .await
            .0,
        StatusCode::NOT_FOUND,
        "source/** must remain authoring-only and outside the Runtime asset surface"
    );
    assert_eq!(
        harness
            .get_public_bytes(&surface_asset_path_for(&descriptor, "nomifun.plugin.json"))
            .await
            .0,
        StatusCode::NOT_FOUND,
    );
    let mut wrong_owner = surface_asset_path_for(&descriptor, "ui/app.js");
    wrong_owner = wrong_owner.replacen(&plugin_id, &Uuid::now_v7().to_string(), 1);
    assert_eq!(harness.get_public_bytes(&wrong_owner).await.0, StatusCode::NOT_FOUND);
    seed_surface_data(&harness, &plugin_id, &descriptor).await;
    harness
        .close_surface(
            &format!("/api/plugins/{plugin_id}/surface/close"),
            &descriptor,
        )
        .await;

    let reopened = harness.open_surface(&plugin_id, revision).await;
    assert_surface_data(&harness, &plugin_id, &reopened).await;
    let runtime = harness.services.plugin_service_runtime.get().unwrap();
    assert_eq!(
        runtime
            .observation(&PluginId::from(plugin_id.clone()))
            .await,
        PluginServiceObservation::Stopped,
        "UI-only Plugins must not own a Node process"
    );

    let (status, library) = harness.get_json("/api/plugins/library-state").await;
    assert_eq!(status, StatusCode::OK);
    let library_revision = library["data"]["revision"].as_u64().unwrap();
    assert_eq!(library["data"]["items"].as_array().unwrap().len(), 1);
    let (status, pinned) = harness
        .json(
            "PUT",
            "/api/plugins/library-state",
            json!({
                "expected_revision":library_revision,
                "collections":[],
                "items":[{"plugin_id":plugin_id,"pinned":true}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "pin failed: {pinned}");
    assert_eq!(pinned["data"]["items"][0]["pinned"], true);
    let (status, _) = harness
        .json(
            "PUT",
            "/api/plugins/library-state",
            json!({
                "expected_revision":library_revision,
                "collections":[],
                "items":[{"plugin_id":plugin_id,"pinned":false}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "stale library CAS must fail");
}

#[tokio::test]
async fn package_and_backup_round_trip_data_without_credential_plaintext() {
    const SECRET: &str = "E2E-CREDENTIAL-PLAINTEXT-MUST-NOT-EXPORT";
    let harness = Harness::new().await;
    let package = harness.package_path("backup-source");
    write_ui_package(&package, "e2e.backup", &["api_key"], &["network"]);
    let (status, first_inspection) = harness
        .json(
            "POST",
            "/api/plugins/import/inspect",
            json!({"source_path":package.to_string_lossy(),"kind":"directory"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        first_inspection["data"]["permission_expansion"]["added_secret_slots"][0],
        "api_key"
    );
    assert_eq!(
        first_inspection["data"]["permission_expansion"]["added_permissions"][0],
        "network"
    );
    let detail = harness.install_directory(&package).await;
    let (plugin_id, revision) = installed_identity(&detail);
    let surface = harness.open_surface(&plugin_id, revision).await;
    seed_surface_data(&harness, &plugin_id, &surface).await;
    harness
        .close_surface(
            &format!("/api/plugins/{plugin_id}/surface/close"),
            &surface,
        )
        .await;

    let provider_id = Uuid::now_v7().to_string();
    let encrypted = nomifun_common::encrypt_string(
        &json!({"api_keys":[SECRET]}).to_string(),
        &harness.services.encryption_key,
    )
    .unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO providers
         (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted,
          enabled, created_at, updated_at)
         VALUES (?, 'openai', 'Plugin E2E', 'https://example.invalid', 'bearer', ?, 1, 1, 1)",
    )
    .bind(&provider_id)
    .bind(encrypted)
    .execute(harness.services.database.pool())
    .await
    .unwrap();
    let (status, credential_references) =
        harness.get_json("/api/plugins/credentials").await;
    assert_eq!(status, StatusCode::OK);
    assert!(credential_references["data"].as_array().unwrap().iter().any(|reference| {
        reference["credential_id"] == format!("provider:{provider_id}")
            && reference["label"] == "Plugin E2E"
            && reference["enabled"] == true
    }));
    assert!(!credential_references.to_string().contains(SECRET));
    let (status, configured) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{plugin_id}/config"),
            json!({
                "expected_revision":revision,
                "config":{"restored":true},
                "credential_bindings":{"api_key":format!("provider:{provider_id}")},
                "grants":{"network":true}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure failed: {configured}");
    let mut configured_revision = configured["data"]["summary"]["revision"]
        .as_u64()
        .unwrap();

    let manifest_path = package.join("nomifun.plugin.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["version"] = json!("1.0.1");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let (status, update_inspection) = harness
        .json(
            "POST",
            "/api/plugins/import/inspect",
            json!({"source_path":package.to_string_lossy(),"kind":"directory"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "update inspect failed: {update_inspection}");
    assert!(update_inspection["data"]["permission_expansion"].is_null());
    let (status, updated) = harness
        .json(
            "POST",
            "/api/plugins/import",
            json!({
                "source_path":package.to_string_lossy(),
                "kind":"directory",
                "expected_plugin_revision":configured_revision,
                "create_copy":false
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "update import failed: {updated}");
    assert_eq!(
        updated["data"]["result"]["plugin"]["config"]["values"]["restored"],
        true,
        "Package updates must preserve the local instance config"
    );
    configured_revision = updated["data"]["result"]["plugin"]["summary"]["revision"]
        .as_u64()
        .unwrap();

    let package_zip = harness.files.path().join("plugin-package.zip");
    let backup_zip = harness.files.path().join("plugin-backup.zip");
    for (route, destination, body) in [
        (
            "export",
            &package_zip,
            json!({"expected_revision":configured_revision,"destination_path":package_zip.to_string_lossy(),"include_source":true}),
        ),
        (
            "backup",
            &backup_zip,
            json!({"expected_revision":configured_revision,"destination_path":backup_zip.to_string_lossy()}),
        ),
    ] {
        let (status, exported) = harness
            .json(
                "POST",
                &format!("/api/plugins/{plugin_id}/{route}"),
                body,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "export failed: {exported}");
        assert!(destination.is_file());
    }
    assert_zip_has_no_secret(&package_zip, SECRET, &provider_id);
    assert_zip_has_no_secret(&backup_zip, SECRET, &provider_id);
    let package_entries = zip_entries(&package_zip);
    assert!(!package_entries.iter().any(|name| name.ends_with("data.sqlite")));
    assert!(zip_entries(&backup_zip).iter().any(|name| name.ends_with("data.sqlite")));

    let (status, package_inspection) = harness
        .json(
            "POST",
            "/api/plugins/import/inspect",
            json!({"source_path":package_zip.to_string_lossy(),"kind":"zip"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "package inspect failed: {package_inspection}");
    let (status, confirmation_required) = harness
        .json(
            "POST",
            "/api/plugins/import",
            json!({
                "source_path":package_zip.to_string_lossy(),
                "kind":"zip",
                "create_copy":true
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        confirmation_required["data"]["result"]["outcome"],
        "confirmation_required"
    );
    let copy_confirmation = confirmation_required["data"]["result"]["confirmation"]
        ["confirmation_id"]
        .as_str()
        .unwrap();
    let (status, package_copy) = harness
        .json(
            "POST",
            "/api/plugins/import",
            json!({
                "source_path":package_zip.to_string_lossy(),
                "kind":"zip",
                "create_copy":true,
                "permission_confirmation_id":copy_confirmation
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "package re-import failed: {package_copy}");
    assert_eq!(package_copy["data"]["result"]["outcome"], "installed");
    assert_eq!(
        package_copy["data"]["result"]["plugin"]["config"]["values"],
        json!({}),
        "Package copies must not inherit another local instance's user config"
    );

    let backup_path = backup_zip.to_string_lossy().into_owned();
    let (status, inspection) = harness
        .json(
            "POST",
            "/api/plugins/import/inspect",
            json!({"source_path":backup_path,"kind":"backup"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "backup inspect failed: {inspection}");
    assert_eq!(inspection["data"]["backup"]["includes_data"], true);
    assert_eq!(
        inspection["data"]["backup"]["credential_slots_to_rebind"][0],
        "api_key"
    );
    let confirmation = inspection["data"]["permission_expansion"]["confirmation_id"]
        .as_str()
        .map(str::to_owned);
    let (status, imported) = harness
        .json(
            "POST",
            "/api/plugins/import",
            json!({
                "source_path":backup_zip.to_string_lossy(),
                "kind":"backup",
                "create_copy":true,
                "permission_confirmation_id":confirmation,
                "credential_bindings":{"api_key":format!("provider:{provider_id}")}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "backup restore failed: {imported}");
    let restored = &imported["data"]["result"]["plugin"];
    let (restored_id, restored_revision) = installed_identity(restored);
    assert_ne!(restored_id, plugin_id);
    assert_eq!(restored["config"]["values"]["restored"], true);
    assert_eq!(restored["credential_bindings"][0]["status"], "bound");
    assert_eq!(
        restored["credential_bindings"][0]["credential_id"],
        format!("provider:{provider_id}")
    );
    let restored_surface = harness.open_surface(&restored_id, restored_revision).await;
    assert_surface_data(&harness, &restored_id, &restored_surface).await;
}

#[tokio::test]
async fn headless_and_mixed_service_actions_publish_desktop_commands_and_lifecycle_revokes_them() {
    let harness = Harness::new().await;
    let headless_path = harness.package_path("headless");
    let mixed_path = harness.package_path("mixed");
    write_service_package(&headless_path, "e2e.headless", "headless", false);
    write_service_package(&mixed_path, "e2e.mixed", "mixed", true);
    let headless = harness.install_directory(&headless_path).await;
    let mixed = harness.install_directory(&mixed_path).await;
    let (headless_id, headless_revision) = installed_identity(&headless);
    let (mixed_id, mixed_revision) = installed_identity(&mixed);
    assert_eq!(headless["summary"]["has_ui"], false);
    assert_eq!(mixed["summary"]["has_ui"], true);
    assert_eq!(headless["summary"]["runtime"]["state"], "running");
    assert_eq!(mixed["summary"]["runtime"]["state"], "stopped");

    let (status, commands) = harness.get_json("/api/plugins/desktop/commands").await;
    assert_eq!(status, StatusCode::OK);
    let command_ids = commands["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["action_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let headless_action = format!("plugin:{headless_id}/echo");
    let mixed_action = format!("plugin:{mixed_id}/echo");
    assert!(command_ids.contains(&headless_action));
    assert!(command_ids.contains(&mixed_action));
    for (action, mode) in [(&headless_action, "headless"), (&mixed_action, "mixed")] {
        let (status, invoked) = harness
            .json(
                "POST",
                "/api/plugins/desktop/commands/invoke",
                json!({"action_id":action,"input":{"value":mode}}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "command failed: {invoked}");
        assert_eq!(invoked["data"]["mode"], mode);
        assert_eq!(invoked["data"]["input"]["value"], mode);
    }

    let mixed_surface = harness.open_surface(&mixed_id, mixed_revision).await;
    let (status, action_result) = harness
        .bridge(
            &format!("/api/plugins/{mixed_id}/surface/bridge"),
            &mixed_surface,
            json!({"target":"actions","action":"echo","input":{"value":"surface"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assert_bridge_success(&action_result)["result"]["mode"], "mixed");
    let (status, shared_storage) = harness
        .bridge(
            &format!("/api/plugins/{mixed_id}/surface/bridge"),
            &mixed_surface,
            json!({"target":"kv","request":{"operation":"get","key":"last-input"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        assert_bridge_success(&shared_storage)["result"]["value"]["value"],
        "surface",
        "UI and Service must observe the same generation DataRoot"
    );

    let (status, disabled) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{mixed_id}/enabled"),
            json!({"expected_revision":mixed_revision,"enabled":false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "disable failed: {disabled}");
    let (status, _) = harness
        .bridge(
            &format!("/api/plugins/{mixed_id}/surface/bridge"),
            &mixed_surface,
            json!({"target":"config"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "disable must revoke Surface admission");
    assert_command_absent(&harness, &mixed_action).await;

    let (status, enabled) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{mixed_id}/enabled"),
            json!({"expected_revision":2,"enabled":true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "re-enable failed: {enabled}");
    let (status, trashed) = harness
        .json(
            "POST",
            &format!("/api/plugins/{mixed_id}/trash"),
            json!({"expected_revision":3}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "trash failed: {trashed}");
    assert_command_absent(&harness, &mixed_action).await;
    assert_eq!(
        harness
            .services
            .plugin_service_runtime
            .get()
            .unwrap()
            .observation(&PluginId::from(mixed_id.clone()))
            .await,
        PluginServiceObservation::Stopped,
        "Trash must stop the mixed Plugin process"
    );
    let mixed_root = harness.services.plugin_data_roots.root().join(&mixed_id);
    assert!(mixed_root.exists());
    let (status, deleted) = harness
        .json(
            "DELETE",
            &format!("/api/plugins/{mixed_id}"),
            json!({"expected_revision":4,"acknowledge_permanent_delete":true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "permanent delete failed: {deleted}");
    assert!(!mixed_root.exists());
    assert_eq!(
        harness
            .get_json(&format!("/api/plugins/{mixed_id}"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (status, disabled) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{headless_id}/enabled"),
            json!({"expected_revision":headless_revision,"enabled":false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "headless disable failed: {disabled}");
    assert_command_absent(&harness, &headless_action).await;
    assert_eq!(
        harness
            .services
            .plugin_service_runtime
            .get()
            .unwrap()
            .observation(&PluginId::from(headless_id))
            .await,
        PluginServiceObservation::Stopped,
        "Disable must stop the headless Plugin process"
    );
}

async fn assert_command_absent(harness: &Harness, action_id: &str) {
    let (status, commands) = harness.get_json("/api/plugins/desktop/commands").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        commands["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|command| command["action_id"] != action_id)
    );
    let (status, _) = harness
        .json(
            "POST",
            "/api/plugins/desktop/commands/invoke",
            json!({"action_id":action_id,"input":{}}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "revoked Actions remain fenced tombstones, never invokable"
    );
}

#[tokio::test]
async fn draft_preview_uses_temporary_data_root_without_polluting_installed_data() {
    let harness = Harness::new().await;
    let package = harness.package_path("preview-base");
    write_ui_package(&package, "e2e.preview", &[], &[]);
    let detail = harness.install_directory(&package).await;
    let (plugin_id, revision) = installed_identity(&detail);
    let production = harness.open_surface(&plugin_id, revision).await;
    let (status, response) = harness
        .bridge(
            &format!("/api/plugins/{plugin_id}/surface/bridge"),
            &production,
            json!({"target":"kv","request":{"operation":"set","key":"scope","value":"production"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_bridge_success(&response);
    harness
        .close_surface(
            &format!("/api/plugins/{plugin_id}/surface/close"),
            &production,
        )
        .await;

    let (status, draft) = harness
        .json(
            "POST",
            "/api/plugin-drafts",
            json!({"plugin_id":plugin_id,"expected_plugin_revision":revision}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "draft create failed: {draft}");
    let draft_id = draft["data"]["summary"]["draft_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, preview) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/preview"),
            json!({"expected_revision":1,"config":{},"access":{"permissions":[],"credential_bindings":{}}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "preview failed: {preview}");
    let descriptor = preview["data"]["descriptor"].clone();
    let (status, response) = harness
        .bridge(
            &format!("/api/plugin-drafts/{draft_id}/surface/bridge"),
            &descriptor,
            json!({"target":"kv","request":{"operation":"set","key":"scope","value":"preview"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_bridge_success(&response);
    let preview_revision = preview["data"]["draft_revision"].as_u64().unwrap();
    let (status, reloaded_preview) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/preview"),
            json!({"expected_revision":preview_revision,"config":{},"access":{"permissions":[],"credential_bindings":{}}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "Preview reload failed: {reloaded_preview}");
    let reloaded_descriptor = reloaded_preview["data"]["descriptor"].clone();
    assert_eq!(
        reloaded_descriptor["surface_session_id"],
        descriptor["surface_session_id"],
    );
    assert!(
        reloaded_descriptor["surface_generation"].as_u64().unwrap()
            > descriptor["surface_generation"].as_u64().unwrap()
    );
    assert_eq!(
        harness
            .bridge(
                &format!("/api/plugin-drafts/{draft_id}/surface/bridge"),
                &descriptor,
                json!({"target":"kv","request":{"operation":"get","key":"scope"}}),
            )
            .await
            .0,
        StatusCode::NOT_FOUND,
    );
    let (status, response) = harness
        .bridge(
            &format!("/api/plugin-drafts/{draft_id}/surface/bridge"),
            &reloaded_descriptor,
            json!({"target":"kv","request":{"operation":"get","key":"scope"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assert_bridge_success(&response)["result"]["value"], "preview");
    let (status, configured) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{plugin_id}/config"),
            json!({
                "expected_revision":revision,
                "config":{},
                "credential_bindings":{},
                "grants":{}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure failed: {configured}");
    let configured_revision = configured["data"]["summary"]["revision"]
        .as_u64()
        .unwrap();
    assert_eq!(
        harness
            .bridge(
                &format!("/api/plugin-drafts/{draft_id}/surface/bridge"),
                &reloaded_descriptor,
                json!({"target":"kv","request":{"operation":"get","key":"scope"}}),
            )
            .await
            .0,
        StatusCode::NOT_FOUND,
        "installed Plugin lifecycle changes must revoke related Preview admission"
    );

    let reopened = harness.open_surface(&plugin_id, configured_revision).await;
    let (status, response) = harness
        .bridge(
            &format!("/api/plugins/{plugin_id}/surface/bridge"),
            &reopened,
            json!({"target":"kv","request":{"operation":"get","key":"scope"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        assert_bridge_success(&response)["result"]["value"],
        "production"
    );
    let preview_staging = harness
        .services
        .plugin_data_roots
        .root()
        .join(&draft_id)
        .join("staging");
    assert!(preview_staging.is_dir());
    assert_eq!(fs::read_dir(preview_staging).unwrap().count(), 0);
}

#[tokio::test]
async fn draft_generation_is_single_flight_cancellable_and_failure_persistent() {
    let harness = Harness::new().await;
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(30))
                .insert_header("content-type", "text/event-stream")
                .set_body_string("data: [DONE]\n\n"),
        )
        .mount(&upstream)
        .await;

    let provider_id = create_chat_provider(
        &harness,
        &format!("{}/v1", upstream.uri()),
        "Draft cancellation fixture",
        "draft-model",
    )
    .await;

    let (status, created) = harness.json("POST", "/api/plugin-drafts", json!({})).await;
    assert_eq!(status, StatusCode::OK, "Draft create failed: {created}");
    let draft_id = created["data"]["summary"]["draft_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let initial_revision = created["data"]["summary"]["revision"].as_u64().unwrap();
    let original_files = created["data"]["files"].clone();

    let app = harness.app.clone();
    let token = harness.token.clone();
    let csrf = harness.csrf.clone();
    let generation_uri = format!("/api/plugin-drafts/{draft_id}/generate");
    let generation_provider = provider_id.clone();
    let generation = tokio::spawn(async move {
        let mut request = json_with_token(
            "POST",
            &generation_uri,
            json!({
                "expected_revision":initial_revision,
                "provider_id":generation_provider,
                "model":"draft-model",
                "requirement":"Create a cancellable Plugin"
            }),
            &token,
            &csrf,
        );
        request
            .headers_mut()
            .insert("x-nomi-local-trust", LOCAL_TRUST.parse().unwrap());
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        (status, body_json(response).await)
    });

    let generating = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (status, detail) = harness
                .get_json(&format!("/api/plugin-drafts/{draft_id}"))
                .await;
            assert_eq!(status, StatusCode::OK);
            if detail["data"]["summary"]["status"] == "generating" {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("generation never entered its persisted Generating state");
    let generating_revision = generating["data"]["summary"]["revision"]
        .as_u64()
        .unwrap();
    assert_eq!(generating_revision, initial_revision + 1);

    let (duplicate_status, _) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/generate"),
            json!({
                "expected_revision":generating_revision,
                "provider_id":provider_id,
                "model":"draft-model",
                "requirement":"A second concurrent request"
            }),
        )
        .await;
    assert_eq!(duplicate_status, StatusCode::CONFLICT);

    let (stale_status, _) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/cancel"),
            json!({"expected_revision":initial_revision}),
        )
        .await;
    assert_eq!(stale_status, StatusCode::CONFLICT);

    let mut other_app = harness.app.clone();
    let (other_token, other_csrf) = setup_and_login(
        &mut other_app,
        &harness.services,
        "draft-generation-other-owner",
        "StrongP@ss2",
    )
    .await;
    let other_cancel = json_with_token(
        "POST",
        &format!("/api/plugin-drafts/{draft_id}/cancel"),
        json!({"expected_revision":generating_revision}),
        &other_token,
        &other_csrf,
    );
    let other_cancel = other_app.oneshot(other_cancel).await.unwrap();
    assert!(
        matches!(
            other_cancel.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ),
        "another owner must not cancel this Draft"
    );

    let (cancel_status, cancelled) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/cancel"),
            json!({"expected_revision":generating_revision}),
        )
        .await;
    assert_eq!(cancel_status, StatusCode::OK, "cancel failed: {cancelled}");
    assert_eq!(cancelled["data"]["summary"]["status"], "ready");
    assert_eq!(
        cancelled["data"]["summary"]["revision"],
        generating_revision + 1
    );

    let (generation_status, generation_result) = tokio::time::timeout(
        Duration::from_secs(5),
        generation,
    )
    .await
    .expect("cancel did not drop the model request")
    .unwrap();
    assert_eq!(generation_status, StatusCode::OK, "{generation_result}");
    assert_eq!(generation_result["data"]["summary"]["status"], "ready");
    assert_eq!(generation_result["data"]["files"], original_files);

    let ready_revision = cancelled["data"]["summary"]["revision"].as_u64().unwrap();
    let (failed_status, failed_response) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/generate"),
            json!({
                "expected_revision":ready_revision,
                "provider_id":"0190f5fe-7c00-7a00-8000-ffffffffffff",
                "model":"missing",
                "requirement":"This provider is unavailable"
            }),
        )
        .await;
    assert_eq!(failed_status, StatusCode::BAD_GATEWAY, "{failed_response}");
    let (status, failed) = harness
        .get_json(&format!("/api/plugin-drafts/{draft_id}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(failed["data"]["summary"]["status"], "failed");
    assert_eq!(
        failed["data"]["summary"]["error_code"],
        "PLUGIN_GENERATION_FAILED"
    );
    assert_eq!(
        failed["data"]["summary"]["revision"],
        ready_revision + 2
    );
}

#[tokio::test]
async fn draft_generation_parse_and_artifact_failures_persist_failed_state() {
    let harness = Harness::new().await;
    let (status, created) = harness.json("POST", "/api/plugin-drafts", json!({})).await;
    assert_eq!(status, StatusCode::OK, "Draft create failed: {created}");
    let draft_id = created["data"]["summary"]["draft_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut revision = created["data"]["summary"]["revision"].as_u64().unwrap();
    let original_files = created["data"]["files"].clone();

    let cases = [
        (
            "parse-failure",
            "not-json".to_owned(),
            StatusCode::BAD_GATEWAY,
            "PLUGIN_GENERATION_FAILED",
        ),
        (
            "artifact-failure",
            json!({
                "assistant_message":"invalid artifact",
                "files":{
                    "nomifun.plugin.json":"{}",
                    "ui/index.html":"<!doctype html><main>invalid</main>"
                }
            })
            .to_string(),
            StatusCode::BAD_REQUEST,
            "PLUGIN_INVALID_INPUT",
        ),
    ];

    for (index, (model, output, expected_status, expected_error)) in
        cases.into_iter().enumerate()
    {
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(openai_sse_text(&output)),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        let provider_id = create_chat_provider(
            &harness,
            &format!("{}/v1", upstream.uri()),
            &format!("Draft failure fixture {index}"),
            model,
        )
        .await;
        let (generation_status, generation) = harness
            .json(
                "POST",
                &format!("/api/plugin-drafts/{draft_id}/generate"),
                json!({
                    "expected_revision":revision,
                    "provider_id":provider_id,
                    "model":model,
                    "requirement":"Exercise a terminal generation failure"
                }),
            )
            .await;
        assert_eq!(generation_status, expected_status, "{generation}");

        let (status, detail) = harness
            .get_json(&format!("/api/plugin-drafts/{draft_id}"))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["data"]["summary"]["status"], "failed");
        assert_eq!(detail["data"]["summary"]["error_code"], expected_error);
        assert_eq!(
            detail["data"]["files"], original_files,
            "failed generation must not publish a partial tree"
        );
        let next_revision = detail["data"]["summary"]["revision"].as_u64().unwrap();
        assert_eq!(next_revision, revision + 2);
        revision = next_revision;
    }
}

#[tokio::test]
async fn successful_draft_generation_publishes_one_exact_package_tree() {
    let harness = Harness::new().await;
    let (status, created) = harness.json("POST", "/api/plugin-drafts", json!({})).await;
    assert_eq!(status, StatusCode::OK, "Draft create failed: {created}");
    let draft_id = created["data"]["summary"]["draft_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let initial_revision = created["data"]["summary"]["revision"].as_u64().unwrap();
    let (status, edited) = harness
        .json(
            "PUT",
            &format!("/api/plugin-drafts/{draft_id}/files"),
            json!({
                "expected_revision":initial_revision,
                "path":"source/obsolete.ts",
                "content_base64":base64::engine::general_purpose::STANDARD.encode("obsolete")
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "Draft edit failed: {edited}");
    let edited_revision = edited["data"]["summary"]["revision"].as_u64().unwrap();

    let manifest = json!({
        "schema":"nomifun.plugin/v1",
        "id":"local.generated.exact",
        "version":"1.0.0",
        "name":"Exact generated Plugin",
        "description":"Exact tree fixture",
        "hostApi":">=1 <2",
        "entrypoints":{"ui":"ui/index.html"},
        "actions":{},
        "bindings":[],
        "dataVersion":0,
        "migrations":[],
        "configSchema":{
            "type":"object",
            "properties":{"label":{"type":"string"}},
            "required":["label"],
            "additionalProperties":false
        },
        "secrets":[],
        "permissions":[]
    })
    .to_string();
    let generated = json!({
        "assistant_message":"Created an exact package",
        "files":{
            "nomifun.plugin.json":manifest,
            "ui/index.html":"<!doctype html><main>exact replacement</main>"
        }
    })
    .to_string();
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(openai_sse_text(&generated)),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    let provider_id = create_chat_provider(
        &harness,
        &format!("{}/v1", upstream.uri()),
        "Exact Draft generation fixture",
        "exact-draft-model",
    )
    .await;
    let (status, generated) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/generate"),
            json!({
                "expected_revision":edited_revision,
                "provider_id":provider_id,
                "model":"exact-draft-model",
                "requirement":"Replace the complete package"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "generation failed: {generated}");
    assert_eq!(generated["data"]["summary"]["status"], "ready");
    let paths = generated["data"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["nomifun.plugin.json", "ui/index.html"]);
    assert_eq!(
        generated["data"]["summary"]["revision"],
        edited_revision + 2
    );
    let generated_revision = generated["data"]["summary"]["revision"]
        .as_u64()
        .unwrap();
    let (status, preview) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/preview"),
            json!({
                "expected_revision":generated_revision,
                "config":{"label":"configured before save"},
                "access":{"permissions":[],"credential_bindings":{}}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "generated Preview failed: {preview}");
    let (status, saved) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/save"),
            json!({
                "expected_revision":generated_revision,
                "config":{"label":"configured before save"},
                "credential_bindings":{}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "generated Draft save failed: {saved}");
    assert_eq!(saved["data"]["result"]["outcome"], "installed");
    assert_eq!(
        saved["data"]["result"]["plugin"]["config"]["values"]["label"],
        "configured before save"
    );
    assert_eq!(saved["data"]["result"]["plugin"]["summary"]["has_ui"], true);
    assert_eq!(
        saved["data"]["result"]["plugin"]["summary"]["runtime"]["state"],
        "stopped",
        "Chat-created UI-only Plugins must still use zero Node processes"
    );
    let saved_plugin = &saved["data"]["result"]["plugin"];
    let (plugin_id, plugin_revision) = installed_identity(saved_plugin);
    let descriptor = harness.open_surface(&plugin_id, plugin_revision).await;
    seed_surface_data(&harness, &plugin_id, &descriptor).await;
    harness
        .close_surface(
            &format!("/api/plugins/{plugin_id}/surface/close"),
            &descriptor,
        )
        .await;
    let reopened = harness.open_surface(&plugin_id, plugin_revision).await;
    assert_surface_data(&harness, &plugin_id, &reopened).await;
    assert_eq!(
        harness
            .services
            .plugin_service_runtime
            .get()
            .unwrap()
            .observation(&PluginId::from(plugin_id))
            .await,
        PluginServiceObservation::Stopped,
        "Chat-created UI-only Plugins must persist DB/KV/Files across Surface restart without Node"
    );
}

#[tokio::test]
async fn chat_generated_headless_action_reaches_the_live_binding_registry() {
    let harness = Harness::new().await;
    let manifest = json!({
        "schema":"nomifun.plugin/v1",
        "id":"local.generated.agent-tool",
        "version":"1.0.0",
        "name":"Generated Agent Tool",
        "description":"Headless Action generated through Chat.",
        "hostApi":">=1 <2",
        "entrypoints":{"service":"service/main.mjs","serviceMode":"continuous"},
        "actions":{
            "echo":{
                "name":"Generated echo",
                "description":"Returns the selected Agent input.",
                "input":{"type":"object"},
                "output":{"type":"object"},
                "effect":"read"
            }
        },
        "bindings":[
            {"point":"agent.tool","action":"echo"},
            {"point":"desktop.command","action":"echo"}
        ],
        "dataVersion":0,
        "migrations":[],
        "configSchema":{"type":"object"},
        "secrets":[],
        "permissions":[]
    });
    let service = r#"export async function activate(ctx) {
  return {
    async invoke(action, input) {
      if (action !== "echo") throw new Error("unsupported action");
      return { source: "chat", input, pluginId: ctx.pluginId };
    }
  };
}
"#;
    let generated = json!({
        "assistant_message":"Created a headless Agent Action",
        "files":{
            "nomifun.plugin.json":manifest.to_string(),
            "service/main.mjs":service
        }
    })
    .to_string();
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(openai_sse_text(&generated)),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    let provider_id = create_chat_provider(
        &harness,
        &format!("{}/v1", upstream.uri()),
        "Generated headless Action fixture",
        "headless-draft-model",
    )
    .await;
    let (status, created) = harness.json("POST", "/api/plugin-drafts", json!({})).await;
    assert_eq!(status, StatusCode::OK, "Draft create failed: {created}");
    let draft_id = created["data"]["summary"]["draft_id"].as_str().unwrap();
    let revision = created["data"]["summary"]["revision"].as_u64().unwrap();
    let (status, generated) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/generate"),
            json!({
                "expected_revision":revision,
                "provider_id":provider_id,
                "model":"headless-draft-model",
                "requirement":"Create a headless Agent echo Action"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "generation failed: {generated}");
    let generated_revision = generated["data"]["summary"]["revision"]
        .as_u64()
        .unwrap();
    let save_body = json!({
        "expected_revision":generated_revision,
        "config":{},
        "credential_bindings":{}
    });
    let (status, confirmation) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/save"),
            save_body.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "save confirmation failed: {confirmation}");
    assert_eq!(confirmation["data"]["result"]["outcome"], "confirmation_required");
    let confirmation_id = confirmation["data"]["result"]["confirmation"]["confirmation_id"]
        .as_str()
        .unwrap();
    let mut confirmed_save = save_body;
    confirmed_save.as_object_mut().unwrap().insert(
        "permission_confirmation_id".into(),
        json!(confirmation_id),
    );
    let (status, saved) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/save"),
            confirmed_save,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "confirmed save failed: {saved}");
    assert_eq!(saved["data"]["result"]["outcome"], "installed");
    let detail = &saved["data"]["result"]["plugin"];
    assert_eq!(detail["summary"]["has_ui"], false);
    assert_eq!(detail["summary"]["has_service"], true);
    assert_eq!(detail["summary"]["runtime"]["state"], "running");
    let (plugin_id, _) = installed_identity(detail);
    let action_id = format!("plugin:{plugin_id}/echo");
    let (status, commands) = harness.get_json("/api/plugins/desktop/commands").await;
    assert_eq!(status, StatusCode::OK);
    assert!(commands["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["action_id"] == action_id));
    let (status, invoked) = harness
        .json(
            "POST",
            "/api/plugins/desktop/commands/invoke",
            json!({"action_id":action_id,"input":{"selected":true}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "generated Action failed: {invoked}");
    assert_eq!(invoked["data"]["source"], "chat");
    assert_eq!(invoked["data"]["input"]["selected"], true);
}

fn zip_entries(path: &Path) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|index| archive.by_index(index).unwrap().name().to_owned())
        .collect()
}

fn assert_zip_has_no_secret(path: &Path, plaintext: &str, credential_id: &str) {
    let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        if entry.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert!(
            !bytes.windows(plaintext.len()).any(|window| window == plaintext.as_bytes()),
            "{} leaked credential plaintext",
            entry.name()
        );
        assert!(
            !bytes
                .windows(credential_id.len())
                .any(|window| window == credential_id.as_bytes()),
            "{} leaked a Host Credential identity",
            entry.name()
        );
    }
}
