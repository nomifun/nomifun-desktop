ALTER TABLE agent_sessions
ADD COLUMN reasoning_effort TEXT
    CHECK (reasoning_effort IS NULL OR reasoning_effort IN ('low', 'medium', 'high'));

UPDATE schema_metadata
SET migration_head = 2,
    canonical_schema_manifest_digest = 'd6fcfed0f24fac2e3045e1a920e36b2d6a3751f7adb2d59de363e171e20e6b1f'
WHERE singleton_key = 'canonical';
