/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { IProvider, ModelTask } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';

import type { CreativeAsset } from '../../assets';
import {
  createEmptyCreativeProjectDocument,
  type CreativeCanvasNode,
} from '../../domain';
import type { CreativeModelOption } from '../../models';
import type { CreativeTask } from '../../tasks';
import { createInitialCanvasState } from '../core';
import { testNode, testUuid } from '../core/testFixtures';
import {
  canvasImageMaskEditConfigForReference,
  canvasImageMaskEditResultPosition,
  canvasImageMaskEditResumeRequests,
  isCanvasImageMaskEditConfig,
  preferredCanvasImageMaskEditModel,
  prepareCanvasImageMaskEdit,
  reconcileCanvasImageMaskEditConfig,
} from './imageMaskEditCanvas';

const PROVIDER_ID = testUuid(40) as ProviderId;
const SOURCE_ASSET_ID = testUuid(41);
const MARKED_ASSET_ID = testUuid(42);

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
  name: 'Image Edit Provider',
  enabled: true,
  platform: 'custom',
  base_url: 'https://example.invalid',
  auth_scheme: 'bearer',
  has_credentials: true,
  models: [
    {
      provider_id: PROVIDER_ID,
      model: 'edit-v1',
      enabled: true,
      sort_order: 0,
      capabilities: [capability('image_edit')],
      created_at: 1,
      updated_at: 1,
    },
  ],
};

const asset = (
  id: string,
  overrides: Partial<CreativeAsset> = {}
): CreativeAsset => ({
  id,
  kind: 'image',
  title: 'Source',
  collection: null,
  tags: [],
  mimeType: 'image/png',
  width: 1_920,
  height: 1_080,
  bytes: 1,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/files/${id}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
  ...overrides,
});

const taskFor = (
  config: Extract<CreativeCanvasNode, { type: 'config' }>,
  projectId: string,
  status: CreativeTask['status']
): CreativeTask => ({
  taskId: config.data.taskId as string,
  owner: { kind: 'canvas_node', projectId, nodeId: config.id },
  providerId: config.data.providerId as string,
  model: config.data.model as string,
  task: 'image_edit',
  capability: 'i2i',
  parameters: {},
  status,
  error:
    status === 'failed'
      ? { kind: 'provider', message: 'provider failed', httpStatus: 500 }
      : null,
  resultAssetIds: status === 'succeeded' ? [testUuid(50)] : [],
  attempt: 1,
  submittedAt: 1,
  startedAt: status === 'queued' ? null : 2,
  finishedAt:
    status === 'succeeded' || status === 'failed' || status === 'canceled'
      ? 3
      : null,
});

