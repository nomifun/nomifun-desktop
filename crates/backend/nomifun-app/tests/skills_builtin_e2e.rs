//! HTTP integration tests for the built-in skills migration surface:
//! `/api/skills/builtin-auto`, `/api/skills/builtin-skill`, `/api/skills`,
//!
//! Covers the spec's §9.2 scenarios end-to-end through
//! `nomifun_app::compatibility::create_router_with_states` against an in-memory DB.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use nomifun_app::compatibility::{
    ModuleStates, build_module_states, create_router_with_states,
};
use nomifun_db::init_database_memory;
use nomifun_skill_library::{ExternalPathsManager, SkillPaths, SkillRouterState};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

use common::{body_json, get_with_token, json_with_token, setup_and_login};

// ---------------------------------------------------------------------------
// Fixture — build router with embedded-corpus paths rooted at a temp dir
// ---------------------------------------------------------------------------

struct Fixture {
    app: axum::Router,
    token: String,
    csrf: String,
    data_dir: std::path::PathBuf,
    _tmp: TempDir,
}

/// Build an app whose skill state points at a freshly materialized
/// builtin-skills tree rooted at a temp `data_dir`. `write_skill` can
/// still seed user skills under `{data_dir}/skills/`.
async fn fixture_embedded() -> Fixture {
    // Ensure no env override interferes.
    // SAFETY: tests in this file may mutate this env var across async
    // tasks on the same process. Rust 2024 marks `remove_var` as unsafe
    // for exactly that reason. The var is only read at router-state
    // construction time, and each test calls `fixture_embedded` once at
    // the top, so the mutation is race-free in practice.
    unsafe {
        std::env::remove_var("NOMIFUN_BUILTIN_SKILLS_PATH");
    }

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().to_path_buf();

    // Materialize the embedded corpus onto the temp data dir so the
    // per-test router can read it just like production would.
    nomifun_skill_library::materialize_if_needed(
        &data_dir,
        nomifun_skill_library::builtin_skills_corpus(),
        "test-fixture",
    )
    .await
    .expect("failed to materialize embedded builtin skills for test fixture");

    let db = init_database_memory().await.unwrap();
    let services = nomifun_app::compatibility::AppServices::from_config(
        db,
        &nomifun_app::AppConfig {
            data_dir: data_dir.join("app-data"),
            work_dir: data_dir.join("app-work"),
            ..nomifun_app::AppConfig::default()
        },
    )
    .await
    .unwrap();
    let (mut states, _): (ModuleStates, _) = build_module_states(&services).await;

    // Replace the skill state with a deterministic one rooted at tmp.
    // `build_module_states` builds a state pointing at `~/.nomifun/`,
    // which is fine for production but unsuitable here.
    let skill_paths = SkillPaths {
        data_dir: data_dir.clone(),
        user_skills_dir: data_dir.join("skills"),
        cron_skills_dir: data_dir.join("cron").join("skills"),
        builtin_skills_dir: data_dir.join("builtin-skills"),
        builtin_rules_dir: data_dir.join("builtin-rules"),
    };
    let ext_paths_mgr = Arc::new(ExternalPathsManager::with_file(data_dir.join("paths.json")).await);
    states.skill = SkillRouterState {
        skill_paths,
        external_paths_manager: ext_paths_mgr,
        skill_tag_repo: std::sync::Arc::new(nomifun_db::SqliteSkillTagRepository::new(
            services.database.pool().clone(),
        )),
        builtin_skill_tags: std::sync::Arc::new(std::collections::HashMap::new()),
    };

    let mut app = create_router_with_states(&services, states);
    // Built-in skills are host-control resources, so exercise these routes as
    // the canonical installation owner rather than a secondary account.
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    Fixture {
        app,
        token,
        csrf,
        data_dir,
        _tmp: tmp,
    }
}

fn write_user_skill(dir: &std::path::Path, name: &str, desc: &str) {
    let skill_dir = dir.join("skills").join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {desc}\n---\nBody for {name}."),
    )
    .unwrap();
}

// ===========================================================================
// GET /api/skills/builtin-auto — embedded corpus
// ===========================================================================

