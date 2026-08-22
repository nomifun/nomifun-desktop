-- Give standalone Creative Studio workbenches a durable aggregate owner and
-- persist the ordered input bindings needed to inspect or retry a task without
-- reverse-engineering its idempotency fingerprint.

CREATE TABLE creation_tasks_v4 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    creation_task_id TEXT NOT NULL UNIQUE
        CHECK (
            length(creation_task_id) = 36
            AND lower(creation_task_id) = creation_task_id
            AND creation_task_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(creation_task_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    project_id TEXT
        CHECK (
            project_id IS NULL
            OR (
                length(project_id) = 36
                AND lower(project_id) = project_id
                AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    workbench_kind TEXT
        CHECK (workbench_kind IS NULL OR workbench_kind IN ('image', 'video', 'audio')),
    workflow_id TEXT
        CHECK (
            workflow_id IS NULL
            OR (
                length(workflow_id) = 36
                AND lower(workflow_id) = workflow_id
                AND workflow_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(workflow_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    workflow_run_id TEXT
        CHECK (
            workflow_run_id IS NULL
            OR (
                length(workflow_run_id) = 36
                AND lower(workflow_run_id) = workflow_run_id
                AND workflow_run_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(workflow_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    workflow_step_id TEXT
        CHECK (
            workflow_step_id IS NULL
            OR (
                length(workflow_step_id) = 36
                AND lower(workflow_step_id) = workflow_step_id
                AND workflow_step_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(workflow_step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    node_id TEXT
        CHECK (
            node_id IS NULL
            OR (
                length(node_id) = 36
                AND lower(node_id) = node_id
                AND node_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(node_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    provider_id TEXT NOT NULL
        CHECK (
            length(provider_id) = 36
            AND lower(provider_id) = provider_id
            AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    model TEXT NOT NULL,
    capability TEXT NOT NULL,
    params TEXT NOT NULL,
    -- NULL is reserved for a pre-043 task whose exact ordered bindings could
    -- not be proven from both request_fingerprint and workshop_assets. New
    -- repository writes always provide a canonical JSON array (including []).
    input_bindings TEXT
        CHECK (
            input_bindings IS NULL
            OR (json_valid(input_bindings) AND json_type(input_bindings) = 'array')
        ),
    status TEXT NOT NULL,
    error TEXT,
    result_asset_ids TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(result_asset_ids) AND json_type(result_asset_ids) = 'array'),
    remote_task_id TEXT,
    attempt INTEGER NOT NULL DEFAULT 0,
    submitted_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    request_fingerprint TEXT NOT NULL
        CHECK (
            json_valid(request_fingerprint)
            AND json_type(request_fingerprint) = 'object'
        ),
    CHECK (
        -- Canonical project-node owner.
        (
            project_id IS NOT NULL
            AND node_id IS NOT NULL
            AND workbench_kind IS NULL
            AND workflow_id IS NULL
            AND workflow_run_id IS NULL
            AND workflow_step_id IS NULL
        )
        OR
        -- Canonical standalone-workbench aggregate owner.
        (
            project_id IS NOT NULL
            AND workbench_kind IS NOT NULL
            AND node_id IS NULL
            AND workflow_id IS NULL
            AND workflow_run_id IS NULL
            AND workflow_step_id IS NULL
        )
        OR
        -- Canonical workflow-step owner.
        (
            project_id IS NULL
            AND workbench_kind IS NULL
            AND node_id IS NULL
            AND workflow_id IS NOT NULL
            AND workflow_run_id IS NOT NULL
            AND workflow_step_id IS NOT NULL
        )
    )
);

INSERT INTO creation_tasks_v4 (
    id, creation_task_id, project_id, workbench_kind, workflow_id,
    workflow_run_id, workflow_step_id, node_id, provider_id, model,
    capability, params, input_bindings, status, error, result_asset_ids,
    remote_task_id, attempt, submitted_at, started_at, finished_at,
    request_fingerprint
)
SELECT
    task.id,
    task.creation_task_id,
    task.project_id,
    NULL,
    task.workflow_id,
    task.workflow_run_id,
    task.workflow_step_id,
    task.node_id,
    task.provider_id,
    task.model,
    task.capability,
    task.params,
    CASE
        -- An explicit array is the only proof that the old request captured
        -- its complete input order. Missing/non-array inputs stay NULL rather
        -- than being fabricated as an empty list.
        WHEN json_type(task.request_fingerprint, '$.inputs') = 'array'
         AND NOT EXISTS (
            SELECT 1
            FROM json_each(task.request_fingerprint, '$.inputs') AS input
            LEFT JOIN workshop_assets AS asset
              ON asset.asset_id = json_extract(input.value, '$.asset_id')
            WHERE json_type(input.value) IS NOT 'object'
               OR json_type(input.value, '$.asset_id') IS NOT 'text'
               OR json_type(input.value, '$.role') IS NOT 'text'
               OR (SELECT COUNT(*) FROM json_each(input.value)) <> 2
               OR EXISTS (
                    SELECT 1 FROM json_each(input.value) AS field
                    WHERE field.key NOT IN ('asset_id', 'role')
               )
               OR json_extract(input.value, '$.role') NOT IN (
                    'reference', 'mask', 'first_frame', 'last_frame', 'video', 'audio'
               )
               OR asset.asset_id IS NULL
               OR asset.kind NOT IN ('image', 'video', 'audio', 'text')
         )
        THEN (
            SELECT COALESCE(json_group_array(json(ordered.binding)), '[]')
            FROM (
                SELECT json_object(
                    'asset_id', json_extract(input.value, '$.asset_id'),
                    'kind', asset.kind,
                    'role', json_extract(input.value, '$.role')
                ) AS binding
                FROM json_each(task.request_fingerprint, '$.inputs') AS input
                JOIN workshop_assets AS asset
                  ON asset.asset_id = json_extract(input.value, '$.asset_id')
                ORDER BY CAST(input.key AS INTEGER)
            ) AS ordered
        )
        ELSE NULL
    END,
    task.status,
    task.error,
    task.result_asset_ids,
    task.remote_task_id,
    task.attempt,
    task.submitted_at,
    task.started_at,
    task.finished_at,
    task.request_fingerprint
FROM creation_tasks AS task;

DROP TABLE creation_tasks;
ALTER TABLE creation_tasks_v4 RENAME TO creation_tasks;

CREATE INDEX idx_creation_tasks_project_id ON creation_tasks(project_id);
CREATE INDEX idx_creation_tasks_workbench_owner
    ON creation_tasks(project_id, workbench_kind, submitted_at DESC, creation_task_id DESC)
    WHERE workbench_kind IS NOT NULL;
CREATE INDEX idx_creation_tasks_workflow_id ON creation_tasks(workflow_id);
CREATE INDEX idx_creation_tasks_workflow_run_id ON creation_tasks(workflow_run_id);
CREATE INDEX idx_creation_tasks_provider_id ON creation_tasks(provider_id);
CREATE INDEX idx_creation_tasks_input_bindings_json ON creation_tasks(input_bindings);
CREATE INDEX idx_creation_tasks_result_asset_ids_json ON creation_tasks(result_asset_ids);
CREATE INDEX idx_creation_tasks_status ON creation_tasks(status);

CREATE TRIGGER validate_creation_task_input_bindings_insert
BEFORE INSERT ON creation_tasks
WHEN NEW.input_bindings IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid creation task input binding')
    WHERE EXISTS (
        SELECT 1
        FROM json_each(NEW.input_bindings) AS input
        WHERE json_type(input.value) IS NOT 'object'
           OR json_type(input.value, '$.asset_id') IS NOT 'text'
           OR length(json_extract(input.value, '$.asset_id')) <> 36
           OR lower(json_extract(input.value, '$.asset_id')) <> json_extract(input.value, '$.asset_id')
           OR json_extract(input.value, '$.asset_id') NOT GLOB '????????-????-7???-[89ab]???-????????????'
           OR replace(json_extract(input.value, '$.asset_id'), '-', '') GLOB '*[^0-9a-f]*'
           OR json_type(input.value, '$.kind') IS NOT 'text'
           OR json_extract(input.value, '$.kind') NOT IN ('image', 'video', 'audio', 'text')
           OR json_type(input.value, '$.role') IS NOT 'text'
           OR json_extract(input.value, '$.role') NOT IN (
                'reference', 'mask', 'first_frame', 'last_frame', 'video', 'audio'
           )
           OR (SELECT COUNT(*) FROM json_each(input.value)) <> 3
           OR EXISTS (
                SELECT 1 FROM json_each(input.value) AS field
                WHERE field.key NOT IN ('asset_id', 'kind', 'role')
           )
    );
END;

CREATE TRIGGER validate_creation_task_input_bindings_update
BEFORE UPDATE OF input_bindings ON creation_tasks
WHEN NEW.input_bindings IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid creation task input binding')
    WHERE EXISTS (
        SELECT 1
        FROM json_each(NEW.input_bindings) AS input
        WHERE json_type(input.value) IS NOT 'object'
           OR json_type(input.value, '$.asset_id') IS NOT 'text'
           OR length(json_extract(input.value, '$.asset_id')) <> 36
           OR lower(json_extract(input.value, '$.asset_id')) <> json_extract(input.value, '$.asset_id')
           OR json_extract(input.value, '$.asset_id') NOT GLOB '????????-????-7???-[89ab]???-????????????'
           OR replace(json_extract(input.value, '$.asset_id'), '-', '') GLOB '*[^0-9a-f]*'
           OR json_type(input.value, '$.kind') IS NOT 'text'
           OR json_extract(input.value, '$.kind') NOT IN ('image', 'video', 'audio', 'text')
           OR json_type(input.value, '$.role') IS NOT 'text'
           OR json_extract(input.value, '$.role') NOT IN (
                'reference', 'mask', 'first_frame', 'last_frame', 'video', 'audio'
           )
           OR (SELECT COUNT(*) FROM json_each(input.value)) <> 3
           OR EXISTS (
                SELECT 1 FROM json_each(input.value) AS field
                WHERE field.key NOT IN ('asset_id', 'kind', 'role')
           )
    );
END;

-- Extend generated-asset provenance with the same exact owner branch. These
-- triggers complement the immutable workshop_assets table CHECK constraints.
DROP TRIGGER IF EXISTS validate_creative_asset_origin_insert;
DROP TRIGGER IF EXISTS validate_creative_asset_origin_update;

CREATE TRIGGER validate_creative_asset_origin_insert
BEFORE INSERT ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'unsupported creative asset origin id key')
    WHERE json_type(NEW.origin, '$.task_id') IS NOT NULL
       OR json_type(NEW.origin, '$.providerId') IS NOT NULL
       OR json_type(NEW.origin, '$.canvas_id') IS NOT NULL
       OR json_type(NEW.origin, '$.canvasId') IS NOT NULL
       OR json_type(NEW.origin, '$.nodeId') IS NOT NULL
       OR json_type(NEW.origin, '$.creationTaskId') IS NOT NULL
       OR json_type(NEW.origin, '$.projectId') IS NOT NULL
       OR json_type(NEW.origin, '$.workbenchKind') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowStepId') IS NOT NULL;
    SELECT RAISE(ABORT, 'invalid creative asset origin project_id')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.project_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.project_id')) = 36
          AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
          AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_id')
    WHERE json_type(NEW.origin, '$.workflow_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_id')) = json_extract(NEW.origin, '$.workflow_id')
          AND json_extract(NEW.origin, '$.workflow_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_run_id')
    WHERE json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_run_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_run_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_run_id')) = json_extract(NEW.origin, '$.workflow_run_id')
          AND json_extract(NEW.origin, '$.workflow_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_step_id')
    WHERE json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_step_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_step_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_step_id')) = json_extract(NEW.origin, '$.workflow_step_id')
          AND json_extract(NEW.origin, '$.workflow_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );

    SELECT RAISE(ABORT, 'invalid creative asset origin workbench_kind')
    WHERE json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workbench_kind') IS 'text'
          AND json_extract(NEW.origin, '$.workbench_kind') IN ('image', 'video', 'audio')
      );
    SELECT RAISE(ABORT, 'invalid creative asset owner branch')
    WHERE (
        json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
    ) AND NOT (
        (
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.workflow_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_run_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_step_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
        )
    );
END;

CREATE TRIGGER validate_creative_asset_origin_update
BEFORE UPDATE OF origin ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'unsupported creative asset origin id key')
    WHERE json_type(NEW.origin, '$.task_id') IS NOT NULL
       OR json_type(NEW.origin, '$.providerId') IS NOT NULL
       OR json_type(NEW.origin, '$.canvas_id') IS NOT NULL
       OR json_type(NEW.origin, '$.canvasId') IS NOT NULL
       OR json_type(NEW.origin, '$.nodeId') IS NOT NULL
       OR json_type(NEW.origin, '$.creationTaskId') IS NOT NULL
       OR json_type(NEW.origin, '$.projectId') IS NOT NULL
       OR json_type(NEW.origin, '$.workbenchKind') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowStepId') IS NOT NULL;
    SELECT RAISE(ABORT, 'invalid creative asset origin project_id')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.project_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.project_id')) = 36
          AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
          AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_id')
    WHERE json_type(NEW.origin, '$.workflow_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_id')) = json_extract(NEW.origin, '$.workflow_id')
          AND json_extract(NEW.origin, '$.workflow_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_run_id')
    WHERE json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_run_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_run_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_run_id')) = json_extract(NEW.origin, '$.workflow_run_id')
          AND json_extract(NEW.origin, '$.workflow_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_step_id')
    WHERE json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_step_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_step_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_step_id')) = json_extract(NEW.origin, '$.workflow_step_id')
          AND json_extract(NEW.origin, '$.workflow_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );

    SELECT RAISE(ABORT, 'invalid creative asset origin workbench_kind')
    WHERE json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workbench_kind') IS 'text'
          AND json_extract(NEW.origin, '$.workbench_kind') IN ('image', 'video', 'audio')
      );
    SELECT RAISE(ABORT, 'invalid creative asset owner branch')
    WHERE (
        json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
    ) AND NOT (
        (
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.workflow_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_run_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_step_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
        )
    );
END;
