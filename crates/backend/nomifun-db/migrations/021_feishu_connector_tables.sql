-- Rename the Feishu connector product tables from their retired office_*
-- names to feishu_* names. Rows, ids, sequence high-water marks and foreign
-- keys are preserved in place; SQLite updates dependent schema objects and
-- sqlite_sequence entries during RENAME TO. No Agent facts are converted,
-- rewritten or imported. The only supported metadata evolution is this exact
-- current-generation rename transition.
CREATE TEMP TABLE feishu_connector_rename_guard(valid INTEGER NOT NULL CHECK(valid=1));
INSERT INTO feishu_connector_rename_guard SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM schema_metadata)
    OR EXISTS(SELECT 1 FROM schema_metadata WHERE singleton_key='canonical'
        AND data_generation=7 AND migration_head=1 AND projection_schema_version=1
        AND canonical_schema_manifest_digest IN (
          '565ff15ff4419e0fad8cf0fbcaa7ade3bd0f4fb50263851efed93f7a39921430',
          '9ed5b23d0567152ba77d5a580b6c8af44a12917d6efc404ecb27ad8b8dc924cb'))
    THEN 1 ELSE 0 END;
DROP TABLE feishu_connector_rename_guard;

ALTER TABLE office_connections RENAME TO feishu_connections;
ALTER TABLE office_operations RENAME TO feishu_operations;
ALTER TABLE office_operation_items RENAME TO feishu_operation_items;
ALTER TABLE office_operation_receipts RENAME TO feishu_operation_receipts;
DROP INDEX idx_office_items_pending_target;
CREATE UNIQUE INDEX idx_feishu_items_pending_target ON feishu_operation_items(target_key)
    WHERE status IN ('queued','running','unknown','accepted_pending_confirmation');

UPDATE schema_metadata SET canonical_schema_manifest_digest='9ed5b23d0567152ba77d5a580b6c8af44a12917d6efc404ecb27ad8b8dc924cb'
WHERE singleton_key='canonical'
  AND data_generation=7 AND migration_head=1 AND projection_schema_version=1
  AND canonical_schema_manifest_digest='565ff15ff4419e0fad8cf0fbcaa7ade3bd0f4fb50263851efed93f7a39921430';
