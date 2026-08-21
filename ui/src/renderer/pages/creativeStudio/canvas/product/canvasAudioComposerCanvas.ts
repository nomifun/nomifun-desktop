/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from '../../assets';
import type {
  CreativeCanvasConnection,
  CreativeCanvasNode,
  CreativeGenerationStatus,
  CreativeProjectDocument,
  CreativeSize,
} from '../../domain';
import type {
  CreativeModelCatalogSnapshot,
  CreativeModelOption,
  CreativeModelSelectionRef,
} from '../../models';
import {
  isCanvasNodeTaskOwner,
  type CreativeTask,
  type CreativeTaskReference,
} from '../../tasks';
import type { AudioWorkbenchFieldSupport } from '../../workbenches/audio';
import {
  prepareAudioWorkbenchRun,
  resolveExactWorkbenchModel,
  workbenchResumeRequestsFromDocument,
  type CreativeWorkbenchReferences,
  type CreativeWorkbenchResumeRequest,
  type PreparedCreativeWorkbenchRun,
} from '../../workbenches/runtime';
import { validateCanvasConnection, type CanvasState } from '../core';
import { nextCanvasImageTaskPosition } from './imageTaskCanvasLayout';
import {
  createCreativeCanvasProductNode,
  CREATIVE_CANVAS_PRODUCT_NODE_SIZES,
} from './nodeFactory';

export const CREATIVE_AUDIO_COMPOSE_OPERATION = 'audio-node-compose';

type AudioNode = Extract<CreativeCanvasNode, { type: 'audio' }>;
type ConfigNode = Extract<CreativeCanvasNode, { type: 'config' }>;

export type CanvasAudioComposeFormat = 'mp3' | 'wav';

export interface CanvasAudioComposeSettings {
  model: CreativeModelSelectionRef | null;
  voice: string;
  format: CanvasAudioComposeFormat;
}

export interface CanvasAudioComposeDraft {
  prompt: string;
  settings: CanvasAudioComposeSettings;
}

export interface CanvasAudioComposeTaskSummary {
  state: CreativeGenerationStatus;
  pendingCount: number;
  message?: string;
}

export interface PreparedCanvasAudioCompose {
  configNode: ConfigNode;
  connection: Omit<CreativeCanvasConnection, 'id'>;
  plan: PreparedCreativeWorkbenchRun;
}

export const DEFAULT_CANVAS_AUDIO_COMPOSE_SETTINGS: CanvasAudioComposeSettings = {
  model: null,
  voice: '',
  format: 'mp3',
};

export const DEFAULT_CANVAS_AUDIO_COMPOSE_DRAFT: CanvasAudioComposeDraft = {
  prompt: '',
  settings: structuredClone(DEFAULT_CANVAS_AUDIO_COMPOSE_SETTINGS),
};

export interface CanvasAudioComposeProtocolProfile {
  fieldSupport: AudioWorkbenchFieldSupport;
  voiceRequired: boolean;
  maxTextLength: number;
}

const fields = (
  voice: boolean,
  format: boolean
): AudioWorkbenchFieldSupport => ({
  voice,
  format,
  speed: false,
  instructions: false,
  references: false,
});

const profile = (
  voice: boolean,
  format: boolean,
  voiceRequired = false,
  maxTextLength = 4_096
): CanvasAudioComposeProtocolProfile => ({
  fieldSupport: fields(voice, format),
  voiceRequired,
  maxTextLength,
});

const MINIMAL_AUDIO_COMPOSE_PROFILE = profile(false, false);

/**
 * Exact protocol profiles verified against the NomiFun invocation adapters.
 * Unknown protocols deliberately receive prompt-only requests. The first
 * canvas slice never sends speed, instructions, or reference audio.
 */
export const CANVAS_AUDIO_COMPOSE_PROTOCOL_PROFILES: Readonly<
  Record<string, CanvasAudioComposeProtocolProfile>
> = {
  'openai.audio_speech': profile(true, true),
  'deepgram.speak_rest': profile(false, true, false, 2_000),
  'minimax.t2a': profile(true, true),
  'mimo.chat_tts': profile(true, true, true),
  'siliconflow.audio_speech': profile(true, true, true),
  'stepfun.audio_speech': profile(true, true, true),
  'volc.tts_v3': profile(true, false, true),
  'xai.tts': profile(true, true),
};

