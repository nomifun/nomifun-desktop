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
      data: { assetId: null, caption: '', alt: '', fit: 'contain', naturalSize: null },
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

    const missingTarget = structuredClone(document);
    missingTarget.connections[0].targetNodeId = 'missing';
    expectContractError(
      () => parseCreativeProjectDocument(missingTarget),
      'INVALID_DOCUMENT',
      '$.connections[0].targetNodeId'
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
      data: { assetId: null, caption: '', alt: '', fit: 'contain', naturalSize: null },
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
