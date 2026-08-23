/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
} from 'react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';

import { creativeAssetClient, type CreativeAsset } from '../../assets';
import type { CreativeProjectDocument, CreativeSize } from '../../domain';
import {
  creativeTaskClient,
  type CreativeTask,
  type CreativeTaskReference,
} from '../../tasks';
import {
  useCreativeWorkbenchRuntime,
  type CreativeWorkbenchRuntimeSnapshot,
  type PreparedCreativeWorkbenchRun,
} from '../../workbenches/runtime';
import type { CreativeCanvasEditorHandle } from '../editor';
import {
  canvasImageComposeConfigForReference,
  canvasImageComposeResumeRequests,
  isCanvasImageComposeConfig,
} from './canvasImageComposerCanvas';
import {
  orphanCanvasImageComposeTask,
  persistCanvasImageComposePendingTask,
  reconcileCanvasImageComposeTask,
  settleCanvasImageComposeTask,
} from './canvasImageComposerRuntime';
import {
  canvasImageMaskEditConfigForReference,
  canvasImageMaskEditResumeRequests,
  isCanvasImageMaskEditConfig,
} from './imageMaskEditCanvas';
import {
  creativeTaskReferenceFromInput,
  isCanvasImageMaskEditTaskNotFound,
  orphanCanvasImageMaskEditTask,
  persistCanvasImageMaskEditPendingTask,
  reconcileCanvasImageMaskEditTask,
  settleCanvasImageMaskEditTask,
  waitForCanvasImageMaskEditAdmission,
  type CanvasImageMaskEditAdmission,
} from './imageMaskEditRuntime';

export interface CanvasImageTaskRuntimeBridgeHandle {
  submit(plan: PreparedCreativeWorkbenchRun): Promise<CanvasImageMaskEditAdmission>;
  retrySubmission(order: number, idempotencyKey: string): Promise<CanvasImageMaskEditAdmission>;
  retryTask(taskId: string): Promise<CreativeWorkbenchRuntimeSnapshot>;
  cancelTask(taskId: string): Promise<CreativeWorkbenchRuntimeSnapshot>;
  recoverTask(reference: CreativeTaskReference): Promise<CreativeWorkbenchRuntimeSnapshot>;
  /** Returns false only when the backend authoritatively answers 404. */
  taskExists(reference: CreativeTaskReference): Promise<boolean>;
  snapshot(): CreativeWorkbenchRuntimeSnapshot;
}

export interface CanvasImageTaskRuntimeBridgeProps {
  projectId: string;
  initialDocument: CreativeProjectDocument;
  editorRef: React.RefObject<CreativeCanvasEditorHandle | null>;
  viewportSize: CreativeSize;
  onAsset(asset: CreativeAsset): void;
  onSnapshot(snapshot: CreativeWorkbenchRuntimeSnapshot): void;
  onNotice(message: string): void;
}

type CanvasImageTaskKind = 'compose' | 'mask-edit';

const requiredEditor = (
  ref: React.RefObject<CreativeCanvasEditorHandle | null>,
  t: TFunction
): CreativeCanvasEditorHandle => {
  const editor = ref.current;
  if (!editor) {
    throw new Error(
      t('creativeStudio.canvas.runtime.image.editorUnavailable', {
        defaultValue: '画布尚未载入，无法同步图片任务。',
      })
    );
  }
  return editor;
};

const taskKindForReference = (
  editor: CreativeCanvasEditorHandle,
  projectId: string,
  reference: CreativeTaskReference,
  t: TFunction
): CanvasImageTaskKind => {
  if (reference.owner.kind !== 'canvas_node' || reference.owner.canvasId !== projectId) {
    throw new Error(
      t('creativeStudio.canvas.runtime.image.wrongCanvas', {
        defaultValue: '图片任务不属于当前画布。',
      })
    );
  }
  const owner = reference.owner;
  const node = editor.getState().document.nodes.find(
    (candidate) => candidate.id === owner.nodeId
  );
  if (isCanvasImageComposeConfig(node)) {
    canvasImageComposeConfigForReference(
      { projectId, nodes: editor.getState().document.nodes },
      reference
    );
    return 'compose';
  }
  if (isCanvasImageMaskEditConfig(node)) {
    canvasImageMaskEditConfigForReference(
      { projectId, nodes: editor.getState().document.nodes },
      reference
    );
    return 'mask-edit';
  }
  throw new Error(
    t('creativeStudio.canvas.runtime.image.missingConfig', {
      defaultValue: '图片任务缺少受支持的 canonical 配置节点。',
    })
  );
};

