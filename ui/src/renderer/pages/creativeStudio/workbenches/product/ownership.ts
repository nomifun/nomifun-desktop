/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { CANONICAL_UUID_V7 } from '@/common/types/ids';

import { makeCanvasNode } from '../../canvas/core';
import type {
  CreativeCanvasNode,
  CreativeConfigNodeData,
  CreativeGenerationStatus,
  CreativeProjectDetail,
  CreativeProjectDocument,
} from '../../domain';
import {
  creativeProjectRepository,
  isCreativeProjectRepositoryError,
  type CreativeProjectRepository,
} from '../../services';
import {
  isCanvasNodeTaskOwner,
  type CreativeTask,
  type CreativeTaskReference,
} from '../../tasks';
import {
  workbenchResumeRequestsFromDocument,
  type CreativeWorkbenchResumeRequest,
} from '../runtime';

export type StandaloneWorkbenchKind = 'image' | 'video';

export const STANDALONE_WORKBENCH_MARKER = 'nomifunStandaloneWorkbench';
export const STANDALONE_VIDEO_MAX_CONCURRENT_TASKS = 1;

export type StandaloneProjectQuery =
  | { state: 'missing'; projectId: null }
  | { state: 'invalid'; projectId: null; message: string }
  | { state: 'valid'; projectId: string };

export interface StandaloneWorkbenchDraft {
  task: CreativeConfigNodeData['task'];
  capability: string;
  prompt: string;
  providerId: string;
  model: string;
  parameters: CreativeConfigNodeData['parameters'];
  inputAssetIds: string[];
}

type ConfigNode = Extract<CreativeCanvasNode, { type: 'config' }>;

export class StandaloneWorkbenchOwnershipError extends Error {
  constructor(
    readonly code:
      | 'invalid-project-query'
      | 'ambiguous-owner'
      | 'owner-missing'
      | 'owner-mismatch'
      | 'revision-conflict',
    message: string
  ) {
    super(message);
    this.name = 'StandaloneWorkbenchOwnershipError';
  }
}

function assertKindContract(
  kind: StandaloneWorkbenchKind,
  task: CreativeConfigNodeData['task'],
  capability: string
): void {
  const valid =
    kind === 'video'
      ? task === 'video_generation' &&
        (capability === 't2v' || capability === 'i2v' || capability === 'v2v')
      : (task === 'image_generation' && capability === 't2i') ||
        (task === 'image_edit' && capability === 'i2i');
  if (!valid) {
    throw new StandaloneWorkbenchOwnershipError(
      'owner-mismatch',
      `${kind} 独立工作台不接受 ${task}/${capability} 任务归属。`
    );
  }
}

const markerOf = (node: ConfigNode): unknown => node.data.parameters[STANDALONE_WORKBENCH_MARKER];

export function parseStandaloneProjectQuery(search: string): StandaloneProjectQuery {
  const params = new URLSearchParams(search);
  const values = params.getAll('projectId');
  if (values.length === 0) return { state: 'missing', projectId: null };
  if (values.length !== 1 || !CANONICAL_UUID_V7.test(values[0] ?? '')) {
    return {
      state: 'invalid',
      projectId: null,
      message: 'projectId 必须是唯一、规范的小写 UUIDv7。',
    };
  }
  return { state: 'valid', projectId: values[0] as string };
}

export function standaloneProjectSearch(search: string, projectId: string | null): string {
  const params = new URLSearchParams(search);
  params.delete('projectId');
  if (projectId) params.set('projectId', projectId);
  const encoded = params.toString();
  return encoded ? `?${encoded}` : '';
}

export function findStandaloneWorkbenchNode(
  document: Pick<CreativeProjectDocument, 'nodes'>,
  kind: StandaloneWorkbenchKind
): ConfigNode | null {
  const matches = document.nodes.filter(
    (node): node is ConfigNode =>
      node.type === 'config' && markerOf(node) === kind
  );
  if (matches.length > 1) {
    throw new StandaloneWorkbenchOwnershipError(
      'ambiguous-owner',
      `项目中存在 ${matches.length} 个 ${kind} 独立工作台归属节点，已拒绝猜测。`
    );
  }
  return matches[0] ?? null;
}

function createStandaloneWorkbenchNode(
  document: CreativeProjectDocument,
  kind: StandaloneWorkbenchKind,
  draft: StandaloneWorkbenchDraft
): ConfigNode {
  assertKindContract(kind, draft.task, draft.capability);
  const highestZ = document.nodes.reduce((value, node) => Math.max(value, node.zIndex), -1);
  const slot = document.nodes.length % 8;
  const zoom = Math.max(0.1, document.viewport.zoom);
  return makeCanvasNode({
    type: 'config',
    position: {
      x: (64 - document.viewport.x) / zoom + slot * 28,
      y: (64 - document.viewport.y) / zoom + slot * 28,
    },
    size: { width: 340, height: 240 },
    zIndex: highestZ + 1,
    data: {
      task: draft.task,
      capability: draft.capability,
      providerId: draft.providerId,
      model: draft.model,
      prompt: draft.prompt,
      negativePrompt: '',
      operation: null,
      parameters: {
        ...structuredClone(draft.parameters),
        [STANDALONE_WORKBENCH_MARKER]: kind,
      },
      inputAssetIds: [...draft.inputAssetIds],
      taskId: null,
      resultAssetIds: [],
      status: 'idle',
      errorMessage: null,
    },
  }) as ConfigNode;
}

