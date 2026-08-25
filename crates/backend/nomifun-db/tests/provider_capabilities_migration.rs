use std::collections::BTreeSet;

use sqlx::migrate::{Migrate, Migrator};
use sqlx::{Connection, Row, SqliteConnection};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn apply_through(connection: &mut SqliteConnection, max_version: i64) {
    connection.ensure_migrations_table().await.unwrap();
    let applied: BTreeSet<i64> = connection
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    for migration in MIGRATOR.iter() {
        if migration.version <= max_version && !applied.contains(&migration.version) {
            connection.apply(migration).await.unwrap();
        }
    }
}

async fn seed_provider(
    connection: &mut SqliteConnection,
    provider_id: &str,
    platform: &str,
    base_url: &str,
) {
    sqlx::query(
        "INSERT INTO providers \
            (provider_id, platform, name, base_url, api_key_encrypted, enabled, \
             bedrock_config, is_full_url, sort_order, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'cipher', 1, NULL, 0, 0, 1, 1)",
    )
    .bind(provider_id)
    .bind(platform)
    .bind(platform)
    .bind(base_url)
    .execute(connection)
    .await
    .unwrap();
}

async fn seed_model(
    connection: &mut SqliteConnection,
    provider_id: &str,
    model: &str,
    tasks: &str,
    params: &str,
) {
    sqlx::query(
        "INSERT INTO provider_models \
            (provider_id, model, enabled, sort_order, tasks, traits, protocol, \
             connection_role, params, context_limit, description, source, health, \
             health_checked_at, created_at, updated_at) \
         VALUES (?, ?, 1, 0, ?, '[\"streaming\"]', NULL, NULL, ?, 8192, \
                 'legacy', 'user', NULL, NULL, 1, 1)",
    )
    .bind(provider_id)
    .bind(model)
    .bind(tasks)
    .bind(params)
    .execute(connection)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_32_materializes_only_verified_task_capabilities() {
    const DASH: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
    const UNKNOWN: &str = "0190f5fe-7c00-7a00-8abc-012345678902";
    const PPIO: &str = "0190f5fe-7c00-7a00-8abc-012345678903";
    const GEMINI: &str = "0190f5fe-7c00-7a00-8abc-012345678905";
    const CTYUN: &str = "0190f5fe-7c00-7a00-8abc-012345678906";
    const ARK_WITHOUT_VOICE: &str = "0190f5fe-7c00-7a00-8abc-012345678907";

    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    apply_through(&mut connection, 30).await;
    seed_provider(
        &mut connection,
        DASH,
        "dashscope",
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
    )
    .await;
    seed_provider(
        &mut connection,
        UNKNOWN,
        "unknown-vendor",
        "https://unknown.test/v1",
    )
    .await;
    seed_provider(
        &mut connection,
        PPIO,
        "ppio",
        "https://api.ppinfra.com/v3/openai/",
    )
    .await;
    seed_provider(
        &mut connection,
        GEMINI,
        "gemini",
        "https://generativelanguage.googleapis.com",
    )
    .await;
    seed_provider(
        &mut connection,
        CTYUN,
        "ctyun",
        "https://wishub-x6.ctyun.cn/v1",
    )
    .await;
    seed_provider(
        &mut connection,
        ARK_WITHOUT_VOICE,
        "ark",
        "https://ark.cn-beijing.volces.com/api/v3",
    )
    .await;

    seed_model(
        &mut connection,
        DASH,
        "wanx-model",
        "[\"image_generation\"]",
        r#"{"task_overrides":{"image_generation":{"protocol":"dashscope.images","connection_role":"default"}},"protocol":"dashscope.images","connection_role":"default","connection":"legacy","connection_id":"legacy","base_url":"https://dashscope.aliyuncs.com","base_url_override":"https://dashscope.aliyuncs.com","base_url_is_full":false,"is_full_url":false,"endpoint":"/api/v1/services/aigc/text2image/image-synthesis","poll_endpoint":"/poll","status_endpoint":"/status","content_endpoint":"/content","realtime_endpoint":"/realtime","allow_cross_origin_credentials":false,"request_shape":"legacy","request_defaults":{"x":1},"request_body":{"x":1},"auth":"legacy","auth_scheme":"bearer","credentials":{"api_key":"secret"},"api_key":"must-not-leak","api_keys":["must-not-leak"],"headers":{"X":"secret"},"seed":7}"#,
    )
    .await;
    seed_model(
        &mut connection,
        DASH,
        "wanx-status-only",
        "[\"image_generation\"]",
        r#"{"status_endpoint":"/legacy-status"}"#,
    )
    .await;
    seed_model(
        &mut connection,
        UNKNOWN,
        "future-model",
        "[\"speech_synthesis\"]",
        "{}",
    )
    .await;
    sqlx::query(
        "UPDATE provider_models SET protocol = 'future.unknown' \
         WHERE provider_id = ? AND model = 'future-model'",
    )
    .bind(UNKNOWN)
    .execute(&mut connection)
    .await
    .unwrap();
    seed_model(&mut connection, PPIO, "chat-model", "[\"chat\"]", "{}").await;
    seed_model(&mut connection, GEMINI, "gemini-native", "[\"chat\"]", "{}").await;
    seed_model(
        &mut connection,
        ARK_WITHOUT_VOICE,
        "tts-without-voice-connection",
        "[\"speech_synthesis\"]",
        "{}",
    )
    .await;

    apply_through(&mut connection, 32).await;

    let migrated_revision: i64 =
        sqlx::query_scalar("SELECT config_revision FROM providers WHERE provider_id = ?")
            .bind(DASH)
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(migrated_revision, 0);

    let dash = sqlx::query(
        "SELECT protocol, connection_role, base_url_override, endpoint, \
                poll_endpoint, content_endpoint, realtime_endpoint, \
                provider_params, context_limit FROM provider_model_capabilities \
         WHERE provider_id = ? AND model = ? AND task = 'image_generation'",
    )
    .bind(DASH)
    .bind("wanx-model")
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(dash.get::<String, _>("protocol"), "dashscope.images");
    assert_eq!(dash.get::<String, _>("connection_role"), "default");
    assert_eq!(
        dash.get::<String, _>("base_url_override"),
        "https://dashscope.aliyuncs.com"
    );
    assert_eq!(
        dash.get::<String, _>("endpoint"),
        "/api/v1/services/aigc/text2image/image-synthesis"
    );
    assert_eq!(dash.get::<String, _>("poll_endpoint"), "/poll");
    assert_eq!(dash.get::<String, _>("content_endpoint"), "/content");
    assert_eq!(dash.get::<String, _>("realtime_endpoint"), "/realtime");
    assert_eq!(dash.get::<String, _>("provider_params"), r#"{"seed":7}"#);
    assert_eq!(dash.get::<i64, _>("context_limit"), 8192);

    let legacy_status_poll: String = sqlx::query_scalar(
        "SELECT poll_endpoint FROM provider_model_capabilities \
         WHERE provider_id = ? AND model = 'wanx-status-only' \
           AND task = 'image_generation'",
    )
    .bind(DASH)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(legacy_status_poll, "/legacy-status");

    let unknown_capabilities: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_model_capabilities WHERE provider_id = ?",
    )
    .bind(UNKNOWN)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        unknown_capabilities, 0,
        "unverified routes must not be guessed"
    );
    let unknown_model: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_models WHERE provider_id = ? AND model = 'future-model'",
    )
    .bind(UNKNOWN)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        unknown_model, 1,
        "unconfigured model remains visible for repair"
    );

    let dangling_role_capabilities: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_model_capabilities WHERE provider_id = ?",
    )
    .bind(ARK_WITHOUT_VOICE)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        dangling_role_capabilities, 0,
        "a verified protocol must not create an unusable capability when its named connection is absent"
    );

    let ppio_url: String =
        sqlx::query_scalar("SELECT base_url FROM providers WHERE provider_id = ?")
            .bind(PPIO)
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(ppio_url, "https://api.ppio.com/openai/v1");

    let ctyun_url: String =
        sqlx::query_scalar("SELECT base_url FROM providers WHERE provider_id = ?")
            .bind(CTYUN)
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(ctyun_url, "https://ai.ctaigw.cn/v1");

    let gemini: (String, String) = sqlx::query_as(
        "SELECT c.protocol, p.auth_scheme \
         FROM provider_model_capabilities c \
         JOIN providers p ON p.provider_id = c.provider_id \
         WHERE c.provider_id = ? AND c.model = 'gemini-native' AND c.task = 'chat'",
    )
    .bind(GEMINI)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(gemini.0, "gemini.generate_text");
    assert_eq!(gemini.1, "header_key:x-goog-api-key");

    for retired in [
        "tasks",
        "traits",
        "protocol",
        "connection_role",
        "params",
        "context_limit",
        "source",
        "health",
        "health_checked_at",
    ] {
        let columns = sqlx::query("PRAGMA table_info(provider_models)")
            .fetch_all(&mut connection)
            .await
            .unwrap();
        assert!(
            columns
                .iter()
                .all(|column| column.get::<String, _>("name") != retired),
            "provider_models.{retired} must be removed"
        );
    }

    for (table, retired) in [
        ("providers", "is_full_url"),
        ("providers", "api_key_encrypted"),
        ("provider_connections", "is_full_url"),
    ] {
        let columns = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(&mut connection)
            .await
            .unwrap();
        assert!(
            columns
                .iter()
                .all(|column| column.get::<String, _>("name") != retired),
            "{table}.{retired} must be removed"
        );
    }
}

