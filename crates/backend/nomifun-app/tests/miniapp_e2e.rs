//! End-to-end coverage for the clean-start MiniApp M1 HTTP surface.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use nomifun_db::{
    IMiniAppM1Repository, MiniAppM1ProjectSourceState, SqliteMiniAppM1Repository,
    UpdateMiniAppM1ProjectSourceParams,
};
use nomifun_plugin_platform::runtime::{PluginRuntimeSourceFileInput, PluginRuntimeSourceStore};
use serde_json::{Value, json};
use tower::ServiceExt;

const LOCAL_TRUST: &str = "miniapp-m1-local-trust";
const LEGACY_MINIAPP_ID: &str =
    "0190f5fe-7c00-7000-8000-000000000451";

#[tokio::test]
async fn m1_routes_replace_the_legacy_product_chain() {
    let (router, services) = common::build_local_trust_app(LOCAL_TRUST).await;
    let owner_id = services.authoritative_user_id.to_string();
    let owner_jwt = services
        .jwt_service
        .sign(&owner_id, "admin")
        .expect("owner JWT");

    nomifun_db::sqlx::query(
        "INSERT INTO miniapps (
            miniapp_id, user_id, name, description, html, html_size,
            created_at, updated_at
         ) VALUES (?, ?, 'retired', '', '<p>retired</p>', 14, 1, 1)",
    )
    .bind(LEGACY_MINIAPP_ID)
    .bind(&owner_id)
    .execute(services.database.pool())
    .await
    .expect("legacy audit row");

    let response = request(&router, Method::GET, "/api/plugins/runtimes", None)
        .header("authorization", format!("Bearer {owner_jwt}"))
        .send()
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let library = response_json(response).await;
    assert_eq!(library["data"]["library_revision"], 0);
    assert_eq!(library["data"]["plugins"], json!([]));

    let create_body = json!({
        "expected_library_revision": 0,
        "display_name": "M1 Notes",
        "description": "clean-start project",
        "kind": "ui_only"
    });
    let response = request(
        &router,
        Method::POST,
        "/api/plugins/runtimes/projects",
        Some(create_body.clone()),
    )
    .header("authorization", format!("Bearer {owner_jwt}"))
    .send()
    .await;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "an owner JWT alone must not authorize host-local creation"
    );

    let response = request(
        &router,
        Method::POST,
        "/api/plugins/runtimes/projects",
        Some(create_body),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let created = response_json(response).await;
    let miniapp_id = created["data"]["plugin"]["plugin_id"]
        .as_str()
        .expect("created miniapp_id")
        .to_owned();
    nomifun_common::MiniAppId::parse(miniapp_id.clone())
        .expect("canonical Plugin UUIDv7");
    assert_eq!(created["data"]["plugin"]["display_name"], "M1 Notes");
    assert_eq!(created["data"]["source_state"], "editable");
    assert_eq!(created["data"]["project_revision"], 1);
    assert_eq!(created["data"]["build_generation"], 1);
    assert_eq!(created["data"]["plugin"]["surface_available"], false);

    let unified = request(&router, Method::GET, "/api/plugins", None)
        .header("authorization", format!("Bearer {owner_jwt}"))
        .send().await;
    assert_eq!(unified.status(), StatusCode::OK);
    let unified = response_json(unified).await;
    assert_eq!(unified["data"]["runtimes"][0]["plugin_id"], miniapp_id);
    assert!(unified["data"]["runtimes"][0].get("miniapp_id").is_none());

    let response = request(
        &router,
        Method::GET,
        &format!("/api/plugins/runtimes/{miniapp_id}/workshop"),
        None,
    )
    .header("authorization", format!("Bearer {owner_jwt}"))
    .send()
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "owner-scoped reads do not require local trust"
    );
    assert_eq!(response_json(response).await["data"], created["data"]);

    let product_owner: String = nomifun_db::sqlx::query_scalar(
        "SELECT owner_user_id FROM miniapp_products WHERE miniapp_id = ?",
    )
    .bind(&miniapp_id)
    .fetch_one(services.database.pool())
    .await
    .expect("M1 product owner");
    assert_eq!(product_owner, owner_id);
    let legacy_name: String = nomifun_db::sqlx::query_scalar(
        "SELECT name FROM miniapps WHERE miniapp_id = ?",
    )
    .bind(LEGACY_MINIAPP_ID)
    .fetch_one(services.database.pool())
    .await
    .expect("legacy row remains frozen");
    assert_eq!(legacy_name, "retired");

    for (method, path) in [
        (Method::POST, "/api/plugins/runtimes".to_owned()),
        (Method::GET, format!("/api/plugins/runtimes/{miniapp_id}")),
        (
            Method::PUT,
            format!("/api/plugins/runtimes/{miniapp_id}"),
        ),
        (
            Method::DELETE,
            format!("/api/plugins/runtimes/{miniapp_id}"),
        ),
        (
            Method::GET,
            format!("/api/plugins/runtimes/{miniapp_id}/serve"),
        ),
        (
            Method::POST,
            format!("/api/plugins/runtimes/{miniapp_id}/workspace"),
        ),
        (Method::POST, "/api/plugins/runtimes/validate".to_owned()),
        (Method::POST, "/api/plugins/runtimes/import".to_owned()),
    ] {
        let response = request(&router, method.clone(), &path, Some(json!({})))
            .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
            .send()
            .await;
        assert!(
            matches!(
                response.status(),
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            "{method} {path} must not reach the retired Plugin chain: {}",
            response.status()
        );
    }

    services
        .shutdown_browser_platform()
        .await
        .expect("background cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn ui_only_miniapp_completes_publish_enable_surface_and_rollback_chain() {
    let (router, services) = common::build_local_trust_app(LOCAL_TRUST).await;
    let owner_id = services.authoritative_user_id.to_string();
    let owner_jwt = services
        .jwt_service
        .sign(&owner_id, "admin")
        .expect("owner JWT");
    let create = request(
        &router,
        Method::POST,
        "/api/plugins/runtimes/projects",
        Some(json!({
            "expected_library_revision": 0,
            "display_name": "Surface Notes",
            "description": "UI-only lifecycle",
            "kind": "ui_only"
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(create.status(), StatusCode::OK);
    let created = response_json(create).await["data"].clone();
    let miniapp_id = created["plugin"]["plugin_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let build_body = json!({
        "plugin_id": miniapp_id,
        "expected_product_revision": created["plugin"]["product_revision"],
        "project_id": created["project_id"],
        "expected_project_revision": created["project_revision"],
        "expected_build_generation": created["build_generation"],
        "expected_source_snapshot_digest": created["source_snapshot_digest"],
        "expected_dependency_lock_digest": created["dependency_lock_digest"]
    });
    let build = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/build"),
        Some(build_body),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(build.status(), StatusCode::OK);
    let ready = response_json(build).await["data"].clone();
    assert_eq!(ready["plugin"]["surface_available"], false);
    assert!(ready["ready"]["release"]["release_id"].as_str().is_some());

    let premature_auto = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/publish-mode"),
        Some(json!({
            "plugin_id": miniapp_id,
            "expected_product_revision": ready["plugin"]["product_revision"],
            "expected_pointer_revision": ready["plugin"]["releases"]["pointer_revision"],
            "mode": "auto_ui_only"
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(premature_auto.status(), StatusCode::BAD_REQUEST);

    let publish_body = json!({
        "plugin_id": miniapp_id,
        "expected_product_revision": ready["plugin"]["product_revision"],
        "expected_pointer_revision": ready["plugin"]["releases"]["pointer_revision"],
        "expected_active_release_epoch": ready["plugin"]["releases"]["active_release_epoch"],
        "ready_release_id": ready["ready"]["release"]["release_id"],
        "expected_ready_release_digest": ready["ready"]["release"]["release_digest"],
        "acknowledge_test_warning": false
    });
    let publish = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/publish"),
        Some(publish_body),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(publish.status(), StatusCode::OK);
    let published = response_json(publish).await["data"].clone();
    assert_eq!(published["plugin"]["lifecycle"], "disabled");
    assert_eq!(published["plugin"]["surface_available"], false);
    assert!(published["plugin"]["releases"]["active"].is_object());
    assert!(published["ready"].is_null());

    let enable_body = json!({
        "plugin_id": miniapp_id,
        "expected_product_revision": published["plugin"]["product_revision"],
        "expected_pointer_revision": published["plugin"]["releases"]["pointer_revision"],
        "expected_active_release_digest": published["plugin"]["releases"]["active"]["release_digest"],
        "enabled": true
    });
    let enable = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/enabled"),
        Some(enable_body),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(enable.status(), StatusCode::OK);
    let enabled = response_json(enable).await["data"].clone();
    assert_eq!(enabled["plugin"]["lifecycle"], "enabled");
    assert_eq!(enabled["plugin"]["surface_available"], true);
    assert_eq!(
        enabled["plugin"]["releases"]["active_release_epoch"],
        1
    );

    let old_surface_get = request(
        &router,
        Method::GET,
        &format!("/api/plugins/runtimes/{miniapp_id}/surface"),
        None,
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(old_surface_get.status(), StatusCode::NOT_FOUND);

    let wrong_method = request(
        &router,
        Method::GET,
        &format!("/api/plugins/runtimes/{miniapp_id}/surface/open"),
        None,
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);

    let owner_only = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/surface/open"),
        Some(json!({ "plugin_id": miniapp_id })),
    )
    .header("authorization", format!("Bearer {owner_jwt}"))
    .send()
    .await;
    assert_eq!(
        owner_only.status(),
        StatusCode::FORBIDDEN,
        "Surface capability signing must require host-local trust"
    );

    let mismatched_open = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/surface/open"),
        Some(json!({
            "plugin_id": "0190f5fe-7c00-7000-8000-000000000452"
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(mismatched_open.status(), StatusCode::BAD_REQUEST);
    let open_session_count: i64 =
        nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM miniapp_surface_sessions")
            .fetch_one(services.database.pool())
            .await
            .unwrap();
    assert_eq!(
        open_session_count, 0,
        "failed Surface open attempts must not sign or persist a capability"
    );

    let surface = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/surface/open"),
        Some(json!({ "plugin_id": miniapp_id })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(surface.status(), StatusCode::OK);
    let descriptor = response_json(surface).await["data"].clone();
    let capability = descriptor["surface_capability"].as_str().unwrap();
    let epoch = descriptor["active_release_epoch"].as_u64().unwrap();
    let digest = descriptor["expected_release_digest"].as_str().unwrap();
    let entrypoint = descriptor["ui_entrypoint"].as_str().unwrap();
    let asset = request(
        &router,
        Method::GET,
        &format!(
            "/api/plugins/runtimes/{miniapp_id}/surface/assets/{capability}/{epoch}/{digest}/{entrypoint}"
        ),
        None,
    )
    .send()
    .await;
    assert_eq!(asset.status(), StatusCode::OK);
    assert_eq!(
        asset.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    let asset_body = asset
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert!(String::from_utf8_lossy(&asset_body).contains("Surface Notes"));

    let bridge_path = format!("/api/plugins/runtimes/{miniapp_id}/surface/bridge");
    let bridge_set = request(
        &router,
        Method::POST,
        &bridge_path,
        Some(json!({
            "surface_capability": capability,
            "active_release_epoch": epoch,
            "expected_release_digest": digest,
            "request": {
                "call_id": "set-preference",
                "target": {
                    "target": "host_kv",
                    "request": {
                        "operation": "set",
                        "key": "preference",
                        "value": {"density": "compact"}
                    }
                }
            }
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(bridge_set.status(), StatusCode::OK);
    assert_eq!(response_json(bridge_set).await["data"]["outcome"], "written");

    let bridge_get = request(
        &router,
        Method::POST,
        &bridge_path,
        Some(json!({
            "surface_capability": capability,
            "active_release_epoch": epoch,
            "expected_release_digest": digest,
            "request": {
                "call_id": "get-preference",
                "target": {
                    "target": "host_kv",
                    "request": {
                        "operation": "get",
                        "key": "preference"
                    }
                }
            }
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(bridge_get.status(), StatusCode::OK);
    let bridge_value = response_json(bridge_get).await;
    assert_eq!(
        bridge_value["data"]["value"]["density"],
        "compact"
    );
    assert_eq!(bridge_value["data"]["revision"], 1);

    let bridge_cas = request(
        &router,
        Method::POST,
        &bridge_path,
        Some(json!({
            "surface_capability": capability,
            "active_release_epoch": epoch,
            "expected_release_digest": digest,
            "request": {
                "call_id": "cas-preference",
                "target": {
                    "target": "host_kv",
                    "request": {
                        "operation": "compare_and_swap",
                        "key": "preference",
                        "expected_revision": 1,
                        "value": {"density": "comfortable"}
                    }
                }
            }
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(bridge_cas.status(), StatusCode::OK);
    let bridge_cas = response_json(bridge_cas).await;
    assert_eq!(bridge_cas["data"]["applied"], true);
    assert_eq!(bridge_cas["data"]["current_revision"], 2);

    let second_build_body = json!({
        "plugin_id": miniapp_id,
        "expected_product_revision": enabled["plugin"]["product_revision"],
        "project_id": enabled["project_id"],
        "expected_project_revision": enabled["project_revision"],
        "expected_build_generation": enabled["build_generation"],
        "expected_source_snapshot_digest": enabled["source_snapshot_digest"],
        "expected_dependency_lock_digest": enabled["dependency_lock_digest"]
    });
    let unchanged_build = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/build"),
        Some(second_build_body.clone()),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(unchanged_build.status(), StatusCode::BAD_REQUEST);

    let repository = SqliteMiniAppM1Repository::new(services.database.pool().clone());
    let before_edit = repository
        .get(&owner_id, &miniapp_id)
        .await
        .unwrap()
        .unwrap();
    let source_store =
        PluginRuntimeSourceStore::new(services.data_dir.join("miniapp-m1").join("source"))
            .unwrap();
    let source = source_store
        .replace_source(
            &owner_id,
            &miniapp_id,
            &before_edit.project.project_id,
            before_edit.project.source_head_digest.as_deref().unwrap(),
            vec![PluginRuntimeSourceFileInput::new(
                "ui/index.html",
                b"<!doctype html><html><body><main><h1>Surface Notes v2</h1></main></body></html>"
                    .to_vec(),
            )],
        )
        .unwrap();
    repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner_id.clone(),
            miniapp_id: miniapp_id.clone(),
            project_id: before_edit.project.project_id.clone(),
            expected_project_revision: before_edit.project.project_revision,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some(source.managed_relative_path),
            source_head_digest: Some(source.source_snapshot_digest.0),
            dependency_lock_digest: Some(source.dependency_lock_digest.0),
            build_profile_version: Some(source.build_profile_version.0),
            build_generation: i64::try_from(source.build_generation).unwrap(),
            updated_at: nomifun_common::now_ms()
                .max(before_edit.product.updated_at)
                .max(before_edit.project.updated_at),
        })
        .await
        .unwrap();
    let refreshed = request(
        &router,
        Method::GET,
        &format!("/api/plugins/runtimes/{miniapp_id}/workshop"),
        None,
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(refreshed.status(), StatusCode::OK);
    let edited = response_json(refreshed).await["data"].clone();
    let second_build_body = json!({
        "plugin_id": miniapp_id,
        "expected_product_revision": edited["plugin"]["product_revision"],
        "project_id": edited["project_id"],
        "expected_project_revision": edited["project_revision"],
        "expected_build_generation": edited["build_generation"],
        "expected_source_snapshot_digest": edited["source_snapshot_digest"],
        "expected_dependency_lock_digest": edited["dependency_lock_digest"]
    });
    let second_build = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/build"),
        Some(second_build_body),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(second_build.status(), StatusCode::OK);
    let second_ready = response_json(second_build).await["data"].clone();
    assert!(second_ready["ready"].is_object());

    let second_publish_body = json!({
        "plugin_id": miniapp_id,
        "expected_product_revision": second_ready["plugin"]["product_revision"],
        "expected_pointer_revision": second_ready["plugin"]["releases"]["pointer_revision"],
        "expected_active_release_epoch": second_ready["plugin"]["releases"]["active_release_epoch"],
        "ready_release_id": second_ready["ready"]["release"]["release_id"],
        "expected_ready_release_digest": second_ready["ready"]["release"]["release_digest"],
        "expected_active_release_digest": second_ready["plugin"]["releases"]["active"]["release_digest"],
        "acknowledge_test_warning": false
    });
    let second_publish = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/publish"),
        Some(second_publish_body),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(second_publish.status(), StatusCode::OK);
    let second_published = response_json(second_publish).await["data"].clone();
    assert_eq!(
        second_published["plugin"]["releases"]["active_release_epoch"],
        2
    );
    assert!(second_published["plugin"]["releases"]["previous"].is_object());

    let old_asset = request(
        &router,
        Method::GET,
        &format!(
            "/api/plugins/runtimes/{miniapp_id}/surface/assets/{capability}/{epoch}/{digest}/{entrypoint}"
        ),
        None,
    )
    .send()
    .await;
    assert_eq!(old_asset.status(), StatusCode::NOT_FOUND);
    let old_bridge = request(
        &router,
        Method::POST,
        &bridge_path,
        Some(json!({
            "surface_capability": capability,
            "active_release_epoch": epoch,
            "expected_release_digest": digest,
            "request": {
                "call_id": "stale-get",
                "target": {
                    "target": "host_kv",
                    "request": {
                        "operation": "get",
                        "key": "preference"
                    }
                }
            }
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(old_bridge.status(), StatusCode::NOT_FOUND);

    let rollback_body = json!({
        "plugin_id": miniapp_id,
        "expected_product_revision": second_published["plugin"]["product_revision"],
        "expected_pointer_revision": second_published["plugin"]["releases"]["pointer_revision"],
        "expected_active_release_epoch": second_published["plugin"]["releases"]["active_release_epoch"],
        "expected_current_release_digest": second_published["plugin"]["releases"]["active"]["release_digest"],
        "previous_release_id": second_published["plugin"]["releases"]["previous"]["release_id"],
        "expected_previous_release_digest": second_published["plugin"]["releases"]["previous"]["release_digest"]
    });
    let rollback = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/rollback"),
        Some(rollback_body),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(rollback.status(), StatusCode::OK);
    let rolled_back = response_json(rollback).await["data"].clone();
    assert_eq!(
        rolled_back["plugin"]["releases"]["active"]["release_digest"],
        enabled["plugin"]["releases"]["active"]["release_digest"]
    );
    assert_eq!(
        rolled_back["plugin"]["releases"]["active_release_epoch"],
        3
    );
    assert_eq!(rolled_back["plugin"]["lifecycle"], "enabled");

    let auto_mode = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/publish-mode"),
        Some(json!({
            "plugin_id": miniapp_id,
            "expected_product_revision": rolled_back["plugin"]["product_revision"],
            "expected_pointer_revision": rolled_back["plugin"]["releases"]["pointer_revision"],
            "mode": "auto_ui_only"
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(auto_mode.status(), StatusCode::OK);
    let auto_enabled = response_json(auto_mode).await["data"].clone();
    assert_eq!(auto_enabled["publish_mode"], "auto_ui_only");

    let pre_auto_surface = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/surface/open"),
        Some(json!({ "plugin_id": miniapp_id })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(pre_auto_surface.status(), StatusCode::OK);
    let pre_auto_descriptor = response_json(pre_auto_surface).await["data"].clone();

    let before_auto_edit = repository
        .get(&owner_id, &miniapp_id)
        .await
        .unwrap()
        .unwrap();
    let auto_source = source_store
        .replace_source(
            &owner_id,
            &miniapp_id,
            &before_auto_edit.project.project_id,
            before_auto_edit
                .project
                .source_head_digest
                .as_deref()
                .unwrap(),
            vec![PluginRuntimeSourceFileInput::new(
                "ui/index.html",
                b"<!doctype html><html><body><main><h1>Surface Notes auto</h1></main></body></html>"
                    .to_vec(),
            )],
        )
        .unwrap();
    repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner_id.clone(),
            miniapp_id: miniapp_id.clone(),
            project_id: before_auto_edit.project.project_id.clone(),
            expected_project_revision: before_auto_edit.project.project_revision,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some(auto_source.managed_relative_path),
            source_head_digest: Some(auto_source.source_snapshot_digest.0),
            dependency_lock_digest: Some(auto_source.dependency_lock_digest.0),
            build_profile_version: Some(auto_source.build_profile_version.0),
            build_generation: i64::try_from(auto_source.build_generation).unwrap(),
            updated_at: nomifun_common::now_ms()
                .max(before_auto_edit.product.updated_at)
                .max(before_auto_edit.project.updated_at),
        })
        .await
        .unwrap();
    let auto_edited = request(
        &router,
        Method::GET,
        &format!("/api/plugins/runtimes/{miniapp_id}/workshop"),
        None,
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(auto_edited.status(), StatusCode::OK);
    let auto_edited = response_json(auto_edited).await["data"].clone();
    let auto_build = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/build"),
        Some(json!({
            "plugin_id": miniapp_id,
            "expected_product_revision": auto_edited["plugin"]["product_revision"],
            "project_id": auto_edited["project_id"],
            "expected_project_revision": auto_edited["project_revision"],
            "expected_build_generation": auto_edited["build_generation"],
            "expected_source_snapshot_digest": auto_edited["source_snapshot_digest"],
            "expected_dependency_lock_digest": auto_edited["dependency_lock_digest"]
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(auto_build.status(), StatusCode::OK);
    let auto_published = response_json(auto_build).await["data"].clone();
    assert!(auto_published["ready"].is_null());
    assert_eq!(auto_published["publish_mode"], "auto_ui_only");
    assert_eq!(
        auto_published["plugin"]["releases"]["active_release_epoch"],
        4
    );
    assert_eq!(
        auto_published["plugin"]["releases"]["previous"]["release_id"],
        rolled_back["plugin"]["releases"]["active"]["release_id"]
    );
    let pre_auto_asset = request(
        &router,
        Method::GET,
        &format!(
            "/api/plugins/runtimes/{miniapp_id}/surface/assets/{}/{}/{}/{}",
            pre_auto_descriptor["surface_capability"].as_str().unwrap(),
            pre_auto_descriptor["active_release_epoch"].as_u64().unwrap(),
            pre_auto_descriptor["expected_release_digest"].as_str().unwrap(),
            pre_auto_descriptor["ui_entrypoint"].as_str().unwrap(),
        ),
        None,
    )
    .send()
    .await;
    assert_eq!(pre_auto_asset.status(), StatusCode::NOT_FOUND);

    let current_surface = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/surface/open"),
        Some(json!({ "plugin_id": miniapp_id })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(current_surface.status(), StatusCode::OK);
    let current_descriptor = response_json(current_surface).await["data"].clone();

    let before_non_ui_edit = repository
        .get(&owner_id, &miniapp_id)
        .await
        .unwrap()
        .unwrap();
    let renamed_at = nomifun_common::now_ms().max(before_non_ui_edit.product.updated_at);
    nomifun_db::sqlx::query(
        "UPDATE miniapp_products
         SET product_revision = product_revision + 1,
             display_name = 'Surface Notes renamed', updated_at = ?
         WHERE owner_user_id = ? AND miniapp_id = ?
           AND product_revision = ? AND updated_at <= ?",
    )
    .bind(renamed_at)
    .bind(&owner_id)
    .bind(&miniapp_id)
    .bind(before_non_ui_edit.product.product_revision)
    .bind(renamed_at)
    .execute(services.database.pool())
    .await
    .unwrap();
    let non_ui_source = source_store
        .replace_source(
            &owner_id,
            &miniapp_id,
            &before_non_ui_edit.project.project_id,
            before_non_ui_edit
                .project
                .source_head_digest
                .as_deref()
                .unwrap(),
            vec![PluginRuntimeSourceFileInput::new(
                "ui/index.html",
                b"<!doctype html><html><body><main><h1>Surface Notes ready only</h1></main></body></html>"
                    .to_vec(),
            )],
        )
        .unwrap();
    repository
        .update_project_source_cas(&UpdateMiniAppM1ProjectSourceParams {
            owner_user_id: owner_id.clone(),
            miniapp_id: miniapp_id.clone(),
            project_id: before_non_ui_edit.project.project_id.clone(),
            expected_project_revision: before_non_ui_edit.project.project_revision,
            source_state: MiniAppM1ProjectSourceState::Editable,
            managed_source_path: Some(non_ui_source.managed_relative_path),
            source_head_digest: Some(non_ui_source.source_snapshot_digest.0),
            dependency_lock_digest: Some(non_ui_source.dependency_lock_digest.0),
            build_profile_version: Some(non_ui_source.build_profile_version.0),
            build_generation: i64::try_from(non_ui_source.build_generation).unwrap(),
            updated_at: nomifun_common::now_ms()
                .max(renamed_at)
                .max(before_non_ui_edit.project.updated_at),
        })
        .await
        .unwrap();
    let non_ui_edited = request(
        &router,
        Method::GET,
        &format!("/api/plugins/runtimes/{miniapp_id}/workshop"),
        None,
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(non_ui_edited.status(), StatusCode::OK);
    let non_ui_edited = response_json(non_ui_edited).await["data"].clone();
    let ready_only_build = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/build"),
        Some(json!({
            "plugin_id": miniapp_id,
            "expected_product_revision": non_ui_edited["plugin"]["product_revision"],
            "project_id": non_ui_edited["project_id"],
            "expected_project_revision": non_ui_edited["project_revision"],
            "expected_build_generation": non_ui_edited["build_generation"],
            "expected_source_snapshot_digest": non_ui_edited["source_snapshot_digest"],
            "expected_dependency_lock_digest": non_ui_edited["dependency_lock_digest"]
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(ready_only_build.status(), StatusCode::OK);
    let ready_only = response_json(ready_only_build).await["data"].clone();
    assert!(ready_only["ready"].is_object());
    assert_eq!(
        ready_only["plugin"]["releases"]["active_release_epoch"],
        auto_published["plugin"]["releases"]["active_release_epoch"]
    );
    assert_eq!(
        ready_only["plugin"]["releases"]["active"]["release_id"],
        auto_published["plugin"]["releases"]["active"]["release_id"]
    );
    let surviving_asset = request(
        &router,
        Method::GET,
        &format!(
            "/api/plugins/runtimes/{miniapp_id}/surface/assets/{}/{}/{}/{}",
            current_descriptor["surface_capability"].as_str().unwrap(),
            current_descriptor["active_release_epoch"].as_u64().unwrap(),
            current_descriptor["expected_release_digest"].as_str().unwrap(),
            current_descriptor["ui_entrypoint"].as_str().unwrap(),
        ),
        None,
    )
    .send()
    .await;
    assert_eq!(
        surviving_asset.status(),
        StatusCode::OK,
        "Ready-only Build must not revoke the unchanged Active Surface"
    );
    let latest_operation_state: String = nomifun_db::sqlx::query_scalar(
        "SELECT state FROM product_operations
         WHERE owner_kind = 'miniapp' AND owner_id = ? AND kind = 'build'
         ORDER BY started_at_ms DESC, operation_id DESC LIMIT 1",
    )
    .bind(&miniapp_id)
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert_eq!(latest_operation_state, "succeeded");

    let manual_mode = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/publish-mode"),
        Some(json!({
            "plugin_id": miniapp_id,
            "expected_product_revision": ready_only["plugin"]["product_revision"],
            "expected_pointer_revision": ready_only["plugin"]["releases"]["pointer_revision"],
            "mode": "manual"
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(manual_mode.status(), StatusCode::OK);
    assert_eq!(response_json(manual_mode).await["data"]["publish_mode"], "manual");

    let close_surface = request(
        &router,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/surface/close"),
        Some(json!({
            "plugin_id": miniapp_id,
            "surface_session_id": current_descriptor["surface_session_id"],
            "surface_capability": current_descriptor["surface_capability"]
        })),
    )
    .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_TRUST)
    .send()
    .await;
    assert_eq!(close_surface.status(), StatusCode::OK);
    assert_eq!(response_json(close_surface).await["data"], true);
    let closed_asset = request(
        &router,
        Method::GET,
        &format!(
            "/api/plugins/runtimes/{miniapp_id}/surface/assets/{}/{}/{}/{}",
            current_descriptor["surface_capability"].as_str().unwrap(),
            current_descriptor["active_release_epoch"].as_u64().unwrap(),
            current_descriptor["expected_release_digest"].as_str().unwrap(),
            current_descriptor["ui_entrypoint"].as_str().unwrap(),
        ),
        None,
    )
    .send()
    .await;
    assert_eq!(closed_asset.status(), StatusCode::NOT_FOUND);

    services
        .shutdown_browser_platform()
        .await
        .expect("background cleanup");
    services.database.close().await;
}

struct RequestBuilder<'a> {
    router: &'a axum::Router,
    builder: axum::http::request::Builder,
    body: Body,
}

impl RequestBuilder<'_> {
    fn header(
        mut self,
        name: &'static str,
        value: impl AsRef<str>,
    ) -> Self {
        self.builder = self.builder.header(name, value.as_ref());
        self
    }

    async fn send(self) -> axum::response::Response {
        self.router
            .clone()
            .oneshot(self.builder.body(self.body).expect("request"))
            .await
            .expect("route response")
    }
}

fn request<'a>(
    router: &'a axum::Router,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> RequestBuilder<'a> {
    let mut builder = Request::builder().method(method).uri(path);
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&body).expect("request JSON"))
        }
        None => Body::empty(),
    };
    RequestBuilder {
        router,
        builder,
        body,
    }
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("response JSON")
}