#[tokio::test]
async fn builtin_auto_lists_entries_from_embedded_corpus() {
    let fx = fixture_embedded().await;

    let resp = fx
        .app
        .clone()
        .oneshot(get_with_token("/api/skills/builtin-auto", &fx.token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    let arr = json["data"].as_array().unwrap();
    assert!(arr.len() >= 3, "expected ≥3 auto-inject entries, got {}", arr.len());
    for item in arr {
        assert!(item["name"].is_string());
        assert_ne!(
            item["name"], "officecli",
            "officecli must remain an opt-in builtin skill"
        );
        assert!(item["description"].is_string());
        let loc = item["location"].as_str().unwrap();
        assert!(loc.starts_with("auto-inject/"), "location={loc}");
        assert!(loc.ends_with("/SKILL.md"));
    }
}

// ===========================================================================
// POST /api/skills/builtin-skill
// ===========================================================================

#[tokio::test]
async fn builtin_skill_read_auto_inject_returns_frontmatter_content() {
    let fx = fixture_embedded().await;

    let resp = fx
        .app
        .clone()
        .oneshot(json_with_token(
            "POST",
            "/api/skills/builtin-skill",
            json!({"file_name": "auto-inject/cron/SKILL.md"}),
            &fx.token,
            &fx.csrf,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    let content = json["data"].as_str().unwrap();
    assert!(content.trim_start().starts_with("---"), "content={content}");
}

#[tokio::test]
async fn builtin_skill_read_opt_in_returns_frontmatter_content() {
    let fx = fixture_embedded().await;

    // planning-with-files is a stable opt-in skill in the corpus.
    let resp = fx
        .app
        .clone()
        .oneshot(json_with_token(
            "POST",
            "/api/skills/builtin-skill",
            json!({"file_name": "planning-with-files/SKILL.md"}),
            &fx.token,
            &fx.csrf,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let content = json["data"].as_str().unwrap();
    assert!(
        !content.is_empty(),
        "planning-with-files SKILL.md is empty"
    );
}

#[tokio::test]
async fn builtin_skill_missing_file_returns_empty_string() {
    let fx = fixture_embedded().await;

    let resp = fx
        .app
        .clone()
        .oneshot(json_with_token(
            "POST",
            "/api/skills/builtin-skill",
            json!({"file_name": "unknown/SKILL.md"}),
            &fx.token,
            &fx.csrf,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"], "");
}

#[tokio::test]
async fn builtin_skill_rejects_traversal() {
    let fx = fixture_embedded().await;

    for bad in ["../etc/passwd", "/etc/passwd", "auto-inject/../../escape", ""] {
        let resp = fx
            .app
            .clone()
            .oneshot(json_with_token(
                "POST",
                "/api/skills/builtin-skill",
                json!({"file_name": bad}),
                &fx.token,
                &fx.csrf,
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "file_name={bad:?} should be rejected",
        );
    }
}

// ===========================================================================
// GET /api/skills — merged list with relative_location for builtin
// ===========================================================================

#[tokio::test]
async fn list_skills_builtin_entries_carry_relative_location() {
    let fx = fixture_embedded().await;

    // Seed one user skill so the merge is non-trivial.
    write_user_skill(&fx.data_dir, "my-custom", "Custom skill for test");

    let resp = fx
        .app
        .clone()
        .oneshot(get_with_token("/api/skills", &fx.token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let arr = json["data"].as_array().unwrap();

    let mut saw_builtin = false;
    let mut saw_custom = false;
    for item in arr {
        match item["source"].as_str().unwrap() {
            "builtin" => {
                saw_builtin = true;
                let rel = item["relative_location"].as_str().unwrap();
                assert!(rel.ends_with("/SKILL.md"));
                let loc = item["location"].as_str().unwrap();
                assert!(
                    loc.contains("builtin-skills"),
                    "builtin location should live under builtin-skills dir: {loc}"
                );
                // The builtin-skills tree is materialized at startup, so
                // SKILL.md must already exist on disk.
                assert!(
                    std::path::Path::new(loc).exists(),
                    "builtin skill file missing on disk: {loc}"
                );
            }
            "custom" => {
                saw_custom = true;
                assert!(item.get("relative_location").is_none());
                assert!(item.get("relative_location").is_none());
                assert_eq!(item["name"], "my-custom");
            }
            other => panic!("unexpected source: {other}"),
        }
    }
    assert!(saw_builtin, "expected at least one builtin entry");
    assert!(saw_custom, "expected the seeded custom entry");

    let officecli = arr
        .iter()
        .find(|item| item["name"] == "officecli")
        .expect("officecli builtin skill listed");
    assert_eq!(officecli["relative_location"], "officecli/SKILL.md");
}

#[tokio::test]
async fn list_skills_builtin_entries_include_display_i18n_metadata() {
    let fx = fixture_embedded().await;

    let resp = fx
        .app
        .clone()
        .oneshot(get_with_token("/api/skills", &fx.token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let arr = json["data"].as_array().unwrap();
    let planning = arr
        .iter()
        .find(|item| item["name"] == "planning-with-files")
        .expect("planning-with-files builtin skill listed");

    assert_eq!(planning["source"], "builtin");
    let zh_desc = planning["description_i18n"]["zh-CN"]
        .as_str()
        .expect("planning-with-files should expose zh-CN display description");
    assert!(
        zh_desc.contains("计划"),
        "unexpected zh-CN display description: {zh_desc}"
    );
    assert!(
        planning.get("name_i18n").is_some(),
        "builtin skills should expose display name metadata, even when it preserves the canonical name"
    );
}