#[tokio::test]
async fn migration_32_materializes_explicit_default_auth_schemes() {
    let fixtures = [
        (
            "0190f5fe-7c00-7a00-8abc-012345678910",
            "anthropic",
            "header_key:x-api-key",
        ),
        (
            "0190f5fe-7c00-7a00-8abc-012345678911",
            "gemini",
            "header_key:x-goog-api-key",
        ),
        ("0190f5fe-7c00-7a00-8abc-012345678912", "deepgram", "token"),
        ("0190f5fe-7c00-7a00-8abc-012345678913", "bedrock", "bedrock"),
        ("0190f5fe-7c00-7a00-8abc-012345678914", "openai", "bearer"),
    ];
    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    apply_through(&mut connection, 30).await;
    for (provider_id, platform, _) in fixtures {
        seed_provider(
            &mut connection,
            provider_id,
            platform,
            "https://api.example.test",
        )
        .await;
    }
    sqlx::query(
        "UPDATE providers SET bedrock_config = ? WHERE platform = 'bedrock'",
    )
    .bind(
        r#"{"auth_method":"accessKey","region":"us-east-1","profile":null,"access_key_id":"AKIA-OLD","secret_access_key":"old-secret","session_token":"old-session","accessKeyId":"camel-old","secretAccessKey":"camel-secret","sessionToken":"camel-session"}"#,
    )
    .execute(&mut connection)
    .await
    .unwrap();

    apply_through(&mut connection, 32).await;

    for (provider_id, _, expected) in fixtures {
        let actual: (String, String) = sqlx::query_as(
            "SELECT auth_scheme, credentials_encrypted FROM providers WHERE provider_id = ?",
        )
        .bind(provider_id)
        .fetch_one(&mut connection)
        .await
        .unwrap();
        assert_eq!(actual.0, expected);
        assert_eq!(
            actual.1, "",
            "retired credentials require explicit re-entry"
        );
    }
    let bedrock_config: String =
        sqlx::query_scalar("SELECT bedrock_config FROM providers WHERE platform = 'bedrock'")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    let bedrock_config: serde_json::Value = serde_json::from_str(&bedrock_config).unwrap();
    assert_eq!(bedrock_config["auth_method"], "accessKey");
    assert_eq!(bedrock_config["region"], "us-east-1");
    for retired_secret in [
        "access_key_id",
        "secret_access_key",
        "session_token",
        "accessKeyId",
        "secretAccessKey",
        "sessionToken",
    ] {
        assert!(bedrock_config.get(retired_secret).is_none());
    }
}

