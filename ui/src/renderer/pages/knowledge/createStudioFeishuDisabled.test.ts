import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  FEISHU_KNOWLEDGE_CREATION_ENABLED,
  canSubmitStudioSourceConfig,
  canSubmitStudioSourceType,
  normalizeStudioInitialKind,
} from './CreateStudio/sourceTypes';

const typeRailSource = readFileSync(new URL('./CreateStudio/TypeRail.tsx', import.meta.url), 'utf8');
const emptyStateSource = readFileSync(new URL('./KnowledgeEmptyState.tsx', import.meta.url), 'utf8');

describe('CreateStudio Feishu creation gate', () => {
  test('keeps Feishu knowledge-space creation disabled', () => {
    expect(FEISHU_KNOWLEDGE_CREATION_ENABLED).toBe(false);
  });

  test('falls back to blank when Feishu is preselected through a shortcut', () => {
    expect(normalizeStudioInitialKind('feishu')).toBe('blank');
    expect(normalizeStudioInitialKind('web')).toBe('web');
    expect(normalizeStudioInitialKind(undefined)).toBe('blank');
  });

  test('prevents submitting a Feishu source while the connector is disabled', () => {
    expect(canSubmitStudioSourceType('feishu')).toBe(false);
    expect(canSubmitStudioSourceType('blank')).toBe(true);
    expect(canSubmitStudioSourceType('local')).toBe(true);
    expect(canSubmitStudioSourceType('web')).toBe(true);
    expect(canSubmitStudioSourceType('import')).toBe(true);
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
    expect(canSubmitStudioSourceConfig('local', { rootPath: '/Users/muri/docs' })).toEqual({ ok: true });
    expect(canSubmitStudioSourceConfig('blank', {})).toEqual({ ok: true });
  });

  test('wires visible Feishu shortcuts to the disabled creation flag', () => {
    expect(typeRailSource.includes('FEISHU_KNOWLEDGE_CREATION_ENABLED')).toBe(true);
    expect(typeRailSource.includes('disabled: !FEISHU_KNOWLEDGE_CREATION_ENABLED')).toBe(true);
    expect(typeRailSource.includes('暂不可用')).toBe(true);

    expect(emptyStateSource.includes('FEISHU_KNOWLEDGE_CREATION_ENABLED')).toBe(true);
    expect(emptyStateSource.includes('disabled: !FEISHU_KNOWLEDGE_CREATION_ENABLED')).toBe(true);
    expect(emptyStateSource.includes('if (!k.disabled) onCreate(k.key);')).toBe(true);
  });
});