function cloneDocument(document: CreativeProjectDocument): CreativeProjectDocument {
  return structuredClone(document);
}

function replaceConfigNode(
  document: CreativeProjectDocument,
  nodeId: string,
  data: CreativeConfigNodeData
): CreativeProjectDocument {
  return {
    ...document,
    nodes: document.nodes.map((node) =>
      node.id === nodeId && node.type === 'config'
        ? { ...node, data: structuredClone(data) }
        : node
    ),
  };
}

interface MutationResult<T> {
  document: CreativeProjectDocument;
  value: T;
  changed: boolean;
}

async function mutateProject<T>(
  projectId: string,
  mutation: (detail: CreativeProjectDetail) => MutationResult<T>,
  repository: CreativeProjectRepository,
  signal?: AbortSignal
): Promise<T> {
  for (let attempt = 0; attempt < 4; attempt += 1) {
    if (signal?.aborted) throw new DOMException('Operation aborted', 'AbortError');
    const detail = await repository.load(projectId);
    if (signal?.aborted) throw new DOMException('Operation aborted', 'AbortError');
    const next = mutation(detail);
    if (!next.changed) return next.value;
    try {
      await repository.save(projectId, detail.project.revision, next.document);
      return next.value;
    } catch (error) {
      if (
        !isCreativeProjectRepositoryError(error) ||
        error.kind !== 'revision-conflict'
      ) {
        throw error;
      }
    }
  }
  throw new StandaloneWorkbenchOwnershipError(
    'revision-conflict',
    '项目在多次保存期间持续变化，未覆盖其他窗口的修改。请重试。'
  );
}

function assertReferenceOwner(
  node: ConfigNode | null,
  kind: StandaloneWorkbenchKind,
  reference: Pick<CreativeTaskReference, 'owner' | 'task' | 'capability'>
): ConfigNode {
  if (!node) {
    throw new StandaloneWorkbenchOwnershipError(
      'owner-missing',
      '任务提交前必须先创建真实的独立工作台配置节点。'
    );
  }
  assertKindContract(kind, reference.task, reference.capability);
  if (
    !isCanvasNodeTaskOwner(reference.owner) ||
    node.id !== reference.owner.nodeId
  ) {
    throw new StandaloneWorkbenchOwnershipError(
      'owner-mismatch',
      '任务身份与独立工作台配置节点不一致，已拒绝提交。'
    );
  }
  return node;
}

export async function ensureStandaloneWorkbenchNode(
  projectId: string,
  kind: StandaloneWorkbenchKind,
  draft: StandaloneWorkbenchDraft,
  repository: CreativeProjectRepository = creativeProjectRepository,
  signal?: AbortSignal
): Promise<ConfigNode> {
  return mutateProject(
    projectId,
    (detail) => {
      const current = findStandaloneWorkbenchNode(detail.document, kind);
      if (!current) {
        const node = createStandaloneWorkbenchNode(detail.document, kind, draft);
        return {
          document: { ...cloneDocument(detail.document), nodes: [...detail.document.nodes, node] },
          value: node,
          changed: true,
        };
      }
      if (current.data.taskId && detail.document.pendingTaskIds.includes(current.data.taskId)) {
        throw new StandaloneWorkbenchOwnershipError(
          'owner-mismatch',
          '当前独立工作台仍有未完成任务，不能覆盖其归属。'
        );
      }
      assertKindContract(kind, draft.task, draft.capability);
      const data: CreativeConfigNodeData = {
        ...current.data,
        task: draft.task,
        capability: draft.capability,
        providerId: draft.providerId,
        model: draft.model,
        prompt: draft.prompt,
        parameters: {
          ...structuredClone(draft.parameters),
          [STANDALONE_WORKBENCH_MARKER]: kind,
        },
        inputAssetIds: [...draft.inputAssetIds],
        status: 'idle',
        errorMessage: null,
      };
      return {
        document: replaceConfigNode(detail.document, current.id, data),
        value: { ...current, data },
        changed: JSON.stringify(data) !== JSON.stringify(current.data),
      };
    },
    repository,
    signal
  );
}

