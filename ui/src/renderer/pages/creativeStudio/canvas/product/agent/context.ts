/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeCanvasConnection,
  CreativeCanvasNode,
  CreativeProjectDocument,
} from '../../../domain';

export const CREATIVE_CANVAS_AGENT_CONTEXT_KIND =
  'nomifun.creative-studio.canvas-context' as const;
export const CREATIVE_CANVAS_AGENT_CONTEXT_VERSION = 1 as const;
export const CREATIVE_CANVAS_AGENT_TURN_KIND =
  'nomifun.creative-studio.planning-turn' as const;
export const MAX_CREATIVE_CANVAS_AGENT_CONTEXT_NODES = 32;
export const MAX_CREATIVE_CANVAS_AGENT_CONTEXT_CONNECTIONS = 64;
export const MAX_CREATIVE_CANVAS_AGENT_CONTEXT_TEXT_CHARS = 2_000;
export const MAX_CREATIVE_CANVAS_AGENT_MODEL_INPUT_CHARS = 262_144;

export type CreativeCanvasAgentContextValue =
  | string
  | number
  | boolean
  | null
  | string[];

export interface CreativeCanvasAgentContextNode {
  id: string;
  type: CreativeCanvasNode['type'];
  selected: boolean;
  position: { x: number; y: number };
  size: { width: number; height: number };
  groupId: string | null;
  locked: boolean;
  label: string;
  details: Record<string, CreativeCanvasAgentContextValue>;
}

export interface CreativeCanvasAgentContextConnection {
  id: string;
  sourceNodeId: string;
  targetNodeId: string;
  sourceHandle: string | null;
  targetHandle: string | null;
}

export interface CreativeCanvasAgentContextSnapshot {
  kind: typeof CREATIVE_CANVAS_AGENT_CONTEXT_KIND;
  version: typeof CREATIVE_CANVAS_AGENT_CONTEXT_VERSION;
  projectId: string;
  projectRevision: string;
  selectedNodeIds: string[];
  nodes: CreativeCanvasAgentContextNode[];
  connections: CreativeCanvasAgentContextConnection[];
  totalNodeCount: number;
  totalConnectionCount: number;
  truncated: boolean;
}

interface SummarizedNode {
  node: CreativeCanvasAgentContextNode;
  truncated: boolean;
}

