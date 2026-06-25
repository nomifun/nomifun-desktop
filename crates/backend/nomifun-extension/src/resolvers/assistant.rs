use std::path::Path;

use tracing::warn;

use crate::asset_paths::resolve_extension_asset_url;
use crate::error::ExtensionError;
use crate::template::resolve_file_reference;
use crate::types::{ExtAssistant, ResolvedAssistant};

/// Resolve a single assistant contribution.
///
/// Long-text fields (`system_prompt`, `context`) support `@file:` references
/// that are replaced with the referenced file's content.
pub fn resolve_assistant(
    assistant: &ExtAssistant,
    extension_name: &str,
    ext_dir: &Path,
) -> Result<ResolvedAssistant, ExtensionError> {
    let system_prompt = assistant
        .system_prompt
        .as_deref()
        .map(|v| resolve_file_reference(v, ext_dir))
        .transpose()?;

    let context = assistant
        .context
        .as_deref()
        .map(|v| resolve_file_reference(v, ext_dir))
        .transpose()?;

    let icon = assistant
        .icon
        .as_deref()
        .and_then(|value| resolve_extension_asset_url(extension_name, value));

    Ok(ResolvedAssistant {
        extension_name: extension_name.to_owned(),
        id: assistant.id.clone(),
        name: assistant.name.clone(),
        description: assistant.description.clone(),
        system_prompt,
        icon,
        context,
        preset_agent_type: assistant.preset_agent_type.clone(),
        enabled_skills: assistant.enabled_skills.clone(),
        prompts: assistant.prompts.clone(),
        models: assistant.models.clone(),
    })
}

/// Resolve all assistant contributions from an extension.
pub fn resolve_assistants(assistants: &[ExtAssistant], extension_name: &str, ext_dir: &Path) -> Vec<ResolvedAssistant> {
    assistants
        .iter()
        .filter_map(|a| {
            resolve_assistant(a, extension_name, ext_dir)
                .map_err(|e| {
                    warn!(
                        extension = extension_name,
                        assistant_id = a.id,
                        "Failed to resolve assistant: {e}"
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
    fn test_resolve_assistant_plain_text() {
        let assistant = ExtAssistant {
            id: "asst-1".into(),
            name: "Helper".into(),
            description: Some("A helpful assistant".into()),
            system_prompt: Some("You are helpful.".into()),
            icon: None,
            context: None,
            preset_agent_type: None,
            enabled_skills: vec![],
            prompts: vec![],
            models: vec![],
        };

        let result = resolve_assistant(&assistant, "my-ext", Path::new("/ext/my-ext")).unwrap();

        assert_eq!(result.extension_name, "my-ext");
        assert_eq!(result.id, "asst-1");
        assert_eq!(result.system_prompt.as_deref(), Some("You are helpful."));
    }

    #[test]
    fn test_resolve_assistant_file_reference() {
        let dir = std::env::temp_dir().join("ext_test_resolve_assistant");
        let prompts = dir.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("system.md"), "Loaded from file").unwrap();

        let assistant = ExtAssistant {
            id: "asst-2".into(),
            name: "File Ref".into(),
            description: None,
            system_prompt: Some("@file:prompts/system.md".into()),
            icon: None,
            context: None,
            preset_agent_type: None,
            enabled_skills: vec![],
            prompts: vec![],
            models: vec![],
        };

        let result = resolve_assistant(&assistant, "my-ext", &dir).unwrap();
        assert_eq!(result.system_prompt.as_deref(), Some("Loaded from file"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_resolve_assistant_file_not_found_error() {
        let assistant = ExtAssistant {
            id: "asst-3".into(),
            name: "Bad Ref".into(),
            description: None,
            system_prompt: Some("@file:missing.md".into()),
            icon: None,
            context: None,
            preset_agent_type: None,
            enabled_skills: vec![],
            prompts: vec![],
            models: vec![],
        };

        let err = resolve_assistant(&assistant, "my-ext", Path::new("/tmp/no_such_ext_dir")).unwrap_err();
        assert!(matches!(err, ExtensionError::FileReferenceNotFound(_)));
    }

    #[test]
    fn test_resolve_assistant_context_file_reference() {
        let dir = std::env::temp_dir().join("ext_test_resolve_assistant_ctx");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("context.md"), "Context content").unwrap();

        let assistant = ExtAssistant {
            id: "asst-4".into(),
            name: "Ctx Ref".into(),
            description: None,
            system_prompt: None,
            icon: None,
            context: Some("@file:context.md".into()),
            preset_agent_type: None,
            enabled_skills: vec![],
            prompts: vec![],
            models: vec![],
        };

        let result = resolve_assistant(&assistant, "my-ext", &dir).unwrap();
        assert_eq!(result.context.as_deref(), Some("Context content"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_resolve_assistants_skips_bad_refs() {
        let assistants = vec![
            ExtAssistant {
                id: "good".into(),
                name: "Good".into(),
                description: None,
                system_prompt: Some("plain text".into()),
                icon: None,
                context: None,
                preset_agent_type: None,
                enabled_skills: vec![],
                prompts: vec![],
                models: vec![],
            },
            ExtAssistant {
                id: "bad".into(),
                name: "Bad".into(),
                description: None,
                system_prompt: Some("@file:missing.md".into()),
                icon: None,
                context: None,
                preset_agent_type: None,
                enabled_skills: vec![],
                prompts: vec![],
                models: vec![],
            },
        ];

        let result = resolve_assistants(&assistants, "my-ext", Path::new("/tmp/no_such_ext"));
        // Only the good one should be resolved
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "good");
    }
}
