#[allow(dead_code)]
#[path = "../src/data_root.rs"]
mod data_root;

use std::collections::BTreeMap;
use std::time::Duration;

use data_root::{
    DataGeneration, PluginDataRootError, PluginDataRootManager, PluginMigrationRecord,
    PluginSqlStatement, PluginSqlValue,
};
use nomifun_agent_contracts::{DigestHex, PluginId};
use serde_json::json;

fn plugin(label: &str) -> PluginId {
    PluginId::from(format!("0199aa00-0000-7000-8000-{label:0>12}"))
}

fn generation(label: &str) -> DataGeneration {
    DataGeneration::new(label).unwrap()
}

fn new_manager(temp: &tempfile::TempDir) -> PluginDataRootManager {
    PluginDataRootManager::new(temp.path().join("plugin-data")).unwrap()
}

#[test]
fn generations_are_isolated_and_kv_persists_with_revisioned_cas() {
    let temp = tempfile::tempdir().unwrap();
    let manager = new_manager(&temp);
    let first_id = plugin("1");
    let second_id = plugin("2");
    let first = manager
        .create_empty_generation(first_id.clone(), generation("generation-1"))
        .unwrap();
    let second = manager
        .create_empty_generation(second_id.clone(), generation("generation-1"))
        .unwrap();

    assert_eq!(
        first.path(),
        std::fs::canonicalize(
            temp.path()
                .join("plugin-data")
                .join(first_id.as_ref())
                .join("generations")
                .join("generation-1")
        )
        .unwrap()
    );
    assert!(first.path().join("data.sqlite").is_file());
    assert!(first.path().join("files").is_dir());

    let first_storage = first.storage();
    let second_storage = second.storage();
    assert_eq!(first_storage.kv_set("shared-key", &json!({"owner": 1})).unwrap(), 1);
    assert_eq!(second_storage.kv_set("shared-key", &json!({"owner": 2})).unwrap(), 1);
    assert_eq!(first_storage.kv_get("shared-key").unwrap().value, Some(json!({"owner": 1})));
    assert_eq!(second_storage.kv_get("shared-key").unwrap().value, Some(json!({"owner": 2})));

    let conflict = first_storage
        .kv_compare_and_swap("shared-key", None, Some(&json!({"owner": 3})))
        .unwrap();
    assert!(!conflict.applied);
    assert_eq!(conflict.revision, Some(1));
    let changed = first_storage
        .kv_compare_and_swap("shared-key", Some(1), Some(&json!({"owner": 3})))
        .unwrap();
    assert!(changed.applied);
    assert_eq!(changed.revision, Some(2));
    assert!(first_storage.kv_delete("shared-key").unwrap());
    let tombstone = first_storage.kv_get("shared-key").unwrap();
    assert_eq!(tombstone.value, None);
    assert_eq!(tombstone.revision, Some(3));
    assert!(!first_storage.kv_delete("shared-key").unwrap());

    drop(first_storage);
    drop(first);
    let reopened_manager = new_manager(&temp);
    let reopened = reopened_manager
        .open_generation(first_id, generation("generation-1"))
        .unwrap();
    assert_eq!(reopened.storage().kv_get("shared-key").unwrap(), tombstone);
}

