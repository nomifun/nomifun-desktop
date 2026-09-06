-- Plugin N1 mount mutation seams.
--
-- Migration 067 owns the durable data root. This migration adds the
-- configuration and credential-binding revision facts needed by host
-- mutation boundaries. Existing uninstalled/retained mounts start at
-- revision zero and receive their first configuration snapshot during the
-- first repository Apply.

ALTER TABLE plugin_mounts ADD COLUMN config_schema_digest TEXT CHECK (
    config_schema_digest IS NULL OR (
        length(config_schema_digest) = 64
        AND lower(config_schema_digest) = config_schema_digest
        AND config_schema_digest NOT GLOB '*[^0-9a-f]*'
    )
);

ALTER TABLE plugin_mounts ADD COLUMN config_revision INTEGER NOT NULL DEFAULT 0 CHECK (
    config_revision >= 0
);

ALTER TABLE plugin_mounts ADD COLUMN credential_bindings_revision INTEGER NOT NULL DEFAULT 0 CHECK (
    credential_bindings_revision >= 0
);

-- Repository-only transaction context. Binding rows are intentionally
-- unwriteable by ad-hoc SQL unless the whole-group CAS has been prepared.
CREATE TABLE plugin_credential_binding_mutations (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_id                    TEXT NOT NULL UNIQUE CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    expected_mount_revision    INTEGER NOT NULL CHECK (expected_mount_revision >= 0),
    expected_current_artifact_digest TEXT CHECK (
        expected_current_artifact_digest IS NULL OR (
            length(expected_current_artifact_digest) = 64
            AND lower(expected_current_artifact_digest) = expected_current_artifact_digest
            AND expected_current_artifact_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    expected_bindings_revision INTEGER NOT NULL CHECK (expected_bindings_revision >= 0),
    target_bindings_revision   INTEGER NOT NULL CHECK (target_bindings_revision >= 1),
    allow_delete_pending       INTEGER NOT NULL DEFAULT 0 CHECK (allow_delete_pending IN (0, 1)),
    updated_at                 INTEGER NOT NULL CHECK (updated_at >= 0)
);

CREATE INDEX idx_plugin_credential_binding_mutations_mount_id
    ON plugin_credential_binding_mutations(mount_id);

CREATE TRIGGER trg_plugin_mount_updated_at_monotonic
BEFORE UPDATE OF updated_at ON plugin_mounts
WHEN NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'plugin mount updated_at cannot move backwards');
END;

CREATE TRIGGER trg_plugin_mount_config_revision_guard
BEFORE UPDATE OF config_json, config_schema_digest, config_revision
ON plugin_mounts
WHEN (
    (
        (NEW.config_json IS NOT OLD.config_json
         OR NEW.config_schema_digest IS NOT OLD.config_schema_digest)
        AND NEW.config_revision <= OLD.config_revision
    )
    OR
    (
        NEW.config_revision <> OLD.config_revision
        AND NEW.config_json IS OLD.config_json
        AND NEW.config_schema_digest IS OLD.config_schema_digest
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount config changes require an advancing config revision');
END;

CREATE TRIGGER trg_plugin_mount_config_revision_shape_guard
BEFORE UPDATE OF config_schema_digest, config_revision ON plugin_mounts
WHEN NEW.config_revision = 0 AND NEW.config_schema_digest IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'plugin mount config schema requires a positive config revision');
END;

CREATE TRIGGER trg_plugin_credential_binding_insert_guard
BEFORE INSERT ON plugin_credential_bindings
WHEN NOT EXISTS (
    SELECT 1
    FROM plugin_mounts mount
    LEFT JOIN plugin_credential_binding_mutations mutation
      ON mutation.mount_id = mount.mount_id
     AND mutation.expected_mount_revision = mount.revision
     AND mutation.expected_current_artifact_digest IS mount.current_artifact_digest
     AND mutation.expected_bindings_revision = mount.credential_bindings_revision
     AND mutation.target_bindings_revision = mount.credential_bindings_revision + 1
    WHERE mount.mount_id = NEW.mount_id
      AND (
          (mount.delete_pending = 0 OR mutation.allow_delete_pending = 1)
          AND mutation.mount_id IS NOT NULL
      )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin credential binding insert requires a whole-group CAS');
END;

CREATE TRIGGER trg_plugin_credential_binding_update_guard
BEFORE UPDATE ON plugin_credential_bindings
WHEN NOT EXISTS (
    SELECT 1
    FROM plugin_mounts mount
    LEFT JOIN plugin_credential_binding_mutations mutation
      ON mutation.mount_id = mount.mount_id
     AND mutation.expected_mount_revision = mount.revision
     AND mutation.expected_current_artifact_digest IS mount.current_artifact_digest
     AND mutation.expected_bindings_revision = mount.credential_bindings_revision
     AND mutation.target_bindings_revision = mount.credential_bindings_revision + 1
    WHERE mount.mount_id = OLD.mount_id
      AND (
          (mount.delete_pending = 0 OR mutation.allow_delete_pending = 1)
          AND mutation.mount_id IS NOT NULL
      )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin credential binding update requires a whole-group CAS');
END;

CREATE TRIGGER trg_plugin_credential_binding_delete_guard
BEFORE DELETE ON plugin_credential_bindings
WHEN NOT EXISTS (
    SELECT 1
    FROM plugin_mounts mount
    LEFT JOIN plugin_credential_binding_mutations mutation
      ON mutation.mount_id = mount.mount_id
     AND mutation.expected_mount_revision = mount.revision
     AND mutation.expected_current_artifact_digest IS mount.current_artifact_digest
     AND mutation.expected_bindings_revision = mount.credential_bindings_revision
     AND mutation.target_bindings_revision = mount.credential_bindings_revision + 1
    WHERE mount.mount_id = OLD.mount_id
      AND (
          (mount.delete_pending = 0 OR mutation.allow_delete_pending = 1)
          AND mutation.mount_id IS NOT NULL
      )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin credential binding delete requires a whole-group CAS');
END;

CREATE TRIGGER trg_plugin_mount_binding_revision_guard
BEFORE UPDATE OF credential_bindings_revision ON plugin_mounts
WHEN (
    NEW.credential_bindings_revision <> OLD.credential_bindings_revision
    AND NOT EXISTS (
        SELECT 1
        FROM plugin_credential_binding_mutations mutation
        WHERE mutation.mount_id = OLD.mount_id
          AND mutation.expected_mount_revision = OLD.revision
          AND mutation.expected_bindings_revision = OLD.credential_bindings_revision
          AND mutation.target_bindings_revision = NEW.credential_bindings_revision
          AND mutation.updated_at = NEW.updated_at
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount credential bindings revision requires a whole-group CAS');
END;

CREATE TRIGGER trg_plugin_mount_binding_revision_shape_guard
BEFORE UPDATE OF credential_bindings_revision ON plugin_mounts
WHEN NEW.credential_bindings_revision <> OLD.credential_bindings_revision
 AND NEW.credential_bindings_revision <> OLD.credential_bindings_revision + 1
BEGIN
    SELECT RAISE(ABORT, 'plugin mount credential bindings revision must advance by one');
END;

CREATE TRIGGER trg_plugin_mount_binding_revision_cleanup
AFTER UPDATE OF credential_bindings_revision ON plugin_mounts
WHEN NEW.credential_bindings_revision <> OLD.credential_bindings_revision
BEGIN
    DELETE FROM plugin_credential_binding_mutations
     WHERE mount_id = NEW.mount_id;
END;

CREATE TRIGGER trg_plugin_credential_binding_updated_at_monotonic
BEFORE UPDATE OF updated_at ON plugin_credential_bindings
WHEN NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'plugin credential binding updated_at cannot move backwards');
END;

CREATE TRIGGER trg_plugin_kv_updated_at_monotonic
BEFORE UPDATE OF updated_at ON plugin_kv
WHEN NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'plugin KV updated_at cannot move backwards');
END;
