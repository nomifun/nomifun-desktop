/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useMemo } from 'react';

import { creativeProjectRepository } from '../../services';
import type { CreativeProjectDetail } from '../../domain';
import type { CreativeTask, CreativeTaskReference } from '../../tasks';
import type { CreativeWorkbenchResumeRequest } from '../runtime';
import {
  persistStandalonePendingTask,
  persistStandaloneSettledTask,
  removeStandaloneOrphanedTask,
  standaloneResumeRequests,
  type StandaloneWorkbenchKind,
} from './ownership';

export interface StandalonePersistenceCallbacks {
  initialResumeRequests: readonly CreativeWorkbenchResumeRequest[];
  resumeError: Error | null;
  onPendingTask(reference: CreativeTaskReference, signal: AbortSignal): Promise<void>;
  onSettledTask(task: CreativeTask, signal: AbortSignal): Promise<void>;
  onRecoveryFailure(
    reference: CreativeTaskReference,
    error: unknown,
    signal: AbortSignal
  ): Promise<boolean>;
}

export function useStandalonePersistence(input: {
  kind: StandaloneWorkbenchKind;
  detail: CreativeProjectDetail;
  refresh(): Promise<CreativeProjectDetail | undefined>;
}): StandalonePersistenceCallbacks {
  const projectId = input.detail.project.projectId;
  const recovery = useMemo(() => {
    try {
      return {
        requests: standaloneResumeRequests(input.detail.document, input.kind),
        error: null,
      };
    } catch (error) {
      return {
        requests: [] as CreativeWorkbenchResumeRequest[],
        error: error instanceof Error ? error : new Error(String(error)),
      };
    }
  }, [input.detail.document, input.kind]);

  const onPendingTask = useCallback(
    async (reference: CreativeTaskReference, signal: AbortSignal) => {
      await persistStandalonePendingTask(
        projectId,
        input.kind,
        reference,
        creativeProjectRepository,
        signal
      );
      await input.refresh();
    },
    [input.kind, input.refresh, projectId]
  );

  const onSettledTask = useCallback(
    async (task: CreativeTask, signal: AbortSignal) => {
      await persistStandaloneSettledTask(
        projectId,
        input.kind,
        task,
        creativeProjectRepository,
        signal
      );
      await input.refresh();
    },
    [input.kind, input.refresh, projectId]
  );

  const onRecoveryFailure = useCallback(
    async (reference: CreativeTaskReference, error: unknown, signal: AbortSignal) => {
      const removed = await removeStandaloneOrphanedTask(
        projectId,
        input.kind,
        reference,
        error,
        creativeProjectRepository,
        signal
      );
      if (removed) await input.refresh();
      return removed;
    },
    [input.kind, input.refresh, projectId]
  );

  return {
    initialResumeRequests: recovery.requests,
    resumeError: recovery.error,
    onPendingTask,
    onSettledTask,
    onRecoveryFailure,
  };
}
