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
import enGuid from '@/renderer/services/i18n/locales/en-US/guid.json';
import zhGuid from '@/renderer/services/i18n/locales/zh-CN/guid.json';
import { getConversationCreateErrorMessage, getConversationRuntimeWorkspaceErrorMessage } from './conversationCreateError';

const i18n = createInstance();
await i18n.init({ resources: { 'zh-CN': { translation: { conversation: zh, guid: zhGuid } }, 'en-US': { translation: { conversation: en, guid: enGuid } } }, fallbackLng: 'en-US', initImmediate: false });
const error = (message: string) => new BackendHttpError({ method: 'POST', path: '/api/agent-sessions/example/turns', status: 409, body: { code: 'CONFLICT', error: message } });
const captured = error('Conflict: AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED: capability workspace.files changed authority, effects, resources, dependencies or provenance');

describe('saved conversation configuration conflict', () => {
  test('preparation and an existing Session have different Chinese recovery guidance', () => {
    const t = i18n.getFixedT('zh-CN');
    const preparation = getConversationCreateErrorMessage(captured, t);
    expect(preparation).toStartWith(zhGuid.agentEntries.contractChanged);
    expect(preparation).toContain('[CONFLICT]');
    expect(preparation).toContain(captured.backendMessage);
    expect(preparation).not.toContain('请新建会话');
    expect(getConversationRuntimeWorkspaceErrorMessage(captured, t)).toBe(zh.agentError.codes.AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED.body);
  });
  test('preparation and an existing Session have different English recovery guidance', () => {
    const t = i18n.getFixedT('en-US');
    expect(getConversationCreateErrorMessage(captured, t)).toStartWith(enGuid.agentEntries.contractChanged);
    expect(getConversationCreateErrorMessage(captured, t)).not.toContain('Start a new session');
    expect(getConversationRuntimeWorkspaceErrorMessage(captured, t)).toBe(en.agentError.codes.AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED.body);
  });
  test.each(['AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED', 'CAPABILITY_ACTION_NOT_MATERIALIZED'])('a typed %s preparation failure keeps only safe diagnostics', code => {
    const failure = new BackendHttpError({ method: 'POST', path: '/api/agent-sessions', status: 409, body: {
      code, error: 'The selected action is unavailable', details: {
        private_data: 'do not render arbitrary details', credential: 'private-credential',
        diagnostics: [{ code: 'ACTION_CHANGED', subject: 'workspace.files', message: 'Review the selected operations.' }],
      },
    } });
    const message = getConversationCreateErrorMessage(failure, i18n.getFixedT('zh-CN'));
    expect(message).toContain(`[${code}]`);
    expect(message).toContain('ACTION_CHANGED: workspace.files: Review the selected operations.');
    expect(message).not.toContain('do not render arbitrary details');
    expect(message).not.toContain('private-credential');
    expect(message).not.toContain('请新建会话');
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
    expect(getConversationCreateErrorMessage(unrelated, i18n.getFixedT('zh-CN'))).toBe(`[CONFLICT] ${unrelated.backendMessage}`);
    expect(getConversationRuntimeWorkspaceErrorMessage(quoted, i18n.getFixedT('zh-CN'))).toBe(quoted.backendMessage);
  });
});
