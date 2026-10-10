import { mcpService } from '@/common/adapter/ipcBridge';
import type { IMcpServer, IMcpServerTransport } from '@/common/config/storage';

type BackendMcpTransport = Exclude<IMcpServerTransport, { type: 'streamable_http' }>;

const catalogChangedListeners = new Set<() => void>();

/** Invalidate renderer views after a saved MCP mutation or a completed probe. */
export const notifyMcpCatalogChanged = () => {
  for (const listener of catalogChangedListeners) listener();
};

/** Notifications carry no projected data: subscribers reload the backend catalog. */
export const subscribeMcpCatalogChanged = (listener: () => void): (() => void) => {
  catalogChangedListeners.add(listener);
  return () => { catalogChangedListeners.delete(listener); };
};

type BackendMcpPayload = {
  name: string;
  description?: string;
  transport: BackendMcpTransport;
  original_json: string;
  builtin?: boolean;
};

const isBuiltinServer = (server: IMcpServer) => server.builtin === true;

const normalizeServerName = (name: string) => name.trim().toLowerCase();

const getCatalogServerKey = (server: Pick<IMcpServer, 'mcp_server_id' | 'name' | 'builtin'>) => {
  const normalizedName = normalizeServerName(server.name);
  if (server.builtin === true) {
    return `builtin:${normalizedName || server.mcp_server_id}`;
  }
  return `user:${normalizedName || server.mcp_server_id}`;
};

const dedupeServers = (servers: IMcpServer[]) => {
  const seen = new Set<string>();
  const deduped: IMcpServer[] = [];

  for (const server of servers) {
    const key = getCatalogServerKey(server);
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    deduped.push(server);
  }

  return deduped;
};

const normalizeTransportForBackend = (transport: IMcpServerTransport): BackendMcpTransport => {
  if (transport.type === 'streamable_http') {
    return {
      type: 'http',
      url: transport.url,
      headers: transport.headers,
    };
  }
  return transport;
};

export const toBackendMcpPayload = (
  server: Pick<IMcpServer, 'name' | 'description' | 'transport' | 'original_json' | 'builtin'>
): BackendMcpPayload => ({
  name: server.name,
  description: server.description,
  transport: normalizeTransportForBackend(server.transport),
  original_json: server.original_json || '{}',
  builtin: Boolean(server.builtin),
});

export const ensureBackendMcpCatalog = async (): Promise<{
  userServers: IMcpServer[];
  builtinServers: IMcpServer[];
  allServers: IMcpServer[];
}> => {
  const allServers = dedupeServers(await mcpService.listServers.invoke());
  const builtinServers = allServers.filter(isBuiltinServer);
  const userServers = allServers.filter((server) => !isBuiltinServer(server));

  return {
    userServers,
    builtinServers,
    allServers,
  };
};
