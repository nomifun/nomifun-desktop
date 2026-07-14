-- Hard-cut the former fleet/orchestrator persistence model to one agent-execution
-- aggregate.  This migration is deliberately strict: corrupt or ambiguous legacy
-- state aborts the transaction instead of being guessed at or silently discarded.

-- SQLite only permits RAISE() inside triggers.  A one-row CHECK table gives every
-- preflight assertion an explicit constraint name while keeping the migration
-- entirely transactional.
CREATE TABLE _m037_guard (
    ok INTEGER NOT NULL CONSTRAINT m037_preflight_failed CHECK (ok = 1)
);

-- The requirement claim timestamp was renamed in the domain model to reflect
-- what it actually fences: the currently active turn, not the lifetime of the
-- owner claim.  This is a hard column rename, so both upgraded and freshly
-- migrated databases expose one canonical name while preserving every value.
ALTER TABLE requirements RENAME COLUMN claimed_at TO active_turn_started_at;

-- Collaboration model preference follows the unified Agent vocabulary. Value
-- priority is canonical > unpublished intermediate > released legacy; remove
-- both obsolete keys in this same transaction.
INSERT OR IGNORE INTO client_preferences (key, value, updated_at)
SELECT 'nomi.collaborationModels', value, updated_at
FROM client_preferences
WHERE key = 'nomi.executionCollaborators';

INSERT OR IGNORE INTO client_preferences (key, value, updated_at)
SELECT 'nomi.collaborationModels', value, updated_at
FROM client_preferences
WHERE key = 'nomi.orchestrationCollaborators';

DELETE FROM client_preferences
WHERE key IN ('nomi.executionCollaborators', 'nomi.orchestrationCollaborators');

-- IDMM's backup provider is a hard executable binding, not a soft candidate.
-- A dangling legacy preference would make failover fail only at runtime, so
-- reject it before any legacy table is transformed.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM client_preferences preference
    WHERE preference.key = 'idmm_backup_provider_id'
      AND NOT EXISTS (
          SELECT 1 FROM providers provider WHERE provider.id = preference.value
      )
);

-- The current scheduler contract has one shared 128-node DAG ceiling. Abort
-- rather than silently truncating a legacy plan whose behavior cannot be
-- preserved under that invariant.
INSERT INTO _m037_guard
SELECT 0
FROM orch_run_tasks
GROUP BY run_id
HAVING COUNT(*) > 128;

-- JSON is about to be interpreted, so invalid documents are fatal.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM conversations WHERE NOT json_valid(extra)
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM orch_runs
    WHERE NOT json_valid(fleet_snapshot)
       OR json_type(fleet_snapshot) <> 'array'
       OR json_array_length(fleet_snapshot) = 0
);
-- Base snapshots and per-step model overrides become one current Participant
-- set. Apply the same 64-snapshot invariant to legacy data before copying it;
-- no grandfathered oversized execution may enter the unified runtime.
INSERT INTO _m037_guard
SELECT 0
FROM orch_runs run
WHERE json_array_length(run.fleet_snapshot) + (
    SELECT COUNT(*) FROM orch_run_tasks task
    WHERE task.run_id = run.id AND task.override_provider_id IS NOT NULL
) > 64;
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs r, json_each(r.fleet_snapshot) m
    WHERE m.type <> 'object'
       OR trim(COALESCE(json_extract(m.value, '$.id'), '')) = ''
       OR json_type(m.value, '$.id') <> 'text'
       -- Ad-hoc model ranges intentionally persisted an empty Agent id: the
       -- member represented the ordinary Nomi runtime plus a concrete model,
       -- not an unresolved executor. Require the released wire field and type,
       -- then canonicalize that legacy sentinel to `nomi` during the copy.
       OR json_type(m.value, '$.agent_id') IS NOT 'text'
       OR (json_extract(m.value, '$.agent_id') <> ''
           AND trim(json_extract(m.value, '$.agent_id'))
               <> json_extract(m.value, '$.agent_id'))
       OR (json_type(m.value, '$.sort_order') IS NOT NULL
           AND json_type(m.value, '$.sort_order') NOT IN ('integer', 'null'))
       OR (json_type(m.value, '$.preset_revision') IS NOT NULL
           AND json_type(m.value, '$.preset_revision') NOT IN ('integer', 'null'))
       OR (json_type(m.value, '$.capability_profile') IS NOT NULL
           AND json_type(m.value, '$.capability_profile') NOT IN ('object', 'null'))
       OR (json_type(m.value, '$.constraints') IS NOT NULL
           AND json_type(m.value, '$.constraints') NOT IN ('object', 'null'))
       OR (json_type(m.value, '$.enabled_skills') IS NOT NULL
           AND json_type(m.value, '$.enabled_skills') NOT IN ('array', 'null'))
       OR (json_type(m.value, '$.disabled_builtin_skills') IS NOT NULL
           AND json_type(m.value, '$.disabled_builtin_skills') NOT IN ('array', 'null'))
       OR (json_type(m.value, '$.preset_snapshot') IS NOT NULL
           AND json_type(m.value, '$.preset_snapshot') NOT IN ('object', 'null'))
       OR ((json_extract(m.value, '$.provider_id') IS NULL)
           <> (json_extract(m.value, '$.model') IS NULL))
       OR (json_extract(m.value, '$.provider_id') IS NOT NULL
           AND (json_type(m.value, '$.provider_id') <> 'text'
                OR json_type(m.value, '$.model') <> 'text'
                OR trim(json_extract(m.value, '$.provider_id')) = ''
                OR trim(json_extract(m.value, '$.model')) = ''))
);
-- Legacy participant constraints had one vocabulary rename. Validate the
-- complete old shape now so the copy below can produce the strict new DTO
-- without a runtime alias or fail-soft JSON decoder.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs r, json_each(r.fleet_snapshot) member
    WHERE json_type(member.value, '$.constraints') = 'object'
      AND (
        EXISTS (
            SELECT 1 FROM json_each(json_extract(member.value, '$.constraints')) entry
            WHERE entry.key NOT IN ('max_concurrency', 'cost_tier', 'allowed_task_kinds')
        )
        OR (json_type(member.value, '$.constraints.max_concurrency') IS NOT NULL
            AND (json_type(member.value, '$.constraints.max_concurrency') NOT IN ('integer', 'null')
                 OR COALESCE(json_extract(member.value, '$.constraints.max_concurrency'), 1) <= 0))
        OR (json_type(member.value, '$.constraints.cost_tier') IS NOT NULL
            AND (json_type(member.value, '$.constraints.cost_tier') NOT IN ('text', 'null')
                 OR trim(COALESCE(json_extract(member.value, '$.constraints.cost_tier'), 'valid-null')) = ''))
        OR (json_type(member.value, '$.constraints.allowed_task_kinds') IS NOT NULL
            AND json_type(member.value, '$.constraints.allowed_task_kinds') NOT IN ('array', 'null'))
        OR EXISTS (
            SELECT 1
            FROM json_each(json_extract(member.value, '$.constraints.allowed_task_kinds')) kind
            WHERE kind.type <> 'text'
               OR trim(kind.value) = ''
        )
      )
);

