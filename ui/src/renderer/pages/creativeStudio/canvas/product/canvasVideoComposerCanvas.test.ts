/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { IProvider, ModelTask } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';

import type { CreativeAsset } from '../../assets';
import { createEmptyCreativeProjectDocument } from '../../domain';
import type { CreativeTask } from '../../tasks';
import type {
  CreativeWorkbenchReferences,
  VideoWorkbenchOperation,
} from '../../workbenches/runtime';
import { createInitialCanvasState } from '../core';
import { testNode, testUuid } from '../core/testFixtures';
import {
  canvasVideoComposeConfigForReference,
  canvasVideoComposeConfigFromTask,
  canvasVideoComposeDimensions,
  canvasVideoComposeDraftFromState,
  canvasVideoComposeMode,
  canvasVideoComposeResultPosition,
  canvasVideoComposeResumeRequests,
  canvasVideoComposeSettings,
  canvasVideoComposeSourceAssetId,
  canvasVideoComposeSourceNodeId,
  canvasVideoComposeTaskSummary,
  clearCanvasVideoComposeDraftModel,
  isCanvasVideoComposeConfig,
  latestCanvasVideoComposeConfig,
  prepareCanvasVideoCompose,
  reconcileCanvasVideoComposeConfig,
  withCanvasVideoComposeDraft,
} from './canvasVideoComposerCanvas';

const PROVIDER_ID = testUuid(400) as ProviderId;
const SOURCE_ASSET_ID = testUuid(401);
const IMAGE_ASSET_ID = testUuid(402);
const UPSTREAM_VIDEO_ASSET_ID = testUuid(403);

const capability = (task: ModelTask) => ({
  task,
  traits: [],
  protocol: `test.${task}`,
  connection_role: 'default' as const,
  allow_cross_origin_credentials: false,
  provider_params: {},
  created_at: 1,
  updated_at: 1,
});

const provider: IProvider = {
  id: PROVIDER_ID,
  name: 'Video Generator',
  enabled: true,
  platform: 'custom',
  base_url: 'https://example.invalid',
  auth_scheme: 'bearer',
  has_credentials: true,
  models: [
    {
      provider_id: PROVIDER_ID,
      model: 'video-v1',
      enabled: true,
      sort_order: 0,
      capabilities: [capability('video_generation')],
      created_at: 1,
      updated_at: 1,
    },
  ],
};

const asset = (
  id: string,
  kind: CreativeAsset['kind']
): CreativeAsset => ({
  id,
  kind,
  title: `${kind} reference`,
  collection: null,
  tags: [],
  mimeType:
    kind === 'image'
      ? 'image/png'
      : kind === 'video'
        ? 'video/mp4'
        : null,
  width: kind === 'image' || kind === 'video' ? 1920 : null,
  height: kind === 'image' || kind === 'video' ? 1080 : null,
  bytes: 1,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/files/${id}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
});

const noReferences = (): CreativeWorkbenchReferences => ({
  assets: [],
  bindings: [],
});

const operation = (
  capability: VideoWorkbenchOperation['capability']
): VideoWorkbenchOperation => ({ task: 'video_generation', capability });

