import { useCallback, useRef, useState } from 'react';
import type { CreationDraft, CreationMode } from './types';
import { agentBrowserStorageGenerationKey } from '@/common/utils/browserStorageKey';
import { emitter } from '@/renderer/utils/emitter';

export function emptyCreationDraft(): CreationDraft {
  return { mode: null, lastMode: 'image', models: { image: null, video: null, music: null }, parameters: { image: {}, video: {}, music: { instrumental: true } }, references: [] };
}

export const creationDraftStorageKey = (scope: string) => agentBrowserStorageGenerationKey(`conversation-creation-draft:${scope}`);
const modes: readonly CreationMode[] = ['image', 'video', 'music'];
const isRecord = (value: unknown): value is Record<string, unknown> => Boolean(value) && typeof value === 'object' && !Array.isArray(value);
const isMode = (value: unknown): value is CreationMode => modes.includes(value as CreationMode);

/** Welcome-page drafts are temporary; existing conversation composers survive app exits. */
function draftStorage(scope: string): Storage {
  return scope === 'guid' ? sessionStorage : localStorage;
}

function isCreationDraft(value: unknown): value is CreationDraft {
  if (!isRecord(value) || !isMode(value.lastMode) || (value.mode !== null && !isMode(value.mode))
    || !isRecord(value.models) || !isRecord(value.parameters) || !Array.isArray(value.references)) return false;
  const { models, parameters } = value;
  const validReferences = value.references.every(reference => isRecord(reference)
    && typeof reference.asset_id === 'string' && reference.asset_id.trim().length > 0
    && ['image', 'video', 'audio', 'text'].includes(reference.kind as string)
    && ['reference', 'mask', 'first_frame', 'last_frame', 'video', 'audio'].includes(reference.role as string)
    && typeof reference.title === 'string'
    && (reference.url === undefined || typeof reference.url === 'string'));
  return modes.every(mode => {
    const model = models[mode];
    return (model === null || (isRecord(model) && typeof model.providerId === 'string' && typeof model.model === 'string'))
      && isRecord(parameters[mode]);
  }) && validReferences && (value.pendingPrompt === undefined || typeof value.pendingPrompt === 'string')
    && (value.pendingFiles === undefined || (Array.isArray(value.pendingFiles) && value.pendingFiles.every(path => typeof path === 'string')));
}
/** Retain editable media inputs only. Agent identity comes from the host projection. */
function normalizeCreationDraft(value: CreationDraft): CreationDraft {
  const { mode, lastMode, models, parameters, references, pendingPrompt, pendingFiles } = value;
  return { mode, lastMode, models, parameters, references,
    ...(pendingPrompt !== undefined ? { pendingPrompt } : {}),
    ...(pendingFiles !== undefined ? { pendingFiles } : {}),
  };
}
function persist(scope: string, key: string, value: CreationDraft): CreationDraft {
  const draft = normalizeCreationDraft(value);
  try { draftStorage(scope).setItem(key, JSON.stringify(draft)); } catch { /* Editing remains available when browser storage is unavailable or full. */ }
  return draft;
}

/** The same writer owns both ordinary edits and the welcome-page to conversation handoff. */
export function writeCreationDraft(scope: string, value: CreationDraft): void {
  persist(scope, creationDraftStorageKey(scope), value);
}

export function removeCreationDraft(scope: string): void {
  const key = creationDraftStorageKey(scope);
  try { localStorage.removeItem(key); } catch { /* Browser cleanup is best effort. */ }
  try { sessionStorage.removeItem(key); } catch { /* Also retire a current-generation temporary copy. */ }
}

// Deletion must clear drafts even when the deleted conversation is not mounted.
// This notification is emitted after successful/authoritative backend deletion.
emitter.on('conversation.deleted', removeCreationDraft);

function read(scope: string, key: string): CreationDraft {
  try {
    const value: unknown = JSON.parse(draftStorage(scope).getItem(key) || 'null');
    // Only this dataset's current-generation media draft is read. Agent
    // names, identities, and retired/unscoped drafts never become a source.
    if (isCreationDraft(value)) return persist(scope, key, value);
  } catch { /* An invalid old draft never blocks the composer. */ }
  return emptyCreationDraft();
}

export function useCreationDraft(scope: string) {
  const key = creationDraftStorageKey(scope);
  const [entry, setEntry] = useState(() => ({ key, value: read(scope, key) }));
  const current = useRef(entry);
  const draft = entry.key === key ? entry.value : read(scope, key);
  if (entry.key !== key) {
    current.current = { key, value: draft };
    setEntry(current.current);
  }
  const update = useCallback((change: (value: CreationDraft) => CreationDraft) => {
    // A delayed callback from a retired dataset cannot seed its successor.
    if (creationDraftStorageKey(scope) !== key) return;
    const value = persist(scope, key, change(current.current.key === key ? current.current.value : read(scope, key)));
    // Old-scope work may finish after navigation. Save it to its own scope,
    // without replacing the currently displayed conversation's state.
    if (current.current.key === key) {
      current.current = { key, value };
      setEntry(current.current);
    }
  }, [key, scope]);
  const setMode = useCallback((mode: CreationMode | null) => update(value => ({ ...value, mode, lastMode: mode || value.lastMode })), [update]);
  return { draft, update, setMode };
}
export type CreationDraftController = ReturnType<typeof useCreationDraft>;