#[test]
fn database_allows_normal_ddl_dml_and_blocks_attach_and_reserved_writes() {
    let temp = tempfile::tempdir().unwrap();
    let root = new_manager(&temp)
        .create_empty_generation(plugin("3"), generation("generation-1"))
        .unwrap();
    let storage = root.storage();

    storage
        .db_execute(&PluginSqlStatement::new(
            "CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL)",
            vec![],
        ))
        .unwrap();
    storage
        .db_batch(&[
            PluginSqlStatement::new(
                "INSERT INTO tasks (id, title) VALUES (?1, ?2)",
                vec![PluginSqlValue::Integer(1), PluginSqlValue::Text("one".into())],
            ),
            PluginSqlStatement::new(
                "INSERT INTO tasks (id, title) VALUES (?1, ?2)",
                vec![PluginSqlValue::Integer(2), PluginSqlValue::Text("two".into())],
            ),
        ])
        .unwrap();
    storage
        .db_execute(&PluginSqlStatement::new(
            "ALTER TABLE tasks ADD COLUMN done INTEGER NOT NULL DEFAULT 0",
            vec![],
        ))
        .unwrap();
    let rows = storage
        .db_query(&PluginSqlStatement::new(
            "SELECT id, title, done FROM tasks ORDER BY id",
            vec![],
        ))
        .unwrap();
    assert_eq!(rows.columns, ["id", "title", "done"]);
    assert_eq!(
        rows.rows,
        [
            vec![
                PluginSqlValue::Integer(1),
                PluginSqlValue::Text("one".into()),
                PluginSqlValue::Integer(0),
            ],
            vec![
                PluginSqlValue::Integer(2),
                PluginSqlValue::Text("two".into()),
                PluginSqlValue::Integer(0),
            ],
        ]
    );

    let attach = storage.db_execute(&PluginSqlStatement::new(
        "ATTACH DATABASE ?1 AS core",
        vec![PluginSqlValue::Text(
            temp.path().join("core.sqlite").display().to_string(),
        )],
    ));
    assert!(attach.is_err());
    assert!(
        storage
            .db_execute(&PluginSqlStatement::new(
                "CREATE TABLE _nomifun_owned (value TEXT)",
                vec![],
            ))
            .is_err()
    );
    assert!(
        storage
            .db_execute(&PluginSqlStatement::new(
                "DELETE FROM _nomifun_kv",
                vec![],
            ))
            .is_err()
    );
    assert!(
        storage
            .db_execute(&PluginSqlStatement::new(
                "ALTER TABLE tasks RENAME TO _nomifun_stolen",
                vec![],
            ))
            .is_err()
    );
    storage
        .db_execute(&PluginSqlStatement::new(
            "ALTER TABLE tasks ADD COLUMN note TEXT NOT NULL DEFAULT '_nomifun_literal'",
            vec![],
        ))
        .unwrap();
    assert!(
        storage
            .db_query(&PluginSqlStatement::new("PRAGMA database_list", vec![]))
            .is_err()
    );
}

#[test]
fn files_are_atomic_portable_and_confined_to_the_generation() {
    let temp = tempfile::tempdir().unwrap();
    let root = new_manager(&temp)
        .create_empty_generation(plugin("4"), generation("generation-1"))
        .unwrap();
    let storage = root.storage();

    storage.file_write("notes/today.txt", b"first").unwrap();
    storage.file_write("notes/today.txt", b"second").unwrap();
    storage.file_write("root.bin", &[0, 1, 2]).unwrap();
    assert_eq!(storage.file_read("notes/today.txt").unwrap(), b"second");
    let list = storage.file_list(None).unwrap();
    assert!(list.iter().any(|entry| entry.path == "notes" && entry.is_directory));
    assert!(list.iter().any(|entry| {
        entry.path == "notes/today.txt" && !entry.is_directory && entry.size_bytes == 6
    }));

    for path in ["../escape", "notes/../../escape", "/absolute", "notes\\escape"] {
        assert!(storage.file_write(path, b"blocked").is_err(), "accepted {path}");
    }
    assert!(!temp.path().join("escape").exists());
    assert!(storage.file_delete("notes").unwrap());
    assert!(matches!(
        storage.file_read("notes/today.txt"),
        Err(PluginDataRootError::NotFound(_))
    ));
}

#[test]
fn cache_is_memory_only_scoped_and_expires_by_ttl() {
    let temp = tempfile::tempdir().unwrap();
    let manager = new_manager(&temp);
    let first = manager
        .create_empty_generation(plugin("5"), generation("generation-1"))
        .unwrap();
    let second = manager
        .create_empty_generation(plugin("6"), generation("generation-1"))
        .unwrap();
    first
        .cache()
        .set("answer", json!(42), Some(Duration::from_millis(20)))
        .unwrap();
    assert_eq!(first.cache().get("answer").unwrap(), Some(json!(42)));
    assert_eq!(second.cache().get("answer").unwrap(), None);
    std::thread::sleep(Duration::from_millis(35));
    assert_eq!(first.cache().get("answer").unwrap(), None);

    first.cache().set("ephemeral", json!(true), None).unwrap();
    drop(manager);
    let reopened_manager = new_manager(&temp);
    let reopened = reopened_manager
        .open_generation(plugin("5"), generation("generation-1"))
        .unwrap();
    assert_eq!(reopened.cache().get("ephemeral").unwrap(), None);
}