export function canvasAudioComposeProtocolProfile(
  protocol: string
): CanvasAudioComposeProtocolProfile {
  const selected =
    CANVAS_AUDIO_COMPOSE_PROTOCOL_PROFILES[protocol] ??
    MINIMAL_AUDIO_COMPOSE_PROFILE;
  return {
    ...selected,
    fieldSupport: { ...selected.fieldSupport },
  };
}

export function canvasAudioComposeFieldSupport(
  protocol: string
): AudioWorkbenchFieldSupport {
  return canvasAudioComposeProtocolProfile(protocol).fieldSupport;
}

export const canvasAudioComposeVoiceRequired = (protocol: string): boolean =>
  canvasAudioComposeProtocolProfile(protocol).voiceRequired;

export const canvasAudioComposeMaxTextLength = (protocol: string): number =>
  canvasAudioComposeProtocolProfile(protocol).maxTextLength;

/** Voice IDs are protocol/provider scoped and must never cross that boundary. */
export function canvasAudioComposeVoiceAfterModelChange(
  current: Pick<CreativeModelOption, 'providerId' | 'protocol'> | null,
  next: Pick<CreativeModelOption, 'providerId' | 'protocol'> | null,
  voice: string
): string {
  return current &&
    next &&
    current.providerId === next.providerId &&
    current.protocol === next.protocol
    ? voice
    : '';
}

const modelSelection = (
  providerId: string | null,
  model: string | null
): CreativeModelSelectionRef | null =>
  providerId && model
    ? {
        providerId: providerId as CreativeModelSelectionRef['providerId'],
        model,
      }
    : null;

const composeFormat = (value: unknown): CanvasAudioComposeFormat =>
  value === 'wav' ? 'wav' : 'mp3';

export function canvasAudioComposeSettings(
  config: ConfigNode | null
): CanvasAudioComposeSettings {
  if (!config) return structuredClone(DEFAULT_CANVAS_AUDIO_COMPOSE_SETTINGS);
  const voice = config.data.parameters.voice;
  return {
    model: modelSelection(config.data.providerId, config.data.model),
    voice: typeof voice === 'string' ? voice : '',
    format: composeFormat(config.data.parameters.format),
  };
}

/** Restore the node-owned draft, falling back to the latest submitted config. */
export function canvasAudioComposeDraftFromState(
  state: CanvasState,
  nodeId: string
): CanvasAudioComposeDraft {
  const source = state.document.nodes.find(
    (node): node is AudioNode => node.id === nodeId && node.type === 'audio'
  );
  const persisted = source?.data.composer;
  if (persisted) {
    return {
      prompt: persisted.prompt,
      settings: {
        model: persisted.model
          ? {
              providerId:
                persisted.model.providerId as CreativeModelSelectionRef['providerId'],
              model: persisted.model.model,
            }
          : null,
        voice: persisted.voice,
        format: persisted.format,
      },
    };
  }
  const config = latestCanvasAudioComposeConfig(state.document, nodeId);
  return {
    prompt: config?.data.prompt ?? DEFAULT_CANVAS_AUDIO_COMPOSE_DRAFT.prompt,
    settings: config
      ? canvasAudioComposeSettings(config)
      : structuredClone(DEFAULT_CANVAS_AUDIO_COMPOSE_SETTINGS),
  };
}

/** Replace the complete durable composer draft without changing node identity. */
export function withCanvasAudioComposeDraft(
  node: AudioNode,
  draft: CanvasAudioComposeDraft
): AudioNode {
  return {
    ...node,
    data: {
      ...node.data,
      composer: {
        prompt: draft.prompt,
        model: draft.settings.model ? { ...draft.settings.model } : null,
        voice: draft.settings.voice,
        format: draft.settings.format,
      },
    },
  };
}

export function canvasAudioComposeTaskSummary(
  config: ConfigNode | null
): CanvasAudioComposeTaskSummary {
  if (!config) return { state: 'idle', pendingCount: 0 };
  const pending =
    config.data.status === 'queued' || config.data.status === 'running';
  return {
    state: config.data.status,
    pendingCount: pending ? 1 : 0,
    message: config.data.errorMessage ?? undefined,
  };
}

export function isCanvasAudioComposeConfig(
  node: CreativeCanvasNode | undefined
): node is ConfigNode {
  return Boolean(
    node?.type === 'config' &&
      node.data.task === 'speech_synthesis' &&
      node.data.capability === 'tts' &&
      node.data.operation?.kind === CREATIVE_AUDIO_COMPOSE_OPERATION &&
      typeof node.data.operation.sourceNodeId === 'string' &&
      Boolean(node.data.operation.sourceNodeId.trim()) &&
      node.data.operation.sourceAssetId === null &&
      node.data.inputAssetIds.length === 0
  );
}

