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
const captured = error('Conflict: AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED: capability workspace.files changed authority, effects, resources, dependencies or provenance');

describe('saved conversation configuration conflict', () => {
  test('gives Chinese recovery guidance for the actual structured 409 in both send paths', () => {
    const t = i18n.getFixedT('zh-CN');
    for (const format of [getConversationCreateErrorMessage, getConversationRuntimeWorkspaceErrorMessage]) {
      expect(format(captured, t)).toBe('升级涉及权限、工具来源或执行要求的变化，无法自动沿用当前会话的 Agent 配置。请通过会话的 Agent 选择器确认配置后继续；已有记录会保留。');
    }
  });
  test('gives English recovery guidance for an English interface', () => {
    expect(getConversationCreateErrorMessage(captured, i18n.getFixedT('en-US'))).toBe('The upgrade changes permissions, tool sources, or execution requirements, so this conversation’s Agent configuration cannot be updated automatically. Confirm its configuration through the conversation’s Agent selector to continue. Existing records are preserved.');
  });
  test('uses the same guidance for a direct structured contract evolution code', () => {
    const rejected = new BackendHttpError({ method: 'PUT', path: '/api/agent-sessions/example/model', status: 409, body: {
      code: 'AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED', error: 'contract evolution changed the role provider authority or provenance',
    } });
    const t = i18n.getFixedT('zh-CN');
    expect(getConversationRuntimeWorkspaceErrorMessage(rejected, t)).toBe(getConversationRuntimeWorkspaceErrorMessage(captured, t));
  });
  test('preserves unrelated conflicts and does not match a quoted internal tag', () => {
    const unrelated = error('Conflict: the operation is already in progress');
    const quoted = error('Conflict: supplied text contains AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED');
    expect(getConversationCreateErrorMessage(unrelated, i18n.getFixedT('zh-CN'))).toBe(unrelated.backendMessage);
    expect(getConversationRuntimeWorkspaceErrorMessage(quoted, i18n.getFixedT('zh-CN'))).toBe(quoted.backendMessage);
  });
});
