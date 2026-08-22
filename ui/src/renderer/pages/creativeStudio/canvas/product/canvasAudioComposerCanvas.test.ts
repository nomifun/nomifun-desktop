/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { IProvider, ModelTask } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';

import type { CreativeAsset, CreativeAssetKind } from '../../assets';
import { createEmptyCreativeProjectDocument } from '../../domain';
import type { CreativeCanvasNode } from '../../domain';
import type { CreativeTask } from '../../tasks';
import type { CreativeWorkbenchReferences } from '../../workbenches/runtime';
import { createInitialCanvasState } from '../core';
import { testNode, testUuid } from '../core/testFixtures';
import {
  canvasAudioComposeConfigForReference,
  canvasAudioComposeConfigFromTask,
  canvasAudioComposeDraftFromState,
  canvasAudioComposeFieldSupport,
  canvasAudioComposeMaxTextLength,
  canvasAudioComposeProtocolProfile,
  canvasAudioComposeResumeRequests,
  canvasAudioComposeSettings,
  canvasAudioComposeSourceAssetId,
  canvasAudioComposeSourceNodeId,
  canvasAudioComposeTaskSummary,
  canvasAudioComposeVoiceRequired,
  canvasAudioComposeVoiceAfterModelChange,
  isCanvasAudioComposeConfig,
  latestCanvasAudioComposeConfig,
  prepareCanvasAudioCompose,
  reconcileCanvasAudioComposeConfig,
  withCanvasAudioComposeDraft,
  type CanvasAudioComposeFormat,
} from './canvasAudioComposerCanvas';

const PROVIDER_ID = testUuid(600) as ProviderId;
const SOURCE_ASSET_ID = testUuid(601);
const REFERENCE_ASSET_ID = testUuid(602);

type AudioNode = Extract<CreativeCanvasNode, { type: 'audio' }>;

const capability = (task: ModelTask, protocol: string) => ({
  task,
  traits: [],
  protocol,
  connection_role: 'default' as const,
  allow_cross_origin_credentials: false,
  provider_params: {},
  created_at: 1,
  updated_at: 1,
});

const providerFor = (protocol: string): IProvider => ({
  id: PROVIDER_ID,
  name: 'Audio Generator',
  enabled: true,
  platform: 'custom',
  base_url: 'https://example.invalid',
  auth_scheme: 'bearer',
  has_credentials: true,
  models: [
    {
      provider_id: PROVIDER_ID,
      model: 'audio-v1',
      enabled: true,
      sort_order: 0,
      capabilities: [capability('speech_synthesis', protocol)],
      created_at: 1,
      updated_at: 1,
    },
  ],
});

