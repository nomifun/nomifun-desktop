use std::path::Path;

use tracing::warn;

use crate::asset_paths::resolve_extension_asset_url;
use crate::error::ExtensionError;
use crate::template::resolve_env_map;
use crate::types::{ExtAcpAdapter, ResolvedAcpAdapter};

/// Resolve a single ACP adapter contribution.
///
/// Env template placeholders (`${VAR}`) in the `env` map are expanded.
/// Avatar paths are resolved relative to the extension directory.
pub fn resolve_acp_adapter(
    adapter: &ExtAcpAdapter,
    extension_name: &str,
    _ext_dir: &Path,
) -> Result<ResolvedAcpAdapter, ExtensionError> {
    let resolved_env = resolve_env_map(&adapter.env, false)?;

    let avatar = adapter
        .avatar
        .as_ref()
        .and_then(|a| resolve_extension_asset_url(extension_name, a));

    Ok(ResolvedAcpAdapter {
        extension_name: extension_name.to_owned(),
        id: adapter.id.clone(),
        name: adapter.name.clone(),
        description: adapter.description.clone(),
        cli_command: adapter.cli_command.clone(),
        default_cli_path: adapter.default_cli_path.clone(),
        acp_args: adapter.acp_args.clone(),
        env: resolved_env,
        avatar,
        auth_required: adapter.auth_required,
        supports_streaming: adapter.supports_streaming,
        connection_type: adapter.connection_type.clone(),
        endpoint: adapter.endpoint.clone(),
        models: adapter.models.clone(),
        yolo_mode: adapter.yolo_mode.clone(),
        health_check: adapter.health_check.clone(),
        api_key_fields: adapter.api_key_fields.clone(),
    })
}

/// Resolve all ACP adapter contributions from an extension.
pub fn resolve_acp_adapters(
    adapters: &[ExtAcpAdapter],
    extension_name: &str,
    ext_dir: &Path,
) -> Vec<ResolvedAcpAdapter> {
    adapters
        .iter()
        .filter_map(|a| {
            resolve_acp_adapter(a, extension_name, ext_dir)
                .map_err(|e| {
                    warn!(
                        extension = extension_name,
                        adapter_id = a.id,
                        "Failed to resolve ACP adapter: {e}"
                    );
                    e
                })
                .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_adapter(env: HashMap<String, String>) -> ExtAcpAdapter {
        ExtAcpAdapter {
            id: "test-adapter".into(),
            name: "Test Adapter".into(),
            description: Some("A test adapter".into()),
            cli_command: Some("test-cli".into()),
            default_cli_path: None,
            acp_args: vec!["--verbose".into()],
            env,
            avatar: Some("icons/avatar.png".into()),
            auth_required: Some(true),
            supports_streaming: Some(true),
            connection_type: Some("stdio".into()),
            endpoint: None,
            models: vec!["model-a".into()],
            yolo_mode: None,
            health_check: None,
            api_key_fields: vec![],
        }
    }

    #[test]
    fn test_resolve_basic_adapter() {
        let adapter = make_adapter(HashMap::new());
        let result = resolve_acp_adapter(&adapter, "my-ext", Path::new("/ext/my-ext")).unwrap();

        assert_eq!(result.extension_name, "my-ext");
        assert_eq!(result.id, "test-adapter");
        assert_eq!(result.name, "Test Adapter");
        assert_eq!(result.cli_command.as_deref(), Some("test-cli"));
        assert_eq!(result.acp_args, vec!["--verbose"]);
        assert!(result.avatar.as_ref().unwrap().contains("icons/avatar.png"));
    }

    #[test]
    fn test_resolve_adapter_env_templates() {
        unsafe { std::env::set_var("_TEST_ACP_KEY", "secret123") };
        let mut env = HashMap::new();
        env.insert("API_KEY".into(), "${_TEST_ACP_KEY}".into());
        env.insert("STATIC".into(), "fixed".into());

        let adapter = make_adapter(env);
        let result = resolve_acp_adapter(&adapter, "my-ext", Path::new("/ext/my-ext")).unwrap();

        assert_eq!(result.env["API_KEY"], "secret123");
        assert_eq!(result.env["STATIC"], "fixed");
        unsafe { std::env::remove_var("_TEST_ACP_KEY") };
    }

    #[test]
    fn test_resolve_adapter_undefined_env_lenient() {
        let mut env = HashMap::new();
        env.insert("KEY".into(), "${_NONEXISTENT_ACP_VAR}".into());

        let adapter = make_adapter(env);
        let result = resolve_acp_adapter(&adapter, "my-ext", Path::new("/ext/my-ext")).unwrap();

        assert_eq!(result.env["KEY"], "");
    }

    #[test]
    fn test_resolve_adapters_skips_failures() {
        // With an empty list, we get an empty result.
        let result = resolve_acp_adapters(&[], "my-ext", Path::new("/ext/my-ext"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_resolve_adapter_no_avatar() {
        let mut adapter = make_adapter(HashMap::new());
        adapter.avatar = None;
        let result = resolve_acp_adapter(&adapter, "my-ext", Path::new("/ext/my-ext")).unwrap();
        assert!(result.avatar.is_none());
    }
}