#[tokio::test]
async fn migration_36_declares_output_limits_and_removes_legacy_body_ceilings() {
    const OPENAI: &str = "0190f5fe-7c00-7a00-8abc-012345678920";
    const ANTHROPIC: &str = "0190f5fe-7c00-7a00-8abc-012345678921";
    const BEDROCK: &str = "0190f5fe-7c00-7a00-8abc-012345678922";
    const GEMINI: &str = "0190f5fe-7c00-7a00-8abc-012345678923";
    const LEGACY_PARAMS: &str = r#"{
        "max_tokens":111,
        "max_completion_tokens":222,
        "maxOutputTokens":333,
        "max_output_tokens":444,
        "max_tokens_field":"custom.limit",
        "custom.limit":555,
        "generationConfig":{"maxOutputTokens":666,"temperature":0.5},
        "temperature":0.2
    }"#;

    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    apply_through(&mut connection, 30).await;
    for (provider_id, platform) in [
        (OPENAI, "openai"),
        (ANTHROPIC, "anthropic"),
        (BEDROCK, "bedrock"),
        (GEMINI, "gemini"),
    ] {
        seed_provider(
            &mut connection,
            provider_id,
            platform,
            "https://api.example.test",
        )
        .await;
        seed_model(
            &mut connection,
            provider_id,
            "chat-model",
            "[\"chat\"]",
            LEGACY_PARAMS,
        )
        .await;
    }

    apply_through(&mut connection, 36).await;

    for (provider_id, expected_protocol, expected_limit) in [
        (OPENAI, "openai.chat_text", None),
        (ANTHROPIC, "anthropic.messages", Some(8192_i64)),
        (
            BEDROCK,
            "bedrock.anthropic_messages",
            Some(8192_i64),
        ),
        (GEMINI, "gemini.generate_text", None),
    ] {
        let (protocol, output_limit, provider_params): (String, Option<i64>, String) =
            sqlx::query_as(
                "SELECT protocol, output_limit, provider_params \
                 FROM provider_model_capabilities \
                 WHERE provider_id = ? AND model = 'chat-model' AND task = 'chat'",
            )
            .bind(provider_id)
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(protocol, expected_protocol);
        assert_eq!(output_limit, expected_limit);

        let params: serde_json::Value = serde_json::from_str(&provider_params).unwrap();
        for retired in [
            "max_tokens",
            "max_completion_tokens",
            "maxOutputTokens",
            "max_output_tokens",
            "custom.limit",
        ] {
            assert!(params.get(retired).is_none(), "legacy key {retired} survived");
        }
        assert_eq!(params["max_tokens_field"], "custom.limit");
        assert_eq!(params["generationConfig"]["temperature"], 0.5);
        assert!(params["generationConfig"].get("maxOutputTokens").is_none());
        assert_eq!(params["temperature"], 0.2);
    }

    for invalid in [0_i64, -1_i64] {
        let error = sqlx::query(
            "UPDATE provider_model_capabilities SET output_limit = ? \
             WHERE provider_id = ? AND model = 'chat-model' AND task = 'chat'",
        )
        .bind(invalid)
        .bind(OPENAI)
        .execute(&mut connection)
        .await
        .unwrap_err();
        assert!(error.to_string().contains("CHECK constraint failed"));
    }
}

