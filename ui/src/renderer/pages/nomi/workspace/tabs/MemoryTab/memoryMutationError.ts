/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isBackendHttpError } from '@/common/adapter/httpBridge';

export const isMemoryRevisionConflict = (error: unknown): boolean => isBackendHttpError(error) && error.status === 409;
export const memoryMutationErrorKey = (error: unknown): 'nomi.memory.conflict' | 'nomi.memory.saveFailed' =>
  isMemoryRevisionConflict(error) ? 'nomi.memory.conflict' : 'nomi.memory.saveFailed';
