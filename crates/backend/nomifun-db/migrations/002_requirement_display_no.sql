-- Give every requirement a compact, immutable, human-facing number while
-- retaining the canonical req_<UUIDv7> primary key for system boundaries.
--
-- The database startup path runs rebuild migrations with foreign_keys=OFF and
-- legacy_alter_table=ON, so attachments / requirement_tags keep referencing
-- the final `requirements` table name while this table is replaced.

CREATE TABLE requirements_new (
    id               TEXT PRIMARY KEY NOT NULL,
    display_no       INTEGER NOT NULL UNIQUE CHECK (display_no > 0),
    title            TEXT    NOT NULL,
    content          TEXT    NOT NULL DEFAULT '',
    tag              TEXT    NOT NULL,
    order_key        TEXT    NOT NULL DEFAULT '',
    sort_seq         TEXT    NOT NULL DEFAULT '',
    status           TEXT    NOT NULL DEFAULT 'pending',
    priority         INTEGER NOT NULL DEFAULT 0,
    completion_note  TEXT,
    owner_conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
    owner_terminal_id     TEXT REFERENCES terminal_sessions(id) ON DELETE SET NULL,
    active_turn_started_at INTEGER,
    lease_expires_at INTEGER,
    started_at       INTEGER,
    completed_at     INTEGER,
    attempt_count    INTEGER NOT NULL DEFAULT 0,
    created_by       TEXT    NOT NULL DEFAULT 'user',
    extra            TEXT    NOT NULL DEFAULT '{}',
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    CHECK (owner_conversation_id IS NULL OR owner_terminal_id IS NULL)
);

INSERT INTO requirements_new (
    id, display_no, title, content, tag, order_key, sort_seq, status, priority,
    completion_note, owner_conversation_id, owner_terminal_id,
    active_turn_started_at, lease_expires_at, started_at, completed_at,
    attempt_count, created_by, extra, created_at, updated_at
)
SELECT
    id,
    ROW_NUMBER() OVER (ORDER BY created_at ASC, id ASC),
    title, content, tag, order_key, sort_seq, status, priority,
    completion_note, owner_conversation_id, owner_terminal_id,
    active_turn_started_at, lease_expires_at, started_at, completed_at,
    attempt_count, created_by, extra, created_at, updated_at
FROM requirements;

DROP TABLE requirements;
ALTER TABLE requirements_new RENAME TO requirements;

CREATE INDEX idx_requirements_owner_conversation ON requirements(owner_conversation_id);
CREATE INDEX idx_requirements_owner_terminal ON requirements(owner_terminal_id);
CREATE INDEX idx_requirements_status ON requirements(status);
CREATE INDEX idx_requirements_tag_order ON requirements(tag, sort_seq);
CREATE INDEX idx_requirements_tag_status ON requirements(tag, status);

-- A singleton counter prevents deleted high-number requirements from causing
-- their visible numbers to be reused. Allocation and insertion happen in one
-- transaction in SqliteRequirementRepository::insert.
CREATE TABLE requirement_display_sequence (
    singleton TEXT PRIMARY KEY CHECK (singleton = 'requirements'),
    last_no   INTEGER NOT NULL CHECK (last_no >= 0)
);

INSERT INTO requirement_display_sequence (singleton, last_no)
SELECT 'requirements', COALESCE(MAX(display_no), 0) FROM requirements;

-- These two table-owned guards are dropped with the old requirements table;
-- recreate them verbatim so the installation-owner boundary remains intact.
CREATE TRIGGER requirement_owner_insert_guard
BEFORE INSERT ON requirements
WHEN (NEW.owner_conversation_id IS NOT NULL AND NEW.owner_terminal_id IS NOT NULL)
  OR (NEW.owner_conversation_id IS NOT NULL AND NOT EXISTS (
         SELECT 1 FROM conversations conversation
         WHERE conversation.id = NEW.owner_conversation_id
           AND conversation.user_id = (
               SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
           )
     ))
  OR (NEW.owner_terminal_id IS NOT NULL AND NOT EXISTS (
         SELECT 1 FROM terminal_sessions terminal
         WHERE terminal.id = NEW.owner_terminal_id
           AND terminal.user_id = (
               SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
           )
     ))
BEGIN
    SELECT RAISE(ABORT, 'requirement owner must be one typed installation-owner session');
END;

CREATE TRIGGER requirement_owner_update_guard
BEFORE UPDATE OF owner_conversation_id, owner_terminal_id ON requirements
WHEN (NEW.owner_conversation_id IS NOT NULL AND NEW.owner_terminal_id IS NOT NULL)
  OR (NEW.owner_conversation_id IS NOT NULL AND NOT EXISTS (
         SELECT 1 FROM conversations conversation
         WHERE conversation.id = NEW.owner_conversation_id
           AND conversation.user_id = (
               SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
           )
     ))
  OR (NEW.owner_terminal_id IS NOT NULL AND NOT EXISTS (
         SELECT 1 FROM terminal_sessions terminal
         WHERE terminal.id = NEW.owner_terminal_id
           AND terminal.user_id = (
               SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
           )
     ))
BEGIN
    SELECT RAISE(ABORT, 'requirement owner must be one typed installation-owner session');
END;

-- Keep the counter correct for maintenance/test inserts that explicitly
-- provide a display number outside the repository allocator.
CREATE TRIGGER requirements_display_sequence_sync
AFTER INSERT ON requirements
WHEN NEW.display_no > (SELECT last_no FROM requirement_display_sequence WHERE singleton = 'requirements')
BEGIN
    UPDATE requirement_display_sequence SET last_no = NEW.display_no WHERE singleton = 'requirements';
END;

CREATE TRIGGER requirements_display_no_immutable
BEFORE UPDATE OF display_no ON requirements
WHEN NEW.display_no <> OLD.display_no
BEGIN
    SELECT RAISE(ABORT, 'requirement display_no is immutable');
END;
