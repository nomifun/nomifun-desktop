//! Black-box integration tests for the Lark (Feishu) plugin.
//!
//! Tests the LarkPlugin through the public ChannelPlugin trait interface
//! and ChannelManager integration.
//!
//! Covers test-plan items: TP-3 (partial — invalid creds), TP-4, EP-5.
//!
//! NOTE: Tests requiring a live Lark API (TP-1, EP-1) are not included.
//! The unit tests within the crate cover pure function logic (event parsing,
//! card building, deduplication, callback encoding/decoding, etc.).

#[cfg(feature = "lark")]
#[path = "support/manager.rs"]
mod manager_support;

#[cfg(feature = "lark")]
mod lark_tests {
    use super::manager_support::manager_fixture;

    use nomifun_channel::manager::{ChannelManager, EnableChannelSpec, PluginFactory};
    use nomifun_channel::plugin::ChannelPlugin;
    use nomifun_channel::plugins::lark::LarkPlugin;
    use nomifun_channel::types::{PluginConfig, PluginCredentials, PluginStatus, PluginType};

    // -- Test infrastructure ------------------------------------------------

    async fn setup() -> (ChannelManager, nomifun_db::Database) {
        manager_fixture("018f1234-5678-7abc-8def-012345678971").await
    }

    fn lark_factory() -> PluginFactory {
        Box::new(|pt| {
            if pt == PluginType::Lark {
                Some(Box::new(LarkPlugin::new()))
            } else {
                None
            }
        })
    }

    fn make_lark_config(app_id: Option<&str>, app_secret: Option<&str>) -> PluginConfig {
        PluginConfig {
            credentials: PluginCredentials {
                app_id: app_id.map(String::from),
                app_secret: app_secret.map(String::from),
                ..Default::default()
            },
            config: None,
        }
    }

    fn make_lark_config_value(app_id: Option<&str>, app_secret: Option<&str>) -> serde_json::Value {
        let mut creds = serde_json::Map::new();
        if let Some(id) = app_id {
            creds.insert("appId".into(), serde_json::Value::String(id.into()));
        }
        if let Some(secret) = app_secret {
            creds.insert("appSecret".into(), serde_json::Value::String(secret.into()));
        }
        serde_json::json!({
            "credentials": creds,
            "config": { "mode": "websocket" }
        })
    }

    // -- Plugin construction ------------------------------------------------

    #[test]
    fn lark_plugin_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LarkPlugin>();
    }

    #[test]
    fn lark_plugin_as_trait_object() {
        let plugin = LarkPlugin::new();
        let boxed: Box<dyn ChannelPlugin> = Box::new(plugin);
        assert_eq!(boxed.plugin_type(), PluginType::Lark);
        assert_eq!(boxed.status(), PluginStatus::Created);
    }

    // -- Factory registration -----------------------------------------------

    #[test]
    fn factory_creates_lark_plugin() {
        let factory = lark_factory();
        let plugin = factory(PluginType::Lark);
        assert!(plugin.is_some());
        let plugin = plugin.unwrap();
        assert_eq!(plugin.plugin_type(), PluginType::Lark);
        assert_eq!(plugin.status(), PluginStatus::Created);
    }

    #[test]
    fn factory_returns_none_for_other_types() {
        let factory = lark_factory();
        assert!(factory(PluginType::Telegram).is_none());
        assert!(factory(PluginType::Dingtalk).is_none());
        assert!(factory(PluginType::Weixin).is_none());
    }

    // -- TP-3: Invalid credentials (app_id + app_secret) --------------------

    #[tokio::test]
    async fn test_plugin_invalid_credentials_fails() {
        let (manager, _db) = setup().await;
        let factory = lark_factory();

        let config = make_lark_config(Some("invalid_app_id"), Some("invalid_secret"));
        let result = manager.test_plugin("lark", config, &factory).await;
        assert!(result.is_err());
    }

    // -- Missing app_id -----------------------------------------------------

    #[tokio::test]
    async fn test_plugin_missing_app_id_fails() {
        let (manager, _db) = setup().await;
        let factory = lark_factory();

        let config = make_lark_config(None, Some("secret123"));
        let result = manager.test_plugin("lark", config, &factory).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.to_lowercase().contains("app_id"),
            "Error should mention app_id: {err_msg}"
        );
    }

    // -- Missing app_secret -------------------------------------------------

    #[tokio::test]
    async fn test_plugin_missing_app_secret_fails() {
        let (manager, _db) = setup().await;
        let factory = lark_factory();

        let config = make_lark_config(Some("cli_123"), None);
        let result = manager.test_plugin("lark", config, &factory).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.to_lowercase().contains("app_secret"),
            "Error should mention app_secret: {err_msg}"
        );
    }

    // -- Empty credentials --------------------------------------------------

    #[tokio::test]
    async fn test_plugin_empty_credentials_fails() {
        let (manager, _db) = setup().await;
        let factory = lark_factory();

        let config = make_lark_config(Some(""), Some(""));
        let result = manager.test_plugin("lark", config, &factory).await;
        assert!(result.is_err());
    }

    // -- EP-5: Invalid plugin type ------------------------------------------

    #[tokio::test]
    async fn enable_invalid_plugin_type_fails() {
        let (manager, _db) = setup().await;
        let factory = lark_factory();

        let config = make_lark_config_value(Some("cli_123"), Some("secret"));
        let result = manager
            .enable_plugin(
                &EnableChannelSpec {
                    plugin_id: None,
                    plugin_type: Some("nonexistent".into()),
                    companion_id: None,
                    owner_domain: None,
                },
                &config,
                &factory,
            )
            .await;
        assert!(result.is_err());
    }

    // -- Enable with invalid credentials ------------------------------------

    #[tokio::test]
    async fn enable_plugin_invalid_credentials_fails() {
        let (manager, _db) = setup().await;
        let factory = lark_factory();

        let config = make_lark_config_value(Some("bad_id"), Some("bad_secret"));
        let result = manager
            .enable_plugin(
                &EnableChannelSpec {
                    plugin_id: None,
                    plugin_type: Some("lark".into()),
                    companion_id: None,
                    owner_domain: None,
                },
                &config,
                &factory,
            )
            .await;
        assert!(result.is_err());
    }


}
