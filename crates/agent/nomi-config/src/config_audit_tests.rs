use super::*;

#[test]
fn long_profile_chains_resolve_without_recursive_stack_growth() {
    let profiles = (0..4096)
        .map(|index| {
            (
                index.to_string(),
                ProfileConfig {
                    extends: (index < 4095).then(|| (index + 1).to_string()),
                    model: (index == 4095).then(|| "base-model".into()),
                    max_turns: (index == 0).then_some(2),
                    ..Default::default()
                },
            )
        })
        .collect();
    let resolved = resolve_profile(&profiles, "0").unwrap();
    assert_eq!(resolved.model.as_deref(), Some("base-model"));
    assert_eq!(resolved.max_turns, Some(2));
    assert!(resolved.extends.is_none());
}

#[test]
fn config_initialization_never_overwrites_existing_content() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "# user config").unwrap();
    init_config_at(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# user config");
}

#[test]
fn concurrent_config_initialization_publishes_one_complete_template() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                barrier.wait();
                init_config_at(&path).unwrap();
                assert_eq!(
                    std::fs::read_to_string(&path).unwrap(),
                    DEFAULT_CONFIG_TEMPLATE
                );
            });
        }
    });
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn provider_extra_body_remains_a_whole_value_override() {
    let merged = merge_sources(
        "[providers.openai.compat]\napi_path = \"/custom\"\nextra_body = {keep = true, nested = {old = 1}}",
        "[providers.openai.compat]\nsupports_image = false\nextra_body = {nested = {new = 2}}",
    );
    let compat = merged.providers["openai"].compat.as_ref().unwrap();
    assert_eq!(compat.api_path.as_deref(), Some("/custom"));
    assert_eq!(compat.supports_image, Some(false));
    assert_eq!(
        compat.extra_body(),
        serde_json::json!({"nested": {"new": 2}})
            .as_object()
            .unwrap()
            .clone()
    );
}

#[test]
fn logging_uses_the_same_explicit_field_merge() {
    for (global, project, enabled, level, dir) in [
        (
            "enabled = false\nlevel = \"warn\"\ndir = \"global\"",
            "enabled = true\nlevel = \"debug\"",
            Some(true),
            Some("debug"),
            Some("global"),
        ),
        ("level = \"info\"", "", None, Some("info"), None),
        ("", "", None, None, None),
    ] {
        let merged = merge_sources(
            &format!("[logging]\n{global}"),
            &format!("[logging]\n{project}"),
        );
        assert_eq!(merged.logging.enabled, enabled);
        assert_eq!(merged.logging.level.as_deref(), level);
        assert_eq!(merged.logging.dir.as_deref(), dir);
    }
}

pub(super) fn merge_sources(global: &str, project: &str) -> ConfigFile {
    merge_config_files(
        toml::from_str(global).unwrap(),
        toml::from_str(project).unwrap(),
    )
    .unwrap()
}

#[test]
fn explicit_default_values_override_global_scalars() {
    let merged = merge_sources(
        "[default]\nprovider = \"openai\"\n[tools]\nmax_recent_images = 8\n[tools.browser]\nsource = \"system\"",
        "[default]\nprovider = \"anthropic\"\n[tools]\nmax_recent_images = 3\n[tools.browser]\nsource = \"managed\"",
    );
    assert_eq!(merged.default.provider, "anthropic");
    assert_eq!(merged.tools.max_recent_images, 3);
    assert_eq!(merged.tools.browser.source, "managed");
}

#[test]
fn compact_partial_overrides_keep_other_global_fields() {
    let merged = merge_sources(
        "[compact]\ncontext_window = 64000\noutput_reserve = 4000",
        "[compact]\ncompaction = \"off\"\ntoon = true\nmicro_keep_recent = 2",
    );
    assert_eq!(
        merged.compact.compaction,
        nomi_compact::CompactionLevel::Off
    );
    assert!(merged.compact.toon);
    assert_eq!(merged.compact.micro_keep_recent, 2);
    assert_eq!(merged.compact.context_window, 64000);
    assert_eq!(merged.compact.output_reserve, 4000);
}