#[test]
fn preview_clone_uses_real_storage_and_never_mutates_or_publishes_production() {
    let temp = tempfile::tempdir().unwrap();
    let manager = new_manager(&temp);
    let plugin_id = plugin("7");
    let production = manager
        .create_empty_generation(plugin_id.clone(), generation("generation-1"))
        .unwrap();
    production.storage().kv_set("counter", &json!(1)).unwrap();
    production
        .storage()
        .file_write("state.txt", b"production")
        .unwrap();
    production
        .storage()
        .db_execute(&PluginSqlStatement::new(
            "CREATE TABLE state (value TEXT NOT NULL)",
            vec![],
        ))
        .unwrap();
    production
        .storage()
        .db_execute(&PluginSqlStatement::new(
            "INSERT INTO state (value) VALUES (?1)",
            vec![PluginSqlValue::Text("production".into())],
        ))
        .unwrap();

    let preview = manager.clone_preview(&production, "session-1").unwrap();
    let preview_path = preview.handle().path().to_path_buf();
    let staging_parent = std::fs::canonicalize(
        temp.path()
            .join("plugin-data")
            .join(plugin_id.as_ref())
            .join("staging"),
    )
    .unwrap();
    assert!(preview_path.starts_with(staging_parent));
    preview.storage().kv_set("counter", &json!(99)).unwrap();
    preview
        .storage()
        .file_write("state.txt", b"preview")
        .unwrap();
    preview
        .storage()
        .db_execute(&PluginSqlStatement::new(
            "UPDATE state SET value = ?1",
            vec![PluginSqlValue::Text("preview".into())],
        ))
        .unwrap();

    assert_eq!(production.storage().kv_get("counter").unwrap().value, Some(json!(1)));
    assert_eq!(production.storage().file_read("state.txt").unwrap(), b"production");
    let production_db = production
        .storage()
        .db_query(&PluginSqlStatement::new("SELECT value FROM state", vec![]))
        .unwrap();
    assert_eq!(production_db.rows, [vec![PluginSqlValue::Text("production".into())]]);
    preview.destroy().unwrap();
    assert!(!preview_path.exists());
    assert!(
        manager
            .open_generation(plugin_id, generation("preview-session-1"))
            .is_err()
    );
}

#[test]
fn startup_cleanup_removes_only_orphaned_uuidv7_preview_roots() {
    let temp = tempfile::tempdir().unwrap();
    let manager = new_manager(&temp);
    let plugin_id = plugin("71");
    let production = manager
        .create_empty_generation(plugin_id.clone(), generation("generation-1"))
        .unwrap();
    let preview = manager
        .clone_preview(&production, &uuid::Uuid::now_v7().to_string())
        .unwrap();
    let preview_path = preview.handle().path().to_path_buf();
    std::mem::forget(preview);
    let staged = manager
        .stage_clone(
            &production,
            DataGeneration::new(uuid::Uuid::now_v7().to_string()).unwrap(),
        )
        .unwrap();
    let staged_path = staged.path().to_path_buf();

    assert_eq!(manager.cleanup_preview_roots().unwrap(), 1);
    assert!(!preview_path.exists());
    assert!(staged_path.exists(), "install staging must not be cleaned as Preview data");
    assert!(production.path().exists());
    manager.discard_staging(staged).unwrap();
}

#[test]
fn clone_is_complete_publish_is_atomic_and_exact_generation_delete_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let manager = new_manager(&temp);
    let plugin_id = plugin("8");
    let first = manager
        .create_empty_generation(plugin_id.clone(), generation("generation-1"))
        .unwrap();
    first.storage().kv_set("version", &json!(1)).unwrap();
    first.storage().file_write("version.txt", b"one").unwrap();

    let staged = manager
        .stage_clone(&first, generation("generation-2"))
        .unwrap();
    assert_eq!(staged.storage().kv_get("version").unwrap().value, Some(json!(1)));
    assert_eq!(staged.storage().file_read("version.txt").unwrap(), b"one");
    staged.storage().kv_set("version", &json!(2)).unwrap();
    staged.storage().file_write("version.txt", b"two").unwrap();
    assert_eq!(first.storage().kv_get("version").unwrap().value, Some(json!(1)));

    let second = manager.publish(staged).unwrap();
    assert_eq!(second.storage().kv_get("version").unwrap().value, Some(json!(2)));
    assert_eq!(second.storage().file_read("version.txt").unwrap(), b"two");
    assert!(
        manager
            .stage_empty(plugin_id.clone(), generation("generation-2"))
            .is_ok(),
        "published and staging namespaces must be independent"
    );
    assert!(manager
        .delete_generation_exact(&plugin_id, &generation("generation-1"))
        .unwrap());
    assert!(!first.path().exists());
    assert!(second.path().exists());
    assert!(!manager
        .delete_generation_exact(&plugin_id, &generation("generation-1"))
        .unwrap());
}

