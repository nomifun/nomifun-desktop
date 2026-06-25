// Phase 10 inline tests for src/skills/bundled/mod.rs
// Covers TC-10.01 ~ TC-10.28 (registration API, field mapping, file extraction,
// resolve_skill_file_path path validation, prepare_bundled_skills, thread safety).

use super::*;
use serial_test::serial;
use std::path::Path;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn minimal_def(name: &'static str) -> BundledSkillDefinition {
    BundledSkillDefinition {
        name,
        description: "test skill",
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

// ---------------------------------------------------------------------------
// TC-10.01: register single skill
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_01_register_single_skill() {
    clear_bundled_skills();
    register_bundled_skill(minimal_def("tc-01"));
    let skills = get_bundled_skills();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "tc-01");
}

// ---------------------------------------------------------------------------
// TC-10.02: multiple registrations accumulate
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_02_register_multiple_accumulate() {
    clear_bundled_skills();
    register_bundled_skill(minimal_def("a"));
    register_bundled_skill(minimal_def("b"));
    register_bundled_skill(minimal_def("c"));
    let skills = get_bundled_skills();
    assert_eq!(skills.len(), 3);
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"a") && names.contains(&"b") && names.contains(&"c"));
}

// ---------------------------------------------------------------------------
// TC-10.03: clear_bundled_skills empties registry
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_03_clear_empties_registry() {
    clear_bundled_skills();
    register_bundled_skill(minimal_def("to-clear"));
    clear_bundled_skills();
    assert!(get_bundled_skills().is_empty());
}

// ---------------------------------------------------------------------------
// TC-10.04: init_bundled_skills registers hello skill
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_04_init_registers_hello() {
    clear_bundled_skills();
    init_bundled_skills();
    let skills = get_bundled_skills();
    assert!(!skills.is_empty());
    assert!(skills.iter().any(|s| s.name == "hello"));
}

// ---------------------------------------------------------------------------
// TC-10.05: full field mapping
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_05_full_field_mapping() {
    clear_bundled_skills();
    register_bundled_skill(BundledSkillDefinition {
        name: "full-skill",
        description: "desc",
        when_to_use: Some("when"),
        argument_hint: Some("arg"),
        allowed_tools: &["Bash", "Read"],
        model: Some("claude-opus-4-6"),
        disable_model_invocation: false,
        user_invocable: true,
        context: Some("inline"),
        agent: Some("my-agent"),
        files: &[],
        content: "body",
    });
    let skills = get_bundled_skills();
    let m = &skills[0];
    assert_eq!(m.name, "full-skill");
    assert_eq!(m.description, "desc");
    assert_eq!(m.when_to_use.as_deref(), Some("when"));
    assert_eq!(m.argument_hint.as_deref(), Some("arg"));
    assert_eq!(m.allowed_tools, vec!["Bash", "Read"]);
    assert_eq!(m.model.as_deref(), Some("claude-opus-4-6"));
    assert!(!m.disable_model_invocation);
    assert!(m.user_invocable);
    assert_eq!(m.agent.as_deref(), Some("my-agent"));
    assert!(m.has_user_specified_description);
}

// ---------------------------------------------------------------------------
// TC-10.06: source and loaded_from are Bundled
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_06_source_and_loaded_from_bundled() {
    clear_bundled_skills();
    register_bundled_skill(minimal_def("src-test"));
    let skills = get_bundled_skills();
    let m = &skills[0];
    assert_eq!(m.source, SkillSource::Bundled);
    assert_eq!(m.loaded_from, LoadedFrom::Bundled);
}

// ---------------------------------------------------------------------------
// TC-10.07: context="inline" maps to ExecutionContext::Inline
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_07_context_inline_maps_correctly() {
    clear_bundled_skills();
    register_bundled_skill(BundledSkillDefinition {
        context: Some("inline"),
        ..minimal_def("ctx-inline")
    });
    let m = &get_bundled_skills()[0];
    assert_eq!(m.execution_context, ExecutionContext::Inline);
}

