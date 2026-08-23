/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeStandaloneWorkbenchKind,
  CreativeTaskOwner,
} from '../../tasks';

export const STANDALONE_VIDEO_MAX_CONCURRENT_TASKS = 1;

/** Standalone workbenches are installation-owned and never carry a Canvas ID. */
export function standaloneWorkbenchOwner(
  workbenchKind: CreativeStandaloneWorkbenchKind
): CreativeTaskOwner {
  return {
    kind: 'standalone_workbench',
    workbenchKind,
  };
}
