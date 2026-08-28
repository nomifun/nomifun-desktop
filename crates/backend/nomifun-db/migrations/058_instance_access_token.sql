-- Remote MCP / REST access belongs to the NomiFun Desktop installation, not
-- to a companion.  Do not promote any legacy companion token: doing so would
-- silently widen a credential that was minted for one companion into full
-- installation-owner authority.  Operators must mint a new instance token.
CREATE TABLE instance_access_token (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key TEXT NOT NULL UNIQUE CHECK (singleton_key = 'instance'),
    token_hash    TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

DROP TABLE companion_access_token;