const asset = (id: string, kind: CreativeAssetKind): CreativeAsset => ({
  id,
  kind,
  title: `${kind} reference`,
  collection: null,
  tags: [],
  mimeType:
    kind === 'audio'
      ? 'audio/mpeg'
      : kind === 'image'
        ? 'image/png'
        : null,
  width: kind === 'image' ? 1024 : null,
  height: kind === 'image' ? 1024 : null,
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

const thrownMessage = (action: () => void): string => {
  try {
    action();
    return '';
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
};

const prepareFixture = (overrides: {
  protocol?: string;
  source?: AudioNode;
  sourceAsset?: CreativeAsset | null;
  references?: CreativeWorkbenchReferences;
  prompt?: string;
  voice?: string;
  format?: CanvasAudioComposeFormat;
  incoming?: CreativeCanvasNode;
} = {}) => {
  const source =
    overrides.source ?? testNode('audio', 1, { width: 340, height: 160 });
  const document = createEmptyCreativeProjectDocument(testUuid(603));
  document.nodes = [source];
  if (overrides.incoming) {
    document.nodes.push(overrides.incoming);
    document.connections.push({
      id: testUuid(604),
      sourceNodeId: overrides.incoming.id,
      targetNodeId: source.id,
      sourceHandle: 'source',
      targetHandle: 'target',
    });
  }
  const protocol = overrides.protocol ?? 'openai.audio_speech';
  const prepared = prepareCanvasAudioCompose({
    projectId: document.projectId,
    state: createInitialCanvasState({ document }),
    viewportSize: { width: 1440, height: 900 },
    sourceNode: source,
    sourceAsset: overrides.sourceAsset ?? null,
    catalog: {
      status: 'ready',
      providers: [providerFor(protocol)],
      error: null,
    },
    model: { providerId: PROVIDER_ID, model: 'audio-v1' },
    references: overrides.references ?? noReferences(),
    prompt: overrides.prompt ?? '欢迎来到 NomiFun 创意工作室',
    settings: {
      voice: overrides.voice ?? 'nova',
      format: overrides.format ?? 'wav',
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

describe('canvas audio composer product model', () => {
  test('prepares one exact speech_synthesis/tts owner without local metadata', () => {
    const { document, prepared, source } = prepareFixture();

    expect(prepared.plan).toMatchObject({
      kind: 'audio',
      repeat: 1,
      outputKind: 'audio',
      references: [],
    });
    expect(prepared.plan.input).toMatchObject({
      owner: {
        kind: 'canvas_node',
        projectId: document.projectId,
        nodeId: prepared.configNode.id,
      },
      providerId: PROVIDER_ID,
      model: 'audio-v1',
      task: 'speech_synthesis',
      capability: 'tts',
      inputs: [],
    });
    expect(prepared.plan.input.parameters).toEqual({
      prompt: '欢迎来到 NomiFun 创意工作室',
      voice: 'nova',
      format: 'wav',
    });
    for (const forbidden of [
      'speed',
      'instructions',
      'references',
      'canvasOperation',
      'sourceNodeId',
      'sourceAssetId',
    ]) {
      expect(
        Object.prototype.hasOwnProperty.call(
          prepared.plan.input.parameters,
          forbidden
        )
      ).toBe(false);
    }
    expect(prepared.configNode).toMatchObject({
      type: 'config',
      locked: true,
      data: {
        task: 'speech_synthesis',
        capability: 'tts',
        operation: {
          kind: 'audio-node-compose',
          sourceNodeId: source.id,
          sourceAssetId: null,
        },
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
  });

  test('uses exact protocol profiles and minimizes unknown protocols', () => {
    expect(canvasAudioComposeFieldSupport('openai.audio_speech')).toEqual({
      voice: true,
      format: true,
      speed: false,
      instructions: false,
      references: false,
    });
    expect(canvasAudioComposeProtocolProfile('deepgram.speak_rest')).toEqual({
      fieldSupport: {
        voice: false,
        format: true,
        speed: false,
        instructions: false,
        references: false,
      },
      voiceRequired: false,
      maxTextLength: 2_000,
    });
    expect(canvasAudioComposeVoiceRequired('stepfun.audio_speech')).toBe(true);
    expect(canvasAudioComposeVoiceRequired('siliconflow.audio_speech')).toBe(
      true
    );
    expect(canvasAudioComposeVoiceRequired('minimax.t2a')).toBe(false);
    expect(canvasAudioComposeMaxTextLength('future.audio')).toBe(4_096);

    const deepgram = prepareFixture({
      protocol: 'deepgram.speak_rest',
      voice: 'must-not-be-sent',
      format: 'wav',
    }).prepared;
    expect(deepgram.plan.input.parameters).toEqual({
      prompt: '欢迎来到 NomiFun 创意工作室',
      format: 'wav',
    });

    const unknown = prepareFixture({
      protocol: 'future.audio',
      voice: 'must-not-be-sent',
      format: 'wav',
    }).prepared;
    expect(unknown.plan.input.parameters).toEqual({
      prompt: '欢迎来到 NomiFun 创意工作室',
    });
  });

  test('keeps voice ids only inside the same provider and protocol scope', () => {
    const current = {
      providerId: PROVIDER_ID,
      protocol: 'openai.audio_speech',
    };
    expect(
      canvasAudioComposeVoiceAfterModelChange(
        current,
        { ...current },
        'alloy'
      )
    ).toBe('alloy');
    expect(
      canvasAudioComposeVoiceAfterModelChange(
        current,
        { ...current, protocol: 'stepfun.audio_speech' },
        'alloy'
      )
    ).toBe('');
    expect(
      canvasAudioComposeVoiceAfterModelChange(
        current,
        { ...current, providerId: testUuid(610) as ProviderId },
        'alloy'
      )
    ).toBe('');
    expect(
      canvasAudioComposeVoiceAfterModelChange(null, current, 'alloy')
    ).toBe('');
  });

  test('fails fast on required voices and exact protocol text limits', () => {
    expect(
      thrownMessage(() =>
        prepareFixture({ protocol: 'stepfun.audio_speech', voice: '   ' })
      ).includes('需要选择非空音色')
    ).toBe(true);
    expect(
      thrownMessage(() =>
        prepareFixture({
          protocol: 'deepgram.speak_rest',
          prompt: '声'.repeat(2_001),
        })
      ).includes('2000')
    ).toBe(true);
    expect(
      thrownMessage(() =>
        prepareFixture({
          protocol: 'future.audio',
          prompt: '声'.repeat(4_097),
        })
      ).includes('4096')
    ).toBe(true);
  });

  test('rejects filled sources, reference inputs, incoming media and formats outside mp3/wav', () => {
    const filled = testNode('audio', 2);
    filled.data.assetId = SOURCE_ASSET_ID;
    expect(
      thrownMessage(() => prepareFixture({ source: filled })).includes(
        '只支持空音频节点上的 TTS'
      )
    ).toBe(true);
    expect(
      thrownMessage(() =>
        prepareFixture({ sourceAsset: asset(SOURCE_ASSET_ID, 'audio') })
      ).includes('不能绑定源素材')
    ).toBe(true);

    const reference = asset(REFERENCE_ASSET_ID, 'audio');
    expect(
      thrownMessage(() =>
        prepareFixture({
          references: {
            assets: [reference],
            bindings: [
              { assetId: reference.id, kind: 'audio', role: 'reference' },
            ],
          },
        })
      ).includes('不接受参考素材')
    ).toBe(true);

    const incoming = testNode('image', 3);
    incoming.data.assetId = REFERENCE_ASSET_ID;
    expect(
      thrownMessage(() => prepareFixture({ incoming })).includes(
        '不接受图片、视频或音频参考素材'
      )
    ).toBe(true);

    expect(
      thrownMessage(() =>
        prepareFixture({ format: 'flac' as CanvasAudioComposeFormat })
      ).includes('只支持 mp3 或 wav')
    ).toBe(true);
  });

  test('restores latest config before a complete node-owned draft', () => {
    const { document, prepared, source } = prepareFixture();
    document.nodes.push(prepared.configNode);
    expect(
      canvasAudioComposeDraftFromState(
        createInitialCanvasState({ document }),
        source.id
      )
    ).toEqual({
      prompt: '欢迎来到 NomiFun 创意工作室',
      settings: {
        model: { providerId: PROVIDER_ID, model: 'audio-v1' },
        voice: 'nova',
        format: 'wav',
      },
    });

    const persisted = withCanvasAudioComposeDraft(source, {
      prompt: '',
      settings: {
        model: { providerId: PROVIDER_ID, model: 'audio-v1' },
        voice: 'alloy',
        format: 'mp3',
      },
    });
    document.nodes[0] = persisted;
    expect(
      canvasAudioComposeDraftFromState(
        createInitialCanvasState({ document }),
        source.id
      )
    ).toEqual({
      prompt: '',
      settings: {
        model: { providerId: PROVIDER_ID, model: 'audio-v1' },
        voice: 'alloy',
        format: 'mp3',
      },
    });
    expect(persisted.id).toBe(source.id);
  });

  test('recovers only the exact owner and reconciles authoritative status', () => {
    const { document, prepared, source } = prepareFixture();
    document.nodes.push(prepared.configNode);
    document.pendingTaskIds = [prepared.plan.input.idempotencyKey];

    expect(canvasAudioComposeResumeRequests(document)).toEqual([
      {
        reference: {
          taskId: prepared.plan.input.idempotencyKey,
          owner: {
            kind: 'canvas_node',
            projectId: document.projectId,
            nodeId: prepared.configNode.id,
          },
          providerId: PROVIDER_ID,
          model: 'audio-v1',
          task: 'speech_synthesis',
          capability: 'tts',
        },
        outputKind: 'audio',
      },
    ]);
    expect(isCanvasAudioComposeConfig(prepared.configNode)).toBe(true);
    expect(latestCanvasAudioComposeConfig(document, source.id)?.id).toBe(
      prepared.configNode.id
    );
    expect(canvasAudioComposeSourceNodeId(prepared.configNode)).toBe(source.id);
    expect(canvasAudioComposeSourceAssetId(prepared.configNode)).toBeNull();
    expect(canvasAudioComposeSettings(prepared.configNode)).toEqual({
      model: { providerId: PROVIDER_ID, model: 'audio-v1' },
      voice: 'nova',
      format: 'wav',
    });
    expect(canvasAudioComposeTaskSummary(prepared.configNode)).toEqual({
      state: 'queued',
      pendingCount: 1,
      message: undefined,
    });

    const running = taskFor(prepared);
    const reference = {
      taskId: running.taskId,
      owner: running.owner,
      providerId: running.providerId,
      model: running.model,
      task: running.task,
      capability: running.capability,
    };
    expect(canvasAudioComposeConfigForReference(document, reference)).toBe(
      prepared.configNode
    );
    expect(canvasAudioComposeConfigFromTask(document, running)).toBe(
      prepared.configNode
    );
    expect(
      thrownMessage(() =>
        canvasAudioComposeConfigForReference(document, {
          ...reference,
          model: 'different-model',
        })
      ).includes('身份不一致')
    ).toBe(true);

    expect(
      reconcileCanvasAudioComposeConfig(prepared.configNode, running)
    ).toMatchObject({
      locked: true,
      data: { status: 'running', resultAssetIds: [], errorMessage: null },
    });
    expect(
      reconcileCanvasAudioComposeConfig(
        prepared.configNode,
        taskFor(prepared, {
          status: 'succeeded',
          resultAssetIds: [testUuid(605)],
          finishedAt: 3,
        })
      )
    ).toMatchObject({
      locked: false,
      data: { status: 'succeeded', resultAssetIds: [testUuid(605)] },
    });
    expect(
      reconcileCanvasAudioComposeConfig(
        prepared.configNode,
        taskFor(prepared, {
          status: 'failed',
          error: {
            kind: 'provider_error',
            message: '真实音频失败',
            httpStatus: 500,
          },
          finishedAt: 3,
        })
      )
    ).toMatchObject({
      locked: false,
      data: {
        status: 'failed',
        resultAssetIds: [],
        errorMessage: '真实音频失败',
      },
    });
  });
});
