import type { AppDevelopmentSessionRequest, AppEnvironmentReport, AppImportKind, AppMcpSource, AppOperation, AppProjectSessionPage, AppUpdateRequest, AppUpdateResponse, HostedApp } from '../types/appHosting';
import { httpDelete, httpGet, httpPost, httpPut, httpRequest, withResponseMap } from './httpBridge';
import { fromApiConversation } from './apiModelMapper';

const pathOf = (id: string) => `/api/apps/${encodeURIComponent(id)}`;

/** Upload ordinary ZIP bytes through the same authenticated Host connection. */
async function uploadZip(path: string, archive: File, query: Record<string, string> = {}): Promise<HostedApp> {
  const requestPath = `${path}?${new URLSearchParams(query)}`;
  return httpRequest<HostedApp>('POST', requestPath, undefined, { rawBody: archive, rawContentType: 'application/zip' });
}

export const appHosting = {
  list: httpGet<HostedApp[], void>('/api/apps'),
  create: httpPost<HostedApp, { name?: string; description?: string; tags?: string[] }>('/api/apps'),
  developmentSession: withResponseMap(
    httpPost<{ app: HostedApp; conversation: unknown }, AppDevelopmentSessionRequest>((p) => `${pathOf(p.id)}/development-session`, ({ id: _id, ...body }) => body),
    (response) => ({ app: response.app, conversation: fromApiConversation(response.conversation) }),
  ),
  developmentSessions: withResponseMap(
    httpGet<Omit<AppProjectSessionPage, 'items'> & { items: Array<{ conversation: unknown; editable_in_app: boolean; archived: boolean }> }, { id: string; cursor?: string; limit?: number }>(p => {
      const query = new URLSearchParams();
      if (p.cursor) query.set('cursor', p.cursor);
      if (p.limit) query.set('limit', String(p.limit));
      return `${pathOf(p.id)}/development-sessions${query.size ? `?${query}` : ''}`;
    }),
    (page): AppProjectSessionPage => ({ ...page, items: page.items.map(item => ({ ...item, conversation: fromApiConversation(item.conversation) })) }),
  ),
  get: httpGet<HostedApp, { id: string }>((p) => pathOf(p.id)),
  import: httpPost<HostedApp, { source: string; name?: string; import_kind: AppImportKind }>('/api/apps/import'),
  importInto: httpPost<HostedApp, { id: string; source: string; name?: string; import_kind: AppImportKind }>((p) => `${pathOf(p.id)}/import`, ({ id: _id, ...body }) => body),
  importIntoZip: (id: string, archive: File, import_kind: AppImportKind, name: string) => uploadZip(`${pathOf(id)}/import-zip`, archive, {
    import_kind, name: name.trim() || archive.name.replace(/\.zip$/i, ''),
  }),
  importZip: (archive: File, import_kind: AppImportKind, name: string) => uploadZip('/api/apps/import-zip', archive, {
    import_kind, name: name.trim() || archive.name.replace(/\.zip$/i, ''),
  }),
  update: httpPut<AppUpdateResponse, AppUpdateRequest>((p) => pathOf(p.id), ({ id: _id, ...body }) => body),
  operate: httpPost<HostedApp, { id: string; operation: AppOperation }>((p) => `${pathOf(p.id)}/operate`, (p) => ({ operation: p.operation })),
  overwrite: httpPost<HostedApp, { id: string; source: string }>((p) => `${pathOf(p.id)}/overwrite`, (p) => ({ source: p.source })),
  overwriteZip: (id: string, archive: File) => uploadZip(`${pathOf(id)}/overwrite-zip`, archive),
  delete: httpDelete<void, { id: string }>((p) => pathOf(p.id)),
  environment: httpPost<AppEnvironmentReport, { node_path?: string | null }>('/api/apps/environment'),
  mcpSources: httpGet<AppMcpSource[], void>('/api/app-hosting/mcp-sources'),
  refreshMcpSource: httpPost<AppMcpSource, { id: string }>((p) => `/api/app-hosting/mcp-sources/${encodeURIComponent(p.id)}/refresh`, () => ({})),
};
