use std::path::Path;

use crate::asset_paths::resolve_extension_asset_url;
use crate::types::{ExtChannelPlugin, ResolvedChannelPlugin};

/// Resolve a single channel plugin contribution.
///
/// Entry point paths are resolved relative to the extension directory.
/// Note: actual plugin code execution must go through the sandbox (not direct eval).
pub fn resolve_channel_plugin(
    plugin: &ExtChannelPlugin,
    extension_name: &str,
    ext_dir: &Path,
) -> ResolvedChannelPlugin {
    let entry_point = plugin
        .entry_point
        .as_ref()
        .map(|ep| ext_dir.join(ep).to_string_lossy().into_owned());
    let icon = plugin
        .icon
        .as_deref()
        .and_then(|value| resolve_extension_asset_url(extension_name, value));

    ResolvedChannelPlugin {
        extension_name: extension_name.to_owned(),
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        description: plugin.description.clone(),
        platform: plugin.platform.clone(),
        entry_point,
        icon,
        credential_fields: plugin.credential_fields.clone(),
        config_fields: plugin.config_fields.clone(),
    }
}

/// Resolve all channel plugin contributions from an extension.
pub fn resolve_channel_plugins(
    plugins: &[ExtChannelPlugin],
    extension_name: &str,
    ext_dir: &Path,
) -> Vec<ResolvedChannelPlugin> {
    plugins
        .iter()
        .map(|p| resolve_channel_plugin(p, extension_name, ext_dir))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_channel_plugin_with_entry() {
        let plugin = ExtChannelPlugin {
            id: "slack-plugin".into(),
            name: "Slack".into(),
            description: Some("Slack integration".into()),
            platform: Some("slack".into()),
            entry_point: Some("plugins/slack.js".into()),
            icon: None,
            credential_fields: vec![],
            config_fields: vec![],
        };

        let result = resolve_channel_plugin(&plugin, "my-ext", Path::new("/ext/my-ext"));

        assert_eq!(result.extension_name, "my-ext");
        assert_eq!(result.id, "slack-plugin");
        assert_eq!(result.platform.as_deref(), Some("slack"));
        assert!(result.entry_point.as_ref().unwrap().contains("plugins/slack.js"));
    }

    #[test]
    fn test_resolve_channel_plugin_no_entry() {
        let plugin = ExtChannelPlugin {
            id: "simple".into(),
            name: "Simple".into(),
            description: None,
            platform: None,
            entry_point: None,
            icon: None,
            credential_fields: vec![],
            config_fields: vec![],
        };

        let result = resolve_channel_plugin(&plugin, "my-ext", Path::new("/ext/my-ext"));
        assert!(result.entry_point.is_none());
    }

    #[test]
    fn test_resolve_channel_plugins_empty() {
        let result = resolve_channel_plugins(&[], "my-ext", Path::new("/ext/my-ext"));
        assert!(result.is_empty());
    }
}
