import { httpGet, httpPost, withResponseMap } from './httpBridge';
import { parseMiniAppId } from '../types/ids';
import type { MiniAppWorkshop } from '../types/miniAppPlatform';

export interface MiniAppCollection {
  id: string;
  name: string;
}
export interface MiniAppLibraryItem {
  collection_id: string | null;
  pinned: boolean;
  last_opened: number;
  name: string | null;
}
export interface MiniAppWorkspace {
  revision: number;
  collections: MiniAppCollection[];
  items: Record<string, MiniAppLibraryItem>;
}
export interface MiniAppDraft {
  id: string;
  revision: number;
  name: string;
  description: string;
  html: string;
  service_source: string | null;
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
  miniapp_id: string | null;
  base_release_digest: string | null;
  updated_at: number;
  import: null | {
    kind: 'share' | 'artifact' | 'backup';
    editable: boolean;
    includes_data: boolean;
    requires_service: boolean;
  };
}
export interface MiniAppGenerateRequest {
  provider_id: string;
  model: string;
  requirement: string;
  draft_id?: string;
  expected_revision?: number;
  miniapp_id?: string;
}
const draftPath = (id: string) =>
  `/api/miniapps/drafts/${encodeURIComponent(id)}`;
type DraftCommand = { id: string; expected_revision: number };
export const miniAppProduct = {
  workspace: httpGet<MiniAppWorkspace, void>('/api/miniapps/workspace'),
  updateWorkspace: httpPost<MiniAppWorkspace, MiniAppWorkspace>(
    '/api/miniapps/workspace',
  ),
  drafts: httpGet<MiniAppDraft[], void>('/api/miniapps/drafts'),
  draft: httpGet<MiniAppDraft, { id: string }>(({ id }) => draftPath(id)),
  generate: httpPost<MiniAppDraft, MiniAppGenerateRequest>(
    '/api/miniapps/authoring',
  ),
  cancel: httpPost<MiniAppDraft, DraftCommand>(
    ({ id }) => `${draftPath(id)}/cancel`,
    ({ expected_revision }) => ({ expected_revision }),
  ),
  discard: httpPost<boolean, DraftCommand>(
    ({ id }) => `${draftPath(id)}/discard`,
    ({ expected_revision }) => ({ expected_revision }),
  ),
  save: withResponseMap(
    httpPost<MiniAppWorkshop, DraftCommand>(
      ({ id }) => `${draftPath(id)}/save`,
      ({ expected_revision }) => ({ expected_revision }),
    ),
    (value) => ({
      ...value,
      miniapp: {
        ...value.miniapp,
        miniapp_id: parseMiniAppId(value.miniapp.miniapp_id),
      },
    }),
  ),
  inspect: httpPost<
    MiniAppDraft,
    { source_path?: string; filename?: string; content?: string }
  >('/api/miniapps/import/inspect'),
  exportFile: httpPost<
    { exported: boolean; resumed: boolean },
    { miniapp_id: string; destination_path: string; backup?: boolean }
  >(
    ({ miniapp_id }) =>
      `/api/miniapps/${encodeURIComponent(miniapp_id)}/export-file`,
    ({ destination_path, backup }) => ({
      destination_path,
      backup: backup ?? false,
    }),
  ),
};
