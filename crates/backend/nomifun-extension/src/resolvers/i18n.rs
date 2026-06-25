use std::collections::HashMap;
use std::path::Path;

use tracing::warn;

use crate::error::ExtensionError;
use crate::types::I18nConfig;

/// Load i18n messages for a specific locale from an extension.
///
/// Looks for `{ext_dir}/{directory}/{locale}.json` where `directory` defaults to "i18n".
/// Returns a flat key-value map of message strings.
pub fn load_extension_i18n(
    i18n_config: &I18nConfig,
    locale: &str,
    extension_name: &str,
    ext_dir: &Path,
) -> Result<HashMap<String, String>, ExtensionError> {
    if !i18n_config.locales.contains(&locale.to_owned()) {
        return Ok(HashMap::new());
    }

    let i18n_dir = ext_dir.join(&i18n_config.directory);
    let file_path = i18n_dir.join(format!("{locale}.json"));

    if !file_path.exists() {
        tracing::debug!(
            extension = extension_name,
            locale = locale,
            path = %file_path.display(),
            "i18n file not found, returning empty map"
        );
        return Ok(HashMap::new());
    }

    let content = std::fs::read_to_string(&file_path)?;
    let messages: HashMap<String, String> =
        serde_json::from_str(&content).map_err(|e| ExtensionError::ResolutionFailed {
            extension_name: extension_name.to_owned(),
            reason: format!("Invalid i18n JSON for locale '{locale}': {e}"),
        })?;

    Ok(messages)
}

/// Load i18n data for a given locale across multiple extensions.
///
/// Returns `HashMap<extension_name, HashMap<key, value>>`.
pub fn resolve_i18n_for_locale(
    extensions: &[(String, Option<I18nConfig>, String)], // (name, i18n_config, ext_dir)
    locale: &str,
) -> HashMap<String, HashMap<String, String>> {
    let mut result = HashMap::new();

    for (name, i18n_config, ext_dir) in extensions {
        let Some(config) = i18n_config else {
            continue;
        };

        match load_extension_i18n(config, locale, name, Path::new(ext_dir)) {
            Ok(messages) if !messages.is_empty() => {
                result.insert(name.clone(), messages);
            }
            Ok(_) => {}
            Err(e) => {
                warn!(
                    extension = name.as_str(),
                    locale = locale,
                    "Failed to load i18n data: {e}"
                );
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_i18n_config(locales: Vec<&str>) -> I18nConfig {
        I18nConfig {
            locales: locales.into_iter().map(String::from).collect(),
            directory: "i18n".to_owned(),
        }
    }

    #[test]
    fn test_load_i18n_supported_locale() {
        let dir = std::env::temp_dir().join("ext_test_i18n_load");
        let i18n_dir = dir.join("i18n");
        std::fs::create_dir_all(&i18n_dir).unwrap();
        std::fs::write(
            i18n_dir.join("en.json"),
            r#"{"greeting": "Hello", "farewell": "Goodbye"}"#,
        )
        .unwrap();

        let config = make_i18n_config(vec!["en", "zh-CN"]);
        let result = load_extension_i18n(&config, "en", "my-ext", &dir).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result["greeting"], "Hello");
        assert_eq!(result["farewell"], "Goodbye");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_load_i18n_unsupported_locale_returns_empty() {
        let config = make_i18n_config(vec!["en"]);
        let result = load_extension_i18n(&config, "fr", "my-ext", Path::new("/tmp")).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_load_i18n_file_not_found_returns_empty() {
        let dir = std::env::temp_dir().join("ext_test_i18n_missing");
        std::fs::create_dir_all(&dir).unwrap();

        let config = make_i18n_config(vec!["en"]);
        let result = load_extension_i18n(&config, "en", "my-ext", &dir).unwrap();
        assert!(result.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_load_i18n_invalid_json_returns_error() {
        let dir = std::env::temp_dir().join("ext_test_i18n_bad_json");
        let i18n_dir = dir.join("i18n");
        std::fs::create_dir_all(&i18n_dir).unwrap();
        std::fs::write(i18n_dir.join("en.json"), "not valid json").unwrap();

        let config = make_i18n_config(vec!["en"]);
        let err = load_extension_i18n(&config, "en", "my-ext", &dir).unwrap_err();
        assert!(matches!(err, ExtensionError::ResolutionFailed { .. }));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_resolve_i18n_for_locale_multiple_extensions() {
        let dir1 = std::env::temp_dir().join("ext_test_i18n_multi_1");
        let dir2 = std::env::temp_dir().join("ext_test_i18n_multi_2");
        let i18n1 = dir1.join("i18n");
        let i18n2 = dir2.join("i18n");
        std::fs::create_dir_all(&i18n1).unwrap();
        std::fs::create_dir_all(&i18n2).unwrap();
        std::fs::write(i18n1.join("en.json"), r#"{"key1": "val1"}"#).unwrap();
        std::fs::write(i18n2.join("en.json"), r#"{"key2": "val2"}"#).unwrap();

        let extensions = vec![
            (
                "ext-a".to_owned(),
                Some(make_i18n_config(vec!["en"])),
                dir1.to_string_lossy().into_owned(),
            ),
            (
                "ext-b".to_owned(),
                Some(make_i18n_config(vec!["en"])),
                dir2.to_string_lossy().into_owned(),
            ),
            (
                "ext-c".to_owned(),
                None, // no i18n config
                "/tmp".to_owned(),
            ),
        ];

        let result = resolve_i18n_for_locale(&extensions, "en");
        assert_eq!(result.len(), 2);
        assert_eq!(result["ext-a"]["key1"], "val1");
        assert_eq!(result["ext-b"]["key2"], "val2");

        std::fs::remove_dir_all(&dir1).unwrap();
        std::fs::remove_dir_all(&dir2).unwrap();
    }
}