// ---------------------------------------------------------------------------
// TC-10.08: context="fork" maps to ExecutionContext::Fork
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_08_context_fork_maps_correctly() {
    clear_bundled_skills();
    register_bundled_skill(BundledSkillDefinition {
        context: Some("fork"),
        ..minimal_def("ctx-fork")
    });
    let m = &get_bundled_skills()[0];
    assert_eq!(m.execution_context, ExecutionContext::Fork);
}

// ---------------------------------------------------------------------------
// TC-10.09: context=None defaults to ExecutionContext::Inline
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_09_context_none_defaults_to_inline() {
    clear_bundled_skills();
    register_bundled_skill(minimal_def("ctx-none"));
    let m = &get_bundled_skills()[0];
    assert_eq!(
        m.execution_context,
        ExecutionContext::Inline,
        "context=None should default to Inline"
    );
}

// ---------------------------------------------------------------------------
// TC-10.10: no files → skill_root is None
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_10_no_files_skill_root_none() {
    clear_bundled_skills();
    register_bundled_skill(minimal_def("no-files"));
    let m = &get_bundled_skills()[0];
    assert!(m.skill_root.is_none());
}

// ---------------------------------------------------------------------------
// TC-10.11: with files → prepare_bundled_skills sets skill_root
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn tc_10_11_files_skill_root_set_by_prepare() {
    clear_bundled_skills();
    register_bundled_skill(BundledSkillDefinition {
        files: &[("guide.md", "# Guide")],
        ..minimal_def("file-skill")
    });
    let skills = prepare_bundled_skills().await;
    let m = skills.iter().find(|s| s.name == "file-skill").unwrap();
    assert!(
        m.skill_root.is_some(),
        "skill_root should be set by prepare_bundled_skills"
    );
    assert!(m.skill_root.as_ref().unwrap().contains("file-skill"));
}

