/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreateCreativeTaskInput,
  CreativeTask,
  CreativeTaskReference,
} from './types';

/** Runtime boundary used by the canvas. It has no dependency on the retired Workshop page. */
export interface CreativeTaskPort {
  create(input: CreateCreativeTaskInput, signal?: AbortSignal): Promise<CreativeTask>;
  get(reference: CreativeTaskReference, signal?: AbortSignal): Promise<CreativeTask>;
  cancel(reference: CreativeTaskReference, signal?: AbortSignal): Promise<CreativeTask>;
}