const referenceFromTask = (task: CreativeTask): CreativeTaskReference => ({
  taskId: task.taskId,
  owner: structuredClone(task.owner),
  providerId: task.providerId,
  model: task.model,
  task: task.task,
  capability: task.capability,
});

/**
 * Owns the single image-task controller for one hydrated project. Mask edits
 * and inline image composition are routed by strict persisted operation marker.
 */
const CanvasImageTaskRuntimeBridge = forwardRef<
  CanvasImageTaskRuntimeBridgeHandle,
  CanvasImageTaskRuntimeBridgeProps
>((props, ref) => {
  const { t } = useTranslation();
  const latest = useRef(props);
  latest.current = props;
  const initialResumeRequestsRef = useRef([
    ...canvasImageMaskEditResumeRequests(props.initialDocument),
    ...canvasImageComposeResumeRequests(props.initialDocument),
  ]);
  const initialResumeRequests = initialResumeRequestsRef.current;

  const onPendingTask = useCallback(
    async (reference: CreativeTaskReference, signal: AbortSignal) => {
      signal.throwIfAborted();
      const current = latest.current;
      const editor = requiredEditor(current.editorRef, t);
      const kind = taskKindForReference(
        editor,
        current.projectId,
        reference,
        t
      );
      if (kind === 'compose') {
        await persistCanvasImageComposePendingTask({
          editor,
          projectId: current.projectId,
          reference,
        });
      } else {
        await persistCanvasImageMaskEditPendingTask({
          editor,
          projectId: current.projectId,
          reference,
        });
      }
      signal.throwIfAborted();
    },
    [t]
  );

  const onSettledTask = useCallback(
    async (task: CreativeTask, signal: AbortSignal) => {
      signal.throwIfAborted();
      const current = latest.current;
      const editor = requiredEditor(current.editorRef, t);
      const kind = taskKindForReference(
        editor,
        current.projectId,
        referenceFromTask(task),
        t
      );
      if (kind === 'compose') {
        await settleCanvasImageComposeTask({
          editor,
          projectId: current.projectId,
          task,
          assets: creativeAssetClient,
          viewportSize: current.viewportSize,
          onAsset: current.onAsset,
        });
      } else {
        await settleCanvasImageMaskEditTask({
          editor,
          projectId: current.projectId,
          task,
          assets: creativeAssetClient,
          viewportSize: current.viewportSize,
          onAsset: current.onAsset,
        });
      }
      signal.throwIfAborted();
      current.onNotice(
        kind === 'compose'
          ? task.status === 'succeeded'
            ? t('creativeStudio.canvas.runtime.image.composeSucceeded', {
                defaultValue: '图片创作已完成，真实结果及连线已保存到画布。',
              })
            : task.status === 'failed'
              ? (task.error?.message ??
                t('creativeStudio.canvas.runtime.image.composeFailed', {
                  defaultValue: '图片创作失败，配置节点已保留。',
                }))
              : t('creativeStudio.canvas.runtime.image.composeCancelled', {
                  defaultValue: '图片创作已取消，配置节点已保留。',
                })
          : task.status === 'succeeded'
            ? t('creativeStudio.canvas.runtime.image.maskSucceeded', {
                defaultValue: '局部编辑已完成，真实结果图片及连线已保存到画布。',
              })
            : task.status === 'failed'
              ? (task.error?.message ??
                t('creativeStudio.canvas.runtime.image.maskFailed', {
                  defaultValue: '局部编辑失败，配置节点已保留。',
                }))
              : t('creativeStudio.canvas.runtime.image.maskCancelled', {
                  defaultValue: '局部编辑已取消，配置节点已保留。',
                })
      );
    },
    [t]
  );

  const onRecoveryFailure = useCallback(
    async (
      reference: CreativeTaskReference,
      error: unknown,
      signal: AbortSignal
    ): Promise<boolean> => {
      if (!isCanvasImageMaskEditTaskNotFound(error)) return false;
      signal.throwIfAborted();
      const current = latest.current;
      const editor = requiredEditor(current.editorRef, t);
      const kind = taskKindForReference(
        editor,
        current.projectId,
        reference,
        t
      );
      if (kind === 'compose') {
        await orphanCanvasImageComposeTask({
          editor,
          projectId: current.projectId,
          reference,
        });
      } else {
        await orphanCanvasImageMaskEditTask({
          editor,
          projectId: current.projectId,
          reference,
        });
      }
      current.onNotice(
        kind === 'compose'
          ? t('creativeStudio.canvas.runtime.image.composeOrphaned', {
              defaultValue:
                '服务器未找到遗留的图片创作任务，已只清理该任务的恢复标记。',
            })
          : t('creativeStudio.canvas.runtime.image.maskOrphaned', {
              defaultValue:
                '服务器未找到遗留的局部编辑任务，已只清理该任务的恢复标记。',
            })
      );
      return true;
    },
    [t]
  );

  const runtime = useCreativeWorkbenchRuntime({
    scopeKey: `${props.projectId}:canvas-image-tasks`,
    tasks: creativeTaskClient,
    assets: creativeAssetClient,
    initialResumeRequests,
    onPendingTask,
    onSettledTask,
    onRecoveryFailure,
    onRuntimeError: (error) =>
      latest.current.onNotice(error instanceof Error ? error.message : String(error)),
  });

  useEffect(() => {
    latest.current.onSnapshot(runtime.controller.snapshot());
  }, [
    runtime.controller,
    runtime.entries,
    runtime.recoveringCount,
    runtime.requestError,
    runtime.state,
    runtime.submissionFailures,
    runtime.submittingCount,
  ]);

  useEffect(() => {
    const current = latest.current;
    const editor = requiredEditor(current.editorRef, t);
    for (const entry of runtime.entries) {
      if (entry.task.status !== 'queued' && entry.task.status !== 'running') continue;
      try {
        const kind = taskKindForReference(
          editor,
          current.projectId,
          referenceFromTask(entry.task),
          t
        );
        if (kind === 'compose') {
          reconcileCanvasImageComposeTask({
            editor,
            projectId: current.projectId,
            task: entry.task,
          });
        } else {
          reconcileCanvasImageMaskEditTask({
            editor,
            projectId: current.projectId,
            task: entry.task,
          });
        }
      } catch (error) {
        current.onNotice(error instanceof Error ? error.message : String(error));
      }
    }
  }, [runtime.entries, t]);

  useImperativeHandle(
    ref,
    () => ({
      submit: (plan) =>
        waitForCanvasImageMaskEditAdmission({
          controller: runtime.controller,
          idempotencyKey: plan.input.idempotencyKey,
          start: () => runtime.controller.run(plan),
        }),
      retrySubmission: (order, idempotencyKey) =>
        waitForCanvasImageMaskEditAdmission({
          controller: runtime.controller,
          idempotencyKey,
          start: () => runtime.controller.retrySubmission(order),
        }),
      retryTask: (taskId) => runtime.controller.retry(taskId),
      cancelTask: (taskId) => runtime.controller.cancel(taskId),
      recoverTask: (reference) =>
        runtime.controller.resume([{ reference, outputKind: 'image' }]),
      taskExists: async (reference) => {
        try {
          await creativeTaskClient.get(reference);
          return true;
        } catch (error) {
          if (isCanvasImageMaskEditTaskNotFound(error)) return false;
          throw error;
        }
      },
      snapshot: () => runtime.controller.snapshot(),
    }),
    [runtime.controller]
  );

  return null;
});

CanvasImageTaskRuntimeBridge.displayName = 'CanvasImageTaskRuntimeBridge';

export const canvasImageTaskReferenceFromPlan = (
  plan: PreparedCreativeWorkbenchRun
): CreativeTaskReference => creativeTaskReferenceFromInput(plan.input);

export default CanvasImageTaskRuntimeBridge;
