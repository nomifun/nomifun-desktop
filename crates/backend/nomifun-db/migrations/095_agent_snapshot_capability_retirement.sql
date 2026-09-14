-- Converge the retired two-bucket capability projection into the single
-- enabled_capabilities set used by the current immutable Agent snapshot.
-- Historical AgentPreset revision payloads remain untouched: their digests
-- cover those exact bytes. Only consumer-owned frozen projections are updated.

CREATE TEMP TABLE retired_snapshot_capability_shape_guard (
    invalid_count INTEGER NOT NULL CHECK (invalid_count = 0)
);

INSERT INTO retired_snapshot_capability_shape_guard (invalid_count)
SELECT count(*)
FROM (
    SELECT agent_snapshot FROM conversations
    UNION ALL
    SELECT agent_snapshot FROM cron_jobs
    UNION ALL
    SELECT agent_snapshot FROM agent_execution_participants
    UNION ALL
    SELECT agent_snapshot FROM agent_execution_template_participants
)
WHERE agent_snapshot IS NOT NULL
  AND json_valid(agent_snapshot)
  AND json_type(agent_snapshot) = 'object'
  AND (
      (
          json_type(agent_snapshot, '$.initial_capabilities') IS NOT NULL
          AND json_type(agent_snapshot, '$.initial_capabilities') <> 'array'
      )
      OR (
          json_type(agent_snapshot, '$.on_demand_capabilities') IS NOT NULL
          AND json_type(agent_snapshot, '$.on_demand_capabilities') <> 'array'
      )
      OR (
          json_type(agent_snapshot, '$.enabled_capabilities') IS NOT NULL
          AND json_type(agent_snapshot, '$.enabled_capabilities') <> 'array'
      )
      OR EXISTS (
          SELECT 1
          FROM json_each(agent_snapshot, '$.initial_capabilities')
          WHERE type <> 'text'
      )
      OR EXISTS (
          SELECT 1
          FROM json_each(agent_snapshot, '$.on_demand_capabilities')
          WHERE type <> 'text'
      )
      OR EXISTS (
          SELECT 1
          FROM json_each(agent_snapshot, '$.enabled_capabilities')
          WHERE type <> 'text'
      )
  );

UPDATE conversations AS owner
SET agent_snapshot = json_set(
    json_remove(
        owner.agent_snapshot,
        '$.initial_capabilities',
        '$.on_demand_capabilities'
    ),
    '$.enabled_capabilities',
    json((
        SELECT json_group_array(value)
        FROM (
            SELECT value FROM json_each(owner.agent_snapshot, '$.enabled_capabilities')
            UNION
            SELECT value FROM json_each(owner.agent_snapshot, '$.initial_capabilities')
            UNION
            SELECT value FROM json_each(owner.agent_snapshot, '$.on_demand_capabilities')
            ORDER BY value
        )
    ))
)
WHERE json_valid(owner.agent_snapshot)
  AND json_type(owner.agent_snapshot) = 'object'
  AND (
      json_type(owner.agent_snapshot, '$.initial_capabilities') IS NOT NULL
      OR json_type(owner.agent_snapshot, '$.on_demand_capabilities') IS NOT NULL
  );

UPDATE cron_jobs AS owner
SET agent_snapshot = json_set(
    json_remove(
        owner.agent_snapshot,
        '$.initial_capabilities',
        '$.on_demand_capabilities'
    ),
    '$.enabled_capabilities',
    json((
        SELECT json_group_array(value)
        FROM (
            SELECT value FROM json_each(owner.agent_snapshot, '$.enabled_capabilities')
            UNION
            SELECT value FROM json_each(owner.agent_snapshot, '$.initial_capabilities')
            UNION
            SELECT value FROM json_each(owner.agent_snapshot, '$.on_demand_capabilities')
            ORDER BY value
        )
    ))
)
WHERE json_valid(owner.agent_snapshot)
  AND json_type(owner.agent_snapshot) = 'object'
  AND (
      json_type(owner.agent_snapshot, '$.initial_capabilities') IS NOT NULL
      OR json_type(owner.agent_snapshot, '$.on_demand_capabilities') IS NOT NULL
  );

UPDATE agent_execution_participants AS owner
SET agent_snapshot = json_set(
    json_remove(
        owner.agent_snapshot,
        '$.initial_capabilities',
        '$.on_demand_capabilities'
    ),
    '$.enabled_capabilities',
    json((
        SELECT json_group_array(value)
        FROM (
            SELECT value FROM json_each(owner.agent_snapshot, '$.enabled_capabilities')
            UNION
            SELECT value FROM json_each(owner.agent_snapshot, '$.initial_capabilities')
            UNION
            SELECT value FROM json_each(owner.agent_snapshot, '$.on_demand_capabilities')
            ORDER BY value
        )
    ))
)
WHERE json_valid(owner.agent_snapshot)
  AND json_type(owner.agent_snapshot) = 'object'
  AND (
      json_type(owner.agent_snapshot, '$.initial_capabilities') IS NOT NULL
      OR json_type(owner.agent_snapshot, '$.on_demand_capabilities') IS NOT NULL
  );

UPDATE agent_execution_template_participants AS owner
SET agent_snapshot = json_set(
    json_remove(
        owner.agent_snapshot,
        '$.initial_capabilities',
        '$.on_demand_capabilities'
    ),
    '$.enabled_capabilities',
    json((
        SELECT json_group_array(value)
        FROM (
            SELECT value FROM json_each(owner.agent_snapshot, '$.enabled_capabilities')
            UNION
            SELECT value FROM json_each(owner.agent_snapshot, '$.initial_capabilities')
            UNION
            SELECT value FROM json_each(owner.agent_snapshot, '$.on_demand_capabilities')
            ORDER BY value
        )
    ))
)
WHERE json_valid(owner.agent_snapshot)
  AND json_type(owner.agent_snapshot) = 'object'
  AND (
      json_type(owner.agent_snapshot, '$.initial_capabilities') IS NOT NULL
      OR json_type(owner.agent_snapshot, '$.on_demand_capabilities') IS NOT NULL
  );

DROP TABLE retired_snapshot_capability_shape_guard;
