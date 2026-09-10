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
  parsePluginProjectId,
} from '../types/ids';
import type {
  ConfigurePluginRequest,
  DeletePluginDataRequest,
  DeletePluginProjectRequest,
  PluginDetail,
  PluginLibraryResponse,
  PluginProjectDetail,
  UpdatePluginDependenciesRequest,
} from '../types/pluginPlatform';
import { plugins } from './pluginPlatformBridge';

const PROJECT_ID = parsePluginProjectId('0190f5fe-7c00-7a00-8000-000000000001');
const MOUNT_ID = parsePluginMountId('0190f5fe-7c00-7a00-8000-000000000002');
const ARTIFACT_ID = parsePluginArtifactId('0190f5fe-7c00-7a00-8000-000000000003');
const CANDIDATE_ID = parsePluginCandidateId('0190f5fe-7c00-7a00-8000-000000000004');
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

    const data = path === '/api/plugins'
      ? library
      : path.includes('/api/plugin-projects/')
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
      { method: 'GET', path: `/api/plugin-projects/${PROJECT_ID}` },
      { method: 'GET', path: `/api/plugin-mounts/${MOUNT_ID}` },
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
      { method: 'PUT', path: `/api/plugin-mounts/${MOUNT_ID}/enabled` },
      { method: 'POST', path: `/api/plugin-mounts/${MOUNT_ID}/retry` },
      { method: 'POST', path: `/api/plugin-mounts/${MOUNT_ID}/restore` },
      { method: 'POST', path: `/api/plugin-mounts/${MOUNT_ID}/uninstall` },
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
        path: `/api/plugin-mounts/${MOUNT_ID}/config`,
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
        path: `/api/plugin-mounts/${MOUNT_ID}/data`,
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
        path: `/api/plugin-projects/${PROJECT_ID}`,
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
        path: `/api/plugin-projects/${PROJECT_ID}/source/dependencies`,
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
