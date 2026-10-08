/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, test } from 'bun:test';
import { cleanup, render, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { initReactI18next } from 'react-i18next';
import { composeMessage, transformMessage, type IMessageText } from '@/common/chat/chatLib';
import { createStoredMessageMapper } from '@/common/adapter/storedMessageMapper';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import MarkdownView from '@/renderer/components/Markdown';
import {
  beginCompanionSessionBubble, emptyCompanionSessionBubble, hydrateCompanionSessionBubble,
  settleCompanionSessionBubble, streamCompanionSessionBubble,
} from './companionSessionBubble';

await createInstance().use(initReactI18next).init({ lng: 'zh-CN', resources: {} });
afterEach(cleanup);

const conversationId = parseConversationId('019f0000-0000-7000-8000-000000000931');
const root = parseMessageId('019f0000-0000-7000-8000-000000000932');
const segment = parseMessageId('019f0000-0000-7000-8000-000000000933');
const markdown = [
  '主人好呀~ 天天帮你把 **Java 数据类型** 整理好啦！',
  '',
  '## 基本数据类型',
  '',
  '| 类型 | 示例 |',
  '| --- | --- |',
  '| `int` | `42` |',
  '| `boolean` | `true` |',
  '',
  '- 整数用于计数。',
  '- 布尔值用于判断。',
  '',
  '```java',
  'int count = 42;',
  'String name = "天天";',
  '```',
  '',
  'JSON 也可以作为代码示例，而不是把整个回复变成内部数据：',
  '',
  '```json',
  '{"types":["int","boolean"]}',
  '```',
  '',
  '第一行提示',
  '第二行提示',
].join('\n');

test.each(['live', 'settled history', 'cold history'] as const)(
  '%s keeps ordinary reply Markdown as text through both conversation surfaces', async source => {
    let bubble = beginCompanionSessionBubble(emptyCompanionSessionBubble(conversationId), root, 'running');
    let messages: ReturnType<typeof composeMessage> = [];
    const chunks = [markdown.slice(0, 35), markdown.slice(35, 145), markdown.slice(145)];
    for (const [index, content] of chunks.entries()) {
      const frame = { conversation_id: conversationId, turn_id: root, msg_id: segment,
        type: 'content', data: index === 0 ? content : { content }, created_at: 100 };
      messages = composeMessage(transformMessage(frame), messages);
      bubble = streamCompanionSessionBubble(bubble, frame, 'Using tools', 'Provider error');
    }
    const stored = createStoredMessageMapper(() => 'history-row')({
      message_id: segment, msg_id: segment, conversation_id: conversationId, type: 'text',
      content: { content: markdown, turn_id: root }, position: 'left', status: 'finish', hidden: false, created_at: 100,
    });
    if (source === 'settled history') {
      bubble = settleCompanionSessionBubble(bubble, root, 'Done');
      bubble = hydrateCompanionSessionBubble(bubble, [stored], bubble);
    } else if (source === 'cold history') {
      bubble = beginCompanionSessionBubble(emptyCompanionSessionBubble(conversationId), root, 'running');
      bubble = hydrateCompanionSessionBubble(bubble, [stored], bubble);
    }
    const message = (source === 'live' ? messages[0] : stored) as IMessageText;
    expect(message.content.content).toBe(markdown);
    expect(bubble.bubble).toBe(markdown);

    // The main conversation and desktop companion use this same renderer.
    // Inspect its actual Shadow DOM instead of substituting a text-only mock.
    const view = render(<MemoryRouter>
      <MarkdownView>{message.content.content}</MarkdownView>
      <MarkdownView>{bubble.bubble}</MarkdownView>
    </MemoryRouter>);
    await waitFor(() => {
      const roots = Array.from(view.container.querySelectorAll('.markdown-shadow')).map(host => host.shadowRoot);
      expect(roots).toHaveLength(2);
      for (const rendered of roots) {
        expect(rendered?.querySelector('h2')?.textContent).toBe('基本数据类型');
        expect(rendered?.querySelectorAll('table tbody tr')).toHaveLength(2);
        expect(rendered?.querySelectorAll('li')).toHaveLength(2);
        const code = Array.from(rendered?.querySelectorAll('pre') ?? []).map(block => block.textContent);
        expect(code.some(text => text?.includes('int count = 42;') && text.includes('String name = "天天";'))).toBe(true);
        expect(code.some(text => text?.replace(/\s/g, '').includes('{"types":["int","boolean"]}'))).toBe(true);
        expect(rendered?.querySelector('br')).not.toBeNull();
        expect(rendered?.textContent).not.toContain('historical_assistant_answer');
        expect(rendered?.textContent).not.toContain('original_text');
      }
    });
  }
);
