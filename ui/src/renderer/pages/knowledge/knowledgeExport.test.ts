import { describe, expect, test } from 'bun:test';
import { joinLocalPath } from '@/common/utils/localPath';
import { defaultKnowledgeExportName, isKnowledgeExportNameValid } from './knowledgeExport';

describe('knowledge export destination', () => {
  test('turns a selected directory into a ZIP file path on Windows, POSIX and UNC', () => {
    const name = defaultKnowledgeExportName('产品/资料:*', new Date(2026, 9, 12));
    expect(name).toBe('产品-资料---2026-10-12.zip');
    expect(isKnowledgeExportNameValid(name)).toBe(true);
    expect(joinLocalPath('C:\\Exports', name)).toBe(`C:\\Exports\\${name}`);
    expect(joinLocalPath('/exports', name)).toBe(`/exports/${name}`);
    expect(joinLocalPath('\\\\server\\share', name)).toBe(`\\\\server\\share\\${name}`);
  });

  test('rejects traversal, path separators, reserved names and non-ZIP filenames', () => {
    for (const name of ['../data.zip', 'C:\\data.zip', '/data.zip', '..\\data.zip', 'CON.zip', 'CON .zip', 'data.txt', 'data.zip ', 'data?.zip', '.zip']) {
      expect(isKnowledgeExportNameValid(name)).toBe(false);
    }
    expect(isKnowledgeExportNameValid('资料.zip')).toBe(true);
    expect(isKnowledgeExportNameValid(defaultKnowledgeExportName('NUL'))).toBe(true);
    expect(isKnowledgeExportNameValid(defaultKnowledgeExportName('CON.notes'))).toBe(true);
    expect(isKnowledgeExportNameValid(defaultKnowledgeExportName('AUX .notes'))).toBe(true);
    expect(isKnowledgeExportNameValid(defaultKnowledgeExportName(' . '))).toBe(true);
  });
});