#[tokio::test]
async fn migration_32_preserves_full_urls_and_only_matching_task_health() {
    const DEFAULT_FULL: &str = "0190f5fe-7c00-7a00-8abc-012345678920";
    const ARK: &str = "0190f5fe-7c00-7a00-8abc-012345678921";
    const VOICE_CONNECTION: &str = "0190f5fe-7c00-7a00-8abc-012345678922";
    const DEFAULT_ENDPOINT: &str =
        "https://full.example.test/v1/chat/completions?api-version=2026-01-01";
    const VOICE_ENDPOINT: &str = "https://voice.example.test/api/v3/tts?version=1";

    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    apply_through(&mut connection, 30).await;
    seed_provider(&mut connection, DEFAULT_FULL, "openai", DEFAULT_ENDPOINT).await;
    sqlx::query("UPDATE providers SET is_full_url = 1 WHERE provider_id = ?")
        .bind(DEFAULT_FULL)
        .execute(&mut connection)
        .await
        .unwrap();
    seed_model(
        &mut connection,
        DEFAULT_FULL,
        "healthy-chat",
        "[\"chat\"]",
        "{}",
    )
    .await;
    seed_model(
        &mut connection,
        DEFAULT_FULL,
        "mismatched-image",
        "[\"image_generation\"]",
        "{}",
    )
    .await;
    for model in ["healthy-chat", "mismatched-image"] {
        sqlx::query(
            "UPDATE provider_models SET health = ?, health_checked_at = 777 \
             WHERE provider_id = ? AND model = ?",
        )
        .bind(r#"{"task":"chat","status":"healthy","latency":17,"error":null,"last_check":666}"#)
        .bind(DEFAULT_FULL)
        .bind(model)
        .execute(&mut connection)
        .await
        .unwrap();
    }

    seed_provider(
        &mut connection,
        ARK,
        "ark",
        "https://ark.example.test/api/v3",
    )
    .await;
    seed_model(
        &mut connection,
        ARK,
        "voice-model",
        "[\"speech_synthesis\"]",
        "{}",
    )
    .await;
    sqlx::query(
        "UPDATE provider_models SET connection_role = 'voice' \
         WHERE provider_id = ? AND model = 'voice-model'",
    )
    .bind(ARK)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO provider_connections
            (connection_id, provider_id, role, label, base_url, auth_scheme,
             credentials_encrypted, is_full_url, extra, created_at, updated_at)
         VALUES (?, ?, 'voice', 'Voice', ?, ' API_KEY ', 'cipher', 1,
                 '{"region":"cn","api_key":"retired-extra-secret","headers":{"Authorization":"retired"}}', 1, 1)"#,
    )
    .bind(VOICE_CONNECTION)
    .bind(ARK)
    .bind(VOICE_ENDPOINT)
    .execute(&mut connection)
    .await
    .unwrap();

    apply_through(&mut connection, 32).await;

    let default_route: (String, String, String, Option<i64>) = sqlx::query_as(
        "SELECT p.base_url, c.endpoint, c.health, c.health_checked_at \
         FROM providers p JOIN provider_model_capabilities c \
           ON c.provider_id = p.provider_id \
         WHERE p.provider_id = ? AND c.model = 'healthy-chat' AND c.task = 'chat'",
    )
    .bind(DEFAULT_FULL)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(default_route.0, "https://full.example.test");
    assert!(!default_route.0.contains(['?', '#']));
    assert_eq!(default_route.1, DEFAULT_ENDPOINT);
    let health: serde_json::Value = serde_json::from_str(&default_route.2).unwrap();
    assert_eq!(health["status"], "healthy");
    assert_eq!(health["latency"], 17);
    assert!(health.get("task").is_none());
    assert!(health.get("last_check").is_none());
    assert_eq!(default_route.3, Some(777));

    let mismatched_health: (Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT health, health_checked_at FROM provider_model_capabilities \
         WHERE provider_id = ? AND model = 'mismatched-image' AND task = 'image_generation'",
    )
    .bind(DEFAULT_FULL)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(mismatched_health, (None, None));

    let voice_route: (String, String, String, String, String) = sqlx::query_as(
        "SELECT pc.base_url, c.endpoint, pc.auth_scheme, pc.credentials_encrypted, pc.extra \
         FROM provider_connections pc JOIN provider_model_capabilities c \
           ON c.provider_id = pc.provider_id AND c.connection_role = pc.role \
         WHERE pc.provider_id = ? AND pc.role = 'voice' AND c.model = 'voice-model'",
    )
    .bind(ARK)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(voice_route.0, "https://voice.example.test");
    assert!(!voice_route.0.contains(['?', '#']));
    assert_eq!(voice_route.1, VOICE_ENDPOINT);
    assert_eq!(voice_route.2, "header_key:x-api-key");
    assert_eq!(
        voice_route.3, "",
        "named credentials require explicit re-entry"
    );
    assert_eq!(voice_route.4, r#"{"region":"cn"}"#);
}

#[tokio::test]
async fn migration_32_canonicalizes_only_retired_chat_protocol_aliases() {
    const OPENAI: &str = "0190f5fe-7c00-7a00-8abc-012345678930";
    const GEMINI: &str = "0190f5fe-7c00-7a00-8abc-012345678931";
    const NEW_API: &str = "0190f5fe-7c00-7a00-8abc-012345678932";
    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    apply_through(&mut connection, 30).await;
    seed_provider(
        &mut connection,
        OPENAI,
        "openai",
        "https://api.openai.com/v1",
    )
    .await;
    for (model, protocol) in [
        ("old-openai", "openai"),
        ("old-anthropic", "anthropic"),
        ("canonical", "openai.chat_text"),
    ] {
        seed_model(&mut connection, OPENAI, model, "[\"chat\"]", "{}").await;
        sqlx::query("UPDATE provider_models SET protocol = ? WHERE provider_id = ? AND model = ?")
            .bind(protocol)
            .bind(OPENAI)
            .bind(model)
            .execute(&mut connection)
            .await
            .unwrap();
    }
    seed_provider(
        &mut connection,
        GEMINI,
        "gemini",
        "https://generativelanguage.googleapis.com/v1beta/openai",
    )
    .await;
    seed_model(&mut connection, GEMINI, "old-gemini", "[\"chat\"]", "{}").await;
    sqlx::query(
        "UPDATE provider_models SET protocol = 'gemini' \
         WHERE provider_id = ? AND model = 'old-gemini'",
    )
    .bind(GEMINI)
    .execute(&mut connection)
    .await
    .unwrap();
    seed_provider(
        &mut connection,
        NEW_API,
        "new-api",
        "https://gateway.example.test/v1",
    )
    .await;
    seed_model(
        &mut connection,
        NEW_API,
        "compat-gemini",
        "[\"chat\"]",
        "{}",
    )
    .await;
    sqlx::query(
        "UPDATE provider_models SET protocol = 'gemini' \
         WHERE provider_id = ? AND model = 'compat-gemini'",
    )
    .bind(NEW_API)
    .execute(&mut connection)
    .await
    .unwrap();

    apply_through(&mut connection, 32).await;

    let openai_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT model, protocol FROM provider_model_capabilities \
         WHERE provider_id = ? ORDER BY model",
    )
    .bind(OPENAI)
    .fetch_all(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        openai_rows,
        vec![
            ("canonical".into(), "openai.chat_text".into()),
            ("old-anthropic".into(), "anthropic.messages".into()),
            ("old-openai".into(), "openai.chat_text".into()),
        ]
    );
    let official_gemini: (String, String, String) = sqlx::query_as(
        "SELECT c.protocol, p.auth_scheme, p.base_url \
         FROM provider_model_capabilities c \
         JOIN providers p ON p.provider_id = c.provider_id \
         WHERE c.provider_id = ? AND c.model = 'old-gemini' AND c.task = 'chat'",
    )
    .bind(GEMINI)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(official_gemini.0, "gemini.generate_text");
    assert_eq!(official_gemini.1, "header_key:x-goog-api-key");
    assert_eq!(
        official_gemini.2,
        "https://generativelanguage.googleapis.com"
    );

    let compatible_gateway_protocol: String = sqlx::query_scalar(
        "SELECT protocol FROM provider_model_capabilities \
         WHERE provider_id = ? AND model = 'compat-gemini' AND task = 'chat'",
    )
    .bind(NEW_API)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(compatible_gateway_protocol, "openai.chat_text");
}

