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
  test('CreateStudio offers feishu as a curated remote-backed source type', () => {
    expect(normalizeStudioInitialKind('web')).toBe('web');
    expect(normalizeStudioInitialKind('feishu')).toBe('feishu');
    expect(normalizeStudioInitialKind(undefined)).toBe('blank');
    // 'feishu' is a StudioSourceType offered by the rail (third-party group).
    expect(typeRailSource.includes("key: 'feishu'")).toBe(true);
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

  test('routes the Feishu shortcut into the studio; the pinned card keeps the workbench route', () => {
    expect(listPageSource.includes("navigate('/knowledge/feishu')")).toBe(false);
    expect(listPageSource.includes('<FeishuKnowledgeCard />')).toBe(true);
  });
});