-- Every reopenable aggregate must be recoverable immediately after upgrade.
-- Completed and failed Executions can be reopened by retry/adopt, so all
-- statuses except irreversible Cancelled need one concrete provider/model pair
-- backed by a provider that still exists. Deferring this failure until recovery
-- would commit an unreadable live Execution. Cancelled history may retain the
-- old nullable snapshot for faithful audit.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs run, json_each(run.fleet_snapshot) member
    WHERE run.status <> 'cancelled'
      AND (
          typeof(COALESCE(
              NULLIF(trim(json_extract(member.value, '$.provider_id')), ''),
              NULLIF(trim(json_extract(member.value, '$.preset_snapshot.resolved_model.provider_id')), '')
          )) <> 'text'
          OR trim(COALESCE(
              NULLIF(trim(json_extract(member.value, '$.provider_id')), ''),
              NULLIF(trim(json_extract(member.value, '$.preset_snapshot.resolved_model.provider_id')), ''),
              ''
          )) = ''
          OR typeof(COALESCE(
              NULLIF(trim(json_extract(member.value, '$.model')), ''),
              NULLIF(trim(json_extract(member.value, '$.preset_snapshot.resolved_model.model')), '')
          )) <> 'text'
          OR trim(COALESCE(
              NULLIF(trim(json_extract(member.value, '$.model')), ''),
              NULLIF(trim(json_extract(member.value, '$.preset_snapshot.resolved_model.model')), ''),
              ''
          )) = ''
      )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs run, json_each(run.fleet_snapshot) member
    WHERE run.status <> 'cancelled'
      AND NOT EXISTS (
          SELECT 1 FROM providers provider
          WHERE provider.id = COALESCE(
              NULLIF(trim(json_extract(member.value, '$.provider_id')), ''),
              NULLIF(trim(json_extract(member.value, '$.preset_snapshot.resolved_model.provider_id')), '')
          )
      )
);
-- Capability snapshots are decoded by the execution domain after migration.
-- Validate the exact historical wire shape now so a successful migration can
-- never leave an execution that only fails when it is first opened.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs r, json_each(r.fleet_snapshot) member
    WHERE json_type(member.value, '$.capability_profile') = 'object'
      AND (
        EXISTS (
            SELECT 1 FROM json_each(json_extract(member.value, '$.capability_profile')) entry
            WHERE entry.key NOT IN (
                'strengths', 'modalities', 'tools', 'reasoning', 'cost_tier', 'speed_tier'
            )
        )
        OR COALESCE(json_type(member.value, '$.capability_profile.strengths'), 'missing') <> 'array'
        OR COALESCE(json_type(member.value, '$.capability_profile.modalities'), 'missing') <> 'array'
        OR COALESCE(json_type(member.value, '$.capability_profile.tools'), 'missing') NOT IN ('true', 'false')
        OR COALESCE(json_type(member.value, '$.capability_profile.reasoning'), 'missing') <> 'text'
        OR COALESCE(json_type(member.value, '$.capability_profile.cost_tier'), 'missing') <> 'text'
        OR COALESCE(json_type(member.value, '$.capability_profile.speed_tier'), 'missing') <> 'text'
        OR trim(COALESCE(json_extract(member.value, '$.capability_profile.reasoning'), '')) = ''
        OR trim(COALESCE(json_extract(member.value, '$.capability_profile.cost_tier'), '')) = ''
        OR trim(COALESCE(json_extract(member.value, '$.capability_profile.speed_tier'), '')) = ''
        OR EXISTS (
            SELECT 1 FROM json_each(member.value, '$.capability_profile.strengths') value
            WHERE value.type <> 'text'
        )
        OR EXISTS (
            SELECT 1 FROM json_each(member.value, '$.capability_profile.modalities') value
            WHERE value.type <> 'text'
        )
      )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_tasks task
    JOIN orch_runs run ON run.id = task.run_id
    WHERE task.override_provider_id IS NOT NULL
      AND run.status <> 'cancelled'
      AND NOT EXISTS (
          SELECT 1 FROM providers provider
          WHERE provider.id = trim(task.override_provider_id)
      )
);
-- `constraints.cost_tier` duplicated the capability snapshot in the legacy
-- contract. Consolidate it into the one canonical capability field, but reject
-- contradictory values rather than choosing one silently.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs r, json_each(r.fleet_snapshot) member
    WHERE json_type(member.value, '$.constraints.cost_tier') = 'text'
      AND json_type(member.value, '$.capability_profile') = 'object'
      AND json_extract(member.value, '$.constraints.cost_tier')
          <> json_extract(member.value, '$.capability_profile.cost_tier')
);
-- Preset snapshots and skill arrays also cross a typed boundary. A snapshot is
-- either absent or complete and consistent with its participant identity; no
-- guessed preset metadata is manufactured during the hard cut.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs r, json_each(r.fleet_snapshot) member
    WHERE (
        EXISTS (
            SELECT 1 FROM json_each(member.value, '$.enabled_skills') value
            WHERE value.type <> 'text'
        )
        OR EXISTS (
            SELECT 1 FROM json_each(member.value, '$.disabled_builtin_skills') value
            WHERE value.type <> 'text'
        )
        OR (
            json_type(member.value, '$.preset_snapshot') = 'object'
            AND (
                COALESCE(json_type(member.value, '$.preset_snapshot.preset_id'), 'missing') <> 'text'
                OR trim(COALESCE(json_extract(member.value, '$.preset_snapshot.preset_id'), '')) = ''
                OR COALESCE(json_type(member.value, '$.preset_snapshot.preset_revision'), 'missing') <> 'integer'
                OR json_extract(member.value, '$.preset_snapshot.preset_revision') <= 0
                OR COALESCE(json_type(member.value, '$.preset_snapshot.preset_name'), 'missing') <> 'text'
                OR trim(COALESCE(json_extract(member.value, '$.preset_snapshot.preset_name'), '')) = ''
                OR COALESCE(json_type(member.value, '$.preset_snapshot.target'), 'missing') <> 'text'
                OR json_extract(member.value, '$.preset_snapshot.target') NOT IN ('cluster_member', 'execution_step')
                OR json_extract(member.value, '$.preset_id') IS NULL
                OR json_extract(member.value, '$.preset_revision') IS NULL
                OR json_extract(member.value, '$.preset_id')
                    <> json_extract(member.value, '$.preset_snapshot.preset_id')
                OR json_extract(member.value, '$.preset_revision')
                    <> json_extract(member.value, '$.preset_snapshot.preset_revision')
                OR (json_type(member.value, '$.preset_snapshot.instructions') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.instructions') <> 'text')
                OR (json_type(member.value, '$.preset_snapshot.routing_description') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.routing_description') NOT IN ('text', 'null'))
                OR (json_type(member.value, '$.preset_snapshot.resolved_agent_id') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.resolved_agent_id') NOT IN ('text', 'null'))
                OR (json_type(member.value, '$.preset_snapshot.resolved_agent_type') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.resolved_agent_type') NOT IN ('text', 'null'))
                OR (json_type(member.value, '$.preset_snapshot.resolved_agent_backend') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.resolved_agent_backend') NOT IN ('text', 'null'))
                OR (json_type(member.value, '$.preset_snapshot.resolved_model') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.resolved_model') NOT IN ('object', 'null'))
                OR (json_type(member.value, '$.preset_snapshot.resolved_model') = 'object'
                    AND (
                        COALESCE(json_type(member.value, '$.preset_snapshot.resolved_model.model'), 'missing') <> 'text'
                        OR trim(COALESCE(json_extract(member.value, '$.preset_snapshot.resolved_model.model'), '')) = ''
                        OR (json_type(member.value, '$.preset_snapshot.resolved_model.provider_id') IS NOT NULL
                            AND json_type(member.value, '$.preset_snapshot.resolved_model.provider_id') NOT IN ('text', 'null'))
                        OR (json_type(member.value, '$.preset_snapshot.resolved_model.required') IS NOT NULL
                            AND json_type(member.value, '$.preset_snapshot.resolved_model.required') NOT IN ('true', 'false'))
                    ))
                OR (json_type(member.value, '$.preset_snapshot.included_skills') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.included_skills') <> 'array')
                OR EXISTS (
                    SELECT 1 FROM json_each(member.value, '$.preset_snapshot.included_skills') value
                    WHERE value.type <> 'text'
                )
                OR (json_type(member.value, '$.preset_snapshot.excluded_auto_skills') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.excluded_auto_skills') <> 'array')
                OR EXISTS (
                    SELECT 1 FROM json_each(member.value, '$.preset_snapshot.excluded_auto_skills') value
                    WHERE value.type <> 'text'
                )
                OR (json_type(member.value, '$.preset_snapshot.knowledge_policy') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.knowledge_policy') <> 'object')
                OR (json_type(member.value, '$.preset_snapshot.knowledge_base_ids') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.knowledge_base_ids') <> 'array')
                OR EXISTS (
                    SELECT 1 FROM json_each(member.value, '$.preset_snapshot.knowledge_base_ids') value
                    WHERE value.type <> 'text'
                )
                OR (json_type(member.value, '$.preset_snapshot.warnings') IS NOT NULL
                    AND json_type(member.value, '$.preset_snapshot.warnings') <> 'array')
                OR EXISTS (
                    SELECT 1 FROM json_each(member.value, '$.preset_snapshot.warnings') value
                    WHERE value.type <> 'text'
                )
            )
        )
    )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs r, json_each(r.fleet_snapshot) m
    GROUP BY r.id, json_extract(m.value, '$.id')
    HAVING COUNT(*) > 1
);

-- Every legacy enum is mapped explicitly below.  Unknown values are not allowed
-- through the boundary because the new schema has strict CHECK constraints.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM orch_runs
    WHERE autonomy NOT IN ('interactive', 'supervised', 'autonomous')
       OR status NOT IN (
            'planning', 'awaiting_plan_approval', 'running', 'paused',
            'completed', 'completed_with_failures', 'failed', 'cancelled'
       )
       OR COALESCE(approval_mode, 'auto') NOT IN ('auto', 'manual')
       OR max_parallel IS NOT NULL AND (max_parallel <= 0 OR max_parallel > 64)
       OR NOT EXISTS (SELECT 1 FROM users u WHERE u.id = orch_runs.user_id)
       OR (workspace_id IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM orch_workspaces w WHERE w.id = orch_runs.workspace_id
       ))
       OR (workspace_id IS NOT NULL AND EXISTS (
            SELECT 1 FROM orch_workspaces w
            WHERE w.id = orch_runs.workspace_id AND w.user_id <> orch_runs.user_id
       ))
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM orch_workspaces
    WHERE context IS NOT NULL AND NOT json_valid(context)
);

-- Fleet and OrchestrationWorkspace were authoring resources, not runtime
-- state. They are flattened into the one AgentExecutionTemplate configuration
-- aggregate below. Validate every standalone row before interpreting it: a
-- safety backup is rollback protection, not permission to discard live data.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM fleets fleet
    WHERE trim(fleet.id) = ''
       OR trim(fleet.name) = ''
       OR fleet.max_parallel IS NOT NULL AND fleet.max_parallel <= 0
       OR NOT EXISTS (SELECT 1 FROM users owner WHERE owner.id = fleet.user_id)
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM orch_workspaces workspace
    WHERE trim(workspace.id) = ''
       OR trim(workspace.name) = ''
       OR NOT EXISTS (SELECT 1 FROM users owner WHERE owner.id = workspace.user_id)
       OR (workspace.default_fleet_id IS NOT NULL AND NOT EXISTS (
            SELECT 1 FROM fleets fleet WHERE fleet.id = workspace.default_fleet_id
       ))
       OR (workspace.default_fleet_id IS NOT NULL AND EXISTS (
            SELECT 1 FROM fleets fleet
            WHERE fleet.id = workspace.default_fleet_id
              AND fleet.user_id <> workspace.user_id
       ))
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT id FROM fleets
    INTERSECT
    SELECT id FROM orch_workspaces
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM fleet_members member
    WHERE trim(member.id) = ''
       OR trim(member.agent_id) = ''
       OR NOT EXISTS (SELECT 1 FROM fleets fleet WHERE fleet.id = member.fleet_id)
       OR ((member.provider_id IS NULL) <> (member.model IS NULL))
       OR (member.provider_id IS NOT NULL
           AND (trim(member.provider_id) = '' OR trim(member.model) = ''))
       OR ((member.preset_id IS NULL) <> (member.preset_revision IS NULL))
       OR ((member.preset_id IS NULL) <> (member.preset_snapshot IS NULL))
       OR (member.preset_id IS NOT NULL AND trim(member.preset_id) = '')
       OR (member.preset_revision IS NOT NULL AND member.preset_revision <= 0)
       OR CASE WHEN member.preset_snapshot IS NULL THEN 0
               WHEN NOT json_valid(member.preset_snapshot) THEN 1
               ELSE json_type(member.preset_snapshot) <> 'object' END
       OR CASE WHEN member.capability_profile IS NULL THEN 0
               WHEN NOT json_valid(member.capability_profile) THEN 1
               ELSE json_type(member.capability_profile) <> 'object' END
       OR CASE WHEN member.constraints IS NULL THEN 0
               WHEN NOT json_valid(member.constraints) THEN 1
               ELSE json_type(member.constraints) <> 'object' END
);
-- Standalone capability and constraint columns cross the same canonical typed
-- boundary as frozen run snapshots. The old fail-soft CRUD decoder is not a
-- valid migration strategy.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM fleet_members member
    WHERE member.capability_profile IS NOT NULL
      AND json_valid(member.capability_profile)
      AND (
        EXISTS (
            SELECT 1 FROM json_each(member.capability_profile) entry
            WHERE entry.key NOT IN (
                'strengths', 'modalities', 'tools', 'reasoning', 'cost_tier', 'speed_tier'
            )
        )
        OR COALESCE(json_type(member.capability_profile, '$.strengths'), 'missing') <> 'array'
        OR COALESCE(json_type(member.capability_profile, '$.modalities'), 'missing') <> 'array'
        OR COALESCE(json_type(member.capability_profile, '$.tools'), 'missing') NOT IN ('true', 'false')
        OR COALESCE(json_type(member.capability_profile, '$.reasoning'), 'missing') <> 'text'
        OR COALESCE(json_type(member.capability_profile, '$.cost_tier'), 'missing') <> 'text'
        OR COALESCE(json_type(member.capability_profile, '$.speed_tier'), 'missing') <> 'text'
        OR trim(COALESCE(json_extract(member.capability_profile, '$.reasoning'), '')) = ''
        OR trim(COALESCE(json_extract(member.capability_profile, '$.cost_tier'), '')) = ''
        OR trim(COALESCE(json_extract(member.capability_profile, '$.speed_tier'), '')) = ''
        OR EXISTS (
            SELECT 1 FROM json_each(member.capability_profile, '$.strengths') value
            WHERE value.type <> 'text'
        )
        OR EXISTS (
            SELECT 1 FROM json_each(member.capability_profile, '$.modalities') value
            WHERE value.type <> 'text'
        )
      )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM fleet_members member
    WHERE member.constraints IS NOT NULL
      AND json_valid(member.constraints)
      AND (
        EXISTS (
            SELECT 1 FROM json_each(member.constraints) entry
            WHERE entry.key NOT IN ('max_concurrency', 'cost_tier', 'allowed_task_kinds')
        )
        OR (json_type(member.constraints, '$.max_concurrency') IS NOT NULL
            AND (json_type(member.constraints, '$.max_concurrency') NOT IN ('integer', 'null')
                 OR COALESCE(json_extract(member.constraints, '$.max_concurrency'), 1) <= 0))
        OR (json_type(member.constraints, '$.cost_tier') IS NOT NULL
            AND (json_type(member.constraints, '$.cost_tier') NOT IN ('text', 'null')
                 OR trim(COALESCE(json_extract(member.constraints, '$.cost_tier'), 'valid-null')) = ''))
        OR (json_type(member.constraints, '$.allowed_task_kinds') IS NOT NULL
            AND json_type(member.constraints, '$.allowed_task_kinds') NOT IN ('array', 'null'))
        OR EXISTS (
            SELECT 1 FROM json_each(member.constraints, '$.allowed_task_kinds') kind
            WHERE kind.type <> 'text' OR trim(kind.value) = ''
        )
      )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM fleet_members member
    WHERE json_type(member.constraints, '$.cost_tier') = 'text'
      AND member.capability_profile IS NOT NULL
      AND json_extract(member.constraints, '$.cost_tier')
          <> json_extract(member.capability_profile, '$.cost_tier')
);
-- A preset lineage is either absent or complete. Canonical templates only use
-- the execution_step target; cluster_member is accepted here solely so it can
-- be converted deterministically during this hard cut.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM fleet_members member
    WHERE member.preset_snapshot IS NOT NULL
      AND json_valid(member.preset_snapshot)
      AND (
        COALESCE(json_type(member.preset_snapshot, '$.preset_id'), 'missing') <> 'text'
        OR trim(COALESCE(json_extract(member.preset_snapshot, '$.preset_id'), '')) = ''
        OR COALESCE(json_type(member.preset_snapshot, '$.preset_revision'), 'missing') <> 'integer'
        OR json_extract(member.preset_snapshot, '$.preset_revision') <= 0
        OR COALESCE(json_type(member.preset_snapshot, '$.preset_name'), 'missing') <> 'text'
        OR trim(COALESCE(json_extract(member.preset_snapshot, '$.preset_name'), '')) = ''
        OR COALESCE(json_type(member.preset_snapshot, '$.target'), 'missing') <> 'text'
        OR json_extract(member.preset_snapshot, '$.target') NOT IN ('cluster_member', 'execution_step')
        OR member.preset_id <> json_extract(member.preset_snapshot, '$.preset_id')
        OR member.preset_revision <> json_extract(member.preset_snapshot, '$.preset_revision')
        OR (json_type(member.preset_snapshot, '$.instructions') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.instructions') <> 'text')
        OR (json_type(member.preset_snapshot, '$.routing_description') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.routing_description') NOT IN ('text', 'null'))
        OR (json_type(member.preset_snapshot, '$.resolved_agent_id') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.resolved_agent_id') NOT IN ('text', 'null'))
        OR (json_type(member.preset_snapshot, '$.resolved_agent_type') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.resolved_agent_type') NOT IN ('text', 'null'))
        OR (json_type(member.preset_snapshot, '$.resolved_agent_backend') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.resolved_agent_backend') NOT IN ('text', 'null'))
        OR (json_type(member.preset_snapshot, '$.resolved_model') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.resolved_model') NOT IN ('object', 'null'))
        OR (json_type(member.preset_snapshot, '$.resolved_model') = 'object'
            AND (
                COALESCE(json_type(member.preset_snapshot, '$.resolved_model.model'), 'missing') <> 'text'
                OR trim(COALESCE(json_extract(member.preset_snapshot, '$.resolved_model.model'), '')) = ''
                OR (json_type(member.preset_snapshot, '$.resolved_model.provider_id') IS NOT NULL
                    AND json_type(member.preset_snapshot, '$.resolved_model.provider_id') NOT IN ('text', 'null'))
                OR (json_type(member.preset_snapshot, '$.resolved_model.required') IS NOT NULL
                    AND json_type(member.preset_snapshot, '$.resolved_model.required') NOT IN ('true', 'false'))
            ))
        OR (json_type(member.preset_snapshot, '$.included_skills') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.included_skills') <> 'array')
        OR EXISTS (
            SELECT 1 FROM json_each(member.preset_snapshot, '$.included_skills') value
            WHERE value.type <> 'text'
        )
        OR (json_type(member.preset_snapshot, '$.excluded_auto_skills') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.excluded_auto_skills') <> 'array')
        OR EXISTS (
            SELECT 1 FROM json_each(member.preset_snapshot, '$.excluded_auto_skills') value
            WHERE value.type <> 'text'
        )
        OR (json_type(member.preset_snapshot, '$.knowledge_policy') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.knowledge_policy') <> 'object')
        OR (json_type(member.preset_snapshot, '$.knowledge_base_ids') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.knowledge_base_ids') <> 'array')
        OR EXISTS (
            SELECT 1 FROM json_each(member.preset_snapshot, '$.knowledge_base_ids') value
            WHERE value.type <> 'text'
        )
        OR (json_type(member.preset_snapshot, '$.warnings') IS NOT NULL
            AND json_type(member.preset_snapshot, '$.warnings') <> 'array')
        OR EXISTS (
            SELECT 1 FROM json_each(member.preset_snapshot, '$.warnings') value
            WHERE value.type <> 'text'
        )
      )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM orch_run_tasks
    WHERE status NOT IN ('pending', 'running', 'needs_review', 'done', 'failed', 'skipped', 'cancelled')
       OR kind NOT IN ('agent', 'synthesis', 'verify', 'judge', 'loop')
       OR (task_profile IS NOT NULL AND CASE
            WHEN NOT json_valid(task_profile) THEN 1
            ELSE json_type(task_profile) <> 'object'
       END)
       OR (pattern_config IS NOT NULL AND CASE
            WHEN NOT json_valid(pattern_config) THEN 1
            ELSE json_type(pattern_config) <> 'object'
       END)
       OR (output_files IS NOT NULL AND CASE
            WHEN NOT json_valid(output_files) THEN 1
            ELSE json_type(output_files) <> 'array'
       END)
       OR ((override_provider_id IS NULL) <> (override_model IS NULL))
       OR (override_provider_id IS NOT NULL
           AND (trim(override_provider_id) = '' OR trim(override_model) = ''))
       OR (kind IN ('verify', 'judge', 'loop') AND override_provider_id IS NOT NULL)
       OR COALESCE(on_fail, 'fail_run') NOT IN ('fail_run', 'skip_and_continue')
       OR typeof(attempt) <> 'integer'
       OR attempt < 0
       -- The released retry worker could settle a task as failed without
       -- clearing its last scheduled deadline. Preserve that deadline on the
       -- immutable historical Attempt; only pending Steps retain a live gate.
       OR (next_retry_at IS NOT NULL
           AND (typeof(next_retry_at) <> 'integer' OR next_retry_at < 0))
       OR (status = 'needs_review' AND trim(COALESCE(pending_question, '')) = '')
       OR (status <> 'needs_review' AND pending_question IS NOT NULL)
       OR NOT EXISTS (SELECT 1 FROM orch_runs r WHERE r.id = orch_run_tasks.run_id)
);

-- The former pattern_config bag is split into typed columns.  Reject values
-- which cannot be carried without loss, then map every accepted legacy shape.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_tasks t, json_each(t.pattern_config) entry
    WHERE t.pattern_config IS NOT NULL
      AND (
        (t.kind IN ('agent', 'synthesis')
         AND entry.key NOT IN ('group', 'delegation_depth', 'loop_prior_output', 'loop_iteration'))
        OR (t.kind = 'verify' AND entry.key NOT IN ('vote', 'delegation_depth'))
        OR (t.kind = 'judge' AND entry.key NOT IN ('aggregate', 'candidates', 'delegation_depth'))
        OR (t.kind = 'loop' AND entry.key NOT IN ('max_iter', 'stop', 'delegation_depth'))
      )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM orch_run_tasks t
    WHERE (json_type(t.pattern_config, '$.group') IS NOT NULL
           AND (json_type(t.pattern_config, '$.group') <> 'text'
                OR trim(json_extract(t.pattern_config, '$.group')) = ''))
       OR (json_type(t.pattern_config, '$.delegation_depth') IS NOT NULL
           AND (json_type(t.pattern_config, '$.delegation_depth') <> 'integer'
                OR json_extract(t.pattern_config, '$.delegation_depth') < 0
                OR json_extract(t.pattern_config, '$.delegation_depth') > 4))
       OR (json_type(t.pattern_config, '$.loop_prior_output') IS NOT NULL
           AND json_type(t.pattern_config, '$.loop_prior_output') <> 'text')
       OR (json_type(t.pattern_config, '$.loop_iteration') IS NOT NULL
           AND (json_type(t.pattern_config, '$.loop_iteration') <> 'integer'
                OR json_extract(t.pattern_config, '$.loop_iteration') < 0))
       OR (t.kind = 'verify' AND json_type(t.pattern_config, '$.vote') IS NOT NULL
           AND json_type(t.pattern_config, '$.vote') NOT IN ('text', 'object'))
       OR (t.kind = 'verify' AND json_type(t.pattern_config, '$.vote') IS NOT NULL AND NOT (
            json_extract(t.pattern_config, '$.vote') IN ('majority', 'unanimous')
            OR (json_type(t.pattern_config, '$.vote') = 'object'
                AND json_type(t.pattern_config, '$.vote.threshold') = 'integer'
                AND json_extract(t.pattern_config, '$.vote.threshold') > 0)
       ))
       OR (t.kind = 'judge' AND json_type(t.pattern_config, '$.aggregate') IS NOT NULL
           AND (json_type(t.pattern_config, '$.aggregate') <> 'text'
                OR json_extract(t.pattern_config, '$.aggregate') NOT IN ('mean', 'borda')))
       OR (t.kind = 'judge' AND json_type(t.pattern_config, '$.candidates') IS NOT NULL
           AND (json_type(t.pattern_config, '$.candidates') <> 'integer'
                OR json_extract(t.pattern_config, '$.candidates') <= 0))
       OR (t.kind = 'loop' AND json_type(t.pattern_config, '$.max_iter') IS NOT NULL
           AND (json_type(t.pattern_config, '$.max_iter') <> 'integer'
                OR json_extract(t.pattern_config, '$.max_iter') <= 0))
       OR (t.kind = 'loop' AND json_type(t.pattern_config, '$.stop') IS NOT NULL
           AND json_type(t.pattern_config, '$.stop') <> 'object')
       OR (t.kind = 'loop' AND json_type(t.pattern_config, '$.stop.kind') IS NOT NULL
           AND (json_type(t.pattern_config, '$.stop.kind') <> 'text'
                OR json_extract(t.pattern_config, '$.stop.kind') NOT IN (
                    'max_iter', 'predicate', 'dry', 'approved', 'verdict', 'verify'
                )))
       OR (t.kind = 'loop'
           AND json_extract(t.pattern_config, '$.stop.kind') = 'predicate'
           AND (json_type(t.pattern_config, '$.stop.done_marker') <> 'text'
                OR trim(json_extract(t.pattern_config, '$.stop.done_marker')) = ''))
       OR (t.kind = 'loop'
           AND json_extract(t.pattern_config, '$.stop.kind') = 'dry'
           AND json_type(t.pattern_config, '$.stop.quiet_rounds') IS NOT NULL
           AND (json_type(t.pattern_config, '$.stop.quiet_rounds') <> 'integer'
                OR json_extract(t.pattern_config, '$.stop.quiet_rounds') <= 0))
       OR (t.kind = 'loop'
           AND COALESCE(json_extract(t.pattern_config, '$.stop.kind'), '') <> 'predicate'
           AND json_type(t.pattern_config, '$.stop.done_marker') IS NOT NULL)
       OR (t.kind = 'loop'
           AND COALESCE(json_extract(t.pattern_config, '$.stop.kind'), '') <> 'dry'
           AND json_type(t.pattern_config, '$.stop.quiet_rounds') IS NOT NULL)
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_tasks t, json_each(json_extract(t.pattern_config, '$.vote')) entry
    WHERE t.kind = 'verify'
      AND json_type(t.pattern_config, '$.vote') = 'object'
      AND entry.key NOT IN ('threshold')
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_tasks t, json_each(json_extract(t.pattern_config, '$.stop')) entry
    WHERE t.kind = 'loop'
      AND json_type(t.pattern_config, '$.stop') = 'object'
      AND entry.key NOT IN ('kind', 'done_marker', 'quiet_rounds')
);

-- Assignment is now a property of a step, therefore at most one legacy row may
-- exist and it must point to a member of the same immutable execution snapshot.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM orch_assignments GROUP BY task_id HAVING COUNT(*) > 1
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_assignments a
    LEFT JOIN orch_run_tasks t ON t.id = a.task_id
    LEFT JOIN orch_runs r ON r.id = t.run_id
    WHERE t.id IS NULL OR r.id IS NULL
       OR a.source NOT IN ('auto', 'override', 'automatic', 'manual')
       OR a.locked NOT IN (0, 1)
       OR NOT EXISTS (
            SELECT 1 FROM json_each(r.fleet_snapshot) m
            WHERE json_extract(m.value, '$.id') = a.member_id
       )
);
-- The released planner could write a routing assignment for every node even
-- though verify/judge/loop were settled synchronously and never dispatched to
-- that member. The unified model correctly keeps these as participant-free
-- Engine steps; their unused legacy assignment is retained in the Migrated
-- event below instead of being revived as executable authority.

-- The old schema could express a dependency across runs because the edge lacked
-- run_id.  Such an edge has no valid interpretation in one execution aggregate.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_task_deps d
    LEFT JOIN orch_run_tasks blocker ON blocker.id = d.blocker_task_id
    LEFT JOIN orch_run_tasks blocked ON blocked.id = d.blocked_task_id
    WHERE blocker.id IS NULL OR blocked.id IS NULL OR blocker.run_id <> blocked.run_id
);
INSERT INTO _m037_guard
WITH RECURSIVE reachable(execution_id, origin_step_id, current_step_id) AS (
    SELECT blocker.run_id, d.blocker_task_id, d.blocked_task_id
    FROM orch_run_task_deps d
    JOIN orch_run_tasks blocker ON blocker.id = d.blocker_task_id
    UNION
    SELECT reachable.execution_id, reachable.origin_step_id, d.blocked_task_id
    FROM reachable
    JOIN orch_run_task_deps d ON d.blocker_task_id = reachable.current_step_id
)
SELECT 0 FROM reachable WHERE origin_step_id = current_step_id LIMIT 1;
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_tasks task
    WHERE (
        task.kind IN ('verify', 'judge')
        AND (SELECT COUNT(*) FROM orch_run_task_deps dependency
             WHERE dependency.blocked_task_id = task.id) = 0
    ) OR (
        task.kind = 'loop'
        AND (SELECT COUNT(*) FROM orch_run_task_deps dependency
             WHERE dependency.blocked_task_id = task.id) <> 1
    ) OR (
        task.kind = 'verify'
        AND json_type(task.pattern_config, '$.vote') = 'object'
        AND json_extract(task.pattern_config, '$.vote.threshold') > (
            SELECT COUNT(*) FROM orch_run_task_deps dependency
            WHERE dependency.blocked_task_id = task.id
        )
    )
);

-- Validate the three conversation preferences before reading them.  Historical
-- model range objects are normalized into a JSON array; malformed variants abort.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM conversations
    WHERE (json_type(extra, '$.agent_cluster_mode') IS NOT NULL
           AND json_type(extra, '$.agent_cluster_mode') NOT IN ('true', 'false'))
       OR COALESCE(json_extract(extra, '$.orchestrator_approval_mode'), 'auto') NOT IN ('auto', 'manual')
       OR CASE
            WHEN json_type(extra, '$.orchestrator_model_range') IS NULL THEN 0
            WHEN json_type(extra, '$.orchestrator_model_range') = 'array' THEN 0
            WHEN json_type(extra, '$.orchestrator_model_range') <> 'object' THEN 1
            WHEN json_extract(extra, '$.orchestrator_model_range.mode') = 'auto' THEN 0
            WHEN json_extract(extra, '$.orchestrator_model_range.mode') = 'range'
                THEN json_type(extra, '$.orchestrator_model_range.models') <> 'array'
            WHEN json_extract(extra, '$.orchestrator_model_range.mode') = 'single'
                THEN NOT (
                    json_type(extra, '$.orchestrator_model_range.model') = 'object'
                    AND json_type(extra, '$.orchestrator_model_range.model.provider_id') = 'text'
                    AND trim(json_extract(extra, '$.orchestrator_model_range.model.provider_id')) <> ''
                    AND json_type(extra, '$.orchestrator_model_range.model.model') = 'text'
                    AND trim(json_extract(extra, '$.orchestrator_model_range.model.model')) <> ''
                )
            ELSE 1
       END
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM conversations c,
         json_each(CASE
             WHEN json_type(c.extra, '$.orchestrator_model_range') = 'array'
                 THEN json_extract(c.extra, '$.orchestrator_model_range')
             WHEN json_extract(c.extra, '$.orchestrator_model_range.mode') = 'range'
                 THEN json_extract(c.extra, '$.orchestrator_model_range.models')
             ELSE '[]'
         END) model_ref
    WHERE model_ref.type <> 'object'
       OR json_type(model_ref.value, '$.provider_id') <> 'text'
       OR trim(json_extract(model_ref.value, '$.provider_id')) = ''
       OR json_type(model_ref.value, '$.model') <> 'text'
       OR trim(json_extract(model_ref.value, '$.model')) = ''
);

-- Conversation identity can be represented by a column, legacy extra JSON, or
-- both. A terminal legacy Execution may outlive its deleted lead Conversation;
-- retain that dangling historical id in the Migrated event and create Links
-- only for Conversations that still exist. Attempt identities are stricter
-- because their transcript is part of the execution audit record.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs execution
    WHERE execution.lead_conv_id IS NOT NULL
      AND execution.status NOT IN (
          'completed', 'completed_with_failures', 'failed', 'cancelled'
      )
      AND NOT EXISTS (
          SELECT 1 FROM conversations conversation
          WHERE conversation.id = execution.lead_conv_id
      )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM orch_run_tasks t
    WHERE t.conversation_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM conversations c WHERE c.id = t.conversation_id)
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM conversations c
    WHERE (json_type(c.extra, '$.orchestrator_run_id') IS NOT NULL
           AND json_type(c.extra, '$.orchestrator_run_id') <> 'text')
       OR (json_type(c.extra, '$.orchestrator_task_id') IS NOT NULL
           AND json_type(c.extra, '$.orchestrator_task_id') <> 'text')
       OR (
            json_extract(c.extra, '$.orchestrator_run_id') IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM orch_runs r
                WHERE r.id = json_extract(c.extra, '$.orchestrator_run_id')
            )
       )
       OR (
            json_extract(c.extra, '$.orchestrator_task_id') IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM orch_run_tasks t
                WHERE t.id = json_extract(c.extra, '$.orchestrator_task_id')
            )
       )
       OR (
            json_extract(c.extra, '$.orchestrator_task_id') IS NOT NULL
            AND json_extract(c.extra, '$.orchestrator_run_id') IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM orch_run_tasks t
                WHERE t.id = json_extract(c.extra, '$.orchestrator_task_id')
                  AND t.run_id = json_extract(c.extra, '$.orchestrator_run_id')
            )
       )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_tasks t
    JOIN conversations c ON c.id = t.conversation_id
    WHERE json_extract(c.extra, '$.orchestrator_task_id') IS NOT NULL
      AND json_extract(c.extra, '$.orchestrator_task_id') <> t.id
);
INSERT INTO _m037_guard
WITH step_link_candidates AS (
    SELECT id AS step_id, conversation_id
    FROM orch_run_tasks WHERE conversation_id IS NOT NULL
    UNION
    SELECT json_extract(c.extra, '$.orchestrator_task_id'), c.id
    FROM conversations c
    WHERE json_extract(c.extra, '$.orchestrator_task_id') IS NOT NULL
), candidate_counts AS (
    SELECT task.id AS step_id,
           task.status,
           task.attempt,
           task.conversation_id AS configured_conversation_id,
           COUNT(DISTINCT candidate.conversation_id) AS candidate_count
    FROM orch_run_tasks task
    JOIN step_link_candidates candidate ON candidate.step_id = task.id
    GROUP BY task.id, task.status, task.attempt, task.conversation_id
)
SELECT 0
FROM candidate_counts candidate
WHERE candidate.candidate_count - CASE
          -- A pending row represents a generation which has not started yet.
          -- Any retained Conversation therefore belongs strictly before it.
          WHEN candidate.status = 'pending' THEN 0
          -- WaitingInput must have a current transcript; for all other settled
          -- or in-flight states a complete 0..attempt candidate set proves that
          -- the newest candidate is current.  The legacy column is only a
          -- fallback because its detached best-effort writer could lag behind.
          WHEN candidate.status = 'needs_review' THEN 1
          WHEN candidate.candidate_count = candidate.attempt + 1 THEN 1
          WHEN candidate.configured_conversation_id IS NOT NULL THEN 1
          ELSE 0
      END > candidate.attempt;
-- When cardinality cannot prove a current transcript, the best-effort task
-- column is usable only if it is also the newest surviving candidate. Choosing
-- an older configured row would place a newer transcript into an earlier
-- Attempt and manufacture generation time travel. That shape is genuinely
-- ambiguous, so fail the atomic hard cut rather than guess.
INSERT INTO _m037_guard
WITH conversation_candidates AS (
    SELECT task.id AS step_id,
           task.status,
           task.attempt,
           task.conversation_id AS configured_conversation_id,
           conversation.id AS conversation_id,
           conversation.created_at
    FROM orch_run_tasks task
    JOIN conversations conversation ON conversation.id = task.conversation_id
    UNION
    SELECT task.id,
           task.status,
           task.attempt,
           task.conversation_id,
           conversation.id,
           conversation.created_at
    FROM conversations conversation
    JOIN orch_run_tasks task
      ON task.id = json_extract(conversation.extra, '$.orchestrator_task_id')
    WHERE json_extract(conversation.extra, '$.orchestrator_task_id') IS NOT NULL
), candidate_facts AS (
    SELECT candidate.*,
           COUNT(*) OVER (PARTITION BY candidate.step_id) AS candidate_count,
           FIRST_VALUE(candidate.conversation_id) OVER (
               PARTITION BY candidate.step_id
               ORDER BY candidate.created_at DESC, candidate.conversation_id DESC
           ) AS newest_conversation_id
    FROM conversation_candidates candidate
)
SELECT 0
FROM candidate_facts candidate
WHERE candidate.status NOT IN ('pending', 'needs_review')
  AND candidate.candidate_count <> candidate.attempt + 1
  AND candidate.configured_conversation_id IS NOT NULL
  AND candidate.configured_conversation_id <> candidate.newest_conversation_id
LIMIT 1;
INSERT INTO _m037_guard
WITH step_link_candidates AS (
    SELECT id AS step_id, conversation_id
    FROM orch_run_tasks WHERE conversation_id IS NOT NULL
    UNION
    SELECT json_extract(c.extra, '$.orchestrator_task_id'), c.id
    FROM conversations c
    WHERE json_extract(c.extra, '$.orchestrator_task_id') IS NOT NULL
)
SELECT 0 FROM step_link_candidates
GROUP BY conversation_id HAVING COUNT(DISTINCT step_id) > 1;
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1 FROM orch_run_tasks
    WHERE conversation_id IS NOT NULL
    GROUP BY conversation_id HAVING COUNT(*) > 1
);
-- Multiple Conversations for one Step are released retry transcripts. They
-- become distinct immutable Attempts below; only one Conversation being
-- claimed by different Steps remains ambiguous and is rejected above.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_tasks task
    WHERE task.status = 'needs_review'
      AND task.conversation_id IS NULL
      AND NOT EXISTS (
          SELECT 1 FROM conversations conversation
          WHERE json_extract(conversation.extra, '$.orchestrator_task_id') = task.id
      )
);
INSERT INTO _m037_guard
WITH lead_candidates AS (
    SELECT run.id AS execution_id, run.lead_conv_id AS conversation_id
    FROM orch_runs run
    JOIN conversations conversation ON conversation.id = run.lead_conv_id
    UNION
    SELECT json_extract(c.extra, '$.orchestrator_run_id'), c.id
    FROM conversations c
    WHERE json_extract(c.extra, '$.orchestrator_run_id') IS NOT NULL
      AND json_extract(c.extra, '$.orchestrator_task_id') IS NULL
)
SELECT 0 FROM lead_candidates
GROUP BY execution_id HAVING COUNT(DISTINCT conversation_id) > 1;
INSERT INTO _m037_guard
WITH all_links AS (
    SELECT r.id AS execution_id, r.user_id, r.lead_conv_id AS conversation_id
    FROM orch_runs r WHERE r.lead_conv_id IS NOT NULL
    UNION
    SELECT r.id, r.user_id, c.id
    FROM conversations c
    JOIN orch_runs r ON r.id = json_extract(c.extra, '$.orchestrator_run_id')
    WHERE json_extract(c.extra, '$.orchestrator_task_id') IS NULL
    UNION
    SELECT t.run_id, r.user_id, t.conversation_id
    FROM orch_run_tasks t JOIN orch_runs r ON r.id = t.run_id
    WHERE t.conversation_id IS NOT NULL
    UNION
    SELECT t.run_id, r.user_id, c.id
    FROM conversations c
    JOIN orch_run_tasks t ON t.id = json_extract(c.extra, '$.orchestrator_task_id')
    JOIN orch_runs r ON r.id = t.run_id
)
SELECT 0
FROM all_links l
JOIN conversations c ON c.id = l.conversation_id
WHERE c.user_id <> l.user_id
LIMIT 1;

-- Materialize every surviving retry transcript before the old identity fields
-- disappear. A pending task's current generation is unstarted and therefore has
-- no current Conversation. For other states, WaitingInput or a complete
-- candidate cardinality proves that the newest transcript is current; only when
-- that evidence is absent may the best-effort task column select current.
-- Historical candidates are right-aligned immediately before the persisted
-- current attempt. This preserves visible leading gaps instead of pretending
-- that a partial surviving transcript set started at generation zero.
CREATE TABLE _m037_attempt_conversation_stage (
    execution_id   TEXT NOT NULL,
    step_id        TEXT NOT NULL,
    conversation_id INTEGER NOT NULL,
    attempt_no     INTEGER NOT NULL CHECK (attempt_no >= 0),
    is_current     INTEGER NOT NULL CHECK (is_current IN (0, 1)),
    current_source TEXT NOT NULL CHECK (current_source IN (
        'none', 'configured', 'needs_review_latest', 'exact_cardinality_latest'
    )),
    candidate_ordinal INTEGER NOT NULL CHECK (candidate_ordinal >= 0),
    candidate_count INTEGER NOT NULL CHECK (candidate_count > 0),
    historical_count INTEGER NOT NULL CHECK (historical_count >= 0),
    legacy_status  TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (step_id, conversation_id),
    UNIQUE (step_id, attempt_no)
);
WITH conversation_candidates AS (
    SELECT task.run_id AS execution_id,
           task.id AS step_id,
           task.attempt AS current_attempt_no,
           task.conversation_id AS configured_current_conversation_id,
           task.status AS legacy_status,
           conversation.id AS conversation_id,
           conversation.created_at,
           conversation.updated_at
    FROM orch_run_tasks task
    JOIN conversations conversation ON conversation.id = task.conversation_id
    UNION
    SELECT task.run_id,
           task.id,
           task.attempt,
           task.conversation_id,
           task.status,
           conversation.id,
           conversation.created_at,
           conversation.updated_at
    FROM conversations conversation
    JOIN orch_run_tasks task
      ON task.id = json_extract(conversation.extra, '$.orchestrator_task_id')
    WHERE json_extract(conversation.extra, '$.orchestrator_task_id') IS NOT NULL
), candidate_facts AS (
    SELECT candidate.*,
           COUNT(*) OVER (PARTITION BY candidate.step_id) AS candidate_count,
           ROW_NUMBER() OVER (
               PARTITION BY candidate.step_id
               ORDER BY candidate.created_at, candidate.conversation_id
           ) - 1 AS candidate_ordinal,
           FIRST_VALUE(candidate.conversation_id) OVER (
               PARTITION BY candidate.step_id
               ORDER BY candidate.created_at DESC, candidate.conversation_id DESC
           ) AS newest_conversation_id
    FROM conversation_candidates candidate
), chosen_current AS (
    SELECT candidate.*,
           CASE
               WHEN candidate.legacy_status = 'pending' THEN NULL
               WHEN candidate.legacy_status = 'needs_review'
                   THEN candidate.newest_conversation_id
               WHEN candidate.candidate_count = candidate.current_attempt_no + 1
                   THEN candidate.newest_conversation_id
               WHEN candidate.configured_current_conversation_id IS NOT NULL
                   THEN candidate.configured_current_conversation_id
               ELSE NULL
           END AS current_conversation_id,
           CASE
               WHEN candidate.legacy_status = 'pending' THEN 'none'
               WHEN candidate.legacy_status = 'needs_review' THEN 'needs_review_latest'
               WHEN candidate.candidate_count = candidate.current_attempt_no + 1
                   THEN 'exact_cardinality_latest'
               WHEN candidate.configured_current_conversation_id IS NOT NULL
                   THEN 'configured'
               ELSE 'none'
           END AS current_source
    FROM candidate_facts candidate
), classified AS (
    SELECT candidate.*,
           CASE
               WHEN candidate.current_conversation_id IS NOT NULL
                AND candidate.conversation_id = candidate.current_conversation_id
               THEN 1 ELSE 0
           END AS is_current,
           candidate.candidate_count - CASE
               WHEN candidate.current_conversation_id IS NOT NULL THEN 1 ELSE 0
           END AS historical_count
    FROM chosen_current candidate
),
ranked AS (
    SELECT candidate.*,
           ROW_NUMBER() OVER (
               PARTITION BY candidate.step_id
               ORDER BY
                   candidate.is_current,
                   candidate.created_at,
                   candidate.conversation_id
           ) - 1 AS historical_attempt_no
    FROM classified candidate
)
INSERT INTO _m037_attempt_conversation_stage (
    execution_id, step_id, conversation_id, attempt_no, is_current, current_source,
    candidate_ordinal, candidate_count, historical_count,
    legacy_status, created_at, updated_at
)
SELECT execution_id,
       step_id,
       conversation_id,
       CASE WHEN is_current = 1
            THEN current_attempt_no
            ELSE current_attempt_no - historical_count + historical_attempt_no
       END,
       is_current,
       current_source,
       candidate_ordinal,
       candidate_count,
       historical_count,
       legacy_status,
       created_at,
       updated_at
FROM ranked;

-- The stage must be an exact lossless projection of source candidates, and its
-- classification/numbering must be independently reproducible from source
-- state. These checks deliberately run before any source identity is removed.
INSERT INTO _m037_guard
WITH source_candidates AS (
    SELECT task.run_id AS execution_id,
           task.id AS step_id,
           conversation.id AS conversation_id,
           task.status AS legacy_status,
           conversation.created_at,
           conversation.updated_at
    FROM orch_run_tasks task
    JOIN conversations conversation ON conversation.id = task.conversation_id
    UNION
    SELECT task.run_id,
           task.id,
           conversation.id,
           task.status,
           conversation.created_at,
           conversation.updated_at
    FROM conversations conversation
    JOIN orch_run_tasks task
      ON task.id = json_extract(conversation.extra, '$.orchestrator_task_id')
    WHERE json_extract(conversation.extra, '$.orchestrator_task_id') IS NOT NULL
), staged_candidates AS (
    SELECT execution_id, step_id, conversation_id, legacy_status, created_at, updated_at
    FROM _m037_attempt_conversation_stage
)
SELECT 0 WHERE EXISTS (
    SELECT * FROM source_candidates
    EXCEPT
    SELECT * FROM staged_candidates
) OR EXISTS (
    SELECT * FROM staged_candidates
    EXCEPT
    SELECT * FROM source_candidates
);
INSERT INTO _m037_guard
WITH staged_facts AS (
    SELECT stage.*,
           task.attempt AS current_attempt_no,
           task.status AS source_status,
           task.conversation_id AS configured_conversation_id,
           COUNT(*) OVER (PARTITION BY stage.step_id) AS source_candidate_count,
           ROW_NUMBER() OVER (
               PARTITION BY stage.step_id
               ORDER BY stage.created_at, stage.conversation_id
           ) - 1 AS source_candidate_ordinal,
           FIRST_VALUE(stage.conversation_id) OVER (
               PARTITION BY stage.step_id
               ORDER BY stage.created_at DESC, stage.conversation_id DESC
           ) AS newest_conversation_id
    FROM _m037_attempt_conversation_stage stage
    JOIN orch_run_tasks task ON task.id = stage.step_id
), expected_classification AS (
    SELECT fact.*,
           CASE
               WHEN fact.source_status = 'pending' THEN NULL
               WHEN fact.source_status = 'needs_review' THEN fact.newest_conversation_id
               WHEN fact.source_candidate_count = fact.current_attempt_no + 1
                   THEN fact.newest_conversation_id
               WHEN fact.configured_conversation_id IS NOT NULL
                   THEN fact.configured_conversation_id
               ELSE NULL
           END AS expected_current_conversation_id,
           CASE
               WHEN fact.source_status = 'pending' THEN 'none'
               WHEN fact.source_status = 'needs_review' THEN 'needs_review_latest'
               WHEN fact.source_candidate_count = fact.current_attempt_no + 1
                   THEN 'exact_cardinality_latest'
               WHEN fact.configured_conversation_id IS NOT NULL THEN 'configured'
               ELSE 'none'
           END AS expected_current_source
    FROM staged_facts fact
), expected_numbering_basis AS (
    SELECT expected.*,
           CASE
               WHEN expected.expected_current_conversation_id IS NOT NULL
                AND expected.conversation_id = expected.expected_current_conversation_id
               THEN 1 ELSE 0
           END AS expected_is_current,
           expected.source_candidate_count - CASE
               WHEN expected.expected_current_conversation_id IS NOT NULL THEN 1 ELSE 0
           END AS expected_historical_count
    FROM expected_classification expected
), expected_numbering AS (
    SELECT expected.*,
           ROW_NUMBER() OVER (
               PARTITION BY expected.step_id
               ORDER BY
                   expected.expected_is_current,
                   expected.created_at,
                   expected.conversation_id
           ) - 1 AS expected_historical_rank
    FROM expected_numbering_basis expected
)
SELECT 0
FROM expected_numbering expected
WHERE expected.candidate_count IS NOT expected.source_candidate_count
   OR expected.candidate_ordinal IS NOT expected.source_candidate_ordinal
   OR expected.current_source IS NOT expected.expected_current_source
   OR expected.is_current IS NOT expected.expected_is_current
   OR expected.historical_count IS NOT expected.expected_historical_count
   OR expected.attempt_no IS NOT CASE
          WHEN expected.expected_is_current = 1 THEN expected.current_attempt_no
          ELSE expected.current_attempt_no
               - expected.expected_historical_count
               + expected.expected_historical_rank
      END
LIMIT 1;
INSERT INTO _m037_guard
SELECT 0
FROM _m037_attempt_conversation_stage stage
JOIN orch_run_tasks task ON task.id = stage.step_id
GROUP BY stage.step_id, task.attempt
HAVING SUM(stage.is_current) > 1
    OR SUM(CASE WHEN stage.is_current = 0 THEN 1 ELSE 0 END) > task.attempt
    OR SUM(CASE WHEN stage.is_current = 1 AND stage.attempt_no <> task.attempt
                THEN 1 ELSE 0 END) > 0
    OR (
        SUM(CASE WHEN stage.is_current = 0 THEN 1 ELSE 0 END) > 0
        AND (
            MIN(CASE WHEN stage.is_current = 0 THEN stage.attempt_no END)
                <> task.attempt - SUM(CASE WHEN stage.is_current = 0 THEN 1 ELSE 0 END)
            OR MAX(CASE WHEN stage.is_current = 0 THEN stage.attempt_no END)
                <> task.attempt - 1
        )
    );

-- One live Agent conversation may represent exactly one active Attempt.
-- Competing Attempt ownership would make decision routing ambiguous.
INSERT INTO _m037_guard
SELECT 0
FROM _m037_attempt_conversation_stage
WHERE is_current = 1 AND legacy_status IN ('running', 'needs_review')
GROUP BY conversation_id
HAVING COUNT(*) > 1;

-- A legacy WaitingInput Attempt remains active after the cut. It therefore
-- cannot share its Conversation with any Execution lead: the unified actor
-- boundary requires exactly one active relation and delegation now extends
-- the current Execution instead of opening a child run. Legacy Running
-- attempts are interrupted/inactivated below, so they do not participate in
-- this preflight.
INSERT INTO _m037_guard
WITH lead_candidates AS (
    SELECT r.id AS execution_id, r.lead_conv_id AS conversation_id
    FROM orch_runs r
    JOIN conversations conversation ON conversation.id = r.lead_conv_id
    UNION
    SELECT json_extract(c.extra, '$.orchestrator_run_id'), c.id
    FROM conversations c
    WHERE json_extract(c.extra, '$.orchestrator_run_id') IS NOT NULL
      AND json_extract(c.extra, '$.orchestrator_task_id') IS NULL
)
SELECT 0
FROM _m037_attempt_conversation_stage attempt
JOIN lead_candidates lead ON lead.conversation_id = attempt.conversation_id
WHERE attempt.is_current = 1 AND attempt.legacy_status = 'needs_review'
LIMIT 1;

-- Runtime execution inheritance is removed, but migration must not reset the
-- recursion budget of an already-forked legacy run. Validate the old forest,
-- resolve the unique parent task whose Conversation became each child lead,
-- and reduce the lineage to one private effective depth per legacy run. Only
-- this staging fact survives in migrated Step rows; no parent relation enters
-- the new runtime schema.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs child
    WHERE child.forked_from IS NOT NULL
      AND NOT EXISTS (
          SELECT 1 FROM orch_runs parent
          WHERE parent.id = child.forked_from
            AND parent.user_id = child.user_id
      )
);
INSERT INTO _m037_guard
WITH RECURSIVE lineage(origin_run_id, current_run_id, path, cycle) AS (
    SELECT run.id, run.id, ',' || run.id || ',', 0
    FROM orch_runs run
    UNION ALL
    SELECT lineage.origin_run_id,
           parent.id,
           lineage.path || parent.id || ',',
           instr(lineage.path, ',' || parent.id || ',') > 0
    FROM lineage
    JOIN orch_runs current ON current.id = lineage.current_run_id
    JOIN orch_runs parent ON parent.id = current.forked_from
    WHERE lineage.cycle = 0
)
SELECT 0 FROM lineage WHERE cycle = 1 LIMIT 1;

INSERT INTO _m037_guard
WITH step_link_candidates AS (
    SELECT task.id AS step_id, task.conversation_id
    FROM orch_run_tasks task WHERE task.conversation_id IS NOT NULL
    UNION
    SELECT json_extract(conversation.extra, '$.orchestrator_task_id'), conversation.id
    FROM conversations conversation
    WHERE json_extract(conversation.extra, '$.orchestrator_task_id') IS NOT NULL
),
lead_candidates AS (
    SELECT run.id AS run_id, run.lead_conv_id AS conversation_id
    FROM orch_runs run
    JOIN conversations conversation ON conversation.id = run.lead_conv_id
    UNION
    SELECT json_extract(conversation.extra, '$.orchestrator_run_id'), conversation.id
    FROM conversations conversation
    WHERE json_extract(conversation.extra, '$.orchestrator_run_id') IS NOT NULL
      AND json_extract(conversation.extra, '$.orchestrator_task_id') IS NULL
),
fork_origin_candidates AS (
    SELECT child.id AS child_run_id, task.id AS origin_step_id
    FROM orch_runs child
    JOIN lead_candidates child_lead ON child_lead.run_id = child.id
    JOIN orch_run_tasks task ON task.run_id = child.forked_from
    JOIN step_link_candidates link ON link.step_id = task.id
                                      AND link.conversation_id = child_lead.conversation_id
    WHERE child.forked_from IS NOT NULL
)
SELECT 0
FROM orch_runs child
LEFT JOIN fork_origin_candidates origin ON origin.child_run_id = child.id
WHERE child.forked_from IS NOT NULL
GROUP BY child.id
HAVING COUNT(origin.origin_step_id) <> 1;

CREATE TABLE _m037_legacy_fork_origins (
    child_run_id       TEXT PRIMARY KEY NOT NULL,
    parent_run_id      TEXT NOT NULL,
    origin_step_id     TEXT NOT NULL,
    origin_task_depth  INTEGER NOT NULL CHECK (origin_task_depth BETWEEN 0 AND 4)
);
WITH step_link_candidates AS (
    SELECT task.id AS step_id, task.conversation_id
    FROM orch_run_tasks task WHERE task.conversation_id IS NOT NULL
    UNION
    SELECT json_extract(conversation.extra, '$.orchestrator_task_id'), conversation.id
    FROM conversations conversation
    WHERE json_extract(conversation.extra, '$.orchestrator_task_id') IS NOT NULL
),
lead_candidates AS (
    SELECT run.id AS run_id, run.lead_conv_id AS conversation_id
    FROM orch_runs run
    JOIN conversations conversation ON conversation.id = run.lead_conv_id
    UNION
    SELECT json_extract(conversation.extra, '$.orchestrator_run_id'), conversation.id
    FROM conversations conversation
    WHERE json_extract(conversation.extra, '$.orchestrator_run_id') IS NOT NULL
      AND json_extract(conversation.extra, '$.orchestrator_task_id') IS NULL
)
INSERT INTO _m037_legacy_fork_origins (
    child_run_id, parent_run_id, origin_step_id, origin_task_depth
)
SELECT child.id, child.forked_from, task.id,
       COALESCE(json_extract(task.pattern_config, '$.delegation_depth'), 0)
FROM orch_runs child
JOIN lead_candidates child_lead ON child_lead.run_id = child.id
JOIN orch_run_tasks task ON task.run_id = child.forked_from
JOIN step_link_candidates link ON link.step_id = task.id
                                  AND link.conversation_id = child_lead.conversation_id
WHERE child.forked_from IS NOT NULL;

CREATE TABLE _m037_legacy_execution_depths (
    run_id           TEXT PRIMARY KEY NOT NULL,
    effective_depth  INTEGER NOT NULL CHECK (effective_depth >= 0)
);
WITH RECURSIVE effective_depth(run_id, depth) AS (
    SELECT run.id, 0
    FROM orch_runs run
    WHERE run.forked_from IS NULL
    UNION ALL
    SELECT origin.child_run_id,
           max(parent.depth, origin.origin_task_depth) + 1
    FROM effective_depth parent
    JOIN _m037_legacy_fork_origins origin
      ON origin.parent_run_id = parent.run_id
)
INSERT INTO _m037_legacy_execution_depths (run_id, effective_depth)
SELECT run_id, depth FROM effective_depth;
INSERT INTO _m037_guard
SELECT 0 WHERE
    (SELECT COUNT(*) FROM _m037_legacy_execution_depths)
        <> (SELECT COUNT(*) FROM orch_runs)
    OR EXISTS (
        SELECT 1 FROM _m037_legacy_execution_depths WHERE effective_depth > 4
    );

-- Conversation preferences are durable columns.  Execution creation freezes
-- their values, while plan gate/adaptation remain execution-only policies.
ALTER TABLE conversations ADD COLUMN delegation_policy TEXT NOT NULL DEFAULT 'automatic'
    CHECK (delegation_policy IN ('disabled', 'automatic', 'prefer_parallel'));
ALTER TABLE conversations ADD COLUMN execution_model_pool TEXT
    CHECK (
        CASE
            WHEN execution_model_pool IS NULL THEN 1
            WHEN NOT json_valid(execution_model_pool) THEN 0
            WHEN json_type(execution_model_pool) <> 'object' THEN 0
            WHEN json_type(execution_model_pool, '$.mode') <> 'text' THEN 0
            WHEN json_extract(execution_model_pool, '$.mode') = 'automatic'
                THEN 1
            WHEN json_extract(execution_model_pool, '$.mode') = 'single'
                THEN json_type(execution_model_pool, '$.model') = 'object'
                 AND json_type(execution_model_pool, '$.model.provider_id') = 'text'
                 AND trim(json_extract(execution_model_pool, '$.model.provider_id')) <> ''
                 AND trim(json_extract(execution_model_pool, '$.model.provider_id'))
                        = json_extract(execution_model_pool, '$.model.provider_id')
                 AND json_type(execution_model_pool, '$.model.model') = 'text'
                 AND trim(json_extract(execution_model_pool, '$.model.model')) <> ''
                 AND trim(json_extract(execution_model_pool, '$.model.model'))
                        = json_extract(execution_model_pool, '$.model.model')
            WHEN json_extract(execution_model_pool, '$.mode') = 'range'
                THEN json_type(execution_model_pool, '$.models') = 'array'
                 AND json_array_length(execution_model_pool, '$.models') BETWEEN 1 AND 16
            ELSE 0
        END
    );
ALTER TABLE conversations ADD COLUMN decision_policy TEXT NOT NULL DEFAULT 'automatic'
    CHECK (decision_policy IN ('automatic', 'ask_user'));

-- In-process consumers can bind a stable creation identity without leaking it
-- into the public Conversation DTO or the open-ended `extra` JSON.  Creating a
-- delegated attempt conversation is therefore retryable across a crash before
-- the Execution link commits, while ordinary user-created conversations remain
-- unchanged.
CREATE TABLE conversation_creation_keys (
    creation_key   TEXT PRIMARY KEY NOT NULL CHECK (trim(creation_key) <> ''),
    user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conversation_id INTEGER NOT NULL UNIQUE REFERENCES conversations(id) ON DELETE CASCADE,
    created_at     INTEGER NOT NULL
);
CREATE TRIGGER conversation_creation_key_owner_guard
BEFORE INSERT ON conversation_creation_keys
WHEN (SELECT COUNT(*) FROM conversations conversation
      WHERE conversation.id = NEW.conversation_id
        AND conversation.user_id = NEW.user_id) <> 1
BEGIN
    SELECT RAISE(ABORT, 'conversation creation key owner must match conversation owner');
END;
CREATE TRIGGER conversation_creation_key_immutable
BEFORE UPDATE ON conversation_creation_keys
BEGIN
    SELECT RAISE(ABORT, 'conversation creation key is immutable');
END;
CREATE TRIGGER conversation_creation_key_delete_guard
BEFORE DELETE ON conversation_creation_keys
WHEN EXISTS (SELECT 1 FROM users WHERE id = OLD.user_id)
 AND EXISTS (SELECT 1 FROM conversations WHERE id = OLD.conversation_id)
BEGIN
    SELECT RAISE(ABORT, 'conversation creation key may only be cascade deleted');
END;

-- Receiver-side idempotency receipts for trusted internal Conversation
-- effects.  Agent Execution keeps the business intent in attempt
-- `runtime_state`; this narrow table records only whether the Conversation
-- subsystem has already completed that stable operation, including terminal
-- errors with no assistant text.
CREATE TABLE conversation_delivery_receipts (
    operation_id    TEXT PRIMARY KEY NOT NULL CHECK (trim(operation_id) <> ''),
    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN ('turn', 'steer', 'projection')),
    request_payload TEXT NOT NULL CHECK (json_valid(request_payload)),
    status          TEXT NOT NULL CHECK (status IN ('accepted', 'completed')),
    result_ok       INTEGER CHECK (result_ok IS NULL OR result_ok IN (0, 1)),
    result_text     TEXT,
    result_error    TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    completed_at    INTEGER,
    CHECK (updated_at >= created_at),
    CHECK (completed_at IS NULL OR (completed_at >= created_at AND completed_at <= updated_at)),
    CHECK (
        (status = 'accepted' AND result_ok IS NULL AND completed_at IS NULL)
        OR (status = 'completed' AND result_ok IS NOT NULL AND completed_at IS NOT NULL)
    )
);
CREATE INDEX idx_conversation_delivery_receipts_conversation
    ON conversation_delivery_receipts(conversation_id, created_at);