#[tokio::test]
async fn migration_32_filters_incompatible_legacy_protocols_per_task() {
    const PROVIDER: &str = "0190f5fe-7c00-7a00-8abc-012345678940";
    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    apply_through(&mut connection, 30).await;
    seed_provider(
        &mut connection,
        PROVIDER,
        "openai",
        "https://api.openai.com/v1",
    )
    .await;
    seed_model(
        &mut connection,
        PROVIDER,
        "legacy-multitask",
        "[\"chat\",\"image_generation\",\"speech_synthesis\"]",
        r#"{"task_overrides":{"image_generation":{"protocol":"future.custom"},"speech_synthesis":{"protocol":"openai.chat_text"}}}"#,
    )
    .await;
    sqlx::query(
        "UPDATE provider_models SET protocol = 'openai.chat_text' \
         WHERE provider_id = ? AND model = 'legacy-multitask'",
    )
    .bind(PROVIDER)
    .execute(&mut connection)
    .await
    .unwrap();

    apply_through(&mut connection, 32).await;

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT task, protocol FROM provider_model_capabilities \
         WHERE provider_id = ? AND model = 'legacy-multitask' ORDER BY task",
    )
    .bind(PROVIDER)
    .fetch_all(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("chat".into(), "openai.chat_text".into()),
            ("image_generation".into(), "openai.images".into()),
            ("speech_synthesis".into(), "openai.audio_speech".into()),
        ]
    );
}

