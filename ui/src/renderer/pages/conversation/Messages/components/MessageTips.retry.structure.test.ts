/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const tipsSource = readFileSync(new URL('./MessageTips.tsx', import.meta.url), 'utf8');
const zhConversation = JSON.parse(
  readFileSync(new URL('../../../../services/i18n/locales/zh-CN/conversation.json', import.meta.url), 'utf8')
) as { stop?: { failed?: string } };
const enConversation = JSON.parse(
  readFileSync(new URL('../../../../services/i18n/locales/en-US/conversation.json', import.meta.url), 'utf8')
) as { stop?: { failed?: string } };

describe('message error retry entry', () => {
  test('uses a dedicated idempotent continuation instead of edit-resubmit for truncation recovery', () => {
    expect(tipsSource.includes("data-testid='message-error-continue-truncated'")).toBe(true);
    expect(tipsSource.includes('ipcBridge.conversation.continueTruncated.invoke')).toBe(true);
    expect(tipsSource.includes('idempotency_key: recovery.source_message_id')).toBe(true);
    expect(tipsSource.includes('if (message.content.recovery) return null')).toBe(true);
    expect(tipsSource.includes("message.content.type === 'error'")).toBe(true);
    expect(tipsSource.includes('message.content.error?.retryable === true')).toBe(true);
    expect(tipsSource.includes("recovery?.failure_code === 'output_truncated'")).toBe(true);
    expect(tipsSource.includes("recovery?.failure_code === 'turn_requests_exhausted'")).toBe(true);
    expect(tipsSource.includes('message.content.error.code === expectedUiErrorCode')).toBe(true);
  });

  test('keeps the recovery action disabled until the authoritative turn-processing fence opens', () => {
    const hookStart = tipsSource.indexOf('const useTruncatedContinuation');
    const hookEnd = tipsSource.indexOf('const MessageTips:', hookStart);
    const continuationHook = tipsSource.slice(hookStart, hookEnd);

    expect(hookStart).toBeGreaterThan(-1);
    expect(continuationHook.includes('conversationContext?.isProcessing === true')).toBe(true);
    expect(tipsSource.includes('disabled={continuation.disabled}')).toBe(true);
  });

  test('offers a retry entry that recalls the failed request into the composer', () => {
    expect(tipsSource.includes("data-testid='message-error-retry'")).toBe(true);
    expect(tipsSource.includes("emitter.emit('sendbox.edit'")).toBe(true);
    expect(tipsSource.includes("conversationContext?.type !== 'nomi'")).toBe(true);
    expect(tipsSource.includes('common.retry')).toBe(true);
  });

  test('retry hides while the conversation is still processing or read-only', () => {
    expect(tipsSource.includes('conversationContext.isProcessing === true')).toBe(true);
    expect(tipsSource.includes('conversationContext.readOnly === true')).toBe(true);
  });

  test('stop failure toast copy exists in both locales', () => {
    expect(zhConversation.stop?.failed).toBeTruthy();
    expect(enConversation.stop?.failed).toBeTruthy();
  });
});
