import type { AgentPresetDocument, AgentPresetDraft, ProductAgentSelection } from '@/common/types/agentPlatform';
import { agentBrowserStorageGenerationKey, type BrowserStoragePersistence } from '@/common/utils/browserStorageKey';
import { agentEditorReturn, editingDocument, type AgentEditorReturn } from './model';

/** Rebase local grants only through unambiguous changes in the Host's current contract. */
export function rebaseEditorActionGrants<T extends Pick<AgentPresetDocument, 'enabled_capabilities'>>(
  document: T,
  saved: AgentPresetDocument | undefined,
  current: AgentPresetDocument,
): T {
  if (!saved) return document;
  const currentById = new Map(current.enabled_capabilities.map(selection => [selection.capability.id, selection]));
  const changes = new Map<string, { removed: string; added?: string; retained?: string[] }>();
  for (const previous of saved.enabled_capabilities) {
    const next = currentById.get(previous.capability.id);
    if (!next) continue;
    const before = new Set(previous.action_allowlist ?? []);
    const after = new Set(next.action_allowlist ?? []);
    const removed = [...before].filter(action => !after.has(action));
    const added = [...after].filter(action => !before.has(action));
    if (removed.length === 1 && added.length === 1) {
      changes.set(previous.capability.id, { removed: removed[0], added: added[0] });
    } else if (removed.length === 1 && added.length === 0 && after.size > 0) {
      // A redundant grant may disappear only when every current baseline
      // grant is still selected locally; trimmed drafts remain repairable.
      changes.set(previous.capability.id, { removed: removed[0], retained: [...after] });
    }
  }
  return {
    ...document,
    enabled_capabilities: document.enabled_capabilities.map(selection => {
      const change = changes.get(selection.capability.id);
      const actions = selection.action_allowlist ?? [];
      if (!change || !actions.includes(change.removed)
        || change.retained?.some(action => !actions.includes(action))) return selection;
      return {
        ...selection,
        action_allowlist: [...new Set(actions.flatMap(action => {
          if (action !== change.removed) return [action];
          return change.added ? [change.added] : [];
        }))],
      };
    }),
  };
}

function browserStorage(): BrowserStoragePersistence | undefined {
  try { return typeof localStorage === 'undefined' ? undefined : localStorage; }
  catch { return undefined; }
}

/** Local editing state only. Runtime configuration still requires an explicit save. */
export class AgentEditorDrafts {
  private readonly prefix: string;

  constructor(ownerUserId: string, private readonly storage: BrowserStoragePersistence | undefined = browserStorage()) {
    this.prefix = agentBrowserStorageGenerationKey(`agent-editor:${ownerUserId}`);
  }

  private key(target: string): string { return `${this.prefix}|${target}`; }

  private read(target: string): unknown {
    try { return JSON.parse(this.storage?.getItem(this.key(target)) ?? 'null'); }
    catch { return null; }
  }

  private write(target: string, value: unknown): void {
    try { this.storage?.setItem(this.key(target), JSON.stringify(value)); }
    catch { /* Editing remains usable when browser storage is unavailable. */ }
  }

  private remove(target: string): void {
    try { this.storage?.removeItem(this.key(target)); }
    catch { /* Browser storage may be unavailable. */ }
  }

  readSelection(): ProductAgentSelection | null {
    const value = this.read('selection') as ProductAgentSelection | null;
    if (value?.kind === 'template' && typeof value.template_key === 'string') return value;
    if (value?.kind === 'preset' && typeof value.preset_id === 'string') return value;
    return null;
  }

  rememberSelection(selection: ProductAgentSelection): void { this.write('selection', selection); }

  readPreset(presetId: string): Extract<AgentEditorReturn, { kind: 'preset' }>['draft'] | undefined {
    const search = `?preset=${presetId}`;
    const snapshot = agentEditorReturn({ agentEditorReturn: {
      version: 1, search, kind: 'preset', draft: this.read(`preset:${presetId}`),
    } }, search);
    return snapshot?.kind === 'preset' ? snapshot.draft : undefined;
  }

  writePreset(draft: AgentPresetDraft): void {
    this.write(`preset:${draft.preset_id}`, { ...draft, document: editingDocument(draft.document) });
  }

  removePreset(presetId: string): void { this.remove(`preset:${presetId}`); }
}
