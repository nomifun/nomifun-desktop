/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type * as Contract from '../types/pluginPlatform';
import { httpGet, httpPost, httpPut, httpRequest } from './httpBridge';

type DraftIdentity = { draft_id: Contract.PluginDraftId };
type PluginIdentity = { plugin_id: Contract.PluginId };
type DraftCommand<Request> = DraftIdentity & { request: Request };
type PluginCommand<Request> = PluginIdentity & { request: Request };

const draftPath = (draftId: Contract.PluginDraftId): string =>
  `/api/plugin-drafts/${encodeURIComponent(draftId)}`;

const pluginPath = (pluginId: Contract.PluginId): string =>
  `/api/plugins/${encodeURIComponent(pluginId)}`;

const requestBody = <Request>({ request }: { request: Request }): Request => request;

const deleteWithBody = <Data, Params>(
  path: (params: Params) => string,
  body: (params: Params) => unknown,
): { provider: () => void; invoke: (params: Params) => Promise<Data> } => ({
  provider: () => {},
  invoke: (params) => httpRequest<Data>('DELETE', path(params), body(params)),
});

const surfaceOwnerPath = (
  request: Pick<
    Contract.DispatchPluginBridgeRequest,
    'plugin_id' | 'draft_id' | 'is_preview'
  >,
  action: 'bridge' | 'close',
): string => {
  if (request.is_preview) {
    if (!request.draft_id) {
      throw new Error('A preview Surface requires draft_id');
    }
    return `${draftPath(request.draft_id)}/surface/${action}`;
  }
  if (!request.plugin_id) {
    throw new Error('An installed Plugin Surface requires plugin_id');
  }
  return `${pluginPath(request.plugin_id)}/surface/${action}`;
};

