/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeCanvasNode, CreativeProjectDocument } from '../../../domain';
import { createEmptyCreativeProjectDocument } from '../../../domain';
import {
  MAX_CREATIVE_CANVAS_AGENT_CONTEXT_NODES,
  MAX_CREATIVE_CANVAS_AGENT_CONTEXT_TEXT_CHARS,
  buildCreativeCanvasAgentContext,
  selectCreativeCanvasAgentContextNodes,
  serializeCreativeCanvasAgentModelInput,
} from './context';

const PROJECT_ID = '0190f5fe-7c00-7a00-8000-000000000301';
const nodeId = (suffix: number): string =>
  `0190f5fe-7c00-7a00-8000-${suffix.toString().padStart(12, '0')}`;
const connectionId = (suffix: number): string =>
  `0190f5fe-7c00-7a00-9000-${suffix.toString().padStart(12, '0')}`;

const textNode = (id: string, text: string): CreativeCanvasNode => ({
  id,
  type: 'text',
  position: { x: 0, y: 0 },
  size: { width: 320, height: 180 },
  groupId: null,
  zIndex: 1,
  locked: false,
  data: { text, format: 'plain', fontSize: 16, textAlign: 'left' },
});

const imageNode = (id: string): CreativeCanvasNode => ({
  id,
  type: 'image',
  position: { x: 400, y: 0 },
  size: { width: 320, height: 240 },
  groupId: null,
  zIndex: 2,
  locked: false,
  data: {
    assetId: nodeId(901),
    caption: 'Reference image',
    alt: '',
    fit: 'contain',
    naturalSize: null,
    composer: null,
  },
});

const fixture = (): CreativeProjectDocument => {
  const document = createEmptyCreativeProjectDocument(PROJECT_ID);
  document.nodes = [
    textNode(nodeId(1), 'First selected'),
    imageNode(nodeId(2)),
    textNode(nodeId(3), 'Second selected'),
    textNode(nodeId(4), 'Unrelated node'),
  ];
  document.connections = [
    {
      id: connectionId(1),
      sourceNodeId: nodeId(1),
      targetNodeId: nodeId(2),
      sourceHandle: null,
      targetHandle: null,
    },
    {
      id: connectionId(2),
      sourceNodeId: nodeId(2),
      targetNodeId: nodeId(4),
      sourceHandle: null,
      targetHandle: null,
    },
  ];
  return document;
};

