use std::path::Path;

use tracing::warn;

use crate::asset_paths::resolve_extension_asset_url;
use crate::error::ExtensionError;
use crate::template::resolve_file_reference;
use crate::types::{ExtPreset, ResolvedPreset};

/// Resolve a single preset contribution.
///
/// Long-text fields (`system_prompt`, `context`) support `@file:` references
/// that are replaced with the referenced file's content.
pub fn resolve_preset(
    preset: &ExtPreset,
    extension_name: &str,
    ext_dir: &Path,
) -> Result<ResolvedPreset, ExtensionError> {
    let system_prompt = preset
        .system_prompt
        .as_deref()
        .map(|v| resolve_file_reference(v, ext_dir))
        .transpose()?;

    let context = preset
        .context
        .as_deref()
        .map(|v| resolve_file_reference(v, ext_dir))
        .transpose()?;

    let icon = preset
        .icon
        .as_deref()
        .and_then(|value| resolve_extension_asset_url(extension_name, value));

    Ok(ResolvedPreset {
        extension_name: extension_name.to_owned(),
        id: preset.id.clone(),
        name: preset.name.clone(),
        description: preset.description.clone(),
        system_prompt,
        icon,
        context,
        preferred_agent_id: preset.preferred_agent_id.clone(),
        enabled_skills: preset.enabled_skills.clone(),
        prompts: preset.prompts.clone(),
        models: preset.models.clone(),
    })
}

/// Resolve all preset contributions from an extension.
pub fn resolve_presets(presets: &[ExtPreset], extension_name: &str, ext_dir: &Path) -> Vec<ResolvedPreset> {
    presets
        .iter()
        .filter_map(|a| {
            resolve_preset(a, extension_name, ext_dir)
                .map_err(|e| {
                    warn!(
                        extension = extension_name,
                        preset_id = a.id,
                        "Failed to resolve preset: {e}"
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
    fn test_resolve_preset_plain_text() {
        let preset = ExtPreset {
            id: "asst-1".into(),
            name: "Helper".into(),
            description: Some("A helpful preset".into()),
            system_prompt: Some("You are helpful.".into()),
            icon: None,
            context: None,
            preferred_agent_id: None,
            enabled_skills: vec![],
            prompts: vec![],
            models: vec![],
        };

        let result = resolve_preset(&preset, "my-ext", Path::new("/ext/my-ext")).unwrap();

        assert_eq!(result.extension_name, "my-ext");
        assert_eq!(result.id, "asst-1");
        assert_eq!(result.system_prompt.as_deref(), Some("You are helpful."));
    }

    #[test]
    fn test_resolve_preset_file_reference() {
        let dir = std::env::temp_dir().join("ext_test_resolve_preset");
        let prompts = dir.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("system.md"), "Loaded from file").unwrap();

        let preset = ExtPreset {
            id: "asst-2".into(),
            name: "File Ref".into(),
            description: None,
            system_prompt: Some("@file:prompts/system.md".into()),
            icon: None,
            context: None,
            preferred_agent_id: None,
            enabled_skills: vec![],
            prompts: vec![],
            models: vec![],
        };

        let result = resolve_preset(&preset, "my-ext", &dir).unwrap();
        assert_eq!(result.system_prompt.as_deref(), Some("Loaded from file"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_resolve_preset_file_not_found_error() {
        let preset = ExtPreset {
            id: "asst-3".into(),
            name: "Bad Ref".into(),
            description: None,
            system_prompt: Some("@file:missing.md".into()),
            icon: None,
            context: None,
            preferred_agent_id: None,
            enabled_skills: vec![],
            prompts: vec![],
            models: vec![],
        };

        let err = resolve_preset(&preset, "my-ext", Path::new("/tmp/no_such_ext_dir")).unwrap_err();
        assert!(matches!(err, ExtensionError::FileReferenceNotFound(_)));
    }

    #[test]
    fn test_resolve_preset_context_file_reference() {
        let dir = std::env::temp_dir().join("ext_test_resolve_preset_ctx");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("context.md"), "Context content").unwrap();

        let preset = ExtPreset {
            id: "asst-4".into(),
            name: "Ctx Ref".into(),
            description: None,
            system_prompt: None,
            icon: None,
            context: Some("@file:context.md".into()),
            preferred_agent_id: None,
            enabled_skills: vec![],
            prompts: vec![],
            models: vec![],
        };

        let result = resolve_preset(&preset, "my-ext", &dir).unwrap();
        assert_eq!(result.context.as_deref(), Some("Context content"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_resolve_presets_skips_bad_refs() {
        let presets = vec![
            ExtPreset {
                id: "good".into(),
                name: "Good".into(),
                description: None,
                system_prompt: Some("plain text".into()),
                icon: None,
                context: None,
                preferred_agent_id: None,
                enabled_skills: vec![],
                prompts: vec![],
                models: vec![],
            },
            ExtPreset {
                id: "bad".into(),
                name: "Bad".into(),
                description: None,
                system_prompt: Some("@file:missing.md".into()),
                icon: None,
                context: None,
                preferred_agent_id: None,
                enabled_skills: vec![],
                prompts: vec![],
                models: vec![],
            },
        ];

        let result = resolve_presets(&presets, "my-ext", Path::new("/tmp/no_such_ext"));
        // Only the good one should be resolved
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "good");
    }
}
