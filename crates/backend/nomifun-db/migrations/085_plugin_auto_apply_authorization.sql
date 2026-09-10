-- User-owned standing authorization for compatible Plugin Candidate Apply.
--
-- The Project row is the single durable owner: authorization always binds the
-- exact linked Mount, carries a monotonic revision, and is absent by shape in
-- ask-before-apply mode. Runtime code still recomputes every candidate/current
-- eligibility predicate immediately before the atomic Apply transaction.

ALTER TABLE plugin_projects ADD COLUMN apply_mode TEXT NOT NULL
    DEFAULT 'ask_before_apply'
    CHECK (apply_mode IN ('ask_before_apply', 'auto_compatible_when_idle'));

ALTER TABLE plugin_projects ADD COLUMN auto_apply_mount_id TEXT CHECK (
    auto_apply_mount_id IS NULL OR (
        length(auto_apply_mount_id) = 36
        AND lower(auto_apply_mount_id) = auto_apply_mount_id
        AND auto_apply_mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(auto_apply_mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
);

ALTER TABLE plugin_projects ADD COLUMN auto_apply_authorization_revision INTEGER NOT NULL
    DEFAULT 0 CHECK (auto_apply_authorization_revision >= 0);

ALTER TABLE plugin_projects ADD COLUMN auto_apply_authorized_at INTEGER CHECK (
    auto_apply_authorized_at IS NULL OR auto_apply_authorized_at > 0
);

CREATE INDEX idx_plugin_projects_auto_apply_mount_id
    ON plugin_projects(auto_apply_mount_id);

ALTER TABLE plugin_mount_revisions ADD COLUMN apply_authorization_kind TEXT NOT NULL
    DEFAULT 'manual_user_confirmation'
    CHECK (apply_authorization_kind IN ('manual_user_confirmation', 'standing_auto'));

ALTER TABLE plugin_mount_revisions ADD COLUMN auto_apply_authorization_revision INTEGER CHECK (
    auto_apply_authorization_revision IS NULL OR auto_apply_authorization_revision > 0
);

CREATE TRIGGER trg_plugin_project_auto_apply_insert_guard
BEFORE INSERT ON plugin_projects
WHEN NOT (
    NEW.apply_mode = 'ask_before_apply'
    AND NEW.auto_apply_mount_id IS NULL
    AND NEW.auto_apply_authorization_revision = 0
    AND NEW.auto_apply_authorized_at IS NULL
)
BEGIN
    SELECT RAISE(ABORT, 'plugin Project must begin without standing auto Apply authorization');
END;

CREATE TRIGGER trg_plugin_project_auto_apply_update_guard
BEFORE UPDATE OF
    apply_mode,
    auto_apply_mount_id,
    auto_apply_authorization_revision,
    auto_apply_authorized_at,
    linked_mount_id,
    managed_source_path,
    source_head_digest,
    dependency_lock_digest
ON plugin_projects
WHEN NOT (
    (
        NEW.apply_mode = 'ask_before_apply'
        AND NEW.auto_apply_mount_id IS NULL
        AND NEW.auto_apply_authorized_at IS NULL
    )
    OR (
        NEW.apply_mode = 'auto_compatible_when_idle'
        AND NEW.auto_apply_mount_id IS NOT NULL
        AND NEW.auto_apply_mount_id = NEW.linked_mount_id
        AND NEW.auto_apply_authorization_revision > 0
        AND NEW.auto_apply_authorized_at IS NOT NULL
        AND NEW.managed_source_path IS NOT NULL
        AND NEW.source_head_digest IS NOT NULL
        AND NEW.dependency_lock_digest IS NOT NULL
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin auto Apply authorization has an invalid Project shape');
END;

CREATE TRIGGER trg_plugin_project_auto_apply_revision_guard
BEFORE UPDATE OF
    apply_mode,
    auto_apply_mount_id,
    auto_apply_authorization_revision,
    auto_apply_authorized_at
ON plugin_projects
WHEN NEW.auto_apply_authorization_revision <> OLD.auto_apply_authorization_revision + 1
BEGIN
    SELECT RAISE(ABORT, 'plugin auto Apply authorization revision must advance exactly once');
END;

CREATE TRIGGER trg_plugin_auto_apply_dependency_fence
BEFORE UPDATE OF
    apply_mode,
    auto_apply_mount_id,
    auto_apply_authorization_revision,
    auto_apply_authorized_at
ON plugin_projects
WHEN EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'plugin auto Apply authorization is fenced by a dependency mutation');
END;

CREATE TRIGGER trg_plugin_mount_revision_authorization_guard
BEFORE INSERT ON plugin_mount_revisions
WHEN NOT (
    (
        NEW.apply_authorization_kind = 'manual_user_confirmation'
        AND NEW.auto_apply_authorization_revision IS NULL
    )
    OR (
        NEW.apply_authorization_kind = 'standing_auto'
        AND NEW.auto_apply_authorization_revision IS NOT NULL
        AND EXISTS (
            SELECT 1
              FROM plugin_ready_candidates candidate
              JOIN plugin_projects project
                ON project.project_id = candidate.project_id
             WHERE candidate.candidate_id = NEW.candidate_key
               AND project.apply_mode = 'auto_compatible_when_idle'
               AND project.auto_apply_mount_id = NEW.mount_id
               AND project.auto_apply_authorization_revision =
                   NEW.auto_apply_authorization_revision
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin Mount revision requires an exact Apply authorization');
END;
