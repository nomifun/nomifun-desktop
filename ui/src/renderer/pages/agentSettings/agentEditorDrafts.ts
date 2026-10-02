import type { AgentPresetDraft, OfficialPresetTemplate, ProductAgentSelection } from '@/common/types/agentPlatform';
import { browserStorageGenerationKey, type BrowserStoragePersistence } from '@/common/utils/browserStorageKey';
import { agentEditorReturn, editingDocument, type AgentEditorReturn, type TemplateEditingState } from './model';

function browserStorage(): BrowserStoragePersistence | undefined {
  try { return typeof localStorage === 'undefined' ? undefined : localStorage; }
  catch { return undefined; }
}

/** Local editing state only. Runtime configuration still requires an explicit save. */
export class AgentEditorDrafts {
  private readonly prefix: string;

  constructor(ownerUserId: string, private readonly storage: BrowserStoragePersistence | undefined = browserStorage()) {
    this.prefix = browserStorageGenerationKey(`agent-editor:${ownerUserId}`);
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

  readTemplate(template: OfficialPresetTemplate): TemplateEditingState | undefined {
    const value = this.read(`template:${template.template_key}`) as { seed?: string; editing?: TemplateEditingState } | null;
    if (value?.seed !== JSON.stringify(template.seed)) return undefined;
    const search = `?template=${template.template_key}`;
    const snapshot = agentEditorReturn({ agentEditorReturn: {
      version: 1, search, kind: 'template', templateKey: template.template_key, editing: value.editing,
    } }, search);
    return snapshot?.kind === 'template' ? snapshot.editing : undefined;
  }

  writeTemplate(template: OfficialPresetTemplate, editing: TemplateEditingState): void {
    this.write(`template:${template.template_key}`, { seed: JSON.stringify(template.seed), editing });
  }

  removeTemplate(template: OfficialPresetTemplate): void { this.remove(`template:${template.template_key}`); }

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
