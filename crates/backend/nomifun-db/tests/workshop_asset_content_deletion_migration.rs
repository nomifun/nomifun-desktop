use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[test]
fn published_asset_deletion_migrations_have_immutable_checksums() {
    for (version, checksum) in [
        (
            59,
            "ae3e1cbb9d66050fc6c5c631b3f578cb15c12a4d7aaf6be284974765b050c91ab9d08ccdae271702b4cee36f9d536f65",
        ),
        (
            60,
            "534979ae4e86c11970f757f63f724e9e191e2da8a60d5bb3d14513da3d4f22bee693165df12e30111a02fa34097c1d09",
        ),
    ] {
        let migration = MIGRATOR
            .iter()
            .find(|migration| migration.version == version)
            .unwrap();
        assert_eq!(
            hex::encode(&migration.checksum),
            checksum,
            "migration {version} was applied in existing installations; append a migration instead of editing it"
        );
    }
}

#[tokio::test]
async fn migration_058_to_060_preserves_assets_and_enforces_tombstone_lifecycle() {
    verify_asset_deletion_upgrade(58).await;
}

#[tokio::test]
async fn migration_published_059_to_060_preserves_assets_and_pending_deletions() {
    verify_asset_deletion_upgrade(59).await;
}

async fn verify_asset_deletion_upgrade(from_version: i64) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version <= from_version)
    {
        connection.apply(migration).await.unwrap();
    }
    drop(connection);

    let asset_id = nomifun_common::WorkshopAssetId::new().into_string();
    sqlx::query("INSERT INTO workshop_assets (asset_id, kind, title, tags, rel_path, thumb_rel_path, in_library, created_at, updated_at) VALUES (?, 'image', 'existing image', '[]', 'workshop/assets/original.png', 'workshop/thumbs/original.jpg', 1, 10, 10)")
        .bind(&asset_id).execute(&pool).await.unwrap();
    let provider_id = nomifun_common::ProviderId::new().into_string();
    let task_id = nomifun_common::CreationTaskId::new().into_string();
    let input_asset_id = nomifun_common::WorkshopAssetId::new().into_string();
    sqlx::query("INSERT INTO workshop_assets (asset_id, kind, title, tags, rel_path, in_library, created_at, updated_at) VALUES (?, 'image', 'existing input', '[]', 'workshop/assets/input.png', 1, 10, 10)")
        .bind(&input_asset_id).execute(&pool).await.unwrap();
    let bindings =
        serde_json::json!([{"asset_id": input_asset_id, "kind": "image", "role": "reference"}])
            .to_string();
    let results = serde_json::json!([asset_id]).to_string();
    sqlx::query("INSERT INTO providers (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, created_at, updated_at) VALUES (?, 'test', 'legacy provider', 'https://example.invalid', 'bearer', '', 1, 1)")
        .bind(&provider_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO creation_tasks (creation_task_id, workbench_kind, provider_id, model, capability, params, input_bindings, result_asset_ids, status, submitted_at, finished_at, request_fingerprint) VALUES (?, 'image', ?, 'legacy-model', 'i2i', '{}', ?, ?, 'succeeded', 1, 2, '{}')")
        .bind(&task_id).bind(&provider_id).bind(&bindings).bind(&results).execute(&pool).await.unwrap();
    sqlx::query("UPDATE workshop_assets SET origin = ? WHERE asset_id = ?")
        .bind(
            serde_json::json!({ "workbench_kind": "image", "creation_task_id": task_id })
                .to_string(),
        )
        .bind(&asset_id)
        .execute(&pool)
        .await
        .unwrap();
    let before_rows = if from_version == 59 {
        // A previously requested deletion must remain pending across upgrade.
        sqlx::query("UPDATE workshop_assets SET deleted_at = 15, in_library = 0, updated_at = 15 WHERE asset_id = ?")
            .bind(&input_asset_id).execute(&pool).await.unwrap();
        let rows = sqlx::query_as::<_, nomifun_db::WorkshopAssetRow>(
            "SELECT * FROM workshop_assets ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        Some(serde_json::to_value(rows).unwrap())
    } else {
        None
    };
    let lineage_before: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        nomifun_db::inspect_supported_migration_lineage(&pool)
            .await
            .unwrap(),
        nomifun_db::MigrationLineageStatus::UpgradeRequired
    );
    MIGRATOR.run(&pool).await.unwrap();
    if let Some(before_rows) = before_rows {
        let rows = sqlx::query_as::<_, nomifun_db::WorkshopAssetRow>(
            "SELECT * FROM workshop_assets ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(serde_json::to_value(rows).unwrap(), before_rows);
    }
    let lineage_after: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        &lineage_after[..lineage_before.len()],
        lineage_before.as_slice()
    );
    assert_eq!(lineage_after.last().unwrap().0, 60);
    nomifun_db::validate_id_schema_contract(&pool)
        .await
        .unwrap();
    let row: (Option<i64>, Option<i64>, bool, String) = sqlx::query_as(
        "SELECT deleted_at, content_deleted_at, in_library, rel_path FROM workshop_assets WHERE asset_id = ?",
    ).bind(&asset_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        row,
        (None, None, true, "workshop/assets/original.png".into())
    );

    for invalid in [
        "deleted_at = -1",
        "deleted_at = 20",
        "content_deleted_at = 20",
    ] {
        assert!(
            sqlx::query(&format!(
                "UPDATE workshop_assets SET {invalid} WHERE asset_id = ?"
            ))
            .bind(&asset_id)
            .execute(&pool)
            .await
            .is_err(),
            "accepted {invalid}"
        );
    }
    sqlx::query("UPDATE workshop_assets SET deleted_at = 20, in_library = 0 WHERE asset_id = ?")
        .bind(&asset_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE workshop_assets SET content_deleted_at = 21 WHERE asset_id = ?")
            .bind(&asset_id)
            .execute(&pool)
            .await
            .is_err(),
        "finish must clear both paths"
    );
    sqlx::query("UPDATE workshop_assets SET content_deleted_at = 21, rel_path = NULL, thumb_rel_path = NULL WHERE asset_id = ?")
        .bind(&asset_id).execute(&pool).await.unwrap();
    for invalid in [
        "deleted_at = NULL",
        "content_deleted_at = NULL",
        "rel_path = 'restored.png'",
        "in_library = 1",
    ] {
        assert!(
            sqlx::query(&format!(
                "UPDATE workshop_assets SET {invalid} WHERE asset_id = ?"
            ))
            .bind(&asset_id)
            .execute(&pool)
            .await
            .is_err(),
            "accepted {invalid}"
        );
    }
    nomifun_db::validate_id_data_contract(&pool).await.unwrap();
    let history: (String, String, String) = sqlx::query_as(
        "SELECT status, input_bindings, result_asset_ids FROM creation_tasks WHERE creation_task_id = ?",
    ).bind(&task_id).fetch_one(&pool).await.unwrap();
    assert_eq!(history, ("succeeded".into(), bindings, results));
    sqlx::query("DROP TRIGGER restrict_template_run_deleted_assets_update")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        nomifun_db::validate_id_schema_contract(&pool)
            .await
            .is_err(),
        "startup must reject a missing deletion race guard"
    );
}