export function canvasAudioComposeSourceNodeId(node: ConfigNode): string {
  const value = node.data.operation?.sourceNodeId;
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error('音频创作配置缺少 sourceNodeId。');
  }
  return value;
}

export function canvasAudioComposeSourceAssetId(
  node: ConfigNode
): string | null {
  const value = node.data.operation?.sourceAssetId;
  if (value === null) return null;
  if (typeof value !== 'string' || !value.trim()) {
    throw new Error('音频创作配置包含无效的 sourceAssetId。');
  }
  return value;
}

export function latestCanvasAudioComposeConfig(
  document: Pick<CreativeProjectDocument, 'nodes'>,
  sourceNodeId: string
): ConfigNode | null {
  const matches = document.nodes.filter(
    (node): node is ConfigNode =>
      isCanvasAudioComposeConfig(node) &&
      canvasAudioComposeSourceNodeId(node) === sourceNodeId
  );
  return matches.at(-1) ?? null;
}

const incomingMediaAssetIds = (
  document: Pick<CreativeProjectDocument, 'nodes' | 'connections'>,
  targetNodeId: string
): string[] => {
  const sourceIds = new Set(
    document.connections
      .filter((connection) => connection.targetNodeId === targetNodeId)
      .map((connection) => connection.sourceNodeId)
  );
  return document.nodes.flatMap((node) => {
    if (!sourceIds.has(node.id)) return [];
    if (
      node.type === 'image' ||
      node.type === 'panorama' ||
      node.type === 'video' ||
      node.type === 'audio'
    ) {
      return node.data.assetId ? [node.data.assetId] : [];
    }
    return [];
  });
};

export type CanvasAudioComposeEligibility =
  | { kind: 'tts' }
  | { kind: 'unsupported'; message: string };

/** Project the honest first-slice TTS mode before any config/task is created. */
export function canvasAudioComposeEligibility(
  document: Pick<CreativeProjectDocument, 'nodes' | 'connections'>,
  sourceNodeId: string
): CanvasAudioComposeEligibility {
  const source = document.nodes.find(
    (node): node is AudioNode =>
      node.id === sourceNodeId && node.type === 'audio'
  );
  if (!source) {
    return { kind: 'unsupported', message: '音频节点已不存在。' };
  }
  if (source.data.assetId !== null) {
    return {
      kind: 'unsupported',
      message: '当前首批音频创作只支持空音频节点上的 TTS。',
    };
  }
  if (incomingMediaAssetIds(document, sourceNodeId).length > 0) {
    return {
      kind: 'unsupported',
      message: '当前 TTS 音频创作不接受图片、视频或音频参考素材。',
    };
  }
  return { kind: 'tts' };
}

