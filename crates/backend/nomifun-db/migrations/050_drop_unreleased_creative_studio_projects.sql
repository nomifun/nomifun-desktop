-- The Creative Studio workflow surface was never released. Its persisted
-- project shape is therefore disposable, rather than a compatibility contract.
-- Migration 049 renamed the live vocabulary to templates but did not remove
-- projects that still carried the retired left-panel value. Remove those
-- unpublished projects and their server-owned Agent bindings so startup never
-- needs to understand the retired document shape.
--
-- Creation-task rows and workshop assets intentionally remain as history. The
-- v3 data contract allows those historical references to outlive a project,
-- and the next startup can still verify their durable payloads.
DELETE FROM creative_studio_agent_proposal_receipts
WHERE project_id IN (
    SELECT project_id
    FROM creative_studio_projects
    WHERE json_valid(document_json)
      AND json_extract(document_json, '$.panels.left.activeView') = 'workflows'
);

DELETE FROM creative_studio_agent_sessions
WHERE project_id IN (
    SELECT project_id
    FROM creative_studio_projects
    WHERE json_valid(document_json)
      AND json_extract(document_json, '$.panels.left.activeView') = 'workflows'
);

DELETE FROM creative_studio_projects
WHERE json_valid(document_json)
  AND json_extract(document_json, '$.panels.left.activeView') = 'workflows';