#[tokio::test]
async fn migration_51_adds_ark_image_edit_without_inventing_health_or_touching_other_protocols() {
    const ARK: &str = "0190f5fe-7c00-7a00-8abc-012345678951";
    const OPENAI: &str = "0190f5fe-7c00-7a00-8abc-012345678952";
    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    apply_through(&mut connection, 30).await;
    seed_provider(
        &mut connection,
        ARK,
        "ark",
        "https://ark.cn-beijing.volces.com/api/v3",
    )
    .await;
    seed_provider(
        &mut connection,
        OPENAI,
        "openai",
        "https://api.openai.com/v1",
    )
    .await;
    seed_model(
        &mut connection,
        ARK,
        "ep-opaque-seedream",
        "[\"image_generation\"]",
        "{}",
    )
    .await;
    seed_model(
        &mut connection,
        ARK,
        "already-paired",
        "[\"image_generation\"]",
        "{}",
    )
    .await;
    seed_model(
        &mut connection,
        OPENAI,
        "generation-only-by-choice",
        "[\"image_generation\"]",
        "{}",
    )
    .await;

    apply_through(&mut connection, 50).await;
    sqlx::query(
        "UPDATE provider_model_capabilities \
            SET base_url_override = 'https://ark-override.example/api/v3', \
                endpoint = '/custom/images/generations', \
                allow_cross_origin_credentials = 1, \
                provider_params = '{\"watermark\":false}', \
                health = '{\"status\":\"healthy\"}', health_checked_at = 777 \
          WHERE provider_id = ? AND model = 'ep-opaque-seedream' \
            AND task = 'image_generation'",
    )
    .bind(ARK)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_model_capabilities \
            (provider_id, model, task, traits, protocol, connection_role, \
             provider_params, created_at, updated_at) \
         VALUES (?, 'already-paired', 'image_edit', '[]', 'ark.images', \
                 'default', '{\"preserve\":true}', 9, 9)",
    )
    .bind(ARK)
    .execute(&mut connection)
    .await
    .unwrap();

    apply_through(&mut connection, 51).await;

    let ark_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT model, task FROM provider_model_capabilities \
         WHERE provider_id = ? ORDER BY model, task",
    )
    .bind(ARK)
    .fetch_all(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        ark_rows,
        vec![
            ("already-paired".into(), "image_edit".into()),
            ("already-paired".into(), "image_generation".into()),
            ("ep-opaque-seedream".into(), "image_edit".into()),
            ("ep-opaque-seedream".into(), "image_generation".into()),
        ]
    );

    let edit: (
        String,
        Option<String>,
        Option<String>,
        bool,
        String,
        Option<String>,
        Option<i64>,
    ) = sqlx::query_as(
        "SELECT protocol, base_url_override, endpoint, \
                allow_cross_origin_credentials, provider_params, health, health_checked_at \
           FROM provider_model_capabilities \
          WHERE provider_id = ? AND model = 'ep-opaque-seedream' \
            AND task = 'image_edit'",
    )
    .bind(ARK)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(edit.0, "ark.images");
    assert_eq!(
        edit.1.as_deref(),
        Some("https://ark-override.example/api/v3")
    );
    assert_eq!(edit.2.as_deref(), Some("/custom/images/generations"));
    assert!(edit.3);
    assert_eq!(edit.4, r#"{"watermark":false}"#);
    assert_eq!((edit.5, edit.6), (None, None));

    let preserved: String = sqlx::query_scalar(
        "SELECT provider_params FROM provider_model_capabilities \
         WHERE provider_id = ? AND model = 'already-paired' AND task = 'image_edit'",
    )
    .bind(ARK)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved, r#"{"preserve":true}"#);
    let ark_revision: i64 =
        sqlx::query_scalar("SELECT config_revision FROM providers WHERE provider_id = ?")
            .bind(ARK)
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(ark_revision, 1);

    let openai_tasks: Vec<String> = sqlx::query_scalar(
        "SELECT task FROM provider_model_capabilities WHERE provider_id = ? ORDER BY task",
    )
    .bind(OPENAI)
    .fetch_all(&mut connection)
    .await
    .unwrap();
    assert_eq!(openai_tasks, vec!["image_generation"]);
}

