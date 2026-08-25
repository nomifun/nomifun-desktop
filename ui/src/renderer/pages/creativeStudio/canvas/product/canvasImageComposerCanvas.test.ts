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
import { createInitialCanvasState } from '../core';
import { testNode, testUuid } from '../core/testFixtures';
import {
  canvasImageComposeDraftFromState,
  canvasImageComposeResumeRequests,
  canvasImageComposeSettings,
  canvasImageComposeTaskSummary,
  clearCanvasImageComposeDraftModel,
  latestCanvasImageComposeConfig,
  prepareCanvasImageCompose,
  reconcileCanvasImageComposeConfig,
  withCanvasImageComposeDraft,
} from './canvasImageComposerCanvas';

const PROVIDER_ID = testUuid(300) as ProviderId;
const SOURCE_ASSET_ID = testUuid(301);

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
  name: 'Image Editor',
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
    {
      provider_id: PROVIDER_ID,
      model: 'generate-v1',
      enabled: true,
      sort_order: 1,
      capabilities: [capability('image_generation')],
      created_at: 1,
      updated_at: 1,
    },
  ],
};

const sourceAsset = (overrides: Partial<CreativeAsset> = {}): CreativeAsset => ({
  id: SOURCE_ASSET_ID,
  kind: 'image',
  title: 'Source',
  collection: null,
  tags: [],
  mimeType: 'image/png',
  width: 1024,
  height: 1024,
  bytes: 1,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/files/${SOURCE_ASSET_ID}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
  ...overrides,
});

const preparedFixture = () => {
  const source = {
    ...testNode('image', 1, { width: 340, height: 240 }),
    data: { ...testNode('image', 1).data, assetId: SOURCE_ASSET_ID },
  };
  const document = createEmptyCreativeProjectDocument(testUuid(302));
  document.nodes = [source];
  const prepared = prepareCanvasImageCompose({
    projectId: document.projectId,
    state: createInitialCanvasState({ document }),
    viewportSize: { width: 1440, height: 900 },
    sourceNode: source,
    sourceAsset: sourceAsset(),
    catalog: { status: 'ready', providers: [provider], error: null },
    model: { providerId: PROVIDER_ID, model: 'edit-v1' },
    prompt: '把画面改成清晨',
    settings: {
      interfaceMode: 'images',
      quality: 'high',
      width: 1536,
      height: 1024,
      aspectRatio: '3:2',
      count: 2,
    },
  });
  return { document, prepared, source };
};

