-- Align built-in ACP agent launch metadata with the current product-owned CLI names.
-- Existing databases created before this migration keep their rows, so update
-- only the known stale command/binary pairs.

UPDATE agent_metadata
SET
  agent_source_info = '{"binary_name":"agent"}',
  command = 'agent',
  args = '["acp"]',
  updated_at = unixepoch('now','subsec') * 1000
WHERE id = 'agent_builtin_cursor'
  AND agent_source = 'builtin'
  AND (
    agent_source_info = '{"binary_name":"cursor"}'
    OR command = 'cursor'
  );

UPDATE agent_metadata
SET
  agent_source_info = '{"binary_name":"kiro-cli"}',
  command = 'kiro-cli',
  args = '["acp"]',
  updated_at = unixepoch('now','subsec') * 1000
WHERE id = 'agent_builtin_kiro'
  AND agent_source = 'builtin'
  AND (
    agent_source_info = '{"binary_name":"kiro-cli-chat"}'
    OR command = 'kiro-cli-chat'
  );
