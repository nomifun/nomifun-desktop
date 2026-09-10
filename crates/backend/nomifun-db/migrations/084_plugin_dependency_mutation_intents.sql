-- Durable Plugin Project dependency mutation intent.
--
-- The filesystem journal owns recovery of package.json + dependency-lock.json;
-- this row owns the matching SQLite generation transition. While an intent
-- exists every ordinary Project UPDATE/DELETE is fenced. The repository inserts
-- one exact commit marker only for the final source/lock/generation CAS.

CREATE TABLE plugin_dependency_mutation_intents (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,
    intent_id                   TEXT NOT NULL UNIQUE CHECK (
        length(intent_id) = 36
        AND lower(intent_id) = intent_id
        AND intent_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(intent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_id                  TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id               TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    expected_project_updated_at INTEGER NOT NULL CHECK (expected_project_updated_at >= 0),
    expected_build_generation   INTEGER NOT NULL CHECK (expected_build_generation >= 0),
    expected_source_digest      TEXT NOT NULL CHECK (
        length(expected_source_digest) = 64
        AND lower(expected_source_digest) = expected_source_digest
        AND expected_source_digest NOT GLOB '*[^0-9a-f]*'
    ),
    expected_lock_digest        TEXT NOT NULL CHECK (
        length(expected_lock_digest) = 64
        AND lower(expected_lock_digest) = expected_lock_digest
        AND expected_lock_digest NOT GLOB '*[^0-9a-f]*'
    ),
    next_source_digest          TEXT NOT NULL CHECK (
        length(next_source_digest) = 64
        AND lower(next_source_digest) = next_source_digest
        AND next_source_digest NOT GLOB '*[^0-9a-f]*'
    ),
    next_lock_digest            TEXT NOT NULL CHECK (
        length(next_lock_digest) = 64
        AND lower(next_lock_digest) = next_lock_digest
        AND next_lock_digest NOT GLOB '*[^0-9a-f]*'
    ),
    created_at                  INTEGER NOT NULL CHECK (created_at > 0),
    CHECK (
        expected_source_digest <> next_source_digest
        OR expected_lock_digest <> next_lock_digest
    ),
    UNIQUE (owner_user_id, project_id)
);

CREATE INDEX idx_plugin_dependency_mutation_intents_owner_user_id
    ON plugin_dependency_mutation_intents(owner_user_id);
CREATE INDEX idx_plugin_dependency_mutation_intents_project_id
    ON plugin_dependency_mutation_intents(project_id);

CREATE TABLE plugin_dependency_mutation_commits (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    intent_id  TEXT NOT NULL UNIQUE CHECK (
        length(intent_id) = 36
        AND lower(intent_id) = intent_id
        AND intent_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(intent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    created_at INTEGER NOT NULL CHECK (created_at > 0)
);

CREATE INDEX idx_plugin_dependency_mutation_commits_project_id
    ON plugin_dependency_mutation_commits(project_id);
CREATE INDEX idx_plugin_dependency_mutation_commits_intent_id
    ON plugin_dependency_mutation_commits(intent_id);

CREATE TRIGGER trg_plugin_dependency_intent_insert_guard
BEFORE INSERT ON plugin_dependency_mutation_intents
WHEN NOT EXISTS (
    SELECT 1
      FROM plugin_projects project
     WHERE project.project_id = NEW.project_id
       AND project.owner_user_id = NEW.owner_user_id
       AND project.managed_source_path IS NOT NULL
       AND project.updated_at = NEW.expected_project_updated_at
       AND project.build_generation = NEW.expected_build_generation
       AND project.source_head_digest = NEW.expected_source_digest
       AND project.dependency_lock_digest = NEW.expected_lock_digest
)
BEGIN
    SELECT RAISE(ABORT, 'plugin dependency intent must bind the exact managed Project head');
END;

CREATE TRIGGER trg_plugin_dependency_commit_insert_guard
BEFORE INSERT ON plugin_dependency_mutation_commits
WHEN NOT EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_intents intent
     WHERE intent.intent_id = NEW.intent_id
       AND intent.project_id = NEW.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'plugin dependency commit marker must bind a durable intent');
END;

CREATE TRIGGER trg_plugin_dependency_project_update_guard
BEFORE UPDATE ON plugin_projects
WHEN EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
AND NOT EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_commits commit_marker
      JOIN plugin_dependency_mutation_intents intent
        ON intent.intent_id = commit_marker.intent_id
       AND intent.project_id = commit_marker.project_id
     WHERE commit_marker.project_id = OLD.project_id
       AND OLD.updated_at = intent.expected_project_updated_at
       AND OLD.build_generation = intent.expected_build_generation
       AND OLD.source_head_digest = intent.expected_source_digest
       AND OLD.dependency_lock_digest = intent.expected_lock_digest
       AND NEW.source_head_digest = intent.next_source_digest
       AND NEW.dependency_lock_digest = intent.next_lock_digest
       AND NEW.build_generation = intent.expected_build_generation + 1
       AND NEW.updated_at > intent.expected_project_updated_at
       AND NEW.id = OLD.id
       AND NEW.project_id = OLD.project_id
       AND NEW.owner_user_id = OLD.owner_user_id
       AND NEW.package_id = OLD.package_id
       AND NEW.managed_source_path IS OLD.managed_source_path
       AND NEW.linked_mount_id IS OLD.linked_mount_id
       AND NEW.ready_candidate_id IS OLD.ready_candidate_id
       AND NEW.created_at = OLD.created_at
       AND NEW.display_name = OLD.display_name
       AND NEW.description = OLD.description
)
BEGIN
    SELECT RAISE(ABORT, 'plugin Project is fenced by a dependency mutation intent');
END;

CREATE TRIGGER trg_plugin_dependency_project_delete_guard
BEFORE DELETE ON plugin_projects
WHEN EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'plugin Project delete is fenced by a dependency mutation intent');
END;

CREATE TRIGGER trg_plugin_dependency_commit_cleanup
AFTER UPDATE ON plugin_projects
WHEN EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_commits commit_marker
     WHERE commit_marker.project_id = NEW.project_id
)
BEGIN
    DELETE FROM plugin_dependency_mutation_commits
     WHERE project_id = NEW.project_id;
END;
