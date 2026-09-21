use nomifun_db::sqlx::Row;

async fn assert_uses_index(
    pool: &nomifun_db::SqlitePool,
    query: &str,
    expected_index: &str,
) {
    let rows = nomifun_db::sqlx::query(&format!("EXPLAIN QUERY PLAN {query}"))
        .fetch_all(pool)
        .await
        .expect("query plan");
    let details = rows
        .iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>();
    assert!(
        details.iter().any(|detail| detail.contains(expected_index)),
        "expected {expected_index} for {query}, plan={details:?}"
    );
}

#[tokio::test]
async fn canonical_baseline_keeps_a_bounded_index_inventory() {
    let database = nomifun_db::init_database_memory()
        .await
        .expect("canonical in-memory database");
    let explicit_indexes: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND sql IS NOT NULL",
    )
    .fetch_one(database.pool())
    .await
    .expect("explicit index count");

    assert_eq!(explicit_indexes, 127);
}

#[tokio::test]
async fn hot_queries_use_the_curated_composite_and_partial_indexes() {
    let database = nomifun_db::init_database_memory()
        .await
        .expect("canonical in-memory database");
    let pool = database.pool();

    for (query, index) in [
        (
            "SELECT * FROM agent_execution_events WHERE published_at IS NULL \
             ORDER BY execution_id, sequence LIMIT 10",
            "idx_execution_events_unpublished",
        ),
        (
            "SELECT * FROM creation_tasks \
             WHERE status IN ('queued', 'running') AND deleted_at IS NULL \
             ORDER BY submitted_at, creation_task_id",
            "idx_creation_tasks_live",
        ),
        (
            "SELECT creation_task_id FROM creation_tasks \
             WHERE project_id = 'project' AND node_id IS NOT NULL \
               AND template_id IS NULL AND template_run_id IS NULL \
               AND template_step_id IS NULL AND status IN ('queued', 'running') \
             ORDER BY submitted_at, creation_task_id LIMIT 1",
            "idx_creation_tasks_live_project",
        ),
        (
            "SELECT creation_task_id FROM creation_tasks \
             WHERE provider_id = 'provider' AND model = 'model' \
               AND status IN ('queued', 'running') \
             ORDER BY submitted_at, creation_task_id LIMIT 1",
            "idx_creation_tasks_live_provider",
        ),
        (
            "SELECT * FROM workshop_assets WHERE deleted_at IS NULL \
             ORDER BY updated_at DESC, id DESC LIMIT 20",
            "idx_workshop_assets_live_updated",
        ),
        (
            "SELECT * FROM workshop_assets \
             WHERE deleted_at IS NULL AND in_library = 1 AND kind = 'image' \
             ORDER BY updated_at DESC, id DESC LIMIT 20",
            "idx_workshop_assets_live_library_kind",
        ),
        (
            "SELECT * FROM cron_jobs \
             WHERE user_id = 'user' AND conversation_id = 'conversation' \
             ORDER BY created_at",
            "idx_cron_jobs_owner_conversation",
        ),
        (
            "SELECT * FROM cron_run_reservations \
             WHERE cron_job_id = 'job' AND status = 'reserved' \
             ORDER BY created_at_ms, id LIMIT 1",
            "idx_cron_run_reservations_cron_job_id",
        ),
        (
            "SELECT * FROM plugin_products WHERE owner_user_id = 'user' \
             ORDER BY updated_at DESC, id DESC",
            "idx_plugin_products_owner",
        ),
        (
            "SELECT * FROM conversation_execution_links \
             WHERE conversation_id = 'conversation' \
               AND relation = 'attempt' AND active = 1 \
             ORDER BY updated_at DESC",
            "idx_conversation_execution_links_conversation_id",
        ),
    ] {
        assert_uses_index(pool, query, index).await;
    }
}
