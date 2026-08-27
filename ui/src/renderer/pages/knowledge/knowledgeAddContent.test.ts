import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const detailSource = readFileSync(new URL('./KnowledgeDetailPage/index.tsx', import.meta.url), 'utf8');
const controlSource = readFileSync(
  new URL('./KnowledgeDetailPage/KnowledgeAddContentControl.tsx', import.meta.url),
  'utf8',
);
const bridgeSource = readFileSync(new URL('../../../common/adapter/ipcBridge.ts', import.meta.url), 'utf8');
const sourceConfig = readFileSync(new URL('./CreateStudio/SourceConfig.tsx', import.meta.url), 'utf8');
const knowledgeHookSource = readFileSync(new URL('./useKnowledge.ts', import.meta.url), 'utf8');

describe('adding content to an existing knowledge base', () => {
  test('turns the primary plus into a three-method intent bubble', () => {
    expect(detailSource.includes('KnowledgeAddContentControl')).toBe(true);
    expect(controlSource.includes('AddKnowledgeMenuPanel')).toBe(true);
    expect(controlSource.includes("trigger='click'")).toBe(true);
    expect(controlSource.includes('openDocument')).toBe(true);
    expect(controlSource.includes('openFolderImport')).toBe(true);
    expect(controlSource.includes('openWebImport')).toBe(true);
    expect(controlSource.includes('uploadTodo')).toBe(false);
  });

  test('uses one append-only API contract for document, folder, and web content', () => {
    expect(bridgeSource.includes("type: 'document'")).toBe(true);
    expect(bridgeSource.includes("type: 'local_folder'")).toBe(true);
    expect(bridgeSource.includes("type: 'web'")).toBe(true);
    expect(bridgeSource.includes('/content')).toBe(true);
    expect(controlSource.match(/addKnowledgeContent\(knowledgeBaseId,/g)?.length).toBe(3);
    expect(knowledgeHookSource.includes('ipcBridge.knowledge.addContent.invoke')).toBe(true);
    expect(controlSource.includes('destination_parent_path: defaultFolderPath || undefined')).toBe(true);
    expect(controlSource.includes('destination_parent_id: defaultFolderEntryId')).toBe(true);
  });

  test('reuses the URL-entry editor in create and post-create flows', () => {
    expect(sourceConfig.includes('KnowledgeUrlEntriesEditor')).toBe(true);
    expect(controlSource.includes('KnowledgeUrlEntriesEditor')).toBe(true);
    expect(controlSource.includes('parseKnowledgeUrlDrafts')).toBe(true);
  });
});
