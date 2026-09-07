-- Migration 059 has already shipped: its bytes and recorded checksum are
-- immutable. Apply the remaining deletion guards as a separate migration so
-- existing 059 databases and fresh installations converge without data resets.
DROP TRIGGER prevent_workshop_asset_content_resurrection;
CREATE TRIGGER prevent_workshop_asset_content_resurrection
BEFORE UPDATE ON workshop_assets
WHEN OLD.deleted_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'deleted workshop asset cannot be restored')
    WHERE NEW.deleted_at IS NOT OLD.deleted_at
       OR NEW.asset_id IS NOT OLD.asset_id
       OR (NEW.rel_path IS NOT NULL AND NEW.rel_path IS NOT OLD.rel_path)
       OR (NEW.thumb_rel_path IS NOT NULL AND NEW.thumb_rel_path IS NOT OLD.thumb_rel_path)
       OR (OLD.content_deleted_at IS NOT NULL
           AND NEW.content_deleted_at IS NOT OLD.content_deleted_at);
END;
-- Canonical template aggregates keep terminal history, but a new run or a
-- live state transition must never start using deleted content. Match only
-- asset fields: an asset UUID mentioned in prompt text is not a reference.
CREATE TRIGGER restrict_template_run_deleted_assets_insert
BEFORE INSERT ON creative_studio_template_runs
BEGIN
    SELECT RAISE(ABORT, 'template run references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        JOIN json_tree(NEW.aggregate_json) ref ON ref.value = asset.asset_id
        LEFT JOIN json_tree(NEW.aggregate_json) parent ON parent.id = ref.parent
        WHERE asset.deleted_at IS NOT NULL AND ref.type = 'text' AND (
            ref.key IN ('assetId', 'defaultAssetId')
            OR parent.key IN ('assetIds', 'defaultAssetIds', 'referenceAssetIds', 'resultAssetIds')
        )
    );
END;

CREATE TRIGGER restrict_template_run_deleted_assets_update
BEFORE UPDATE OF aggregate_json, status ON creative_studio_template_runs
BEGIN
    SELECT RAISE(ABORT, 'template run references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        JOIN json_tree(NEW.aggregate_json) ref ON ref.value = asset.asset_id
        LEFT JOIN json_tree(NEW.aggregate_json) parent ON parent.id = ref.parent
        WHERE asset.deleted_at IS NOT NULL AND ref.type = 'text' AND (
            ref.key IN ('assetId', 'defaultAssetId')
            OR parent.key IN ('assetIds', 'defaultAssetIds', 'referenceAssetIds', 'resultAssetIds')
        ) AND (
            NEW.status NOT IN ('succeeded', 'failed', 'cancelled')
            OR NOT EXISTS (
                SELECT 1 FROM json_tree(OLD.aggregate_json) old_ref
                LEFT JOIN json_tree(OLD.aggregate_json) old_parent ON old_parent.id = old_ref.parent
                WHERE old_ref.value = asset.asset_id AND old_ref.type = 'text' AND (
                    old_ref.key IN ('assetId', 'defaultAssetId')
                    OR old_parent.key IN ('assetIds', 'defaultAssetIds', 'referenceAssetIds', 'resultAssetIds')
                )
            )
        )
    );
END;