export function prepareCanvasAudioCompose(input: {
  projectId: string;
  state: CanvasState;
  viewportSize: CreativeSize;
  sourceNode: AudioNode;
  sourceAsset: CreativeAsset | null;
  catalog: CreativeModelCatalogSnapshot;
  model: CreativeModelSelectionRef;
  references: CreativeWorkbenchReferences;
  prompt: string;
  settings: Omit<CanvasAudioComposeSettings, 'model'>;
}): PreparedCanvasAudioCompose {
  const eligibility = canvasAudioComposeEligibility(
    input.state.document,
    input.sourceNode.id
  );
  if (eligibility.kind === 'unsupported') throw new Error(eligibility.message);
  if (input.sourceAsset !== null) {
    throw new Error('当前音频创作不能绑定源素材。');
  }
  if (
    input.references.assets.length > 0 ||
    input.references.bindings.length > 0
  ) {
    throw new Error('当前 TTS 音频创作不接受参考素材。');
  }
  if (input.settings.format !== 'mp3' && input.settings.format !== 'wav') {
    throw new Error('当前音频创作只支持 mp3 或 wav 格式。');
  }

  const resolved = resolveExactWorkbenchModel(
    input.catalog,
    input.model,
    'speech_synthesis'
  );
  const protocolProfile = canvasAudioComposeProtocolProfile(resolved.protocol);
  const voice = input.settings.voice.trim();
  if (protocolProfile.voiceRequired && !voice) {
    throw new Error(`调用协议 ${resolved.protocol} 需要选择非空音色。`);
  }
  const configPosition = nextCanvasImageTaskPosition(
    input.state.document.nodes,
    input.sourceNode,
    CREATIVE_CANVAS_PRODUCT_NODE_SIZES.config
  );
  const base = createCreativeCanvasProductNode(
    'config',
    input.state,
    input.viewportSize,
    { position: configPosition, locked: true }
  );
  const plan = prepareAudioWorkbenchRun({
    catalog: input.catalog,
    projectId: input.projectId,
    nodeId: base.id,
    model: input.model,
    references: { assets: [], bindings: [] },
    value: {
      text: input.prompt,
      instructions: '',
      voice,
      format: input.settings.format,
      speed: 1,
      model: { ...input.model },
    },
    fieldSupport: protocolProfile.fieldSupport,
    maxTextLength: protocolProfile.maxTextLength,
  });
  const configNode: ConfigNode = {
    ...base,
    data: {
      ...base.data,
      operation: {
        kind: CREATIVE_AUDIO_COMPOSE_OPERATION,
        sourceNodeId: input.sourceNode.id,
        sourceAssetId: null,
      },
      task: plan.input.task,
      capability: plan.input.capability,
      providerId: plan.model.providerId,
      model: plan.model.model,
      prompt: input.prompt.trim(),
      parameters: structuredClone(plan.input.parameters),
      inputAssetIds: [],
      taskId: plan.input.idempotencyKey,
      resultAssetIds: [],
      status: 'queued',
      errorMessage: null,
    },
  };
  const connection = {
    sourceNodeId: input.sourceNode.id,
    targetNodeId: configNode.id,
    sourceHandle: 'source',
    targetHandle: 'target',
  };
  const validation = validateCanvasConnection(
    {
      ...input.state.document,
      nodes: [...input.state.document.nodes, configNode],
    },
    connection
  );
  if (!validation.ok) {
    throw new Error(`无法连接音频创作配置节点：${validation.code}。`);
  }
  return { configNode, connection, plan };
}

export function canvasAudioComposeResumeRequests(
  document: CreativeProjectDocument
): CreativeWorkbenchResumeRequest[] {
  const owners = new Set(
    document.nodes.filter(isCanvasAudioComposeConfig).map((node) => node.id)
  );
  return workbenchResumeRequestsFromDocument(document).filter(
    (request) =>
      request.reference.owner.kind === 'canvas_node' &&
      owners.has(request.reference.owner.nodeId)
  );
}

export function canvasAudioComposeConfigForReference(
  document: Pick<CreativeProjectDocument, 'projectId' | 'nodes'>,
  reference: CreativeTaskReference
): ConfigNode {
  if (
    !isCanvasNodeTaskOwner(reference.owner) ||
    reference.owner.projectId !== document.projectId
  ) {
    throw new Error('音频创作任务不属于当前画布项目。');
  }
  const owner = reference.owner;
  const node = document.nodes.find((candidate) => candidate.id === owner.nodeId);
  if (!isCanvasAudioComposeConfig(node)) {
    throw new Error('音频创作任务缺少 canonical 配置节点。');
  }
  if (
    node.data.taskId !== reference.taskId ||
    node.data.providerId !== reference.providerId ||
    node.data.model !== reference.model ||
    node.data.task !== reference.task ||
    node.data.capability !== reference.capability
  ) {
    throw new Error('音频创作任务与配置节点身份不一致。');
  }
  return node;
}

export function canvasAudioComposeConfigFromTask(
  document: Pick<CreativeProjectDocument, 'projectId' | 'nodes'>,
  task: CreativeTask
): ConfigNode {
  return canvasAudioComposeConfigForReference(document, {
    taskId: task.taskId,
    owner: task.owner,
    providerId: task.providerId,
    model: task.model,
    task: task.task,
    capability: task.capability,
  });
}

export function reconcileCanvasAudioComposeConfig(
  node: ConfigNode,
  task: CreativeTask
): ConfigNode {
  const terminal =
    task.status === 'succeeded' ||
    task.status === 'failed' ||
    task.status === 'canceled';
  return {
    ...node,
    locked: !terminal,
    data: {
      ...node.data,
      taskId: task.taskId,
      resultAssetIds:
        task.status === 'succeeded' ? [...task.resultAssetIds] : [],
      status: task.status,
      errorMessage:
        task.status === 'failed'
          ? (task.error?.message ?? '音频创作失败。')
          : task.status === 'canceled'
            ? '音频创作已取消。'
            : null,
    },
  };
}