#[tokio::test]
async fn migration_32_reclassifies_stepfun_realtime_without_name_wildcards() {
    const STEP: &str = "0190f5fe-7c00-7a00-8abc-012345678904";
    const ZHIPU: &str = "0190f5fe-7c00-7a00-8abc-012345678905";
    let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    apply_through(&mut connection, 30).await;
    seed_provider(
        &mut connection,
        STEP,
        "stepfun",
        "https://api.stepfun.com/v1",
    )
    .await;
    for (model, source) in [
        ("stepaudio-2.5-realtime", "inferred"),
        ("my-realtime-preview", "inferred"),
    ] {
        sqlx::query(
            "INSERT INTO provider_models \
                (provider_id, model, enabled, sort_order, tasks, traits, params, source, \
                 created_at, updated_at) \
             VALUES (?, ?, 1, 0, '[\"chat\"]', '[]', '{}', ?, 1, 1)",
        )
        .bind(STEP)
        .bind(model)
        .bind(source)
        .execute(&mut connection)
        .await
        .unwrap();
    }
    seed_provider(
        &mut connection,
        ZHIPU,
        "zhipu",
        "https://open.bigmodel.cn/api/paas/v4",
    )
    .await;
    sqlx::query(
        "INSERT INTO provider_models \
            (provider_id, model, enabled, sort_order, tasks, traits, params, source, \
             created_at, updated_at) \
         VALUES (?, 'glm-realtime', 1, 0, '[\"chat\"]', '[]', '{}', 'inferred', 1, 1)",
    )
    .bind(ZHIPU)
    .execute(&mut connection)
    .await
    .unwrap();
    apply_through(&mut connection, 32).await;
    let tasks: Vec<(String, String)> = sqlx::query_as(
        "SELECT model, task FROM provider_model_capabilities WHERE provider_id = ? ORDER BY model",
    )
    .bind(STEP)
    .fetch_all(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        tasks,
        vec![
            ("my-realtime-preview".into(), "chat".into()),
            (
                "stepaudio-2.5-realtime".into(),
                "realtime_conversation".into()
            ),
        ]
    );
    let zhipu_capabilities: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_model_capabilities WHERE provider_id = ?",
    )
    .bind(ZHIPU)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        zhipu_capabilities, 0,
        "glm-realtime taxonomy must not invent an unimplemented protocol"
    );
    let zhipu_model: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_models \
         WHERE provider_id = ? AND model = 'glm-realtime'",
    )
    .bind(ZHIPU)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        zhipu_model, 1,
        "the unconfigured model remains user-editable"
    );
}