const thrownMessage = (action: () => void): string => {
  try {
    action();
    return '';
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
};

const prepareFixture = (overrides: {
  source?: ReturnType<typeof testNode<'video'>>;
  sourceAsset?: CreativeAsset | null;
  operation?: VideoWorkbenchOperation;
  references?: CreativeWorkbenchReferences;
  prompt?: string;
  resolution?: string;
  aspectRatio?: string;
  seconds?: number;
} = {}) => {
  const source = overrides.source ?? testNode('video', 1, { width: 420, height: 236 });
  const document = createEmptyCreativeProjectDocument(testUuid(404));
  document.nodes = [source];
  const prepared = prepareCanvasVideoCompose({
    projectId: document.projectId,
    state: createInitialCanvasState({ document }),
    viewportSize: { width: 1440, height: 900 },
    sourceNode: source,
    sourceAsset: overrides.sourceAsset ?? null,
    catalog: { status: 'ready', providers: [provider], error: null },
    model: { providerId: PROVIDER_ID, model: 'video-v1' },
    operation: overrides.operation ?? operation('t2v'),
    references: overrides.references ?? noReferences(),
    prompt: overrides.prompt ?? '海面上的晨雾缓慢散开',
    settings: {
      resolution: overrides.resolution ?? '1080p',
      aspectRatio: overrides.aspectRatio ?? '16:9',
      seconds: overrides.seconds ?? 5,
    },
  });
  return { document, prepared, source };
};

const taskFor = (
  prepared: ReturnType<typeof prepareFixture>['prepared'],
  overrides: Partial<CreativeTask> = {}
): CreativeTask => ({
  taskId: prepared.plan.input.idempotencyKey,
  owner: { ...prepared.plan.input.owner },
  providerId: prepared.plan.input.providerId,
  model: prepared.plan.input.model,
  task: prepared.plan.input.task,
  capability: prepared.plan.input.capability,
  parameters: structuredClone(prepared.plan.input.parameters),
  inputs: structuredClone(prepared.plan.input.inputs),
  status: 'running',
  error: null,
  resultAssetIds: [],
  attempt: 1,
  submittedAt: 1,
  startedAt: 2,
  finishedAt: null,
  deletedAt: null,
  ...overrides,
});

describe('canvas video composer product model', () => {
  test('prepares exact t2v owner, protocol parameters and source edge', () => {
    const { document, prepared, source } = prepareFixture();

    expect(prepared.plan.repeat).toBe(1);
    expect(prepared.plan.input).toMatchObject({
      owner: {
        kind: 'canvas_node',
        canvasId: document.projectId,
        nodeId: prepared.configNode.id,
      },
      providerId: PROVIDER_ID,
      model: 'video-v1',
      task: 'video_generation',
      capability: 't2v',
      inputs: [],
    });
    expect(prepared.plan.input.parameters).toEqual({
      prompt: '海面上的晨雾缓慢散开',
      seconds: 5,
      width: 1920,
      height: 1080,
    });
    expect(prepared.configNode).toMatchObject({
      type: 'config',
      position: { x: 500, y: 0 },
      locked: true,
      data: {
        operation: {
          kind: 'video-node-compose',
          sourceNodeId: source.id,
          sourceAssetId: null,
        },
        parameters: prepared.plan.input.parameters,
        inputAssetIds: [],
        taskId: prepared.plan.input.idempotencyKey,
        status: 'queued',
      },
    });
    expect(prepared.connection).toEqual({
      sourceNodeId: source.id,
      targetNodeId: prepared.configNode.id,
      sourceHandle: 'source',
      targetHandle: 'target',
    });
    expect(canvasVideoComposeDimensions('720p', '9:16')).toEqual({
      width: 720,
      height: 1280,
    });
  });

  test('prepares i2v from exactly one real image reference', () => {
    const reference = asset(IMAGE_ASSET_ID, 'image');
    const { prepared } = prepareFixture({
      operation: operation('i2v'),
      references: {
        assets: [reference],
        bindings: [
          { assetId: reference.id, kind: 'image', role: 'reference' },
        ],
      },
      resolution: '720p',
      aspectRatio: '1:1',
    });

    expect(prepared.plan.input).toMatchObject({
      task: 'video_generation',
      capability: 'i2v',
      inputs: [{ assetId: IMAGE_ASSET_ID, kind: 'image', role: 'reference' }],
      parameters: {
        width: 720,
        height: 720,
      },
    });
    expect(prepared.configNode.data.inputAssetIds).toEqual([IMAGE_ASSET_ID]);
    expect(prepared.configNode.data.operation).toMatchObject({
      sourceAssetId: null,
    });
  });

  test('rejects unsupported reference contracts, v2v and non-empty targets', () => {
    const image = asset(IMAGE_ASSET_ID, 'image');
    const video = asset(UPSTREAM_VIDEO_ASSET_ID, 'video');

    expect(
      thrownMessage(() =>
        prepareFixture({
          operation: operation('i2v'),
          references: {
            assets: [image],
            bindings: [
              { assetId: image.id, kind: 'image', role: 'first_frame' },
            ],
          },
        })
      ).includes('一张 role=reference')
    ).toBe(true);
    expect(
      thrownMessage(() =>
        prepareFixture({
          operation: operation('i2v'),
          references: {
            assets: [image, asset(testUuid(405), 'image')],
            bindings: [
              { assetId: image.id, kind: 'image', role: 'reference' },
              { assetId: testUuid(405), kind: 'image', role: 'reference' },
            ],
          },
        })
      ).includes('一张 role=reference')
    ).toBe(true);
    expect(
      thrownMessage(() =>
        prepareFixture({
          operation: operation('v2v'),
          references: {
            assets: [video],
            bindings: [
              { assetId: video.id, kind: 'video', role: 'video' },
            ],
          },
        })
      ).includes('尚不支持 v2v')
    ).toBe(true);

    const filled = {
      ...testNode('video', 2),
      data: { ...testNode('video', 2).data, assetId: SOURCE_ASSET_ID },
    };
    expect(
      thrownMessage(() => prepareFixture({ source: filled })).includes(
        '只能使用空视频源节点'
      )
    ).toBe(true);
    expect(
      thrownMessage(() =>
        prepareFixture({ sourceAsset: asset(SOURCE_ASSET_ID, 'video') })
      ).includes('只能使用空视频源节点')
    ).toBe(true);
    expect(
      thrownMessage(() =>
        prepareFixture({
          references: {
            assets: [image],
            bindings: [
              { assetId: image.id, kind: 'image', role: 'reference' },
            ],
          },
        })
      ).includes('t2v does not consume reference assets')
    ).toBe(true);
  });

  test('derives t2v/i2v only from direct real incoming canvas media', () => {
    const source = testNode('video', 1);
    const image = {
      ...testNode('image', 2),
      data: { ...testNode('image', 2).data, assetId: IMAGE_ASSET_ID },
    };
    const secondImage = {
      ...testNode('image', 3),
      data: { ...testNode('image', 3).data, assetId: testUuid(404) },
    };
    const upstreamVideo = {
      ...testNode('video', 4),
      data: { ...testNode('video', 4).data, assetId: UPSTREAM_VIDEO_ASSET_ID },
    };
    const document = createEmptyCreativeProjectDocument(testUuid(405));
    document.nodes = [source, image, secondImage, upstreamVideo];
    expect(canvasVideoComposeMode(document, source.id)).toEqual({ kind: 't2v' });

    document.connections = [{
      id: testUuid(406),
      sourceNodeId: image.id,
      targetNodeId: source.id,
      sourceHandle: null,
      targetHandle: null,
    }];
    expect(canvasVideoComposeMode(document, source.id)).toEqual({
      kind: 'i2v',
      assetId: IMAGE_ASSET_ID,
    });
    document.connections.push({
      id: testUuid(407),
      sourceNodeId: secondImage.id,
      targetNodeId: source.id,
      sourceHandle: null,
      targetHandle: null,
    });
    expect(canvasVideoComposeMode(document, source.id).kind).toBe('unsupported');

    document.connections = [{
      id: testUuid(408),
      sourceNodeId: upstreamVideo.id,
      targetNodeId: source.id,
      sourceHandle: null,
      targetHandle: null,
    }];
    expect(canvasVideoComposeMode(document, source.id).kind).toBe('unsupported');
    document.nodes[0] = {
      ...source,
      data: { ...source.data, assetId: SOURCE_ASSET_ID },
    };
    expect(canvasVideoComposeMode(document, source.id).kind).toBe('unsupported');
  });

  test('restores node-owned draft before latest config and clears its model', () => {
    const { document, prepared, source } = prepareFixture();
    document.nodes.push(prepared.configNode);

    expect(
      canvasVideoComposeDraftFromState(
        createInitialCanvasState({ document }),
        source.id
      )
    ).toEqual({
      prompt: '海面上的晨雾缓慢散开',
      settings: {
        model: { providerId: PROVIDER_ID, model: 'video-v1' },
        resolution: '1080p',
        aspectRatio: '16:9',
        seconds: 5,
      },
    });

    const persisted = withCanvasVideoComposeDraft(source, {
      prompt: '尚未提交的竖屏草稿',
      settings: {
        model: { providerId: PROVIDER_ID, model: 'video-v1' },
        resolution: '720p',
        aspectRatio: '9:16',
        seconds: 10,
      },
    });
    document.nodes[0] = persisted;
    expect(
      canvasVideoComposeDraftFromState(
        createInitialCanvasState({ document }),
        source.id
      )
    ).toEqual({
      prompt: '尚未提交的竖屏草稿',
      settings: {
        model: { providerId: PROVIDER_ID, model: 'video-v1' },
        resolution: '720p',
        aspectRatio: '9:16',
        seconds: 10,
      },
    });

    const cleared = clearCanvasVideoComposeDraftModel(persisted);
    expect(cleared.id).toBe(source.id);
    expect(cleared.data.composer).toEqual({
      ...persisted.data.composer,
      model: null,
    });
    expect(clearCanvasVideoComposeDraftModel(cleared)).toBe(cleared);
  });

  test('recovers only the exact config owner and restores canonical settings', () => {
    const { document, prepared, source } = prepareFixture();
    document.nodes.push(prepared.configNode);
    document.pendingTaskIds = [prepared.plan.input.idempotencyKey];

    expect(canvasVideoComposeResumeRequests(document)).toEqual([
      {
        reference: {
          taskId: prepared.plan.input.idempotencyKey,
          owner: {
            kind: 'canvas_node',
            canvasId: document.projectId,
            nodeId: prepared.configNode.id,
          },
          providerId: PROVIDER_ID,
          model: 'video-v1',
          task: 'video_generation',
          capability: 't2v',
        },
        outputKind: 'video',
      },
    ]);
    expect(isCanvasVideoComposeConfig(prepared.configNode)).toBe(true);
    expect(latestCanvasVideoComposeConfig(document, source.id)?.id).toBe(
      prepared.configNode.id
    );
    expect(canvasVideoComposeSourceNodeId(prepared.configNode)).toBe(source.id);
    expect(canvasVideoComposeSourceAssetId(prepared.configNode)).toBeNull();
    expect(canvasVideoComposeSettings(prepared.configNode)).toEqual({
      model: { providerId: PROVIDER_ID, model: 'video-v1' },
      resolution: '1080p',
      aspectRatio: '16:9',
      seconds: 5,
    });
    expect(canvasVideoComposeTaskSummary(prepared.configNode)).toEqual({
      state: 'queued',
      pendingCount: 1,
      message: undefined,
    });
  });

  test('matches exact task identity and reconciles every terminal outcome', () => {
    const { document, prepared, source } = prepareFixture();
    document.nodes.push(prepared.configNode);
    const running = taskFor(prepared);
    const reference = {
      taskId: running.taskId,
      owner: running.owner,
      providerId: running.providerId,
      model: running.model,
      task: running.task,
      capability: running.capability,
    };

    expect(canvasVideoComposeConfigForReference(document, reference)).toBe(
      prepared.configNode
    );
    expect(canvasVideoComposeConfigFromTask(document, running)).toBe(
      prepared.configNode
    );
    expect(
      thrownMessage(() =>
        canvasVideoComposeConfigForReference(document, {
          ...reference,
          model: 'different-model',
        })
      ).includes('身份不一致')
    ).toBe(true);

    expect(reconcileCanvasVideoComposeConfig(prepared.configNode, running)).toMatchObject({
      locked: true,
      data: { status: 'running', resultAssetIds: [], errorMessage: null },
    });
    const resultAssetId = testUuid(406);
    expect(
      reconcileCanvasVideoComposeConfig(
        prepared.configNode,
        taskFor(prepared, {
          status: 'succeeded',
          resultAssetIds: [resultAssetId],
          finishedAt: 3,
        })
      )
    ).toMatchObject({
      locked: false,
      data: {
        status: 'succeeded',
        resultAssetIds: [resultAssetId],
        errorMessage: null,
      },
    });
    expect(
      reconcileCanvasVideoComposeConfig(
        prepared.configNode,
        taskFor(prepared, {
          status: 'failed',
          error: { kind: 'provider_error', message: '真实失败', httpStatus: 500 },
          finishedAt: 3,
        })
      )
    ).toMatchObject({
      locked: false,
      data: { status: 'failed', resultAssetIds: [], errorMessage: '真实失败' },
    });
    expect(
      canvasVideoComposeResultPosition(
        [source, prepared.configNode],
        prepared.configNode
      )
    ).toEqual({ x: 500, y: 0 });
  });
});
