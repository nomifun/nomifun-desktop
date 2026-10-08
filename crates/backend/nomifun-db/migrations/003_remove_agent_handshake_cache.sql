-- Retire the unused ACP discovery cache without changing canonical Agent facts
-- or the published checksums of the earlier migrations.
ALTER TABLE agent_metadata DROP COLUMN yolo_id;
ALTER TABLE agent_metadata DROP COLUMN agent_capabilities;
ALTER TABLE agent_metadata DROP COLUMN auth_methods;
ALTER TABLE agent_metadata DROP COLUMN config_options;
ALTER TABLE agent_metadata DROP COLUMN available_modes;
ALTER TABLE agent_metadata DROP COLUMN available_models;
ALTER TABLE agent_metadata DROP COLUMN available_commands;
