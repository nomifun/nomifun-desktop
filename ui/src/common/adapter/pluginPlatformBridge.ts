/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  ApplyPluginCandidateRequest,
  ApplyPluginSourceEditRequest,
  BuildPluginProjectRequest,
  CancelPluginOperationRequest,
  ConfigurePluginRequest,
  CreatePluginProjectRequest,
  DeletePluginDataRequest,
  DeletePluginProjectRequest,
  DurablePluginOperationDetail,
  DurablePluginOperationOwner,
  DurablePluginOperationSummary,
  GeneratePluginDraftRequest,
  GeneratedPluginDraft,
  ImportPluginRequest,
  PluginCandidateRef,
  PluginAuthoringContext,
  PluginImportInspection,
  PluginCapabilityContribution,
  PluginDetail,
  PluginLibraryResponse,
  PluginProjectDetail,
  PluginProjectSummary,
  PluginReadyCandidate,
  PluginSummary,
  PluginTargetRef,
  RestorePluginPreviousRequest,
  RetryPluginRequest,
  SetPluginEnabledRequest,
  SetPluginAutoApplyRequest,
  SharePluginRequest,
  TestPluginCandidateRequest,
  UninstallPluginRequest,
  UpdatePluginDependenciesRequest,
} from '../types/pluginPlatform';
import {
  parsePluginArtifactId,
  parsePluginCandidateId,
  parsePluginMountId,
  parsePluginOperationId,
  parsePluginProjectId,
  parsePluginRuntimeId,
} from '../types/ids';
import {
  httpGet,
  httpPost,
  httpPut,
  httpRequest,
  withResponseMap,
} from './httpBridge';

const mapTarget = (target: PluginTargetRef): PluginTargetRef => ({
  ...target,
  artifact_id: parsePluginArtifactId(target.artifact_id),
});

const mapCandidateRef = (candidate: PluginCandidateRef): PluginCandidateRef => ({
  ...candidate,
  candidate_id: parsePluginCandidateId(candidate.candidate_id),
});

const mapSummary = (summary: PluginSummary): PluginSummary => ({
  ...summary,
  mount_id: parsePluginMountId(summary.mount_id),
  current: summary.current ? mapTarget(summary.current) : undefined,
  previous: summary.previous ? mapTarget(summary.previous) : undefined,
  linked_project_id:
    summary.linked_project_id == null
      ? undefined
      : parsePluginProjectId(summary.linked_project_id),
});

const mapProjectSummary = (summary: PluginProjectSummary): PluginProjectSummary => ({
  ...summary,
  project_id: parsePluginProjectId(summary.project_id),
  linked_mount_id:
    summary.linked_mount_id == null ? undefined : parsePluginMountId(summary.linked_mount_id),
  ready_candidate: summary.ready_candidate
    ? mapCandidateRef(summary.ready_candidate)
    : undefined,
});

const mapCapability = (
  capability: PluginCapabilityContribution
): PluginCapabilityContribution => ({
  ...capability,
  provenance: {
    ...capability.provenance,
    mount_id: parsePluginMountId(capability.provenance.mount_id),
    artifact_id: parsePluginArtifactId(capability.provenance.artifact_id),
  },
});

const mapOperationOwner = (
  owner: DurablePluginOperationOwner
): DurablePluginOperationOwner => {
  if (owner.owner === 'plugin_project') {
    return { ...owner, project_id: parsePluginProjectId(owner.project_id) };
  }
  if (owner.owner === 'plugin_mount') {
    return { ...owner, mount_id: parsePluginMountId(owner.mount_id) };
  }
  return owner;
};

const mapOperationSummary = (
  operation: DurablePluginOperationSummary
): DurablePluginOperationSummary => ({
  ...operation,
  operation_id: parsePluginOperationId(operation.operation_id),
  owner: mapOperationOwner(operation.owner),
});

const mapReadyCandidate = (ready: PluginReadyCandidate): PluginReadyCandidate => ({
  ...ready,
  candidate: mapCandidateRef(ready.candidate),
  target: mapTarget(ready.target),
  test: {
    ...ready.test,
    candidate_id: parsePluginCandidateId(ready.test.candidate_id),
  },
});

const mapLibrary = (library: PluginLibraryResponse): PluginLibraryResponse => ({
  ...library,
  plugins: library.plugins.map(mapSummary),
  projects: library.projects.map(mapProjectSummary),
  runtimes: library.runtimes?.map((runtime) => ({
    ...runtime,
    plugin_id: parsePluginRuntimeId(runtime.plugin_id),
  })),
});

const mapDetail = (detail: PluginDetail): PluginDetail => ({
  ...detail,
  summary: mapSummary(detail.summary),
  capabilities: detail.capabilities.map(mapCapability),
});

const mapProjectDetail = (detail: PluginProjectDetail): PluginProjectDetail => ({
  ...detail,
  summary: mapProjectSummary(detail.summary),
  ready: detail.ready ? mapReadyCandidate(detail.ready) : undefined,
  active_operation: detail.active_operation
    ? mapOperationSummary(detail.active_operation)
    : undefined,
});

const mapOperationDetail = (
  detail: DurablePluginOperationDetail
): DurablePluginOperationDetail => ({
  ...detail,
  summary: mapOperationSummary(detail.summary),
});

const deleteWithBody = <Data, Params>(
  path: (params: Params) => string
): {
  provider: () => void;
  invoke: (params: Params) => Promise<Data>;
} => ({
  provider: () => {},
  invoke: (params) => httpRequest<Data>('DELETE', path(params), params),
});

