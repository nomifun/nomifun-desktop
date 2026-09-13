/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import {
  InvalidEntityIdError,
  parsePluginArtifactId,
  parsePluginCandidateId,
  parsePluginMountId,
  parsePluginOperationId,
  parsePluginProjectId,
} from '../types/ids';
import type {
  ConfigurePluginRequest,
  DeletePluginDataRequest,
  DeletePluginProjectRequest,
  PluginDetail,
  PluginLibraryResponse,
  PluginProjectDetail,
  ImportPluginRequest,
  SetPluginAutoApplyRequest,
  SharePluginRequest,
  UpdatePluginDependenciesRequest,
} from '../types/pluginPlatform';
import { plugins } from './pluginPlatformBridge';

const PROJECT_ID = parsePluginProjectId('0190f5fe-7c00-7a00-8000-000000000001');
const MOUNT_ID = parsePluginMountId('0190f5fe-7c00-7a00-8000-000000000002');
const ARTIFACT_ID = parsePluginArtifactId('0190f5fe-7c00-7a00-8000-000000000003');
const CANDIDATE_ID = parsePluginCandidateId('0190f5fe-7c00-7a00-8000-000000000004');
const OPERATION_ID = parsePluginOperationId('0190f5fe-7c00-7a00-8000-000000000005');
const realFetch = globalThis.fetch;

const target = {
  package_id: 'com.nomifun.example',
  package_version: '1.0.0',
  artifact_id: ARTIFACT_ID,
  artifact_digest: 'a'.repeat(64),
  manifest_digest: 'b'.repeat(64),
};

const summary = {
  mount_id: MOUNT_ID,
  mount_revision: 7,
  display_name: 'Example Plugin',
  description: 'Fixture',
  lifecycle: 'enabled' as const,
  current: target,
  previous: { ...target, artifact_digest: 'c'.repeat(64) },
  linked_project_id: PROJECT_ID,
  contribution_count: 1,
  updated_at_ms: 1_788_716_800_000,
};

const projectSummary = {
  project_id: PROJECT_ID,
  project_revision: 5,
  display_name: 'Example Plugin',
  linked_mount_id: MOUNT_ID,
  source_state: 'editable' as const,
  build_generation: 3,
  ready_candidate: {
    candidate_id: CANDIDATE_ID,
    candidate_digest: 'd'.repeat(64),
  },
  apply_mode: 'ask_before_apply' as const,
  auto_apply_authorization_revision: 0,
  updated_at_ms: 1_788_716_800_000,
};

const library: PluginLibraryResponse = {
  library_revision: 12,
  plugins: [summary],
  projects: [projectSummary],
};

const detail: PluginDetail = {
  summary,
  capabilities: [],
  config_schema: { schema_digest: 'e'.repeat(64), schema: {} },
  config: {
    config_revision: 2,
    schema_digest: 'e'.repeat(64),
    values: {},
    valid: true,
    validation_errors: [],
  },
  credential_bindings_revision: 4,
  credential_slots: [],
  retained_data: false,
};

const projectDetail: PluginProjectDetail = {
  summary: projectSummary,
  source_snapshot_digest: 'f'.repeat(64),
  dependency_lock_digest: '1'.repeat(64),
  direct_dependencies: {},
};

const exportOperation = {
  summary: {
    operation_id: OPERATION_ID,
    operation_revision: 2,
    kind: 'export' as const,
    owner: { owner: 'plugin_project' as const, project_id: PROJECT_ID },
    state: 'succeeded' as const,
    cancelable: false,
    progress_percent: 100,
    started_at_ms: 1,
    completed_at_ms: 2,
  },
  bounded_log_tail: ['Plugin Share Bundle exported'],
  result_artifact_digests: { share_bundle: '9'.repeat(64) },
};

const generatedDraft = {
  assistant_message: 'Created',
  display_name: 'Summary enhancer',
  description: 'Adds summaries to NomiFun.',
  package_id: 'user.nomifun.summary',
  package_version: '0.1.0',
  language: 'type_script' as const,
  manifest_content: '{}',
  source_path: 'src/main.ts' as const,
  source_content: 'export async function activate() {}',
  dependencies: {},
  capabilities: [],
};

const authoringContext = {
  package_id: generatedDraft.package_id,
  package_version: generatedDraft.package_version,
  display_name: generatedDraft.display_name,
  description: generatedDraft.description,
  source_path: generatedDraft.source_path,
  source_content: generatedDraft.source_content,
};

const importInspection = {
  import_kind: 'prebuilt_artifact' as const,
  expected_digest: '9'.repeat(64),
  package_id: 'example.imported',
  package_version: '1.0.0',
  display_name: 'Imported Plugin',
  description: 'Imported fixture',
  capability_count: 2,
  editable_source: false,
};

type RecordedCall = {
  method: string;
  path: string;
  body: unknown;
};

const calls: RecordedCall[] = [];

