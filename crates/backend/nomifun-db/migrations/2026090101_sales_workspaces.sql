-- One authoritative sales workspace per authenticated WebUI user.
--
-- Custom product migrations use a date-based range so they cannot collide
-- with upstream NomiFun's sequential migration numbers. IF NOT EXISTS keeps
-- upgrades safe for local databases that already created this table before
-- the migration-number split was introduced.
CREATE TABLE IF NOT EXISTS sales_workspaces (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id        TEXT NOT NULL UNIQUE
                   CHECK (
                       length(user_id) = 36
                       AND lower(user_id) = user_id
                       AND user_id GLOB '????????-????-7???-[89ab]???-????????????'
                       AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                   ),
    workspace_json TEXT NOT NULL
                   CHECK (json_valid(workspace_json) AND json_type(workspace_json) = 'object'),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sales_workspaces_user_id ON sales_workspaces(user_id);
CREATE INDEX IF NOT EXISTS idx_sales_workspaces_updated_at ON sales_workspaces(updated_at);
