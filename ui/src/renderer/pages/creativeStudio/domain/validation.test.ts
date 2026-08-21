/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  CREATIVE_STUDIO_DOCUMENT_SCHEMA,
  createEmptyCreativeProjectDocument,
  CreativeStudioContractError,
  parseCreativeProjectDetailResponse,
  parseCreativeProjectDocument,
  parseCreativeProjectListResponse,
  parseSaveCreativeProjectRequest,
  type CreativeCanvasNode,
} from '.';

const PROJECT_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f112';
const OTHER_PROJECT_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f113';
const CHAT_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f114';
const IDEMPOTENCY_KEY = '0198f8bb-8424-7b3d-8f17-bc6a1676f115';
const USER_MESSAGE_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f116';
const ASSISTANT_MESSAGE_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f117';

const summary = (projectId = PROJECT_ID) => ({
  projectId,
  title: '产品视觉探索',
  revision: '12',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 1_770_000_000_000,
  updatedAt: 1_770_000_100_000,
});

const expectContractError = (
  operation: () => unknown,
  code: CreativeStudioContractError['code'],
  path: string
) => {
  try {
    operation();
    throw new Error('Expected contract parser to reject the value');
  } catch (error) {
    expect(error instanceof CreativeStudioContractError).toBe(true);
    expect((error as CreativeStudioContractError).code).toBe(code);
    expect((error as CreativeStudioContractError).path).toBe(path);
  }
};

