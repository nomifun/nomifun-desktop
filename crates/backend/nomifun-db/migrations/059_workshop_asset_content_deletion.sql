-- Keep a durable identity for historical references while permanently removing
-- the content. Pending file paths remain available until cleanup succeeds.
ALTER TABLE workshop_assets ADD COLUMN deleted_at INTEGER
    CHECK (deleted_at IS NULL OR (typeof(deleted_at) = 'integer' AND deleted_at >= 0));
ALTER TABLE workshop_assets ADD COLUMN content_deleted_at INTEGER
    CHECK (
        (deleted_at IS NULL OR (in_library = 0 AND text_content IS NULL))
        AND (
            content_deleted_at IS NULL
            OR (
                typeof(content_deleted_at) = 'integer'
                AND deleted_at IS NOT NULL
                AND content_deleted_at >= deleted_at
                AND rel_path IS NULL
                AND thumb_rel_path IS NULL
            )
        )
    );

CREATE INDEX idx_workshop_assets_pending_content_deletion
    ON workshop_assets(deleted_at, asset_id)
    WHERE deleted_at IS NOT NULL AND content_deleted_at IS NULL;

CREATE TRIGGER prevent_workshop_asset_content_resurrection
BEFORE UPDATE ON workshop_assets
WHEN OLD.deleted_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'deleted workshop asset cannot be restored')
    WHERE NEW.deleted_at IS NOT OLD.deleted_at
       OR (OLD.content_deleted_at IS NOT NULL
           AND NEW.content_deleted_at IS NOT OLD.content_deleted_at);
END;

DROP INDEX uq_workshop_assets_prompt_library_identity;
CREATE UNIQUE INDEX uq_workshop_assets_prompt_library_identity
    ON workshop_assets(
        json_extract(origin, '$.prompt_library_source'),
        json_extract(origin, '$.prompt_library_id')
    )
    WHERE kind = 'text'
      AND deleted_at IS NULL
      AND json_type(origin, '$.prompt_library_source') = 'text'
      AND json_type(origin, '$.prompt_library_id') = 'text';

-- A deletion and a task submission serialize at the SQLite write boundary.
-- Existing terminal history may still name a tombstone; new work cannot.
CREATE TRIGGER restrict_creation_task_deleted_assets_insert
BEFORE INSERT ON creation_tasks
BEGIN
    SELECT RAISE(ABORT, 'creation task references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        WHERE asset.deleted_at IS NOT NULL AND (
            EXISTS (SELECT 1 FROM json_each(NEW.input_bindings) input
                    WHERE json_extract(input.value, '$.asset_id') = asset.asset_id)
            OR EXISTS (SELECT 1 FROM json_each(NEW.result_asset_ids) result
                       WHERE result.value = asset.asset_id)
        )
    );
END;

CREATE TRIGGER restrict_creation_task_deleted_assets_update
BEFORE UPDATE OF input_bindings, result_asset_ids, status ON creation_tasks
BEGIN
    SELECT RAISE(ABORT, 'creation task references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        WHERE asset.deleted_at IS NOT NULL AND (
            EXISTS (
                SELECT 1 FROM json_each(NEW.input_bindings) input
                WHERE json_extract(input.value, '$.asset_id') = asset.asset_id
                  AND (NEW.status IN ('queued', 'running') OR NOT EXISTS (
                      SELECT 1 FROM json_each(OLD.input_bindings) old_input
                      WHERE json_extract(old_input.value, '$.asset_id') = asset.asset_id
                  ))
            )
            OR EXISTS (
                SELECT 1 FROM json_each(NEW.result_asset_ids) result
                WHERE result.value = asset.asset_id
                  AND (NEW.status IN ('queued', 'running') OR NOT EXISTS (
                      SELECT 1 FROM json_each(OLD.result_asset_ids) old_result
                      WHERE old_result.value = asset.asset_id
                  ))
            )
        )
    );
END;
