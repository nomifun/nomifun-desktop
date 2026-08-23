-- Canonical Creative Studio workflow definitions.
--
-- Workflows are installation-scoped product entities, not legacy Workshop
-- canvas documents and not hidden text assets.  The definition JSON is a
-- closed v1 contract owned by `nomifun-workshop`; revision is duplicated in
-- the row so every edit can be guarded by one SQLite compare-and-swap.

CREATE TABLE creative_studio_workflows (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id TEXT NOT NULL UNIQUE
        CHECK (
            length(workflow_id) = 36
            AND lower(workflow_id) = workflow_id
            AND workflow_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(workflow_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
    description TEXT NOT NULL CHECK (length(description) <= 2000),
    category TEXT NOT NULL CHECK (length(category) <= 80),
    visibility TEXT NOT NULL CHECK (visibility IN ('private', 'public')),
    definition_json TEXT NOT NULL
        CHECK (json_valid(definition_json))
        CHECK (
            json_type(definition_json, '$.id') = 'text'
            AND json_extract(definition_json, '$.id') = workflow_id
        )
        CHECK (
            json_type(definition_json, '$.revision') = 'integer'
            AND json_extract(definition_json, '$.revision') = revision
        ),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
);

CREATE INDEX idx_creative_studio_workflows_updated
    ON creative_studio_workflows(updated_at DESC, id DESC);

CREATE INDEX idx_creative_studio_workflows_category
    ON creative_studio_workflows(category, updated_at DESC, id DESC);
