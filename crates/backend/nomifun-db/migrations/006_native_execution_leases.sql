-- Native producer ownership. Epoch and holder must be checked in the same
-- transaction as model/tool admission and journal writes.
ALTER TABLE agent_turns ADD COLUMN execution_owner TEXT;
ALTER TABLE agent_turns ADD COLUMN execution_generation INTEGER NOT NULL DEFAULT 0 CHECK (execution_generation >= 0);
ALTER TABLE agent_turns ADD COLUMN execution_lease_until INTEGER NOT NULL DEFAULT 0
    CHECK (execution_lease_until >= 0);

UPDATE schema_metadata SET migration_head = 5,
    canonical_schema_manifest_digest = '6c3ea4f8d5d3e12cbc36ebb48d7d86ef79d1794630c1acbab0e646799de92d96'
WHERE singleton_key = 'canonical';
