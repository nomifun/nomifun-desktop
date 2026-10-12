import { expect, test } from 'bun:test';
import type { TMessage } from '@/common/chat/chatLib';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId } from '@/common/types/ids';
import type { ExportZipFile } from '../types';
import { appendWorkspaceFilesToZip, buildConversationMarkdown } from './exportHelpers';

test('recursive inventory entries retain binary file sources and nested relative archive paths', () => {
  const files: ExportZipFile[] = [];
  appendWorkspaceFilesToZip(files, [
    { name: 'photo.png', fullPath: 'D:\\project\\assets\\photo.png', relativePath: 'assets\\photo.png' },
  ], 'topic');
  expect(files).toEqual([{ name: 'topic/workspace/assets/photo.png', file_path: 'D:\\project\\assets\\photo.png' }]);
  expect(() => appendWorkspaceFilesToZip(files, [
    { name: 'escape', fullPath: 'D:/escape', relativePath: '../escape' },
  ], 'topic')).toThrow('invalid relative path');
});

test('Markdown messages containing code fences retain their complete literal content', () => {
  const id = parseConversationId('0190f5fe-7c00-7a00-8000-000000000001');
  const content = 'Example:\n```ts\nconst x = 1;\n```';
  const output = buildConversationMarkdown({ id, name: 'topic', type: 'nomi' } as TChatConversation, [
    { id: 'row', conversation_id: id, type: 'text', position: 'left', content: { content } } as TMessage,
  ]);
  expect(output).toContain(`\n\`\`\`\`text\n${content}\n\`\`\`\`\n`);
});
