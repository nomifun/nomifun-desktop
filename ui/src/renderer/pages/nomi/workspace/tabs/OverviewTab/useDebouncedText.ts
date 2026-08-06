/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';

/**
 * Debounced text editing over an optimistically-patched source value: the local
 * draft follows keystrokes, the commit fires after `delay` ms of quiet.
 *
 * Kept local to the tab (rather than shared) because it is only ever used to
 * front a `patchCompanion` write — the draft must re-sync whenever the profile
 * changes underneath (companion switch, server reconcile), which is exactly the
 * `source` effect below.
 */
export const useDebouncedText = (source: string, commit: (value: string) => void, delay = 500) => {
  const [draft, setDraft] = useState(source);
  const timerRef = useRef<number | undefined>(undefined);
  /** The value a scheduled-but-not-yet-fired commit would write; null when idle. */
  const pendingRef = useRef<string | null>(null);
  const commitRef = useRef(commit);
  commitRef.current = commit;

  useEffect(() => {
    setDraft(source);
  }, [source]);

  /**
   * Unmounting must not silently discard keystrokes still inside the debounce
   * window — closing the pane, switching tab or switching companion within
   * `delay` ms used to drop the edit outright, which for the name field reads as
   * "my rename didn't save".
   *
   * Flushing here is safe across a companion switch: `commitRef` holds the
   * closure from this component's LAST render, so it is still bound to the
   * companion (and its `patchCompanion`) the text was actually typed for.
   */
  useEffect(
    () => () => {
      window.clearTimeout(timerRef.current);
      const pending = pendingRef.current;
      pendingRef.current = null;
      if (pending !== null) commitRef.current(pending);
    },
    []
  );

  const onChange = useCallback(
    (value: string) => {
      setDraft(value);
      pendingRef.current = value;
      window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => {
        pendingRef.current = null;
        commitRef.current(value);
      }, delay);
    },
    [delay]
  );

  return [draft, onChange] as const;
};

export default useDebouncedText;
