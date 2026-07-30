-- provider_models rows (migration 014) are the authoritative per-model store
-- and every Rust consumer now reads them; the six legacy providers columns
-- (the models JSON array + five per-model JSON maps) were dual-written but no
-- longer read. Drop them. The baseline DDL defines no index, constraint, or
-- generated column over these columns, so SQLite DROP COLUMN is legal here.
-- `capabilities` deliberately stays until the frontend stops sending it.
ALTER TABLE providers DROP COLUMN models;
ALTER TABLE providers DROP COLUMN model_context_limits;
ALTER TABLE providers DROP COLUMN model_protocols;
ALTER TABLE providers DROP COLUMN model_descriptions;
ALTER TABLE providers DROP COLUMN model_enabled;
ALTER TABLE providers DROP COLUMN model_health;