describe('canvas image mask edit product model', () => {
  test('prepares one exact image_edit task and its durable config owner', () => {
    const source = {
      ...testNode('image', 1, { width: 320, height: 220 }),
      data: {
        ...testNode('image', 1).data,
        assetId: SOURCE_ASSET_ID,
      },
    };
    const document = createEmptyCreativeProjectDocument(testUuid(60));
    document.nodes = [source];
    const state = createInitialCanvasState({ document });
    const prepared = prepareCanvasImageMaskEdit({
      projectId: document.projectId,
      state,
      viewportSize: { width: 1_440, height: 900 },
      sourceNode: source,
      sourceAsset: asset(SOURCE_ASSET_ID),
      markedReference: asset(MARKED_ASSET_ID, {
        inLibrary: false,
        tags: ['canvas-mask-edit-operation:op'],
      }),
      referenceDimensions: { width: 1_920, height: 1_080 },
      catalog: { status: 'ready', providers: [provider], error: null },
      model: { providerId: PROVIDER_ID, model: 'edit-v1' },
      userPrompt: '把外套改成红色',
    });

    expect(prepared.plan.input).toMatchObject({
      owner: {
        kind: 'canvas_node',
        projectId: document.projectId,
        nodeId: prepared.configNode.id,
      },
      providerId: PROVIDER_ID,
      model: 'edit-v1',
      task: 'image_edit',
      capability: 'i2i',
      inputs: [{ assetId: MARKED_ASSET_ID, role: 'reference' }],
    });
    expect(prepared.configNode).toMatchObject({
      type: 'config',
      position: { x: 400, y: 0 },
      locked: true,
      data: {
        task: 'image_edit',
        capability: 'i2i',
        providerId: PROVIDER_ID,
        model: 'edit-v1',
        inputAssetIds: [MARKED_ASSET_ID],
        taskId: prepared.plan.input.idempotencyKey,
        status: 'queued',
        operation: {
          kind: 'image-mask-edit',
          sourceNodeId: source.id,
          sourceAssetId: SOURCE_ASSET_ID,
          markedReferenceAssetId: MARKED_ASSET_ID,
        },
      },
    });
    expect(prepared.connection).toMatchObject({
      sourceNodeId: source.id,
      targetNodeId: prepared.configNode.id,
    });
  });

  test('prefers a valid previous or source-origin model and only auto-selects one option', () => {
    const option = (
      providerId: ProviderId,
      model: string
    ): CreativeModelOption => ({
      providerId,
      model,
      providerName: model,
      platform: 'custom',
      task: 'image_edit',
      traits: [],
      protocol: 'test.image_edit',
    });
    const secondProvider = testUuid(70) as ProviderId;
    const options = [
      option(PROVIDER_ID, 'edit-v1'),
      option(secondProvider, 'edit-v2'),
    ];
    expect(
      preferredCanvasImageMaskEditModel(
        options,
        { providerId: secondProvider, model: 'edit-v2' },
        asset(SOURCE_ASSET_ID)
      )
    ).toEqual({ providerId: secondProvider, model: 'edit-v2' });
    expect(
      preferredCanvasImageMaskEditModel(
        options,
        null,
        asset(SOURCE_ASSET_ID, {
          origin: { providerId: PROVIDER_ID, model: 'edit-v1' },
        })
      )
    ).toEqual({ providerId: PROVIDER_ID, model: 'edit-v1' });
    expect(
      preferredCanvasImageMaskEditModel(options, null, asset(SOURCE_ASSET_ID))
    ).toBeNull();
    expect(
      preferredCanvasImageMaskEditModel(
        [options[0]],
        null,
        asset(SOURCE_ASSET_ID)
      )
    ).toEqual({ providerId: PROVIDER_ID, model: 'edit-v1' });
  });

  test('derives recovery only for exact mask-edit owners and reconciles terminal state', () => {
    const document = createEmptyCreativeProjectDocument(testUuid(80));
    const base = testNode('config', 2);
    const config = {
      ...base,
      locked: true,
      data: {
        ...base.data,
        task: 'image_edit' as const,
        capability: 'i2i',
        providerId: PROVIDER_ID,
        model: 'edit-v1',
        taskId: testUuid(81),
        status: 'running' as const,
        operation: {
          kind: 'image-mask-edit' as const,
          sourceNodeId: testUuid(82),
          sourceAssetId: testUuid(83),
          markedReferenceAssetId: testUuid(84),
        },
      },
    };
    document.nodes = [config];
    document.pendingTaskIds = [config.data.taskId];

    const [request] = canvasImageMaskEditResumeRequests(document);
    if (!request) throw new Error('expected one mask-edit recovery request');
    expect(request.reference.taskId).toBe(config.data.taskId);
    expect(
      canvasImageMaskEditConfigForReference(document, request.reference).id
    ).toBe(config.id);
    const succeeded = reconcileCanvasImageMaskEditConfig(
      config,
      taskFor(config, document.projectId, 'succeeded')
    );
    expect(succeeded.locked).toBe(false);
    expect(succeeded.data.status).toBe('succeeded');
    expect(succeeded.data.resultAssetIds).toEqual([testUuid(50)]);
    const failed = reconcileCanvasImageMaskEditConfig(
      config,
      taskFor(config, document.projectId, 'failed')
    );
    expect(failed.data.errorMessage).toBe('provider failed');
    expect(isCanvasImageMaskEditConfig(failed)).toBe(true);
  });

  test('places results to the config right and skips arbitrary collisions', () => {
    const config = {
      ...testNode('config', 1, { x: 400, width: 340, height: 240 }),
      data: {
        ...testNode('config', 1).data,
        task: 'image_edit' as const,
        capability: 'i2i',
        operation: {
          kind: 'image-mask-edit' as const,
          sourceNodeId: testUuid(82),
          sourceAssetId: testUuid(83),
          markedReferenceAssetId: testUuid(84),
        },
      },
    };
    const blocker = testNode('image', 2, {
      x: 820,
      y: 0,
      width: 320,
      height: 500,
    });
    expect(canvasImageMaskEditResultPosition([config], config)).toEqual({
      x: 820,
      y: 0,
    });
    expect(
      canvasImageMaskEditResultPosition([config, blocker], config)
    ).toEqual({
      x: 820,
      y: 560,
    });
  });
});
