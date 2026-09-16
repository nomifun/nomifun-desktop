import { useCallback, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import { browserStorageGenerationKey } from '@/common/utils/browserStorageKey';

// Drafts belong to the renderer session, not to the lifetime of the welcome page.
// Keep a memory fallback when browser storage is unavailable or full.
const drafts = new Map<string, unknown>();

export function useGuidDraftState<T>(field: string, initial: T | (() => T)): [T, Dispatch<SetStateAction<T>>] {
  const key = browserStorageGenerationKey(`guid-draft:${field}`);
  const [value, setValue] = useState<T>(() => {
    if (drafts.has(key)) return drafts.get(key) as T;
    try {
      const saved = sessionStorage.getItem(key);
      if (saved !== null) return JSON.parse(saved) as T;
    } catch { /* A draft must not prevent opening the composer. */ }
    return typeof initial === 'function' ? (initial as () => T)() : initial;
  });
  const current = useRef(value);
  const setDraft = useCallback<Dispatch<SetStateAction<T>>>((next) => {
    const resolved = typeof next === 'function' ? (next as (previous: T) => T)(current.current) : next;
    current.current = resolved;
    drafts.set(key, resolved);
    try { sessionStorage.setItem(key, JSON.stringify(resolved)); } catch { /* Retain the memory draft. */ }
    // Persist before scheduling React work, including successful send cleanup
    // that may finish after navigation has unmounted this page.
    setValue(resolved);
  }, [key]);
  return [value, setDraft];
}
