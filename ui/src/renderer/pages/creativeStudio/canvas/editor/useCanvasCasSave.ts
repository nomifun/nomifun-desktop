/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import {
  CanvasCasSaveController,
  type CanvasCasSaveOperation,
} from './casSaveController';

export function useCanvasCasSave(
  save: CanvasCasSaveOperation,
  debounceMs: number | undefined,
  scopeKey: string
): {
  controller: CanvasCasSaveController;
  snapshot: ReturnType<CanvasCasSaveController['getSnapshot']>;
} {
  const saveRef = useRef(save);
  saveRef.current = save;
  const controller = useMemo(
    () =>
      new CanvasCasSaveController(
        (expectedRevision, document) => saveRef.current(expectedRevision, document),
        { debounceMs }
      ),
    [debounceMs, scopeKey]
  );
  const snapshot = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot
  );

  useEffect(() => () => controller.dispose(), [controller]);
  return { controller, snapshot };
}
