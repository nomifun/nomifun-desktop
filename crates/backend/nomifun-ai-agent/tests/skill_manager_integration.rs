//! Integration tests for the skill system.
//!
//! These tests verify the production skill lifecycle:
//! - Snapshot-driven discovery by name (`discover_by_names`)
//! - Skill index generation
//! - First message preparation

// Pre-existing: ENV_MUTEX MutexGuard held across await points is intentional —
// it serializes env-var mutation across tests.
#![allow(clippy::await_holding_lock)]

use std::fs;
use std::sync::{Arc, Mutex};

use nomifun_ai_agent::{AcpSkillManager, build_skills_index_text, prepare_first_message_with_skills_index};
use nomifun_extension::{BUILTIN_SKILLS_ENV_VAR, resolve_skill_paths};
use tempfile::TempDir;
/// Serialize env var mutations across tests — `BUILTIN_SKILLS_ENV_VAR` is
/// process-global so concurrent tests that set it must not interleave.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Snapshot-driven discovery (production path via first_message_injector)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discover_by_names_selects_only_requested_skills() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    let builtin_src = tmp.path().join("builtin-skills-src");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();

    // auto-inject skill: builtin-src/auto-inject/cron/SKILL.md
    let auto_dir = builtin_src.join("auto-inject").join("cron");
    fs::create_dir_all(&auto_dir).unwrap();
    fs::write(
        auto_dir.join("SKILL.md"),
        "---\nname: cron\ndescription: Cron helper\n---\nBody",
    )
    .unwrap();

    // user custom: data/skills/my-skill/SKILL.md
    let user_dir = data_dir.join("skills").join("my-skill");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(
        user_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: User skill\n---\nBody",
    )
    .unwrap();

    unsafe {
        std::env::set_var(BUILTIN_SKILLS_ENV_VAR, &builtin_src);
    }

    let paths = Arc::new(resolve_skill_paths(tmp.path(), &data_dir));
    let mgr = AcpSkillManager::new(paths);

    let idx = mgr.discover_by_names(&["my-skill".to_owned()]).await;
    let names: std::collections::HashSet<&str> = idx.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains("my-skill"), "requested skill missing: got {names:?}");
    assert!(!names.contains("cron"), "unrequested skill leaked into the index");

    unsafe {
        std::env::remove_var(BUILTIN_SKILLS_ENV_VAR);
    }
}

#[tokio::test]
async fn discover_by_names_empty_returns_empty_index() {
    let _guard = ENV_MUTEX.lock().unwrap();
    unsafe {
        std::env::remove_var(BUILTIN_SKILLS_ENV_VAR);
    }
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();

    let paths = Arc::new(resolve_skill_paths(tmp.path(), &data_dir));
    let mgr = AcpSkillManager::new(paths);
    assert!(mgr.discover_by_names(&[]).await.is_empty());
}

// ---------------------------------------------------------------------------
// Skill Index (pure function)
// ---------------------------------------------------------------------------

#[test]
fn build_index_text_lists_skills_without_load_protocol() {
    let skills = vec![
        nomifun_ai_agent::SkillIndex {
            name: "security".into(),
            description: "Security review".into(),
        },
        nomifun_ai_agent::SkillIndex {
            name: "tdd".into(),
            description: "Test-driven development".into(),
        },
    ];
    let text = build_skills_index_text(&skills);

    assert!(text.contains("- **security**: Security review"));
    assert!(text.contains("- **tdd**: Test-driven development"));
    // No code fulfills [LOAD_SKILL: ...] requests, so the index must not
    // instruct the model to emit that dead protocol marker.
    assert!(!text.contains("LOAD_SKILL"));
}

// ---------------------------------------------------------------------------
// First message builder
// ---------------------------------------------------------------------------

#[test]
fn first_message_with_skills_index_for_acp() {
    let skills = vec![nomifun_ai_agent::SkillIndex {
        name: "review".into(),
        description: "Code review".into(),
    }];
    let result = prepare_first_message_with_skills_index("Please review my code.", &skills, None);

    assert!(result.contains("[Assistant Rules]"));
    assert!(result.contains("- **review**: Code review"));
    assert!(result.contains("[/Assistant Rules]"));
    assert!(result.ends_with("Please review my code."));
}