const sanitizeText = (value: string): string =>
  value.replace(/\b(?:data|blob):[^\s<>"']+/giu, '[omitted-media-url]');

const boundedText = (value: string): { value: string; truncated: boolean } => {
  const sanitized = sanitizeText(value).trim();
  const chars = Array.from(sanitized);
  if (chars.length <= MAX_CREATIVE_CANVAS_AGENT_CONTEXT_TEXT_CHARS) {
    return { value: sanitized, truncated: false };
  }
  return {
    value: `${chars.slice(0, MAX_CREATIVE_CANVAS_AGENT_CONTEXT_TEXT_CHARS - 1).join('')}…`,
    truncated: true,
  };
};

const addText = (
  details: Record<string, CreativeCanvasAgentContextValue>,
  key: string,
  value: string | null | undefined
): boolean => {
  if (!value) return false;
  const bounded = boundedText(value);
  if (bounded.value) details[key] = bounded.value;
  return bounded.truncated;
};

const addModel = (
  details: Record<string, CreativeCanvasAgentContextValue>,
  providerId: string | null | undefined,
  model: string | null | undefined
): void => {
  if (providerId && model) {
    details.providerId = providerId;
    details.model = model;
  }
};

const nodeLabel = (
  node: CreativeCanvasNode,
  details: Record<string, CreativeCanvasAgentContextValue>
): string => {
  const candidate = ['title', 'caption', 'text', 'prompt']
    .map((key) => details[key])
    .find((value): value is string => typeof value === 'string' && Boolean(value));
  if (!candidate) return `${node.type} · ${node.id.slice(0, 8)}`;
  const chars = Array.from(candidate);
  return chars.length <= 48 ? candidate : `${chars.slice(0, 47).join('')}…`;
};

const summarizeNode = (node: CreativeCanvasNode, selected: boolean): SummarizedNode => {
  const details: Record<string, CreativeCanvasAgentContextValue> = {};
  let truncated = false;
  switch (node.type) {
    case 'text':
      truncated = addText(details, 'text', node.data.text) || truncated;
      details.format = node.data.format;
      break;
    case 'image':
      details.assetId = node.data.assetId;
      truncated = addText(details, 'caption', node.data.caption) || truncated;
      truncated = addText(details, 'alt', node.data.alt) || truncated;
      if (node.data.composer) {
        truncated = addText(details, 'prompt', node.data.composer.prompt) || truncated;
        addModel(
          details,
          node.data.composer.model?.providerId,
          node.data.composer.model?.model
        );
      }
      break;
    case 'panorama':
      details.assetId = node.data.assetId;
      details.projection = node.data.projection;
      break;
    case 'video':
      details.assetId = node.data.assetId;
      details.posterAssetId = node.data.posterAssetId;
      if (node.data.composer) {
        truncated = addText(details, 'prompt', node.data.composer.prompt) || truncated;
        details.resolution = node.data.composer.resolution;
        details.aspectRatio = node.data.composer.aspectRatio;
        details.seconds = node.data.composer.seconds;
        addModel(
          details,
          node.data.composer.model?.providerId,
          node.data.composer.model?.model
        );
      }
      break;
    case 'audio':
      details.assetId = node.data.assetId;
      truncated = addText(details, 'title', node.data.title) || truncated;
      if (node.data.composer) {
        truncated = addText(details, 'prompt', node.data.composer.prompt) || truncated;
        details.voice = node.data.composer.voice;
        details.format = node.data.composer.format;
        addModel(
          details,
          node.data.composer.model?.providerId,
          node.data.composer.model?.model
        );
      }
      break;
    case 'config':
      details.task = node.data.task;
      details.capability = node.data.capability;
      addModel(details, node.data.providerId, node.data.model);
      truncated = addText(details, 'prompt', node.data.prompt) || truncated;
      truncated = addText(details, 'negativePrompt', node.data.negativePrompt) || truncated;
      details.status = node.data.status;
      details.taskId = node.data.taskId;
      details.inputAssetIds = node.data.inputAssetIds.slice(0, 32);
      details.resultAssetIds = node.data.resultAssetIds.slice(0, 32);
      truncated =
        node.data.inputAssetIds.length > 32 ||
        node.data.resultAssetIds.length > 32 ||
        truncated;
      truncated = addText(details, 'errorMessage', node.data.errorMessage) || truncated;
      break;
    case 'director':
      details.sceneId = node.data.sceneId;
      details.cameraId = node.data.cameraId;
      details.timelineMs = node.data.timelineMs;
      details.durationMs = node.data.durationMs;
      break;
    case 'group':
      truncated = addText(details, 'title', node.data.title) || truncated;
      details.collapsed = node.data.collapsed;
      break;
  }
  return {
    node: {
      id: node.id,
      type: node.type,
      selected,
      position: { ...node.position },
      size: { ...node.size },
      groupId: node.groupId,
      locked: node.locked,
      label: nodeLabel(node, details),
      details,
    },
    truncated,
  };
};

const orderedKnownIds = (
  ids: Iterable<string>,
  nodeIndex: ReadonlyMap<string, number>
): string[] =>
  [...new Set(ids)]
    .filter((id) => nodeIndex.has(id))
    .sort((left, right) => nodeIndex.get(left)! - nodeIndex.get(right)!);

const referencedNodeIds = (
  document: CreativeProjectDocument,
  selectedIds: readonly string[]
): string[] => {
  const selected = new Set(selectedIds);
  const referenced = new Set<string>();
  for (const connection of document.connections) {
    if (selected.has(connection.sourceNodeId)) referenced.add(connection.targetNodeId);
    if (selected.has(connection.targetNodeId)) referenced.add(connection.sourceNodeId);
  }
  for (const node of document.nodes) {
    if (!selected.has(node.id)) continue;
    if (node.groupId) referenced.add(node.groupId);
    if (node.type === 'group') {
      for (const child of document.nodes) {
        if (child.groupId === node.id) referenced.add(child.id);
      }
    }
    if (node.type === 'config' && node.data.operation) {
      referenced.add(node.data.operation.sourceNodeId);
    }
  }
  for (const id of selected) referenced.delete(id);
  return [...referenced];
};

const contextConnection = (
  connection: CreativeCanvasConnection
): CreativeCanvasAgentContextConnection => ({
  id: connection.id,
  sourceNodeId: connection.sourceNodeId,
  targetNodeId: connection.targetNodeId,
  sourceHandle: connection.sourceHandle,
  targetHandle: connection.targetHandle,
});

export function buildCreativeCanvasAgentContext(input: {
  document: CreativeProjectDocument;
  projectRevision: string;
  selectedNodeIds: readonly string[];
}): CreativeCanvasAgentContextSnapshot {
  const nodeIndex = new Map(input.document.nodes.map((node, index) => [node.id, index]));
  const selected = orderedKnownIds(input.selectedNodeIds, nodeIndex);
  const neighbors = orderedKnownIds(referencedNodeIds(input.document, selected), nodeIndex);
  const candidates = [...selected, ...neighbors];
  const includedIds = candidates.slice(0, MAX_CREATIVE_CANVAS_AGENT_CONTEXT_NODES);
  const included = new Set(includedIds);
  const selectedSet = new Set(selected);
  let truncated = candidates.length > includedIds.length;
  const nodes = includedIds.map((id) => {
    const summarized = summarizeNode(input.document.nodes[nodeIndex.get(id)!]!, selectedSet.has(id));
    truncated = summarized.truncated || truncated;
    return summarized.node;
  });
  const relevantConnections = input.document.connections.filter(
    (connection) => included.has(connection.sourceNodeId) && included.has(connection.targetNodeId)
  );
  if (relevantConnections.length > MAX_CREATIVE_CANVAS_AGENT_CONTEXT_CONNECTIONS) {
    truncated = true;
  }
  return {
    kind: CREATIVE_CANVAS_AGENT_CONTEXT_KIND,
    version: CREATIVE_CANVAS_AGENT_CONTEXT_VERSION,
    projectId: input.document.projectId,
    projectRevision: input.projectRevision,
    selectedNodeIds: selected.filter((id) => included.has(id)),
    nodes,
    connections: relevantConnections
      .slice(0, MAX_CREATIVE_CANVAS_AGENT_CONTEXT_CONNECTIONS)
      .map(contextConnection),
    totalNodeCount: input.document.nodes.length,
    totalConnectionCount: input.document.connections.length,
    truncated,
  };
}

export function serializeCreativeCanvasAgentModelInput(input: {
  prompt: string;
  context: CreativeCanvasAgentContextSnapshot;
  skillIds: readonly string[];
}): string {
  const prompt = input.prompt.trim();
  if (!prompt) throw new Error('Creative Studio Agent prompt must be non-empty');
  const serialized = JSON.stringify({
    kind: CREATIVE_CANVAS_AGENT_TURN_KIND,
    version: 1,
    userRequest: prompt,
    selectedSkills: [...input.skillIds],
    canvasContext: input.context,
    responseContract: {
      mode: 'plan-and-propose',
      allowedArtifactKinds: ['nomifun.creative-studio.canvas-ops/v1'],
      requiresUserApproval: true,
      forbiddenActions: ['delete-node', 'media-generation'],
    },
  });
  if (Array.from(serialized).length > MAX_CREATIVE_CANVAS_AGENT_MODEL_INPUT_CHARS) {
    throw new Error('Creative Studio Agent model input exceeds the durable planning limit');
  }
  return serialized;
}

/** Apply the user's composer exclusions without rebuilding or widening the snapshot. */
export function selectCreativeCanvasAgentContextNodes(
  context: CreativeCanvasAgentContextSnapshot,
  includedNodeIds: readonly string[]
): CreativeCanvasAgentContextSnapshot {
  const requested = new Set(includedNodeIds);
  const nodes = context.nodes.filter((node) => requested.has(node.id));
  const included = new Set(nodes.map((node) => node.id));
  return {
    ...context,
    selectedNodeIds: context.selectedNodeIds.filter((id) => included.has(id)),
    nodes,
    connections: context.connections.filter(
      (connection) =>
        included.has(connection.sourceNodeId) && included.has(connection.targetNodeId)
    ),
  };
}
