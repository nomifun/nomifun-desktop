/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect, useSyncExternalStore } from 'react';

import {
  creativeWorkflowRunController,
  type CreativeWorkflowRunController,
} from './controller';

export function useCreativeWorkflowRuntime(
  controller: CreativeWorkflowRunController = creativeWorkflowRunController
) {
  const snapshot = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot
  );

  useEffect(() => {
    void controller.load().catch(() => undefined);
  }, [controller]);

  return { controller, snapshot };
}