#[test]
fn partial_section_overrides_do_not_reset_omitted_fields() {
    let merged = merge_sources(
        "[session]\nenabled = false\nmax_sessions = 70\n[plan]\nenabled = false\n[file_cache]\nenabled = false\nmax_size_bytes = 1234",
        "[session]\ndirectory = \"sessions\"\n[plan]\nplan_directory = \"plans\"\n[file_cache]\nmax_entries = 50",
    );
    assert!(!merged.session.enabled);
    assert_eq!(merged.session.max_sessions, 70);
    assert_eq!(merged.session.directory, "sessions");
    assert!(!merged.plan.enabled);
    assert_eq!(merged.plan.plan_directory, "plans");
    assert!(!merged.file_cache.enabled);
    assert_eq!(merged.file_cache.max_size_bytes, 1234);
    assert_eq!(merged.file_cache.max_entries, 50);
}

#[test]
fn explicit_true_can_restore_default_feature_settings() {
    let merged = merge_sources(
        "[compact]\nenabled = false\n[plan]\nenabled = false\n[file_cache]\nenabled = false",
        "[compact]\nenabled = true\n[plan]\nenabled = true\n[file_cache]\nenabled = true",
    );
    assert!(merged.compact.enabled);
    assert!(merged.plan.enabled);
    assert!(merged.file_cache.enabled);
}

#[test]
fn merge_retains_documented_additive_tool_policies() {
    let merged = merge_sources(
        "[tools]\nbash_sandbox = true\nwrite_root = \"guarded\"\nbuiltin_allowlist = [\"Read\"]\n[tools.browser]\nfull_power = true\n[session]\nenabled = false",
        "[tools]\nbash_sandbox = false\nwrite_root = \"\"\nbuiltin_allowlist = []\n[tools.browser]\nfull_power = false\n[session]\nenabled = true",
    );
    assert!(merged.tools.bash_sandbox);
    assert!(merged.tools.browser.full_power);
    assert_eq!(merged.tools.write_root, "guarded");
    assert_eq!(merged.tools.builtin_allowlist, ["Read"]);
    assert!(!merged.session.enabled);
}

#[test]
fn invalid_existing_config_is_not_silently_replaced_by_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let invalid = "[tools]\nbash_sandbox = \"typo\"\n";
    std::fs::write(&path, invalid).unwrap();
    assert!(load_config_file(&path).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), invalid);
}

#[test]
fn unreadable_config_is_an_error_but_missing_config_is_optional() {
    let temp = tempfile::tempdir().unwrap();
    assert!(
        load_config_file(temp.path()).is_err(),
        "a directory cannot be a config file"
    );
    assert!(load_config_file(&temp.path().join("absent.toml")).is_ok());
}

#[test]
fn profile_model_overrides_the_providers_default_model() {
    let config: ConfigFile = toml::from_str(
        r#"
[providers.anthropic]
model = "provider-default"
[profiles.review]
model = "profile-model"
"#,
    )
    .unwrap();
    let config = apply_profile(config, "review").unwrap();
    let resolved = resolve_provider_alias(&config.providers, &config.default.provider).unwrap();
    assert_eq!(
        resolved.effective_config.model.as_deref(),
        Some("profile-model")
    );
}

#[test]
fn child_profile_keeps_unmodified_parent_compat_fields() {
    let config: ConfigFile = toml::from_str(
        r#"
[profiles.parent.compat]
api_path = "/custom/messages"
supports_image = true
[profiles.child]
extends = "parent"
[profiles.child.compat]
supports_image = false
"#,
    )
    .unwrap();
    let profile = resolve_profile(&config.profiles, "child").unwrap();
    let compat = profile.compat.unwrap();
    assert_eq!(compat.api_path.as_deref(), Some("/custom/messages"));
    assert_eq!(compat.supports_image, Some(false));
}
