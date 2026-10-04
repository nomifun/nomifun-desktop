/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import en from '@/renderer/services/i18n/locales/en-US/conversation.json';
import zh from '@/renderer/services/i18n/locales/zh-CN/conversation.json';
import { getConversationCreateErrorMessage, getConversationRuntimeWorkspaceErrorMessage } from './conversationCreateError';

const i18n = createInstance();
await i18n.init({ resources: { 'zh-CN': { translation: { conversation: zh } }, 'en-US': { translation: { conversation: en } } }, fallbackLng: 'en-US', initImmediate: false });
const error = (message: string) => new BackendHttpError({ method: 'POST', path: '/api/agent-sessions/example/turns', status: 409, body: { code: 'CONFLICT', error: message } });
const captured = error('Conflict: AGENT_SESSION_NON_MODEL_CONTRACT_CHANGED: model switching would change the frozen Agent capability contract');

describe('saved conversation configuration conflict', () => {
  test('gives Chinese recovery guidance for the actual structured 409 in both send paths', () => {
    const t = i18n.getFixedT('zh-CN');
    for (const format of [getConversationCreateErrorMessage, getConversationRuntimeWorkspaceErrorMessage]) {
      expect(format(captured, t)).toBe('当前会话保存的 Agent 配置与当前配置不兼容，无法直接继续。请通过会话的 Agent 切换流程确认兼容配置，或新建会话；已有记录会保留。');
    }
  });
  test('gives English recovery guidance for an English interface', () => {
    expect(getConversationCreateErrorMessage(captured, i18n.getFixedT('en-US'))).toBe('This conversation’s saved agent configuration is incompatible with the current configuration. Confirm a compatible configuration through the conversation’s agent selector, or start a new conversation. Existing records are preserved.');
  });
  test('preserves unrelated conflicts and does not match a quoted internal tag', () => {
    const unrelated = error('Conflict: the operation is already in progress');
    const quoted = error('Conflict: supplied text contains AGENT_SESSION_NON_MODEL_CONTRACT_CHANGED');
    expect(getConversationCreateErrorMessage(unrelated, i18n.getFixedT('zh-CN'))).toBe(unrelated.backendMessage);
    expect(getConversationRuntimeWorkspaceErrorMessage(quoted, i18n.getFixedT('zh-CN'))).toBe(quoted.backendMessage);
  });
});