function installFetchFixture(): void {
  globalThis.fetch = (async (input, init) => {
    const path = new URL(String(input), 'http://127.0.0.1').pathname;
    const method = init?.method ?? 'GET';
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, path, body });

    const data = path === '/api/plugins/authoring/generate'
      ? generatedDraft
      : path.endsWith('/authoring-context')
        ? authoringContext
        : path === '/api/plugins/imports/inspect'
          ? importInspection
          : path.endsWith('/share')
      ? exportOperation
      : path === '/api/plugins'
        ? library
        : path === '/api/plugins/imports' || path.includes('/api/plugins/projects/')
        ? projectDetail
        : path.endsWith('/data')
          ? undefined
          : detail;
    return new Response(JSON.stringify({ success: true, data }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
}

afterEach(() => {
  globalThis.fetch = realFetch;
  calls.length = 0;
});

describe('Plugin Platform bridge', () => {
  test('wires AI authoring, resumable Source context, and automatic import inspection', async () => {
    installFetchFixture();
    const request = {
      provider_id: 'provider',
      model: 'model',
      requirement: 'Summarize web pages',
      package_id: generatedDraft.package_id,
      package_version: generatedDraft.package_version,
    };

    expect(await plugins.generateDraft.invoke(request)).toEqual(generatedDraft);
    expect(await plugins.getAuthoringContext.invoke({ project_id: PROJECT_ID })).toEqual(authoringContext);
    expect(await plugins.inspectImport.invoke({ source_path: 'C:\\imports\\plugin.zip' })).toEqual(importInspection);
    expect(calls.map(({ method, path }) => ({ method, path }))).toEqual([
      { method: 'POST', path: '/api/plugins/authoring/generate' },
      { method: 'GET', path: `/api/plugins/projects/${PROJECT_ID}/authoring-context` },
      { method: 'POST', path: '/api/plugins/imports/inspect' },
    ]);
  });

  test('maps collection and detail identities at the HTTP boundary', async () => {
    installFetchFixture();

    const inventory = await plugins.list.invoke();
    const project = await plugins.getProject.invoke({ project_id: PROJECT_ID });
    const mount = await plugins.getMount.invoke({ mount_id: MOUNT_ID });

    expect(inventory.plugins[0]?.mount_id).toBe(MOUNT_ID);
    expect(inventory.projects[0]?.project_id).toBe(PROJECT_ID);
    expect(project.summary.linked_mount_id).toBe(MOUNT_ID);
    expect(mount.summary.current?.artifact_id).toBe(ARTIFACT_ID);
    expect(calls.map(({ method, path }) => ({ method, path }))).toEqual([
      { method: 'GET', path: '/api/plugins' },
      { method: 'GET', path: `/api/plugins/projects/${PROJECT_ID}` },
      { method: 'GET', path: `/api/plugins/installations/${MOUNT_ID}` },
    ]);
  });

  test('sends exact lifecycle CAS bodies to the mounted routes', async () => {
    installFetchFixture();

    await plugins.setEnabled.invoke({
      mount_id: MOUNT_ID as PluginDetail['summary']['mount_id'],
      expected_mount_revision: 7,
      expected_current_target_digest: target.artifact_digest,
      enabled: false,
    });
    await plugins.retryMount.invoke({
      mount_id: MOUNT_ID as PluginDetail['summary']['mount_id'],
      expected_mount_revision: 7,
      expected_current_target_digest: target.artifact_digest,
    });
    await plugins.restorePrevious.invoke({
      mount_id: MOUNT_ID as PluginDetail['summary']['mount_id'],
      expected_mount_revision: 7,
      expected_current_target_digest: target.artifact_digest,
      expected_previous_target_digest: 'c'.repeat(64),
    });
    await plugins.uninstall.invoke({
      mount_id: MOUNT_ID as PluginDetail['summary']['mount_id'],
      expected_mount_revision: 7,
      expected_current_target_digest: target.artifact_digest,
    });

    expect(calls.map(({ method, path }) => ({ method, path }))).toEqual([
      { method: 'PUT', path: `/api/plugins/installations/${MOUNT_ID}/enabled` },
      { method: 'POST', path: `/api/plugins/installations/${MOUNT_ID}/retry` },
      { method: 'POST', path: `/api/plugins/installations/${MOUNT_ID}/restore` },
      { method: 'POST', path: `/api/plugins/installations/${MOUNT_ID}/uninstall` },
    ]);
    expect(calls[0]?.body).toEqual({
      mount_id: MOUNT_ID,
      expected_mount_revision: 7,
      expected_current_target_digest: target.artifact_digest,
      enabled: false,
    });
  });

  test('sends the complete config, schema, and credential CAS body', async () => {
    installFetchFixture();
    const request: ConfigurePluginRequest = {
      mount_id: MOUNT_ID,
      expected_mount_revision: 7,
      expected_current_target_digest: target.artifact_digest,
      expected_config_revision: 2,
      expected_schema_digest: 'e'.repeat(64),
      values: {
        endpoint: 'https://api.example.test',
        retries: 3,
      },
      credential_bindings: {
        api_key: 'credential://stepfun-coding-plan',
        optional_token: null,
      },
      expected_credential_bindings_revision: 4,
    };

    await plugins.configure.invoke(request);

    expect(calls).toEqual([
      {
        method: 'PUT',
        path: `/api/plugins/installations/${MOUNT_ID}/config`,
        body: request,
      },
    ]);
  });

  test('keeps the delete-data CAS request in the DELETE body', async () => {
    installFetchFixture();
    const request: DeletePluginDataRequest = {
      mount_id: MOUNT_ID as DeletePluginDataRequest['mount_id'],
      expected_mount_revision: 8,
      expected_lifecycle: 'uninstalled_data_retained',
      expected_data_revision: 8,
    };

    await plugins.deleteData.invoke(request);

    expect(calls).toEqual([
      {
        method: 'DELETE',
        path: `/api/plugins/installations/${MOUNT_ID}/data`,
        body: request,
      },
    ]);
  });

  test('keeps the exact Project deletion CAS in the DELETE body', async () => {
    installFetchFixture();
    const request: DeletePluginProjectRequest = {
      project_id: PROJECT_ID,
      expected_project_revision: 5,
      expected_build_generation: 3,
      expected_ready_candidate_id: CANDIDATE_ID,
      expected_ready_candidate_digest: 'd'.repeat(64),
    };

    await plugins.deleteProject.invoke(request);

    expect(calls).toEqual([
      {
        method: 'DELETE',
        path: `/api/plugins/projects/${PROJECT_ID}`,
        body: request,
      },
    ]);
  });

  test('sends dependency requests with the exact Source and lock CAS', async () => {
    installFetchFixture();
    const request: UpdatePluginDependenciesRequest = {
      project_id: PROJECT_ID,
      expected_project_revision: 5,
      expected_build_generation: 3,
      expected_source_snapshot_digest: 'f'.repeat(64),
      expected_dependency_lock_digest: '1'.repeat(64),
      dependencies: { alpha: '^1.0.0' },
    };

    await plugins.updateDependencies.invoke(request);

    expect(calls).toEqual([
      {
        method: 'PUT',
        path: `/api/plugins/projects/${PROJECT_ID}/source/dependencies`,
        body: request,
      },
    ]);
  });

  test('sends standing auto Apply authorization with exact Project and Mount CAS', async () => {
    installFetchFixture();
    const request: SetPluginAutoApplyRequest = {
      project_id: PROJECT_ID,
      expected_project_revision: 5,
      expected_build_generation: 3,
      linked_mount_id: MOUNT_ID,
      expected_linked_mount_revision: 7,
      expected_linked_target_digest: 'a'.repeat(64),
      apply_mode: 'auto_compatible_when_idle',
    };

    await plugins.setAutoApply.invoke(request);

    expect(calls).toEqual([
      {
        method: 'PUT',
        path: `/api/plugins/projects/${PROJECT_ID}/auto-apply`,
        body: request,
      },
    ]);
  });

  test('exports an exact Ready Candidate Share Bundle without user data fields', async () => {
    installFetchFixture();
    const request: SharePluginRequest = {
      project_id: PROJECT_ID,
      expected_project_revision: 5,
      source: 'ready_candidate',
      candidate_id: CANDIDATE_ID,
      expected_candidate_digest: 'd'.repeat(64),
      destination_path: 'C:\\exports\\plugin-share',
      include_source: true,
    };

    const operation = await plugins.exportShare.invoke(request);

    expect(operation.summary.operation_id).toBe(OPERATION_ID);
    expect(calls).toEqual([
      {
        method: 'POST',
        path: `/api/plugins/projects/${PROJECT_ID}/share`,
        body: request,
      },
    ]);
    expect(JSON.stringify(calls[0]?.body).includes('credential')).toBe(false);
    expect(JSON.stringify(calls[0]?.body).includes('data_dir')).toBe(false);
  });

  test('imports a Share Bundle through the same Ready Candidate boundary', async () => {
    installFetchFixture();
    const request: ImportPluginRequest = {
      expected_library_revision: 12,
      import_kind: 'share_bundle',
      source_path: 'C:\\imports\\plugin-share',
      expected_bundle_or_artifact_digest: '9'.repeat(64),
    };

    await plugins.importPrebuilt.invoke(request);

    expect(calls).toEqual([
      {
        method: 'POST',
        path: '/api/plugins/imports',
        body: request,
      },
    ]);
  });

  test('rejects legacy-prefixed Plugin identities instead of normalizing them', async () => {
    installFetchFixture();
    const original = library.plugins[0]!.mount_id;
    library.plugins[0]!.mount_id = `mount_${MOUNT_ID}` as typeof original;
    let error: unknown;
    try {
      await plugins.list.invoke();
    } catch (caught) {
      error = caught;
    } finally {
      library.plugins[0]!.mount_id = original;
    }
    expect(error instanceof InvalidEntityIdError).toBe(true);
  });
});
