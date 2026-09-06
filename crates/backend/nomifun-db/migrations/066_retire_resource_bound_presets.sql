-- Clean-cut migration for the target-scoped resource contract.
--
-- Revisions written by retired authoring models can contain concrete resource
-- identities anywhere in `payload_json` or `snapshot_json`. They cannot be
-- safely interpreted as the new resource-neutral contract, so retire those
-- product entries once, remove active bindings, and retain the immutable rows
-- as historical audit data.

WITH legacy_presets AS (
    SELECT DISTINCT revision.preset_id
    FROM nomi_agent_preset_revisions AS revision
    WHERE EXISTS (
        SELECT 1
        FROM json_tree(revision.payload_json) AS payload_node
        WHERE payload_node.key IN (
            'resource_bindings',
            'resource_binding_refs',
            'typed_resource_bindings',
            'typed_resource_defaults'
        )
    )
    OR EXISTS (
        SELECT 1
        FROM json_tree(revision.snapshot_json) AS snapshot_node
        WHERE snapshot_node.key IN (
            'resource_bindings',
            'resource_binding_refs',
            'typed_resource_bindings',
            'typed_resource_defaults'
        )
    )
)
DELETE FROM nomi_agent_bindings
WHERE json_extract(agent_binding_json, '$.preset_revision_ref.preset_id') IN (
    SELECT preset_id FROM legacy_presets
);

WITH legacy_presets AS (
    SELECT DISTINCT revision.preset_id
    FROM nomi_agent_preset_revisions AS revision
    WHERE EXISTS (
        SELECT 1
        FROM json_tree(revision.payload_json) AS payload_node
        WHERE payload_node.key IN (
            'resource_bindings',
            'resource_binding_refs',
            'typed_resource_bindings',
            'typed_resource_defaults'
        )
    )
    OR EXISTS (
        SELECT 1
        FROM json_tree(revision.snapshot_json) AS snapshot_node
        WHERE snapshot_node.key IN (
            'resource_bindings',
            'resource_binding_refs',
            'typed_resource_bindings',
            'typed_resource_defaults'
        )
    )
)
DELETE FROM remote_bindings
WHERE json_extract(agent_binding_json, '$.preset_revision_ref.preset_id') IN (
    SELECT preset_id FROM legacy_presets
);

WITH legacy_presets AS (
    SELECT DISTINCT revision.preset_id
    FROM nomi_agent_preset_revisions AS revision
    WHERE EXISTS (
        SELECT 1
        FROM json_tree(revision.payload_json) AS payload_node
        WHERE payload_node.key IN (
            'resource_bindings',
            'resource_binding_refs',
            'typed_resource_bindings',
            'typed_resource_defaults'
        )
    )
    OR EXISTS (
        SELECT 1
        FROM json_tree(revision.snapshot_json) AS snapshot_node
        WHERE snapshot_node.key IN (
            'resource_bindings',
            'resource_binding_refs',
            'typed_resource_bindings',
            'typed_resource_defaults'
        )
    )
)
UPDATE nomi_agent_presets
SET retired_at_ms = COALESCE(
    retired_at_ms,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
)
WHERE preset_id IN (
    SELECT preset_id FROM legacy_presets
);
