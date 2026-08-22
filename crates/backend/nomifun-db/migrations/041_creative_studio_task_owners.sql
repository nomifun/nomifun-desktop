-- Durable Workflow runs and a strict discriminated owner for canonical
-- Creative Studio creation tasks.
--
-- The legacy `/api/creation/tasks` surface keeps its historical nullable
-- canvas ownership until the old Workshop is removed. The new
-- `/api/creative-studio/tasks` surface uses exactly one canonical branch:
--
--   canvas_node   = project_id + node_id
--   workflow_step = workflow_id + workflow_run_id + workflow_step_id

CREATE TABLE creative_studio_workflow_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_run_id TEXT NOT NULL UNIQUE
        CHECK (
            length(workflow_run_id) = 36
            AND lower(workflow_run_id) = workflow_run_id
            AND workflow_run_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(workflow_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    workflow_id TEXT NOT NULL
        CHECK (
            length(workflow_id) = 36
            AND lower(workflow_id) = workflow_id
            AND workflow_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(workflow_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    workflow_revision INTEGER NOT NULL CHECK (workflow_revision >= 1),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    status TEXT NOT NULL
        CHECK (status IN (
            'requested', 'awaiting-review', 'queued', 'running',
            'succeeded', 'failed', 'cancelled'
        )),
    step_ids_json TEXT NOT NULL
        CHECK (
            json_valid(step_ids_json)
            AND json_type(step_ids_json) = 'array'
            AND json_array_length(step_ids_json) BETWEEN 1 AND 128
        ),
    aggregate_json TEXT NOT NULL
        CHECK (json_valid(aggregate_json) AND json_type(aggregate_json) = 'object')
        CHECK (json_extract(aggregate_json, '$.kind') = 'nomifun.creative-studio.workflow-run')
        CHECK (json_extract(aggregate_json, '$.version') = 1)
        CHECK (json_extract(aggregate_json, '$.revision') = revision)
        CHECK (json_extract(aggregate_json, '$.workflowSnapshot.id') = workflow_id)
        CHECK (json_extract(aggregate_json, '$.workflowSnapshot.revision') = workflow_revision)
        CHECK (json_extract(aggregate_json, '$.request.id') = workflow_run_id)
        CHECK (json_extract(aggregate_json, '$.request.workflowId') = workflow_id)
        CHECK (json_extract(aggregate_json, '$.request.workflowRevision') = workflow_revision)
        CHECK (json_extract(aggregate_json, '$.record.requestId') = workflow_run_id)
        CHECK (json_extract(aggregate_json, '$.record.workflowId') = workflow_id)
        CHECK (json_extract(aggregate_json, '$.record.status') = status),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
);

CREATE INDEX idx_creative_workflow_runs_workflow_id
    ON creative_studio_workflow_runs(workflow_id, updated_at DESC, id DESC);
CREATE INDEX idx_creative_workflow_runs_status
    ON creative_studio_workflow_runs(status, updated_at DESC, id DESC);

-- Migration 037 added column-level CHECK clauses whose project-only predicate
-- cannot express a second canonical owner. Rebuild once with the final tagged
-- union instead of stacking nullable compatibility columns.
CREATE TABLE creation_tasks_v2 (
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
    canvas_id TEXT
        CHECK (
            canvas_id IS NULL
            OR (
                length(canvas_id) = 36
                AND lower(canvas_id) = canvas_id
                AND canvas_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(canvas_id, '-', '') NOT GLOB '*[^0-9a-f]*'
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
    status TEXT NOT NULL,
    error TEXT,
    result_asset_ids TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(result_asset_ids) AND json_type(result_asset_ids) = 'array'),
    remote_task_id TEXT,
    attempt INTEGER NOT NULL DEFAULT 0,
    submitted_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    request_fingerprint TEXT,
    CHECK (
        -- Historical Workshop/global task. Canonical owner columns and the
        -- idempotent fingerprint must all be absent.
        (
            request_fingerprint IS NULL
            AND project_id IS NULL
            AND workflow_id IS NULL
            AND workflow_run_id IS NULL
            AND workflow_step_id IS NULL
        )
        OR
        -- Canonical canvas-node owner.
        (
            request_fingerprint IS NOT NULL
            AND json_valid(request_fingerprint)
            AND json_type(request_fingerprint) = 'object'
            AND project_id IS NOT NULL
            AND node_id IS NOT NULL
            AND canvas_id IS NULL
            AND workflow_id IS NULL
            AND workflow_run_id IS NULL
            AND workflow_step_id IS NULL
        )
        OR
        -- Canonical workflow-step owner.
        (
            request_fingerprint IS NOT NULL
            AND json_valid(request_fingerprint)
            AND json_type(request_fingerprint) = 'object'
            AND workflow_id IS NOT NULL
            AND workflow_run_id IS NOT NULL
            AND workflow_step_id IS NOT NULL
            AND project_id IS NULL
            AND canvas_id IS NULL
            AND node_id IS NULL
        )
    )
);

INSERT INTO creation_tasks_v2 (
    id, creation_task_id, project_id, workflow_id, workflow_run_id,
    workflow_step_id, canvas_id, node_id, provider_id, model, capability,
    params, status, error, result_asset_ids, remote_task_id, attempt,
    submitted_at, started_at, finished_at, request_fingerprint
)
SELECT
    id, creation_task_id, project_id, NULL, NULL,
    NULL, canvas_id, node_id, provider_id, model, capability,
    params, status, error, result_asset_ids, remote_task_id, attempt,
    submitted_at, started_at, finished_at, request_fingerprint
FROM creation_tasks;

DROP TABLE creation_tasks;
ALTER TABLE creation_tasks_v2 RENAME TO creation_tasks;

CREATE INDEX idx_creation_tasks_canvas_id ON creation_tasks(canvas_id);
CREATE INDEX idx_creation_tasks_project_id ON creation_tasks(project_id);
CREATE INDEX idx_creation_tasks_workflow_id ON creation_tasks(workflow_id);
CREATE INDEX idx_creation_tasks_workflow_run_id ON creation_tasks(workflow_run_id);
CREATE INDEX idx_creation_tasks_provider_id ON creation_tasks(provider_id);
CREATE INDEX idx_creation_tasks_result_asset_ids_json ON creation_tasks(result_asset_ids);
CREATE INDEX idx_creation_tasks_status ON creation_tasks(status);

-- Generated-asset provenance carries the same tagged owner. The baseline
-- asset table predates Creative Studio, so enforce new keys with write-time
-- triggers instead of leaving unvalidated identifiers in an open JSON object.
CREATE INDEX idx_workshop_assets_origin_project_id
    ON workshop_assets(json_extract(origin, '$.project_id'))
    WHERE origin IS NOT NULL;
CREATE INDEX idx_workshop_assets_origin_workflow_id
    ON workshop_assets(json_extract(origin, '$.workflow_id'))
    WHERE origin IS NOT NULL;
CREATE INDEX idx_workshop_assets_origin_workflow_run_id
    ON workshop_assets(json_extract(origin, '$.workflow_run_id'))
    WHERE origin IS NOT NULL;
CREATE INDEX idx_workshop_assets_origin_workflow_step_id
    ON workshop_assets(json_extract(origin, '$.workflow_step_id'))
    WHERE origin IS NOT NULL;

CREATE TRIGGER validate_creative_asset_origin_insert
BEFORE INSERT ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'unsupported creative asset origin id key')
    WHERE json_type(NEW.origin, '$.projectId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowStepId') IS NOT NULL;
    SELECT RAISE(ABORT, 'invalid creative asset origin project_id')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.project_id') = 'text'
          AND length(json_extract(NEW.origin, '$.project_id')) = 36
          AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
          AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_id')
    WHERE json_type(NEW.origin, '$.workflow_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_id') = 'text'
          AND length(json_extract(NEW.origin, '$.workflow_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_id')) = json_extract(NEW.origin, '$.workflow_id')
          AND json_extract(NEW.origin, '$.workflow_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_run_id')
    WHERE json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_run_id') = 'text'
          AND length(json_extract(NEW.origin, '$.workflow_run_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_run_id')) = json_extract(NEW.origin, '$.workflow_run_id')
          AND json_extract(NEW.origin, '$.workflow_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_step_id')
    WHERE json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_step_id') = 'text'
          AND length(json_extract(NEW.origin, '$.workflow_step_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_step_id')) = json_extract(NEW.origin, '$.workflow_step_id')
          AND json_extract(NEW.origin, '$.workflow_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset owner branch')
    WHERE (
        json_type(NEW.origin, '$.project_id') IS NOT NULL
        AND NOT (
            json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
    ) OR (
        (
            json_type(NEW.origin, '$.workflow_id') IS NOT NULL
            OR json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
            OR json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
        )
        AND NOT (
            json_type(NEW.origin, '$.workflow_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_run_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_step_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
        )
    );
END;

CREATE TRIGGER validate_creative_asset_origin_update
BEFORE UPDATE OF origin ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'unsupported creative asset origin id key')
    WHERE json_type(NEW.origin, '$.projectId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowStepId') IS NOT NULL;
    SELECT RAISE(ABORT, 'invalid creative asset origin project_id')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.project_id') = 'text'
          AND length(json_extract(NEW.origin, '$.project_id')) = 36
          AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
          AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_id')
    WHERE json_type(NEW.origin, '$.workflow_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_id') = 'text'
          AND length(json_extract(NEW.origin, '$.workflow_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_id')) = json_extract(NEW.origin, '$.workflow_id')
          AND json_extract(NEW.origin, '$.workflow_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_run_id')
    WHERE json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_run_id') = 'text'
          AND length(json_extract(NEW.origin, '$.workflow_run_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_run_id')) = json_extract(NEW.origin, '$.workflow_run_id')
          AND json_extract(NEW.origin, '$.workflow_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_step_id')
    WHERE json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_step_id') = 'text'
          AND length(json_extract(NEW.origin, '$.workflow_step_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_step_id')) = json_extract(NEW.origin, '$.workflow_step_id')
          AND json_extract(NEW.origin, '$.workflow_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset owner branch')
    WHERE (
        json_type(NEW.origin, '$.project_id') IS NOT NULL
        AND NOT (
            json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
    ) OR (
        (
            json_type(NEW.origin, '$.workflow_id') IS NOT NULL
            OR json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
            OR json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
        )
        AND NOT (
            json_type(NEW.origin, '$.workflow_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_run_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_step_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
        )
    );
END;
