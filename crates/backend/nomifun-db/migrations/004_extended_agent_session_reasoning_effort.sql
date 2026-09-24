-- `reasoning_effort` from migration 003 remains as a bounded lineage column.
-- The v2 column is authoritative and can represent newer provider tiers
-- without rewriting a parent table referenced by the Agent Store graph.
ALTER TABLE agent_sessions
ADD COLUMN reasoning_effort_v2 TEXT
    CHECK (
        reasoning_effort_v2 IS NULL OR
        reasoning_effort_v2 IN ('low', 'medium', 'high', 'xhigh', 'max', 'ultra')
    );

UPDATE agent_sessions
SET reasoning_effort_v2 = reasoning_effort
WHERE reasoning_effort IS NOT NULL;

UPDATE schema_metadata
SET migration_head = 3,
    canonical_schema_manifest_digest = '263a05e5d0a8bb3e535b531791fc3828600cd37f47b645288b0b98acb4cd8856'
WHERE singleton_key = 'canonical';
