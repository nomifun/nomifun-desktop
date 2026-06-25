use std::path::Path;

use tracing::warn;

use crate::asset_paths::resolve_extension_asset_url;
use crate::error::ExtensionError;
use crate::types::{ExtTheme, ResolvedTheme};

/// Resolve a single theme contribution by reading CSS file content.
///
/// The CSS file path is relative to the extension directory.
/// Cover image path is resolved to an absolute path.
pub fn resolve_theme(theme: &ExtTheme, extension_name: &str, ext_dir: &Path) -> Result<ResolvedTheme, ExtensionError> {
    let css_path = ext_dir.join(&theme.css_file);
    if !css_path.exists() {
        return Err(ExtensionError::ThemeCssNotFound(css_path.display().to_string()));
    }

    let css_content = std::fs::read_to_string(&css_path)?;

    let cover_image = theme
        .cover_image
        .as_deref()
        .and_then(|img| resolve_extension_asset_url(extension_name, img));

    Ok(ResolvedTheme {
        extension_name: extension_name.to_owned(),
        id: theme.id.clone(),
        name: theme.name.clone(),
        description: theme.description.clone(),
        css_content,
        cover_image,
    })
}

/// Resolve all theme contributions from an extension.
pub fn resolve_themes(themes: &[ExtTheme], extension_name: &str, ext_dir: &Path) -> Vec<ResolvedTheme> {
    themes
        .iter()
        .filter_map(|t| {
            resolve_theme(t, extension_name, ext_dir)
                .map_err(|e| {
                    warn!(
                        extension = extension_name,
                        theme_id = t.id,
                        "Failed to resolve theme: {e}"
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

    #[test]
    fn test_resolve_theme_reads_css() {
        let dir = std::env::temp_dir().join("ext_test_resolve_theme");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("dark.css"), ":root { --bg: #000; }").unwrap();

        let theme = ExtTheme {
            id: "dark-theme".into(),
            name: "Dark Theme".into(),
            description: Some("A dark theme".into()),
            css_file: "dark.css".into(),
            cover_image: Some("images/dark.png".into()),
        };

        let result = resolve_theme(&theme, "my-ext", &dir).unwrap();

        assert_eq!(result.extension_name, "my-ext");
        assert_eq!(result.id, "dark-theme");
        assert_eq!(result.css_content, ":root { --bg: #000; }");
        assert!(result.cover_image.as_ref().unwrap().contains("images/dark.png"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_resolve_theme_css_not_found() {
        let theme = ExtTheme {
            id: "missing-theme".into(),
            name: "Missing".into(),
            description: None,
            css_file: "nonexistent.css".into(),
            cover_image: None,
        };

        let err = resolve_theme(&theme, "my-ext", Path::new("/tmp/no_such_ext")).unwrap_err();
        assert!(matches!(err, ExtensionError::ThemeCssNotFound(_)));
    }

    #[test]
    fn test_resolve_theme_no_cover_image() {
        let dir = std::env::temp_dir().join("ext_test_resolve_theme_no_cover");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("light.css"), "body { color: #333; }").unwrap();

        let theme = ExtTheme {
            id: "light-theme".into(),
            name: "Light".into(),
            description: None,
            css_file: "light.css".into(),
            cover_image: None,
        };

        let result = resolve_theme(&theme, "my-ext", &dir).unwrap();
        assert!(result.cover_image.is_none());
        assert_eq!(result.css_content, "body { color: #333; }");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_resolve_themes_skips_missing_css() {
        let dir = std::env::temp_dir().join("ext_test_resolve_themes_skip");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ok.css"), "ok").unwrap();

        let themes = vec![
            ExtTheme {
                id: "good".into(),
                name: "Good".into(),
                description: None,
                css_file: "ok.css".into(),
                cover_image: None,
            },
            ExtTheme {
                id: "bad".into(),
                name: "Bad".into(),
                description: None,
                css_file: "missing.css".into(),
                cover_image: None,
            },
        ];

        let result = resolve_themes(&themes, "my-ext", &dir);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "good");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