/** The only frontend entry point for Unified Plugin Core. */
export const pluginPlatform = {
  drafts: {
    list: httpGet<Contract.PluginDraftListResponse, void>('/api/plugin-drafts'),
    create: httpPost<Contract.PluginDraftDetail, Contract.CreatePluginDraftRequest>(
      '/api/plugin-drafts',
    ),
    get: httpGet<Contract.PluginDraftDetail, DraftIdentity>(({ draft_id }) =>
      draftPath(draft_id),
    ),
    generate: httpPost<
      Contract.PluginDraftDetail,
      DraftCommand<Contract.GeneratePluginDraftRequest>
    >(
      ({ draft_id }) => `${draftPath(draft_id)}/generate`,
      requestBody,
    ),
    cancelGeneration: httpPost<
      Contract.PluginDraftDetail,
      DraftCommand<Contract.CancelPluginDraftGenerationRequest>
    >(
      ({ draft_id }) => `${draftPath(draft_id)}/cancel`,
      requestBody,
    ),
    replaceFile: httpPut<
      Contract.PluginDraftDetail,
      DraftCommand<Contract.ReplacePluginDraftFileRequest>
    >(
      ({ draft_id }) => `${draftPath(draft_id)}/files`,
      requestBody,
    ),
    deleteFile: deleteWithBody<
      Contract.PluginDraftDetail,
      DraftCommand<Contract.DeletePluginDraftFileRequest>
    >(
      ({ draft_id }) => `${draftPath(draft_id)}/files`,
      requestBody,
    ),
    preview: httpPost<
      Contract.PluginDraftPreviewResponse,
      DraftCommand<Contract.PreviewPluginDraftRequest>
    >(
      ({ draft_id }) => `${draftPath(draft_id)}/preview`,
      requestBody,
    ),
    save: httpPost<
      Contract.SavePluginDraftResponse,
      DraftCommand<Contract.SavePluginDraftRequest>
    >(
      ({ draft_id }) => `${draftPath(draft_id)}/save`,
      requestBody,
    ),
    delete: deleteWithBody<
      boolean,
      DraftCommand<Contract.DeletePluginDraftRequest>
    >(
      ({ draft_id }) => draftPath(draft_id),
      requestBody,
    ),
  },

  plugins: {
    list: httpGet<Contract.PluginLibraryResponse, void>('/api/plugins'),
    get: httpGet<Contract.PluginDetail, PluginIdentity>(({ plugin_id }) =>
      pluginPath(plugin_id),
    ),
    inspectImport: httpPost<
      Contract.PluginImportInspection,
      Contract.InspectPluginImportRequest
    >('/api/plugins/import/inspect'),
    installImport: httpPost<
      Contract.InstallPluginImportResponse,
      Contract.InstallPluginImportRequest
    >('/api/plugins/import'),
    setEnabled: httpPut<
      Contract.PluginDetail,
      PluginCommand<Contract.SetPluginEnabledRequest>
    >(
      ({ plugin_id }) => `${pluginPath(plugin_id)}/enabled`,
      requestBody,
    ),
    configure: httpPut<
      Contract.PluginDetail,
      PluginCommand<Contract.ConfigurePluginRequest>
    >(
      ({ plugin_id }) => `${pluginPath(plugin_id)}/config`,
      requestBody,
    ),
    restore: httpPost<
      Contract.PluginDetail,
      PluginCommand<Contract.RestorePluginRequest>
    >(
      ({ plugin_id }) => `${pluginPath(plugin_id)}/restore`,
      requestBody,
    ),
    trash: httpPost<
      Contract.PluginDetail,
      PluginCommand<Contract.TrashPluginRequest>
    >(
      ({ plugin_id }) => `${pluginPath(plugin_id)}/trash`,
      requestBody,
    ),
    delete: deleteWithBody<
      Contract.PluginLibraryResponse,
      PluginCommand<Contract.DeletePluginRequest>
    >(
      ({ plugin_id }) => pluginPath(plugin_id),
      requestBody,
    ),
    exportPackage: httpPost<
      Contract.PluginExportResult,
      PluginCommand<Contract.ExportPluginPackageRequest>
    >(
      ({ plugin_id }) => `${pluginPath(plugin_id)}/export`,
      requestBody,
    ),
    exportBackup: httpPost<
      Contract.PluginExportResult,
      PluginCommand<Contract.ExportPluginBackupRequest>
    >(
      ({ plugin_id }) => `${pluginPath(plugin_id)}/backup`,
      requestBody,
    ),
    openSurface: httpPost<
      Contract.PluginSurfaceDescriptor,
      PluginCommand<Contract.OpenPluginSurfaceRequest>
    >(
      ({ plugin_id }) => `${pluginPath(plugin_id)}/surface/open`,
      requestBody,
    ),
  },

  libraryState: {
    get: httpGet<Contract.PluginLibraryState, void>('/api/plugins/library-state'),
    update: httpPut<
      Contract.PluginLibraryState,
      Contract.UpdatePluginLibraryStateRequest
    >('/api/plugins/library-state'),
  },

  credentials: {
    list: httpGet<Contract.PluginCredentialReference[], void>('/api/plugins/credentials'),
  },

  desktop: {
    commands: httpGet<Contract.PluginDesktopCommand[], void>(
      '/api/plugins/desktop/commands',
    ),
    invoke: httpPost<unknown, Contract.InvokePluginDesktopCommandRequest>(
      '/api/plugins/desktop/commands/invoke',
    ),
    emit: httpPost<Contract.PluginDesktopEventReport, Contract.DispatchPluginDesktopEventRequest>(
      '/api/plugins/desktop/events',
    ),
  },

  surface: {
    close: httpPost<boolean, Contract.ClosePluginSurfaceCommand>(
      (command) => surfaceOwnerPath(command, 'close'),
      ({ request }) => request,
    ),
    bridge: httpPost<
      Contract.PluginBridgeResult,
      Contract.DispatchPluginBridgeRequest
    >((request) => surfaceOwnerPath(request, 'bridge')),
  },
};
