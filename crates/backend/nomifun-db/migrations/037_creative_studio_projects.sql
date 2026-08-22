-- Canonical Creative Studio projects. This is intentionally a new product
-- surface: legacy workshop_canvases rows and canvas.json files are neither
-- copied nor read through this schema.
--
-- The project document is JSON TEXT owned by `nomifun-workshop`. Keeping the
-- document and its monotonic revision in one SQLite row makes save a single
-- compare-and-swap operation; assets remain in the existing workshop asset
-- store and are referenced by business ID from the document.

CREATE TABLE creative_studio_projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL UNIQUE
        CHECK (
            length(project_id) = 36
            AND lower(project_id) = project_id
            AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    title TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    node_count INTEGER NOT NULL DEFAULT 0 CHECK (node_count >= 0),
    connection_count INTEGER NOT NULL DEFAULT 0 CHECK (connection_count >= 0),
    document_json TEXT NOT NULL
        CHECK (json_valid(document_json))
        CHECK (
            json_type(document_json, '$.schema') = 'text'
            AND json_extract(document_json, '$.schema') = 'nomifun.creative-studio/v1'
        )
        CHECK (
            json_type(document_json, '$.projectId') = 'text'
            AND json_extract(document_json, '$.projectId') = project_id
        ),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_creative_studio_projects_updated
    ON creative_studio_projects(updated_at DESC, id DESC);
