/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ICreateConversationParams } from '@/common/adapter/ipcBridge';
import type { TProviderWithModel } from '@/common/config/storage';
import { Message } from '@arco-design/web-react';
import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { emitter } from '@/renderer/utils/emitter';
import { seedConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import { getConversationCreateErrorMessage } from '@/renderer/pages/conversation/utils/conversationCreateError';
import { useGuidModelSelection } from '@/renderer/pages/guid/hooks/useGuidModelSelection';
import { conversationTarget, parseConversationId } from '@/common/types/ids';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import { uuidv7 } from '@/common/utils/uuidv7';
import { prepareOfficialAgent } from '@/renderer/pages/guid/hooks/officialAgentLaunch';

const FIXED_RESOURCE_IDS: Record<string, string> = {
  workspace: 'default-workspace',
  process_session: 'managed-process-session',
  terminal: 'managed-terminal',
  scheduler: 'installation-scheduler',
  browser: 'managed-browser',
  computer: 'local-desktop',
  project_memory: 'default-project-memory',
};

/**
 * Additions merged onto the create call's `extra` bag.
 *
 * The index signature is deliberate: `extra` is a closed literal shape owned by
 * the integration spine, but the backend keeps unknown keys, which is how
 * capability markers and `system_prompt` ride along.
 */
export type NomiQuickStartExtra = Partial<ICreateConversationParams['extra']> & Record<string, unknown>;

export interface NomiQuickStartOptions {
  /** Conversation title. */
  name: string;
  /** Initial user content, either sent automatically or restored as a draft. */
  prompt: string;
  /** Defaults to true. When false, the prompt is prefilled instead of sent. */
  send?: boolean;
  /**
   * Overrides the hook's own model selection. Pass this when the caller already
   * owns a `useGuidModelSelection` instance (a second instance would not see the
   * model the user picked in the caller's picker).
   */
  model?: TProviderWithModel;
  /** Merged onto `extra`, overriding the workspace defaults below. */
  extra?: NomiQuickStartExtra;
}

/**
 * Spin up a fresh Nomi conversation seeded with initial content, then jump to
 * it. Mirrors the Nomi branch of `useGuidSend`: create → refresh history →
 * stash the initial content in sessionStorage (consumed by `NomiSendBox`) →
 * navigate. Callers can opt out of auto-send when the user must review or
 * confirm the content first.
 */
export const useNomiQuickStart = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { current_model } = useGuidModelSelection('nomi');

  const start = useCallback(
    async ({ name, prompt, send = true, model, extra }: NomiQuickStartOptions): Promise<boolean> => {
      const effectiveModel = model ?? current_model;
      if (!effectiveModel) {
        Message.warning(t('conversation.noModelConfigured'));
        return false;
      }
      try {
        if (extra && Object.keys(extra).length > 0) {
          throw new Error('Quick start resources must be selected through the Agent binding');
        }
        const library = await ipcBridge.agentPlatform.library.invoke();
        const template = library.official_templates.find(
          (candidate) => candidate.template_key === 'assistant.general'
        );
        if (!template) throw new Error('AGENT_PRESET_REQUIRED');
        const preset = await prepareOfficialAgent(template, name, effectiveModel);
        const resourceSelections = template.seed.required_resource_kinds.flatMap((resource_kind) => {
          const resource_id = FIXED_RESOURCE_IDS[resource_kind];
          return resource_id ? [{ resource_kind, resource_id }] : [];
        });
        const session = await ipcBridge.agentPlatform.sessions.create.invoke({
          preset_id: preset.preset_id,
          title: name,
          model: { provider_id: effectiveModel.id, model: effectiveModel.use_model },
          ...(resourceSelections.length ? { resource_selections: resourceSelections } : {}),
        });
        const conversation = await ipcBridge.conversation.get.invoke({
          conversation_id: parseConversationId(session.agent_session_id),
        });
        if (!conversation || !conversation.id) {
          Message.error(t('conversation.createFailed'));
          return false;
        }
        emitter.emit('chat.history.refresh');
        const target = conversationTarget(conversation.id);
        sessionStorage.setItem(
          send
            ? sessionStorageKey('initial-message-nomi', target)
            : sessionStorageKey('draft', target),
          JSON.stringify(
            send
              ? {
                  conversation_id: conversation.id,
                  initial_admission_epoch: 0,
                  input: prompt,
                  idempotency_key: uuidv7(),
                }
              : { input: prompt }
          )
        );
        seedConversationCache(conversation);
        await navigate(`/conversation/${conversation.id}`);
        return true;
      } catch (error) {
        console.error('Nomi quick start failed:', error);
        Message.error(getConversationCreateErrorMessage(error, t));
        return false;
      }
    },
    [current_model, navigate, t]
  );

  return { start, canStart: Boolean(current_model) };
};
