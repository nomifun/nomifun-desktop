-- Latest native execution state belongs to the canonical Turn, not a second
-- transcript. Replacing it does not grow the immutable payload archive.
ALTER TABLE agent_turns ADD COLUMN native_checkpoint_json TEXT
    CHECK (native_checkpoint_json IS NULL OR json_valid(native_checkpoint_json));
ALTER TABLE agent_turns ADD COLUMN native_checkpoint_digest TEXT
    CHECK (native_checkpoint_digest IS NULL OR length(native_checkpoint_digest) = 64);
ALTER TABLE agent_turns ADD COLUMN native_checkpoint_revision INTEGER NOT NULL DEFAULT 0
    CHECK (native_checkpoint_revision >= 0);
ALTER TABLE agent_turns ADD COLUMN native_checkpoint_seq INTEGER
    CHECK (native_checkpoint_seq IS NULL OR native_checkpoint_seq >= 0);
ALTER TABLE agent_turns ADD COLUMN execution_fence INTEGER NOT NULL DEFAULT 0
    CHECK (execution_fence >= 0);

UPDATE schema_metadata SET migration_head = 4,
    canonical_schema_manifest_digest = 'af0ebd60c86565247e1ea28ae61a1afcdf4c06a407da79c9552a1996d2023b5d'
WHERE singleton_key = 'canonical';
