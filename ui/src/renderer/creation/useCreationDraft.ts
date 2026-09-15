import { useCallback, useState } from 'react';
import type { CreationDraft, CreationMode } from './types';
import { browserStorageGenerationKey } from '@/common/utils/browserStorageKey';

export function emptyCreationDraft(): CreationDraft {
  return { mode: null, lastMode: 'image', models: { image: null, video: null, music: null }, parameters: { image: {}, video: {}, music: { instrumental: true } }, references: [] };
}

export const creationDraftStorageKey = (scope: string) => browserStorageGenerationKey(`conversation-creation-draft:${scope}`);
const storageKey = creationDraftStorageKey;
function read(scope: string): CreationDraft {
  try {
    const value = JSON.parse(sessionStorage.getItem(storageKey(scope)) || 'null');
    if (value && ['image', 'video', 'music'].includes(value.lastMode) && (value.mode === null || ['image', 'video', 'music'].includes(value.mode)) && value.models && value.parameters && Array.isArray(value.references)) return value;
  } catch { /* An invalid old draft never blocks the composer. */ }
  return emptyCreationDraft();
}

export function useCreationDraft(scope: string) {
  const [entry, setEntry] = useState(() => ({ scope, value: read(scope) }));
  const draft = entry.scope === scope ? entry.value : read(scope);
  if (entry.scope !== scope) setEntry({ scope, value: draft });
  const update = useCallback((change: (value: CreationDraft) => CreationDraft) => {
    setEntry(previous => {
      const value = change(previous.scope === scope ? previous.value : read(scope));
      try { sessionStorage.setItem(storageKey(scope), JSON.stringify(value)); } catch { /* Memory remains authoritative if browser storage is full. */ }
      return { scope, value };
    });
  }, [scope]);
  const setMode = useCallback((mode: CreationMode | null) => update(value => ({ ...value, mode, lastMode: mode || value.lastMode })), [update]);
  return { draft, update, setMode };
}
export type CreationDraftController = ReturnType<typeof useCreationDraft>;
