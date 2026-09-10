-- Durable MiniApp Project Source mutation intent.
--
-- MiniApp Source revisions are immutable and the Source Store changes only
-- its atomic head.json pointer. This intent binds the one allowed old->new
-- Project generation transition so startup/application recovery can
-- deterministically abort while the filesystem still exposes the old head or
-- finalize after it exposes the prepared new head.

CREATE TABLE miniapp_source_mutation_intents (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    intent_id                 TEXT NOT NULL UNIQUE CHECK (
        length(intent_id) = 36
        AND lower(intent_id) = intent_id
        AND intent_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(intent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id             TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    miniapp_id                TEXT NOT NULL CHECK (
        length(miniapp_id) = 36
        AND lower(miniapp_id) = miniapp_id
        AND miniapp_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(miniapp_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_id                TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    expected_product_revision INTEGER NOT NULL CHECK (expected_product_revision >= 1),
    expected_project_revision INTEGER NOT NULL CHECK (expected_project_revision >= 1),
    expected_build_generation INTEGER NOT NULL CHECK (expected_build_generation >= 1),
    expected_source_digest    TEXT NOT NULL CHECK (
        length(expected_source_digest) = 64
        AND lower(expected_source_digest) = expected_source_digest
        AND expected_source_digest NOT GLOB '*[^0-9a-f]*'
    ),
    next_source_digest        TEXT NOT NULL CHECK (
        length(next_source_digest) = 64
        AND lower(next_source_digest) = next_source_digest
        AND next_source_digest NOT GLOB '*[^0-9a-f]*'
    ),
    next_build_generation     INTEGER NOT NULL CHECK (
        next_build_generation = expected_build_generation + 1
    ),
    created_at                INTEGER NOT NULL CHECK (created_at > 0),
    CHECK (expected_source_digest <> next_source_digest),
    UNIQUE (owner_user_id, miniapp_id, project_id)
);

CREATE INDEX idx_miniapp_source_mutation_intents_owner_user_id
    ON miniapp_source_mutation_intents(owner_user_id);
CREATE INDEX idx_miniapp_source_mutation_intents_miniapp_id
    ON miniapp_source_mutation_intents(miniapp_id);
CREATE INDEX idx_miniapp_source_mutation_intents_project_id
    ON miniapp_source_mutation_intents(project_id);

CREATE TABLE miniapp_source_mutation_commits (
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

CREATE INDEX idx_miniapp_source_mutation_commits_project_id
    ON miniapp_source_mutation_commits(project_id);
CREATE INDEX idx_miniapp_source_mutation_commits_intent_id
    ON miniapp_source_mutation_commits(intent_id);

CREATE TRIGGER trg_miniapp_source_intent_insert_guard
BEFORE INSERT ON miniapp_source_mutation_intents
WHEN NOT EXISTS (
    SELECT 1
      FROM miniapp_products product
      JOIN miniapp_projects project
        ON project.miniapp_id = product.miniapp_id
       AND project.owner_user_id = product.owner_user_id
     WHERE product.owner_user_id = NEW.owner_user_id
       AND product.miniapp_id = NEW.miniapp_id
       AND product.product_revision = NEW.expected_product_revision
       AND product.lifecycle IN ('enabled', 'disabled')
       AND project.project_id = NEW.project_id
       AND project.project_revision = NEW.expected_project_revision
       AND project.source_state = 'editable'
       AND project.build_generation = NEW.expected_build_generation
       AND project.source_head_digest = NEW.expected_source_digest
       AND NOT EXISTS (
           SELECT 1
             FROM product_operations operation
            WHERE operation.owner_kind = 'miniapp'
              AND operation.owner_id = NEW.miniapp_id
              AND operation.kind = 'build'
              AND operation.state = 'running'
       )
)
BEGIN
    SELECT RAISE(ABORT, 'MiniApp Source intent must bind the exact editable Project head');
END;

CREATE TRIGGER trg_miniapp_source_commit_insert_guard
BEFORE INSERT ON miniapp_source_mutation_commits
WHEN NOT EXISTS (
    SELECT 1
      FROM miniapp_source_mutation_intents intent
     WHERE intent.intent_id = NEW.intent_id
       AND intent.project_id = NEW.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'MiniApp Source commit marker must bind a durable intent');
END;

CREATE TRIGGER trg_miniapp_source_project_update_guard
BEFORE UPDATE ON miniapp_projects
WHEN EXISTS (
    SELECT 1
      FROM miniapp_source_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
AND NOT EXISTS (
    SELECT 1
      FROM miniapp_source_mutation_commits commit_marker
      JOIN miniapp_source_mutation_intents intent
        ON intent.intent_id = commit_marker.intent_id
       AND intent.project_id = commit_marker.project_id
     WHERE commit_marker.project_id = OLD.project_id
       AND OLD.owner_user_id = intent.owner_user_id
       AND OLD.miniapp_id = intent.miniapp_id
       AND OLD.project_revision = intent.expected_project_revision
       AND OLD.build_generation = intent.expected_build_generation
       AND OLD.source_head_digest = intent.expected_source_digest
       AND NEW.project_revision = OLD.project_revision + 1
       AND NEW.build_generation = intent.next_build_generation
       AND NEW.source_head_digest = intent.next_source_digest
       AND NEW.updated_at > OLD.updated_at
       AND NEW.id = OLD.id
       AND NEW.project_id = OLD.project_id
       AND NEW.miniapp_id = OLD.miniapp_id
       AND NEW.owner_user_id = OLD.owner_user_id
       AND NEW.source_state = OLD.source_state
       AND NEW.managed_source_path IS OLD.managed_source_path
       AND NEW.dependency_lock_digest IS OLD.dependency_lock_digest
       AND NEW.build_profile_version IS OLD.build_profile_version
       AND NEW.created_at = OLD.created_at
)
BEGIN
    SELECT RAISE(ABORT, 'MiniApp Project is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_miniapp_source_project_delete_guard
BEFORE DELETE ON miniapp_projects
WHEN EXISTS (
    SELECT 1 FROM miniapp_source_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'MiniApp Project delete is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_miniapp_source_product_update_guard
BEFORE UPDATE ON miniapp_products
WHEN EXISTS (
    SELECT 1 FROM miniapp_source_mutation_intents intent
     WHERE intent.miniapp_id = OLD.miniapp_id
)
BEGIN
    SELECT RAISE(ABORT, 'MiniApp Product is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_miniapp_source_product_delete_guard
BEFORE DELETE ON miniapp_products
WHEN EXISTS (
    SELECT 1 FROM miniapp_source_mutation_intents intent
     WHERE intent.miniapp_id = OLD.miniapp_id
)
BEGIN
    SELECT RAISE(ABORT, 'MiniApp delete is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_miniapp_source_build_start_guard
BEFORE INSERT ON product_operations
WHEN NEW.owner_kind = 'miniapp'
AND NEW.kind = 'build'
AND NEW.state = 'running'
AND EXISTS (
    SELECT 1 FROM miniapp_source_mutation_intents intent
     WHERE intent.miniapp_id = NEW.owner_id
)
BEGIN
    SELECT RAISE(ABORT, 'MiniApp Build is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_miniapp_source_commit_cleanup
AFTER UPDATE ON miniapp_projects
WHEN EXISTS (
    SELECT 1 FROM miniapp_source_mutation_commits commit_marker
     WHERE commit_marker.project_id = NEW.project_id
)
BEGIN
    DELETE FROM miniapp_source_mutation_commits
     WHERE project_id = NEW.project_id;
END;
