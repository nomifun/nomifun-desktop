-- Persist user-facing Plugin Project metadata independently from package identity.
--
-- Migration 067 intentionally started with package identity only. Existing
-- projects are backfilled from package_id; every new repository write supplies
-- the explicit display metadata captured by the source or imported manifest.

ALTER TABLE plugin_projects ADD COLUMN display_name TEXT NOT NULL
    DEFAULT 'Plugin Runtime Project'
    CHECK (
        length(display_name) BETWEEN 1 AND 255
        AND trim(display_name) <> ''
        AND instr(display_name, char(0)) = 0
    );

ALTER TABLE plugin_projects ADD COLUMN description TEXT NOT NULL
    DEFAULT ''
    CHECK (
        length(description) <= 4096
        AND instr(description, char(0)) = 0
    );

UPDATE plugin_projects
SET display_name = package_id
WHERE package_id IS NOT NULL
  AND display_name = 'Plugin Runtime Project';

CREATE TRIGGER trg_plugin_project_metadata_insert_guard
BEFORE INSERT ON plugin_projects
WHEN NEW.plugin_product_id IS NULL
 AND NEW.display_name = 'Plugin Runtime Project'
BEGIN
    SELECT RAISE(ABORT, 'plugin project display metadata must be explicit');
END;

CREATE TRIGGER trg_plugin_project_metadata_update_guard
BEFORE UPDATE OF display_name, description ON plugin_projects
WHEN NEW.plugin_product_id IS NULL
 AND NEW.display_name = 'Plugin Runtime Project'
BEGIN
    SELECT RAISE(ABORT, 'plugin project display metadata must be explicit');
END;
