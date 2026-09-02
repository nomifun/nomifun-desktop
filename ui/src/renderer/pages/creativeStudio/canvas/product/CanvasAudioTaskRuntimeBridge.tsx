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
import type { CreativeProjectDocument } from '../../domain';
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
  canvasAudioComposeConfigForReference,
  canvasAudioComposeResumeRequests,
} from './canvasAudioComposerCanvas';
import {
  orphanCanvasAudioComposeTask,
  persistCanvasAudioComposePendingTask,
  reconcileCanvasAudioComposeTask,
  settleCanvasAudioComposeTask,
} from './canvasAudioComposerRuntime';
import {
  creativeTaskReferenceFromInput,
  isCanvasImageMaskEditTaskNotFound,
  waitForCanvasImageMaskEditAdmission,
  type CanvasImageMaskEditAdmission,
} from './imageMaskEditRuntime';

export type CanvasAudioTaskAdmission = CanvasImageMaskEditAdmission;

export interface CanvasAudioTaskRuntimeBridgeHandle {
  submit(plan: PreparedCreativeWorkbenchRun): Promise<CanvasAudioTaskAdmission>;
  retrySubmission(
    order: number,
    idempotencyKey: string
  ): Promise<CanvasAudioTaskAdmission>;
  retryTask(taskId: string): Promise<CreativeWorkbenchRuntimeSnapshot>;
  cancelTask(taskId: string): Promise<CreativeWorkbenchRuntimeSnapshot>;
  recoverTask(
    reference: CreativeTaskReference
  ): Promise<CreativeWorkbenchRuntimeSnapshot>;
  /** Returns false only when the backend authoritatively answers 404. */
  taskExists(reference: CreativeTaskReference): Promise<boolean>;
  snapshot(): CreativeWorkbenchRuntimeSnapshot;
}

export interface CanvasAudioTaskRuntimeBridgeProps {
  projectId: string;
  initialDocument: CreativeProjectDocument;
  editorRef: React.RefObject<CreativeCanvasEditorHandle | null>;
  onAsset(asset: CreativeAsset): void;
  onSnapshot(snapshot: CreativeWorkbenchRuntimeSnapshot): void;
  onNotice(message: string): void;
}

const requiredEditor = (
  ref: React.RefObject<CreativeCanvasEditorHandle | null>,
  t: TFunction
): CreativeCanvasEditorHandle => {
  const editor = ref.current;
  if (!editor) {
    throw new Error(
      t('creativeStudio.canvas.runtime.audio.editorUnavailable', {
        defaultValue: '画布尚未载入，无法同步音频任务。',
      })
    );
  }
  return editor;
};

/**
 * Owns the single audio-task controller for one hydrated project. Every
 * callback revalidates the persisted canvasAudioCompose owner before it may
 * mutate the document, so standalone TTS tasks cannot cross this bridge.
 */
const CanvasAudioTaskRuntimeBridge = forwardRef<
  CanvasAudioTaskRuntimeBridgeHandle,
  CanvasAudioTaskRuntimeBridgeProps
>((props, ref) => {
  const { t } = useTranslation();
  const latest = useRef(props);
  latest.current = props;
  const initialResumeRequestsRef = useRef(
    canvasAudioComposeResumeRequests(props.initialDocument)
  );
  const initialResumeRequests = initialResumeRequestsRef.current;

  const onPendingTask = useCallback(
    async (reference: CreativeTaskReference, signal: AbortSignal) => {
      signal.throwIfAborted();
      const current = latest.current;
      await persistCanvasAudioComposePendingTask({
        editor: requiredEditor(current.editorRef, t),
        projectId: current.projectId,
        reference,
      });
      signal.throwIfAborted();
    },
    [t]
  );

  const onSettledTask = useCallback(
    async (task: CreativeTask, signal: AbortSignal) => {
      signal.throwIfAborted();
      const current = latest.current;
      await settleCanvasAudioComposeTask({
        editor: requiredEditor(current.editorRef, t),
        projectId: current.projectId,
        task,
        assets: creativeAssetClient,
        onAsset: current.onAsset,
      });
      signal.throwIfAborted();
      current.onNotice(
        task.status === 'succeeded'
          ? t('creativeStudio.canvas.runtime.audio.succeeded', {
              defaultValue: '音频创作已完成，真实结果已原位保存到画布。',
            })
          : task.status === 'failed'
            ? (task.error?.message ??
              t('creativeStudio.canvas.runtime.audio.failed', {
                defaultValue: '音频创作失败。',
              }))
            : t('creativeStudio.canvas.runtime.audio.cancelled', {
                defaultValue: '音频创作已取消。',
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
      await orphanCanvasAudioComposeTask({
        editor: requiredEditor(current.editorRef, t),
        projectId: current.projectId,
        reference,
      });
      current.onNotice(
        t('creativeStudio.canvas.runtime.audio.orphaned', {
          defaultValue:
            '服务器未找到遗留的音频创作任务，已只清理该任务的恢复标记。',
        })
      );
      return true;
    },
    [t]
  );

  const runtime = useCreativeWorkbenchRuntime({
    scopeKey: `${props.projectId}:canvas-audio-tasks`,
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
    const editor = requiredEditor(current.editorRef, t);
    for (const entry of runtime.entries) {
      if (entry.task.status !== 'queued' && entry.task.status !== 'running') {
        continue;
      }
      try {
        reconcileCanvasAudioComposeTask({
          editor,
          projectId: current.projectId,
          task: entry.task,
        });
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
      recoverTask: (reference) => {
        const current = latest.current;
        canvasAudioComposeConfigForReference(
          {
            projectId: current.projectId,
            nodes: requiredEditor(current.editorRef, t).getState().document.nodes,
          },
          reference
        );
        return runtime.controller.resume([{ reference, outputKind: 'audio' }]);
      },
      taskExists: async (reference) => {
        const current = latest.current;
        canvasAudioComposeConfigForReference(
          {
            projectId: current.projectId,
            nodes: requiredEditor(current.editorRef, t).getState().document.nodes,
          },
          reference
        );
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
    [runtime.controller, t]
  );

  return null;
});

CanvasAudioTaskRuntimeBridge.displayName = 'CanvasAudioTaskRuntimeBridge';

export const canvasAudioTaskReferenceFromPlan = (
  plan: PreparedCreativeWorkbenchRun
): CreativeTaskReference => creativeTaskReferenceFromInput(plan.input);

export default CanvasAudioTaskRuntimeBridge;