describe('Creative Canvas Agent context', () => {
  test('orders selected nodes before one-hop neighbors and excludes unrelated nodes', () => {
    const context = buildCreativeCanvasAgentContext({
      document: fixture(),
      canvasRevision: '7',
      selectedNodeIds: [nodeId(3), nodeId(1), nodeId(3), nodeId(999)],
    });
    expect(context.selectedNodeIds).toEqual([nodeId(1), nodeId(3)]);
    expect(context.nodes.map((node) => node.id)).toEqual([
      nodeId(1),
      nodeId(3),
      nodeId(2),
    ]);
    expect(context.nodes.map((node) => node.selected)).toEqual([true, true, false]);
    expect(context.connections.map((connection) => connection.id)).toEqual([connectionId(1)]);
    expect(context.totalNodeCount).toBe(4);
    expect(context.totalConnectionCount).toBe(2);
    expect(context.truncated).toBe(false);
  });

  test('bounds node count and text while omitting media URLs and opaque parameters', () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);
    document.nodes = Array.from({ length: 40 }, (_, index) =>
      textNode(
        nodeId(index + 1),
        index === 0
          ? `data:image/png;base64,${'a'.repeat(2_500)}`
          : `node-${index}`
      )
    );
    const context = buildCreativeCanvasAgentContext({
      document,
      canvasRevision: '8',
      selectedNodeIds: document.nodes.map((node) => node.id),
    });
    expect(context.nodes).toHaveLength(MAX_CREATIVE_CANVAS_AGENT_CONTEXT_NODES);
    expect(context.truncated).toBe(true);
    const firstText = context.nodes[0]!.details.text as string;
    expect(firstText.includes('[omitted-media-url]')).toBe(true);
    expect(firstText.length).toBeLessThanOrEqual(
      MAX_CREATIVE_CANVAS_AGENT_CONTEXT_TEXT_CHARS
    );
    const encoded = JSON.stringify(context);
    expect(encoded.includes('base64')).toBe(false);
    expect(encoded.includes('parameters')).toBe(false);
    expect(encoded.includes('"src":')).toBe(false);
    expect(encoded.includes('"url":')).toBe(false);
  });

  test('projects config identity without provider parameters or local operation payloads', () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);
    const inputs = Array.from({ length: 33 }, (_, index) => nodeId(500 + index));
    document.nodes = [
      {
        id: nodeId(41),
        type: 'config',
        position: { x: 0, y: 0 },
        size: { width: 360, height: 300 },
        groupId: null,
        zIndex: 1,
        locked: false,
        data: {
          task: 'image_generation',
          capability: 't2i',
          providerId: nodeId(800),
          model: 'image-model',
          prompt: 'Use blob:https://secret.invalid/raw as a hidden source',
          negativePrompt: '',
          operation: {
            kind: 'image-node-compose',
            sourceNodeId: nodeId(42),
            sourceAssetId: nodeId(900),
          },
          parameters: { providerSecret: 'data:application/octet-stream,opaque' },
          inputAssetIds: inputs,
          taskId: null,
          resultAssetIds: [],
          status: 'failed',
          errorMessage: 'Provider rejected the request',
        },
      },
    ];
    const context = buildCreativeCanvasAgentContext({
      document,
      canvasRevision: '8',
      selectedNodeIds: [nodeId(41)],
    });
    expect(context.truncated).toBe(true);
    expect(context.nodes[0]!.details).toMatchObject({
      task: 'image_generation',
      capability: 't2i',
      providerId: nodeId(800),
      model: 'image-model',
      prompt: 'Use [omitted-media-url] as a hidden source',
      status: 'failed',
      inputAssetIds: inputs.slice(0, 32),
    });
    expect('parameters' in context.nodes[0]!.details).toBe(false);
    expect('operation' in context.nodes[0]!.details).toBe(false);
    expect(JSON.stringify(context).includes('providerSecret')).toBe(false);
  });

  test('serializes a deterministic approval-only planning envelope', () => {
    const context = buildCreativeCanvasAgentContext({
      document: fixture(),
      canvasRevision: '9',
      selectedNodeIds: [nodeId(1)],
    });
    const first = serializeCreativeCanvasAgentModelInput({
      prompt: '  Organize the selected nodes  ',
      context,
      skillIds: ['creative-studio-canvas', 'creative-studio-organize'],
    });
    const second = serializeCreativeCanvasAgentModelInput({
      prompt: 'Organize the selected nodes',
      context,
      skillIds: ['creative-studio-canvas', 'creative-studio-organize'],
    });
    expect(first).toBe(second);
    expect(JSON.parse(first)).toEqual({
      kind: 'nomifun.creative-studio.planning-turn',
      version: 1,
      userRequest: 'Organize the selected nodes',
      selectedSkills: ['creative-studio-canvas', 'creative-studio-organize'],
      canvasContext: context,
      responseContract: {
        mode: 'plan-and-propose',
        allowedArtifactKinds: ['nomifun.creative-studio.canvas-ops/v1'],
        requiresUserApproval: true,
        forbiddenActions: ['delete-node', 'media-generation'],
      },
    });
  });

  test('applies composer exclusions without reintroducing nodes or dangling connections', () => {
    const context = buildCreativeCanvasAgentContext({
      document: fixture(),
      canvasRevision: '10',
      selectedNodeIds: [nodeId(1), nodeId(3)],
    });
    const selected = selectCreativeCanvasAgentContextNodes(context, [nodeId(3), nodeId(1)]);
    expect(selected.nodes.map((node) => node.id)).toEqual([nodeId(1), nodeId(3)]);
    expect(selected.selectedNodeIds).toEqual([nodeId(1), nodeId(3)]);
    expect(selected.connections).toEqual([]);
    expect(selected.totalNodeCount).toBe(context.totalNodeCount);
    expect(selected.totalConnectionCount).toBe(context.totalConnectionCount);
  });
});