// ---------------------------------------------------------------------------
// TC-10.12: extraction — directory and file created
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn tc_10_12_extract_creates_dir_and_file() {
    let result = extract_bundled_skill_files("tc-12-skill", &[("data.md", "content")]).await;
    let dir = result.expect("extraction should succeed");
    let file = dir.join("data.md");
    assert!(file.exists(), "extracted file should exist");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "content",
        "file content should match"
    );
    // cleanup
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// TC-10.13: directory permission 0o700 (unix only)
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn tc_10_13_dir_permission_0700() {
    let result = extract_bundled_skill_files("tc-13-skill", &[("perm.md", "x")]).await;
    let dir = result.expect("extraction should succeed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&dir).unwrap();
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o700,
            "directory must be owner-only (0o700)"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// TC-10.14: file permission 0o600 (unix only)
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn tc_10_14_file_permission_0600() {
    let result = extract_bundled_skill_files("tc-14-skill", &[("file.md", "y")]).await;
    let dir = result.expect("extraction should succeed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let fmeta = std::fs::metadata(dir.join("file.md")).unwrap();
        assert_eq!(
            fmeta.permissions().mode() & 0o777,
            0o600,
            "file must be owner-only (0o600)"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// TC-10.15: path traversal rejected at integration layer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_10_15_path_traversal_rejected_integration() {
    let result = extract_bundled_skill_files("tc-15-evil", &[("../escape.txt", "pwned")]).await;
    // Either extraction fails entirely, or the traversal entry is skipped
    if let Some(dir) = result {
        assert!(
            !dir.parent().unwrap().join("escape.txt").exists(),
            "traversal file must not be created outside extract dir"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    // If result is None, the test also passes (extraction was rejected)
}

// ---------------------------------------------------------------------------
// TC-10.16: extraction failure returns None, not panic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_10_16_extraction_failure_returns_none() {
    // Pass an empty files slice — extract_bundled_skill_files returns None for empty
    let result = extract_bundled_skill_files("tc-16-empty", &[]).await;
    assert!(
        result.is_none(),
        "empty files should return None without panic"
    );
}

// ---------------------------------------------------------------------------
// TC-10.17: get_bundled_skill_extract_dir path format
// ---------------------------------------------------------------------------

#[test]
fn tc_10_17_extract_dir_path_format() {
    let path = get_bundled_skill_extract_dir("my-skill");
    let s = path.to_string_lossy();
    assert!(
        s.contains("nomi-bundled-skills"),
        "path should contain nomi-bundled-skills"
    );
    assert!(s.contains("my-skill"), "path should contain skill name");
}

// ---------------------------------------------------------------------------
// TC-10.19: get_bundled_skills is idempotent (does not consume registry)
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_19_get_bundled_skills_idempotent() {
    clear_bundled_skills();
    register_bundled_skill(minimal_def("idem-a"));
    register_bundled_skill(minimal_def("idem-b"));
    assert_eq!(get_bundled_skills().len(), 2);
    assert_eq!(get_bundled_skills().len(), 2);
}

// ---------------------------------------------------------------------------
// TC-10.23: content_length field is correct
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_23_content_length_correct() {
    clear_bundled_skills();
    register_bundled_skill(BundledSkillDefinition {
        content: "hello world",
        ..minimal_def("cl-skill")
    });
    let m = &get_bundled_skills()[0];
    assert_eq!(m.content_length, "hello world".len());
}

// ---------------------------------------------------------------------------
// TC-10.24: concurrent registration does not panic
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_24_concurrent_registration_no_panic() {
    clear_bundled_skills();
    let handles: Vec<_> = (0..10_u8)
        .map(|i| {
            std::thread::spawn(move || {
                // SAFETY: each thread registers a unique name literal via a
                // fixed array; we pick from the set of 10 pre-defined literals.
                let names: [&'static str; 10] =
                    ["t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8", "t9"];
                register_bundled_skill(minimal_def(names[i as usize]));
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread should not panic");
    }
    let skills = get_bundled_skills();
    assert_eq!(
        skills.len(),
        10,
        "all 10 concurrent registrations should be present"
    );
}

// ---------------------------------------------------------------------------
// TC-10.25: unknown context string defaults to Inline
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_25_unknown_context_defaults_to_inline() {
    clear_bundled_skills();
    register_bundled_skill(BundledSkillDefinition {
        context: Some("unknown-value"),
        ..minimal_def("ctx-unknown")
    });
    let m = &get_bundled_skills()[0];
    assert_eq!(m.execution_context, ExecutionContext::Inline);
}

// ---------------------------------------------------------------------------
// TC-10.27: resolve_skill_file_path path validation (private fn, inline test)
// ---------------------------------------------------------------------------

#[test]
fn tc_10_27a_resolve_normal_path_ok() {
    let result = resolve_skill_file_path(Path::new("/base"), "sub/file.md");
    assert!(result.is_ok(), "normal relative path should be Ok");
    assert_eq!(
        result.unwrap(),
        std::path::PathBuf::from("/base/sub/file.md")
    );
}

#[test]
fn tc_10_27b_resolve_traversal_rejected() {
    let result = resolve_skill_file_path(Path::new("/base"), "../escape.txt");
    assert!(result.is_err(), "path traversal '../' must be rejected");
}

#[test]
fn tc_10_27c_resolve_absolute_path_rejected() {
    // Use a platform-appropriate absolute path so `Path::is_absolute()` returns true
    #[cfg(unix)]
    let abs_path = "/etc/passwd";
    #[cfg(windows)]
    let abs_path = "C:\\Windows\\System32\\drivers\\etc\\hosts";

    let result = resolve_skill_file_path(Path::new("/base"), abs_path);
    assert!(result.is_err(), "absolute path must be rejected");
}

#[test]
fn tc_10_27d_resolve_disguised_traversal_rejected() {
    let result = resolve_skill_file_path(Path::new("/base"), "sub/../escape");
    assert!(
        result.is_err(),
        "disguised traversal 'sub/../escape' must be rejected"
    );
}

// ---------------------------------------------------------------------------
// TC-10.28: init_bundled_skills is idempotent
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn tc_10_28_init_idempotent() {
    clear_bundled_skills();
    init_bundled_skills();
    init_bundled_skills();
    let skills = get_bundled_skills();
    let hello_count = skills.iter().filter(|s| s.name == "hello").count();
    assert_eq!(
        hello_count, 1,
        "init_bundled_skills must be idempotent — hello should appear exactly once"
    );
}
