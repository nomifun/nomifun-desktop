-- Durable, reversible-at-the-storage-layer history retirement. Task rows and
-- their input/result asset manifests remain authoritative for direct reads,
-- idempotent replay, boot audit, and reference protection.

ALTER TABLE creation_tasks ADD COLUMN deleted_at INTEGER
    CHECK (
        deleted_at IS NULL
        OR (
            deleted_at >= 0
            AND deleted_at >= submitted_at
            AND project_id IS NOT NULL
            AND workbench_kind IS NOT NULL
            AND node_id IS NULL
            AND workflow_id IS NULL
            AND workflow_run_id IS NULL
            AND workflow_step_id IS NULL
            AND status IN ('failed', 'canceled', 'succeeded')
        )
    );

DROP INDEX idx_creation_tasks_workbench_owner;

CREATE INDEX idx_creation_tasks_workbench_owner_deleted_page
    ON creation_tasks(
        project_id,
        workbench_kind,
        deleted_at,
        submitted_at DESC,
        creation_task_id DESC
    )
    WHERE workbench_kind IS NOT NULL;

CREATE TRIGGER restrict_workshop_asset_delete_creation_task_refs
BEFORE DELETE ON workshop_assets
WHEN EXISTS (
    SELECT 1
    FROM creation_tasks task
    WHERE EXISTS (
        SELECT 1 FROM json_each(task.input_bindings) input
        WHERE json_extract(input.value, '$.asset_id') = OLD.asset_id
    ) OR EXISTS (
        SELECT 1 FROM json_each(task.result_asset_ids) result
        WHERE result.value = OLD.asset_id
    )
)
BEGIN
    SELECT RAISE(ABORT, 'workshop asset is referenced by creation task input or result');
END;
