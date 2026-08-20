/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/config/storage';

/** Canonical persistence marker for the rebuilt Creative Studio product. */
export const CREATIVE_STUDIO_DOCUMENT_SCHEMA = 'nomifun.creative-studio/v1' as const;

/** New documents never share a schema or a fallback reader with the retired workshop. */
export type CreativeStudioDocumentSchema = typeof CREATIVE_STUDIO_DOCUMENT_SCHEMA;

export type CreativeCanvasBackground = 'dots' | 'lines' | 'blank';

export type CreativeCanvasNodeKind =
  | 'image'
  | 'panorama'
  | 'text'
  | 'config'
  | 'video'
  | 'audio'
  | 'director'
  | 'group';

export interface CreativePoint {
  x: number;
  y: number;
}

export interface CreativeSize {
  width: number;
  height: number;
}

export interface CreativeViewport extends CreativePoint {
  zoom: number;
}

export type CreativeModelTask = Extract<
  ModelTask,
  'chat' | 'image_generation' | 'image_edit' | 'video_generation' | 'speech_synthesis'
>;

export type CreativeGenerationStatus =
  | 'idle'
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'canceled';

export type CreativeJsonPrimitive = string | number | boolean | null;
export type CreativeJsonValue =
  | CreativeJsonPrimitive
  | CreativeJsonValue[]
  | { [key: string]: CreativeJsonValue };
export type CreativeJsonObject = { [key: string]: CreativeJsonValue };

export interface CreativeImageNodeData {
  assetId: string | null;
  caption: string;
  alt: string;
  fit: 'contain' | 'cover';
  naturalSize: CreativeSize | null;
}

export interface CreativePanoramaNodeData {
  assetId: string | null;
  projection: 'equirectangular';
  yaw: number;
  pitch: number;
  fieldOfView: number;
}

export interface CreativeTextNodeData {
  text: string;
  format: 'plain' | 'markdown';
  fontSize: number;
  textAlign: 'left' | 'center' | 'right';
}

/**
 * A persisted generation configuration. Provider/model identity is explicit;
 * the UI must not derive either value from a display name or another task.
 */
export interface CreativeConfigNodeData {
  task: CreativeModelTask;
  capability: string;
  providerId: string | null;
  model: string | null;
  prompt: string;
  negativePrompt: string;
  parameters: CreativeJsonObject;
  inputAssetIds: string[];
  taskId: string | null;
  resultAssetIds: string[];
  status: CreativeGenerationStatus;
  errorMessage: string | null;
}

export interface CreativeVideoNodeData {
  assetId: string | null;
  posterAssetId: string | null;
  autoplay: boolean;
  loop: boolean;
  muted: boolean;
  trimStartMs: number;
  trimEndMs: number | null;
}

export interface CreativeAudioNodeData {
  assetId: string | null;
  title: string;
  loop: boolean;
  volume: number;
  trimStartMs: number;
  trimEndMs: number | null;
}

export interface CreativeDirectorNodeData {
  sceneId: string | null;
  cameraId: string | null;
  timelineMs: number;
  durationMs: number;
}

export interface CreativeGroupNodeData {
  title: string;
  color: string | null;
  collapsed: boolean;
}

export interface CreativeCanvasNodeDataByKind {
  image: CreativeImageNodeData;
  panorama: CreativePanoramaNodeData;
  text: CreativeTextNodeData;
  config: CreativeConfigNodeData;
  video: CreativeVideoNodeData;
  audio: CreativeAudioNodeData;
  director: CreativeDirectorNodeData;
  group: CreativeGroupNodeData;
}

export interface CreativeCanvasNodeBase<K extends CreativeCanvasNodeKind> {
  id: string;
  type: K;
  position: CreativePoint;
  size: CreativeSize;
  groupId: string | null;
  zIndex: number;
  locked: boolean;
  data: CreativeCanvasNodeDataByKind[K];
}

export type CreativeCanvasNode = {
  [K in CreativeCanvasNodeKind]: CreativeCanvasNodeBase<K>;
}[CreativeCanvasNodeKind];

export interface CreativeCanvasConnection {
  id: string;
  sourceNodeId: string;
  targetNodeId: string;
  sourceHandle: string | null;
  targetHandle: string | null;
}

/** Conversation content remains owned by NomiFun; the canvas stores stable references only. */
export interface CreativeChatSessionReference {
  id: string;
  title: string;
  messageIds: string[];
  createdAt: number;
  updatedAt: number;
}

export type CreativeLeftPanelView = 'canvas' | 'assets' | 'prompts' | 'workflows';
export type CreativeRightPanelView = 'assistant' | 'properties';
export type CreativeBottomPanelView = 'timeline' | 'history';

export interface CreativeStudioPanelState {
  left: {
    open: boolean;
    width: number;
    activeView: CreativeLeftPanelView;
  };
  right: {
    open: boolean;
    width: number;
    activeView: CreativeRightPanelView;
  };
  bottom: {
    open: boolean;
    height: number;
    activeView: CreativeBottomPanelView;
  };
}

/** Complete, versioned persistence unit for one Creative Studio project. */
export interface CreativeProjectDocument {
  schema: CreativeStudioDocumentSchema;
  projectId: string;
  viewport: CreativeViewport;
  background: CreativeCanvasBackground;
  nodes: CreativeCanvasNode[];
  connections: CreativeCanvasConnection[];
  chatSessions: CreativeChatSessionReference[];
  activeChatId: string | null;
  panels: CreativeStudioPanelState;
  pendingTaskIds: string[];
}

export interface CreativeProjectSummary {
  projectId: string;
  title: string;
  /** Decimal, monotonic document revision used for compare-and-swap saves. */
  revision: string;
  nodeCount: number;
  connectionCount: number;
  /** Unix epoch milliseconds. */
  createdAt: number;
  /** Unix epoch milliseconds. */
  updatedAt: number;
}

export interface CreativeProjectDetail {
  project: CreativeProjectSummary;
  document: CreativeProjectDocument;
}

export interface CreateCreativeProjectRequest {
  title?: string;
}

export interface RenameCreativeProjectRequest {
  title: string;
}

export interface SaveCreativeProjectRequest {
  expectedRevision: string;
  document: CreativeProjectDocument;
}

export interface CreativeProjectListResponse {
  projects: CreativeProjectSummary[];
}

export interface CreativeProjectResponse {
  project: CreativeProjectSummary;
}

export interface CreativeProjectDetailResponse extends CreativeProjectDetail {}

export type SaveCreativeProjectResponse = CreativeProjectResponse;

export const DEFAULT_CREATIVE_STUDIO_PANELS: CreativeStudioPanelState = {
  left: { open: true, width: 288, activeView: 'canvas' },
  right: { open: true, width: 340, activeView: 'assistant' },
  bottom: { open: false, height: 240, activeView: 'history' },
};

/** Build the only supported empty-document shape. */
export function createEmptyCreativeProjectDocument(projectId: string): CreativeProjectDocument {
  return {
    schema: CREATIVE_STUDIO_DOCUMENT_SCHEMA,
    projectId,
    viewport: { x: 0, y: 0, zoom: 1 },
    background: 'lines',
    nodes: [],
    connections: [],
    chatSessions: [],
    activeChatId: null,
    panels: {
      left: { ...DEFAULT_CREATIVE_STUDIO_PANELS.left },
      right: { ...DEFAULT_CREATIVE_STUDIO_PANELS.right },
      bottom: { ...DEFAULT_CREATIVE_STUDIO_PANELS.bottom },
    },
    pendingTaskIds: [],
  };
}