describe('Creative Studio v1 document contract', () => {
  test('builds and parses the only supported empty document', () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);

    expect(document.schema).toBe(CREATIVE_STUDIO_DOCUMENT_SCHEMA);
    expect(parseCreativeProjectDocument(document, PROJECT_ID)).toEqual(document);
    expect(document.nodes).toEqual([]);
    expect(document.connections).toEqual([]);
    expect(document.viewport.zoom).toBe(1);
    expect(document.background).toBe('lines');
    expect(document.panels).toEqual({
      left: { open: true, width: 280, activeView: 'canvas' },
      right: { open: false, width: 390, activeView: 'assistant' },
      bottom: { open: false, height: 240, activeView: 'history' },
    });
  });

  test('rejects legacy or future schema markers without fallback conversion', () => {
    const document = {
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      schema: 1,
    };

    expectContractError(
      () => parseCreativeProjectDocument(document),
      'SCHEMA_MISMATCH',
      '$.schema'
    );
  });

  test('pins Agent model, recovery fence, and completed message pairs in the document', () => {
    const pending = createEmptyCreativeProjectDocument(PROJECT_ID);
    pending.chatSessions = [
      {
        id: CHAT_ID,
        title: '海报创作',
        messageIds: [],
        model: { providerId: '0198f8bb-8424-7b3d-8f17-bc6a1676f118', model: 'gpt-5' },
        pendingTurn: {
          idempotencyKey: IDEMPOTENCY_KEY,
          prompt: '生成一张海报',
          createdAt: 20,
        },
        createdAt: 10,
        updatedAt: 20,
      },
    ];
    pending.activeChatId = CHAT_ID;
    expect(parseCreativeProjectDocument(pending)).toEqual(pending);

    const completed = structuredClone(pending);
    completed.chatSessions[0]!.messageIds = [USER_MESSAGE_ID, ASSISTANT_MESSAGE_ID];
    completed.chatSessions[0]!.pendingTurn = null;
    expect(parseCreativeProjectDocument(completed)).toEqual(completed);

    const missingModel = structuredClone(pending);
    missingModel.chatSessions[0]!.model = null;
    expectContractError(
      () => parseCreativeProjectDocument(missingModel),
      'INVALID_DOCUMENT',
      '$.chatSessions[0].model'
    );

    const halfPair = structuredClone(completed);
    halfPair.chatSessions[0]!.messageIds = [USER_MESSAGE_ID];
    expectContractError(
      () => parseCreativeProjectDocument(halfPair),
      'INVALID_DOCUMENT',
      '$.chatSessions[0].messageIds'
    );

    const inactivePending = structuredClone(pending);
    inactivePending.activeChatId = null;
    expectContractError(
      () => parseCreativeProjectDocument(inactivePending),
      'INVALID_DOCUMENT',
      '$.activeChatId'
    );
  });

  test('rejects unknown fields and mismatched project ownership', () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);
    expectContractError(
      () => parseCreativeProjectDocument({ ...document, legacyCanvasId: 'legacy' }),
      'INVALID_DOCUMENT',
      '$.legacyCanvasId'
    );
    expectContractError(
      () => parseCreativeProjectDocument(document, OTHER_PROJECT_ID),
      'PROJECT_MISMATCH',
      '$.projectId'
    );
  });

  test('requires UUIDv7 project identity, a bounded viewport, and a source background mode', () => {
    const invalidId = {
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      projectId: 'creative-studio-project-123',
    };
    const invalidZoom = {
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      viewport: { x: 0, y: 0, zoom: 5.01 },
    };
    const inventedBackground = {
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      background: 'grid',
    };

    expectContractError(() => parseCreativeProjectDocument(invalidId), 'INVALID_DOCUMENT', '$.projectId');
    expectContractError(() => parseCreativeProjectDocument(invalidZoom), 'INVALID_DOCUMENT', '$.viewport.zoom');
    expectContractError(
      () => parseCreativeProjectDocument(inventedBackground),
      'INVALID_DOCUMENT',
      '$.background'
    );
  });

  test('parses typed nodes and validates graph references', () => {
    const group: CreativeCanvasNode = {
      id: 'group-1',
      type: 'group',
      position: { x: 10, y: 20 },
      size: { width: 640, height: 480 },
      groupId: null,
      zIndex: 0,
      locked: false,
      data: { title: '构图', color: '#7c5cff', collapsed: false },
    };
    const text: CreativeCanvasNode = {
      id: 'text-1',
      type: 'text',
      position: { x: 40, y: 60 },
      size: { width: 280, height: 160 },
      groupId: group.id,
      zIndex: 1,
      locked: false,
      data: { text: '镜头描述', format: 'markdown', fontSize: 16, textAlign: 'left' },
    };
    const image: CreativeCanvasNode = {
      id: 'image-1',
      type: 'image',
      position: { x: 360, y: 60 },
      size: { width: 320, height: 240 },
      groupId: group.id,
      zIndex: 2,
      locked: false,
      data: {
        assetId: null,
        caption: '',
        alt: '',
        fit: 'contain',
        naturalSize: null,
        composer: {
          prompt: '雾中灯塔',
          model: {
            providerId: '0198f8bb-8424-7b3d-8f17-bc6a1676f118',
            model: 'image-v1',
          },
          interfaceMode: 'images',
          quality: 'high',
          width: 1536,
          height: 1024,
          aspectRatio: '3:2',
          count: 2,
        },
      },
    };
    const document = {
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      nodes: [group, text, image],
      connections: [
        {
          id: 'edge-1',
          sourceNodeId: text.id,
          targetNodeId: image.id,
          sourceHandle: null,
          targetHandle: null,
        },
      ],
    };

    expect(parseCreativeProjectDocument(document).nodes[1]).toEqual(text);
    expect(parseCreativeProjectDocument(document).nodes[2]).toEqual(image);

    const missingTarget = structuredClone(document);
    missingTarget.connections[0].targetNodeId = 'missing';
    expectContractError(
      () => parseCreativeProjectDocument(missingTarget),
      'INVALID_DOCUMENT',
      '$.connections[0].targetNodeId'
    );
  });

  test('defaults old v1 image composer data and rejects malformed durable drafts', () => {
    const image: CreativeCanvasNode = {
      id: 'image-legacy',
      type: 'image',
      position: { x: 0, y: 0 },
      size: { width: 320, height: 240 },
      groupId: null,
      zIndex: 0,
      locked: false,
      data: { assetId: null, caption: '', alt: '', fit: 'contain', naturalSize: null, composer: null },
    };
    const oldV1 = structuredClone(image) as unknown as {
      data: Record<string, unknown>;
    };
    delete oldV1.data.composer;
    const parsed = parseCreativeProjectDocument({
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      nodes: [oldV1],
    });
    expect(parsed.nodes[0].type === 'image' && parsed.nodes[0].data.composer).toBeNull();

    const malformed = structuredClone(image);
    if (malformed.type !== 'image') throw new Error('fixture must be an image');
    malformed.data.composer = {
      prompt: 'draft',
      model: {
        providerId: 'not-a-provider-id',
        model: 'image-v1',
      },
      interfaceMode: 'images',
      quality: 'auto',
      width: 1024,
      height: 1024,
      aspectRatio: '1:1',
      count: 1,
    };
    expectContractError(
      () =>
        parseCreativeProjectDocument({
          ...createEmptyCreativeProjectDocument(PROJECT_ID),
          nodes: [malformed],
        }),
      'INVALID_DOCUMENT',
      '$.nodes[0].data.composer.model.providerId'
    );

    const partialDimensions = structuredClone(malformed);
    if (partialDimensions.type !== 'image' || !partialDimensions.data.composer) {
      throw new Error('fixture must contain an image composer');
    }
    partialDimensions.data.composer.model = null;
    partialDimensions.data.composer.width = null;
    expectContractError(
      () =>
        parseCreativeProjectDocument({
          ...createEmptyCreativeProjectDocument(PROJECT_ID),
          nodes: [partialDimensions],
        }),
      'INVALID_DOCUMENT',
      '$.nodes[0].data.composer'
    );

    const paddedAspect = structuredClone(partialDimensions);
    if (paddedAspect.type !== 'image' || !paddedAspect.data.composer) {
      throw new Error('fixture must contain an image composer');
    }
    paddedAspect.data.composer.width = 1024;
    paddedAspect.data.composer.aspectRatio = ' 1:1 ';
    expectContractError(
      () =>
        parseCreativeProjectDocument({
          ...createEmptyCreativeProjectDocument(PROJECT_ID),
          nodes: [paddedAspect],
        }),
      'INVALID_DOCUMENT',
      '$.nodes[0].data.composer.aspectRatio'
    );
  });

  test('round-trips video composer drafts and defaults old v1 video nodes', () => {
    const video: CreativeCanvasNode = {
      id: 'video-1',
      type: 'video',
      position: { x: 0, y: 0 },
      size: { width: 420, height: 236 },
      groupId: null,
      zIndex: 0,
      locked: false,
      data: {
        assetId: null,
        posterAssetId: null,
        autoplay: false,
        loop: false,
        muted: true,
        trimStartMs: 0,
        trimEndMs: null,
        composer: {
          prompt: '缓慢推进到城市天际线',
          model: {
            providerId: '0198f8bb-8424-7b3d-8f17-bc6a1676f118',
            model: 'video-v1',
          },
          resolution: '1080p',
          aspectRatio: '16:9',
          seconds: 5,
        },
      },
    };
    const parsed = parseCreativeProjectDocument({
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      nodes: [video],
    });
    expect(parsed.nodes[0]).toEqual(video);

    const oldV1 = structuredClone(video) as unknown as { data: Record<string, unknown> };
    delete oldV1.data.composer;
    const oldParsed = parseCreativeProjectDocument({
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      nodes: [oldV1],
    });
    expect(oldParsed.nodes[0].type === 'video' && oldParsed.nodes[0].data.composer).toBeNull();

    const invalid = structuredClone(video);
    if (invalid.type !== 'video' || !invalid.data.composer) {
      throw new Error('fixture must contain a video composer');
    }
    invalid.data.composer.seconds = 0;
    expectContractError(
      () =>
        parseCreativeProjectDocument({
          ...createEmptyCreativeProjectDocument(PROJECT_ID),
          nodes: [invalid],
        }),
      'INVALID_DOCUMENT',
      '$.nodes[0].data.composer.seconds'
    );
    invalid.data.composer.seconds = 5;
    invalid.data.composer.resolution = ' 1080p ';
    expectContractError(
      () =>
        parseCreativeProjectDocument({
          ...createEmptyCreativeProjectDocument(PROJECT_ID),
          nodes: [invalid],
        }),
      'INVALID_DOCUMENT',
      '$.nodes[0].data.composer.resolution'
    );
  });

  test('round-trips audio composer drafts and defaults old v1 audio nodes', () => {
    const audio: CreativeCanvasNode = {
      id: 'audio-1',
      type: 'audio',
      position: { x: 0, y: 0 },
      size: { width: 340, height: 160 },
      groupId: null,
      zIndex: 0,
      locked: false,
      data: {
        assetId: null,
        title: '',
        loop: false,
        volume: 1,
        trimStartMs: 0,
        trimEndMs: null,
        composer: {
          prompt: '欢迎来到 NomiFun 创作空间',
          model: {
            providerId: '0198f8bb-8424-7b3d-8f17-bc6a1676f119',
            model: 'tts-v1',
          },
          voice: 'alloy',
          format: 'mp3',
        },
      },
    };
    const parsed = parseCreativeProjectDocument({
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      nodes: [audio],
    });
    expect(parsed.nodes[0]).toEqual(audio);

    const oldV1 = structuredClone(audio) as unknown as {
      data: Record<string, unknown>;
    };
    delete oldV1.data.composer;
    const oldParsed = parseCreativeProjectDocument({
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      nodes: [oldV1],
    });
    expect(
      oldParsed.nodes[0].type === 'audio' && oldParsed.nodes[0].data.composer
    ).toBeNull();

    const invalidFormat = structuredClone(audio);
    if (invalidFormat.type !== 'audio' || !invalidFormat.data.composer) {
      throw new Error('fixture must contain an audio composer');
    }
    invalidFormat.data.composer.format = 'aac' as 'mp3';
    expectContractError(
      () =>
        parseCreativeProjectDocument({
          ...createEmptyCreativeProjectDocument(PROJECT_ID),
          nodes: [invalidFormat],
        }),
      'INVALID_DOCUMENT',
      '$.nodes[0].data.composer.format'
    );

    const invalidVoice = structuredClone(audio);
    if (invalidVoice.type !== 'audio' || !invalidVoice.data.composer) {
      throw new Error('fixture must contain an audio composer');
    }
    invalidVoice.data.composer.voice = ' alloy ';
    expectContractError(
      () =>
        parseCreativeProjectDocument({
          ...createEmptyCreativeProjectDocument(PROJECT_ID),
          nodes: [invalidVoice],
        }),
      'INVALID_DOCUMENT',
      '$.nodes[0].data.composer.voice'
    );
  });

  test('normalizes legacy canvas operation metadata out of provider parameters', () => {
    const legacyConfig = {
      id: 'config-legacy',
      type: 'config',
      position: { x: 0, y: 0 },
      size: { width: 440, height: 240 },
      groupId: null,
      zIndex: 0,
      locked: false,
      data: {
        task: 'image_edit',
        capability: 'i2i',
        providerId: null,
        model: null,
        prompt: 'edit',
        negativePrompt: '',
        parameters: {
          prompt: 'provider prompt',
          width: 1024,
          canvasOperation: 'image-mask-edit',
          sourceNodeId: 'image-source',
          sourceAssetId: 'asset-source',
          markedReferenceAssetId: 'asset-mask',
          userPrompt: 'local prompt',
          referenceWidth: 1024,
          referenceHeight: 1024,
        },
        inputAssetIds: ['asset-mask'],
        taskId: null,
        resultAssetIds: [],
        status: 'idle',
        errorMessage: null,
      },
    };
    const parsed = parseCreativeProjectDocument({
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      nodes: [legacyConfig],
    });
    if (parsed.nodes[0]?.type !== 'config') throw new Error('expected config');
    expect(parsed.nodes[0].data.operation).toEqual({
      kind: 'image-mask-edit',
      sourceNodeId: 'image-source',
      sourceAssetId: 'asset-source',
      markedReferenceAssetId: 'asset-mask',
    });
    expect(parsed.nodes[0].data.parameters).toEqual({
      prompt: 'provider prompt',
      width: 1024,
    });

    const duplicate = {
      ...structuredClone(legacyConfig),
      data: {
        ...structuredClone(legacyConfig.data),
        operation: {
          kind: 'image-mask-edit',
          sourceNodeId: 'image-source',
          sourceAssetId: 'asset-source',
          markedReferenceAssetId: 'asset-mask',
        },
      },
    };
    expectContractError(
      () =>
        parseCreativeProjectDocument({
          ...createEmptyCreativeProjectDocument(PROJECT_ID),
          nodes: [duplicate],
        }),
      'INVALID_DOCUMENT',
      '$.nodes[0].data.parameters.canvasOperation'
    );
  });

  test('rejects graph states the editor cannot create', () => {
    const image: CreativeCanvasNode = {
      id: 'image-1',
      type: 'image',
      position: { x: 0, y: 0 },
      size: { width: 320, height: 240 },
      groupId: null,
      zIndex: 0,
      locked: false,
      data: { assetId: null, caption: '', alt: '', fit: 'contain', naturalSize: null, composer: null },
    };
    const director: CreativeCanvasNode = {
      id: 'director-1',
      type: 'director',
      position: { x: 400, y: 0 },
      size: { width: 360, height: 300 },
      groupId: null,
      zIndex: 1,
      locked: false,
      data: { sceneId: null, cameraId: null, timelineMs: 0, durationMs: 0 },
    };
    const valid = {
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      nodes: [image, director],
      connections: [
        {
          id: 'edge-1',
          sourceNodeId: image.id,
          targetNodeId: director.id,
          sourceHandle: null,
          targetHandle: null,
        },
      ],
    };

    expect(parseCreativeProjectDocument(valid).connections).toHaveLength(1);

    const selfConnected = structuredClone(valid);
    selfConnected.connections[0].targetNodeId = image.id;
    expectContractError(
      () => parseCreativeProjectDocument(selfConnected),
      'INVALID_DOCUMENT',
      '$.connections[0].targetNodeId'
    );

    const directorOutput = structuredClone(valid);
    directorOutput.connections[0].sourceNodeId = director.id;
    directorOutput.connections[0].targetNodeId = image.id;
    expectContractError(
      () => parseCreativeProjectDocument(directorOutput),
      'INVALID_DOCUMENT',
      '$.connections[0].sourceNodeId'
    );

    const duplicate = structuredClone(valid);
    duplicate.connections.push({ ...duplicate.connections[0], id: 'edge-2' });
    expectContractError(
      () => parseCreativeProjectDocument(duplicate),
      'INVALID_DOCUMENT',
      '$.connections[1]'
    );
  });

  test('rejects node payload drift instead of accepting open legacy records', () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);
    document.nodes.push({
      id: 'text-1',
      type: 'text',
      position: { x: 0, y: 0 },
      size: { width: 300, height: 180 },
      groupId: null,
      zIndex: 0,
      locked: false,
      data: {
        text: 'Hello',
        format: 'plain',
        fontSize: 16,
        textAlign: 'left',
        legacyHtml: '<b>Hello</b>',
      },
    } as CreativeCanvasNode);

    expectContractError(
      () => parseCreativeProjectDocument(document),
      'INVALID_DOCUMENT',
      '$.nodes[0].data.legacyHtml'
    );
  });
});

describe('Creative Studio project wire contract', () => {
  test('parses list/detail envelopes and verifies redundant node count', () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);
    expect(parseCreativeProjectListResponse({ projects: [summary()] }).projects).toHaveLength(1);
    expect(parseCreativeProjectDetailResponse({ project: summary(), document })).toEqual({
      project: summary(),
      document,
    });

    expectContractError(
      () => parseCreativeProjectDetailResponse({ project: { ...summary(), nodeCount: 1 }, document }),
      'INVALID_RESPONSE',
      '$.project.nodeCount'
    );
    expectContractError(
      () =>
        parseCreativeProjectDetailResponse({
          project: { ...summary(), connectionCount: 1 },
          document,
        }),
      'INVALID_RESPONSE',
      '$.project.connectionCount'
    );
  });

  test('validates CAS requests against the URL project identity', () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);
    expect(parseSaveCreativeProjectRequest({ expectedRevision: '12', document }, PROJECT_ID)).toEqual({
      expectedRevision: '12',
      document,
    });
    expectContractError(
      () => parseSaveCreativeProjectRequest({ expectedRevision: 12, document }, PROJECT_ID),
      'INVALID_REQUEST',
      '$.expectedRevision'
    );
  });
});
