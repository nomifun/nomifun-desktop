import { createContext, useCallback, useContext, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import { agentBrowserStorageGenerationKey } from '@/common/utils/browserStorageKey';

// Drafts belong to the renderer session, not to the lifetime of the welcome page.
// Keep a memory fallback when browser storage is unavailable or full.
const drafts = new Map<string, unknown>();

export function writeGuidDraftState<T>(field: string, value: T, scope?: string): void {
  const key = agentBrowserStorageGenerationKey(`guid-draft:${scope ? `${scope}:` : ''}${field}`);
  drafts.set(key, value);
  try { sessionStorage.setItem(key, JSON.stringify(value)); } catch { /* Keep the unsent draft in memory. */ }
}

/** A product surface isolates unsent UI drafts; Agent history stays canonical. */
export const GuidDraftScopeContext = createContext<string | undefined>(undefined);

/** Move only unsent composer input after a manual import establishes its owner. */
export function moveGuidInputDraft(fromScope: string, toScope: string): void {
  if (fromScope === toScope) return;
  for (const field of ['input', 'files']) {
    const from = agentBrowserStorageGenerationKey(`guid-draft:${fromScope}:${field}`);
    const to = agentBrowserStorageGenerationKey(`guid-draft:${toScope}:${field}`);
    let value = drafts.get(from);
    if (!drafts.has(from)) {
      try {
        const saved = sessionStorage.getItem(from);
        if (saved === null) continue;
        value = JSON.parse(saved);
      } catch { continue; }
    }
    drafts.set(to, value);
    drafts.delete(from);
    try {
      sessionStorage.setItem(to, JSON.stringify(value));
      sessionStorage.removeItem(from);
    } catch { /* The same unsent UI input remains in the in-memory scope. */ }
  }
}

export function useGuidDraftState<T>(field: string, initial: T | (() => T), scopeOverride?: string): [T, Dispatch<SetStateAction<T>>] {
  const contextScope = useContext(GuidDraftScopeContext);
  const scope = scopeOverride ?? contextScope;
  const key = agentBrowserStorageGenerationKey(`guid-draft:${scope ? `${scope}:` : ''}${field}`);
  const readValue = () => {
    if (drafts.has(key)) return drafts.get(key) as T;
    try {
      const saved = sessionStorage.getItem(key);
      if (saved !== null) return JSON.parse(saved) as T;
    } catch { /* A draft must not prevent opening the composer. */ }
    return typeof initial === 'function' ? (initial as () => T)() : initial;
  };
  const [state, setState] = useState(() => ({ key, value: readValue() }));
  const value = state.key === key ? state.value : readValue();
  if (state.key !== key) setState({ key, value });
  const current = useRef({ key, value });
  if (current.current.key !== key) current.current = { key, value };
  const setDraft = useCallback<Dispatch<SetStateAction<T>>>((next) => {
    // Late cleanup may write its original scope after navigation, but must
    // never read or replace the newly opened conversation's draft.
    const previous = current.current.key === key ? current.current.value : readValue();
    const resolved = typeof next === 'function' ? (next as (previous: T) => T)(previous) : next;
    const isCurrent = current.current.key === key;
    if (isCurrent) current.current = { key, value: resolved };
    drafts.set(key, resolved);
    try { sessionStorage.setItem(key, JSON.stringify(resolved)); } catch { /* Retain the memory draft. */ }
    // Persist before scheduling React work, including successful send cleanup
    // that may finish after navigation has unmounted this page.
    if (isCurrent) setState({ key, value: resolved });
  }, [key]);
  return [value, setDraft];
}
