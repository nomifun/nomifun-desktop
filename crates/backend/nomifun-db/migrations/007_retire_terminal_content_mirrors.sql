-- Terminal Event/Payload facts already own these exact contents. Validate the
-- canonical references before retiring copies; damaged current data must roll
-- back this migration instead of losing its diagnostic evidence.
CREATE TEMP TABLE terminal_content_integrity (
    valid INTEGER NOT NULL CHECK (valid = 1)
);
INSERT INTO terminal_content_integrity
SELECT CASE WHEN turn_id = operation_id AND EXISTS (
    SELECT 1 FROM agent_events event
    LEFT JOIN agent_payloads payload ON payload.payload_id = event.payload_id AND payload.session_id = event.session_id
    WHERE event.event_id = agent_turns.terminal_event_id
      AND event.session_id = agent_turns.session_id
      AND event.correlation_id = agent_turns.operation_id
      AND event.kind_version = 1 AND event.seq = agent_turns.finished_at
      AND event.kind = CASE agent_turns.state
          WHEN 'completed' THEN 'turn/completed' WHEN 'failed' THEN 'turn/failed'
          ELSE 'turn/cancelled' END
      AND (event.inline_json IS NOT NULL OR payload.body IS NOT NULL)
) THEN 1 ELSE 0 END
FROM agent_turns WHERE state IN ('completed', 'failed', 'cancelled', 'interrupted');

INSERT INTO terminal_content_integrity
SELECT CASE WHEN EXISTS (
    SELECT 1 FROM agent_events event
    LEFT JOIN agent_payloads payload ON payload.payload_id = event.payload_id AND payload.session_id = event.session_id
    WHERE event.event_id = agent_effects.terminal_event_id
      AND event.session_id = agent_effects.session_id
      AND event.correlation_id = agent_effects.effect_id AND event.kind_version = 1
      AND (event.inline_json IS NOT NULL OR payload.body IS NOT NULL)
      AND agent_effects.state = CASE event.kind
          WHEN 'effect/succeeded' THEN 'returned' WHEN 'effect/failed' THEN 'rejected'
          WHEN 'effect/uncertain' THEN 'unknown'
          WHEN 'effect/reconciled' THEN CASE COALESCE(json_extract(event.inline_json, '$.outcome'),
              json_extract(CAST(payload.body AS TEXT), '$.value.outcome'))
              WHEN 'confirmed_succeeded' THEN 'returned' WHEN 'confirmed_failed' THEN 'rejected'
              WHEN 'still_uncertain' THEN 'unknown' END END
) THEN 1 ELSE 0 END
FROM agent_effects WHERE state <> 'pending';

-- Retain canonical identities, references and lifecycle indexes, with no body
-- conversion and no second completion or reconciliation record.
ALTER TABLE agent_turns DROP COLUMN result_json;
ALTER TABLE agent_turns DROP COLUMN error_json;
ALTER TABLE agent_effects DROP COLUMN bounded_observation_json;
DROP TABLE terminal_content_integrity;