const thrownMessage = (action: () => void): string => {
  try {
    action();
    return '';
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
};

describe('canvas image composer product model', () => {
  test('prepares exact i2i task, durable config owner and source edge', () => {
    const { document, prepared, source } = preparedFixture();
    expect(prepared.plan.input).toMatchObject({
      owner: {
        kind: 'canvas_node',
        canvasId: document.projectId,
        nodeId: prepared.configNode.id,
      },
      providerId: PROVIDER_ID,
      model: 'edit-v1',
      task: 'image_edit',
      capability: 'i2i',
      inputs: [{ assetId: SOURCE_ASSET_ID, kind: 'image', role: 'reference' }],
      parameters: {
        prompt: '把画面改成清晨',
        quality: 'high',
        aspect: '3:2',
        count: 2,
      },
    });
    expect(prepared.configNode).toMatchObject({
      type: 'config',
      position: { x: 420, y: 0 },
      locked: true,
      data: {
        operation: {
          kind: 'image-node-compose',
          sourceNodeId: source.id,
          sourceAssetId: SOURCE_ASSET_ID,
        },
        inputAssetIds: [SOURCE_ASSET_ID],
        taskId: prepared.plan.input.idempotencyKey,
        status: 'queued',
      },
    });
    expect(prepared.connection).toMatchObject({
      sourceNodeId: source.id,
      targetNodeId: prepared.configNode.id,
    });
  });

  test('prepares an empty image node as exact t2i with no invented input asset', () => {
    const source = testNode('image', 1, { width: 340, height: 240 });
    const document = createEmptyCreativeProjectDocument(testUuid(304));
    document.nodes = [source];
    const prepared = prepareCanvasImageCompose({
      projectId: document.projectId,
      state: createInitialCanvasState({ document }),
      viewportSize: { width: 1440, height: 900 },
      sourceNode: source,
      sourceAsset: null,
      catalog: { status: 'ready', providers: [provider], error: null },
      model: { providerId: PROVIDER_ID, model: 'generate-v1' },
      prompt: '清晨湖面上的薄雾',
      settings: {
        interfaceMode: 'images',
        quality: 'auto',
        width: 1024,
        height: 1024,
        aspectRatio: '1:1',
        count: 2,
      },
    });

    expect(prepared.plan.input).toMatchObject({
      task: 'image_generation',
      capability: 't2i',
      inputs: [],
      parameters: {
        prompt: '清晨湖面上的薄雾',
        count: 2,
      },
    });
    expect(prepared.configNode.data).toMatchObject({
      task: 'image_generation',
      capability: 't2i',
      operation: {
        kind: 'image-node-compose',
        sourceNodeId: source.id,
        sourceAssetId: null,
      },
      inputAssetIds: [],
      status: 'queued',
    });
    expect(prepared.connection).toMatchObject({
      sourceNodeId: source.id,
      targetNodeId: prepared.configNode.id,
    });
  });

  test('prepares connected multi-image references in exact visible order', () => {
    const target = testNode('image', 20, { width: 340, height: 240 });
    const personAsset = sourceAsset({ id: testUuid(320), title: '人物图' });
    const clothingAsset = sourceAsset({ id: testUuid(321), title: '服装图' });
    const person = {
      ...testNode('image', 21),
      data: { ...testNode('image', 21).data, assetId: personAsset.id },
    };
    const clothing = {
      ...testNode('image', 22),
      data: { ...testNode('image', 22).data, assetId: clothingAsset.id },
    };
    const document = createEmptyCreativeProjectDocument(testUuid(322));
    document.nodes = [target, person, clothing];
    document.connections = [
      {
        id: testUuid(323),
        sourceNodeId: person.id,
        targetNodeId: target.id,
        sourceHandle: 'source',
        targetHandle: 'target',
      },
      {
        id: testUuid(324),
        sourceNodeId: clothing.id,
        targetNodeId: target.id,
        sourceHandle: 'source',
        targetHandle: 'target',
      },
    ];

    const prepared = prepareCanvasImageCompose({
      projectId: document.projectId,
      state: createInitialCanvasState({ document }),
      viewportSize: { width: 1440, height: 900 },
      sourceNode: target,
      sourceAsset: null,
      references: {
        assets: [personAsset, clothingAsset],
        bindings: [personAsset, clothingAsset].map((asset) => ({
          assetId: asset.id,
          kind: 'image' as const,
          role: 'reference' as const,
        })),
      },
      catalog: { status: 'ready', providers: [provider], error: null },
      model: { providerId: PROVIDER_ID, model: 'edit-v1' },
      prompt: '@人物图 是人物，@服装图 是服装',
      providerPrompt: 'Reference 1 是人物，Reference 2 是服装',
      settings: {
        interfaceMode: 'images',
        quality: 'auto',
        width: 1024,
        height: 1024,
        aspectRatio: '1:1',
        count: 1,
      },
    });

    expect(prepared.plan.input).toMatchObject({
      task: 'image_edit',
      capability: 'i2i',
      inputs: [
        { assetId: personAsset.id, kind: 'image', role: 'reference' },
        { assetId: clothingAsset.id, kind: 'image', role: 'reference' },
      ],
      parameters: {
        prompt: 'Reference 1 是人物，Reference 2 是服装',
      },
    });
    expect(prepared.configNode.data).toMatchObject({
      prompt: '@人物图 是人物，@服装图 是服装',
      inputAssetIds: [personAsset.id, clothingAsset.id],
      operation: {
        kind: 'image-node-compose',
        sourceNodeId: target.id,
        sourceAssetId: null,
      },
    });
  });

  test('rejects a source node whose asset identity or kind does not match', () => {
    const { document, source } = preparedFixture();
    const base = {
      projectId: document.projectId,
      state: createInitialCanvasState({ document }),
      viewportSize: { width: 1440, height: 900 },
      sourceNode: source,
      catalog: { status: 'ready' as const, providers: [provider], error: null },
      model: { providerId: PROVIDER_ID, model: 'edit-v1' },
      prompt: 'edit',
      settings: {
        interfaceMode: 'images' as const,
        quality: 'auto' as const,
        width: 1024,
        height: 1024,
        aspectRatio: '1:1',
        count: 1,
      },
    };
    expect(
      thrownMessage(() =>
        prepareCanvasImageCompose({
          ...base,
          sourceAsset: sourceAsset({ id: testUuid(303) }),
        })
      ).includes('源节点与真实图片素材不一致')
    ).toBe(true);
    expect(
      thrownMessage(() =>
        prepareCanvasImageCompose({
          ...base,
          sourceAsset: sourceAsset({ kind: 'video' }),
        })
      ).includes('源节点与真实图片素材不一致')
    ).toBe(true);
  });

  test('recovers only exact composer owners and restores persisted settings', () => {
    const { document, prepared, source } = preparedFixture();
    document.nodes.push(prepared.configNode);
    document.pendingTaskIds = [prepared.plan.input.idempotencyKey];
    expect(canvasImageComposeResumeRequests(document)).toHaveLength(1);
    expect(latestCanvasImageComposeConfig(document, source.id)?.id).toBe(
      prepared.configNode.id
    );
    expect(canvasImageComposeSettings(prepared.configNode)).toEqual({
      model: { providerId: PROVIDER_ID, model: 'edit-v1' },
      interfaceMode: 'images',
      quality: 'high',
      width: 1536,
      height: 1024,
      aspectRatio: '3:2',
      count: 2,
    });
    expect(canvasImageComposeTaskSummary(prepared.configNode)).toEqual({
      state: 'queued',
      pendingCount: 1,
      message: undefined,
    });
  });

  test('fails closed when the backend task input snapshot differs from config', () => {
    const { prepared } = preparedFixture();
    const config = prepared.configNode;
    expect(
      thrownMessage(() =>
        reconcileCanvasImageComposeConfig(config, {
          taskId: config.data.taskId as string,
          owner: {
            kind: 'canvas_node',
            canvasId: testUuid(302),
            nodeId: config.id,
          },
          providerId: PROVIDER_ID,
          model: 'edit-v1',
          task: 'image_edit',
          capability: 'i2i',
          parameters: { prompt: '把画面改成清晨' },
          inputs: [
            { assetId: testUuid(399), kind: 'image', role: 'reference' },
          ],
          status: 'running',
          error: null,
          resultAssetIds: [],
          attempt: 1,
          submittedAt: 1,
          startedAt: 2,
          finishedAt: null,
          deletedAt: null,
        })
      ).includes('输入与画布配置快照不一致')
    ).toBe(true);
  });

  test('restores the node-owned draft before submitted config and clears task-specific models', () => {
    const { document, prepared, source } = preparedFixture();
    document.nodes.push(prepared.configNode);
    const persisted = withCanvasImageComposeDraft(source, {
      prompt: '尚未提交的新草稿',
      settings: {
        model: { providerId: PROVIDER_ID, model: 'edit-v1' },
        interfaceMode: 'responses',
        quality: 'medium',
        width: 1280,
        height: 720,
        aspectRatio: '16:9',
        count: 3,
      },
    });
    document.nodes[0] = persisted;
    const restored = canvasImageComposeDraftFromState(
      createInitialCanvasState({ document }),
      source.id
    );
    expect(restored).toEqual({
      prompt: '尚未提交的新草稿',
      mentions: [],
      settings: {
        model: { providerId: PROVIDER_ID, model: 'edit-v1' },
        interfaceMode: 'responses',
        quality: 'medium',
        width: 1280,
        height: 720,
        aspectRatio: '16:9',
        count: 3,
      },
    });

    const cleared = clearCanvasImageComposeDraftModel(persisted);
    expect(cleared.id).toBe(source.id);
    expect(cleared.data.composer).toEqual({
      ...persisted.data.composer,
      model: null,
    });
    expect(clearCanvasImageComposeDraftModel(cleared)).toBe(cleared);
  });
});
