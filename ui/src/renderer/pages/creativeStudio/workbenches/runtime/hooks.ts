/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import type { CreativeAssetPort } from "../../assets";
import type { CreativeTaskPollOptions, CreativeTaskPort } from "../../tasks";
import { CreativeWorkbenchRuntimeController } from "./controller";
import type { CreativeWorkbenchRuntimeControllerOptions } from "./controller";
import type {
  CreativeWorkbenchResumeRequest,
  CreativeWorkbenchRuntimeSnapshot,
  PreparedCreativeWorkbenchRun,
} from "./types";
import {
  prepareAudioWorkbenchRun,
  prepareImageWorkbenchRun,
  prepareVideoWorkbenchRun,
} from "./plans";
import type {
  PrepareAudioWorkbenchRunInput,
  PrepareImageWorkbenchRunInput,
  PrepareVideoWorkbenchRunInput,
} from "./plans";

export interface UseCreativeWorkbenchRuntimeOptions {
  /** Project/node scope; changing it synchronously hides the previous snapshot. */
  scopeKey: string;
  tasks: CreativeTaskPort;
  assets: CreativeAssetPort;
  poll?: Omit<CreativeTaskPollOptions, "signal" | "onTask">;
  initialResumeRequests?: readonly CreativeWorkbenchResumeRequest[];
  onPendingTask?: CreativeWorkbenchRuntimeControllerOptions["onPendingTask"];
  onSettledTask?: CreativeWorkbenchRuntimeControllerOptions["onSettledTask"];
  onRecoveryFailure?: CreativeWorkbenchRuntimeControllerOptions["onRecoveryFailure"];
  onRuntimeError?: (error: unknown) => void;
}

export interface UseCreativeWorkbenchRuntimeResult extends CreativeWorkbenchRuntimeSnapshot {
  controller: CreativeWorkbenchRuntimeController;
  run(
    plan: PreparedCreativeWorkbenchRun,
  ): Promise<CreativeWorkbenchRuntimeSnapshot>;
  resume(
    requests: readonly CreativeWorkbenchResumeRequest[],
  ): Promise<CreativeWorkbenchRuntimeSnapshot>;
  cancel(taskId?: string): Promise<CreativeWorkbenchRuntimeSnapshot>;
  retry(taskId: string): Promise<CreativeWorkbenchRuntimeSnapshot>;
  retrySubmission(order: number): Promise<CreativeWorkbenchRuntimeSnapshot>;
  reset(): void;
}

/** React subscription for the imperative controller; all state remains backend-derived. */
export function useCreativeWorkbenchRuntime(
  options: UseCreativeWorkbenchRuntimeOptions,
): UseCreativeWorkbenchRuntimeResult {
  const intervalMs = options.poll?.intervalMs;
  const maxWaitMs = options.poll?.maxWaitMs;
  const wait = options.poll?.wait;
  const now = options.poll?.now;
  const controller = useMemo(
    () =>
      new CreativeWorkbenchRuntimeController(options.tasks, options.assets, {
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
    (plan: PreparedCreativeWorkbenchRun) => controller.run(plan),
    [controller],
  );
  const resume = useCallback(
    (requests: readonly CreativeWorkbenchResumeRequest[]) =>
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
  const reset = useCallback(() => controller.reset(), [controller]);

  return {
    ...snapshot,
    controller,
    run,
    resume,
    cancel,
    retry,
    retrySubmission,
    reset,
  };
}

export interface UseImageWorkbenchRuntimeResult extends UseCreativeWorkbenchRuntimeResult {
  generate(
    input: PrepareImageWorkbenchRunInput,
  ): Promise<CreativeWorkbenchRuntimeSnapshot>;
}

export function useImageWorkbenchRuntime(
  options: UseCreativeWorkbenchRuntimeOptions,
): UseImageWorkbenchRuntimeResult {
  const runtime = useCreativeWorkbenchRuntime(options);
  const generate = useCallback(
    (input: PrepareImageWorkbenchRunInput) =>
      runtime.run(prepareImageWorkbenchRun(input)),
    [runtime.run],
  );
  return { ...runtime, generate };
}

export interface UseVideoWorkbenchRuntimeResult extends UseCreativeWorkbenchRuntimeResult {
  generate(
    input: PrepareVideoWorkbenchRunInput,
  ): Promise<CreativeWorkbenchRuntimeSnapshot>;
}

export function useVideoWorkbenchRuntime(
  options: UseCreativeWorkbenchRuntimeOptions,
): UseVideoWorkbenchRuntimeResult {
  const runtime = useCreativeWorkbenchRuntime(options);
  const generate = useCallback(
    (input: PrepareVideoWorkbenchRunInput) =>
      runtime.run(prepareVideoWorkbenchRun(input)),
    [runtime.run],
  );
  return { ...runtime, generate };
}

export interface UseAudioWorkbenchRuntimeResult extends UseCreativeWorkbenchRuntimeResult {
  generate(
    input: PrepareAudioWorkbenchRunInput,
  ): Promise<CreativeWorkbenchRuntimeSnapshot>;
}

export function useAudioWorkbenchRuntime(
  options: UseCreativeWorkbenchRuntimeOptions,
): UseAudioWorkbenchRuntimeResult {
  const runtime = useCreativeWorkbenchRuntime(options);
  const generate = useCallback(
    (input: PrepareAudioWorkbenchRunInput) =>
      runtime.run(prepareAudioWorkbenchRun(input)),
    [runtime.run],
  );
  return { ...runtime, generate };
}
