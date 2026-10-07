import type { McpServerId } from '@/common/types/ids';

/** Namespace canonical MCP server IDs away from other UI state keys. */
export const mcpServerUiKey = (id: McpServerId): `server:${string}` => `server:${id}`;
