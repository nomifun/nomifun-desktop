-- Nonterminal native suspension and explicitly authorized allowance changes.
-- Do not rewrite immutable completed/failed/cancelled Turn terminals.
ALTER TABLE agent_turns ADD COLUMN native_pause_revision INTEGER NOT NULL DEFAULT 0
    CHECK (native_pause_revision >= 0);
ALTER TABLE agent_turns ADD COLUMN native_pause_json TEXT
    CHECK (native_pause_json IS NULL OR json_valid(native_pause_json));
ALTER TABLE agent_turns ADD COLUMN native_pause_requested_json TEXT
    CHECK (native_pause_requested_json IS NULL OR json_valid(native_pause_requested_json));
ALTER TABLE agent_turns ADD COLUMN native_budget_json TEXT
    CHECK (native_budget_json IS NULL OR json_valid(native_budget_json));

-- Earlier effect/uncertain projections cleared the active pointer without
-- ending the native Turn. Repair only the unambiguous nonterminal projection;
-- acquisition/side-effect gates still require lease and outcome reconciliation.
UPDATE agent_session_heads
SET status = 'reconciliation',
    active_turn_id = (
        SELECT t.operation_id FROM agent_turns t
        WHERE t.session_id = agent_session_heads.session_id
          AND t.state = 'running' AND t.terminal_event_id IS NULL
          AND t.execution_owner IS NOT NULL
        ORDER BY t.started_at DESC LIMIT 1
    )
WHERE status = 'failed' AND active_turn_id IS NULL
  AND 1 = (
      SELECT COUNT(*) FROM agent_turns t
      WHERE t.session_id = agent_session_heads.session_id
        AND t.state = 'running' AND t.terminal_event_id IS NULL
        AND t.execution_owner IS NOT NULL
  );

UPDATE schema_metadata SET migration_head = 6,
    canonical_schema_manifest_digest = 'de5d31539af0cc75f26d2090546a908b39e8719665d3f8dac6eda6e0b26fa036'
WHERE singleton_key = 'canonical';
