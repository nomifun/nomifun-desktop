-- Server-owned binding between one Creative Studio project chat session and
-- one dedicated Nomi Conversation. Public Conversation JSON cannot create or
-- mutate these rows. The repository resolves the binding and inserts the
-- Conversation in one SQLite transaction.

CREATE TABLE creative_studio_agent_sessions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_id        TEXT NOT NULL
                    CHECK (
                        length(owner_id) = 36
                        AND lower(owner_id) = owner_id
                        AND owner_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    project_id      TEXT NOT NULL
                    CHECK (
                        length(project_id) = 36
                        AND lower(project_id) = project_id
                        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    session_id      TEXT NOT NULL
                    CHECK (
                        length(session_id) = 36
                        AND lower(session_id) = session_id
                        AND session_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    conversation_id TEXT NOT NULL
                    CHECK (
                        length(conversation_id) = 36
                        AND lower(conversation_id) = conversation_id
                        AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (owner_id, project_id, session_id)
);

CREATE INDEX idx_creative_agent_sessions_owner
    ON creative_studio_agent_sessions(owner_id);
CREATE INDEX idx_creative_agent_sessions_project
    ON creative_studio_agent_sessions(project_id);
CREATE UNIQUE INDEX idx_creative_agent_sessions_session
    ON creative_studio_agent_sessions(session_id);
CREATE UNIQUE INDEX idx_creative_agent_sessions_conversation
    ON creative_studio_agent_sessions(conversation_id);
