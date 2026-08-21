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
import { canvasImageMaskEditResumeRequests } from './imageMaskEditCanvas';
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

export interface CanvasImageMaskEditRuntimeBridgeHandle {
  submit(
    plan: PreparedCreativeWorkbenchRun
  ): Promise<CanvasImageMaskEditAdmission>;
  retrySubmission(
    order: number,
    idempotencyKey: string
  ): Promise<CanvasImageMaskEditAdmission>;
  retryTask(taskId: string): Promise<CreativeWorkbenchRuntimeSnapshot>;
  cancelTask(taskId: string): Promise<CreativeWorkbenchRuntimeSnapshot>;
  recoverTask(
    reference: CreativeTaskReference
  ): Promise<CreativeWorkbenchRuntimeSnapshot>;
  /** Returns false only when the backend authoritatively answers 404. */
  taskExists(reference: CreativeTaskReference): Promise<boolean>;
  snapshot(): CreativeWorkbenchRuntimeSnapshot;
}

export interface CanvasImageMaskEditRuntimeBridgeProps {
  projectId: string;
  initialDocument: CreativeProjectDocument;
  editorRef: React.RefObject<CreativeCanvasEditorHandle | null>;
  viewportSize: CreativeSize;
  onAsset(asset: CreativeAsset): void;
  onSnapshot(snapshot: CreativeWorkbenchRuntimeSnapshot): void;
  onNotice(message: string): void;
}

const requiredEditor = (
  ref: React.RefObject<CreativeCanvasEditorHandle | null>
): CreativeCanvasEditorHandle => {
  const editor = ref.current;
  if (!editor) throw new Error('画布尚未载入，无法同步局部编辑任务。');
  return editor;
};

/**
 * Owns exactly one image-mask task controller for one hydrated project. It is
 * intentionally renderless; the canonical canvas editor remains the only CAS
 * and reducer owner.
 */
const CanvasImageMaskEditRuntimeBridge = forwardRef<
  CanvasImageMaskEditRuntimeBridgeHandle,
  CanvasImageMaskEditRuntimeBridgeProps
>((props, ref) => {
  const latest = useRef(props);
  latest.current = props;
  const initialResumeRequestsRef = useRef(
    canvasImageMaskEditResumeRequests(props.initialDocument)
  );
  const initialResumeRequests = initialResumeRequestsRef.current;

  const onPendingTask = useCallback(
    async (reference: CreativeTaskReference, signal: AbortSignal) => {
      signal.throwIfAborted();
      const current = latest.current;
      await persistCanvasImageMaskEditPendingTask({
        editor: requiredEditor(current.editorRef),
        projectId: current.projectId,
        reference,
      });
      signal.throwIfAborted();
    },
    []
  );

  const onSettledTask = useCallback(
    async (task: CreativeTask, signal: AbortSignal) => {
      signal.throwIfAborted();
      const current = latest.current;
      await settleCanvasImageMaskEditTask({
        editor: requiredEditor(current.editorRef),
        projectId: current.projectId,
        task,
        assets: creativeAssetClient,
        viewportSize: current.viewportSize,
        onAsset: current.onAsset,
      });
      signal.throwIfAborted();
      current.onNotice(
        task.status === 'succeeded'
          ? '局部编辑已完成，真实结果图片及连线已保存到画布。'
          : task.status === 'failed'
            ? (task.error?.message ?? '局部编辑失败，配置节点已保留。')
            : '局部编辑已取消，配置节点已保留。'
      );
    },
    []
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
      await orphanCanvasImageMaskEditTask({
        editor: requiredEditor(current.editorRef),
        projectId: current.projectId,
        reference,
      });
      current.onNotice(
        '服务器未找到遗留的局部编辑任务，已只清理该任务的恢复标记。'
      );
      return true;
    },
    []
  );

  const runtime = useCreativeWorkbenchRuntime({
    scopeKey: `${props.projectId}:canvas-image-mask-edit`,
    tasks: creativeTaskClient,
    assets: creativeAssetClient,
    initialResumeRequests,
    onPendingTask,
    onSettledTask,
    onRecoveryFailure,
    onRuntimeError: (error) =>
      latest.current.onNotice(
        error instanceof Error ? error.message : String(error)
      ),
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
    for (const entry of runtime.entries) {
      if (entry.task.status !== 'queued' && entry.task.status !== 'running')
        continue;
      try {
        reconcileCanvasImageMaskEditTask({
          editor: requiredEditor(current.editorRef),
          projectId: current.projectId,
          task: entry.task,
        });
      } catch (error) {
        current.onNotice(
          error instanceof Error ? error.message : String(error)
        );
      }
    }
  }, [runtime.entries]);

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

CanvasImageMaskEditRuntimeBridge.displayName =
  'CanvasImageMaskEditRuntimeBridge';

export const canvasImageMaskEditReferenceFromPlan = (
  plan: PreparedCreativeWorkbenchRun
): CreativeTaskReference => creativeTaskReferenceFromInput(plan.input);

export default CanvasImageMaskEditRuntimeBridge;
