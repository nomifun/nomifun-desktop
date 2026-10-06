import { useCallback, useRef, useState } from 'react';
import type { CreationDraft, CreationMode } from './types';
import { agentBrowserStorageGenerationKey } from '@/common/utils/browserStorageKey';

export function emptyCreationDraft(): CreationDraft {
  return { mode: null, lastMode: 'image', models: { image: null, video: null, music: null }, parameters: { image: {}, video: {}, music: { instrumental: true } }, references: [] };
}

export const creationDraftStorageKey = (scope: string) => agentBrowserStorageGenerationKey(`conversation-creation-draft:${scope}`);
const storageKey = creationDraftStorageKey;
/** Retain editable media inputs only. Agent identity comes from the host projection. */
function normalizeCreationDraft(value: CreationDraft): CreationDraft {
  const { mode, lastMode, models, parameters, references, pendingPrompt, pendingFiles } = value;
  return { mode, lastMode, models, parameters, references,
    ...(pendingPrompt !== undefined ? { pendingPrompt } : {}),
    ...(pendingFiles !== undefined ? { pendingFiles } : {}),
  };
}
function read(scope: string): CreationDraft {
  try {
    const value = JSON.parse(sessionStorage.getItem(storageKey(scope)) || 'null');
    if (value && ['image', 'video', 'music'].includes(value.lastMode) && (value.mode === null || ['image', 'video', 'music'].includes(value.mode)) && value.models && value.parameters && Array.isArray(value.references)) {
      const draft = normalizeCreationDraft(value);
      // Rewrite the same-generation draft so removed labels/identity mirrors
      // cannot survive in browser storage after the composer loads.
      try { sessionStorage.setItem(storageKey(scope), JSON.stringify(draft)); } catch { /* Keep editable media inputs when storage is full. */ }
      return draft;
    }
  } catch { /* An invalid old draft never blocks the composer. */ }
  return emptyCreationDraft();
}

export function useCreationDraft(scope: string) {
  const [entry, setEntry] = useState(() => ({ scope, value: read(scope) }));
  const current = useRef(entry);
  const draft = entry.scope === scope ? entry.value : read(scope);
  if (entry.scope !== scope) {
    current.current = { scope, value: draft };
    setEntry(current.current);
  }
  const update = useCallback((change: (value: CreationDraft) => CreationDraft) => {
    const value = normalizeCreationDraft(change(current.current.scope === scope ? current.current.value : read(scope)));
    current.current = { scope, value };
    try { sessionStorage.setItem(storageKey(scope), JSON.stringify(value)); } catch { /* Memory remains authoritative if browser storage is full. */ }
    setEntry(current.current);
  }, [scope]);
  const setMode = useCallback((mode: CreationMode | null) => update(value => ({ ...value, mode, lastMode: mode || value.lastMode })), [update]);
  return { draft, update, setMode };
}
export type CreationDraftController = ReturnType<typeof useCreationDraft>;