export async function persistStandalonePendingTask(
  projectId: string,
  kind: StandaloneWorkbenchKind,
  reference: CreativeTaskReference,
  repository: CreativeProjectRepository = creativeProjectRepository,
  signal?: AbortSignal
): Promise<void> {
  if (!isCanvasNodeTaskOwner(reference.owner) || reference.owner.projectId !== projectId) {
    throw new StandaloneWorkbenchOwnershipError('owner-mismatch', '任务不属于当前项目。');
  }
  await mutateProject(
    projectId,
    (detail) => {
      const node = assertReferenceOwner(
        findStandaloneWorkbenchNode(detail.document, kind),
        kind,
        reference
      );
      if (
        node.data.taskId &&
        node.data.taskId !== reference.taskId &&
        detail.document.pendingTaskIds.includes(node.data.taskId)
      ) {
        throw new StandaloneWorkbenchOwnershipError(
          'owner-mismatch',
          '该配置节点已归属另一个未完成任务，未覆盖其 pendingTaskId。'
        );
      }
      const data: CreativeConfigNodeData = {
        ...node.data,
        providerId: reference.providerId,
        model: reference.model,
        task: reference.task,
        capability: reference.capability,
        taskId: reference.taskId,
        status: 'queued',
        resultAssetIds: [],
        errorMessage: null,
      };
      return {
        document: {
          ...replaceConfigNode(detail.document, node.id, data),
          pendingTaskIds: [...new Set([...detail.document.pendingTaskIds, reference.taskId])],
        },
        value: undefined,
        changed: true,
      };
    },
    repository,
    signal
  );
}

export async function persistStandaloneSettledTask(
  projectId: string,
  kind: StandaloneWorkbenchKind,
  task: CreativeTask,
  repository: CreativeProjectRepository = creativeProjectRepository,
  signal?: AbortSignal
): Promise<void> {
  if (!isCanvasNodeTaskOwner(task.owner) || task.owner.projectId !== projectId) {
    throw new StandaloneWorkbenchOwnershipError('owner-mismatch', '终态任务不属于当前项目。');
  }
  await mutateProject(
    projectId,
    (detail) => {
      const node = assertReferenceOwner(
        findStandaloneWorkbenchNode(detail.document, kind),
        kind,
        task
      );
      const status = task.status as CreativeGenerationStatus;
      const data: CreativeConfigNodeData = {
        ...node.data,
        providerId: task.providerId,
        model: task.model,
        task: task.task,
        capability: task.capability,
        taskId: task.taskId,
        parameters: {
          ...structuredClone(task.parameters),
          [STANDALONE_WORKBENCH_MARKER]: kind,
        },
        resultAssetIds: [...task.resultAssetIds],
        status,
        errorMessage: task.error?.message ?? null,
      };
      const pendingTaskIds = detail.document.pendingTaskIds.filter((id) => id !== task.taskId);
      const changed =
        JSON.stringify(data) !== JSON.stringify(node.data) ||
        pendingTaskIds.length !== detail.document.pendingTaskIds.length;
      return {
        document: {
          ...replaceConfigNode(detail.document, node.id, data),
          pendingTaskIds,
        },
        value: undefined,
        changed,
      };
    },
    repository,
    signal
  );
}

export async function removeStandaloneOrphanedTask(
  projectId: string,
  kind: StandaloneWorkbenchKind,
  reference: CreativeTaskReference,
  error: unknown,
  repository: CreativeProjectRepository = creativeProjectRepository,
  signal?: AbortSignal
): Promise<boolean> {
  if (!isBackendHttpError(error) || error.status !== 404) return false;
  await mutateProject(
    projectId,
    (detail) => {
      const node = assertReferenceOwner(
        findStandaloneWorkbenchNode(detail.document, kind),
        kind,
        reference
      );
      const data: CreativeConfigNodeData = {
        ...node.data,
        taskId: node.data.taskId === reference.taskId ? null : node.data.taskId,
        status: node.data.taskId === reference.taskId ? 'failed' : node.data.status,
        errorMessage:
          node.data.taskId === reference.taskId
            ? '任务已不存在，已从恢复队列移除。'
            : node.data.errorMessage,
      };
      const pendingTaskIds = detail.document.pendingTaskIds.filter(
        (id) => id !== reference.taskId
      );
      return {
        document: {
          ...replaceConfigNode(detail.document, node.id, data),
          pendingTaskIds,
        },
        value: undefined,
        changed: pendingTaskIds.length !== detail.document.pendingTaskIds.length,
      };
    },
    repository,
    signal
  );
  return true;
}

export function standaloneResumeRequests(
  document: CreativeProjectDocument,
  kind: StandaloneWorkbenchKind
): CreativeWorkbenchResumeRequest[] {
  const node = findStandaloneWorkbenchNode(document, kind);
  if (!node) return [];
  return workbenchResumeRequestsFromDocument(document).filter(
    (request) =>
      isCanvasNodeTaskOwner(request.reference.owner) &&
      request.reference.owner.nodeId === node.id
  );
}
