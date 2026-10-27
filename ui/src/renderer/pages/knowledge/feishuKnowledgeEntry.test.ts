import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  canSubmitStudioSourceConfig,
  normalizeStudioInitialKind,
} from './CreateStudio/sourceTypes';

const typeRailSource = readFileSync(new URL('./CreateStudio/TypeRail.tsx', import.meta.url), 'utf8');
const emptyStateSource = readFileSync(new URL('./KnowledgeEmptyState.tsx', import.meta.url), 'utf8');
const listPageSource = readFileSync(new URL('./KnowledgeListPage/index.tsx', import.meta.url), 'utf8');

describe('Feishu knowledge entry', () => {
  test('CreateStudio only creates local bases; Feishu is not a source type', () => {
    expect(normalizeStudioInitialKind('web')).toBe('web');
    expect(normalizeStudioInitialKind(undefined)).toBe('blank');
    // 'feishu' is no longer a StudioSourceType — the rail must not offer it.
    expect(typeRailSource.includes("key: 'feishu'")).toBe(false);
    expect(typeRailSource.includes('FEISHU_KNOWLEDGE_CREATION_ENABLED')).toBe(false);
  });

  test('requires an explicit folder path before submitting a local-folder source', () => {
    expect(canSubmitStudioSourceConfig('local', {})).toEqual({
      ok: false,
      messageKey: 'knowledge.studio.localFolderRequired',
    });
    expect(canSubmitStudioSourceConfig('local', { rootPath: '   ' })).toEqual({
      ok: false,
      messageKey: 'knowledge.studio.localFolderRequired',
    });
    expect(canSubmitStudioSourceConfig('local', { rootPath: '/Users/developer/docs' })).toEqual({ ok: true });
    expect(canSubmitStudioSourceConfig('blank', {})).toEqual({ ok: true });
  });

  test('keeps the Feishu empty-state tile enabled as a navigation entry', () => {
    expect(emptyStateSource.includes("key: 'feishu'")).toBe(true);
    expect(emptyStateSource.includes('FEISHU_KNOWLEDGE_CREATION_ENABLED')).toBe(false);
    expect(emptyStateSource.includes('if (!k.disabled) onCreate(k.key);')).toBe(true);
  });

  test('routes the Feishu shortcut to the knowledge space, not the studio', () => {
    expect(listPageSource.includes("initialKind === 'feishu'")).toBe(true);
    expect(listPageSource.includes("navigate('/knowledge/feishu')")).toBe(true);
  });
});
