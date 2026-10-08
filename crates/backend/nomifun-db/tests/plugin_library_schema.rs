use nomifun_agent_contracts::AGENT_STORE_BASELINE_SQL;
use nomifun_db::{init_database, validate_current_migration_lineage};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

static MIGRATOR: Migrator = sqlx::migrate!();
const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000611";
const PLUGIN: &str = "0190f5fe-7c00-7a00-8000-000000000612";
const DRAFT: &str = "0190f5fe-7c00-7a00-8000-000000000613";
const GENERATION: &str = "0190f5fe-7c00-7a00-8000-000000000614";

#[tokio::test]
async fn forward_migration_removes_versions_preserves_library_and_accepts_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("library.sqlite");
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
        SqliteConnectOptions::new().filename(&path).create_if_missing(true).foreign_keys(true)
    ).await.unwrap();
    sqlx::raw_sql(AGENT_STORE_BASELINE_SQL).execute(&pool).await.unwrap();
    sqlx::raw_sql("CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT NOT NULL, installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, success BOOLEAN NOT NULL, checksum BLOB NOT NULL, execution_time BIGINT NOT NULL)")
        .execute(&pool).await.unwrap();
    let baseline = MIGRATOR.iter().next().unwrap();
    sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES (?,?,1,?,0)")
        .bind(baseline.version).bind(baseline.description.as_ref()).bind(baseline.checksum.as_ref())
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES (?,'admin','',1,1)")
        .bind(OWNER).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO installation_identity(singleton_key,owner_user_id) VALUES ('installation',?)")
        .bind(OWNER).execute(&pool).await.unwrap();
    for digest in ["a".repeat(64), "b".repeat(64)] {
        sqlx::query("INSERT INTO plugin_artifacts(artifact_digest,package_id,version,manifest_json,files_json,artifact_root,has_ui,has_service,data_version,created_at_ms) VALUES (?,'test.library','1.0.0','{}','[]','test/artifact',1,0,0,1)")
            .bind(digest).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO plugins(plugin_id,owner_user_id,package_id,name,description,enabled,active_artifact_digest,previous_artifact_digest,data_generation,revision,config_json,created_at_ms,updated_at_ms) VALUES (?,?,'test.library','Daily notes','A saved local app',1,?,?,?,8,'{\"theme\":\"dark\"}',1,1)")
        .bind(PLUGIN).bind(OWNER).bind("a".repeat(64)).bind("b".repeat(64)).bind(GENERATION)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO plugin_drafts(draft_id,owner_user_id,plugin_id,base_revision,revision,name,workspace_path,status,created_at_ms,updated_at_ms) VALUES (?,?,?,8,3,'In progress','test/draft','ready',1,1)")
        .bind(DRAFT).bind(OWNER).bind(PLUGIN).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO plugin_library_state(owner_user_id,plugin_id,pinned,collection,revision) VALUES (?,?,1,'Work',4)")
        .bind(OWNER).bind(PLUGIN).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO plugin_credential_bindings(owner_user_id,plugin_id,slot,credential_id,updated_at_ms) VALUES (?,?,'api_key','provider:test',1)")
        .bind(OWNER).bind(PLUGIN).execute(&pool).await.unwrap();
    pool.close().await;

    for _ in 0..2 {
        let database = init_database(&path).await.unwrap();
        let pool = database.pool();
        validate_current_migration_lineage(pool).await.unwrap();
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(pool).await.unwrap(), MIGRATOR.iter().count() as i64);
        for table in ["plugins", "plugin_mutations"] {
            let columns: Vec<String> = sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .fetch_all(pool).await.unwrap();
            assert!(!columns.iter().any(|name| name.contains("previous")));
        }
        let installed: (String, String, i64) = sqlx::query_as("SELECT data_generation,config_json,revision FROM plugins WHERE plugin_id=?")
            .bind(PLUGIN).fetch_one(pool).await.unwrap();
        assert_eq!(installed, (GENERATION.into(), "{\"theme\":\"dark\"}".into(), 8));
        let draft: (String, i64, i64) = sqlx::query_as("SELECT plugin_id,base_revision,revision FROM plugin_drafts WHERE draft_id=?")
            .bind(DRAFT).fetch_one(pool).await.unwrap();
        assert_eq!(draft, (PLUGIN.into(), 8, 3), "table replacement must preserve the saved working copy");
        assert_eq!(sqlx::query_scalar::<_, String>("SELECT collection FROM plugin_library_state WHERE plugin_id=?")
            .bind(PLUGIN).fetch_one(pool).await.unwrap(), "Work");
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM plugin_credential_bindings")
            .fetch_one(pool).await.unwrap(), 1);
        assert_eq!(sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys").fetch_one(pool).await.unwrap(), 1);
        assert!(sqlx::query("PRAGMA foreign_key_check").fetch_all(pool).await.unwrap().is_empty());
        database.close().await;
    }
}