#[test]
fn generation_pruning_keeps_only_current_and_previous() {
    let temp = tempfile::tempdir().unwrap();
    let manager = new_manager(&temp);
    let plugin_id = plugin("12");
    for name in ["generation-1", "generation-2", "generation-3"] {
        manager
            .create_empty_generation(plugin_id.clone(), generation(name))
            .unwrap();
    }
    let keep = std::collections::BTreeSet::from([
        generation("generation-2"),
        generation("generation-3"),
    ]);
    assert_eq!(manager.prune_generations(&plugin_id, &keep).unwrap(), 1);
    assert!(
        manager
            .open_generation(plugin_id.clone(), generation("generation-1"))
            .is_err()
    );
    assert!(manager
        .open_generation(plugin_id.clone(), generation("generation-2"))
        .is_ok());
    assert!(manager
        .open_generation(plugin_id, generation("generation-3"))
        .is_ok());
}

#[test]
fn migration_ledger_is_ordered_and_digest_immutable() {
    let temp = tempfile::tempdir().unwrap();
    let root = new_manager(&temp)
        .create_empty_generation(plugin("9"), generation("generation-1"))
        .unwrap();
    let storage = root.storage();
    let first = PluginMigrationRecord {
        migration_id: "create_tasks".into(),
        migration_digest: DigestHex::from("a".repeat(64)),
        from_version: 0,
        to_version: 1,
        applied_at_ms: 1,
    };
    storage.record_migration(&first).unwrap();
    storage.record_migration(&first).unwrap();
    assert_eq!(storage.migrations().unwrap(), [first.clone()]);

    let rewritten = PluginMigrationRecord {
        migration_digest: DigestHex::from("b".repeat(64)),
        ..first.clone()
    };
    assert!(storage.record_migration(&rewritten).is_err());
    let skipped = PluginMigrationRecord {
        migration_id: "skip".into(),
        migration_digest: DigestHex::from("c".repeat(64)),
        from_version: 2,
        to_version: 3,
        applied_at_ms: 2,
    };
    assert!(storage.record_migration(&skipped).is_err());
}

#[test]
fn backup_bytes_stage_into_an_unpublished_generation() {
    let temp = tempfile::tempdir().unwrap();
    let manager = new_manager(&temp);
    let source = manager
        .create_empty_generation(plugin("10"), generation("source"))
        .unwrap();
    source.storage().kv_set("restored", &json!(true)).unwrap();
    drop(source.storage());
    let sqlite = std::fs::read(source.path().join("data.sqlite")).unwrap();

    let target_id = plugin("11");
    let staged = manager
        .stage_import(
            target_id.clone(),
            generation("restored"),
            &sqlite,
            &BTreeMap::from([("nested/value.txt".into(), b"backup".to_vec())]),
        )
        .unwrap();
    assert_eq!(
        staged.storage().kv_get("restored").unwrap().value,
        Some(json!(true))
    );
    assert_eq!(
        staged.storage().file_read("nested/value.txt").unwrap(),
        b"backup"
    );
    assert!(
        manager
            .open_generation(target_id.clone(), generation("restored"))
            .is_err(),
        "staging must not become authoritative before publication"
    );
    let published = manager.publish(staged).unwrap();
    assert_eq!(
        manager
            .open_generation(target_id, generation("restored"))
            .unwrap()
            .path(),
        published.path()
    );
}

#[test]
fn exact_plugin_cleanup_removes_generations_staging_and_cache_but_not_siblings() {
    let temp = tempfile::tempdir().unwrap();
    let manager = new_manager(&temp);
    let removed_id = plugin("12");
    let sibling_id = plugin("13");
    let live = manager
        .create_empty_generation(removed_id.clone(), generation("live"))
        .unwrap();
    let staged = manager
        .stage_clone(&live, generation("pending"))
        .unwrap();
    let sibling = manager
        .create_empty_generation(sibling_id.clone(), generation("live"))
        .unwrap();
    live.cache().set("value", json!("live"), None).unwrap();
    staged
        .cache()
        .set("value", json!("staging"), None)
        .unwrap();
    sibling
        .cache()
        .set("value", json!("sibling"), None)
        .unwrap();

    assert!(manager.delete_plugin_exact(&removed_id).unwrap());
    assert!(!temp.path().join("plugin-data").join(removed_id.as_ref()).exists());
    assert_eq!(live.cache().get("value").unwrap(), None);
    assert_eq!(staged.cache().get("value").unwrap(), None);
    assert_eq!(sibling.cache().get("value").unwrap(), Some(json!("sibling")));
    assert!(sibling.path().exists());
    assert!(!manager.delete_plugin_exact(&removed_id).unwrap());
}
