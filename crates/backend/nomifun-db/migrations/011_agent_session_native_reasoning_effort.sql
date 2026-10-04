-- Older bounded columns remain intact for migration lineage. v3 represents
-- native none/minimal tiers without rebuilding the Agent Store parent table.
ALTER TABLE agent_sessions
ADD COLUMN reasoning_effort_v3 TEXT
    CHECK (
        reasoning_effort_v3 IS NULL OR (
            state <> 'deleted' AND
            reasoning_effort_v3 IN ('none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra')
        )
    );

UPDATE agent_sessions
SET reasoning_effort_v3 = COALESCE(reasoning_effort_v2, reasoning_effort)
WHERE state <> 'deleted';

UPDATE schema_metadata
SET migration_head = 7,
    canonical_schema_manifest_digest = '72689a4befa1699d84fd99e4f15c17b3191d8ce244c0c15120da95c239f464af'
WHERE singleton_key = 'canonical';
