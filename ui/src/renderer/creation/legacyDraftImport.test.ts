import { describe, expect, test } from 'bun:test';
import { acknowledgeLegacyCreationDraft, readLegacyCreationDraft } from './legacyDraftImport';

const key = 'nomifun:creative-studio:standalone-workbench-draft:image';
const draft = { version: 1, workbenchKind: 'image', layout: 'side', prompt: '保留这个草稿', model: null, parameters: { interfaceMode: 'images', quality: 'high', width: 1536, height: 1024, aspectRatio: '3:2', count: 2 }, referenceAssetIds: ['0190f5fe-7c00-7a00-8000-000000000005'] };
describe('one-way legacy draft import', () => {
  test('retains original parameters and references until the destination is acknowledged', () => {
    const values = new Map([[key, JSON.stringify(draft)]]);
    const storage = { getItem: (key: string) => values.get(key) ?? null, removeItem: (key: string) => { values.delete(key); } };
    const imported = readLegacyCreationDraft('image', storage)!;
    expect(imported.parameters).toEqual(draft.parameters);
    expect(imported.referenceAssetIds).toEqual(draft.referenceAssetIds);
    expect(values.has(key)).toBe(true);
    values.set(key, JSON.stringify({ ...draft, prompt: 'later edit' }));
    expect(acknowledgeLegacyCreationDraft('image', imported.source, storage)).toBe(false);
    expect(values.has(key)).toBe(true);
    const latest = readLegacyCreationDraft('image', storage)!;
    expect(acknowledgeLegacyCreationDraft('image', latest.source, storage)).toBe(true);
    expect(readLegacyCreationDraft('image', storage)).toBeNull();
  });
  test('leaves incompatible or malformed data untouched and never executes it', () => {
    for (const raw of ['{', JSON.stringify({ ...draft, version: 2 }), JSON.stringify({ ...draft, referenceAssetIds: ['invalid-id'] })]) {
      let removed = false;
      const storage = { getItem: () => raw, removeItem: () => { removed = true; } };
      expect(readLegacyCreationDraft('image', storage)).toBeNull();
      expect(removed).toBe(false);
    }
  });
});
