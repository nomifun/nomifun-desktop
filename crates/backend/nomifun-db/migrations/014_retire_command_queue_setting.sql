-- Ordinary input delivery is selected per canonical message. This legacy
-- switch has no execution consumer; preserve all other settings and facts.
ALTER TABLE system_settings DROP COLUMN command_queue_enabled;
