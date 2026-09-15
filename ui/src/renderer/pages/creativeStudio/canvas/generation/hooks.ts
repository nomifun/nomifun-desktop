/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import type { CreativeAssetPort } from "../../assets";
import type { CreativeTaskPollOptions, CreativeTaskPort } from "../../tasks";
import { CanvasGenerationRuntimeController } from "./controller";
import type { CanvasGenerationRuntimeControllerOptions } from "./controller";
import type {
  CanvasGenerationResumeRequest,
  CanvasGenerationRuntimeSnapshot,
  PreparedCanvasGenerationRun,
} from "./types";

export interface UseCanvasGenerationRuntimeOptions {
  /** Project/node scope; changing it synchronously hides the previous snapshot. */
  scopeKey: string;
  tasks: CreativeTaskPort;
  assets: CreativeAssetPort;
  poll?: Omit<CreativeTaskPollOptions, "signal" | "onTask">;
  initialResumeRequests?: readonly CanvasGenerationResumeRequest[];
  onPendingTask?: CanvasGenerationRuntimeControllerOptions["onPendingTask"];
  onSettledTask?: CanvasGenerationRuntimeControllerOptions["onSettledTask"];
  onRecoveryFailure?: CanvasGenerationRuntimeControllerOptions["onRecoveryFailure"];
  onRuntimeError?: (error: unknown) => void;
}

export interface UseCanvasGenerationRuntimeResult extends CanvasGenerationRuntimeSnapshot {
  controller: CanvasGenerationRuntimeController;
  run(
    plan: PreparedCanvasGenerationRun,
  ): Promise<CanvasGenerationRuntimeSnapshot>;
  resume(
    requests: readonly CanvasGenerationResumeRequest[],
  ): Promise<CanvasGenerationRuntimeSnapshot>;
  cancel(taskId?: string): Promise<CanvasGenerationRuntimeSnapshot>;
  retry(taskId: string): Promise<CanvasGenerationRuntimeSnapshot>;
  retrySubmission(order: number): Promise<CanvasGenerationRuntimeSnapshot>;
  dismiss(taskIds: readonly string[]): CanvasGenerationRuntimeSnapshot;
  reset(): void;
}

/** React subscription for the imperative controller; all state remains backend-derived. */
export function useCanvasGenerationRuntime(
  options: UseCanvasGenerationRuntimeOptions,
): UseCanvasGenerationRuntimeResult {
  const intervalMs = options.poll?.intervalMs;
  const maxWaitMs = options.poll?.maxWaitMs;
  const wait = options.poll?.wait;
  const now = options.poll?.now;
  const controller = useMemo(
    () =>
      new CanvasGenerationRuntimeController(options.tasks, options.assets, {
        poll: { intervalMs, maxWaitMs, wait, now },
        onPendingTask: options.onPendingTask,
        onSettledTask: options.onSettledTask,
        onRecoveryFailure: options.onRecoveryFailure,
      }),
    [
      intervalMs,
      maxWaitMs,
      now,
      options.assets,
      options.onPendingTask,
      options.onRecoveryFailure,
      options.onSettledTask,
      options.scopeKey,
      options.tasks,
      wait,
    ],
  );
  const [subscription, setSubscription] = useState(() => ({
    controller,
    snapshot: controller.snapshot(),
  }));
  const snapshot =
    subscription.controller === controller
      ? subscription.snapshot
      : controller.snapshot();

  useEffect(() => {
    const unsubscribe = controller.subscribe((next) => {
      setSubscription({ controller, snapshot: next });
    });
    if (options.initialResumeRequests?.length) {
      void controller
        .resume(options.initialResumeRequests)
        .catch((error) => options.onRuntimeError?.(error));
    }
    return () => {
      unsubscribe();
      controller.dispose();
    };
  }, [controller]);

  const run = useCallback(
    (plan: PreparedCanvasGenerationRun) => controller.run(plan),
    [controller],
  );
  const resume = useCallback(
    (requests: readonly CanvasGenerationResumeRequest[]) =>
      controller.resume(requests),
    [controller],
  );
  const cancel = useCallback(
    (taskId?: string) => controller.cancel(taskId),
    [controller],
  );
  const retry = useCallback(
    (taskId: string) => controller.retry(taskId),
    [controller],
  );
  const retrySubmission = useCallback(
    (order: number) => controller.retrySubmission(order),
    [controller],
  );
  const dismiss = useCallback(
    (taskIds: readonly string[]) => controller.dismiss(taskIds),
    [controller],
  );
  const reset = useCallback(() => controller.reset(), [controller]);

  return {
    ...snapshot,
    controller,
    run,
    resume,
    cancel,
    retry,
    retrySubmission,
    dismiss,
    reset,
  };
}
