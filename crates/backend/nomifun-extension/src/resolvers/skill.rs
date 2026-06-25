use std::path::Path;

use tracing::warn;

use crate::types::{ExtSkill, ResolvedSkill};

/// Resolve a single skill contribution.
///
/// Skill file paths are resolved relative to the extension directory.
pub fn resolve_skill(skill: &ExtSkill, extension_name: &str, ext_dir: &Path) -> Option<ResolvedSkill> {
    let path = skill.path.as_ref()?;
    let location = ext_dir.join(path);
    if !location.exists() {
        return None;
    }

    Some(ResolvedSkill {
        extension_name: extension_name.to_owned(),
        name: skill.name.clone(),
        description: skill.description.clone(),
        path: Some(location.to_string_lossy().into_owned()),
    })
}

/// Resolve all skill contributions from an extension.
pub fn resolve_skills(skills: &[ExtSkill], extension_name: &str, ext_dir: &Path) -> Vec<ResolvedSkill> {
    skills
        .iter()
        .filter_map(|s| {
            resolve_skill(s, extension_name, ext_dir).or_else(|| {
                warn!(
                    extension = extension_name,
                    skill_name = s.name,
                    "Failed to resolve skill path"
                );
                None
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_skill_with_path() {
        let dir = std::env::temp_dir().join("ext_test_resolve_skill_with_path");
        std::fs::create_dir_all(dir.join("skills")).unwrap();
        std::fs::write(dir.join("skills/code-review.md"), "# review").unwrap();

        let skill = ExtSkill {
            name: "code-review".into(),
            description: Some("Code review skill".into()),
            path: Some("skills/code-review.md".into()),
        };

        let result = resolve_skill(&skill, "my-ext", &dir).unwrap();

        assert_eq!(result.extension_name, "my-ext");
        assert_eq!(result.name, "code-review");
        assert!(result.path.as_ref().unwrap().contains("skills/code-review"));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_resolve_skill_no_path() {
        let skill = ExtSkill {
            name: "inline-skill".into(),
            description: None,
            path: None,
        };

        let result = resolve_skill(&skill, "my-ext", Path::new("/ext/my-ext"));
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_skill_missing_path() {
        let skill = ExtSkill {
            name: "missing-skill".into(),
            description: None,
            path: Some("skills/missing.md".into()),
        };

        let result = resolve_skill(&skill, "my-ext", Path::new("/ext/my-ext"));
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_skills_multiple() {
        let dir = std::env::temp_dir().join("ext_test_resolve_skills_multiple");
        std::fs::create_dir_all(dir.join("skills")).unwrap();
        std::fs::write(dir.join("skills/b.md"), "# b").unwrap();

        let skills = vec![
            ExtSkill {
                name: "a".into(),
                description: None,
                path: None,
            },
            ExtSkill {
                name: "b".into(),
                description: None,
                path: Some("skills/b.md".into()),
            },
        ];

        let result = resolve_skills(&skills, "my-ext", &dir);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "b");

        std::fs::remove_dir_all(dir).unwrap();
    }
}
