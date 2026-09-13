import { httpGet, httpPost, withResponseMap } from './httpBridge';
import { parsePluginRuntimeId } from '../types/ids';
import type { PluginRuntimeWorkshop } from '../types/pluginRuntimePlatform';

export interface PluginRuntimeCollection {
  id: string;
  name: string;
}
export interface PluginRuntimeLibraryItem {
  collection_id: string | null;
  pinned: boolean;
  last_opened: number;
  name: string | null;
}
export interface PluginRuntimeWorkspace {
  revision: number;
  collections: PluginRuntimeCollection[];
  items: Record<string, PluginRuntimeLibraryItem>;
}
export interface PluginRuntimeDraft {
  id: string;
  revision: number;
  name: string;
  description: string;
  html: string;
  service_source: string | null;
  source_manifest?: { actions?: Array<{ id: string; name: string; description: string }> } | null;
  messages: Array<{ role: 'user' | 'assistant'; content: string }>;
  status:
    | 'generating'
    | 'ready'
    | 'saved'
    | 'stopped'
    | 'failed'
    | 'interrupted'
    | 'saving';
  error: string | null;
  plugin_id: string | null;
  base_release_digest: string | null;
  updated_at: number;
  import: null | {
    kind: 'share' | 'artifact' | 'backup';
    editable: boolean;
    includes_data: boolean;
    requires_service: boolean;
  };
}
export interface PluginRuntimeGenerateRequest {
  provider_id: string;
  model: string;
  requirement: string;
  draft_id?: string;
  expected_revision?: number;
  plugin_id?: string;
}
const draftPath = (id: string) =>
  `/api/plugins/drafts/${encodeURIComponent(id)}`;
type DraftCommand = { id: string; expected_revision: number };
export const pluginRuntimeProduct = {
  workspace: httpGet<PluginRuntimeWorkspace, void>('/api/plugins/workspace'),
  updateWorkspace: httpPost<PluginRuntimeWorkspace, PluginRuntimeWorkspace>(
    '/api/plugins/workspace',
  ),
  drafts: httpGet<PluginRuntimeDraft[], void>('/api/plugins/drafts'),
  draft: httpGet<PluginRuntimeDraft, { id: string }>(({ id }) => draftPath(id)),
  generate: httpPost<PluginRuntimeDraft, PluginRuntimeGenerateRequest>(
    '/api/plugins/authoring',
  ),
  cancel: httpPost<PluginRuntimeDraft, DraftCommand>(
    ({ id }) => `${draftPath(id)}/cancel`,
    ({ expected_revision }) => ({ expected_revision }),
  ),
  discard: httpPost<boolean, DraftCommand>(
    ({ id }) => `${draftPath(id)}/discard`,
    ({ expected_revision }) => ({ expected_revision }),
  ),
  save: withResponseMap(
    httpPost<PluginRuntimeWorkshop, DraftCommand>(
      ({ id }) => `${draftPath(id)}/save`,
      ({ expected_revision }) => ({ expected_revision }),
    ),
    (value) => ({
      ...value,
      plugin: {
        ...value.plugin,
        plugin_id: parsePluginRuntimeId(value.plugin.plugin_id),
      },
    }),
  ),
  inspect: httpPost<
    PluginRuntimeDraft,
    { source_path?: string; filename?: string; content?: string }
  >('/api/plugins/runtimes/import/inspect'),
  exportFile: httpPost<
    { exported: boolean; resumed: boolean },
    { plugin_id: string; destination_path: string; backup?: boolean }
  >(
    ({ plugin_id }) =>
      `/api/plugins/runtimes/${encodeURIComponent(plugin_id)}/export-file`,
    ({ destination_path, backup }) => ({
      destination_path,
      backup: backup ?? false,
    }),
  ),
};
