import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  DEFAULT_LOCAL_FOLDER_TREE_ACCESS,
  resolveLocalFolderTreeAccess,
} from './knowledgeTreeAccess';

describe('local-folder knowledge tree access', () => {
  test('defaults a new local folder to full file management', () => {
    expect(DEFAULT_LOCAL_FOLDER_TREE_ACCESS).toBe('editable');
    expect(resolveLocalFolderTreeAccess(undefined)).toBe('editable');
  });

  test('preserves an explicit read-only or editable choice', () => {
    expect(resolveLocalFolderTreeAccess('read_only')).toBe('read_only');
    expect(resolveLocalFolderTreeAccess('editable')).toBe('editable');
  });

  test('uses the same resolver for the create payload and the visible switch', () => {
    const studio = readFileSync(new URL('./CreateStudio/index.tsx', import.meta.url), 'utf8');
    const sourceConfig = readFileSync(
      new URL('./CreateStudio/SourceConfig.tsx', import.meta.url),
      'utf8',
    );
    expect(studio.includes('resolveLocalFolderTreeAccess(sourceConfigValue.localTreeAccess)')).toBe(true);
    expect(sourceConfig.includes('resolveLocalFolderTreeAccess(value.localTreeAccess)')).toBe(true);
    expect(studio.includes('allowLocalEdits')).toBe(false);
    expect(sourceConfig.includes('allowLocalEdits')).toBe(false);
  });
});
