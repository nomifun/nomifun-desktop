use std::borrow::Cow;
use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::sqlite::SqlitePoolOptions;

static ALL_MIGRATIONS: Migrator = sqlx::migrate!("./migrations");

fn migrator_through(version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    }
}

#[derive(Debug, Deserialize)]
struct MigratedKnowledgePolicy {
    enabled: bool,
    mode: String,
    writeback: bool,
    grounded: bool,
}

#[derive(Debug, Deserialize)]
struct MigratedSnapshot {
    preset_id: String,
    preset_revision: i64,
    preset_name: String,
    target: String,
    instructions: String,
    #[serde(default)]
    resolved_agent_id: Option<String>,
    included_skills: Vec<String>,
    excluded_auto_skills: Vec<String>,
    knowledge_policy: MigratedKnowledgePolicy,
    knowledge_base_ids: Vec<String>,
    warnings: Vec<String>,
}

async fn seed_pre_preset_data(conn: &mut sqlx::SqliteConnection) {
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES ('system_default_user', 'migration-fixture', 'unused', 1, 1)",
    )
    .execute(&mut *conn)
    .await
    .unwrap();

    sqlx::query(
        r#"INSERT INTO assistants (
              id, name, description, avatar, preset_agent_type,
              enabled_skills, custom_skill_names, disabled_builtin_skills,
              prompts, models, name_i18n, description_i18n, prompts_i18n,
              created_at, updated_at, audience_tags, scenario_tags
           ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind("assistant_user_research")
    .bind("Research Assistant")
    .bind("Evidence-focused role")
    .bind("/legacy/research.png")
    .bind("codex")
    .bind(r#"["skill-a","shared-skill"]"#)
    .bind(r#"["skill-b","shared-skill"]"#)
    .bind(r#"["auto-risky"]"#)
    .bind(r#"["Research this","Compare sources"]"#)
    .bind(r#"["gpt-5","claude-sonnet"]"#)
    .bind(r#"{"zh-CN":"研究助手"}"#)
    .bind(r#"{"zh-CN":"重视证据","fr-FR":"Recherche fondee sur des preuves"}"#)
    .bind(r#"{"zh-CN":["研究这个主题","比较来源"],"en-US":["Research this topic"]}"#)
    .bind(1_000_i64)
    .bind(2_000_i64)
    .bind(r#"["audience-engineer"]"#)
    .bind(r#"["scenario-research"]"#)
    .execute(&mut *conn)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO assistant_overrides \
         (assistant_id, enabled, sort_order, preset_agent_type, last_used_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?), (?, ?, ?, ?, ?, ?)",
    )
    .bind("assistant_user_research")
    .bind(false)
    .bind(7_i64)
    .bind(Option::<String>::None)
    .bind(2_500_i64)
    .bind(2_600_i64)
    .bind("builtin_writer")
    .bind(false)
    .bind(11_i64)
    .bind("claude")
    .bind(2_700_i64)
    .bind(2_800_i64)
    .execute(&mut *conn)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO assistant_tags (key, dimension, label, sort_order, created_at) \
         VALUES ('audience-engineer','audience','Engineer',3,3000), \
                ('scenario-research','scenario','Research',4,3001)",
    )
    .execute(&mut *conn)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO conversations \
         (user_id,name,type,extra,status,pinned,created_at,updated_at) \
         VALUES ('system_default_user','Migrated conversation','nomi',?,'finished',0,4000,4001)",
    )
    .bind(
        r#"{"preset_assistant_id":"assistant_user_research","preset_context":"Cite primary sources.","skills":["skill-a","skill-b"]}"#,
    )
    .execute(&mut *conn)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cron_jobs (
             id,name,enabled,schedule_kind,schedule_value,payload_message,
             execution_mode,agent_config,agent_type,created_by,target_kind,
             created_at,updated_at
         ) VALUES ('cron_migrated','Daily research',1,'cron','0 9 * * *','research',
             'new_conversation',?,'nomi','user','agent',5000,5001)",
    )
    .bind(r#"{"presetAssistantId":"assistant_user_research"}"#)
    .execute(&mut *conn)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO fleets (id,user_id,name,created_at,updated_at) \
         VALUES ('fleet_migrated','system_default_user','Research fleet',6000,6001)",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO fleet_members \
         (id,fleet_id,agent_id,role_hint,sort_order,created_at,updated_at) \
         VALUES ('fleet_member_migrated','fleet_migrated','assistant_user_research',
                 'researcher',0,6002,6003)",
    )
    .execute(&mut *conn)
    .await
    .unwrap();

    let fleet_snapshot = serde_json::json!([{
        "id": "run_member_migrated",
        "agent_id": "assistant_user_research",
        "provider_id": null,
        "model": null,
        "role_hint": "researcher",
        "capability_profile": null,
        "constraints": null,
        "sort_order": 0,
        "description": "Evidence researcher",
        "system_prompt": null,
        "enabled_skills": [],
        "disabled_builtin_skills": []
    }])
    .to_string();
    sqlx::query(
        "INSERT INTO orch_runs \
         (id,workspace_id,user_id,goal,fleet_snapshot,autonomy,status,created_at,updated_at) \
         VALUES ('run_migrated',NULL,'system_default_user','Research migration',?,'balanced',
                 'completed',7000,7001)",
    )
    .bind(fleet_snapshot)
    .execute(&mut *conn)
    .await
    .unwrap();
}

fn assert_snapshot_contract(snapshot: &MigratedSnapshot, target: &str) {
    assert_eq!(snapshot.preset_id, "assistant_user_research");
    assert_eq!(snapshot.preset_revision, 1);
    assert_eq!(snapshot.preset_name, "Research Assistant");
    assert_eq!(snapshot.target, target);
    assert_eq!(snapshot.knowledge_policy.mode, "inherit");
    assert!(!snapshot.knowledge_policy.enabled);
    assert!(!snapshot.knowledge_policy.writeback);
    assert!(!snapshot.knowledge_policy.grounded);
    assert!(snapshot.knowledge_base_ids.is_empty());
    assert!(!snapshot.warnings.is_empty());
}

#[tokio::test]
async fn migration_034_upgrades_full_legacy_preset_state_without_data_loss() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON")
        .execute(&mut *conn)
        .await
        .unwrap();

    migrator_through(33).run(&mut *conn).await.unwrap();
    seed_pre_preset_data(&mut conn).await;
    migrator_through(34).run(&mut *conn).await.unwrap();

    sqlx::query("PRAGMA foreign_keys = ON; PRAGMA legacy_alter_table = OFF")
        .execute(&mut *conn)
        .await
        .unwrap();

    let preset: (String, String, String, Option<String>, i64, String, Option<String>) = sqlx::query_as(
        "SELECT source_kind,name,description,avatar,revision,routing_description,source_key \
         FROM presets WHERE id='assistant_user_research'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(preset.0, "user");
    assert_eq!(preset.1, "Research Assistant");
    assert_eq!(preset.2, "Evidence-focused role");
    assert_eq!(preset.3.as_deref(), Some("/legacy/research.png"));
    assert_eq!(preset.4, 1);
    assert_eq!(preset.5, "Evidence-focused role");
    assert_eq!(preset.6.as_deref(), Some("assistant_user_research"));

    let targets: HashSet<String> = sqlx::query_scalar(
        "SELECT target_kind FROM preset_targets WHERE preset_id='assistant_user_research'",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap()
    .into_iter()
    .collect();
    assert_eq!(targets, ["conversation", "cluster_member", "companion", "cron"].into_iter().map(str::to_owned).collect());

    let agent: String = sqlx::query_scalar(
        "SELECT agent_id FROM preset_agent_preferences WHERE preset_id='assistant_user_research'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(agent, "codex");

    let skills: Vec<(String, String)> = sqlx::query_as(
        "SELECT skill_name,binding FROM preset_skill_bindings \
         WHERE preset_id='assistant_user_research' ORDER BY binding,skill_name",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        skills,
        vec![
            ("auto-risky".into(), "exclude_auto".into()),
            ("shared-skill".into(), "include".into()),
            ("skill-a".into(), "include".into()),
            ("skill-b".into(), "include".into()),
        ]
    );

    let models: Vec<(Option<String>, String)> = sqlx::query_as(
        "SELECT provider_id,model FROM preset_model_preferences \
         WHERE preset_id='assistant_user_research' ORDER BY rank",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    assert_eq!(models, vec![(None, "gpt-5".into()), (None, "claude-sonnet".into())]);

    let examples: Vec<(String, String)> = sqlx::query_as(
        "SELECT locale,prompt FROM preset_examples \
         WHERE preset_id='assistant_user_research' ORDER BY locale,sort_order",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        examples,
        vec![
            ("".into(), "Research this".into()),
            ("".into(), "Compare sources".into()),
            ("en-US".into(), "Research this topic".into()),
            ("zh-CN".into(), "研究这个主题".into()),
            ("zh-CN".into(), "比较来源".into()),
        ]
    );

    let localizations: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT locale,name,description FROM preset_localizations \
         WHERE preset_id='assistant_user_research' ORDER BY locale",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        localizations,
        vec![
            ("fr-FR".into(), None, Some("Recherche fondee sur des preuves".into())),
            ("zh-CN".into(), Some("研究助手".into()), Some("重视证据".into())),
        ]
    );

    let tag_bindings: HashSet<(String, String)> = sqlx::query_as(
        "SELECT tag_key,dimension FROM preset_tag_bindings \
         WHERE preset_id='assistant_user_research'",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap()
    .into_iter()
    .collect();
    assert_eq!(
        tag_bindings,
        [
            ("audience-engineer".into(), "audience".into()),
            ("scenario-research".into(), "scenario".into()),
        ]
        .into_iter()
        .collect()
    );

    let states: Vec<(String, bool, bool, Option<String>, i64, Option<i64>)> = sqlx::query_as(
        "SELECT preset_id,enabled,auto_selectable,preferred_agent_id,sort_order,last_used_at \
         FROM preset_user_state ORDER BY preset_id",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        states,
        vec![
            ("assistant_user_research".into(), false, false, None, 7, Some(2_500)),
            ("builtin_writer".into(), false, false, Some("claude".into()), 11, Some(2_700)),
        ]
    );

    let conversation_lineage: (String, i64, String) = sqlx::query_as(
        "SELECT preset_id,preset_revision,preset_snapshot FROM conversations \
         WHERE name='Migrated conversation'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(conversation_lineage.0, "assistant_user_research");
    assert_eq!(conversation_lineage.1, 1);
    let conversation_snapshot: MigratedSnapshot = serde_json::from_str(&conversation_lineage.2).unwrap();
    assert_snapshot_contract(&conversation_snapshot, "conversation");
    assert_eq!(conversation_snapshot.instructions, "Cite primary sources.");
    assert_eq!(conversation_snapshot.included_skills, vec!["skill-a", "skill-b"]);
    assert!(conversation_snapshot.excluded_auto_skills.is_empty());

    let cron_lineage: (String, i64, String) = sqlx::query_as(
        "SELECT preset_id,preset_revision,preset_snapshot FROM cron_jobs WHERE id='cron_migrated'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(cron_lineage.0, "assistant_user_research");
    assert_eq!(cron_lineage.1, 1);
    let cron_snapshot: MigratedSnapshot = serde_json::from_str(&cron_lineage.2).unwrap();
    assert_snapshot_contract(&cron_snapshot, "cron");
    assert_eq!(
        cron_snapshot.included_skills,
        vec!["skill-a", "shared-skill", "skill-b"]
    );
    assert_eq!(cron_snapshot.excluded_auto_skills, vec!["auto-risky"]);

    let fleet_lineage: (String, String, i64, String) = sqlx::query_as(
        "SELECT agent_id,preset_id,preset_revision,preset_snapshot FROM fleet_members \
         WHERE id='fleet_member_migrated'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(fleet_lineage.0, "agent_builtin_codex");
    assert_eq!(fleet_lineage.1, "assistant_user_research");
    assert_eq!(fleet_lineage.2, 1);
    let fleet_snapshot: MigratedSnapshot = serde_json::from_str(&fleet_lineage.3).unwrap();
    assert_snapshot_contract(&fleet_snapshot, "cluster_member");
    assert_eq!(fleet_snapshot.resolved_agent_id.as_deref(), Some("agent_builtin_codex"));
    assert_eq!(
        fleet_snapshot.included_skills,
        vec!["skill-a", "shared-skill", "skill-b"]
    );
    assert_eq!(fleet_snapshot.excluded_auto_skills, vec!["auto-risky"]);

    let run_snapshot: String = sqlx::query_scalar(
        "SELECT fleet_snapshot FROM orch_runs WHERE id='run_migrated'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    let run_members: Vec<Value> = serde_json::from_str(&run_snapshot).unwrap();
    assert_eq!(run_members[0]["agent_id"], "agent_builtin_codex");
    assert_eq!(run_members[0]["preset_id"], "assistant_user_research");
    assert_eq!(run_members[0]["preset_revision"], 1);
    let run_member_snapshot: MigratedSnapshot =
        serde_json::from_value(run_members[0]["preset_snapshot"].clone()).unwrap();
    assert_snapshot_contract(&run_member_snapshot, "cluster_member");
    assert_eq!(
        run_member_snapshot.resolved_agent_id.as_deref(),
        Some("agent_builtin_codex")
    );
    assert_eq!(
        run_member_snapshot.included_skills,
        vec!["skill-a", "shared-skill", "skill-b"]
    );
    assert_eq!(
        run_member_snapshot.excluded_auto_skills,
        vec!["auto-risky"]
    );
    assert_eq!(
        run_members[0]["enabled_skills"],
        serde_json::json!(["skill-a", "shared-skill", "skill-b"])
    );
    assert_eq!(
        run_members[0]["disabled_builtin_skills"],
        serde_json::json!(["auto-risky"])
    );

    for legacy_table in ["assistants", "assistant_overrides", "assistant_tags"] {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
        )
        .bind(legacy_table)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
        assert_eq!(count, 0, "legacy table {legacy_table} survived migration 034");
    }

    let fk_error_rows: Vec<(String, Option<i64>, String, i64)> = sqlx::query_as(
        "SELECT \"table\", rowid, parent, fkid FROM pragma_foreign_key_check",
    )
    .fetch_all(&mut *conn)
        .await
        .unwrap();
    assert!(fk_error_rows.is_empty(), "foreign key errors: {fk_error_rows:?}");
}
