//! Black-box integration tests for the Telegram plugin.
//!
//! Tests the TelegramPlugin through the public ChannelPlugin trait interface
//! and ChannelManager integration.
//!
//! Covers test-plan items: TP-2, TP-5, EP-5, DP-2.
//!
//! NOTE: Tests that require a live Telegram API (TP-1, EP-1) are not included
//! here — they would need a real bot token. The unit tests within the crate
//! cover pure function logic (content extraction, callback parsing, message
//! truncation, backoff, markup building, etc.).

#[cfg(feature = "telegram")]
#[path = "support/manager.rs"]
mod manager_support;

#[cfg(feature = "telegram")]
mod telegram_tests {
    use super::manager_support::manager_fixture;

    use nomifun_channel::manager::{ChannelManager, EnableChannelSpec, PluginFactory};
    use nomifun_channel::plugin::ChannelPlugin;
    use nomifun_channel::plugins::telegram::TelegramPlugin;
    use nomifun_channel::types::{PluginConfig, PluginCredentials, PluginStatus, PluginType};

    // -- Test infrastructure ------------------------------------------------

    async fn setup() -> (ChannelManager, nomifun_db::Database) {
        manager_fixture("018f1234-5678-7abc-8def-012345678970").await
    }

    fn telegram_factory() -> PluginFactory {
        Box::new(|pt| {
            if pt == PluginType::Telegram {
                Some(Box::new(TelegramPlugin::new()))
            } else {
                None
            }
        })
    }

    fn make_plugin_config(token: Option<&str>) -> PluginConfig {
        PluginConfig {
            credentials: PluginCredentials {
                token: token.map(String::from),
                ..Default::default()
            },
            config: None,
        }
    }

    fn make_config_value(token: Option<&str>) -> serde_json::Value {
        let mut creds = serde_json::Map::new();
        if let Some(t) = token {
            creds.insert("token".into(), serde_json::Value::String(t.into()));
        }
        serde_json::json!({
            "credentials": creds,
            "config": { "mode": "polling" }
        })
    }

    fn platform_spec(plugin_type: &str) -> EnableChannelSpec {
        EnableChannelSpec {
            plugin_id: None,
            plugin_type: Some(plugin_type.to_owned()),
            companion_id: None,
            owner_domain: None,
        }
    }

    // -- Plugin construction ------------------------------------------------

    #[test]
    fn telegram_plugin_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TelegramPlugin>();
    }

    #[test]
    fn telegram_plugin_as_trait_object() {
        let plugin = TelegramPlugin::new();
        let boxed: Box<dyn ChannelPlugin> = Box::new(plugin);
        assert_eq!(boxed.plugin_type(), PluginType::Telegram);
        assert_eq!(boxed.status(), PluginStatus::Created);
    }

    // -- Factory registration -----------------------------------------------

    #[test]
    fn factory_creates_telegram_plugin() {
        let factory = telegram_factory();
        let plugin = factory(PluginType::Telegram);
        assert!(plugin.is_some());
        let plugin = plugin.unwrap();
        assert_eq!(plugin.plugin_type(), PluginType::Telegram);
        assert_eq!(plugin.status(), PluginStatus::Created);
    }

    #[test]
    fn factory_returns_none_for_other_types() {
        let factory = telegram_factory();
        assert!(factory(PluginType::Lark).is_none());
        assert!(factory(PluginType::Dingtalk).is_none());
        assert!(factory(PluginType::Weixin).is_none());
    }

    // -- TP-2: Test invalid token -------------------------------------------

    #[tokio::test]
    async fn test_plugin_invalid_token_fails() {
        let (manager, _db) = setup().await;
        let factory = telegram_factory();

        // Invalid token → getMe will fail with HTTP or API error
        let config = make_plugin_config(Some("invalid-token-12345"));
        let result = manager.test_plugin("telegram", config, &factory).await;
        assert!(result.is_err());
    }

    // -- TP-5: Missing token ------------------------------------------------

    #[tokio::test]
    async fn test_plugin_missing_token_fails() {
        let (manager, _db) = setup().await;
        let factory = telegram_factory();

        let config = make_plugin_config(None);
        let result = manager.test_plugin("telegram", config, &factory).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.to_lowercase().contains("token"),
            "Error should mention token: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_plugin_empty_token_fails() {
        let (manager, _db) = setup().await;
        let factory = telegram_factory();

        let config = make_plugin_config(Some(""));
        let result = manager.test_plugin("telegram", config, &factory).await;
        assert!(result.is_err());
    }

    // -- EP-5: Invalid plugin type ------------------------------------------

    #[tokio::test]
    async fn enable_invalid_plugin_type_fails() {
        let (manager, _db) = setup().await;
        let factory = telegram_factory();

        let config = make_config_value(Some("bot:123"));
        let result = manager.enable_plugin(&platform_spec("nonexistent"), &config, &factory).await;
        assert!(result.is_err());
    }

    // -- Enable with invalid token ------------------------------------------

    #[tokio::test]
    async fn enable_plugin_invalid_token_fails() {
        let (manager, _db) = setup().await;
        let factory = telegram_factory();

        let config = make_config_value(Some("bad-token"));
        let result = manager.enable_plugin(&platform_spec("telegram"), &config, &factory).await;
        assert!(result.is_err());
    }


}
