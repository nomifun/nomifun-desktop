-- Most users have Node/npm available but do not have bun installed.
-- Built-in ACP adapter bridge rows must therefore use npm directly.

UPDATE agent_metadata
SET
  agent_source_info = '{"binary_name":"claude","bridge_binary":"npm"}',
  command = 'npm',
  args = '["exec","--yes","--package","@agentclientprotocol/claude-agent-acp@0.33.1","--","claude-agent-acp"]',
  updated_at = unixepoch('now','subsec') * 1000
WHERE id = 'agent_builtin_claude'
  AND agent_source = 'builtin'
  AND command = 'bun';

UPDATE agent_metadata
SET
  agent_source_info = '{"binary_name":"codex","bridge_binary":"npm"}',
  command = 'npm',
  args = '["exec","--yes","--package","@zed-industries/codex-acp@0.14.0","--","codex-acp"]',
  updated_at = unixepoch('now','subsec') * 1000
WHERE id = 'agent_builtin_codex'
  AND agent_source = 'builtin'
  AND command = 'bun';

UPDATE agent_metadata
SET
  agent_source_info = '{"binary_name":"codebuddy","bridge_binary":"npm"}',
  command = 'npm',
  args = '["exec","--yes","--package","@tencent-ai/codebuddy-code@2.97.0","--","codebuddy","--acp"]',
  updated_at = unixepoch('now','subsec') * 1000
WHERE id = 'agent_builtin_codebuddy'
  AND agent_source = 'builtin'
  AND command = 'bun';