CREATE TRIGGER conversation_delivery_receipt_owner_guard
BEFORE INSERT ON conversation_delivery_receipts
WHEN (SELECT COUNT(*) FROM conversations conversation
      WHERE conversation.id = NEW.conversation_id
        AND conversation.user_id = NEW.user_id) <> 1
BEGIN
    SELECT RAISE(ABORT, 'conversation delivery receipt owner must match conversation owner');
END;

-- A receipt is a receiver-owned idempotency fence, not ordinary mutable
-- application state.  Its operation identity and payload never change; the
-- only state transition is the one-way accepted -> completed settlement.
CREATE TRIGGER conversation_delivery_receipt_update_guard
BEFORE UPDATE ON conversation_delivery_receipts
FOR EACH ROW
BEGIN
    SELECT CASE WHEN
        NEW.operation_id IS NOT OLD.operation_id
        OR NEW.conversation_id IS NOT OLD.conversation_id
        OR NEW.user_id IS NOT OLD.user_id
        OR NEW.kind IS NOT OLD.kind
        OR NEW.request_payload IS NOT OLD.request_payload
        OR NEW.created_at IS NOT OLD.created_at
    THEN RAISE(ABORT, 'conversation delivery receipt identity is immutable') END;

    SELECT CASE WHEN NEW.updated_at < OLD.updated_at
        THEN RAISE(ABORT, 'conversation delivery receipt time must be monotonic') END;

    SELECT CASE WHEN NOT (
        (
            OLD.status = 'accepted'
            AND NEW.status = 'completed'
            AND NEW.result_ok IS NOT NULL
            AND NEW.completed_at IS NOT NULL
        )
        OR (
            NEW.status IS OLD.status
            AND NEW.result_ok IS OLD.result_ok
            AND NEW.result_text IS OLD.result_text
            AND NEW.result_error IS OLD.result_error
            AND NEW.completed_at IS OLD.completed_at
            AND NEW.updated_at IS OLD.updated_at
        )
    ) THEN RAISE(ABORT, 'conversation delivery receipt has one terminal transition') END;
END;

-- Direct deletion would erase the replay fence.  Cascades remain legal: when
-- SQLite deletes a receipt for a deleted Conversation or account, at least one
-- owning parent row has already left the database.
CREATE TRIGGER conversation_delivery_receipt_delete_guard
BEFORE DELETE ON conversation_delivery_receipts
FOR EACH ROW
WHEN EXISTS (SELECT 1 FROM users WHERE id = OLD.user_id)
 AND EXISTS (SELECT 1 FROM conversations WHERE id = OLD.conversation_id)
BEGIN
    SELECT RAISE(ABORT, 'conversation delivery receipt may only be cascade deleted');
END;

UPDATE conversations
SET delegation_policy = CASE json_type(extra, '$.agent_cluster_mode')
        WHEN 'true' THEN 'prefer_parallel'
        ELSE 'automatic'
    END,
    execution_model_pool = CASE
        WHEN json_extract(extra, '$.orchestrator_model_range.mode') IN ('auto', 'automatic')
            THEN json_object('mode', 'automatic')
        WHEN json_type(extra, '$.orchestrator_model_range') = 'array'
          OR json_extract(extra, '$.orchestrator_model_range.mode') IN ('range', 'single')
        THEN (
            SELECT CASE WHEN COUNT(*) = 0 THEN NULL ELSE
                json_object('mode', 'range', 'models', json(json_group_array(json(model.value))))
            END
            FROM (
                SELECT candidate.value
                FROM (
                    -- A finite collaboration authority must contain its lead.
                    -- Prefix a valid persisted Conversation model, then retain
                    -- legacy range order. GROUP BY below removes duplicates
                    -- deterministically before applying the shared limit.
                    SELECT json_object(
                               'provider_id', COALESCE(
                                   json_extract(conversations.model, '$.provider_id'),
                                   json_extract(conversations.model, '$.providerId'),
                                   json_extract(conversations.model, '$.id')
                               ),
                               'model', CASE
                                   WHEN trim(COALESCE(
                                            json_extract(conversations.model, '$.use_model'),
                                            json_extract(conversations.model, '$.useModel'),
                                            ''
                                        )) = ''
                                   THEN json_extract(conversations.model, '$.model')
                                   ELSE COALESCE(
                                            json_extract(conversations.model, '$.use_model'),
                                            json_extract(conversations.model, '$.useModel')
                                        )
                               END
                           ) AS value,
                           -1 AS sort_order
                    WHERE json_valid(conversations.model)
                      AND typeof(COALESCE(
                              json_extract(conversations.model, '$.provider_id'),
                              json_extract(conversations.model, '$.providerId'),
                              json_extract(conversations.model, '$.id')
                          )) = 'text'
                      AND trim(COALESCE(
                              json_extract(conversations.model, '$.provider_id'),
                              json_extract(conversations.model, '$.providerId'),
                              json_extract(conversations.model, '$.id')
                          )) <> ''
                      AND trim(COALESCE(
                              json_extract(conversations.model, '$.provider_id'),
                              json_extract(conversations.model, '$.providerId'),
                              json_extract(conversations.model, '$.id')
                          )) = COALESCE(
                              json_extract(conversations.model, '$.provider_id'),
                              json_extract(conversations.model, '$.providerId'),
                              json_extract(conversations.model, '$.id')
                          )
                      AND typeof(CASE
                              WHEN trim(COALESCE(
                                       json_extract(conversations.model, '$.use_model'),
                                       json_extract(conversations.model, '$.useModel'),
                                       ''
                                   )) = ''
                              THEN json_extract(conversations.model, '$.model')
                              ELSE COALESCE(
                                       json_extract(conversations.model, '$.use_model'),
                                       json_extract(conversations.model, '$.useModel')
                                   )
                          END) = 'text'
                      AND trim(CASE
                              WHEN trim(COALESCE(
                                       json_extract(conversations.model, '$.use_model'),
                                       json_extract(conversations.model, '$.useModel'),
                                       ''
                                   )) = ''
                              THEN json_extract(conversations.model, '$.model')
                              ELSE COALESCE(
                                       json_extract(conversations.model, '$.use_model'),
                                       json_extract(conversations.model, '$.useModel')
                                   )
                          END) <> ''
                      AND trim(CASE
                              WHEN trim(COALESCE(
                                       json_extract(conversations.model, '$.use_model'),
                                       json_extract(conversations.model, '$.useModel'),
                                       ''
                                   )) = ''
                              THEN json_extract(conversations.model, '$.model')
                              ELSE COALESCE(
                                       json_extract(conversations.model, '$.use_model'),
                                       json_extract(conversations.model, '$.useModel')
                                   )
                          END) = CASE
                              WHEN trim(COALESCE(
                                       json_extract(conversations.model, '$.use_model'),
                                       json_extract(conversations.model, '$.useModel'),
                                       ''
                                   )) = ''
                              THEN json_extract(conversations.model, '$.model')
                              ELSE COALESCE(
                                       json_extract(conversations.model, '$.use_model'),
                                       json_extract(conversations.model, '$.useModel')
                                   )
                          END
                    UNION ALL
                    SELECT legacy.value, CAST(legacy.key AS INTEGER)
                    FROM json_each(
                        CASE
                            WHEN json_type(conversations.extra, '$.orchestrator_model_range') = 'array'
                                THEN json_extract(conversations.extra, '$.orchestrator_model_range')
                            WHEN json_extract(conversations.extra, '$.orchestrator_model_range.mode') = 'range'
                                THEN json_extract(conversations.extra, '$.orchestrator_model_range.models')
                            ELSE json_array(json_extract(conversations.extra, '$.orchestrator_model_range.model'))
                        END
                    ) AS legacy
                ) AS candidate
                WHERE json_type(candidate.value) = 'object'
                  AND json_type(candidate.value, '$.provider_id') = 'text'
                  AND trim(json_extract(candidate.value, '$.provider_id')) <> ''
                  AND trim(json_extract(candidate.value, '$.provider_id'))
                        = json_extract(candidate.value, '$.provider_id')
                  AND json_type(candidate.value, '$.model') = 'text'
                  AND trim(json_extract(candidate.value, '$.model')) <> ''
                  AND trim(json_extract(candidate.value, '$.model'))
                        = json_extract(candidate.value, '$.model')
                  AND (SELECT COUNT(*) FROM json_each(candidate.value)) = 2
                GROUP BY json_extract(candidate.value, '$.provider_id'),
                         json_extract(candidate.value, '$.model')
                ORDER BY MIN(candidate.sort_order),
                         json_extract(candidate.value, '$.provider_id'),
                         json_extract(candidate.value, '$.model')
                LIMIT 16
            ) AS model
        )
        ELSE NULL
    END,
    decision_policy = CASE json_extract(extra, '$.orchestrator_approval_mode')
        WHEN 'manual' THEN 'ask_user'
        ELSE 'automatic'
    END;

