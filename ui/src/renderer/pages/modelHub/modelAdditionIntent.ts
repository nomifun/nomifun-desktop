/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { MODEL_TASK_ORDER } from '@/common/modelCapabilities';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';

const ADDITION_TASK_PARAM = 'addTask';

/** Keep the originating use case while opening the shared provider manager. */
export const modelProviderManagementRoute = (task?: ModelTask): string => {
  const params = new URLSearchParams({ section: 'models' });
  if (task) params.set(ADDITION_TASK_PARAM, task);
  return `/models?${params.toString()}`;
};

/** Generic provider management never invents a task for a new model. */
export const modelAdditionTask = (params: URLSearchParams): ModelTask | undefined => {
  if (params.get('section') !== 'models') return undefined;
  const requestedTask = params.get(ADDITION_TASK_PARAM);
  return MODEL_TASK_ORDER.find((task) => task === requestedTask);
};

/** Explicit sidebar navigation starts a fresh page scope. */
export const withoutModelAdditionTask = (params: URLSearchParams): URLSearchParams => {
  const next = new URLSearchParams(params);
  next.delete(ADDITION_TASK_PARAM);
  return next;
};
