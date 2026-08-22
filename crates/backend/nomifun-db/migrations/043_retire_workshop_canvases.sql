-- Retire the pre-Creative-Studio canvas index. Canonical project documents
-- live entirely in creative_studio_projects; workshop_assets remains the
-- shared binary/text library but may only carry canonical project/workflow
-- provenance.

DROP TRIGGER IF EXISTS validate_creative_asset_origin_insert;
DROP TRIGGER IF EXISTS validate_creative_asset_origin_update;
DROP INDEX IF EXISTS idx_workshop_assets_origin_canvas_id;

-- Preserve user assets while removing provenance that belonged to the
-- discarded file-backed canvas product. A node ID from that branch has no
-- meaning without its canvas, so retire the pair together. Portable metadata
-- such as prompt/model/provider/params remains intact.
UPDATE workshop_assets
SET origin = NULLIF(json_remove(origin, '$.canvas_id', '$.node_id'), '{}')
WHERE origin IS NOT NULL
  AND json_type(origin, '$.canvas_id') IS NOT NULL;

DROP TABLE workshop_canvases;

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
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
    ) OR (
        json_type(NEW.origin, '$.node_id') IS NOT NULL
        AND json_type(NEW.origin, '$.project_id') IS NULL
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
            AND json_type(NEW.origin, '$.node_id') IS NULL
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
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
    ) OR (
        json_type(NEW.origin, '$.node_id') IS NOT NULL
        AND json_type(NEW.origin, '$.project_id') IS NULL
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
            AND json_type(NEW.origin, '$.node_id') IS NULL
        )
    );
END;
