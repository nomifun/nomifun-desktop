/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./MessageText.tsx', import.meta.url), 'utf8');
const typographySource = readFileSync(new URL('../typography.ts', import.meta.url), 'utf8');

describe('MessageText process action chrome', () => {
  test('keeps copy and time visible while allowing active process text to hide the row', () => {
    expect(source.includes('hideActions?: boolean')).toBe(true);
    expect(source.includes('const shouldShowActions = !hideActions;')).toBe(true);
    expect(source.includes("data-testid='message-copy-action'")).toBe(true);
    expect(source.includes("fill='currentColor'")).toBe(true);
    const copyButtonSource =
      source.match(/const copyButton = \([\s\S]*?const canEdit =/)?.[0] ?? '';
    expect(copyButtonSource.includes('opacity-0')).toBe(false);
    expect(copyButtonSource.includes('pointer-events-none')).toBe(false);
    expect(source.includes('text-t-secondary opacity-0 group-hover:opacity-100')).toBe(false);
    expect(source.includes("className='text-12px leading-20px text-inherit select-none'")).toBe(true);
  });

  test('can render the unchanged message actions at the visual end of a turn', () => {
    expect(source.includes('actionsOnly?: boolean')).toBe(true);
    expect(source.includes('if (actionsOnly)')).toBe(true);
    expect(source.includes('{actionsRow}')).toBe(true);
  });

  test('uses one body typography contract for plain text and markdown text', () => {
    expect(typographySource.includes("export const MESSAGE_BODY_FONT_SIZE = 'var(--conversation-message-font-size)';")).toBe(
      true
    );
    expect(
      typographySource.includes("export const MESSAGE_BODY_LINE_HEIGHT = 'var(--conversation-message-line-height)';")
    ).toBe(true);
    expect(typographySource.includes("export const MESSAGE_BODY_CLASS_NAME = 'message-text-body whitespace-pre-wrap break-words';")).toBe(
      true
    );
    expect(source.includes("from '../typography'")).toBe(true);
    expect(source.includes('className={MESSAGE_BODY_CLASS_NAME}')).toBe(true);
    expect(source.includes('fontSize={MESSAGE_BODY_FONT_SIZE}')).toBe(true);
    expect(source.includes('lineHeight={MESSAGE_BODY_LINE_HEIGHT}')).toBe(true);
  });

  test('does not render the retired Conversation knowledge-writeback projection', () => {
    expect(source.includes('ipcBridge.conversation.retryKnowledgeWriteback.invoke')).toBe(false);
    expect(source.includes('MessageKnowledgeWriteback')).toBe(false);
    expect(source.includes('knowledge_writeback')).toBe(false);
  });

  test('routes file marker parsing through the message-side trust boundary', () => {
    expect(source.includes("import { parseMessageFileMarker } from './messageFileMarker';")).toBe(true);
    expect(source.includes('parseMessageFileMarker(contentToRender, message.position)')).toBe(true);
    expect(source.includes('const parseFileMarker')).toBe(false);
  });
});
