-- Swap the builtin Codex ACP bridge from the DEPRECATED, archived
-- @zed-industries/codex-acp (0.14.0, native ~60MB platform binary, unfixed
-- startup-hang bugs, repo archived 2026-07-22) to the sanctioned replacement
-- @agentclientprotocol/codex-acp (joint Zed+JetBrains bridge over the blessed
-- `codex app-server`; 1.1MB JS package whose @openai/codex dependency ships
-- the codex binary and honours CODEX_HOME, so the managed-home sandbox/auth
-- mirroring keeps working unchanged).
--
-- Verified live against 1.1.7 (2026-07-31, stdio probe):
--   initialize ~0.5s warm; loadSession=true; session/new returns models
--   (currentModelId from the local default config) + modes + configOptions;
--   session/set_model works; stdio MCP entries accepted.
--
-- The new bridge renames the session modes:
--   read-only  -> read-only        (unchanged)
--   auto       -> agent
--   full-access-> agent-full-access
-- so yolo_id must follow; runtime normalization of persisted legacy ids
-- lives in mode_normalize.rs.
--
-- Cached handshake columns are cleared: they hold the OLD bridge's advertised
-- modes/models/capabilities, which are wrong for the new bridge and would be
-- served to the UI until the first new session overwrites them.
UPDATE agent_metadata
SET args = '["x","--bun","@agentclientprotocol/codex-acp@1.1.7"]',
    yolo_id = 'agent-full-access',
    agent_capabilities = NULL,
    auth_methods = NULL,
    config_options = NULL,
    available_modes = NULL,
    available_models = NULL,
    available_commands = NULL,
    updated_at = unixepoch('now','subsec')*1000
WHERE agent_id = '0190f5fe-7c00-7a00-8000-000000000102'
  AND agent_source = 'builtin'
  AND backend = 'codex';