-- The JSON CHECK above validates the tagged root shape. These guards validate
-- every Range member and uniqueness, which SQLite cannot express with a
-- subquery inside a portable column CHECK. No post-migration array alias is
-- accepted.
CREATE TRIGGER conversation_execution_model_pool_insert_guard
BEFORE INSERT ON conversations
WHEN NEW.execution_model_pool IS NOT NULL
 AND (
      (json_extract(NEW.execution_model_pool, '$.mode') = 'automatic'
       AND (SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 1)
      OR (json_extract(NEW.execution_model_pool, '$.mode') = 'single'
          AND ((SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 2
               OR (SELECT COUNT(*) FROM json_each(NEW.execution_model_pool, '$.model')) <> 2))
      OR (json_extract(NEW.execution_model_pool, '$.mode') = 'range'
          AND ((SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 2
               OR EXISTS (
          SELECT 1 FROM json_each(NEW.execution_model_pool, '$.models') AS model
          WHERE json_type(model.value) <> 'object'
             OR json_type(model.value, '$.provider_id') <> 'text'
             OR trim(json_extract(model.value, '$.provider_id')) = ''
             OR trim(json_extract(model.value, '$.provider_id'))
                    <> json_extract(model.value, '$.provider_id')
             OR json_type(model.value, '$.model') <> 'text'
             OR trim(json_extract(model.value, '$.model')) = ''
             OR trim(json_extract(model.value, '$.model'))
                    <> json_extract(model.value, '$.model')
             OR (SELECT COUNT(*) FROM json_each(model.value)) <> 2
               )
               OR (
          SELECT COUNT(*)
          FROM (
              SELECT json_extract(model.value, '$.provider_id'),
                     json_extract(model.value, '$.model')
              FROM json_each(NEW.execution_model_pool, '$.models') AS model
              GROUP BY 1, 2
          )
               ) <> json_array_length(NEW.execution_model_pool, '$.models')))
 )
BEGIN
    SELECT RAISE(ABORT, 'invalid conversation execution model pool');
END;

CREATE TRIGGER conversation_execution_model_pool_update_guard
BEFORE UPDATE OF execution_model_pool ON conversations
WHEN NEW.execution_model_pool IS NOT NULL
 AND (
      (json_extract(NEW.execution_model_pool, '$.mode') = 'automatic'
       AND (SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 1)
      OR (json_extract(NEW.execution_model_pool, '$.mode') = 'single'
          AND ((SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 2
               OR (SELECT COUNT(*) FROM json_each(NEW.execution_model_pool, '$.model')) <> 2))
      OR (json_extract(NEW.execution_model_pool, '$.mode') = 'range'
          AND ((SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 2
               OR EXISTS (
          SELECT 1 FROM json_each(NEW.execution_model_pool, '$.models') AS model
          WHERE json_type(model.value) <> 'object'
             OR json_type(model.value, '$.provider_id') <> 'text'
             OR trim(json_extract(model.value, '$.provider_id')) = ''
             OR trim(json_extract(model.value, '$.provider_id'))
                    <> json_extract(model.value, '$.provider_id')
             OR json_type(model.value, '$.model') <> 'text'
             OR trim(json_extract(model.value, '$.model')) = ''
             OR trim(json_extract(model.value, '$.model'))
                    <> json_extract(model.value, '$.model')
             OR (SELECT COUNT(*) FROM json_each(model.value)) <> 2
               )
               OR (
          SELECT COUNT(*)
          FROM (
              SELECT json_extract(model.value, '$.provider_id'),
                     json_extract(model.value, '$.model')
              FROM json_each(NEW.execution_model_pool, '$.models') AS model
              GROUP BY 1, 2
          )
               ) <> json_array_length(NEW.execution_model_pool, '$.models')))
 )
BEGIN
    SELECT RAISE(ABORT, 'invalid conversation execution model pool');
END;

-- Finite collaboration authority and the Conversation lead are one atomic
-- invariant. This also guards repository-level writers such as failover; they
-- must update model and pool in the same statement. NULL means inherit the
-- lead and explicit Automatic is catalog-scoped, so neither needs membership.
CREATE TRIGGER conversation_execution_model_authority_insert_guard
BEFORE INSERT ON conversations
WHEN NEW.execution_model_pool IS NOT NULL
 AND json_extract(NEW.execution_model_pool, '$.mode') IN ('single', 'range')
 AND (
      NEW.model IS NULL
      OR NOT json_valid(NEW.model)
      OR typeof(COALESCE(
             json_extract(NEW.model, '$.provider_id'),
             json_extract(NEW.model, '$.providerId'),
             json_extract(NEW.model, '$.id')
         )) <> 'text'
      OR trim(COALESCE(
             json_extract(NEW.model, '$.provider_id'),
             json_extract(NEW.model, '$.providerId'),
             json_extract(NEW.model, '$.id')
         )) = ''
      OR typeof(CASE
             WHEN trim(COALESCE(
                      json_extract(NEW.model, '$.use_model'),
                      json_extract(NEW.model, '$.useModel'),
                      ''
                  )) = ''
             THEN json_extract(NEW.model, '$.model')
             ELSE COALESCE(
                      json_extract(NEW.model, '$.use_model'),
                      json_extract(NEW.model, '$.useModel')
                  )
         END) <> 'text'
      OR trim(CASE
             WHEN trim(COALESCE(
                      json_extract(NEW.model, '$.use_model'),
                      json_extract(NEW.model, '$.useModel'),
                      ''
                  )) = ''
             THEN json_extract(NEW.model, '$.model')
             ELSE COALESCE(
                      json_extract(NEW.model, '$.use_model'),
                      json_extract(NEW.model, '$.useModel')
                  )
         END) = ''
      OR (
          json_extract(NEW.execution_model_pool, '$.mode') = 'single'
          AND (
              json_extract(NEW.execution_model_pool, '$.model.provider_id')
                  <> COALESCE(
                      json_extract(NEW.model, '$.provider_id'),
                      json_extract(NEW.model, '$.providerId'),
                      json_extract(NEW.model, '$.id')
                  )
              OR json_extract(NEW.execution_model_pool, '$.model.model')
                  <> CASE
                      WHEN trim(COALESCE(
                               json_extract(NEW.model, '$.use_model'),
                               json_extract(NEW.model, '$.useModel'),
                               ''
                           )) = ''
                      THEN json_extract(NEW.model, '$.model')
                      ELSE COALESCE(
                               json_extract(NEW.model, '$.use_model'),
                               json_extract(NEW.model, '$.useModel')
                           )
                  END
          )
      )
      OR (
          json_extract(NEW.execution_model_pool, '$.mode') = 'range'
          AND NOT EXISTS (
              SELECT 1
              FROM json_each(NEW.execution_model_pool, '$.models') AS allowed
              WHERE json_extract(allowed.value, '$.provider_id') = COALESCE(
                        json_extract(NEW.model, '$.provider_id'),
                        json_extract(NEW.model, '$.providerId'),
                        json_extract(NEW.model, '$.id')
                    )
                AND json_extract(allowed.value, '$.model') = CASE
                        WHEN trim(COALESCE(
                                 json_extract(NEW.model, '$.use_model'),
                                 json_extract(NEW.model, '$.useModel'),
                                 ''
                             )) = ''
                        THEN json_extract(NEW.model, '$.model')
                        ELSE COALESCE(
                                 json_extract(NEW.model, '$.use_model'),
                                 json_extract(NEW.model, '$.useModel')
                             )
                    END
          )
      )
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation lead model must belong to execution model pool');
END;

CREATE TRIGGER conversation_execution_model_authority_update_guard
BEFORE UPDATE OF model, execution_model_pool ON conversations
WHEN NEW.execution_model_pool IS NOT NULL
 AND json_extract(NEW.execution_model_pool, '$.mode') IN ('single', 'range')
 AND (
      NEW.model IS NULL
      OR NOT json_valid(NEW.model)
      OR typeof(COALESCE(
             json_extract(NEW.model, '$.provider_id'),
             json_extract(NEW.model, '$.providerId'),
             json_extract(NEW.model, '$.id')
         )) <> 'text'
      OR trim(COALESCE(
             json_extract(NEW.model, '$.provider_id'),
             json_extract(NEW.model, '$.providerId'),
             json_extract(NEW.model, '$.id')
         )) = ''
      OR typeof(CASE
             WHEN trim(COALESCE(
                      json_extract(NEW.model, '$.use_model'),
                      json_extract(NEW.model, '$.useModel'),
                      ''
                  )) = ''
             THEN json_extract(NEW.model, '$.model')
             ELSE COALESCE(
                      json_extract(NEW.model, '$.use_model'),
                      json_extract(NEW.model, '$.useModel')
                  )
         END) <> 'text'
      OR trim(CASE
             WHEN trim(COALESCE(
                      json_extract(NEW.model, '$.use_model'),
                      json_extract(NEW.model, '$.useModel'),
                      ''
                  )) = ''
             THEN json_extract(NEW.model, '$.model')
             ELSE COALESCE(
                      json_extract(NEW.model, '$.use_model'),
                      json_extract(NEW.model, '$.useModel')
                  )
         END) = ''
      OR (
          json_extract(NEW.execution_model_pool, '$.mode') = 'single'
          AND (
              json_extract(NEW.execution_model_pool, '$.model.provider_id')
                  <> COALESCE(
                      json_extract(NEW.model, '$.provider_id'),
                      json_extract(NEW.model, '$.providerId'),
                      json_extract(NEW.model, '$.id')
                  )
              OR json_extract(NEW.execution_model_pool, '$.model.model')
                  <> CASE
                      WHEN trim(COALESCE(
                               json_extract(NEW.model, '$.use_model'),
                               json_extract(NEW.model, '$.useModel'),
                               ''
                           )) = ''
                      THEN json_extract(NEW.model, '$.model')
                      ELSE COALESCE(
                               json_extract(NEW.model, '$.use_model'),
                               json_extract(NEW.model, '$.useModel')
                           )
                  END
          )
      )
      OR (
          json_extract(NEW.execution_model_pool, '$.mode') = 'range'
          AND NOT EXISTS (
              SELECT 1
              FROM json_each(NEW.execution_model_pool, '$.models') AS allowed
              WHERE json_extract(allowed.value, '$.provider_id') = COALESCE(
                        json_extract(NEW.model, '$.provider_id'),
                        json_extract(NEW.model, '$.providerId'),
                        json_extract(NEW.model, '$.id')
                    )
                AND json_extract(allowed.value, '$.model') = CASE
                        WHEN trim(COALESCE(
                                 json_extract(NEW.model, '$.use_model'),
                                 json_extract(NEW.model, '$.useModel'),
                                 ''
                             )) = ''
                        THEN json_extract(NEW.model, '$.model')
                        ELSE COALESCE(
                                 json_extract(NEW.model, '$.use_model'),
                                 json_extract(NEW.model, '$.useModel')
                             )
                    END
          )
      )
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation lead model must belong to execution model pool');
END;

-- Rename the preset target at the data boundary by rebuilding the CHECK table.
CREATE TABLE preset_targets_new (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (target_kind IN
        ('conversation', 'execution_step', 'companion', 'public_companion', 'cron')),
    PRIMARY KEY (preset_id, target_kind)
);
INSERT INTO preset_targets_new (preset_id, target_kind)
SELECT preset_id,
       CASE target_kind WHEN 'cluster_member' THEN 'execution_step' ELSE target_kind END
FROM preset_targets;
DROP TABLE preset_targets;
ALTER TABLE preset_targets_new RENAME TO preset_targets;

-- AgentExecutionTemplate is the single authoring/configuration aggregate for a
-- reusable collaboration plan. It deliberately has no runtime status, DAG,
-- Attempt, scheduler, or inheritance relation. The seven execution tables
-- below remain the complete runtime aggregate.
CREATE TABLE agent_execution_templates (
    id              TEXT PRIMARY KEY NOT NULL CHECK (trim(id) <> ''),
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name            TEXT NOT NULL CHECK (trim(name) <> ''),
    description     TEXT,
    max_parallel    INTEGER CHECK (max_parallel IS NULL OR max_parallel BETWEEN 1 AND 64),
    work_dir        TEXT,
    context         TEXT CHECK (context IS NULL OR json_valid(context)),
    -- A Template has no hidden draft/status state: every persisted aggregate
    -- names at least one concrete Participant. The circular FK is deferred so
    -- parent and children can be inserted/replaced atomically in one
    -- transaction, while commit can never leave an empty Template.
    primary_participant_id TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL CHECK (updated_at >= created_at),
    FOREIGN KEY (id, primary_participant_id)
        REFERENCES agent_execution_template_participants(template_id, id)
        DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX idx_agent_execution_templates_owner_updated
    ON agent_execution_templates(user_id, updated_at DESC, id);

CREATE TRIGGER agent_execution_template_identity_guard
BEFORE UPDATE ON agent_execution_templates
WHEN NEW.id IS NOT OLD.id
  OR NEW.user_id IS NOT OLD.user_id
  OR NEW.created_at IS NOT OLD.created_at
  OR NEW.version < OLD.version
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template identity is immutable');
END;

CREATE TABLE agent_execution_template_participants (
    id                      TEXT NOT NULL CHECK (trim(id) <> ''),
    template_id             TEXT NOT NULL
                                REFERENCES agent_execution_templates(id) ON DELETE CASCADE,
    source_agent_id         TEXT NOT NULL CHECK (trim(source_agent_id) <> ''),
    preset_id               TEXT,
    preset_revision         INTEGER,
    preset_snapshot         TEXT,
    provider_id             TEXT,
    model                   TEXT,
    role                    TEXT,
    capability              TEXT CHECK (
                                CASE WHEN capability IS NULL THEN 1
                                     WHEN NOT json_valid(capability) THEN 0
                                     ELSE json_type(capability) = 'object' END
                            ),
    constraints             TEXT CHECK (
                                CASE WHEN constraints IS NULL THEN 1
                                     WHEN NOT json_valid(constraints) THEN 0
                                     ELSE json_type(constraints) = 'object'
                                          AND (
                                              json_type(constraints, '$.max_concurrency') IS NULL
                                              OR json_type(constraints, '$.max_concurrency') = 'null'
                                              OR (
                                                  json_type(constraints, '$.max_concurrency') = 'integer'
                                                  AND json_extract(constraints, '$.max_concurrency') BETWEEN 1 AND 64
                                              )
                                          )
                                     END
                            ),
    description             TEXT,
    system_prompt           TEXT,
    enabled_skills          TEXT NOT NULL DEFAULT '[]' CHECK (
                                json_valid(enabled_skills)
                                AND json_type(enabled_skills) = 'array'
                            ),
    disabled_builtin_skills TEXT NOT NULL DEFAULT '[]' CHECK (
                                json_valid(disabled_builtin_skills)
                                AND json_type(disabled_builtin_skills) = 'array'
                            ),
    sort_order              INTEGER NOT NULL DEFAULT 0,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL CHECK (updated_at >= created_at),
    PRIMARY KEY (template_id, id),
    CHECK (
        provider_id IS NOT NULL AND model IS NOT NULL
        AND trim(provider_id) <> '' AND trim(provider_id) = provider_id
        AND trim(model) <> '' AND trim(model) = model
    ),
    CHECK (
        CASE WHEN preset_id IS NULL THEN
            preset_revision IS NULL AND preset_snapshot IS NULL
        WHEN trim(preset_id) = '' OR preset_revision IS NULL OR preset_revision <= 0
             OR preset_snapshot IS NULL OR NOT json_valid(preset_snapshot) THEN 0
        ELSE json_type(preset_snapshot) = 'object'
             AND json_extract(preset_snapshot, '$.preset_id') = preset_id
             AND json_extract(preset_snapshot, '$.preset_revision') = preset_revision
             AND json_extract(preset_snapshot, '$.target') = 'execution_step'
        END
    )
);
CREATE INDEX idx_agent_execution_template_participants_order
    ON agent_execution_template_participants(template_id, sort_order, id);
CREATE INDEX idx_agent_execution_template_participants_provider
    ON agent_execution_template_participants(provider_id, template_id)
    WHERE provider_id IS NOT NULL;

CREATE TRIGGER agent_execution_template_participant_identity_guard
BEFORE UPDATE ON agent_execution_template_participants
WHEN NEW.id IS NOT OLD.id
  OR NEW.template_id IS NOT OLD.template_id
  OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template Participant identity is immutable');
END;

CREATE TRIGGER agent_execution_template_participant_limit
BEFORE INSERT ON agent_execution_template_participants
WHEN (SELECT COUNT(*) FROM agent_execution_template_participants participant
      WHERE participant.template_id = NEW.template_id) >= 64
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template exceeds 64 participants');
END;

CREATE TRIGGER agent_execution_template_model_limit
BEFORE INSERT ON agent_execution_template_participants
WHEN NOT EXISTS (
         SELECT 1 FROM agent_execution_template_participants participant
         WHERE participant.template_id = NEW.template_id
           AND participant.provider_id = NEW.provider_id
           AND participant.model = NEW.model
     )
 AND (SELECT COUNT(*) FROM (
          SELECT participant.provider_id, participant.model
          FROM agent_execution_template_participants participant
          WHERE participant.template_id = NEW.template_id
          GROUP BY participant.provider_id, participant.model
     )) >= 16
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template exceeds 16 distinct models');
END;

-- First build one canonical, converted member set per former Fleet. Workspace
-- templates copy this staging set, so preset/constraint vocabulary cannot
-- diverge between the two legacy source paths.
CREATE TABLE _m037_template_participant_stage (
    id                      TEXT NOT NULL,
    fleet_id                TEXT NOT NULL,
    source_agent_id         TEXT NOT NULL,
    preset_id               TEXT,
    preset_revision         INTEGER,
    preset_snapshot         TEXT,
    provider_id             TEXT,
    model                   TEXT,
    role                    TEXT,
    capability              TEXT,
    constraints             TEXT,
    description             TEXT,
    system_prompt           TEXT,
    enabled_skills          TEXT NOT NULL,
    disabled_builtin_skills TEXT NOT NULL,
    sort_order              INTEGER NOT NULL,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    PRIMARY KEY (fleet_id, id)
);
INSERT INTO _m037_template_participant_stage (
    id, fleet_id, source_agent_id, preset_id, preset_revision, preset_snapshot,
    provider_id, model, role, capability, constraints, description, system_prompt,
    enabled_skills, disabled_builtin_skills, sort_order, created_at, updated_at
)
SELECT
    member.id,
    member.fleet_id,
    member.agent_id,
    member.preset_id,
    member.preset_revision,
    CASE WHEN member.preset_snapshot IS NOT NULL THEN
        CASE WHEN json_extract(member.preset_snapshot, '$.target') = 'cluster_member'
             THEN json_set(json(member.preset_snapshot), '$.target', 'execution_step')
             ELSE json(member.preset_snapshot) END
    END,
    member.provider_id,
    member.model,
    member.role_hint,
    CASE
        WHEN member.capability_profile IS NOT NULL THEN json(member.capability_profile)
        WHEN json_type(member.constraints, '$.cost_tier') = 'text' THEN json_object(
            'strengths', json('[]'),
            'modalities', json('[]'),
            'tools', json('false'),
            'reasoning', 'medium',
            'cost_tier', json_extract(member.constraints, '$.cost_tier'),
            'speed_tier', 'standard'
        )
    END,
    CASE WHEN member.constraints IS NOT NULL THEN
        CASE WHEN json_type(member.constraints, '$.allowed_task_kinds') = 'array' THEN
            json_remove(
                json_set(
                    json(member.constraints),
                    '$.allowed_profile_kinds',
                    json((
                        SELECT json_group_array(mapped) FROM (
                            SELECT DISTINCT trim(kind.value) AS mapped
                            FROM json_each(member.constraints, '$.allowed_task_kinds') kind
                            ORDER BY mapped
                        )
                    ))
                ),
                '$.allowed_task_kinds',
                '$.cost_tier'
            )
        ELSE json_remove(
            json(member.constraints), '$.allowed_task_kinds', '$.cost_tier'
        ) END
    END,
    NULL,
    NULL,
    '[]',
    '[]',
    member.sort_order,
    member.created_at,
    member.updated_at
FROM fleet_members member;

-- Legacy authoring allowed a participant-local concurrency value above the
-- unified scheduler ceiling. Preserve the configuration while narrowing that
-- single scalar to the canonical 1..=64 authority.
UPDATE _m037_template_participant_stage
SET constraints = json_replace(constraints, '$.max_concurrency', 64)
WHERE json_type(constraints, '$.max_concurrency') = 'integer'
  AND json_extract(constraints, '$.max_concurrency') > 64;

-- Materialize a concrete runtime model on every retained participant. Direct
-- legacy fields win; a valid preset snapshot is the deterministic fallback.
-- Rows that still cannot execute are discarded rather than migrated into a
-- delayed runtime failure.
UPDATE _m037_template_participant_stage
SET provider_id = COALESCE(
        NULLIF(trim(provider_id), ''),
        NULLIF(trim(json_extract(preset_snapshot, '$.resolved_model.provider_id')), '')
    ),
    model = COALESCE(
        NULLIF(trim(model), ''),
        NULLIF(trim(json_extract(preset_snapshot, '$.resolved_model.model')), '')
    );
DELETE FROM _m037_template_participant_stage
WHERE provider_id IS NULL OR trim(provider_id) = ''
   OR model IS NULL OR trim(model) = '';

-- Keep the first 16 distinct model pairs per Fleet, then at most the first 64
-- participants, both in stable authoring order. This converts oversized
-- legacy configuration once instead of preserving a Template that can never
-- instantiate under the shared runtime ceilings.
WITH model_order AS (
    SELECT fleet_id, provider_id, model,
           ROW_NUMBER() OVER (
               PARTITION BY fleet_id
               ORDER BY first_sort_order, first_id, provider_id, model
           ) AS model_rank
    FROM (
        SELECT fleet_id, provider_id, model,
               MIN(sort_order) AS first_sort_order,
               MIN(id) AS first_id
        FROM _m037_template_participant_stage
        GROUP BY fleet_id, provider_id, model
    )
)
DELETE FROM _m037_template_participant_stage
WHERE EXISTS (
    SELECT 1 FROM model_order allowed
    WHERE allowed.fleet_id = _m037_template_participant_stage.fleet_id
      AND allowed.provider_id = _m037_template_participant_stage.provider_id
      AND allowed.model = _m037_template_participant_stage.model
      AND allowed.model_rank > 16
);
WITH participant_order AS (
    SELECT fleet_id, id,
           ROW_NUMBER() OVER (
               PARTITION BY fleet_id ORDER BY sort_order, id
           ) AS participant_rank
    FROM _m037_template_participant_stage
)
DELETE FROM _m037_template_participant_stage
WHERE (fleet_id, id) IN (
    SELECT fleet_id, id FROM participant_order WHERE participant_rank > 64
);

INSERT INTO agent_execution_templates (
    id, user_id, name, description, max_parallel, work_dir, context,
    primary_participant_id,
    version, created_at, updated_at
)
SELECT fleet.id, fleet.user_id, fleet.name, fleet.description,
       CASE WHEN fleet.max_parallel IS NULL THEN NULL
            ELSE min(max(fleet.max_parallel, 1), 64) END,
       NULL, NULL,
       (SELECT stage.id FROM _m037_template_participant_stage stage
        WHERE stage.fleet_id = fleet.id ORDER BY stage.sort_order, stage.id LIMIT 1),
       0, fleet.created_at, fleet.updated_at
FROM fleets fleet
WHERE EXISTS (
    SELECT 1 FROM _m037_template_participant_stage stage
    WHERE stage.fleet_id = fleet.id
);

INSERT INTO agent_execution_templates (
    id, user_id, name, description, max_parallel, work_dir, context,
    primary_participant_id,
    version, created_at, updated_at
)
SELECT workspace.id, workspace.user_id, workspace.name,
       fleet.description,
       CASE WHEN fleet.max_parallel IS NULL THEN NULL
            ELSE min(max(fleet.max_parallel, 1), 64) END,
       workspace.workspace_dir, workspace.context,
       (SELECT stage.id FROM _m037_template_participant_stage stage
        WHERE stage.fleet_id = workspace.default_fleet_id
        ORDER BY stage.sort_order, stage.id LIMIT 1),
       0, workspace.created_at, workspace.updated_at
FROM orch_workspaces workspace
JOIN fleets fleet ON fleet.id = workspace.default_fleet_id
WHERE EXISTS (
    SELECT 1 FROM _m037_template_participant_stage stage
    WHERE stage.fleet_id = workspace.default_fleet_id
);

INSERT INTO agent_execution_template_participants (
    id, template_id, source_agent_id, preset_id, preset_revision, preset_snapshot,
    provider_id, model, role, capability, constraints, description, system_prompt,
    enabled_skills, disabled_builtin_skills, sort_order, created_at, updated_at
)
SELECT id, fleet_id, source_agent_id, preset_id, preset_revision, preset_snapshot,
       provider_id, model, role, capability, constraints, description, system_prompt,
       enabled_skills, disabled_builtin_skills, sort_order, created_at, updated_at
FROM _m037_template_participant_stage;

INSERT INTO agent_execution_template_participants (
    id, template_id, source_agent_id, preset_id, preset_revision, preset_snapshot,
    provider_id, model, role, capability, constraints, description, system_prompt,
    enabled_skills, disabled_builtin_skills, sort_order, created_at, updated_at
)
SELECT stage.id, workspace.id, stage.source_agent_id,
       stage.preset_id, stage.preset_revision, stage.preset_snapshot,
       stage.provider_id, stage.model, stage.role, stage.capability, stage.constraints,
       stage.description, stage.system_prompt, stage.enabled_skills,
       stage.disabled_builtin_skills, stage.sort_order, stage.created_at, stage.updated_at
FROM orch_workspaces workspace
JOIN _m037_template_participant_stage stage
  ON stage.fleet_id = workspace.default_fleet_id;

-- Template selection is a typed Conversation preference, not an open-ended
-- `extra` convention. It is owner-scoped and can only reference a complete,
-- executable Template. Deleting authoring configuration clears future launch
-- selection through the FK; already-materialized Executions contain snapshots
-- and have no Template FK.
ALTER TABLE conversations ADD COLUMN execution_template_id TEXT
    REFERENCES agent_execution_templates(id) ON DELETE SET NULL;

CREATE TRIGGER conversation_execution_template_owner_guard_insert
BEFORE INSERT ON conversations
WHEN NEW.execution_template_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM agent_execution_templates template
     JOIN agent_execution_template_participants participant
       ON participant.template_id = template.id
     WHERE template.id = NEW.execution_template_id
       AND template.user_id = NEW.user_id
       AND participant.provider_id = COALESCE(
           json_extract(NEW.model, '$.provider_id'),
           json_extract(NEW.model, '$.providerId'),
           json_extract(NEW.model, '$.id')
       )
       AND participant.model = CASE
           WHEN typeof(COALESCE(
                    json_extract(NEW.model, '$.use_model'),
                    json_extract(NEW.model, '$.useModel')
                )) = 'text'
            AND trim(COALESCE(
                    json_extract(NEW.model, '$.use_model'),
                    json_extract(NEW.model, '$.useModel')
                )) <> ''
           THEN COALESCE(
                    json_extract(NEW.model, '$.use_model'),
                    json_extract(NEW.model, '$.useModel')
                )
           ELSE json_extract(NEW.model, '$.model')
       END
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation execution template must be executable, owner-scoped, and contain the lead model');
END;

CREATE TRIGGER conversation_execution_template_owner_guard_update
BEFORE UPDATE OF user_id, execution_template_id, model ON conversations
WHEN NEW.execution_template_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM agent_execution_templates template
     JOIN agent_execution_template_participants participant
       ON participant.template_id = template.id
     WHERE template.id = NEW.execution_template_id
       AND template.user_id = NEW.user_id
       AND participant.provider_id = COALESCE(
           json_extract(NEW.model, '$.provider_id'),
           json_extract(NEW.model, '$.providerId'),
           json_extract(NEW.model, '$.id')
       )
       AND participant.model = CASE
           WHEN typeof(COALESCE(
                    json_extract(NEW.model, '$.use_model'),
                    json_extract(NEW.model, '$.useModel')
                )) = 'text'
            AND trim(COALESCE(
                    json_extract(NEW.model, '$.use_model'),
                    json_extract(NEW.model, '$.useModel')
                )) <> ''
           THEN COALESCE(
                    json_extract(NEW.model, '$.use_model'),
                    json_extract(NEW.model, '$.useModel')
                )
           ELSE json_extract(NEW.model, '$.model')
       END
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation execution template must be executable, owner-scoped, and contain the lead model');
END;

UPDATE conversations
SET execution_template_id = json_extract(extra, '$.execution_template_id')
WHERE json_type(extra, '$.execution_template_id') = 'text'
  AND EXISTS (
      SELECT 1
      FROM agent_execution_templates template
      JOIN agent_execution_template_participants participant
        ON participant.template_id = template.id
      WHERE template.id = json_extract(conversations.extra, '$.execution_template_id')
        AND template.user_id = conversations.user_id
        AND participant.provider_id = COALESCE(
            json_extract(conversations.model, '$.provider_id'),
            json_extract(conversations.model, '$.providerId'),
            json_extract(conversations.model, '$.id')
        )
        AND participant.model = CASE
            WHEN typeof(COALESCE(
                     json_extract(conversations.model, '$.use_model'),
                     json_extract(conversations.model, '$.useModel')
                 )) = 'text'
             AND trim(COALESCE(
                     json_extract(conversations.model, '$.use_model'),
                     json_extract(conversations.model, '$.useModel')
                 )) <> ''
            THEN COALESCE(
                     json_extract(conversations.model, '$.use_model'),
                     json_extract(conversations.model, '$.useModel')
                 )
            ELSE json_extract(conversations.model, '$.model')
        END
  );

CREATE TABLE agent_executions (
    id                    TEXT PRIMARY KEY NOT NULL,
    user_id               TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    goal                  TEXT NOT NULL CHECK (trim(goal) <> ''),
    status                TEXT NOT NULL CHECK (status IN (
                              'planning', 'awaiting_approval', 'running', 'paused',
                              'waiting_input', 'completed', 'completed_with_failures',
                              'failed', 'cancelled'
                          )),
    plan_gate             TEXT NOT NULL CHECK (plan_gate IN ('automatic', 'require_approval')),
    adaptation_policy     TEXT NOT NULL CHECK (adaptation_policy IN ('fixed', 'adaptive')),
    decision_policy       TEXT NOT NULL CHECK (decision_policy IN ('automatic', 'ask_user')),
    delegation_policy     TEXT NOT NULL CHECK (delegation_policy IN ('disabled', 'automatic', 'prefer_parallel')),
    max_parallel          INTEGER NOT NULL DEFAULT 4 CHECK (max_parallel BETWEEN 1 AND 64),
    work_dir              TEXT,
    initial_plan_input    TEXT NOT NULL CHECK (
                              json_valid(initial_plan_input)
                              AND json_type(initial_plan_input) = 'object'
                              AND json_extract(initial_plan_input, '$.mode') IN ('automatic', 'explicit')
                              AND (
                                  (json_extract(initial_plan_input, '$.mode') = 'automatic'
                                   AND json_type(initial_plan_input, '$.plan') IS NULL)
                                  OR
                                  (json_extract(initial_plan_input, '$.mode') = 'explicit'
                                   AND json_type(initial_plan_input, '$.plan') = 'object'
                                   AND json_type(initial_plan_input, '$.plan.steps') = 'array'
                                   AND json_array_length(initial_plan_input, '$.plan.steps') > 0)
                              )
                          ),
    summary               TEXT,
    total_tokens          INTEGER CHECK (total_tokens IS NULL OR total_tokens >= 0),
    version               INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    plan_revision         INTEGER NOT NULL DEFAULT 0 CHECK (plan_revision >= 0),
    event_sequence        INTEGER NOT NULL DEFAULT 0 CHECK (event_sequence >= 0),
    lease_owner           TEXT,
    lease_expires_at      INTEGER,
    deleted_at            INTEGER CHECK (deleted_at IS NULL OR deleted_at >= created_at),
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    CHECK (
        (lease_owner IS NULL AND lease_expires_at IS NULL)
        OR (trim(lease_owner) <> '' AND lease_expires_at IS NOT NULL)
    ),
    -- WaitingInput is an aggregate attention signal, not a global execution
    -- mutex: independent ready steps and durable decision continuations may
    -- still run while another attempt waits for the user.
    CHECK (status IN ('running', 'waiting_input')
           OR (lease_owner IS NULL AND lease_expires_at IS NULL)),
    CHECK (updated_at >= created_at)
);
CREATE INDEX idx_agent_executions_owner_updated
    ON agent_executions(user_id, updated_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_agent_executions_status_lease
    ON agent_executions(status, lease_expires_at) WHERE deleted_at IS NULL;

-- Product deletion is a durable tombstone, not a physical cascade.  This
-- preserves the execution graph and attempt transcripts while
-- keeping every normal read/mutation owner-scoped to non-deleted aggregates.
CREATE TRIGGER agent_execution_deleted_immutable
BEFORE UPDATE ON agent_executions
WHEN OLD.deleted_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'deleted agent execution is immutable');
END;

CREATE TRIGGER agent_execution_identity_monotonic
BEFORE UPDATE ON agent_executions
WHEN NEW.id IS NOT OLD.id
  OR NEW.user_id IS NOT OLD.user_id
  OR NEW.initial_plan_input IS NOT OLD.initial_plan_input
  OR NEW.created_at IS NOT OLD.created_at
  OR NEW.version < OLD.version
  OR NEW.plan_revision < OLD.plan_revision
  OR NEW.event_sequence < OLD.event_sequence
BEGIN
    SELECT RAISE(ABORT, 'agent execution identity and revisions are immutable');
END;

-- Settled results may be reopened only to running by an explicit versioned
-- retry/adopt command. Cancelled is the irreversible product terminal state.
CREATE TRIGGER agent_execution_status_transition
BEFORE UPDATE OF status ON agent_executions
WHEN NEW.status <> OLD.status AND NOT (
       (OLD.status = 'planning'
        AND NEW.status IN ('awaiting_approval', 'running', 'failed', 'cancelled'))
    OR (OLD.status = 'awaiting_approval'
        AND NEW.status IN ('running', 'failed', 'cancelled'))
    OR (OLD.status = 'running'
        AND NEW.status IN ('awaiting_approval', 'paused', 'waiting_input', 'completed',
                           'completed_with_failures', 'failed', 'cancelled'))
    OR (OLD.status = 'paused'
        AND NEW.status IN (
            'awaiting_approval', 'running', 'waiting_input', 'failed', 'cancelled'
        ))
    OR (OLD.status = 'waiting_input'
        AND NEW.status IN ('awaiting_approval', 'running', 'paused', 'failed', 'cancelled'))
    OR (OLD.status IN ('completed', 'completed_with_failures', 'failed')
        AND NEW.status = 'running')
)
BEGIN
    SELECT RAISE(ABORT, 'invalid agent execution status transition');
END;

-- A normal product delete must use the tombstone transition so child history
-- cannot disappear through a raw cascade.  Full account deletion is still
-- allowed: the owning user row no longer exists when its FK cascade runs.
CREATE TRIGGER agent_execution_requires_tombstone
BEFORE DELETE ON agent_executions
WHEN EXISTS (SELECT 1 FROM users WHERE id = OLD.user_id)
BEGIN
    SELECT RAISE(ABORT, 'agent execution must be tombstoned, not physically deleted');
END;

CREATE TABLE agent_execution_participants (
    id                      TEXT NOT NULL,
    execution_id            TEXT NOT NULL REFERENCES agent_executions(id) ON DELETE CASCADE,
    source_agent_id         TEXT NOT NULL CHECK (trim(source_agent_id) <> ''),
    preset_id               TEXT,
    preset_revision         INTEGER CHECK (preset_revision IS NULL OR preset_revision > 0),
    preset_snapshot         TEXT CHECK (
                                preset_snapshot IS NULL
                                OR (json_valid(preset_snapshot) AND json_type(preset_snapshot) = 'object')
                            ),
    provider_id             TEXT,
    model                   TEXT,
    role                    TEXT,
    capability              TEXT CHECK (
                                capability IS NULL
                                OR (json_valid(capability) AND json_type(capability) = 'object')
                            ),
    constraints             TEXT CHECK (
                                CASE WHEN constraints IS NULL THEN 1
                                     WHEN NOT json_valid(constraints) THEN 0
                                     ELSE json_type(constraints) = 'object'
                                          AND (
                                              json_type(constraints, '$.max_concurrency') IS NULL
                                              OR json_type(constraints, '$.max_concurrency') = 'null'
                                              OR (
                                                  json_type(constraints, '$.max_concurrency') = 'integer'
                                                  AND json_extract(constraints, '$.max_concurrency') BETWEEN 1 AND 64
                                              )
                                          )
                                     END
                            ),
    description             TEXT,
    system_prompt           TEXT,
    enabled_skills          TEXT NOT NULL DEFAULT '[]'
                                CHECK (json_valid(enabled_skills) AND json_type(enabled_skills) = 'array'),
    disabled_builtin_skills TEXT NOT NULL DEFAULT '[]'
                                CHECK (json_valid(disabled_builtin_skills)
                                       AND json_type(disabled_builtin_skills) = 'array'),
    sort_order              INTEGER NOT NULL DEFAULT 0,
    introduced_in_revision  INTEGER NOT NULL CHECK (introduced_in_revision >= 0),
    retired_in_revision     INTEGER CHECK (
                                retired_in_revision IS NULL
                                OR retired_in_revision > introduced_in_revision
                            ),
    created_at              INTEGER NOT NULL,
    PRIMARY KEY (execution_id, id),
    CHECK (
        (provider_id IS NULL AND model IS NULL)
        OR (provider_id IS NOT NULL AND model IS NOT NULL
            AND trim(provider_id) <> '' AND trim(provider_id) = provider_id
            AND trim(model) <> '' AND trim(model) = model)
    )
);
CREATE INDEX idx_agent_execution_participants_order
    ON agent_execution_participants(execution_id, sort_order, id);
CREATE INDEX idx_agent_execution_participants_active
    ON agent_execution_participants(execution_id, sort_order, id)
    WHERE retired_in_revision IS NULL;

CREATE TRIGGER agent_execution_participants_immutable
BEFORE UPDATE ON agent_execution_participants
WHEN OLD.retired_in_revision IS NOT NULL
  OR NEW.retired_in_revision IS NULL
  OR NEW.retired_in_revision <= OLD.introduced_in_revision
  OR NEW.id IS NOT OLD.id
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.source_agent_id IS NOT OLD.source_agent_id
  OR NEW.preset_id IS NOT OLD.preset_id
  OR NEW.preset_revision IS NOT OLD.preset_revision
  OR NEW.preset_snapshot IS NOT OLD.preset_snapshot
  OR NEW.provider_id IS NOT OLD.provider_id
  OR NEW.model IS NOT OLD.model
  OR NEW.role IS NOT OLD.role
  OR NEW.capability IS NOT OLD.capability
  OR NEW.constraints IS NOT OLD.constraints
  OR NEW.description IS NOT OLD.description
  OR NEW.system_prompt IS NOT OLD.system_prompt
  OR NEW.enabled_skills IS NOT OLD.enabled_skills
  OR NEW.disabled_builtin_skills IS NOT OLD.disabled_builtin_skills
  OR NEW.sort_order IS NOT OLD.sort_order
  OR NEW.introduced_in_revision IS NOT OLD.introduced_in_revision
  OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'participant snapshot is immutable; only first retirement is allowed');
END;

-- Every participant in a reopenable aggregate has one concrete, canonical
-- provider/model binding. Nullable pairs exist solely to preserve irreversible
-- Cancelled audit history; they can never enter a live or reopenable aggregate.
CREATE TRIGGER agent_execution_reopenable_participant_model_guard
BEFORE INSERT ON agent_execution_participants
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
      AND execution.status <> 'cancelled'
)
 AND (NEW.provider_id IS NULL OR NEW.model IS NULL)
BEGIN
    SELECT RAISE(ABORT, 'non-terminal Agent Execution participant requires a concrete model');
END;

-- Terminal legacy history may contain an unresolved participant, but such an
-- aggregate is read-only until its active participant set becomes executable.
-- Since participant snapshots are immutable, reopening that history would be
-- an unrecoverable state transition and is rejected at the database boundary.
CREATE TRIGGER agent_execution_reopen_participant_model_guard
BEFORE UPDATE OF status ON agent_executions
WHEN NEW.status = 'running'
 AND OLD.status IN ('completed', 'completed_with_failures', 'failed')
 AND EXISTS (
     SELECT 1 FROM agent_execution_participants participant
     WHERE participant.execution_id = OLD.id
       AND participant.retired_in_revision IS NULL
       AND (participant.provider_id IS NULL OR participant.model IS NULL)
 )
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution cannot reopen with an unresolved participant');
END;

CREATE TABLE agent_execution_steps (
    id                       TEXT NOT NULL,
    execution_id             TEXT NOT NULL REFERENCES agent_executions(id) ON DELETE CASCADE,
    title                    TEXT NOT NULL CHECK (trim(title) <> ''),
    spec                     TEXT NOT NULL,
    role                     TEXT,
    tool_policy              TEXT NOT NULL DEFAULT 'full'
                                 CHECK (tool_policy IN ('full', 'read_only', 'read_shell')),
    kind                     TEXT NOT NULL CHECK (kind IN ('agent', 'verify', 'judge', 'loop')),
    agent_mode               TEXT CHECK (agent_mode IS NULL OR agent_mode IN ('normal', 'synthesis')),
    profile                  TEXT CHECK (
                                 profile IS NULL
                                 OR (json_valid(profile) AND json_type(profile) = 'object')
                             ),
    fanout_group             TEXT CHECK (fanout_group IS NULL OR trim(fanout_group) <> ''),
    control_policy           TEXT CHECK (
                                 control_policy IS NULL
                                 OR (json_valid(control_policy) AND json_type(control_policy) = 'object')
                             ),
    delegation_depth         INTEGER NOT NULL DEFAULT 0
                                 CHECK (delegation_depth BETWEEN 0 AND 4),
    status                   TEXT NOT NULL CHECK (status IN (
                               'pending', 'running', 'waiting_input', 'completed',
                               'failed', 'skipped', 'cancelled'
                           )),
    assigned_participant_id  TEXT,
    assignment_score         REAL,
    assignment_rationale     TEXT,
    assignment_source        TEXT CHECK (
                                 assignment_source IS NULL
                                 OR assignment_source IN ('planner', 'automatic', 'manual')
                             ),
    assignment_locked        INTEGER NOT NULL DEFAULT 0 CHECK (assignment_locked IN (0, 1)),
    failure_policy           TEXT NOT NULL DEFAULT 'fail_execution'
                                 CHECK (failure_policy IN ('fail_execution', 'skip_dependents')),
    preset_prompt            TEXT,
    graph_x                  REAL,
    graph_y                  REAL,
    dispatch_after           INTEGER CHECK (
                                 dispatch_after IS NULL
                                 OR (dispatch_after >= 0 AND status = 'pending')
                             ),
    version                  INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    introduced_in_revision   INTEGER NOT NULL CHECK (introduced_in_revision >= 0),
    superseded_in_revision   INTEGER CHECK (
                                 superseded_in_revision IS NULL
                                 OR superseded_in_revision > introduced_in_revision
                             ),
    created_at               INTEGER NOT NULL,
    updated_at               INTEGER NOT NULL,
    PRIMARY KEY (execution_id, id),
    FOREIGN KEY (execution_id, assigned_participant_id)
        REFERENCES agent_execution_participants(execution_id, id),
    CHECK (
        (kind = 'agent' AND agent_mode IS NOT NULL AND control_policy IS NULL
            AND assigned_participant_id IS NOT NULL AND assignment_source IS NOT NULL)
        OR
        (kind IN ('verify', 'judge', 'loop') AND agent_mode IS NULL
            AND control_policy IS NOT NULL
            AND json_extract(control_policy, '$.kind') = kind
            AND assigned_participant_id IS NULL
            AND assignment_score IS NULL AND assignment_rationale IS NULL
            AND assignment_source IS NULL AND assignment_locked = 0)
    ),
    CHECK (kind = 'agent' OR tool_policy = 'full'),
    CHECK (kind = 'agent' OR fanout_group IS NULL)
);
CREATE INDEX idx_agent_execution_steps_status
    ON agent_execution_steps(execution_id, status, updated_at);
CREATE INDEX idx_agent_execution_steps_active_status
    ON agent_execution_steps(execution_id, status, updated_at)
    WHERE superseded_in_revision IS NULL;

-- A step row is one immutable semantic snapshot. Runtime lifecycle fields may
-- advance in place, and a plan revision may supersede the snapshot exactly
-- once; changing title/spec/routing/policy requires a replacement row.
CREATE TRIGGER agent_execution_step_snapshot_immutable
BEFORE UPDATE ON agent_execution_steps
WHEN NEW.id IS NOT OLD.id
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.title IS NOT OLD.title
  OR NEW.spec IS NOT OLD.spec
  OR NEW.role IS NOT OLD.role
  OR NEW.tool_policy IS NOT OLD.tool_policy
  OR NEW.kind IS NOT OLD.kind
  OR NEW.agent_mode IS NOT OLD.agent_mode
  OR NEW.profile IS NOT OLD.profile
  OR NEW.fanout_group IS NOT OLD.fanout_group
  OR NEW.control_policy IS NOT OLD.control_policy
  OR NEW.delegation_depth IS NOT OLD.delegation_depth
  OR NEW.assigned_participant_id IS NOT OLD.assigned_participant_id
  OR NEW.assignment_score IS NOT OLD.assignment_score
  OR NEW.assignment_rationale IS NOT OLD.assignment_rationale
  OR NEW.assignment_source IS NOT OLD.assignment_source
  OR NEW.assignment_locked IS NOT OLD.assignment_locked
  OR NEW.failure_policy IS NOT OLD.failure_policy
  OR NEW.preset_prompt IS NOT OLD.preset_prompt
  OR NEW.graph_x IS NOT OLD.graph_x
  OR NEW.graph_y IS NOT OLD.graph_y
  OR NEW.introduced_in_revision IS NOT OLD.introduced_in_revision
  OR NEW.created_at IS NOT OLD.created_at
  OR NEW.version < OLD.version
  OR NEW.updated_at < OLD.updated_at
  OR (
       NEW.superseded_in_revision IS NOT OLD.superseded_in_revision
       AND (
            OLD.superseded_in_revision IS NOT NULL
            OR NEW.superseded_in_revision IS NULL
            OR NEW.superseded_in_revision <= OLD.introduced_in_revision
       )
     )
  OR (OLD.superseded_in_revision IS NOT NULL
      AND (NEW.status IS NOT OLD.status
           OR NEW.dispatch_after IS NOT OLD.dispatch_after
           OR NEW.version IS NOT OLD.version
           OR NEW.updated_at IS NOT OLD.updated_at))
BEGIN
    SELECT RAISE(ABORT, 'execution step semantics are immutable; create a new plan revision');
END;

-- Defense in depth for every write path, including future repository methods.
-- Superseded history is deliberately excluded from the current-DAG ceiling.
CREATE TRIGGER agent_execution_active_step_limit
BEFORE INSERT ON agent_execution_steps
WHEN (SELECT COUNT(*) FROM agent_execution_steps
      WHERE execution_id = NEW.execution_id
        AND superseded_in_revision IS NULL) >= 128
BEGIN
    SELECT RAISE(ABORT, 'active Agent Execution DAG exceeds 128 steps');
END;

CREATE TRIGGER agent_execution_step_status_transition
BEFORE UPDATE OF status ON agent_execution_steps
WHEN NEW.status <> OLD.status AND NOT (
       (OLD.status = 'pending'
        AND NEW.status IN ('running', 'completed', 'failed', 'skipped', 'cancelled'))
    OR (OLD.status = 'running'
        AND NEW.status IN ('pending', 'waiting_input', 'completed', 'failed', 'cancelled'))
    OR (OLD.status = 'waiting_input'
        AND NEW.status IN ('pending', 'running', 'completed', 'failed', 'cancelled'))
    OR (OLD.status = 'completed' AND NEW.status = 'pending')
    OR (OLD.status IN ('failed', 'skipped') AND NEW.status IN ('pending', 'completed'))
)
BEGIN
    SELECT RAISE(ABORT, 'invalid agent execution step status transition');
END;

CREATE TABLE agent_execution_step_dependencies (
    execution_id   TEXT NOT NULL,
    blocker_step_id TEXT NOT NULL,
    blocked_step_id TEXT NOT NULL,
    introduced_in_revision INTEGER NOT NULL CHECK (introduced_in_revision >= 0),
    superseded_in_revision INTEGER CHECK (
        superseded_in_revision IS NULL OR superseded_in_revision > introduced_in_revision
    ),
    PRIMARY KEY (execution_id, blocker_step_id, blocked_step_id, introduced_in_revision),
    FOREIGN KEY (execution_id, blocker_step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    FOREIGN KEY (execution_id, blocked_step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    CHECK (blocker_step_id <> blocked_step_id)
);
CREATE INDEX idx_agent_execution_dependencies_blocked
    ON agent_execution_step_dependencies(execution_id, blocked_step_id);
CREATE UNIQUE INDEX idx_agent_execution_dependencies_active
    ON agent_execution_step_dependencies(execution_id, blocker_step_id, blocked_step_id)
    WHERE superseded_in_revision IS NULL;
CREATE INDEX idx_agent_execution_dependencies_active_blocked
    ON agent_execution_step_dependencies(execution_id, blocked_step_id)
    WHERE superseded_in_revision IS NULL;

CREATE TRIGGER agent_execution_dependency_lifecycle
BEFORE UPDATE ON agent_execution_step_dependencies
WHEN OLD.superseded_in_revision IS NOT NULL
  OR NEW.superseded_in_revision IS NULL
  OR NEW.superseded_in_revision <= OLD.introduced_in_revision
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.blocker_step_id IS NOT OLD.blocker_step_id
  OR NEW.blocked_step_id IS NOT OLD.blocked_step_id
  OR NEW.introduced_in_revision IS NOT OLD.introduced_in_revision
BEGIN
    SELECT RAISE(ABORT, 'dependency revision is immutable; only first supersession is allowed');
END;

CREATE TABLE agent_execution_attempts (
    id                   TEXT NOT NULL,
    execution_id         TEXT NOT NULL,
    step_id              TEXT NOT NULL,
    attempt_no           INTEGER NOT NULL CHECK (attempt_no >= 0),
    participant_id       TEXT,
    status               TEXT NOT NULL CHECK (status IN (
                             'queued', 'running', 'waiting_input', 'completed',
                             'failed', 'cancelled', 'interrupted'
                         )),
    trigger_reason       TEXT NOT NULL CHECK (trim(trigger_reason) <> ''),
    effective_config     TEXT NOT NULL DEFAULT '{}'
                             CHECK (json_valid(effective_config) AND json_type(effective_config) = 'object'),
    question             TEXT,
    error                TEXT,
    output_summary       TEXT,
    output_files         TEXT NOT NULL DEFAULT '[]'
                             CHECK (json_valid(output_files) AND json_type(output_files) = 'array'),
    tokens               INTEGER CHECK (tokens IS NULL OR tokens >= 0),
    retry_after          INTEGER CHECK (retry_after IS NULL OR retry_after >= 0),
    runtime_state        TEXT CHECK (
                             runtime_state IS NULL
                             OR (json_valid(runtime_state) AND json_type(runtime_state) = 'object')
                         ),
    started_at           INTEGER,
    finished_at          INTEGER,
    version              INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    PRIMARY KEY (execution_id, step_id, id),
    UNIQUE (execution_id, step_id, attempt_no),
    FOREIGN KEY (execution_id, step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    FOREIGN KEY (execution_id, participant_id)
        REFERENCES agent_execution_participants(execution_id, id),
    CHECK (
        (status IN ('completed', 'failed', 'cancelled', 'interrupted') AND finished_at IS NOT NULL)
        OR (status NOT IN ('completed', 'failed', 'cancelled', 'interrupted') AND finished_at IS NULL)
    ),
    CHECK (
        (status = 'queued' AND started_at IS NULL)
        OR (status IN ('running', 'waiting_input', 'completed', 'failed', 'interrupted')
            AND started_at IS NOT NULL)
        OR status = 'cancelled'
    ),
    CHECK (
        (status = 'waiting_input' AND trim(COALESCE(question, '')) <> '')
        OR (status <> 'waiting_input' AND question IS NULL)
    )
);
CREATE UNIQUE INDEX idx_agent_execution_attempts_one_active
    ON agent_execution_attempts(execution_id, step_id)
    WHERE status IN ('queued', 'running', 'waiting_input');
CREATE INDEX idx_agent_execution_attempts_step
    ON agent_execution_attempts(execution_id, step_id, attempt_no DESC);

CREATE TRIGGER agent_execution_attempt_kind_guard_insert
BEFORE INSERT ON agent_execution_attempts
WHEN EXISTS (
    SELECT 1
    FROM agent_execution_steps step
    WHERE step.execution_id = NEW.execution_id AND step.id = NEW.step_id
      AND (
          step.superseded_in_revision IS NOT NULL
          OR (step.kind = 'agent' AND NEW.participant_id IS NULL)
          OR (step.kind IN ('verify', 'judge', 'loop') AND (
              NEW.participant_id IS NOT NULL OR NEW.status = 'queued'
          ))
      )
)
BEGIN
    SELECT RAISE(ABORT, 'attempt kind/participant does not match its active step');
END;

CREATE TRIGGER agent_execution_attempt_kind_guard_update
BEFORE UPDATE ON agent_execution_attempts
WHEN EXISTS (
    SELECT 1
    FROM agent_execution_steps step
    WHERE step.execution_id = NEW.execution_id AND step.id = NEW.step_id
      AND (
          step.superseded_in_revision IS NOT NULL
          OR (step.kind = 'agent' AND NEW.participant_id IS NULL)
          OR (step.kind IN ('verify', 'judge', 'loop') AND (
              NEW.participant_id IS NOT NULL OR NEW.status = 'queued'
          ))
      )
)
BEGIN
    SELECT RAISE(ABORT, 'attempt kind/participant does not match its active step');
END;

CREATE TRIGGER agent_execution_attempt_runtime_snapshot_immutable
BEFORE UPDATE ON agent_execution_attempts
WHEN NEW.id IS NOT OLD.id
    OR NEW.execution_id IS NOT OLD.execution_id
    OR NEW.step_id IS NOT OLD.step_id
    OR NEW.attempt_no IS NOT OLD.attempt_no
    OR NEW.participant_id IS NOT OLD.participant_id
    OR NEW.trigger_reason <> OLD.trigger_reason
    OR NEW.effective_config <> OLD.effective_config
    OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'attempt identity and runtime snapshot are immutable');
END;

CREATE TRIGGER agent_execution_attempt_status_transition
BEFORE UPDATE OF status ON agent_execution_attempts
WHEN NEW.status <> OLD.status AND NOT (
       (OLD.status = 'queued' AND NEW.status IN ('running', 'cancelled'))
    OR (OLD.status = 'running'
        AND NEW.status IN ('waiting_input', 'completed', 'failed', 'cancelled', 'interrupted'))
    OR (OLD.status = 'waiting_input'
        AND NEW.status IN ('running', 'completed', 'failed', 'cancelled', 'interrupted'))
)
BEGIN
    SELECT RAISE(ABORT, 'invalid agent execution attempt status transition');
END;

CREATE TRIGGER agent_execution_settled_attempt_immutable
BEFORE UPDATE ON agent_execution_attempts
WHEN OLD.status IN ('completed', 'failed', 'cancelled', 'interrupted')
BEGIN
    SELECT RAISE(ABORT, 'settled execution attempt is immutable');
END;

CREATE TABLE conversation_execution_links (
    id              TEXT PRIMARY KEY NOT NULL,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    execution_id    TEXT NOT NULL REFERENCES agent_executions(id) ON DELETE CASCADE,
    relation        TEXT NOT NULL CHECK (relation IN ('lead', 'attempt')),
    step_id         TEXT,
    attempt_id      TEXT,
    active          INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    -- NULL is a durable cancellation/termination intent.  The execution
    -- engine marks this only after the Conversation runtime acknowledges the
    -- idempotent cleanup, so a process crash simply retries it at boot.
    cleanup_completed_at INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY (execution_id, step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    FOREIGN KEY (execution_id, step_id, attempt_id)
        REFERENCES agent_execution_attempts(execution_id, step_id, id) ON DELETE CASCADE,
    CHECK (
        (relation = 'lead' AND step_id IS NULL AND attempt_id IS NULL)
        OR (relation = 'attempt' AND step_id IS NOT NULL AND attempt_id IS NOT NULL)
    ),
    CHECK (cleanup_completed_at IS NULL OR (relation = 'attempt' AND active = 0))
);
CREATE UNIQUE INDEX idx_conversation_execution_active_lead
    ON conversation_execution_links(execution_id) WHERE relation = 'lead' AND active = 1;
CREATE UNIQUE INDEX idx_conversation_execution_active_attempt
    ON conversation_execution_links(execution_id, step_id, attempt_id)
    WHERE relation = 'attempt' AND active = 1;
CREATE UNIQUE INDEX idx_conversation_execution_active_attempt_conversation
    ON conversation_execution_links(conversation_id)
    WHERE relation = 'attempt' AND active = 1;
CREATE INDEX idx_conversation_execution_lookup
    ON conversation_execution_links(conversation_id, active DESC, created_at DESC);
CREATE INDEX idx_conversation_execution_execution
    ON conversation_execution_links(execution_id, relation, active);
CREATE INDEX idx_conversation_execution_pending_cleanup
    ON conversation_execution_links(execution_id, conversation_id)
    WHERE relation = 'attempt' AND active = 0 AND cleanup_completed_at IS NULL;

CREATE TRIGGER conversation_execution_link_owner_guard
BEFORE INSERT ON conversation_execution_links
WHEN (SELECT COUNT(*)
      FROM agent_executions execution
      JOIN conversations conversation ON conversation.id = NEW.conversation_id
      WHERE execution.id = NEW.execution_id
        AND execution.user_id = conversation.user_id) <> 1
BEGIN
    SELECT RAISE(ABORT, 'conversation execution link owner mismatch');
END;

-- Once a Conversation participates in a stable creation/delivery identity or
-- Execution audit relation, changing its owner would split the aggregate
-- across accounts. Normal account deletion still cascades these rows.
CREATE TRIGGER execution_conversation_owner_immutable
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id IS NOT OLD.user_id
 AND (
     EXISTS (SELECT 1 FROM conversation_creation_keys key
             WHERE key.conversation_id = OLD.id)
     OR EXISTS (SELECT 1 FROM conversation_delivery_receipts receipt
                WHERE receipt.conversation_id = OLD.id)
     OR EXISTS (SELECT 1 FROM conversation_execution_links link
                WHERE link.conversation_id = OLD.id)
 )
BEGIN
    SELECT RAISE(ABORT, 'execution conversation owner is immutable');
END;

-- Link identity is audit history.  A link may only move once from active to
-- inactive; replacement creates another row instead of rewriting ownership.
CREATE TRIGGER conversation_execution_link_identity_immutable
BEFORE UPDATE ON conversation_execution_links
WHEN NEW.id IS NOT OLD.id
  OR NEW.conversation_id IS NOT OLD.conversation_id
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.relation IS NOT OLD.relation
  OR NEW.step_id IS NOT OLD.step_id
  OR NEW.attempt_id IS NOT OLD.attempt_id
  OR NEW.created_at IS NOT OLD.created_at
  OR NEW.active > OLD.active
  OR (OLD.cleanup_completed_at IS NOT NULL
      AND NEW.cleanup_completed_at IS NOT OLD.cleanup_completed_at)
  OR NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'conversation execution link identity is immutable');
END;

CREATE TRIGGER conversation_execution_link_delete_guard
BEFORE DELETE ON conversation_execution_links
WHEN EXISTS (
        SELECT 1 FROM agent_executions execution
        JOIN users owner ON owner.id = execution.user_id
        WHERE execution.id = OLD.execution_id
     )
 AND EXISTS (SELECT 1 FROM conversations WHERE id = OLD.conversation_id)
BEGIN
    SELECT RAISE(ABORT, 'conversation execution links cannot be deleted directly');
END;

-- Attempt transcripts are part of the Execution audit record.  Direct
-- conversation deletion is rejected while the owner exists; deleting the
-- owner account may still cascade the complete aggregate.
CREATE TRIGGER agent_execution_attempt_conversation_retained
BEFORE DELETE ON conversations
WHEN EXISTS (
        SELECT 1 FROM conversation_execution_links link
        WHERE link.conversation_id = OLD.id AND link.relation = 'attempt'
     )
 AND EXISTS (SELECT 1 FROM users WHERE id = OLD.user_id)
BEGIN
    SELECT RAISE(ABORT, 'execution attempt conversation is retained for audit');
END;

-- An unfinished execution must not lose its authoritative lead link through a
-- conversation cascade.  Product deletion becomes legal again after the
-- execution settles; account deletion also remains a full aggregate cascade
-- because the owner row no longer exists while these guards run.
CREATE TRIGGER agent_execution_active_lead_conversation_retained
BEFORE DELETE ON conversations
WHEN EXISTS (
    SELECT 1
    FROM conversation_execution_links link
    JOIN agent_executions execution ON execution.id = link.execution_id
    JOIN users owner ON owner.id = execution.user_id
    WHERE link.conversation_id = OLD.id
      AND link.relation = 'lead' AND link.active = 1
      AND execution.deleted_at IS NULL
      AND execution.status NOT IN (
          'completed', 'completed_with_failures', 'failed', 'cancelled'
      )
)
BEGIN
    SELECT RAISE(ABORT, 'active Agent Execution lead conversation cannot be deleted');
END;

-- Transactional outbox.  Writers append a monotonically increasing per-execution
-- sequence in the same transaction as aggregate state changes.
CREATE TABLE agent_execution_events (
    id               TEXT PRIMARY KEY NOT NULL,
    execution_id     TEXT NOT NULL REFERENCES agent_executions(id) ON DELETE CASCADE,
    sequence         INTEGER NOT NULL CHECK (sequence > 0),
    event_type       TEXT NOT NULL CHECK (event_type IN (
        'created', 'migrated', 'status_changed', 'plan_changed',
        'step_changed', 'attempt_changed', 'decision_requested',
        'decision_answered', 'deleted'
    )),
    step_id          TEXT,
    attempt_id       TEXT,
    actor_type       TEXT NOT NULL CHECK (actor_type IN ('system', 'user', 'agent')),
    actor_id         TEXT,
    actor_conversation_id INTEGER,
    actor_attempt_id TEXT,
    on_behalf_of_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    payload          TEXT NOT NULL CHECK (json_valid(payload)),
    created_at       INTEGER NOT NULL,
    published_at     INTEGER,
    UNIQUE (execution_id, sequence),
    FOREIGN KEY (execution_id, step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    FOREIGN KEY (execution_id, step_id, attempt_id)
        REFERENCES agent_execution_attempts(execution_id, step_id, id) ON DELETE CASCADE,
    CHECK (attempt_id IS NULL OR step_id IS NOT NULL),
    CHECK (actor_attempt_id IS NULL OR trim(actor_attempt_id) <> ''),
    CHECK (
        (actor_type = 'system' AND actor_id IS NULL)
        OR (actor_type IN ('user', 'agent') AND trim(COALESCE(actor_id, '')) <> '')
    ),
    CHECK (
        (actor_type = 'system'
            AND actor_id IS NULL
            AND actor_conversation_id IS NULL
            AND actor_attempt_id IS NULL)
        OR (actor_type = 'user'
            AND actor_id = on_behalf_of_user_id
            AND actor_conversation_id IS NULL
            AND actor_attempt_id IS NULL)
        OR (actor_type = 'agent'
            AND trim(actor_id) <> ''
            AND (
                (actor_conversation_id IS NULL AND actor_attempt_id IS NULL)
                OR (actor_conversation_id > 0
                    AND actor_id = CAST(actor_conversation_id AS TEXT))
            ))
    )
);
CREATE INDEX idx_agent_execution_events_unpublished
    ON agent_execution_events(execution_id, sequence)
    WHERE published_at IS NULL;
CREATE UNIQUE INDEX idx_agent_execution_delegation_operation
    ON agent_execution_events(
        execution_id,
        json_extract(payload, '$.operation_id')
    )
    WHERE event_type = 'plan_changed'
      AND json_type(payload, '$.operation_id') = 'text';

-- Raw SQL cannot bypass caller attribution. The repository performs the same
-- checks before insert to return a domain conflict instead of a generic SQLite
-- constraint error; these triggers are the final integrity boundary.
CREATE TRIGGER agent_execution_event_owner_guard
BEFORE INSERT ON agent_execution_events
WHEN NEW.on_behalf_of_user_id IS NOT (
    SELECT execution.user_id FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'execution event on-behalf user must match execution owner');
END;

-- Sequence 1 is the unique immutable provenance baseline. Live writes use
-- Created; the one-time hard-cut migration writes Migrated. Every later fact
-- must be the next sequence already reserved on the aggregate.
CREATE TRIGGER agent_execution_event_sequence_guard
BEFORE INSERT ON agent_execution_events
WHEN NEW.sequence IS NOT (
         SELECT execution.event_sequence FROM agent_executions execution
         WHERE execution.id = NEW.execution_id
     )
  OR NEW.sequence <> COALESCE((
         SELECT max(event.sequence) FROM agent_execution_events event
         WHERE event.execution_id = NEW.execution_id
     ), 0) + 1
  OR (NEW.sequence = 1 AND NEW.event_type NOT IN ('created', 'migrated'))
  OR (NEW.sequence <> 1 AND NEW.event_type IN ('created', 'migrated'))
BEGIN
    SELECT RAISE(ABORT, 'execution event sequence or baseline kind is invalid');
END;

-- An external Agent establishes its stable identity on this Execution's
-- Created baseline. Every later external write must match that initiator.
-- Conversation-backed Agents are authorized separately by their active typed
-- link below.
CREATE TRIGGER agent_execution_event_external_actor_guard
BEFORE INSERT ON agent_execution_events
WHEN NEW.actor_type = 'agent'
 AND NEW.actor_conversation_id IS NULL
 AND NOT (
     (NEW.sequence = 1
      AND NEW.event_type = 'created')
     OR EXISTS (
         SELECT 1
         FROM agent_execution_events baseline
         WHERE baseline.execution_id = NEW.execution_id
          AND baseline.sequence = 1
          AND baseline.event_type = 'created'
          AND baseline.actor_type = 'agent'
          AND baseline.actor_conversation_id IS NULL
          AND baseline.actor_attempt_id IS NULL
          AND baseline.actor_id = NEW.actor_id
     )
 )
BEGIN
    SELECT RAISE(ABORT, 'external Agent actor must match execution initiator');
END;

CREATE TRIGGER agent_execution_event_agent_link_guard
BEFORE INSERT ON agent_execution_events
WHEN NEW.actor_type = 'agent'
 AND NEW.actor_conversation_id IS NOT NULL
 AND (
    (SELECT COUNT(*) FROM conversation_execution_links link
     JOIN agent_executions execution ON execution.id = link.execution_id
     WHERE link.execution_id = NEW.execution_id
       AND link.conversation_id = NEW.actor_conversation_id
       AND link.active = 1
       AND execution.user_id = NEW.on_behalf_of_user_id
       AND execution.deleted_at IS NULL) <> 1
    OR EXISTS (
        SELECT 1 FROM conversation_execution_links link
        WHERE link.execution_id = NEW.execution_id
          AND link.conversation_id = NEW.actor_conversation_id
          AND link.active = 1
          AND link.relation = 'attempt'
          AND link.attempt_id IS NOT NEW.actor_attempt_id
    )
    OR (NEW.actor_attempt_id IS NOT NULL AND
        (SELECT COUNT(*) FROM conversation_execution_links link
         JOIN agent_executions execution ON execution.id = link.execution_id
         WHERE link.conversation_id = NEW.actor_conversation_id
           AND link.attempt_id = NEW.actor_attempt_id
           AND link.relation = 'attempt'
           AND link.active = 1
           AND execution.user_id = NEW.on_behalf_of_user_id
           AND execution.deleted_at IS NULL) <> 1)
)
BEGIN
    SELECT RAISE(ABORT, 'execution event Agent actor requires one active caller link');
END;

-- Delivery bookkeeping is mutable; the committed domain fact is not.
CREATE TRIGGER agent_execution_event_fact_immutable
BEFORE UPDATE ON agent_execution_events
WHEN NEW.id IS NOT OLD.id
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.sequence IS NOT OLD.sequence
  OR NEW.event_type IS NOT OLD.event_type
  OR NEW.step_id IS NOT OLD.step_id
  OR NEW.attempt_id IS NOT OLD.attempt_id
  OR NEW.actor_type IS NOT OLD.actor_type
  OR NEW.actor_id IS NOT OLD.actor_id
  OR NEW.actor_conversation_id IS NOT OLD.actor_conversation_id
  OR NEW.actor_attempt_id IS NOT OLD.actor_attempt_id
  OR NEW.on_behalf_of_user_id IS NOT OLD.on_behalf_of_user_id
  OR NEW.payload IS NOT OLD.payload
  OR NEW.created_at IS NOT OLD.created_at
  OR (OLD.published_at IS NOT NULL AND NEW.published_at IS NOT OLD.published_at)
BEGIN
    SELECT RAISE(ABORT, 'agent execution event fact is immutable');
END;

-- All five child collections are append-only audit history.  Runtime/product
-- code may retire, supersede or settle rows but never physically delete them.
-- Joining through the live owner deliberately permits only the FK cascade
-- initiated by deleting that owner account.
CREATE TRIGGER agent_execution_participant_delete_guard
BEFORE DELETE ON agent_execution_participants
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution participants cannot be deleted directly');
END;

CREATE TRIGGER agent_execution_step_delete_guard
BEFORE DELETE ON agent_execution_steps
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution steps cannot be deleted directly');
END;

CREATE TRIGGER agent_execution_dependency_delete_guard
BEFORE DELETE ON agent_execution_step_dependencies
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution dependencies cannot be deleted directly');
END;

CREATE TRIGGER agent_execution_attempt_delete_guard
BEFORE DELETE ON agent_execution_attempts
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution attempts cannot be deleted directly');
END;

CREATE TRIGGER agent_execution_event_delete_guard
BEFORE DELETE ON agent_execution_events
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution events cannot be deleted directly');
END;

-- Runs become executions. A run-bound workspace contributes only its fallback
-- working directory; the reusable OrchestrationWorkspace resource itself is
-- intentionally removed by this hard cut. Legacy `forked_from` is retained
-- only in the Migrated event payload below; every Execution is independent.
INSERT INTO agent_executions (
    id, user_id, goal, status, plan_gate, adaptation_policy, decision_policy,
    delegation_policy, max_parallel, work_dir, initial_plan_input,
    summary, total_tokens, version,
    plan_revision, event_sequence, created_at, updated_at
)
SELECT
    r.id,
    r.user_id,
    r.goal,
    CASE
        WHEN r.status = 'awaiting_plan_approval' THEN 'awaiting_approval'
        WHEN r.status = 'running' AND EXISTS (
            SELECT 1 FROM orch_run_tasks t
            WHERE t.run_id = r.id AND t.status = 'needs_review'
        ) THEN 'waiting_input'
        ELSE r.status
    END,
    CASE r.autonomy WHEN 'interactive' THEN 'require_approval' ELSE 'automatic' END,
    CASE r.autonomy
        WHEN 'interactive' THEN 'fixed'
        WHEN 'supervised' THEN 'fixed'
        WHEN 'autonomous' THEN 'adaptive'
    END,
    CASE COALESCE(r.approval_mode, 'auto') WHEN 'manual' THEN 'ask_user' ELSE 'automatic' END,
    COALESCE(
        (SELECT c.delegation_policy FROM conversations c WHERE c.id = r.lead_conv_id),
        (SELECT c.delegation_policy FROM conversations c
         WHERE json_extract(c.extra, '$.orchestrator_run_id') = r.id
           AND json_extract(c.extra, '$.orchestrator_task_id') IS NULL
         LIMIT 1),
        'automatic'
    ),
    COALESCE(r.max_parallel, 4),
    COALESCE(r.work_dir, w.workspace_dir),
    '{"mode":"automatic"}',
    r.summary,
    r.total_tokens,
    0,
    0,
    1,
    r.created_at,
    r.updated_at
FROM orch_runs r
LEFT JOIN orch_workspaces w ON w.id = r.workspace_id;

-- Expand every frozen fleet member into a first-class immutable participant.
INSERT INTO agent_execution_participants (
    id, execution_id, source_agent_id, preset_id, preset_revision, preset_snapshot,
    provider_id, model, role, capability, constraints, description, system_prompt,
    enabled_skills, disabled_builtin_skills, sort_order,
    introduced_in_revision, retired_in_revision, created_at
)
SELECT
    json_extract(m.value, '$.id'),
    r.id,
    COALESCE(NULLIF(trim(json_extract(m.value, '$.agent_id')), ''), 'nomi'),
    json_extract(m.value, '$.preset_id'),
    json_extract(m.value, '$.preset_revision'),
    CASE WHEN json_type(m.value, '$.preset_snapshot') = 'object' THEN
        CASE WHEN json_extract(m.value, '$.preset_snapshot.target') = 'cluster_member'
             THEN json_set(
                 json(json_extract(m.value, '$.preset_snapshot')),
                 '$.target', 'execution_step'
             )
             ELSE json(json_extract(m.value, '$.preset_snapshot'))
        END
    END,
    COALESCE(
        NULLIF(trim(json_extract(m.value, '$.provider_id')), ''),
        NULLIF(trim(json_extract(m.value, '$.preset_snapshot.resolved_model.provider_id')), '')
    ),
    COALESCE(
        NULLIF(trim(json_extract(m.value, '$.model')), ''),
        NULLIF(trim(json_extract(m.value, '$.preset_snapshot.resolved_model.model')), '')
    ),
    json_extract(m.value, '$.role_hint'),
    CASE
        WHEN json_type(m.value, '$.capability_profile') = 'object'
            THEN json(json_extract(m.value, '$.capability_profile'))
        WHEN json_type(m.value, '$.constraints.cost_tier') = 'text'
            THEN json_object(
                'strengths', json('[]'),
                'modalities', json('[]'),
                'tools', json('false'),
                'reasoning', 'medium',
                'cost_tier', json_extract(m.value, '$.constraints.cost_tier'),
                'speed_tier', 'standard'
            )
    END,
    CASE WHEN json_type(m.value, '$.constraints') = 'object' THEN
        CASE WHEN json_type(m.value, '$.constraints.allowed_task_kinds') = 'array' THEN
            json_remove(
                json_set(
                    json_replace(
                        json(json_extract(m.value, '$.constraints')),
                        '$.max_concurrency',
                        min(json_extract(m.value, '$.constraints.max_concurrency'), 64)
                    ),
                    '$.allowed_profile_kinds',
                    json((
                        SELECT json_group_array(mapped) FROM (
                            SELECT DISTINCT trim(kind.value) AS mapped
                            FROM json_each(
                                json_extract(m.value, '$.constraints.allowed_task_kinds')
                            ) kind
                            ORDER BY mapped
                        )
                    ))
                ),
                '$.allowed_task_kinds',
                '$.cost_tier'
            )
        ELSE json_remove(
            json_replace(
                json(json_extract(m.value, '$.constraints')),
                '$.max_concurrency',
                min(json_extract(m.value, '$.constraints.max_concurrency'), 64)
            ),
            '$.allowed_task_kinds',
            '$.cost_tier'
        ) END
    END,
    json_extract(m.value, '$.description'),
    json_extract(m.value, '$.system_prompt'),
    CASE WHEN json_type(m.value, '$.enabled_skills') = 'array'
         THEN json(json_extract(m.value, '$.enabled_skills')) ELSE '[]' END,
    CASE WHEN json_type(m.value, '$.disabled_builtin_skills') = 'array'
         THEN json(json_extract(m.value, '$.disabled_builtin_skills')) ELSE '[]' END,
    COALESCE(json_extract(m.value, '$.sort_order'), CAST(m.key AS INTEGER)),
    0,
    NULL,
    r.created_at
FROM orch_runs r, json_each(r.fleet_snapshot) m;

-- A per-step model override is itself a frozen participant snapshot.  This keeps
-- participants immutable and avoids leaking provider/model execution data onto steps.
WITH override_base AS (
    SELECT t.*, r.fleet_snapshot,
           COALESCE(
               (SELECT a.member_id FROM orch_assignments a WHERE a.task_id = t.id),
               (SELECT json_extract(m.value, '$.id')
                FROM json_each(r.fleet_snapshot) m ORDER BY CAST(m.key AS INTEGER) LIMIT 1)
           ) AS base_member_id
    FROM orch_run_tasks t
    JOIN orch_runs r ON r.id = t.run_id
    WHERE t.override_provider_id IS NOT NULL
)
INSERT INTO agent_execution_participants (
    id, execution_id, source_agent_id, preset_id, preset_revision, preset_snapshot,
    provider_id, model, role, capability, constraints, description, system_prompt,
    enabled_skills, disabled_builtin_skills, sort_order,
    introduced_in_revision, retired_in_revision, created_at
)
SELECT
    'execpart_override_' || b.id,
    b.run_id,
    COALESCE(NULLIF(trim(json_extract(m.value, '$.agent_id')), ''), 'nomi'),
    json_extract(m.value, '$.preset_id'),
    json_extract(m.value, '$.preset_revision'),
    CASE WHEN json_type(m.value, '$.preset_snapshot') = 'object' THEN
        CASE WHEN json_extract(m.value, '$.preset_snapshot.target') = 'cluster_member'
             THEN json_set(
                 json(json_extract(m.value, '$.preset_snapshot')),
                 '$.target', 'execution_step'
             )
             ELSE json(json_extract(m.value, '$.preset_snapshot'))
        END
    END,
    trim(b.override_provider_id),
    trim(b.override_model),
    json_extract(m.value, '$.role_hint'),
    CASE
        WHEN json_type(m.value, '$.capability_profile') = 'object'
            THEN json(json_extract(m.value, '$.capability_profile'))
        WHEN json_type(m.value, '$.constraints.cost_tier') = 'text'
            THEN json_object(
                'strengths', json('[]'),
                'modalities', json('[]'),
                'tools', json('false'),
                'reasoning', 'medium',
                'cost_tier', json_extract(m.value, '$.constraints.cost_tier'),
                'speed_tier', 'standard'
            )
    END,
    CASE WHEN json_type(m.value, '$.constraints') = 'object' THEN
        CASE WHEN json_type(m.value, '$.constraints.allowed_task_kinds') = 'array' THEN
            json_remove(
                json_set(
                    json_replace(
                        json(json_extract(m.value, '$.constraints')),
                        '$.max_concurrency',
                        min(json_extract(m.value, '$.constraints.max_concurrency'), 64)
                    ),
                    '$.allowed_profile_kinds',
                    json((
                        SELECT json_group_array(mapped) FROM (
                            SELECT DISTINCT trim(kind.value) AS mapped
                            FROM json_each(
                                json_extract(m.value, '$.constraints.allowed_task_kinds')
                            ) kind
                            ORDER BY mapped
                        )
                    ))
                ),
                '$.allowed_task_kinds',
                '$.cost_tier'
            )
        ELSE json_remove(
            json_replace(
                json(json_extract(m.value, '$.constraints')),
                '$.max_concurrency',
                min(json_extract(m.value, '$.constraints.max_concurrency'), 64)
            ),
            '$.allowed_task_kinds',
            '$.cost_tier'
        ) END
    END,
    json_extract(m.value, '$.description'),
    json_extract(m.value, '$.system_prompt'),
    CASE WHEN json_type(m.value, '$.enabled_skills') = 'array'
         THEN json(json_extract(m.value, '$.enabled_skills')) ELSE '[]' END,
    CASE WHEN json_type(m.value, '$.disabled_builtin_skills') = 'array'
         THEN json(json_extract(m.value, '$.disabled_builtin_skills')) ELSE '[]' END,
    COALESCE(json_extract(m.value, '$.sort_order'), CAST(m.key AS INTEGER)),
    0,
    NULL,
    b.created_at
FROM override_base b
JOIN json_each(b.fleet_snapshot) m
  ON json_extract(m.value, '$.id') = b.base_member_id;

-- Bound every future current participant set at the database boundary.  This
-- trigger is intentionally created only after both legacy participant INSERTs:
-- migration must preserve every frozen historical snapshot before enforcing
-- the new execution-complexity ceiling on runtime/replan writes.
CREATE TRIGGER agent_execution_active_participant_limit
BEFORE INSERT ON agent_execution_participants
WHEN NEW.retired_in_revision IS NULL
 AND (SELECT COUNT(*) FROM agent_execution_participants
      WHERE execution_id = NEW.execution_id
        AND retired_in_revision IS NULL) >= 64
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution exceeds 64 active participants');
END;

-- Assignment is folded into the step.  Legacy unassigned executable nodes retain
-- the engine's former deterministic fallback to the first frozen participant.
INSERT INTO agent_execution_steps (
    id, execution_id, title, spec, role, tool_policy, kind, agent_mode, profile,
    fanout_group, control_policy, delegation_depth,
    status, assigned_participant_id, assignment_score, assignment_rationale,
    assignment_source, assignment_locked, failure_policy, preset_prompt,
    graph_x, graph_y, dispatch_after, version, introduced_in_revision, superseded_in_revision,
    created_at, updated_at
)
SELECT
    t.id,
    t.run_id,
    t.title,
    t.spec,
    t.role,
    CASE
        WHEN t.kind NOT IN ('agent', 'synthesis') THEN 'full'
        ELSE CASE lower(trim(COALESCE(t.role, '')))
            WHEN 'searcher' THEN 'read_only'
            WHEN 'scout' THEN 'read_only'
            WHEN 'reviewer' THEN 'read_only'
            WHEN 'verifier' THEN 'read_shell'
            WHEN 'tester' THEN 'read_shell'
            ELSE 'full'
        END
    END,
    CASE t.kind WHEN 'synthesis' THEN 'agent' ELSE t.kind END,
    CASE t.kind
        WHEN 'agent' THEN 'normal'
        WHEN 'synthesis' THEN 'synthesis'
        ELSE NULL
    END,
    t.task_profile,
    CASE WHEN t.kind IN ('agent', 'synthesis')
         THEN json_extract(t.pattern_config, '$.group') END,
    CASE t.kind
        WHEN 'verify' THEN json_object(
            'kind', 'verify',
            'vote', CASE
                WHEN json_extract(t.pattern_config, '$.vote') = 'unanimous'
                    THEN json_object('mode', 'unanimous')
                WHEN json_type(t.pattern_config, '$.vote') = 'object'
                    THEN json_object(
                        'mode', 'at_least',
                        'count', json_extract(t.pattern_config, '$.vote.threshold')
                    )
                ELSE json_object('mode', 'majority')
            END
        )
        WHEN 'judge' THEN json_object(
            'kind', 'judge',
            'aggregation', COALESCE(json_extract(t.pattern_config, '$.aggregate'), 'mean'),
            'candidate_count', json_extract(t.pattern_config, '$.candidates')
        )
        WHEN 'loop' THEN json_object(
            'kind', 'loop',
            'max_iterations', COALESCE(json_extract(t.pattern_config, '$.max_iter'), 5),
            'stop', CASE json_extract(t.pattern_config, '$.stop.kind')
                WHEN 'predicate' THEN json_object(
                    'kind', 'predicate',
                    'done_marker', json_extract(t.pattern_config, '$.stop.done_marker')
                )
                WHEN 'dry' THEN json_object(
                    'kind', 'stable',
                    'quiet_rounds', COALESCE(
                        json_extract(t.pattern_config, '$.stop.quiet_rounds'), 1
                    )
                )
                WHEN 'approved' THEN json_object('kind', 'approved')
                WHEN 'verdict' THEN json_object('kind', 'approved')
                WHEN 'verify' THEN json_object('kind', 'approved')
                ELSE json_object('kind', 'max_iterations')
            END
        )
        ELSE NULL
    END,
    max(
        COALESCE(json_extract(t.pattern_config, '$.delegation_depth'), 0),
        legacy_depth.effective_depth
    ),
    CASE t.status
        WHEN 'running' THEN 'pending'
        WHEN 'needs_review' THEN 'waiting_input'
        WHEN 'done' THEN 'completed'
        ELSE t.status
    END,
    CASE
        WHEN t.kind IN ('verify', 'judge', 'loop') THEN NULL
        WHEN t.override_provider_id IS NOT NULL THEN 'execpart_override_' || t.id
        ELSE COALESCE(
            a.member_id,
            (SELECT json_extract(m.value, '$.id')
             FROM orch_runs fallback_run, json_each(fallback_run.fleet_snapshot) m
             WHERE fallback_run.id = t.run_id
             ORDER BY CAST(m.key AS INTEGER) LIMIT 1)
        )
    END,
    CASE WHEN t.kind IN ('verify', 'judge', 'loop') THEN NULL ELSE a.score END,
    CASE WHEN t.kind IN ('verify', 'judge', 'loop') THEN NULL ELSE a.rationale END,
    CASE
        WHEN t.kind IN ('verify', 'judge', 'loop') THEN NULL
        WHEN a.id IS NULL THEN 'automatic'
        WHEN a.source IN ('override', 'manual') THEN 'manual'
        ELSE 'automatic'
    END,
    CASE
        WHEN t.kind IN ('verify', 'judge', 'loop') THEN 0
        ELSE COALESCE(a.locked, 0)
    END,
    CASE COALESCE(t.on_fail, 'fail_run')
        WHEN 'skip_and_continue' THEN 'skip_dependents'
        ELSE 'fail_execution'
    END,
    t.preset_prompt,
    t.graph_x,
    t.graph_y,
    CASE WHEN t.status = 'pending' THEN t.next_retry_at END,
    0,
    0,
    NULL,
    t.created_at,
    t.updated_at
FROM orch_run_tasks t
JOIN _m037_legacy_execution_depths legacy_depth ON legacy_depth.run_id = t.run_id
LEFT JOIN orch_assignments a ON a.task_id = t.id;

INSERT INTO agent_execution_step_dependencies (
    execution_id, blocker_step_id, blocked_step_id,
    introduced_in_revision, superseded_in_revision
)
SELECT blocker.run_id, d.blocker_task_id, d.blocked_task_id, 0, NULL
FROM orch_run_task_deps d
JOIN orch_run_tasks blocker ON blocker.id = d.blocker_task_id;

-- Plan revision is the sole clock for immutable graph snapshots. Future
-- writers cannot backfill invented history or retire/supersede a snapshot at
-- a revision other than the aggregate's currently committed revision.
CREATE TRIGGER agent_execution_participant_revision_insert_guard
BEFORE INSERT ON agent_execution_participants
WHEN NEW.introduced_in_revision IS NOT (
         SELECT execution.plan_revision FROM agent_executions execution
         WHERE execution.id = NEW.execution_id
     )
  OR NEW.retired_in_revision IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'participant must be introduced in the current plan revision');
END;

CREATE TRIGGER agent_execution_participant_revision_retire_guard
BEFORE UPDATE OF retired_in_revision ON agent_execution_participants
WHEN NEW.retired_in_revision IS NOT (
    SELECT execution.plan_revision FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'participant must retire in the current plan revision');
END;

CREATE TRIGGER agent_execution_step_revision_insert_guard
BEFORE INSERT ON agent_execution_steps
WHEN NEW.introduced_in_revision IS NOT (
         SELECT execution.plan_revision FROM agent_executions execution
         WHERE execution.id = NEW.execution_id
     )
  OR NEW.superseded_in_revision IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'step must be introduced in the current plan revision');
END;

CREATE TRIGGER agent_execution_step_revision_supersede_guard
BEFORE UPDATE OF superseded_in_revision ON agent_execution_steps
WHEN NEW.superseded_in_revision IS NOT (
    SELECT execution.plan_revision FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'step must be superseded in the current plan revision');
END;

CREATE TRIGGER agent_execution_dependency_revision_insert_guard
BEFORE INSERT ON agent_execution_step_dependencies
WHEN NEW.introduced_in_revision IS NOT (
         SELECT execution.plan_revision FROM agent_executions execution
         WHERE execution.id = NEW.execution_id
     )
  OR NEW.superseded_in_revision IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'dependency must be introduced in the current plan revision');
END;

CREATE TRIGGER agent_execution_dependency_revision_supersede_guard
BEFORE UPDATE OF superseded_in_revision ON agent_execution_step_dependencies
WHEN NEW.superseded_in_revision IS NOT (
    SELECT execution.plan_revision FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'dependency must be superseded in the current plan revision');
END;

-- Preserve the legacy current attempt as one historical attempt row. A legacy
-- running worker cannot survive process replacement: the old engine recovered
-- it by resetting the task to pending, so migration records the abandoned call
-- as Interrupted, deactivates its Conversation link below, and leaves the Step
-- pending for a fresh Attempt. Pending Agent attempts remain queued. Recovery
-- cancels (never interrupts) this
-- not-yet-started reservation and keeps the step pending under both fixed and
-- adaptive policies. Pending control attempts become an explicitly-labelled
-- cancelled compatibility reservation because the unified runtime creates
-- control attempts directly in the running state; their step remains pending
-- and can be scheduled normally.
-- Outputs, errors, retry gates, conversation, and tokens live here only.
INSERT INTO agent_execution_attempts (
    id, execution_id, step_id, attempt_no, participant_id, status,
    trigger_reason, effective_config, question, error, output_summary, output_files,
    tokens, retry_after, runtime_state, started_at, finished_at, version,
    created_at, updated_at
)
SELECT
    'execattempt_migrated_' || t.id || '_' || t.attempt,
    t.run_id,
    t.id,
    t.attempt,
    s.assigned_participant_id,
    CASE t.status
        WHEN 'pending' THEN CASE
            WHEN t.kind IN ('verify', 'judge', 'loop') THEN 'cancelled'
            ELSE 'queued'
        END
        WHEN 'running' THEN 'interrupted'
        WHEN 'needs_review' THEN 'waiting_input'
        WHEN 'done' THEN 'completed'
        WHEN 'skipped' THEN 'cancelled'
        ELSE t.status
    END,
    CASE
        WHEN t.status = 'pending' AND t.kind IN ('verify', 'judge', 'loop')
            THEN 'migrated_unstarted_control_reservation'
        ELSE 'migrated_current_attempt'
    END,
    json_object(
        'participant_id', s.assigned_participant_id,
        'decision_policy', e.decision_policy,
        'adaptation_policy', e.adaptation_policy,
        'legacy_configured_conversation_id', t.conversation_id,
        'legacy_conversation_id', current_conversation.conversation_id,
        'legacy_conversation_source', COALESCE(
            current_conversation.current_source,
            'none'
        )
    ),
    t.pending_question,
    t.last_error,
    t.output_summary,
    COALESCE(t.output_files, '[]'),
    t.tokens,
    t.next_retry_at,
    CASE WHEN json_extract(t.pattern_config, '$.loop_prior_output') IS NOT NULL
              OR json_extract(t.pattern_config, '$.loop_iteration') IS NOT NULL
         THEN json_object(
             'loop_prior_output', json_extract(t.pattern_config, '$.loop_prior_output'),
             'loop_iteration', json_extract(t.pattern_config, '$.loop_iteration')
         )
    END,
    CASE WHEN t.status <> 'pending' THEN COALESCE(
        current_conversation.created_at,
        t.created_at
    ) END,
    CASE
        WHEN t.status IN ('running', 'done', 'failed', 'skipped', 'cancelled')
          OR (t.status = 'pending' AND t.kind IN ('verify', 'judge', 'loop'))
        THEN COALESCE(current_conversation.updated_at, t.updated_at)
    END,
    0,
    COALESCE(current_conversation.created_at, t.created_at),
    COALESCE(current_conversation.updated_at, t.updated_at)
FROM orch_run_tasks t
JOIN agent_execution_steps s ON s.execution_id = t.run_id AND s.id = t.id
JOIN agent_executions e ON e.id = t.run_id
LEFT JOIN _m037_attempt_conversation_stage current_conversation
  ON current_conversation.execution_id = t.run_id
 AND current_conversation.step_id = t.id
 AND current_conversation.is_current = 1;

-- Older retry Conversations are real Attempt audit history even though the
-- legacy task row retained detailed status/output only for its current
-- generation. Preserve each transcript as a settled Interrupted Attempt: it
-- was superseded by a later generation, and inventing completed/failed output
-- from message text would be less truthful than this explicit classification.
INSERT INTO agent_execution_attempts (
    id, execution_id, step_id, attempt_no, participant_id, status,
    trigger_reason, effective_config, question, error, output_summary, output_files,
    tokens, retry_after, runtime_state, started_at, finished_at, version,
    created_at, updated_at
)
SELECT
    'execattempt_migrated_' || history.step_id || '_' || history.attempt_no,
    history.execution_id,
    history.step_id,
    history.attempt_no,
    step.assigned_participant_id,
    'interrupted',
    'migrated_superseded_retry_transcript',
    json_object(
        'participant_id', step.assigned_participant_id,
        'decision_policy', execution.decision_policy,
        'adaptation_policy', execution.adaptation_policy,
        'legacy_conversation_id', history.conversation_id,
        'legacy_attempt_no_inferred', json('true'),
        'legacy_candidate_ordinal', history.candidate_ordinal,
        'legacy_candidate_count', history.candidate_count,
        'legacy_numbering', 'right_aligned'
    ),
    NULL,
    NULL,
    NULL,
    '[]',
    NULL,
    NULL,
    NULL,
    history.created_at,
    history.updated_at,
    0,
    history.created_at,
    history.updated_at
FROM _m037_attempt_conversation_stage history
JOIN agent_execution_steps step
  ON step.execution_id = history.execution_id AND step.id = history.step_id
JOIN agent_executions execution ON execution.id = history.execution_id
WHERE history.is_current = 0;

-- Attempts are append-only generations of the current step snapshot. The
-- migration copied arbitrary legacy attempt numbers before this trigger; all
-- future writes continue contiguously from that preserved maximum.
CREATE TRIGGER agent_execution_attempt_insert_guard
BEFORE INSERT ON agent_execution_attempts
WHEN NOT EXISTS (
         SELECT 1 FROM agent_execution_steps step
         WHERE step.execution_id = NEW.execution_id
           AND step.id = NEW.step_id
           AND step.superseded_in_revision IS NULL
     )
  OR NEW.attempt_no IS NOT COALESCE((
         SELECT max(attempt.attempt_no) + 1
         FROM agent_execution_attempts attempt
         WHERE attempt.execution_id = NEW.execution_id
           AND attempt.step_id = NEW.step_id
     ), 0)
BEGIN
    SELECT RAISE(ABORT, 'attempt must append contiguously to the current step');
END;

-- Explicit links replace all conversation extra identity fields.
WITH lead_candidates AS (
    SELECT r.id AS execution_id, r.lead_conv_id AS conversation_id, r.created_at
    FROM orch_runs r
    JOIN conversations conversation ON conversation.id = r.lead_conv_id
    UNION
    SELECT json_extract(c.extra, '$.orchestrator_run_id'), c.id, c.created_at
    FROM conversations c
    WHERE json_extract(c.extra, '$.orchestrator_run_id') IS NOT NULL
      AND json_extract(c.extra, '$.orchestrator_task_id') IS NULL
)
INSERT INTO conversation_execution_links (
    id, conversation_id, execution_id, relation, step_id, attempt_id,
    active, created_at, updated_at
)
SELECT 'execlink_lead_' || execution_id, conversation_id, execution_id,
       'lead', NULL, NULL, 1, MIN(created_at), MIN(created_at)
FROM lead_candidates
GROUP BY execution_id, conversation_id;

INSERT INTO conversation_execution_links (
    id, conversation_id, execution_id, relation, step_id, attempt_id,
    active, created_at, updated_at
)
SELECT 'execlink_attempt_' || step_id || '_' || attempt_no,
       conversation_id,
       execution_id,
       'attempt',
       step_id,
       'execattempt_migrated_' || step_id || '_' || attempt_no,
       CASE WHEN is_current = 1 AND legacy_status = 'needs_review' THEN 1 ELSE 0 END,
       created_at,
       created_at
FROM _m037_attempt_conversation_stage;

-- A lead link is immutable audit identity, while `active = 1` means the one
-- current Execution projected by the Conversation.  Legacy orchestrator data
-- could retain several runs for the same lead Conversation.  Preserve every
-- row, but choose one current lead deterministically before adding the hard
-- uniqueness constraint: a live aggregate wins, followed by the newest
-- aggregate snapshot and stable identifiers.
WITH ranked_leads AS (
    SELECT link.id,
           ROW_NUMBER() OVER (
               PARTITION BY link.conversation_id
               ORDER BY
                   CASE
                       WHEN execution.deleted_at IS NULL
                        AND execution.status NOT IN (
                            'completed', 'completed_with_failures', 'failed', 'cancelled'
                        ) THEN 1 ELSE 0
                   END DESC,
                   execution.updated_at DESC,
                   execution.created_at DESC,
                   execution.id DESC,
                   link.id DESC
           ) AS current_rank
    FROM conversation_execution_links link
    JOIN agent_executions execution ON execution.id = link.execution_id
    WHERE link.relation = 'lead'
)
UPDATE conversation_execution_links
SET active = 0
WHERE id IN (
    SELECT id FROM ranked_leads WHERE current_rank > 1
);

CREATE UNIQUE INDEX idx_conversation_execution_current_lead
    ON conversation_execution_links(conversation_id)
    WHERE relation = 'lead' AND active = 1;

-- Attempt message transcripts are immutable execution audit history.  Define
-- this only after legacy attempt links are materialized so the guard has one
-- authoritative relation source from its first effective write onward.
-- Account deletion remains the sole physical cleanup boundary because the
-- execution owner no longer exists while the FK cascade runs.
CREATE TRIGGER agent_execution_attempt_message_retained
BEFORE DELETE ON messages
WHEN EXISTS (
    SELECT 1
    FROM conversation_execution_links link
    JOIN agent_executions execution ON execution.id = link.execution_id
    JOIN users owner ON owner.id = execution.user_id
    WHERE link.conversation_id = OLD.conversation_id
      AND link.relation = 'attempt'
)
BEGIN
    SELECT RAISE(ABORT, 'execution attempt messages are retained for audit');
END;

-- Delegation appends Steps to the current aggregate; an active Attempt
-- Conversation can never become the lead of a second Execution. Attempt links
-- are immutable audit identity, so settling/deactivating the runtime relation
-- does not release the Conversation for a different aggregate.
CREATE TRIGGER conversation_execution_attempt_cannot_lead_another_execution
BEFORE INSERT ON conversation_execution_links
WHEN NEW.relation = 'lead'
 AND EXISTS (
     SELECT 1
     FROM conversation_execution_links attempt_link
     JOIN agent_executions current_execution
       ON current_execution.id = attempt_link.execution_id
     JOIN agent_executions incoming_execution
       ON incoming_execution.id = NEW.execution_id
     WHERE attempt_link.conversation_id = NEW.conversation_id
       AND attempt_link.relation = 'attempt'
       AND current_execution.user_id = incoming_execution.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'Attempt Conversation permanently belongs to its Agent Execution');
END;

-- A Conversation may retain immutable lead history, while only one row is the
-- current lead.  Give repository callers a domain error when they try to add
-- another unfinished current owner; the partial unique index above is the
-- unconditional database invariant for all current-lead writes.
CREATE TRIGGER conversation_execution_single_unfinished_lead_insert
BEFORE INSERT ON conversation_execution_links
WHEN NEW.relation = 'lead' AND NEW.active = 1
 AND EXISTS (
     SELECT 1 FROM agent_executions incoming
     WHERE incoming.id = NEW.execution_id
       AND incoming.status NOT IN ('completed', 'completed_with_failures', 'failed', 'cancelled')
 )
 AND EXISTS (
     SELECT 1
     FROM conversation_execution_links existing_link
     JOIN agent_executions existing_execution
       ON existing_execution.id = existing_link.execution_id
     WHERE existing_link.conversation_id = NEW.conversation_id
       AND existing_link.relation = 'lead' AND existing_link.active = 1
       AND existing_link.execution_id <> NEW.execution_id
       AND existing_execution.deleted_at IS NULL
       AND existing_execution.status NOT IN (
           'completed', 'completed_with_failures', 'failed', 'cancelled'
       )
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation already has an unfinished Agent Execution');
END;

INSERT INTO agent_execution_events (
    id, execution_id, sequence, event_type, step_id, attempt_id,
    actor_type, actor_id, actor_conversation_id, actor_attempt_id,
    on_behalf_of_user_id, payload, created_at
)
SELECT 'aevt_migrated_' || execution.id, execution.id, 1, 'migrated', NULL, NULL,
       'system', NULL, NULL, NULL, execution.user_id,
       json_object(
           'migration', 37,
           'legacy_forked_from', legacy.forked_from,
           'legacy_missing_lead_conversation_id', CASE
               WHEN legacy.lead_conv_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM conversations conversation
                    WHERE conversation.id = legacy.lead_conv_id
                )
               THEN legacy.lead_conv_id
           END,
           'legacy_control_assignments', json(COALESCE((
               SELECT json_group_array(json(control.assignment_json))
               FROM (
                   SELECT json_object(
                       'id', assignment.id,
                       'task_id', assignment.task_id,
                       'member_id', assignment.member_id,
                       'score', assignment.score,
                       'rationale', assignment.rationale,
                       'source', assignment.source,
                       'locked', json(CASE assignment.locked WHEN 1 THEN 'true' ELSE 'false' END),
                       'created_at', assignment.created_at
                   ) AS assignment_json
                   FROM orch_assignments assignment
                   JOIN orch_run_tasks task ON task.id = assignment.task_id
                   WHERE task.run_id = execution.id
                     AND task.kind IN ('verify', 'judge', 'loop')
                   ORDER BY task.id, assignment.id
               ) control
           ), '[]')),
           -- The detached legacy writer could leave task.conversation_id one
           -- generation behind the complete Conversation evidence. Preserve
           -- both identities whenever the deterministic selection supersedes
           -- that stale hint, rather than silently rewriting history.
           'legacy_conversation_selection_conflicts', json(COALESCE((
               SELECT json_group_array(json(conflict.conflict_json))
               FROM (
                   SELECT json_object(
                       'step_id', task.id,
                       'configured_conversation_id', task.conversation_id,
                       'selected_conversation_id', selected.conversation_id,
                       'selection_source', selected.current_source
                   ) AS conflict_json
                   FROM orch_run_tasks task
                   JOIN _m037_attempt_conversation_stage selected
                     ON selected.execution_id = task.run_id
                    AND selected.step_id = task.id
                    AND selected.is_current = 1
                   WHERE task.run_id = execution.id
                     AND task.conversation_id IS NOT NULL
                     AND task.conversation_id <> selected.conversation_id
                   ORDER BY task.id
               ) conflict
           ), '[]'))
       ),
       execution.updated_at
FROM agent_executions execution
JOIN orch_runs legacy ON legacy.id = execution.id;

-- Provider references are value-object snapshots rather than FKs because
-- Conversation and Execution history must survive account/provider lifecycle
-- changes. New executable writes nevertheless require a provider that exists;
-- these guards are intentionally installed after the one-time legacy copy so
-- irreversible Cancelled history can remain readable without becoming live.
CREATE TRIGGER conversation_provider_binding_insert_guard
BEFORE INSERT ON conversations
WHEN (
    NEW.model IS NOT NULL
    AND NOT EXISTS (
        SELECT 1 FROM providers provider
        WHERE provider.id = COALESCE(
            json_extract(NEW.model, '$.provider_id'),
            json_extract(NEW.model, '$.providerId'),
            json_extract(NEW.model, '$.id')
        )
    )
) OR (
    NEW.execution_model_pool IS NOT NULL
    AND json_extract(NEW.execution_model_pool, '$.mode') = 'single'
    AND NOT EXISTS (
        SELECT 1 FROM providers provider
        WHERE provider.id = json_extract(NEW.execution_model_pool, '$.model.provider_id')
    )
) OR (
    NEW.execution_model_pool IS NOT NULL
    AND json_extract(NEW.execution_model_pool, '$.mode') = 'range'
    AND EXISTS (
        SELECT 1 FROM json_each(NEW.execution_model_pool, '$.models') model_ref
        WHERE NOT EXISTS (
            SELECT 1 FROM providers provider
            WHERE provider.id = json_extract(model_ref.value, '$.provider_id')
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'Conversation model authority references a missing provider');
END;

CREATE TRIGGER conversation_provider_binding_update_guard
BEFORE UPDATE OF model, execution_model_pool ON conversations
WHEN (
    NEW.model IS NOT NULL
    AND NOT EXISTS (
        SELECT 1 FROM providers provider
        WHERE provider.id = COALESCE(
            json_extract(NEW.model, '$.provider_id'),
            json_extract(NEW.model, '$.providerId'),
            json_extract(NEW.model, '$.id')
        )
    )
) OR (
    NEW.execution_model_pool IS NOT NULL
    AND json_extract(NEW.execution_model_pool, '$.mode') = 'single'
    AND NOT EXISTS (
        SELECT 1 FROM providers provider
        WHERE provider.id = json_extract(NEW.execution_model_pool, '$.model.provider_id')
    )
) OR (
    NEW.execution_model_pool IS NOT NULL
    AND json_extract(NEW.execution_model_pool, '$.mode') = 'range'
    AND EXISTS (
        SELECT 1 FROM json_each(NEW.execution_model_pool, '$.models') model_ref
        WHERE NOT EXISTS (
            SELECT 1 FROM providers provider
            WHERE provider.id = json_extract(model_ref.value, '$.provider_id')
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'Conversation model authority references a missing provider');
END;

CREATE TRIGGER agent_execution_template_provider_insert_guard
BEFORE INSERT ON agent_execution_template_participants
WHEN NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.provider_id)
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template references a missing provider');
END;

CREATE TRIGGER agent_execution_template_provider_update_guard
BEFORE UPDATE OF provider_id, model ON agent_execution_template_participants
WHEN NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.provider_id)
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template references a missing provider');
END;

CREATE TRIGGER agent_execution_participant_provider_insert_guard
BEFORE INSERT ON agent_execution_participants
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
      AND execution.deleted_at IS NULL
      AND execution.status <> 'cancelled'
)
 AND NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.provider_id)
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution participant references a missing provider');
END;

CREATE TRIGGER idmm_backup_provider_insert_guard
BEFORE INSERT ON client_preferences
WHEN NEW.key = 'idmm_backup_provider_id'
 AND NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.value)
BEGIN
    SELECT RAISE(ABORT, 'IDMM backup references a missing provider');
END;

CREATE TRIGGER idmm_backup_provider_update_guard
BEFORE UPDATE OF key, value ON client_preferences
WHEN NEW.key = 'idmm_backup_provider_id'
 AND NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.value)
BEGIN
    SELECT RAISE(ABORT, 'IDMM backup references a missing provider');
END;

-- The application-level usage scan supplies friendly, labeled conflicts. This
-- trigger is the atomic authority that closes the scan/delete race and protects
-- every hard binding in the same SQLite statement that removes the provider.
CREATE TRIGGER provider_hard_binding_delete_guard
BEFORE DELETE ON providers
WHEN EXISTS (
    SELECT 1 FROM conversations conversation
    WHERE COALESCE(
        json_extract(conversation.model, '$.provider_id'),
        json_extract(conversation.model, '$.providerId'),
        json_extract(conversation.model, '$.id')
    ) = OLD.id
) OR EXISTS (
    SELECT 1 FROM agent_execution_template_participants participant
    WHERE participant.provider_id = OLD.id
) OR EXISTS (
    SELECT 1
    FROM agent_execution_participants participant
    JOIN agent_executions execution ON execution.id = participant.execution_id
    WHERE participant.provider_id = OLD.id
      AND participant.retired_in_revision IS NULL
      AND execution.deleted_at IS NULL
      AND execution.status <> 'cancelled'
) OR EXISTS (
    SELECT 1 FROM client_preferences preference
    WHERE preference.key = 'idmm_backup_provider_id'
      AND preference.value = OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'provider is still referenced by an executable Agent binding');
END;

-- Soft collaboration/failover candidates do not block deletion, but they must
-- disappear atomically with the provider rather than relying on a post-delete
-- best-effort cleanup that can leave durable dangling references.
CREATE TRIGGER provider_soft_reference_cleanup
AFTER DELETE ON providers
BEGIN
    UPDATE conversations
    SET execution_model_pool = CASE
        WHEN json_extract(conversations.execution_model_pool, '$.mode') = 'single'
            THEN NULL
        ELSE COALESCE(
            (
                SELECT CASE WHEN COUNT(*) = 0 THEN NULL ELSE
                    json_object(
                        'mode', 'range',
                        'models', json(json_group_array(json(item.value)))
                    )
                END
                FROM json_each(conversations.execution_model_pool, '$.models') item
                WHERE json_extract(item.value, '$.provider_id') <> OLD.id
                  AND EXISTS (
                      SELECT 1 FROM providers provider
                      WHERE provider.id = json_extract(item.value, '$.provider_id')
                  )
            ),
            NULL
        )
    END
    WHERE conversations.execution_model_pool IS NOT NULL
      AND (
          (json_extract(conversations.execution_model_pool, '$.mode') = 'single'
           AND json_extract(conversations.execution_model_pool, '$.model.provider_id') = OLD.id)
          OR
          (json_extract(conversations.execution_model_pool, '$.mode') = 'range'
           AND EXISTS (
               SELECT 1
               FROM json_each(conversations.execution_model_pool, '$.models') target
               WHERE json_extract(target.value, '$.provider_id') = OLD.id
           ))
      );

    -- The collaboration picker persists a canonical, ordered array of concrete
    -- provider/model value objects. Keep the preference itself when every
    -- candidate disappears: [] means "lead model only" at the product boundary.
    UPDATE client_preferences
    SET value = json(COALESCE(
            (
                SELECT json_group_array(json(retained.value))
                FROM (
                    SELECT candidate.value
                    FROM json_each(client_preferences.value) candidate
                    WHERE candidate.type = 'object'
                      AND (SELECT COUNT(*) FROM json_each(candidate.value)) = 2
                      AND json_type(candidate.value, '$.provider_id') = 'text'
                      AND trim(json_extract(candidate.value, '$.provider_id')) <> ''
                      AND trim(json_extract(candidate.value, '$.provider_id'))
                            = json_extract(candidate.value, '$.provider_id')
                      AND json_type(candidate.value, '$.model') = 'text'
                      AND trim(json_extract(candidate.value, '$.model')) <> ''
                      AND trim(json_extract(candidate.value, '$.model'))
                            = json_extract(candidate.value, '$.model')
                      AND EXISTS (
                          SELECT 1 FROM providers provider
                          WHERE provider.id = json_extract(candidate.value, '$.provider_id')
                      )
                    ORDER BY CAST(candidate.key AS INTEGER)
                ) retained
            ),
            '[]'
        )),
        updated_at = MAX(
            client_preferences.updated_at,
            CAST(strftime('%s', 'now') AS INTEGER) * 1000
        )
    WHERE client_preferences.key = 'nomi.collaborationModels'
      AND json_valid(client_preferences.value)
      AND json_type(client_preferences.value) = 'array'
      AND EXISTS (
          SELECT 1 FROM json_each(client_preferences.value) candidate
          WHERE candidate.type <> 'object'
             OR (SELECT COUNT(*) FROM json_each(candidate.value)) <> 2
             OR json_type(candidate.value, '$.provider_id') IS NOT 'text'
             OR trim(COALESCE(json_extract(candidate.value, '$.provider_id'), '')) = ''
             OR trim(json_extract(candidate.value, '$.provider_id'))
                   <> json_extract(candidate.value, '$.provider_id')
             OR json_type(candidate.value, '$.model') IS NOT 'text'
             OR trim(COALESCE(json_extract(candidate.value, '$.model'), '')) = ''
             OR trim(json_extract(candidate.value, '$.model'))
                   <> json_extract(candidate.value, '$.model')
             OR NOT EXISTS (
                 SELECT 1 FROM providers provider
                 WHERE provider.id = json_extract(candidate.value, '$.provider_id')
             )
      );

    UPDATE client_preferences
    SET value = json_set(
            client_preferences.value,
            '$.queue',
            json(COALESCE((
                SELECT json_group_array(json(candidate.value))
                FROM json_each(client_preferences.value, '$.queue') candidate
                WHERE json_extract(candidate.value, '$.provider_id') <> OLD.id
                  AND EXISTS (
                      SELECT 1 FROM providers provider
                      WHERE provider.id = json_extract(candidate.value, '$.provider_id')
                  )
            ), '[]'))
        ),
        updated_at = MAX(
            client_preferences.updated_at,
            CAST(strftime('%s', 'now') AS INTEGER) * 1000
        )
    WHERE client_preferences.key = 'agent.model_failover'
      AND json_valid(client_preferences.value)
      AND json_type(client_preferences.value, '$.queue') = 'array'
      AND EXISTS (
          SELECT 1 FROM json_each(client_preferences.value, '$.queue') candidate
          WHERE json_extract(candidate.value, '$.provider_id') = OLD.id
      );
END;

-- Aggregate-level reconciliation before deleting any source table.
INSERT INTO _m037_guard
SELECT 0 WHERE
    (SELECT COUNT(*) FROM agent_executions) <> (SELECT COUNT(*) FROM orch_runs)
    OR (SELECT COUNT(*) FROM agent_execution_steps) <> (SELECT COUNT(*) FROM orch_run_tasks)
    OR (SELECT COUNT(*) FROM agent_execution_step_dependencies)
       <> (SELECT COUNT(*) FROM orch_run_task_deps)
    OR (SELECT COUNT(*) FROM agent_execution_attempts) <> (
        (SELECT COUNT(*) FROM orch_run_tasks)
        + (SELECT COUNT(*) FROM _m037_attempt_conversation_stage WHERE is_current = 0)
    )
    OR (SELECT COUNT(*) FROM agent_execution_events) <> (SELECT COUNT(*) FROM orch_runs)
    OR (SELECT COUNT(*) FROM agent_execution_participants) <> (
        SELECT COALESCE(SUM(json_array_length(fleet_snapshot)), 0) FROM orch_runs
    ) + (SELECT COUNT(*) FROM orch_run_tasks WHERE override_provider_id IS NOT NULL);
-- Bare model members used an empty `agent_id` as an intentional sentinel in
-- the released ad-hoc execution path. Verify that both direct and override
-- snapshots crossed that boundary as the canonical Nomi Agent identity.
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_runs legacy_execution, json_each(legacy_execution.fleet_snapshot) legacy_member
    LEFT JOIN agent_execution_participants participant
      ON participant.execution_id = legacy_execution.id
     AND participant.id = json_extract(legacy_member.value, '$.id')
    WHERE participant.source_agent_id IS NOT COALESCE(
        NULLIF(trim(json_extract(legacy_member.value, '$.agent_id')), ''),
        'nomi'
    )
);
INSERT INTO _m037_guard
WITH override_base AS (
    SELECT task.id AS step_id, task.run_id, execution.fleet_snapshot,
           COALESCE(
               (SELECT assignment.member_id FROM orch_assignments assignment
                WHERE assignment.task_id = task.id),
               (SELECT json_extract(member.value, '$.id')
                FROM json_each(execution.fleet_snapshot) member
                ORDER BY CAST(member.key AS INTEGER) LIMIT 1)
           ) AS base_member_id
    FROM orch_run_tasks task
    JOIN orch_runs execution ON execution.id = task.run_id
    WHERE task.override_provider_id IS NOT NULL
)
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM override_base base
    JOIN json_each(base.fleet_snapshot) legacy_member
      ON json_extract(legacy_member.value, '$.id') = base.base_member_id
    LEFT JOIN agent_execution_participants participant
      ON participant.execution_id = base.run_id
     AND participant.id = 'execpart_override_' || base.step_id
    WHERE participant.source_agent_id IS NOT COALESCE(
        NULLIF(trim(json_extract(legacy_member.value, '$.agent_id')), ''),
        'nomi'
    )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_tasks legacy
    LEFT JOIN agent_execution_steps step
      ON step.execution_id = legacy.run_id AND step.id = legacy.id
    LEFT JOIN agent_executions execution ON execution.id = legacy.run_id
    LEFT JOIN agent_execution_attempts attempt
      ON attempt.execution_id = legacy.run_id
     AND attempt.step_id = legacy.id
     AND attempt.id = 'execattempt_migrated_' || legacy.id || '_' || legacy.attempt
    LEFT JOIN _m037_attempt_conversation_stage current_conversation
      ON current_conversation.execution_id = legacy.run_id
     AND current_conversation.step_id = legacy.id
     AND current_conversation.is_current = 1
    WHERE step.id IS NULL
       OR execution.id IS NULL
       OR attempt.id IS NULL
       OR step.dispatch_after IS NOT CASE
              WHEN legacy.status = 'pending' THEN legacy.next_retry_at
              ELSE NULL
          END
       OR attempt.attempt_no IS NOT legacy.attempt
       OR attempt.participant_id IS NOT step.assigned_participant_id
       OR attempt.status IS NOT CASE legacy.status
              WHEN 'pending' THEN CASE
                  WHEN legacy.kind IN ('verify', 'judge', 'loop') THEN 'cancelled'
                  ELSE 'queued'
              END
              WHEN 'running' THEN 'interrupted'
              WHEN 'needs_review' THEN 'waiting_input'
              WHEN 'done' THEN 'completed'
              WHEN 'skipped' THEN 'cancelled'
              ELSE legacy.status
          END
       OR attempt.trigger_reason IS NOT CASE
              WHEN legacy.status = 'pending'
               AND legacy.kind IN ('verify', 'judge', 'loop')
                  THEN 'migrated_unstarted_control_reservation'
              ELSE 'migrated_current_attempt'
          END
       OR json_extract(attempt.effective_config, '$.participant_id')
              IS NOT step.assigned_participant_id
       OR json_extract(attempt.effective_config, '$.decision_policy')
              IS NOT execution.decision_policy
       OR json_extract(attempt.effective_config, '$.adaptation_policy')
              IS NOT execution.adaptation_policy
       OR json_extract(
              attempt.effective_config,
              '$.legacy_configured_conversation_id'
          ) IS NOT legacy.conversation_id
       OR json_extract(attempt.effective_config, '$.legacy_conversation_id')
              IS NOT current_conversation.conversation_id
       OR json_extract(attempt.effective_config, '$.legacy_conversation_source')
              IS NOT COALESCE(current_conversation.current_source, 'none')
       OR attempt.question IS NOT legacy.pending_question
       OR attempt.error IS NOT legacy.last_error
       OR attempt.output_summary IS NOT legacy.output_summary
       OR attempt.output_files IS NOT COALESCE(legacy.output_files, '[]')
       OR attempt.tokens IS NOT legacy.tokens
       OR attempt.retry_after IS NOT legacy.next_retry_at
       OR attempt.runtime_state IS NOT CASE
              WHEN json_extract(legacy.pattern_config, '$.loop_prior_output') IS NOT NULL
                OR json_extract(legacy.pattern_config, '$.loop_iteration') IS NOT NULL
              THEN json_object(
                  'loop_prior_output',
                  json_extract(legacy.pattern_config, '$.loop_prior_output'),
                  'loop_iteration',
                  json_extract(legacy.pattern_config, '$.loop_iteration')
              )
          END
       OR attempt.started_at IS NOT CASE WHEN legacy.status <> 'pending'
              THEN COALESCE(current_conversation.created_at, legacy.created_at)
          END
       OR attempt.finished_at IS NOT CASE
              WHEN legacy.status IN ('running', 'done', 'failed', 'skipped', 'cancelled')
                OR (legacy.status = 'pending'
                    AND legacy.kind IN ('verify', 'judge', 'loop'))
              THEN COALESCE(current_conversation.updated_at, legacy.updated_at)
          END
       OR attempt.created_at IS NOT COALESCE(
              current_conversation.created_at,
              legacy.created_at
          )
       OR attempt.updated_at IS NOT COALESCE(
              current_conversation.updated_at,
              legacy.updated_at
          )
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM _m037_attempt_conversation_stage history
    LEFT JOIN agent_execution_steps step
      ON step.execution_id = history.execution_id AND step.id = history.step_id
    LEFT JOIN agent_execution_attempts attempt
      ON attempt.execution_id = history.execution_id
     AND attempt.step_id = history.step_id
     AND attempt.id = 'execattempt_migrated_' || history.step_id || '_' || history.attempt_no
    LEFT JOIN conversation_execution_links link
      ON link.id = 'execlink_attempt_' || history.step_id || '_' || history.attempt_no
    WHERE step.id IS NULL
       OR attempt.id IS NULL
       OR attempt.attempt_no IS NOT history.attempt_no
       OR attempt.participant_id IS NOT step.assigned_participant_id
       OR link.id IS NULL
       OR link.conversation_id IS NOT history.conversation_id
       OR link.execution_id IS NOT history.execution_id
       OR link.step_id IS NOT history.step_id
       OR link.attempt_id IS NOT attempt.id
       OR link.active IS NOT CASE
              WHEN history.is_current = 1 AND history.legacy_status = 'needs_review' THEN 1
              ELSE 0
          END
       OR (history.is_current = 0 AND (
              attempt.status IS NOT 'interrupted'
              OR attempt.trigger_reason IS NOT 'migrated_superseded_retry_transcript'
              OR json_extract(
                     attempt.effective_config,
                     '$.legacy_conversation_id'
                 ) IS NOT history.conversation_id
              OR json_extract(
                     attempt.effective_config,
                     '$.legacy_attempt_no_inferred'
                 ) IS NOT 1
              OR json_extract(
                     attempt.effective_config,
                     '$.legacy_candidate_ordinal'
                 ) IS NOT history.candidate_ordinal
              OR json_extract(
                     attempt.effective_config,
                     '$.legacy_candidate_count'
                 ) IS NOT history.candidate_count
              OR json_extract(
                     attempt.effective_config,
                     '$.legacy_numbering'
                 ) IS NOT 'right_aligned'
              OR attempt.question IS NOT NULL
              OR attempt.error IS NOT NULL
              OR attempt.output_summary IS NOT NULL
              OR attempt.output_files IS NOT '[]'
              OR attempt.tokens IS NOT NULL
              OR attempt.retry_after IS NOT NULL
              OR attempt.runtime_state IS NOT NULL
              OR attempt.started_at IS NOT history.created_at
              OR attempt.finished_at IS NOT history.updated_at
              OR attempt.created_at IS NOT history.created_at
              OR attempt.updated_at IS NOT history.updated_at
          ))
);
INSERT INTO _m037_guard
WITH expected_attempt_links AS (
    SELECT 'execlink_attempt_' || stage.step_id || '_' || stage.attempt_no AS id,
           stage.conversation_id,
           stage.execution_id,
           'attempt' AS relation,
           stage.step_id,
           'execattempt_migrated_' || stage.step_id || '_' || stage.attempt_no AS attempt_id,
           CASE WHEN stage.is_current = 1 AND stage.legacy_status = 'needs_review'
                THEN 1 ELSE 0 END AS active,
           CAST(NULL AS INTEGER) AS cleanup_completed_at,
           stage.created_at,
           stage.created_at AS updated_at
    FROM _m037_attempt_conversation_stage stage
), actual_attempt_links AS (
    SELECT link.id, link.conversation_id, link.execution_id, link.relation,
           link.step_id, link.attempt_id, link.active, link.cleanup_completed_at,
           link.created_at, link.updated_at
    FROM conversation_execution_links link
    WHERE link.relation = 'attempt'
)
SELECT 0 WHERE EXISTS (
    SELECT * FROM expected_attempt_links
    EXCEPT
    SELECT * FROM actual_attempt_links
) OR EXISTS (
    SELECT * FROM actual_attempt_links
    EXCEPT
    SELECT * FROM expected_attempt_links
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_run_tasks legacy
    JOIN _m037_legacy_execution_depths run_depth ON run_depth.run_id = legacy.run_id
    JOIN agent_execution_steps step
      ON step.execution_id = legacy.run_id AND step.id = legacy.id
    WHERE step.delegation_depth <> max(
        COALESCE(json_extract(legacy.pattern_config, '$.delegation_depth'), 0),
        run_depth.effective_depth
    )
);
-- Configuration-level reconciliation is separate from the seven-table
-- runtime reconciliation above. Only executable legacy authoring aggregates
-- become Templates; invalid/empty sources were deterministically filtered in
-- the canonical staging set.
INSERT INTO _m037_guard
SELECT 0 WHERE
    (SELECT COUNT(*) FROM agent_execution_templates)
        <> (SELECT COUNT(*) FROM fleets fleet
            WHERE EXISTS (
                SELECT 1 FROM _m037_template_participant_stage stage
                WHERE stage.fleet_id = fleet.id
            ))
           + (SELECT COUNT(*) FROM orch_workspaces workspace
              WHERE EXISTS (
                  SELECT 1 FROM _m037_template_participant_stage stage
                  WHERE stage.fleet_id = workspace.default_fleet_id
              ))
    OR (SELECT COUNT(*) FROM agent_execution_template_participants)
        <> (SELECT COUNT(*) FROM _m037_template_participant_stage)
           + (SELECT COUNT(*)
              FROM orch_workspaces workspace
              JOIN _m037_template_participant_stage stage
                ON stage.fleet_id = workspace.default_fleet_id);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM fleets fleet
    LEFT JOIN agent_execution_templates template ON template.id = fleet.id
    WHERE EXISTS (
              SELECT 1 FROM _m037_template_participant_stage stage
              WHERE stage.fleet_id = fleet.id
          )
      AND (template.id IS NULL
       OR template.user_id IS NOT fleet.user_id
       OR template.name IS NOT fleet.name
       OR template.description IS NOT fleet.description
       OR template.max_parallel IS NOT CASE
              WHEN fleet.max_parallel IS NULL THEN NULL
              ELSE min(max(fleet.max_parallel, 1), 64) END
       OR template.work_dir IS NOT NULL
       OR template.context IS NOT NULL
       OR template.primary_participant_id IS NOT (
              SELECT stage.id FROM _m037_template_participant_stage stage
              WHERE stage.fleet_id = fleet.id
              ORDER BY stage.sort_order, stage.id LIMIT 1
          )
       OR template.created_at IS NOT fleet.created_at
       OR template.updated_at IS NOT fleet.updated_at)
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_workspaces workspace
    LEFT JOIN fleets fleet ON fleet.id = workspace.default_fleet_id
    LEFT JOIN agent_execution_templates template ON template.id = workspace.id
    WHERE EXISTS (
              SELECT 1 FROM _m037_template_participant_stage stage
              WHERE stage.fleet_id = workspace.default_fleet_id
          )
      AND (template.id IS NULL
       OR template.user_id IS NOT workspace.user_id
       OR template.name IS NOT workspace.name
       OR template.description IS NOT fleet.description
       OR template.max_parallel IS NOT CASE
              WHEN fleet.max_parallel IS NULL THEN NULL
              ELSE min(max(fleet.max_parallel, 1), 64) END
       OR template.work_dir IS NOT workspace.workspace_dir
       OR template.context IS NOT workspace.context
       OR template.primary_participant_id IS NOT (
              SELECT stage.id FROM _m037_template_participant_stage stage
              WHERE stage.fleet_id = workspace.default_fleet_id
              ORDER BY stage.sort_order, stage.id LIMIT 1
          )
       OR template.created_at IS NOT workspace.created_at
       OR template.updated_at IS NOT workspace.updated_at)
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM _m037_template_participant_stage stage
    LEFT JOIN agent_execution_template_participants participant
      ON participant.template_id = stage.fleet_id AND participant.id = stage.id
    WHERE participant.id IS NULL
       OR participant.source_agent_id IS NOT stage.source_agent_id
       OR participant.preset_id IS NOT stage.preset_id
       OR participant.preset_revision IS NOT stage.preset_revision
       OR participant.preset_snapshot IS NOT stage.preset_snapshot
       OR participant.provider_id IS NOT stage.provider_id
       OR participant.model IS NOT stage.model
       OR participant.role IS NOT stage.role
       OR participant.capability IS NOT stage.capability
       OR participant.constraints IS NOT stage.constraints
       OR participant.description IS NOT stage.description
       OR participant.system_prompt IS NOT stage.system_prompt
       OR participant.enabled_skills IS NOT stage.enabled_skills
       OR participant.disabled_builtin_skills IS NOT stage.disabled_builtin_skills
       OR participant.sort_order IS NOT stage.sort_order
       OR participant.created_at IS NOT stage.created_at
       OR participant.updated_at IS NOT stage.updated_at
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM orch_workspaces workspace
    JOIN _m037_template_participant_stage stage
      ON stage.fleet_id = workspace.default_fleet_id
    LEFT JOIN agent_execution_template_participants participant
      ON participant.template_id = workspace.id AND participant.id = stage.id
    WHERE participant.id IS NULL
       OR participant.source_agent_id IS NOT stage.source_agent_id
       OR participant.preset_id IS NOT stage.preset_id
       OR participant.preset_revision IS NOT stage.preset_revision
       OR participant.preset_snapshot IS NOT stage.preset_snapshot
       OR participant.provider_id IS NOT stage.provider_id
       OR participant.model IS NOT stage.model
       OR participant.role IS NOT stage.role
       OR participant.capability IS NOT stage.capability
       OR participant.constraints IS NOT stage.constraints
       OR participant.description IS NOT stage.description
       OR participant.system_prompt IS NOT stage.system_prompt
       OR participant.enabled_skills IS NOT stage.enabled_skills
       OR participant.disabled_builtin_skills IS NOT stage.disabled_builtin_skills
       OR participant.sort_order IS NOT stage.sort_order
       OR participant.created_at IS NOT stage.created_at
       OR participant.updated_at IS NOT stage.updated_at
);
INSERT INTO _m037_guard
WITH lead_candidates AS (
    SELECT run.id AS execution_id,
           run.lead_conv_id AS conversation_id,
           run.created_at
    FROM orch_runs run
    JOIN conversations conversation ON conversation.id = run.lead_conv_id
    UNION
    SELECT json_extract(conversation.extra, '$.orchestrator_run_id'),
           conversation.id,
           conversation.created_at
    FROM conversations conversation
    WHERE json_extract(conversation.extra, '$.orchestrator_run_id') IS NOT NULL
      AND json_extract(conversation.extra, '$.orchestrator_task_id') IS NULL
), expected_lead_base AS (
    SELECT 'execlink_lead_' || execution_id AS id,
           conversation_id,
           execution_id,
           MIN(created_at) AS created_at
    FROM lead_candidates
    GROUP BY execution_id, conversation_id
), ranked_expected_leads AS (
    SELECT expected.*,
           ROW_NUMBER() OVER (
               PARTITION BY expected.conversation_id
               ORDER BY
                   CASE
                       WHEN execution.deleted_at IS NULL
                        AND execution.status NOT IN (
                            'completed', 'completed_with_failures', 'failed', 'cancelled'
                        ) THEN 1 ELSE 0
                   END DESC,
                   execution.updated_at DESC,
                   execution.created_at DESC,
                   execution.id DESC,
                   expected.id DESC
           ) AS current_rank
    FROM expected_lead_base expected
    JOIN agent_executions execution ON execution.id = expected.execution_id
), expected_lead_links AS (
    SELECT expected.id,
           expected.conversation_id,
           expected.execution_id,
           'lead' AS relation,
           CAST(NULL AS TEXT) AS step_id,
           CAST(NULL AS TEXT) AS attempt_id,
           CASE WHEN expected.current_rank = 1 THEN 1 ELSE 0 END AS active,
           CAST(NULL AS INTEGER) AS cleanup_completed_at,
           expected.created_at,
           expected.created_at AS updated_at
    FROM ranked_expected_leads expected
), actual_lead_links AS (
    SELECT link.id, link.conversation_id, link.execution_id, link.relation,
           link.step_id, link.attempt_id, link.active, link.cleanup_completed_at,
           link.created_at, link.updated_at
    FROM conversation_execution_links link
    WHERE link.relation = 'lead'
)
SELECT 0 WHERE EXISTS (
    SELECT * FROM expected_lead_links
    EXCEPT
    SELECT * FROM actual_lead_links
) OR EXISTS (
    SELECT * FROM actual_lead_links
    EXCEPT
    SELECT * FROM expected_lead_links
);
INSERT INTO _m037_guard
WITH expected_links AS (
    SELECT COUNT(*) AS count FROM (
        SELECT r.id
        FROM orch_runs r
        JOIN conversations conversation ON conversation.id = r.lead_conv_id
        UNION
        SELECT json_extract(c.extra, '$.orchestrator_run_id')
        FROM conversations c
        WHERE json_extract(c.extra, '$.orchestrator_run_id') IS NOT NULL
          AND json_extract(c.extra, '$.orchestrator_task_id') IS NULL
    )
    UNION ALL
    SELECT COUNT(*) FROM _m037_attempt_conversation_stage
)
SELECT 0
WHERE (SELECT COUNT(*) FROM conversation_execution_links)
      <> (SELECT SUM(count) FROM expected_links);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM agent_executions e
    WHERE e.event_sequence <> COALESCE((
        SELECT MAX(event.sequence) FROM agent_execution_events event
        WHERE event.execution_id = e.id
    ), 0)
);
INSERT INTO _m037_guard
SELECT 0 WHERE EXISTS (
    SELECT 1
    FROM agent_execution_events migrated_event
    WHERE migrated_event.event_type = 'migrated'
      AND (
          json_extract(
              migrated_event.payload,
              '$.legacy_missing_lead_conversation_id'
          ) IS NOT (
              SELECT CASE
                  WHEN legacy.lead_conv_id IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM conversations conversation
                       WHERE conversation.id = legacy.lead_conv_id
                   )
                  THEN legacy.lead_conv_id
              END
              FROM orch_runs legacy
              WHERE legacy.id = migrated_event.execution_id
          )
          OR json_type(migrated_event.payload, '$.legacy_control_assignments') IS NOT 'array'
          OR json_array_length(
                 migrated_event.payload,
                 '$.legacy_control_assignments'
             ) <> (
                 SELECT COUNT(*)
                 FROM orch_assignments assignment
                 JOIN orch_run_tasks task ON task.id = assignment.task_id
                 WHERE task.run_id = migrated_event.execution_id
                   AND task.kind IN ('verify', 'judge', 'loop')
             )
          OR EXISTS (
              SELECT 1
              FROM orch_assignments assignment
              JOIN orch_run_tasks task ON task.id = assignment.task_id
              WHERE task.run_id = migrated_event.execution_id
                AND task.kind IN ('verify', 'judge', 'loop')
                AND NOT EXISTS (
                    SELECT 1
                    FROM json_each(
                        migrated_event.payload,
                        '$.legacy_control_assignments'
                    ) archived
                    WHERE json_extract(archived.value, '$.id') IS assignment.id
                      AND json_extract(archived.value, '$.task_id') IS assignment.task_id
                      AND json_extract(archived.value, '$.member_id') IS assignment.member_id
                      AND json_extract(archived.value, '$.score') IS assignment.score
                      AND json_extract(archived.value, '$.rationale') IS assignment.rationale
                      AND json_extract(archived.value, '$.source') IS assignment.source
                      AND json_extract(archived.value, '$.locked') IS assignment.locked
                      AND json_extract(archived.value, '$.created_at') IS assignment.created_at
                )
          )
          OR json_type(
                 migrated_event.payload,
                 '$.legacy_conversation_selection_conflicts'
             ) IS NOT 'array'
          OR json_array_length(
                 migrated_event.payload,
                 '$.legacy_conversation_selection_conflicts'
             ) <> (
                 SELECT COUNT(*)
                 FROM orch_run_tasks task
                 JOIN _m037_attempt_conversation_stage selected
                   ON selected.execution_id = task.run_id
                  AND selected.step_id = task.id
                  AND selected.is_current = 1
                 WHERE task.run_id = migrated_event.execution_id
                   AND task.conversation_id IS NOT NULL
                   AND task.conversation_id <> selected.conversation_id
             )
          OR EXISTS (
              SELECT 1
              FROM orch_run_tasks task
              JOIN _m037_attempt_conversation_stage selected
                ON selected.execution_id = task.run_id
               AND selected.step_id = task.id
               AND selected.is_current = 1
              WHERE task.run_id = migrated_event.execution_id
                AND task.conversation_id IS NOT NULL
                AND task.conversation_id <> selected.conversation_id
                AND NOT EXISTS (
                    SELECT 1
                    FROM json_each(
                        migrated_event.payload,
                        '$.legacy_conversation_selection_conflicts'
                    ) archived
                    WHERE json_extract(archived.value, '$.step_id') IS task.id
                      AND json_extract(
                              archived.value,
                              '$.configured_conversation_id'
                          ) IS task.conversation_id
                      AND json_extract(
                              archived.value,
                              '$.selected_conversation_id'
                          ) IS selected.conversation_id
                      AND json_extract(
                              archived.value,
                              '$.selection_source'
                          ) IS selected.current_source
                )
          )
      )
);

UPDATE conversations
SET extra = json_remove(
    extra,
    '$.agent_cluster_mode',
    '$.orchestrator_model_range',
    '$.orchestrator_approval_mode',
    '$.orchestrator_run_id',
    '$.orchestrator_task_id',
    '$.orchestrator_delegation_depth',
    '$.orchestrator_role',
    '$.execution_template_id',
    '$.team_id',
    '$.teamId'
);

-- No aliases, compatibility views, or dual-write tables remain after the cut.
DROP TABLE _m037_template_participant_stage;
DROP TABLE _m037_attempt_conversation_stage;
DROP TABLE _m037_legacy_execution_depths;
DROP TABLE _m037_legacy_fork_origins;
DROP TABLE orch_assignments;
DROP TABLE orch_run_task_deps;
DROP TABLE orch_run_tasks;
DROP TABLE orch_runs;
DROP TABLE orch_workspaces;
DROP TABLE fleet_members;
DROP TABLE fleets;
DROP TABLE _m037_guard;
