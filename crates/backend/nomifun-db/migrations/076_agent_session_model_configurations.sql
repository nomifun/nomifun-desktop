-- Session model choices freeze a separate configuration without changing the
-- user's Agent or adding model-specific copies to the visible Agent library.
ALTER TABLE nomi_agent_presets ADD COLUMN session_only INTEGER NOT NULL DEFAULT 0
    CHECK (session_only IN (0, 1));
