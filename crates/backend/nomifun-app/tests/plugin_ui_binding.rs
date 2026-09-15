//! Preset presentation consent uses the real product Catalog and original store.
use super::*;

#[tokio::test]
async fn preset_ui_binding_is_persistent_exact_owned_and_independent_of_execution() {
    // Use a file-backed fixture because this card promises durable consent.
    let root = tempfile::Builder::new()
        .prefix("nomifun-ui-binding-")
        .tempdir()
        .unwrap();
    let database = nomifun_db::init_database(&root.path().join("source.sqlite"))
        .await
        .unwrap();
    let services = nomifun_app::compatibility::AppServices::from_config(
        database,
        &nomifun_app::AppConfig {
            data_dir: root.path().join("data"),
            work_dir: root.path().join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(std::sync::Arc::from(TRUST)),
            ..nomifun_app::AppConfig::default()
        },
    )
    .await
    .unwrap();
    let router = nomifun_app::compatibility::create_router(&services).await;
    let upstream = wiremock::MockServer::start().await;
    let provider = post(&router, "/api/providers", json!({
        "platform": "stepfun-plan", "name": "UI binding model",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": {"api_keys": ["test-only"]},
        "enabled": true, "initial_model": {"model": "step-3.7-flash", "enabled": true,
            "capabilities": [{"task": "chat", "traits": ["function_calling", "streaming"],
                "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {}}]}
    })).await;
    let preset = post(
        &router,
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Remember this Agent page", "reuse_existing": false,
            "model": {"provider_id": provider["provider_id"], "model": "step-3.7-flash"}
        }),
    )
    .await;
    let preset_id = preset["preset"]["preset_id"].as_str().unwrap();
    let binding_path = format!("/api/agent-presets/{preset_id}/ui-binding");
    let editor_path = format!("/api/agent-presets/{preset_id}/editor");
    let editor_before = get(&router, &editor_path).await;
    let session = post(
        &router,
        "/api/agent-sessions",
        json!({
            "preset_id": preset_id, "title": "First session"
        }),
    )
    .await;
    let session_id = session["agent_session_id"].as_str().unwrap();
    let session_path = format!("/api/agent-sessions/{session_id}");
    let session_binding_path = format!("{session_path}/ui-binding");
    let session_before = get(&router, &session_path).await;
    let initial = get(&router, &binding_path).await;
    assert_eq!(
        initial["binding"],
        json!({"binding_version": 0, "selection": null})
    );
    assert_eq!(get(&router, &session_binding_path).await, initial);

    let plugin = install_ui(&router).await;
    let plugin_id = plugin["plugin_id"].as_str().unwrap();
    publish_agent_view(&router, plugin_id).await;
    let choice = get(&router, "/api/agent-catalog/ui/agent-session").await[0].clone();
    let mut spoofed = choice.clone();
    spoofed["display_name"] = json!("Untrusted label");
    let (status, saved) = request(
        &router,
        "PUT",
        &binding_path,
        json!({
            "expected_binding_version": 0, "selection": spoofed
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let saved = saved["data"].clone();
    assert_eq!(
        saved["binding"],
        json!({"binding_version": 1, "selection": choice})
    );
    assert_eq!(get(&router, &session_binding_path).await, saved);
    assert_eq!(
        get(&router, &editor_path).await,
        editor_before,
        "page consent must not rewrite execution Revision/Snapshot"
    );
    assert_eq!(
        get(&router, &session_path).await,
        session_before,
        "page consent must not mutate the Session"
    );

    // The HTTP contract requires explicit null to clear, never an omitted field.
    assert_eq!(
        request(
            &router,
            "PUT",
            &binding_path,
            json!({"expected_binding_version": 1})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for field in ["plugin_id", "expected_release_digest", "capability"] {
        let mut invalid = choice.clone();
        invalid[field] = match field {
            "plugin_id" => json!(uuid::Uuid::now_v7().to_string()),
            "capability" => json!({"id": choice["capability"]["id"], "version": "99.0.0"}),
            _ => json!("0".repeat(64)),
        };
        assert_eq!(
            request(
                &router,
                "PUT",
                &binding_path,
                json!({
                    "expected_binding_version": 1, "selection": invalid
                })
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        request(
            &router,
            "PUT",
            &binding_path,
            json!({
                "expected_binding_version": 0, "selection": null
            })
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let write = json!({"expected_binding_version": 1, "selection": choice});
    let (left, right) = tokio::join!(
        request(&router, "PUT", &binding_path, write.clone()),
        request(&router, "PUT", &binding_path, write),
    );
    assert!(
        (left.0 == StatusCode::OK && right.0 == StatusCode::CONFLICT)
            || (right.0 == StatusCode::OK && left.0 == StatusCode::CONFLICT)
    );

    // Read through the public routes; disk reopen below separately proves
    // persistence. The runtime composition remains single-owner/single-install.
    let persisted = get(&router, &binding_path).await;
    assert_eq!(
        persisted["binding"],
        json!({"binding_version": 2, "selection": choice})
    );
    let second = post(
        &router,
        "/api/agent-sessions",
        json!({"preset_id": preset_id}),
    )
    .await;
    let second_id = second["agent_session_id"].as_str().unwrap();
    assert_eq!(
        get(
            &router,
            &format!("/api/agent-sessions/{second_id}/ui-binding")
        )
        .await,
        persisted
    );
    let grant = json!({"plugin_id": plugin_id, "agent_session": {
        "agent_session_id": second_id, "expected_release_digest": choice["expected_release_digest"],
        "ui_capability": choice["capability"]
    }});
    let open_path = format!("/api/plugins/runtimes/{plugin_id}/surface/open");
    let surface = post(&router, &open_path, grant.clone()).await;
    let observed = post(
        &router,
        &format!("/api/plugins/runtimes/{plugin_id}/surface/bridge"),
        bridge_body(
            &surface,
            json!({"operation": "observe", "after_seq": 0, "limit": 10}),
        ),
    )
    .await;
    assert_eq!(observed["session"]["agent_session_id"], second_id);
    assert_eq!(observed["messages"], json!([]));
    assert!(upstream.received_requests().await.unwrap().is_empty());

    let unrelated = post(
        &router,
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Another Agent", "reuse_existing": false,
            "model": {"provider_id": provider["provider_id"], "model": "step-3.7-flash"}
        }),
    )
    .await;
    let unrelated_id = unrelated["preset"]["preset_id"].as_str().unwrap();
    assert_ne!(unrelated_id, preset_id);
    let unrelated_path = format!("/api/agent-presets/{unrelated_id}/ui-binding");
    assert_eq!(
        get(&router, &unrelated_path).await["binding"],
        initial["binding"]
    );
    // Workbench entry: configure the fresh preset before its first Session.
    // Creating the empty Session afterwards requires no initial message or
    // model override and reads that preset's independently saved presentation.
    let (status, pre_session) = request(
        &router,
        "PUT",
        &unrelated_path,
        json!({
            "expected_binding_version": 0, "selection": choice
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pre_session}");
    let first = post(
        &router,
        "/api/agent-sessions",
        json!({
            "preset_id": unrelated_id, "title": "Workbench opens empty Agent page"
        }),
    )
    .await;
    let first_id = first["agent_session_id"].as_str().unwrap();
    assert_eq!(
        get(
            &router,
            &format!("/api/agent-sessions/{first_id}/ui-binding")
        )
        .await,
        pre_session["data"]
    );
    assert_eq!(
        get(&router, &format!("/api/agent-sessions/{first_id}")).await["messages"],
        json!([])
    );
    assert_eq!(
        get(&router, &binding_path).await,
        persisted,
        "another Agent's page choice must stay independent"
    );
    assert!(upstream.received_requests().await.unwrap().is_empty());
    let missing = format!("/api/agent-presets/{}/ui-binding", uuid::Uuid::now_v7());
    assert_eq!(
        request(&router, "GET", &missing, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    // Controlled fixture: the public owner must not read/write another owner.
    let other_owner = uuid::Uuid::now_v7().to_string();
    sqlx::query("UPDATE nomi_agent_presets SET owner_user_id = ? WHERE preset_id = ?")
        .bind(&other_owner)
        .bind(unrelated_id)
        .execute(services.database.pool())
        .await
        .unwrap();
    assert!(
        !request(&router, "GET", &unrelated_path, Value::Null)
            .await
            .0
            .is_success()
    );
    assert!(
        !request(
            &router,
            "PUT",
            &unrelated_path,
            json!({"expected_binding_version": 1, "selection": choice})
        )
        .await
        .0
        .is_success()
    );
    let owner = nomifun_db::installation_owner_id(services.database.pool())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE nomi_agent_presets SET owner_user_id = ?, retired_at_ms = 1 WHERE preset_id = ?",
    )
    .bind(&owner)
    .bind(unrelated_id)
    .execute(services.database.pool())
    .await
    .unwrap();
    assert_eq!(
        request(&router, "GET", &unrelated_path, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    sqlx::query("UPDATE nomi_agent_presets SET retired_at_ms = NULL WHERE preset_id = ?")
        .bind(unrelated_id)
        .execute(services.database.pool())
        .await
        .unwrap();

    // Catalog metadata alone cannot authorize another owner's product.
    sqlx::query("UPDATE plugin_products SET owner_user_id = ? WHERE plugin_product_id = ?")
        .bind(&other_owner)
        .bind(plugin_id)
        .execute(services.database.pool())
        .await
        .unwrap();
    assert_eq!(
        request(
            &router,
            "PUT",
            &binding_path,
            json!({
                "expected_binding_version": 2, "selection": choice
            })
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let invalid_scope = nomifun_db::validate_id_data_contract(services.database.pool())
        .await
        .unwrap_err();
    assert!(
        invalid_scope
            .to_string()
            .contains("cross-owner plugin reference")
    );
    sqlx::query("UPDATE plugin_products SET owner_user_id = ? WHERE plugin_product_id = ?")
        .bind(&owner)
        .bind(plugin_id)
        .execute(services.database.pool())
        .await
        .unwrap();

    let w = get(
        &router,
        &format!("/api/plugins/runtimes/{plugin_id}/workshop"),
    )
    .await;
    post(&router, &format!("/api/plugins/runtimes/{plugin_id}/enabled"), json!({
        "plugin_id": plugin_id, "expected_product_revision": w["plugin"]["product_revision"],
        "expected_pointer_revision": w["plugin"]["releases"]["pointer_revision"],
        "expected_active_release_digest": w["plugin"]["releases"]["active"]["release_digest"], "enabled": false
    })).await;
    assert_eq!(
        get(&router, &binding_path).await,
        persisted,
        "withdrawal must retain explicit consent, not select another plugin"
    );
    assert_eq!(
        request(
            &router,
            "PUT",
            &binding_path,
            json!({
                "expected_binding_version": 2, "selection": choice
            })
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(
        !request(&router, "POST", &open_path, grant)
            .await
            .0
            .is_success()
    );

    // Exercise the actual schema/ID registry with a nonempty stored choice.
    nomifun_db::validate_id_schema_contract(services.database.pool())
        .await
        .unwrap();
    nomifun_db::validate_id_data_contract(services.database.pool())
        .await
        .unwrap();
    // A historical choice with no remaining product is retained, not repaired
    // into another plugin. Invalid identities are still rejected by the audit.
    let mut historical = persisted["binding"].clone();
    historical["selection"]["plugin_id"] = json!(uuid::Uuid::now_v7().to_string());
    sqlx::query("UPDATE nomi_agent_presets SET ui_binding_json = ? WHERE preset_id = ?")
        .bind(historical.to_string())
        .bind(preset_id)
        .execute(services.database.pool())
        .await
        .unwrap();
    nomifun_db::validate_id_data_contract(services.database.pool())
        .await
        .unwrap();
    historical["selection"]["plugin_id"] = json!("not-a-product-id");
    sqlx::query("UPDATE nomi_agent_presets SET ui_binding_json = ? WHERE preset_id = ?")
        .bind(historical.to_string())
        .bind(preset_id)
        .execute(services.database.pool())
        .await
        .unwrap();
    assert!(
        nomifun_db::validate_id_data_contract(services.database.pool())
            .await
            .is_err()
    );
    sqlx::query("UPDATE nomi_agent_presets SET ui_binding_json = ? WHERE preset_id = ?")
        .bind(persisted["binding"].to_string())
        .bind(preset_id)
        .execute(services.database.pool())
        .await
        .unwrap();
    for invalid in [
        json!({}),
        json!({"binding_version": 2}),
        json!({"selection": null}),
        json!({"binding_version": -1, "selection": null}),
    ] {
        assert!(
            sqlx::query("UPDATE nomi_agent_presets SET ui_binding_json = ? WHERE preset_id = ?")
                .bind(invalid.to_string())
                .bind(preset_id)
                .execute(services.database.pool())
                .await
                .is_err()
        );
    }
    // Snapshot/reopen verifies disk persistence and the current lineage/restore
    // contract without opening or resetting any developer's dataset.
    let path = root.path().join("snapshot.sqlite");
    services
        .database
        .snapshot_into(&path)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "snapshot {} (exists={}): {error}",
                path.display(),
                path.exists()
            )
        });
    let reopened = nomifun_db::init_database(&path).await.unwrap();
    let json: String =
        sqlx::query_scalar("SELECT ui_binding_json FROM nomi_agent_presets WHERE preset_id = ?")
            .bind(preset_id)
            .fetch_one(reopened.pool())
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&json).unwrap(),
        persisted["binding"]
    );
    reopened.close().await;

    let (status, cleared) = request(
        &router,
        "PUT",
        &binding_path,
        json!({
            "expected_binding_version": 2, "selection": null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(
        cleared["data"]["binding"],
        json!({"binding_version": 3, "selection": null})
    );
    assert_eq!(get(&router, &editor_path).await, editor_before);
    assert_eq!(get(&router, &session_path).await, session_before);
    assert!(upstream.received_requests().await.unwrap().is_empty());
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}
