-- One authoritative sales workspace per authenticated WebUI user.
--
-- The workspace is intentionally stored as a versioned JSON document for the
-- first multi-tenant phase.  Ownership lives in a first-class SQL column and
-- is never accepted from client input; route handlers always bind the current
-- authenticated user id.  This makes the isolation boundary enforceable now
-- without freezing the still-evolving sales task/result schema too early.
CREATE TABLE sales_workspaces (
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

CREATE INDEX idx_sales_workspaces_user_id ON sales_workspaces(user_id);
CREATE INDEX idx_sales_workspaces_updated_at ON sales_workspaces(updated_at);
