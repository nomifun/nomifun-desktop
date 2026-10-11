/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TMessage } from '@/common/chat/chatLib';
import type { IWorkspaceFlatFile } from '@/common/adapter/ipcBridge';
import type { TChatConversation } from '@/common/config/storage';
import { getMessageRoleKey, readMessageContent, sanitizeFileName } from '@/renderer/utils/chat/conversationExport';

import type { ExportZipFile } from '../types';

const normalizeZipPath = (value: string): string => value.replace(/\\/g, '/');

export const buildTopicFolderName = (conversation: TChatConversation): string => {
  const safeName = sanitizeFileName(conversation.name || conversation.id);
  return `${safeName}__${conversation.id}`;
};

export const appendWorkspaceFilesToZip = (
  files: ExportZipFile[],
  workspaceFiles: IWorkspaceFlatFile[],
  prefix: string
): void => {
  for (const file of workspaceFiles) {
    const relativePath = normalizeZipPath(file.relativePath);
    if (!relativePath || /^[A-Za-z]:/.test(relativePath) || relativePath.split('/').some((part) => part === '..' || part === '.' || !part)) {
      throw new Error(`Workspace export file has an invalid relative path: ${file.relativePath}`);
    }
    files.push({ name: `${prefix}/workspace/${relativePath}`, file_path: file.fullPath });
  }
};

export const getBackendKeyFromConversation = (conversation: TChatConversation): string | undefined =>
  conversation.type;

const getMarkdownMessageRoleLabel = (message: TMessage): string => {
  switch (getMessageRoleKey(message)) {
    case 'user':
      return 'User';
    case 'assistant':
      return 'Assistant';
    case 'system':
      return 'System';
  }
};

export const buildConversationMarkdown = (conversation: TChatConversation, messages: TMessage[]): string => {
  const lines: string[] = [];
  lines.push(`# ${conversation.name || 'Conversation'}`);
  lines.push('');
  lines.push(`- Conversation ID: ${conversation.id}`);
  lines.push(`- Exported At: ${new Date().toISOString()}`);
  lines.push(`- Type: ${conversation.type}`);
  lines.push('');
  lines.push('## Messages');
  lines.push('');

  messages.forEach((message, index) => {
    lines.push(`### ${index + 1}. ${getMarkdownMessageRoleLabel(message)} (${message.type})`);
    lines.push('');
    const content = readMessageContent(message);
    const longestBackticks = (content.match(/`+/g) ?? []).reduce((longest, run) => Math.max(longest, run.length), 0);
    const fence = '`'.repeat(Math.max(3, longestBackticks + 1));
    lines.push(`${fence}text`);
    lines.push(content);
    lines.push(fence);
    lines.push('');
  });

  return lines.join('\n');
};

export const buildConversationJson = (conversation: TChatConversation, messages: TMessage[]): string => {
  return JSON.stringify(
    {
      version: 1,
      exportedAt: new Date().toISOString(),
      conversation,
      messages,
    },
    null,
    2
  );
};
