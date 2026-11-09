/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { modelProtocol } from '@/common/adapter/ipcBridge';
import type { ModelTask } from '@/common/config/storage';
import type { ProtocolDescriptor } from '@/common/protocolBindings/ProtocolDescriptor';
import type { ProtocolExecutorKind } from '@/common/protocolBindings/ProtocolExecutorKind';
import type { ProtocolTransportKind } from '@/common/protocolBindings/ProtocolTransportKind';
import { useMemo } from 'react';
import useSWR, { useSWRConfig } from 'swr';
import useSWRSubscription from 'swr/subscription';

export interface TaskModelProtocolRequirements {
  executors?: readonly ProtocolExecutorKind[];
  transports?: readonly ProtocolTransportKind[];
}

/** `/api/stt` accepts complete files and returns a transcript in one request. */
export const FILE_SPEECH_MODEL_PROTOCOLS: TaskModelProtocolRequirements = {
  executors: ['model_invoke'],
  transports: ['http'],
};

/** Companion speech owns synchronous files and its registered incremental
 * executor, but cannot run generic realtime sessions or asynchronous jobs. */
export const COMPANION_SPEECH_MODEL_PROTOCOLS: TaskModelProtocolRequirements = {
  executors: ['model_invoke'],
};

export const matchingTaskProtocols = (
  descriptors: readonly ProtocolDescriptor[], task: ModelTask, requirements: TaskModelProtocolRequirements
): ReadonlySet<string> => new Set(descriptors.filter(descriptor =>
  descriptor.supported_tasks.includes(task)
  && (!requirements.executors || requirements.executors.includes(descriptor.executor))
  && (!requirements.transports || requirements.transports.includes(descriptor.transport))
).map(descriptor => descriptor.protocol_id));

/** Only execution-scoped selectors request registry metadata. Model/task
 * membership still comes from saved capabilities, never a preset or name. */
export function useTaskModelProtocols(task: ModelTask, requirements?: TaskModelProtocolRequirements) {
  const { mutate: refreshCache } = useSWRConfig();
  // One subscription per cache covers every scoped selector, including the
  // chat microphone. A backend restart must recover failed registry requests.
  useSWRSubscription(requirements ? 'model-selector-protocols.reconnected' : null, () =>
    ipcBridge.conversation.reconnected.on(() => {
      void refreshCache(key => Array.isArray(key) && key[0] === 'model-selector-protocol-manifest')
        .catch(() => undefined);
    })
  );
  const { data, error, isLoading, mutate } = useSWR(
    requirements ? ['model-selector-protocol-manifest', task] as const : null,
    ([, requestedTask]) => modelProtocol.list.invoke({ preset: 'custom', task: requestedTask }),
    { revalidateOnFocus: false, shouldRetryOnError: false }
  );
  const protocols = useMemo(() => requirements
    ? matchingTaskProtocols(data?.protocols ?? [], task, requirements) : undefined,
  [data, requirements, task]);
  return { protocols, error, refresh: mutate, isLoading: !!requirements && (isLoading || !data || !!error) };
}