export const plugins = {
  generateDraft: httpPost<GeneratedPluginDraft, GeneratePluginDraftRequest>(
    '/api/plugins/authoring/generate'
  ),
  list: withResponseMap(
    httpGet<PluginLibraryResponse, void>('/api/plugins'),
    mapLibrary
  ),
  getProject: withResponseMap(
    httpGet<PluginProjectDetail, { project_id: string }>(
      (request) => `/api/plugins/projects/${encodeURIComponent(request.project_id)}`
    ),
    mapProjectDetail
  ),
  getAuthoringContext: httpGet<PluginAuthoringContext, { project_id: string }>(
    (request) => `/api/plugins/projects/${encodeURIComponent(request.project_id)}/authoring-context`
  ),
  createProject: withResponseMap(
    httpPost<PluginProjectDetail, CreatePluginProjectRequest>('/api/plugins/projects'),
    mapProjectDetail
  ),
  importPrebuilt: withResponseMap(
    httpPost<PluginProjectDetail, ImportPluginRequest>('/api/plugins/imports'),
    mapProjectDetail
  ),
  inspectImport: httpPost<PluginImportInspection, { source_path: string }>(
    '/api/plugins/imports/inspect'
  ),
  exportShare: withResponseMap(
    httpPost<DurablePluginOperationDetail, SharePluginRequest>(
      (request) => `/api/plugins/projects/${encodeURIComponent(request.project_id)}/share`
    ),
    mapOperationDetail
  ),
  buildProject: withResponseMap(
    httpPost<PluginProjectDetail, BuildPluginProjectRequest>(
      (request) => `/api/plugins/projects/${encodeURIComponent(request.project_id)}/build`
    ),
    mapProjectDetail
  ),
  applySourceEdit: withResponseMap(
    httpPost<PluginProjectDetail, ApplyPluginSourceEditRequest>(
      (request) =>
        `/api/plugins/projects/${encodeURIComponent(request.project_id)}/source/edit`
    ),
    mapProjectDetail
  ),
  updateDependencies: withResponseMap(
    httpPut<PluginProjectDetail, UpdatePluginDependenciesRequest>(
      (request) =>
        `/api/plugins/projects/${encodeURIComponent(request.project_id)}/source/dependencies`
    ),
    mapProjectDetail
  ),
  setAutoApply: withResponseMap(
    httpPut<PluginProjectDetail, SetPluginAutoApplyRequest>(
      (request) =>
        `/api/plugins/projects/${encodeURIComponent(request.project_id)}/auto-apply`
    ),
    mapProjectDetail
  ),
  testCandidate: withResponseMap(
    httpPost<PluginProjectDetail, TestPluginCandidateRequest>(
      (request) => `/api/plugins/projects/${encodeURIComponent(request.project_id)}/test`
    ),
    mapProjectDetail
  ),
  applyCandidate: withResponseMap(
    httpPost<PluginDetail, ApplyPluginCandidateRequest>(
      (request) => `/api/plugins/projects/${encodeURIComponent(request.project_id)}/apply`
    ),
    mapDetail
  ),
  deleteProject: deleteWithBody<boolean, DeletePluginProjectRequest>(
    (request) => `/api/plugins/projects/${encodeURIComponent(request.project_id)}`
  ),
  getMount: withResponseMap(
    httpGet<PluginDetail, { mount_id: string }>(
      (request) => `/api/plugins/installations/${encodeURIComponent(request.mount_id)}`
    ),
    mapDetail
  ),
  configure: withResponseMap(
    httpPut<PluginDetail, ConfigurePluginRequest>(
      (request) => `/api/plugins/installations/${encodeURIComponent(request.mount_id)}/config`
    ),
    mapDetail
  ),
  setEnabled: withResponseMap(
    httpPut<PluginDetail, SetPluginEnabledRequest>(
      (request) => `/api/plugins/installations/${encodeURIComponent(request.mount_id)}/enabled`
    ),
    mapDetail
  ),
  retryMount: withResponseMap(
    httpPost<PluginDetail, RetryPluginRequest>(
      (request) => `/api/plugins/installations/${encodeURIComponent(request.mount_id)}/retry`
    ),
    mapDetail
  ),
  restorePrevious: withResponseMap(
    httpPost<PluginDetail, RestorePluginPreviousRequest>(
      (request) => `/api/plugins/installations/${encodeURIComponent(request.mount_id)}/restore`
    ),
    mapDetail
  ),
  uninstall: withResponseMap(
    httpPost<PluginDetail, UninstallPluginRequest>(
      (request) => `/api/plugins/installations/${encodeURIComponent(request.mount_id)}/uninstall`
    ),
    mapDetail
  ),
  deleteData: deleteWithBody<void, DeletePluginDataRequest>(
    (request) => `/api/plugins/installations/${encodeURIComponent(request.mount_id)}/data`
  ),
  listOperations: withResponseMap(
    httpGet<DurablePluginOperationSummary[], void>('/api/plugins/operations'),
    (operations) => operations.map(mapOperationSummary)
  ),
  getOperation: withResponseMap(
    httpGet<DurablePluginOperationDetail, { operation_id: string }>(
      (request) =>
        `/api/plugins/operations/${encodeURIComponent(request.operation_id)}`
    ),
    mapOperationDetail
  ),
  cancelOperation: withResponseMap(
    httpPost<DurablePluginOperationSummary, CancelPluginOperationRequest>(
      (request) =>
        `/api/plugins/operations/${encodeURIComponent(request.operation_id)}/cancel`,
      (request) => ({
        expected_operation_revision: request.expected_operation_revision,
      })
    ),
    mapOperationSummary
  ),
};
