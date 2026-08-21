/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  createEmptyCreativeProjectDocument,
  type CreativeProjectDetail,
} from '../../domain';
import type { CreativeProjectRepository } from '../../services';
import type { CreativeTask, CreativeTaskReference } from '../../tasks';
import {
  ensureStandaloneWorkbenchNode,
  findStandaloneWorkbenchNode,
  parseStandaloneProjectQuery,
  persistStandalonePendingTask,
  persistStandaloneSettledTask,
  removeStandaloneOrphanedTask,
  standaloneResumeRequests,
  STANDALONE_VIDEO_MAX_CONCURRENT_TASKS,
} from './ownership';

const PROJECT_ID = '0190f5fe-7c00-7a00-8000-000000000701';
const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000702';
const TASK_ID = '0190f5fe-7c00-7a00-8000-000000000703';
const ASSET_ID = '0190f5fe-7c00-7a00-8000-000000000704';

function repositoryFixture(): {
  repository: CreativeProjectRepository;
  current(): CreativeProjectDetail;
  saves(): number;
} {
  let saves = 0;
  let detail: CreativeProjectDetail = {
    project: {
      projectId: PROJECT_ID,
      title: '项目',
      revision: '1',
      nodeCount: 0,
      connectionCount: 0,
      createdAt: 1,
      updatedAt: 1,
    },
    document: createEmptyCreativeProjectDocument(PROJECT_ID),
  };
  const repository: CreativeProjectRepository = {
    list: async () => [detail.project],
    create: async () => detail.project,
    load: async () => structuredClone(detail),
    save: async (_projectId, expectedRevision, document) => {
      if (expectedRevision !== detail.project.revision) throw new Error('unexpected revision');
      saves += 1;
      detail = {
        project: {
          ...detail.project,
          revision: String(Number(detail.project.revision) + 1),
          nodeCount: document.nodes.length,
          connectionCount: document.connections.length,
        },
        document: structuredClone(document),
      };
      return detail.project;
    },
    rename: async () => detail.project,
    remove: async () => undefined,
  };
  return { repository, current: () => structuredClone(detail), saves: () => saves };
}

const draft = {
  task: 'image_generation' as const,
  capability: 't2i',
  prompt: '海边日落',
  providerId: PROVIDER_ID,
  model: 'image-real',
  parameters: { quality: 'high' },
  inputAssetIds: [] as string[],
};

describe('standalone workbench ownership', () => {
  test('requires one explicit canonical query and never infers a project', () => {
    expect(parseStandaloneProjectQuery('').state).toBe('missing');
    expect(parseStandaloneProjectQuery('?projectId=recent').state).toBe('invalid');
    expect(
      parseStandaloneProjectQuery(`?projectId=${PROJECT_ID}&projectId=${PROJECT_ID}`).state
    ).toBe('invalid');
    expect(parseStandaloneProjectQuery(`?projectId=${PROJECT_ID}`)).toEqual({
      state: 'valid',
      projectId: PROJECT_ID,
    });
  });

  test('creates one visible canonical config node and reuses it on later submissions', async () => {
    const fixture = repositoryFixture();
    const first = await ensureStandaloneWorkbenchNode(
      PROJECT_ID,
      'image',
      draft,
      fixture.repository
    );
    const second = await ensureStandaloneWorkbenchNode(
      PROJECT_ID,
      'image',
      { ...draft, prompt: '新的提示词' },
      fixture.repository
    );
    expect(second.id).toBe(first.id);
    expect(fixture.current().document.nodes).toHaveLength(1);
    expect(fixture.current().document.nodes[0]?.position.x).toBeGreaterThanOrEqual(0);
    expect(findStandaloneWorkbenchNode(fixture.current().document, 'image')?.data.prompt).toBe(
      '新的提示词'
    );
  });

  test('persists pending ownership before settling and rebuilds mount recovery', async () => {
    const fixture = repositoryFixture();
    const node = await ensureStandaloneWorkbenchNode(
      PROJECT_ID,
      'image',
      draft,
      fixture.repository
    );
    const reference: CreativeTaskReference = {
      taskId: TASK_ID,
      owner: {
        kind: 'canvas_node',
        projectId: PROJECT_ID,
        nodeId: node.id,
      },
      providerId: PROVIDER_ID,
      model: 'image-real',
      task: 'image_generation',
      capability: 't2i',
    };
    await persistStandalonePendingTask(PROJECT_ID, 'image', reference, fixture.repository);
    expect(fixture.current().document.pendingTaskIds).toEqual([TASK_ID]);
    expect(standaloneResumeRequests(fixture.current().document, 'image')[0]?.reference).toEqual(
      reference
    );

    const task: CreativeTask = {
      ...reference,
      parameters: { prompt: '海边日落' },
      inputs: [],
      status: 'succeeded',
      error: null,
      resultAssetIds: [ASSET_ID],
      attempt: 1,
      submittedAt: 1,
      startedAt: 2,
      finishedAt: 3,
    };
    await persistStandaloneSettledTask(PROJECT_ID, 'image', task, fixture.repository);
    const settled = fixture.current().document;
    expect(settled.pendingTaskIds).toEqual([]);
    expect(findStandaloneWorkbenchNode(settled, 'image')?.data.resultAssetIds).toEqual([ASSET_ID]);
    expect(findStandaloneWorkbenchNode(settled, 'image')?.data.status).toBe('succeeded');
  });

  test('404 recovery removes only its own orphan and video fan-out remains fail-closed', async () => {
    const fixture = repositoryFixture();
    const node = await ensureStandaloneWorkbenchNode(PROJECT_ID, 'video', {
      ...draft,
      task: 'video_generation',
      capability: 't2v',
      model: 'video-real',
    }, fixture.repository);
    const reference: CreativeTaskReference = {
      taskId: TASK_ID,
      owner: {
        kind: 'canvas_node',
        projectId: PROJECT_ID,
        nodeId: node.id,
      },
      providerId: PROVIDER_ID,
      model: 'video-real',
      task: 'video_generation',
      capability: 't2v',
    };
    await persistStandalonePendingTask(PROJECT_ID, 'video', reference, fixture.repository);
    const removed = await removeStandaloneOrphanedTask(
      PROJECT_ID,
      'video',
      reference,
      { name: 'BackendHttpError', status: 404, code: 'not_found' },
      fixture.repository
    );
    expect(removed).toBe(true);
    expect(fixture.current().document.pendingTaskIds).toEqual([]);
    expect(STANDALONE_VIDEO_MAX_CONCURRENT_TASKS).toBe(1);
  });
});
