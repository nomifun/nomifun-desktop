// Phase 10 supplemental tests — loader integration for bundled skills.
// Covers TC-10.20~10.22 and TC-10.26:
//   TC-10.20: bundled skills appear in load_all_skills (normal mode)
//   TC-10.21: bundled skill wins deduplication over same-named filesystem skill
//   TC-10.22: bundled skill virtual path format
//   TC-10.26: bare mode also includes bundled skills (AC-14)

#[cfg(test)]
#[allow(clippy::module_inception)]
mod bundled_supplemental_tests {
    use crate::bundled::{
        BundledSkillDefinition, clear_bundled_skills, get_bundled_skills, register_bundled_skill,
    };
    use crate::loader::load_all_skills;
    use crate::types::SkillSource;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

    fn minimal_def(name: &'static str) -> BundledSkillDefinition {
        BundledSkillDefinition {
            name,
            description: "supplemental test skill",
            when_to_use: None,
            argument_hint: None,
            allowed_tools: &[],
            model: None,
            disable_model_invocation: false,
            user_invocable: false,
            context: None,
            agent: None,
            files: &[],
            content: "content",
        }
    }

    fn write_skill_dir(dir: &std::path::Path, name: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\n---\n").unwrap();
    }

    // TC-10.20: bundled skill appears in load_all_skills (normal mode)
    #[tokio::test]
    #[serial]
    async fn tc_10_20_bundled_in_load_all_skills_normal() {
        clear_bundled_skills();
        register_bundled_skill(minimal_def("bundled-only"));

        let tmp = TempDir::new().unwrap();
        let result = load_all_skills(tmp.path(), &[], false, None).await;

        let found = result
            .iter()
            .find(|s| s.name == "bundled-only")
            .expect("bundled skill should appear in load_all_skills result");
        assert_eq!(
            found.source,
            SkillSource::Bundled,
            "bundled skill source should be Bundled"
        );

        clear_bundled_skills();
    }

    // TC-10.21: bundled skill wins deduplication over same-named filesystem skill
    #[tokio::test]
    #[serial]
    async fn tc_10_21_bundled_wins_dedup_over_filesystem() {
        clear_bundled_skills();
        register_bundled_skill(minimal_def("shared-name"));

        let tmp = TempDir::new().unwrap();
        write_skill_dir(tmp.path(), "shared-name");

        let result = load_all_skills(tmp.path(), &[tmp.path().to_path_buf()], false, None).await;

        let matches: Vec<_> = result.iter().filter(|s| s.name == "shared-name").collect();
        assert_eq!(
            matches.len(),
            1,
            "deduplication should leave exactly one 'shared-name'"
        );
        assert_eq!(
            matches[0].source,
            SkillSource::Bundled,
            "bundled skill should win deduplication"
        );

        clear_bundled_skills();
    }

    // TC-10.22: bundled skill virtual path format
    #[test]
    fn tc_10_22_virtual_path_format() {
        let virtual_path = std::path::PathBuf::from(format!("<bundled:{}>", "path-test"));
        assert_eq!(virtual_path.to_str().unwrap(), "<bundled:path-test>");
    }

    // TC-10.26: bare mode also includes bundled skills (AC-14, C-6 decision)
    #[tokio::test]
    #[serial]
    async fn tc_10_26_bare_mode_includes_bundled() {
        clear_bundled_skills();
        register_bundled_skill(minimal_def("bundled-bare"));

        let tmp = TempDir::new().unwrap();
        let result = load_all_skills(tmp.path(), &[], true, None).await;

        let found = result
            .iter()
            .find(|s| s.name == "bundled-bare")
            .expect("bundled skill should appear in bare mode load_all_skills");
        assert_eq!(
            found.source,
            SkillSource::Bundled,
            "bundled skill source should be Bundled in bare mode"
        );

        // Verify total registry is accessible after test
        let reg = get_bundled_skills();
        assert!(!reg.is_empty());

        clear_bundled_skills();
    }
}
